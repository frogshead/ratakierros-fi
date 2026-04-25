use argon2::{
    password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
    Argon2,
};
use rand_core::OsRng;
use chrono::Utc;
use geo::HaversineDistance;
use geo_types::Point;
use jsonwebtoken::{decode, encode, DecodingKey, EncodingKey, Header, Validation};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};

pub type Db = Arc<Mutex<Connection>>;

// --- Types ---

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Track {
    pub id: i64,
    pub osm_id: String,
    pub name: Option<String>,
    pub lat: f64,
    pub lon: f64,
    pub city: Option<String>,
    pub suburb: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TrackWithDistance {
    #[serde(flatten)]
    pub track: Track,
    pub distance_m: Option<f64>,
    pub record: Option<f64>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RecordEntry {
    pub rank: i64,
    pub display_name: String,
    pub time_seconds: f64,
    pub logged_at: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TrackRecords {
    pub track: Track,
    pub records: Vec<RecordEntry>,
    pub personal_best: Option<f64>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Claims {
    pub sub: i64,
    pub exp: usize,
    pub display_name: String,
}

// --- Database ---

pub fn init_db(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "PRAGMA journal_mode=WAL;
         PRAGMA foreign_keys=ON;

         CREATE TABLE IF NOT EXISTS tracks (
             id      INTEGER PRIMARY KEY AUTOINCREMENT,
             osm_id  TEXT UNIQUE NOT NULL,
             name    TEXT,
             lat     REAL NOT NULL,
             lon     REAL NOT NULL,
             city    TEXT,
             suburb  TEXT
         );

         CREATE TABLE IF NOT EXISTS users (
             id            INTEGER PRIMARY KEY AUTOINCREMENT,
             email         TEXT UNIQUE NOT NULL,
             display_name  TEXT NOT NULL,
             password_hash TEXT NOT NULL,
             created_at    TEXT NOT NULL
         );

         CREATE TABLE IF NOT EXISTS runs (
             id           INTEGER PRIMARY KEY AUTOINCREMENT,
             user_id      INTEGER NOT NULL REFERENCES users(id),
             track_id     INTEGER NOT NULL REFERENCES tracks(id),
             time_seconds REAL NOT NULL,
             logged_at    TEXT NOT NULL
         );",
    )
}

pub fn migrate_db(conn: &Connection) {
    // Add suburb column if upgrading from older schema
    conn.execute_batch("ALTER TABLE tracks ADD COLUMN suburb TEXT").ok();
}

// Merge tracks that are within 400 m of each other (same venue, different OSM objects).
// Keeps the richest metadata and re-points any existing runs to the surviving track.
// Returns the number of tracks removed.
pub fn deduplicate_nearby_tracks(db: &Db) -> usize {
    let conn = db.lock().unwrap();

    type TrackRow = (i64, Option<String>, f64, f64, Option<String>, Option<String>);

    let all: Vec<TrackRow> = {
        let mut stmt = match conn.prepare(
            "SELECT id, name, lat, lon, city, suburb FROM tracks ORDER BY id",
        ) {
            Ok(s) => s,
            Err(_) => return 0,
        };
        let rows: Vec<TrackRow> = match stmt.query_map([], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?))
        }) {
            Ok(mapped) => mapped.filter_map(|r| r.ok()).collect(),
            Err(_) => return 0,
        };
        rows
    };

    let n = all.len();
    if n == 0 { return 0; }

    // Union-Find (iterative, path-halving)
    let mut parent: Vec<usize> = (0..n).collect();
    let mut find = |parent: &mut Vec<usize>, mut i: usize| -> usize {
        while parent[i] != i {
            parent[i] = parent[parent[i]];
            i = parent[i];
        }
        i
    };

    for i in 0..n {
        for j in (i + 1)..n {
            let (_, _, lat1, lon1, _, _) = all[i];
            let (_, _, lat2, lon2, _, _) = all[j];
            let dist = Point::new(lon1, lat1).haversine_distance(&Point::new(lon2, lat2));
            if dist <= 400.0 {
                let ri = find(&mut parent, i);
                let rj = find(&mut parent, j);
                if ri != rj {
                    parent[ri] = rj;
                }
            }
        }
    }

    // Resolve all roots (path compression)
    let roots: Vec<usize> = (0..n).map(|i| find(&mut parent, i)).collect();

    // Group indices by root
    let mut clusters: std::collections::HashMap<usize, Vec<usize>> =
        std::collections::HashMap::new();
    for (i, &root) in roots.iter().enumerate() {
        clusters.entry(root).or_default().push(i);
    }

    let mut removed = 0;

    for members in clusters.values().filter(|m| m.len() > 1) {
        // Canonical: prefer track with a name; fall back to first
        let canonical_idx = members
            .iter()
            .copied()
            .find(|&i| all[i].1.is_some())
            .unwrap_or(members[0]);
        let canonical_id = all[canonical_idx].0;

        // Best metadata across all cluster members
        let best_name = members
            .iter()
            .filter_map(|&i| all[i].1.as_deref())
            .max_by_key(|s| s.len())
            .map(String::from);
        let best_city = members
            .iter()
            .filter_map(|&i| all[i].4.as_deref())
            .next()
            .map(String::from);
        let best_suburb = members
            .iter()
            .filter_map(|&i| all[i].5.as_deref())
            .next()
            .map(String::from);

        conn.execute(
            "UPDATE tracks SET name = ?1, city = ?2, suburb = ?3 WHERE id = ?4",
            params![best_name, best_city, best_suburb, canonical_id],
        )
        .ok();

        for &i in members.iter().filter(|&&i| i != canonical_idx) {
            let dup_id = all[i].0;
            // Re-point runs so we don't lose history
            conn.execute(
                "UPDATE runs SET track_id = ?1 WHERE track_id = ?2",
                params![canonical_id, dup_id],
            )
            .ok();
            conn.execute("DELETE FROM tracks WHERE id = ?1", params![dup_id]).ok();
            removed += 1;
        }
    }

    removed
}

