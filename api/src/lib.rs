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

pub mod gpx_analyze;
pub mod lipas;
pub use gpx_analyze::{analyze_gpx, AnalyzeError, AnalyzeResult, BestLap, TraceSummary, DEFAULT_TARGET_DISTANCE_M};
pub use lipas::fetch_and_cache_lipas_tracks;

pub type Db = Arc<Mutex<Connection>>;

pub const BUILD_VERSION: &str = env!("CARGO_PKG_VERSION");
pub const BUILD_COMMIT: &str = env!("GIT_COMMIT");

#[cfg(test)]
mod build_info_tests {
    use super::*;

    #[test]
    fn build_consts_are_populated() {
        assert!(!BUILD_VERSION.is_empty());
        assert!(!BUILD_COMMIT.is_empty());
        // build.rs truncates to 7 chars; "unknown" is the local-dev fallback.
        assert!(BUILD_COMMIT.len() <= 7);
    }
}

// --- Types ---

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Track {
    pub id: i64,
    pub lipas_id: i64,
    pub name: Option<String>,
    pub lat: f64,
    pub lon: f64,
    pub city: Option<String>,
    pub suburb: Option<String>,
    pub address: Option<String>,
    pub postal_code: Option<String>,
    pub surface: Option<String>,
    pub track_length_m: Option<i64>,
    pub lanes: Option<i64>,
    pub status: String,
    pub type_code: i64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TrackWithDistance {
    #[serde(flatten)]
    pub track: Track,
    pub distance_m: Option<f64>,
    pub record: Option<f64>,
    pub is_favorite: bool,
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

const TRACK_COLUMNS: &str =
    "t.id, t.lipas_id, t.name, t.lat, t.lon, t.city, t.suburb, t.address, t.postal_code, \
     t.surface, t.track_length_m, t.lanes, t.status, t.type_code";

// --- Database ---

pub fn init_db(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "PRAGMA journal_mode=WAL;
         PRAGMA foreign_keys=ON;

         CREATE TABLE IF NOT EXISTS tracks (
             id               INTEGER PRIMARY KEY AUTOINCREMENT,
             lipas_id         INTEGER UNIQUE NOT NULL,
             name             TEXT,
             lat              REAL NOT NULL,
             lon              REAL NOT NULL,
             type_code        INTEGER NOT NULL,
             status           TEXT NOT NULL,
             address          TEXT,
             postal_code      TEXT,
             city             TEXT,
             suburb           TEXT,
             surface          TEXT,
             track_length_m   INTEGER,
             lanes            INTEGER,
             geometry_geojson TEXT,
             last_synced_at   TEXT NOT NULL
         );

         CREATE INDEX IF NOT EXISTS idx_tracks_city ON tracks(city);
         CREATE INDEX IF NOT EXISTS idx_tracks_type_status ON tracks(type_code, status);

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
         );

         CREATE TABLE IF NOT EXISTS favorites (
             id         INTEGER PRIMARY KEY AUTOINCREMENT,
             user_id    INTEGER NOT NULL REFERENCES users(id),
             track_id   INTEGER NOT NULL REFERENCES tracks(id),
             created_at TEXT NOT NULL,
             UNIQUE(user_id, track_id)
         );",
    )
}

// Move legacy `tracks` (osm_id-keyed) out of the way before init_db creates the new schema.
// The legacy rows are needed later by `finalize_legacy_migration` to remap runs.track_id.
// MUST run before init_db().
pub fn migrate_db(conn: &Connection) {
    if column_exists(conn, "tracks", "osm_id") && !column_exists(conn, "tracks", "lipas_id") {
        // foreign_keys=OFF during the rename prevents SQLite from auto-redirecting
        // `runs.track_id` FK to `tracks_legacy`. We want it to keep referencing the
        // literal name `tracks`, which init_db re-creates immediately after with the
        // new schema. legacy_alter_table=ON keeps trigger/view defs intact too.
        // Recover from a prior interrupted migration: tracks_legacy may already exist.
        conn.execute_batch(
            "PRAGMA foreign_keys=OFF; \
             PRAGMA legacy_alter_table=ON; \
             DROP TABLE IF EXISTS tracks_legacy; \
             ALTER TABLE tracks RENAME TO tracks_legacy; \
             PRAGMA legacy_alter_table=OFF; \
             PRAGMA foreign_keys=ON;",
        )
        .expect("Failed to rename legacy tracks table");
        println!("Migration: renamed legacy tracks → tracks_legacy");
    }
}