pub fn tracks_count(db: &Db) -> i64 {
    let conn = db.lock().unwrap();
    conn.query_row("SELECT COUNT(*) FROM tracks", [], |row| row.get(0))
        .unwrap_or(0)
}

// --- Track cache ---

pub async fn fetch_and_cache_tracks(db: Db) -> Result<usize, String> {
    // Finland bounding box: S 59.7, W 19.1, N 70.1, E 31.6
    let query = concat!(
        "[out:json][timeout:180];",
        r#"area["ISO3166-1"="FI"][admin_level=2]->.fi;"#,
        "(",
        r#"node["sport"="athletics"](area.fi);"#,
        r#"way["sport"="athletics"](area.fi);"#,
        r#"relation["sport"="athletics"](area.fi);"#,
        r#"node["name"~"yleisurheilukent",i](area.fi);"#,
        r#"way["name"~"yleisurheilukent",i](area.fi);"#,
        r#"relation["name"~"yleisurheilukent",i](area.fi);"#,
        ");",
        "out center tags;"
    );

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(210))
        .build()
        .map_err(|e| format!("HTTP client error: {}", e))?;

    let endpoints = [
        "https://overpass-api.de/api/interpreter",
        "https://overpass.kumi.systems/api/interpreter",
    ];

    let mut last_err = String::new();
    let mut body_opt: Option<String> = None;

    for endpoint in &endpoints {
        let result = client
            .post(*endpoint)
            .header("User-Agent", "ratakierros-fi/1.0 (https://ratakierros.fi)")
            .form(&[("data", query)])
            .send()
            .await;

        match result {
            Err(e) => { last_err = format!("Request to {} failed: {}", endpoint, e); }
            Ok(response) => {
                let status = response.status();
                let body = response.text().await
                    .map_err(|e| format!("Failed to read response: {}", e))?;
                if status.is_success() {
                    body_opt = Some(body);
                    break;
                }
                last_err = format!("Overpass {} returned HTTP {}: {}", endpoint, status, &body[..body.len().min(200)]);
            }
        }
    }

    let body = body_opt.ok_or(last_err)?;

    let data: serde_json::Value = serde_json::from_str(&body)
        .map_err(|e| format!("Failed to parse Overpass response ({}): body starts with: {}", e, &body[..body.len().min(200)]))?;

    let elements = match data["elements"].as_array() {
        Some(e) => e,
        None => return Err("No elements in Overpass response".to_string()),
    };

    let conn = db.lock().unwrap();
    let mut count = 0;

    for element in elements {
        let (lat, lon) = if element["type"] == "node" {
            (element["lat"].as_f64(), element["lon"].as_f64())
        } else if let Some(center) = element.get("center") {
            (center["lat"].as_f64(), center["lon"].as_f64())
        } else {
            continue;
        };

        let (lat, lon) = match (lat, lon) {
            (Some(la), Some(lo)) if la != 0.0 || lo != 0.0 => (la, lo),
            _ => continue,
        };

        let osm_type = element["type"].as_str().unwrap_or("node");
        let osm_id = format!("{}/{}", osm_type, element["id"].as_i64().unwrap_or(0));

        let tags = &element["tags"];
        let name = tags["name"].as_str().map(String::from);
        let city = tags["addr:city"]
            .as_str()
            .or_else(|| tags["addr:municipality"].as_str())
            .or_else(|| tags["is_in:city"].as_str())
            .map(String::from);

        conn.execute(
            "INSERT OR REPLACE INTO tracks (osm_id, name, lat, lon, city) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![osm_id, name, lat, lon, city],
        )
        .ok();

        count += 1;
    }

    Ok(count)
}

// Enrich tracks missing a city via Nominatim reverse geocode.
// Nominatim public API: max 1 req/s. Runs as a background task after fetch.
pub async fn enrich_missing_cities(db: Db) {
    let missing: Vec<(i64, f64, f64)> = {
        let conn = db.lock().unwrap();
        let mut stmt = match conn.prepare("SELECT id, lat, lon FROM tracks WHERE city IS NULL") {
            Ok(s) => s,
            Err(_) => return,
        };
        let rows: Vec<(i64, f64, f64)> = match stmt
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
        {
            Ok(mapped) => mapped.filter_map(|r| r.ok()).collect(),
            Err(_) => return,
        };
        rows
    };

    let total = missing.len();
    if total == 0 { return; }
    println!("Enriching {} tracks without city via Nominatim...", total);

    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
    {
        Ok(c) => c,
        Err(_) => return,
    };

    for (i, (id, lat, lon)) in missing.into_iter().enumerate() {
        tokio::time::sleep(std::time::Duration::from_millis(1100)).await;

        let url = format!(
            "https://nominatim.openstreetmap.org/reverse?lat={}&lon={}&format=json",
            lat, lon
        );

        let resp = match client.get(&url).header("User-Agent", "ratakierros-fi/1.0").send().await {
            Ok(r) => r,
            Err(_) => continue,
        };
        let data: serde_json::Value = match resp.json().await {
            Ok(d) => d,
            Err(_) => continue,
        };

        let addr = &data["address"];
        let city = ["city", "town", "municipality", "village", "county"]
            .iter()
            .find_map(|k| addr[k].as_str().map(String::from));
        let suburb = ["suburb", "village", "hamlet", "quarter", "city_district"]
            .iter()
            .find_map(|k| addr[k].as_str().map(String::from))
            .filter(|s| Some(s) != city.as_ref()); // skip if same as city

        if city.is_some() || suburb.is_some() {
            let conn = db.lock().unwrap();
            conn.execute(
                "UPDATE tracks SET city = ?1, suburb = ?2 WHERE id = ?3",
                params![city, suburb, id],
            ).ok();
        }

        if (i + 1) % 10 == 0 || i + 1 == total {
            println!("Nominatim enrichment: {}/{}", i + 1, total);
        }
    }

    println!("Nominatim enrichment complete.");
}

// --- Track queries ---