fn column_exists(conn: &Connection, table: &str, col: &str) -> bool {
    let sql = format!("PRAGMA table_info({})", table);
    let mut stmt = match conn.prepare(&sql) {
        Ok(s) => s,
        Err(_) => return false,
    };
    let rows = stmt.query_map([], |row| row.get::<_, String>(1));
    if let Ok(rows) = rows {
        for r in rows.flatten() {
            if r == col {
                return true;
            }
        }
    }
    false
}

fn table_exists(conn: &Connection, table: &str) -> bool {
    conn.query_row(
        "SELECT name FROM sqlite_master WHERE type='table' AND name=?1",
        params![table],
        |_| Ok(()),
    )
    .is_ok()
}

// After a successful Lipas fetch, remap runs.track_id from legacy ids to new ids.
// Strategy: nearest new track within 400 m. Orphans get a synthetic placeholder row
// (negative lipas_id, status='legacy') so run history is never lost.
// Returns (remapped, orphaned). No-op if no legacy table exists.
pub fn finalize_legacy_migration(db: &Db) -> Result<(usize, usize), String> {
    let conn = db.lock().unwrap();
    if !table_exists(&conn, "tracks_legacy") {
        return Ok((0, 0));
    }

    type LegacyRow = (i64, Option<String>, f64, f64, Option<String>, Option<String>);
    let legacy: Vec<LegacyRow> = {
        let mut stmt = conn
            .prepare("SELECT id, name, lat, lon, city, suburb FROM tracks_legacy")
            .map_err(|e| e.to_string())?;
        let collected: Vec<LegacyRow> = stmt
            .query_map([], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?))
            })
            .map_err(|e| e.to_string())?
            .filter_map(|r| r.ok())
            .collect();
        collected
    };

    let new_tracks: Vec<(i64, f64, f64)> = {
        let mut stmt = conn
            .prepare("SELECT id, lat, lon FROM tracks")
            .map_err(|e| e.to_string())?;
        let collected: Vec<(i64, f64, f64)> = stmt
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
            .map_err(|e| e.to_string())?
            .filter_map(|r| r.ok())
            .collect();
        collected
    };

    let mut remapped = 0usize;
    let mut orphaned = 0usize;
    let now = Utc::now().to_rfc3339();

    for (old_id, name, lat, lon, city, suburb) in &legacy {
        // Skip legacy tracks with no run history — they were just OSM noise, no need
        // to preserve them as placeholder rows in the new tracks table.
        let run_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM runs WHERE track_id = ?1",
                params![*old_id],
                |r| r.get(0),
            )
            .unwrap_or(0);
        if run_count == 0 {
            continue;
        }

        let mut best: Option<(i64, f64)> = None;
        for &(nid, nlat, nlon) in &new_tracks {
            let d = Point::new(*lon, *lat).haversine_distance(&Point::new(nlon, nlat));
            if d <= 400.0 && best.map_or(true, |(_, bd)| d < bd) {
                best = Some((nid, d));
            }
        }

        let new_id = match best {
            Some((nid, _)) => {
                remapped += 1;
                nid
            }
            None => {
                let synthetic = -*old_id;
                conn.execute(
                    "INSERT OR IGNORE INTO tracks \
                     (lipas_id, name, lat, lon, type_code, status, city, suburb, last_synced_at) \
                     VALUES (?1, ?2, ?3, ?4, 0, 'legacy', ?5, ?6, ?7)",
                    params![synthetic, name, lat, lon, city, suburb, now],
                )
                .map_err(|e| e.to_string())?;
                let nid: i64 = conn
                    .query_row(
                        "SELECT id FROM tracks WHERE lipas_id = ?1",
                        params![synthetic],
                        |row| row.get(0),
                    )
                    .map_err(|e| e.to_string())?;
                orphaned += 1;
                nid
            }
        };

        conn.execute(
            "UPDATE runs SET track_id = ?1 WHERE track_id = ?2",
            params![new_id, *old_id],
        )
        .map_err(|e| e.to_string())?;
    }

    conn.execute_batch("DROP TABLE tracks_legacy")
        .map_err(|e| e.to_string())?;

    println!(
        "Migration finalized: {} legacy tracks with runs matched to Lipas, \
         {} orphan placeholders preserved (legacy tracks without runs were dropped)",
        remapped, orphaned
    );
    Ok((remapped, orphaned))
}