pub fn list_tracks(
    db: &Db,
    lat: Option<f64>,
    lon: Option<f64>,
    q: Option<&str>,
) -> Result<Vec<TrackWithDistance>, String> {
    let conn = db.lock().unwrap();

    type Row = (i64, String, Option<String>, f64, f64, Option<String>, Option<String>, Option<f64>);

    let rows: Vec<Row> = if let Some(q_str) = q.filter(|s| !s.is_empty()) {
        let pattern = format!("%{}%", q_str);
        let mut stmt = conn
            .prepare(
                "SELECT t.id, t.osm_id, t.name, t.lat, t.lon, t.city, t.suburb, MIN(r.time_seconds) \
                 FROM tracks t LEFT JOIN runs r ON r.track_id = t.id \
                 WHERE LOWER(t.name) LIKE LOWER(?1) OR LOWER(t.city) LIKE LOWER(?1) \
                    OR LOWER(t.suburb) LIKE LOWER(?1) \
                 GROUP BY t.id ORDER BY t.name",
            )
            .map_err(|e| e.to_string())?;
        let collected: Vec<Row> = stmt
            .query_map(params![pattern], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?, row.get(6)?, row.get(7)?))
            })
            .map_err(|e| e.to_string())?
            .filter_map(|r| r.ok())
            .collect();
        collected
    } else {
        let mut stmt = conn
            .prepare(
                "SELECT t.id, t.osm_id, t.name, t.lat, t.lon, t.city, t.suburb, MIN(r.time_seconds) \
                 FROM tracks t LEFT JOIN runs r ON r.track_id = t.id \
                 GROUP BY t.id ORDER BY t.name",
            )
            .map_err(|e| e.to_string())?;
        let collected: Vec<Row> = stmt
            .query_map([], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?, row.get(6)?, row.get(7)?))
            })
            .map_err(|e| e.to_string())?
            .filter_map(|r| r.ok())
            .collect();
        collected
    };

    let mut result: Vec<TrackWithDistance> = rows
        .into_iter()
        .map(|(id, osm_id, name, tlat, tlon, city, suburb, record)| {
            let distance_m = lat.zip(lon).map(|(ulat, ulon)| {
                Point::new(ulon, ulat).haversine_distance(&Point::new(tlon, tlat))
            });
            TrackWithDistance {
                track: Track { id, osm_id, name, lat: tlat, lon: tlon, city, suburb },
                distance_m,
                record,
            }
        })
        .collect();

    if lat.is_some() {
        result.sort_by(|a, b| match (a.distance_m, b.distance_m) {
            (Some(da), Some(db)) => da.partial_cmp(&db).unwrap_or(std::cmp::Ordering::Equal),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => a.track.name.cmp(&b.track.name),
        });
    }

    Ok(result)
}

pub fn get_track(db: &Db, id: i64) -> Result<Option<TrackWithDistance>, String> {
    let conn = db.lock().unwrap();
    let result = conn.query_row(
        "SELECT t.id, t.osm_id, t.name, t.lat, t.lon, t.city, t.suburb, MIN(r.time_seconds) \
         FROM tracks t LEFT JOIN runs r ON r.track_id = t.id \
         WHERE t.id = ?1 GROUP BY t.id",
        params![id],
        |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, f64>(3)?,
                row.get::<_, f64>(4)?,
                row.get::<_, Option<String>>(5)?,
                row.get::<_, Option<String>>(6)?,
                row.get::<_, Option<f64>>(7)?,
            ))
        },
    );

    match result {
        Ok((id, osm_id, name, lat, lon, city, suburb, record)) => Ok(Some(TrackWithDistance {
            track: Track { id, osm_id, name, lat, lon, city, suburb },
            distance_m: None,
            record,
        })),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}

// --- Records ---

pub fn get_records(
    db: &Db,
    track_id: i64,
    user_id: Option<i64>,
) -> Result<TrackRecords, String> {
    let conn = db.lock().unwrap();

    let track = conn
        .query_row(
            "SELECT id, osm_id, name, lat, lon, city, suburb FROM tracks WHERE id = ?1",
            params![track_id],
            |row| {
                Ok(Track {
                    id: row.get(0)?,
                    osm_id: row.get(1)?,
                    name: row.get(2)?,
                    lat: row.get(3)?,
                    lon: row.get(4)?,
                    city: row.get(5)?,
                    suburb: row.get(6)?,
                })
            },
        )
        .map_err(|e| e.to_string())?;

    let mut stmt = conn
        .prepare(
            "SELECT u.display_name, r.time_seconds, r.logged_at \
             FROM runs r JOIN users u ON u.id = r.user_id \
             WHERE r.track_id = ?1 \
             ORDER BY r.time_seconds ASC LIMIT 10",
        )
        .map_err(|e| e.to_string())?;

    let records: Vec<RecordEntry> = stmt
        .query_map(params![track_id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, f64>(1)?, row.get::<_, String>(2)?))
        })
        .map_err(|e| e.to_string())?
        .filter_map(|r| r.ok())
        .enumerate()
        .map(|(i, (display_name, time_seconds, logged_at))| RecordEntry {
            rank: (i + 1) as i64,
            display_name,
            time_seconds,
            logged_at,
        })
        .collect();

    let personal_best = if let Some(uid) = user_id {
        conn.query_row(
            "SELECT MIN(time_seconds) FROM runs WHERE track_id = ?1 AND user_id = ?2",
            params![track_id, uid],
            |row| row.get::<_, Option<f64>>(0),
        )
        .ok()
        .flatten()
    } else {
        None
    };

    Ok(TrackRecords { track, records, personal_best })
}

pub fn log_run(db: &Db, user_id: i64, track_id: i64, time_seconds: f64) -> Result<(), String> {
    let conn = db.lock().unwrap();
    conn.execute(
        "INSERT INTO runs (user_id, track_id, time_seconds, logged_at) VALUES (?1, ?2, ?3, ?4)",
        params![user_id, track_id, time_seconds, Utc::now().to_rfc3339()],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

// --- Auth ---

pub fn register_user(
    db: &Db,
    email: &str,
    display_name: &str,
    password: &str,
) -> Result<(String, i64, String), String> {
    let salt = SaltString::generate(&mut OsRng);
    let password_hash = Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map_err(|e| format!("Hash error: {}", e))?
        .to_string();

    let conn = db.lock().unwrap();
    conn.execute(
        "INSERT INTO users (email, display_name, password_hash, created_at) VALUES (?1, ?2, ?3, ?4)",
        params![email, display_name, password_hash, Utc::now().to_rfc3339()],
    )
    .map_err(|e| {
        if e.to_string().contains("UNIQUE") {
            "Sähköpostiosoite on jo käytössä".to_string()
        } else {
            e.to_string()
        }
    })?;

    let user_id = conn.last_insert_rowid();
    let token = make_jwt(user_id, display_name)?;
    Ok((token, user_id, display_name.to_string()))
}

pub fn login_user(db: &Db, email: &str, password: &str) -> Result<(String, i64, String), String> {
    let conn = db.lock().unwrap();

    let (user_id, password_hash, display_name): (i64, String, String) = conn
        .query_row(
            "SELECT id, password_hash, display_name FROM users WHERE email = ?1",
            params![email],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .map_err(|_| "Väärä sähköposti tai salasana".to_string())?;

    let parsed_hash =
        PasswordHash::new(&password_hash).map_err(|e| format!("Hash error: {}", e))?;
    Argon2::default()
        .verify_password(password.as_bytes(), &parsed_hash)
        .map_err(|_| "Väärä sähköposti tai salasana".to_string())?;

    let token = make_jwt(user_id, &display_name)?;
    Ok((token, user_id, display_name))
}

fn make_jwt(user_id: i64, display_name: &str) -> Result<String, String> {
    let exp = (Utc::now() + chrono::Duration::days(30)).timestamp() as usize;
    let claims = Claims { sub: user_id, exp, display_name: display_name.to_string() };
    encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(jwt_secret().as_bytes()),
    )
    .map_err(|e| format!("JWT error: {}", e))
}

pub fn verify_jwt(token: &str) -> Result<Claims, String> {
    decode::<Claims>(
        token,
        &DecodingKey::from_secret(jwt_secret().as_bytes()),
        &Validation::default(),
    )
    .map(|d| d.claims)
    .map_err(|e| format!("JWT invalid: {}", e))
}

fn jwt_secret() -> String {
    std::env::var("JWT_SECRET")
        .unwrap_or_else(|_| "ratakierros-dev-secret-change-in-prod".to_string())
}