pub fn tracks_count(db: &Db) -> i64 {
    let conn = db.lock().unwrap();
    conn.query_row("SELECT COUNT(*) FROM tracks", [], |row| row.get(0))
        .unwrap_or(0)
}

// --- Track queries ---

fn row_to_track(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<Track> {
    Ok(Track {
        id: row.get(0)?,
        lipas_id: row.get(1)?,
        name: row.get(2)?,
        lat: row.get(3)?,
        lon: row.get(4)?,
        city: row.get(5)?,
        suburb: row.get(6)?,
        address: row.get(7)?,
        postal_code: row.get(8)?,
        surface: row.get(9)?,
        track_length_m: row.get(10)?,
        lanes: row.get(11)?,
        status: row.get(12)?,
        type_code: row.get(13)?,
    })
}

pub fn list_tracks(
    db: &Db,
    lat: Option<f64>,
    lon: Option<f64>,
    q: Option<&str>,
    user_id: Option<i64>,
) -> Result<Vec<TrackWithDistance>, String> {
    let conn = db.lock().unwrap();

    let user_param: rusqlite::types::Value = match user_id {
        Some(uid) => uid.into(),
        None => rusqlite::types::Value::Null,
    };

    let (sql, params_vec): (String, Vec<rusqlite::types::Value>) =
        if let Some(q_str) = q.filter(|s| !s.is_empty()) {
            let pattern = format!("%{}%", q_str);
            (
                format!(
                    "SELECT {}, MIN(r.time_seconds), \
                            MAX(CASE WHEN f.id IS NOT NULL THEN 1 ELSE 0 END) \
                     FROM tracks t \
                     LEFT JOIN runs r ON r.track_id = t.id \
                     LEFT JOIN favorites f ON f.track_id = t.id AND f.user_id = ?1 \
                     WHERE LOWER(t.name) LIKE LOWER(?2) OR LOWER(t.city) LIKE LOWER(?2) \
                        OR LOWER(t.suburb) LIKE LOWER(?2) \
                     GROUP BY t.id ORDER BY t.name",
                    TRACK_COLUMNS
                ),
                vec![user_param, pattern.into()],
            )
        } else {
            (
                format!(
                    "SELECT {}, MIN(r.time_seconds), \
                            MAX(CASE WHEN f.id IS NOT NULL THEN 1 ELSE 0 END) \
                     FROM tracks t \
                     LEFT JOIN runs r ON r.track_id = t.id \
                     LEFT JOIN favorites f ON f.track_id = t.id AND f.user_id = ?1 \
                     GROUP BY t.id ORDER BY t.name",
                    TRACK_COLUMNS
                ),
                vec![user_param],
            )
        };

    let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
    let params_refs: Vec<&dyn rusqlite::ToSql> =
        params_vec.iter().map(|v| v as &dyn rusqlite::ToSql).collect();
    let tracks: Vec<(Track, Option<f64>, bool)> = stmt
        .query_map(params_refs.as_slice(), |row| {
            let track = row_to_track(row)?;
            let record: Option<f64> = row.get(14)?;
            let is_favorite: i64 = row.get(15)?;
            Ok((track, record, is_favorite != 0))
        })
        .map_err(|e| e.to_string())?
        .filter_map(|r| r.ok())
        .collect();

    let mut result: Vec<TrackWithDistance> = tracks
        .into_iter()
        .map(|(track, record, is_favorite)| {
            let distance_m = lat.zip(lon).map(|(ulat, ulon)| {
                Point::new(ulon, ulat).haversine_distance(&Point::new(track.lon, track.lat))
            });
            TrackWithDistance { track, distance_m, record, is_favorite }
        })
        .collect();

    // Favorites bubble to the top; within each group, sort by distance (when known)
    // or fall back to the SQL-provided name order.
    result.sort_by(|a, b| {
        b.is_favorite.cmp(&a.is_favorite).then_with(|| {
            if lat.is_some() {
                match (a.distance_m, b.distance_m) {
                    (Some(da), Some(db)) => {
                        da.partial_cmp(&db).unwrap_or(std::cmp::Ordering::Equal)
                    }
                    (Some(_), None) => std::cmp::Ordering::Less,
                    (None, Some(_)) => std::cmp::Ordering::Greater,
                    (None, None) => a.track.name.cmp(&b.track.name),
                }
            } else {
                std::cmp::Ordering::Equal
            }
        })
    });

    Ok(result)
}

pub fn get_track(
    db: &Db,
    id: i64,
    user_id: Option<i64>,
) -> Result<Option<TrackWithDistance>, String> {
    let conn = db.lock().unwrap();
    let user_param: rusqlite::types::Value = match user_id {
        Some(uid) => uid.into(),
        None => rusqlite::types::Value::Null,
    };
    let sql = format!(
        "SELECT {}, MIN(r.time_seconds), \
                MAX(CASE WHEN f.id IS NOT NULL THEN 1 ELSE 0 END) \
         FROM tracks t \
         LEFT JOIN runs r ON r.track_id = t.id \
         LEFT JOIN favorites f ON f.track_id = t.id AND f.user_id = ?1 \
         WHERE t.id = ?2 GROUP BY t.id",
        TRACK_COLUMNS
    );
    let result = conn.query_row(&sql, params![user_param, id], |row| {
        let track = row_to_track(row)?;
        let record: Option<f64> = row.get(14)?;
        let is_favorite: i64 = row.get(15)?;
        Ok((track, record, is_favorite != 0))
    });

    match result {
        Ok((track, record, is_favorite)) => Ok(Some(TrackWithDistance {
            track,
            distance_m: None,
            record,
            is_favorite,
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

    let sql = format!(
        "SELECT {} FROM tracks t WHERE t.id = ?1",
        TRACK_COLUMNS
    );
    let track = conn
        .query_row(&sql, params![track_id], row_to_track)
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

// --- Favorites ---

pub fn add_favorite(db: &Db, user_id: i64, track_id: i64) -> Result<(), String> {
    let conn = db.lock().unwrap();
    conn.execute(
        "INSERT OR IGNORE INTO favorites (user_id, track_id, created_at) \
         VALUES (?1, ?2, ?3)",
        params![user_id, track_id, Utc::now().to_rfc3339()],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn remove_favorite(db: &Db, user_id: i64, track_id: i64) -> Result<(), String> {
    let conn = db.lock().unwrap();
    conn.execute(
        "DELETE FROM favorites WHERE user_id = ?1 AND track_id = ?2",
        params![user_id, track_id],
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

#[cfg(test)]
mod migration_tests {
    use super::*;

    fn open_legacy_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE tracks (
                 id INTEGER PRIMARY KEY AUTOINCREMENT,
                 osm_id TEXT UNIQUE NOT NULL,
                 name TEXT,
                 lat REAL NOT NULL,
                 lon REAL NOT NULL,
                 city TEXT,
                 suburb TEXT
             );
             CREATE TABLE users (
                 id INTEGER PRIMARY KEY AUTOINCREMENT,
                 email TEXT UNIQUE NOT NULL,
                 display_name TEXT NOT NULL,
                 password_hash TEXT NOT NULL,
                 created_at TEXT NOT NULL
             );
             CREATE TABLE runs (
                 id INTEGER PRIMARY KEY AUTOINCREMENT,
                 user_id INTEGER NOT NULL REFERENCES users(id),
                 track_id INTEGER NOT NULL REFERENCES tracks(id),
                 time_seconds REAL NOT NULL,
                 logged_at TEXT NOT NULL
             );",
        )
        .unwrap();
        conn
    }

    #[test]
    fn legacy_run_is_remapped_to_nearby_new_track() {
        let conn = open_legacy_db();
        // Legacy track at Helsinki Olympiastadion-ish coords.
        conn.execute(
            "INSERT INTO tracks (osm_id, name, lat, lon, city) VALUES (?1, ?2, ?3, ?4, ?5)",
            params!["way/1", "Olympiastadion", 60.1875, 24.9275, "Helsinki"],
        )
        .unwrap();
        let old_track_id: i64 = conn
            .query_row("SELECT id FROM tracks WHERE osm_id = 'way/1'", [], |r| r.get(0))
            .unwrap();
        conn.execute(
            "INSERT INTO users (email, display_name, password_hash, created_at) VALUES \
             ('a@b.c', 'A', 'h', '2024-01-01')",
            [],
        )
        .unwrap();
        let user_id: i64 = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO runs (user_id, track_id, time_seconds, logged_at) VALUES (?1, ?2, 65.5, '2024-01-01')",
            params![user_id, old_track_id],
        )
        .unwrap();

        migrate_db(&conn);
        init_db(&conn).unwrap();

        // Simulate Lipas fetch: insert a new track within 400 m of the legacy one.
        conn.execute(
            "INSERT INTO tracks (lipas_id, name, lat, lon, type_code, status, last_synced_at) \
             VALUES (501, 'Olympiastadion (Lipas)', 60.1880, 24.9270, 1220, 'active', '2026-05-02')",
            [],
        )
        .unwrap();
        let new_track_id: i64 = conn
            .query_row("SELECT id FROM tracks WHERE lipas_id = 501", [], |r| r.get(0))
            .unwrap();

        let db: Db = Arc::new(Mutex::new(conn));
        let (remapped, orphaned) = finalize_legacy_migration(&db).unwrap();
        assert_eq!(remapped, 1);
        assert_eq!(orphaned, 0);

        let conn = db.lock().unwrap();
        let mapped: i64 = conn
            .query_row("SELECT track_id FROM runs", [], |r| r.get(0))
            .unwrap();
        assert_eq!(mapped, new_track_id);
        // tracks_legacy is dropped after finalize.
        let exists: bool = conn
            .query_row(
                "SELECT 1 FROM sqlite_master WHERE type='table' AND name='tracks_legacy'",
                [],
                |_| Ok(()),
            )
            .is_ok();
        assert!(!exists);
    }

    #[test]
    fn legacy_run_with_no_nearby_match_becomes_orphan_placeholder() {
        let conn = open_legacy_db();
        conn.execute(
            "INSERT INTO tracks (osm_id, name, lat, lon, city) VALUES \
             ('way/9', 'Erämaakenttä', 67.9, 25.5, 'Inari')",
            [],
        )
        .unwrap();
        let old_track_id: i64 = conn
            .query_row("SELECT id FROM tracks WHERE osm_id = 'way/9'", [], |r| r.get(0))
            .unwrap();
        conn.execute(
            "INSERT INTO users (email, display_name, password_hash, created_at) VALUES \
             ('a@b.c', 'A', 'h', '2024-01-01')",
            [],
        )
        .unwrap();
        let user_id: i64 = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO runs (user_id, track_id, time_seconds, logged_at) VALUES (?1, ?2, 90.0, '2024-01-01')",
            params![user_id, old_track_id],
        )
        .unwrap();

        migrate_db(&conn);
        init_db(&conn).unwrap();

        // No nearby Lipas track — only one far away.
        conn.execute(
            "INSERT INTO tracks (lipas_id, name, lat, lon, type_code, status, last_synced_at) \
             VALUES (700, 'Helsinki', 60.18, 24.93, 1220, 'active', '2026-05-02')",
            [],
        )
        .unwrap();

        let db: Db = Arc::new(Mutex::new(conn));
        let (remapped, orphaned) = finalize_legacy_migration(&db).unwrap();
        assert_eq!(remapped, 0);
        assert_eq!(orphaned, 1);

        let conn = db.lock().unwrap();
        // Run still has a valid (non-zero) track_id pointing at the orphan placeholder.
        let mapped: i64 = conn
            .query_row("SELECT track_id FROM runs", [], |r| r.get(0))
            .unwrap();
        let placeholder_status: String = conn
            .query_row("SELECT status FROM tracks WHERE id = ?1", params![mapped], |r| r.get(0))
            .unwrap();
        assert_eq!(placeholder_status, "legacy");
    }
}

#[cfg(test)]
mod favorites_tests {
    use super::*;

    fn setup() -> (Db, i64, i64, i64) {
        let conn = Connection::open_in_memory().unwrap();
        init_db(&conn).unwrap();
        conn.execute(
            "INSERT INTO users (email, display_name, password_hash, created_at) \
             VALUES ('u@e.c', 'U', 'h', '2026-05-05')",
            [],
        )
        .unwrap();
        let user_id = conn.last_insert_rowid();
        // Track A — favorited; Track B — not.
        conn.execute(
            "INSERT INTO tracks (lipas_id, name, lat, lon, type_code, status, last_synced_at) \
             VALUES (1, 'A-rata', 60.0, 24.0, 1220, 'active', '2026-05-05')",
            [],
        )
        .unwrap();
        let track_a = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO tracks (lipas_id, name, lat, lon, type_code, status, last_synced_at) \
             VALUES (2, 'B-rata', 60.1, 24.1, 1220, 'active', '2026-05-05')",
            [],
        )
        .unwrap();
        let track_b = conn.last_insert_rowid();
        let db: Db = Arc::new(Mutex::new(conn));
        (db, user_id, track_a, track_b)
    }

    #[test]
    fn add_favorite_is_idempotent() {
        let (db, user_id, track_a, _) = setup();
        add_favorite(&db, user_id, track_a).unwrap();
        add_favorite(&db, user_id, track_a).unwrap();
        let count: i64 = db
            .lock()
            .unwrap()
            .query_row(
                "SELECT COUNT(*) FROM favorites WHERE user_id = ?1 AND track_id = ?2",
                params![user_id, track_a],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn remove_favorite_is_noop_when_not_favorited() {
        let (db, user_id, track_a, _) = setup();
        // No favorite yet — must not error.
        remove_favorite(&db, user_id, track_a).unwrap();
        let count: i64 = db
            .lock()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM favorites", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn list_tracks_marks_and_orders_favorites_first_when_user_logged_in() {
        let (db, user_id, _track_a, track_b) = setup();
        // Favorite the *second* track (B-rata) so it must surface above the alphabetically-
        // earlier A-rata when the user is logged in.
        add_favorite(&db, user_id, track_b).unwrap();

        let result = list_tracks(&db, None, None, None, Some(user_id)).unwrap();
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].track.id, track_b);
        assert!(result[0].is_favorite);
        assert!(!result[1].is_favorite);
    }

    #[test]
    fn list_tracks_returns_is_favorite_false_when_no_user() {
        let (db, user_id, _track_a, track_b) = setup();
        add_favorite(&db, user_id, track_b).unwrap();

        let result = list_tracks(&db, None, None, None, None).unwrap();
        assert!(result.iter().all(|t| !t.is_favorite));
        // With no user, fall back to the SQL-provided alphabetical order: A before B.
        assert_eq!(result[0].track.name.as_deref(), Some("A-rata"));
    }

    #[test]
    fn get_track_returns_is_favorite_for_logged_in_user() {
        let (db, user_id, track_a, _track_b) = setup();
        add_favorite(&db, user_id, track_a).unwrap();

        let with_user = get_track(&db, track_a, Some(user_id)).unwrap().unwrap();
        assert!(with_user.is_favorite);

        let without_user = get_track(&db, track_a, None).unwrap().unwrap();
        assert!(!without_user.is_favorite);
    }
}
