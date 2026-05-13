use argon2::{
    password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
    Argon2,
};
use rand_core::OsRng;
use chrono::{Datelike, Utc};
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
    #[serde(flatten)]
    pub period_info: PeriodInfo,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct LeaderboardEntry {
    pub rank: i64,
    pub user_id: i64,
    pub display_name: String,
    pub time_seconds: f64,
    pub logged_at: String,
    pub track_id: i64,
    pub track_name: Option<String>,
    pub track_city: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct PersonalBestEntry {
    pub time_seconds: f64,
    pub logged_at: String,
    pub track_id: i64,
    pub track_name: Option<String>,
    pub track_city: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Leaderboard {
    pub entries: Vec<LeaderboardEntry>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub personal_best: Option<PersonalBestEntry>,
    #[serde(flatten)]
    pub period_info: PeriodInfo,
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

// --- Period filter ---
//
// `runs.logged_at` is stored as RFC3339 UTC (`Utc::now().to_rfc3339()`), so
// lexicographic comparison matches chronological order. Range filters are
// half-open [start, end) and use the same string format as the inserted rows.

pub const LEADERBOARD_DEFAULT_LIMIT: u32 = 25;
pub const LEADERBOARD_MAX_LIMIT: u32 = 100;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Period {
    All,
    Range { start: String, end: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeriodInfo {
    pub period: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub month: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub year: Option<String>,
    // Echoed back when a `category=` filter was applied. Format: "M40", "N55" etc.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
}

// --- WMA age category ---
//
// Bands: M30..M90 and N30..N90 in 5-year increments (World Masters Athletics
// convention; we use Finnish 'N' for women internally to match the gender
// stored on `users.gender`). Age is computed as `ref_year - birth_year`,
// snapshot to the current UTC year — i.e. the leaderboard shows users who
// are currently in the requested band, independent of when they ran.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgeCategory {
    pub gender: char, // 'M' | 'N'
    pub band: u32,    // 30, 35, ..., 90
}

impl AgeCategory {
    pub fn as_code(&self) -> String {
        format!("{}{}", self.gender, self.band)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CategoryFilter {
    pub min_birth_year: i32,
    pub max_birth_year: i32,
    pub gender: char,
}

pub fn parse_age_category(s: &str) -> Result<AgeCategory, String> {
    if s.len() < 2 {
        return Err(format!("Tuntematon ikäluokka: {}", s));
    }
    let first = s.as_bytes()[0] as char;
    let gender = match first.to_ascii_uppercase() {
        'M' => 'M',
        'W' | 'N' | 'F' => 'N',
        _ => return Err(format!("Tuntematon sukupuoli ikäluokassa: {}", s)),
    };
    let band: u32 = s[1..]
        .parse()
        .map_err(|_| format!("Virheellinen ikäluokka: {}", s))?;
    if !(30..=90).contains(&band) || band % 5 != 0 {
        return Err(format!("Ikäluokka {} ei ole WMA-välillä (30..90, 5 v välein)", band));
    }
    Ok(AgeCategory { gender, band })
}

pub fn category_filter(cat: &AgeCategory, ref_year: i32) -> CategoryFilter {
    // age = ref_year - birth_year. age ∈ [band, band+5) ⇒
    //   birth_year ∈ [ref_year - band - 4, ref_year - band]
    let max_birth_year = ref_year - cat.band as i32;
    let min_birth_year = max_birth_year - 4;
    CategoryFilter { min_birth_year, max_birth_year, gender: cat.gender }
}

// Convenience: parse + compute filter using the current UTC year, if a value was provided.
pub fn resolve_age_category(
    raw: Option<&str>,
) -> Result<Option<(AgeCategory, CategoryFilter)>, String> {
    match raw.map(str::trim).filter(|s| !s.is_empty()) {
        None => Ok(None),
        Some(s) => {
            let cat = parse_age_category(s)?;
            let filter = category_filter(&cat, Utc::now().year());
            Ok(Some((cat, filter)))
        }
    }
}

pub fn resolve_period(
    period: Option<&str>,
    month: Option<&str>,
    year: Option<&str>,
) -> Result<(Period, PeriodInfo), String> {
    match period.unwrap_or("all") {
        "all" => Ok((
            Period::All,
            PeriodInfo { period: "all".to_string(), month: None, year: None, category: None },
        )),
        "month" => {
            let (y, m) = match month {
                Some(s) => parse_year_month(s)?,
                None => {
                    let now = Utc::now();
                    (now.year(), now.month() as i32)
                }
            };
            let (ny, nm) = if m == 12 { (y + 1, 1) } else { (y, m + 1) };
            Ok((
                Period::Range {
                    start: format!("{:04}-{:02}-01T00:00:00+00:00", y, m),
                    end:   format!("{:04}-{:02}-01T00:00:00+00:00", ny, nm),
                },
                PeriodInfo {
                    period: "month".to_string(),
                    month: Some(format!("{:04}-{:02}", y, m)),
                    year: None,
                    category: None,
                },
            ))
        }
        "year" => {
            let y: i32 = match year {
                Some(s) => s.parse().map_err(|_| format!("Virheellinen vuosi: {}", s))?,
                None => Utc::now().year(),
            };
            if !(1900..=2100).contains(&y) {
                return Err(format!("Vuosi alueen ulkopuolella: {}", y));
            }
            Ok((
                Period::Range {
                    start: format!("{:04}-01-01T00:00:00+00:00", y),
                    end:   format!("{:04}-01-01T00:00:00+00:00", y + 1),
                },
                PeriodInfo {
                    period: "year".to_string(),
                    month: None,
                    year: Some(format!("{:04}", y)),
                    category: None,
                },
            ))
        }
        other => Err(format!("Tuntematon period: {}", other)),
    }
}

fn parse_year_month(s: &str) -> Result<(i32, i32), String> {
    let mut parts = s.split('-');
    let y_str = parts.next().ok_or_else(|| format!("Virheellinen kuukausi: {}", s))?;
    let m_str = parts.next().ok_or_else(|| format!("Virheellinen kuukausi: {}", s))?;
    if parts.next().is_some() {
        return Err(format!("Virheellinen kuukausi: {}", s));
    }
    let y: i32 = y_str.parse().map_err(|_| format!("Virheellinen vuosi: {}", y_str))?;
    let m: i32 = m_str.parse().map_err(|_| format!("Virheellinen kuukausi: {}", m_str))?;
    if !(1..=12).contains(&m) {
        return Err(format!("Kuukausi alueen ulkopuolella: {}", m));
    }
    if !(1900..=2100).contains(&y) {
        return Err(format!("Vuosi alueen ulkopuolella: {}", y));
    }
    Ok((y, m))
}

pub fn clamp_limit(requested: Option<u32>) -> u32 {
    requested.unwrap_or(LEADERBOARD_DEFAULT_LIMIT).clamp(1, LEADERBOARD_MAX_LIMIT)
}

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
         );

         CREATE INDEX IF NOT EXISTS idx_runs_track_logged ON runs(track_id, logged_at);
         CREATE INDEX IF NOT EXISTS idx_runs_user_time    ON runs(user_id, time_seconds);
         CREATE INDEX IF NOT EXISTS idx_runs_logged_at    ON runs(logged_at);

         CREATE TABLE IF NOT EXISTS finnish_records (
             id           INTEGER PRIMARY KEY AUTOINCREMENT,
             category     TEXT    UNIQUE NOT NULL,
             time_seconds REAL    NOT NULL,
             holder_name  TEXT,
             set_year     INTEGER,
             set_location TEXT,
             notes        TEXT,
             updated_at   TEXT    NOT NULL
         );",
    )?;

    // Phase 2 column additions on `users`. Idempotent: skipped when the columns
    // already exist. Both nullable — gender is validated at the app layer
    // ('M' | 'F' | NULL) so CHECK constraints don't have to be rewritten later.
    if !column_exists(conn, "users", "birth_year") {
        conn.execute_batch(
            "ALTER TABLE users ADD COLUMN birth_year INTEGER;
             ALTER TABLE users ADD COLUMN gender TEXT;",
        )?;
    }

    seed_finnish_records(conn)?;
    Ok(())
}

// Seed the curated open-class Finnish 400 m records. INSERT OR IGNORE keeps
// hand-edits from an admin (or a future curated-CSV ingest) intact across
// restarts. Masters / N-band rows are intentionally left for a follow-up
// import — we couldn't identify a public SUL/Tilastopaja API at the time of
// writing, and only the two open records are sourceable from Wikipedia.
fn seed_finnish_records(conn: &Connection) -> rusqlite::Result<()> {
    let now = Utc::now().to_rfc3339();
    let seeds = [
        ("OPEN_M", 45.49_f64, "Markku Kukkoaho", 1972_i64, "München (OK)"),
        ("OPEN_N", 50.14_f64, "Riitta Salin",    1974_i64, "Rooma (EM)"),
    ];
    for (cat, t, name, year, loc) in seeds {
        conn.execute(
            "INSERT OR IGNORE INTO finnish_records \
             (category, time_seconds, holder_name, set_year, set_location, updated_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![cat, t, name, year, loc, now],
        )?;
    }
    Ok(())
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
    period: &Period,
    period_info: PeriodInfo,
    category: Option<&CategoryFilter>,
    limit: u32,
) -> Result<TrackRecords, String> {
    use rusqlite::types::Value;
    let conn = db.lock().unwrap();

    let sql = format!(
        "SELECT {} FROM tracks t WHERE t.id = ?1",
        TRACK_COLUMNS
    );
    let track = conn
        .query_row(&sql, params![track_id], row_to_track)
        .map_err(|e| e.to_string())?;

    let mut where_parts: Vec<&'static str> = vec!["r.track_id = ?"];
    let mut vals: Vec<Value> = vec![Value::Integer(track_id)];
    if let Period::Range { start, end } = period {
        where_parts.push("r.logged_at >= ?");
        where_parts.push("r.logged_at < ?");
        vals.push(Value::Text(start.clone()));
        vals.push(Value::Text(end.clone()));
    }
    if let Some(cat) = category {
        where_parts.push("u.birth_year >= ?");
        where_parts.push("u.birth_year <= ?");
        where_parts.push("u.gender = ?");
        vals.push(Value::Integer(cat.min_birth_year as i64));
        vals.push(Value::Integer(cat.max_birth_year as i64));
        vals.push(Value::Text(cat.gender.to_string()));
    }
    let sql_records = format!(
        "SELECT u.display_name, r.time_seconds, r.logged_at \
         FROM runs r JOIN users u ON u.id = r.user_id \
         WHERE {} \
         ORDER BY r.time_seconds ASC, r.logged_at ASC LIMIT {}",
        where_parts.join(" AND "),
        limit
    );

    let mut stmt = conn.prepare(&sql_records).map_err(|e| e.to_string())?;
    let records_iter: Vec<(String, f64, String)> = stmt
        .query_map(rusqlite::params_from_iter(vals.iter()), |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, f64>(1)?, row.get::<_, String>(2)?))
        })
        .map_err(|e| e.to_string())?
        .filter_map(|r| r.ok())
        .collect();

    let records: Vec<RecordEntry> = records_iter
        .into_iter()
        .enumerate()
        .map(|(i, (display_name, time_seconds, logged_at))| RecordEntry {
            rank: (i + 1) as i64,
            display_name,
            time_seconds,
            logged_at,
        })
        .collect();

    // personal_best is all-time (matches Strava segment PR semantics), independent of period.
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

    Ok(TrackRecords { track, records, personal_best, period_info })
}

// Cross-track leaderboard: best single run per user (Strava-style "fastest 400 m" board).
// Period filter narrows the ranking window; category filter restricts which users appear.
pub fn get_leaderboard(
    db: &Db,
    user_id: Option<i64>,
    period: &Period,
    period_info: PeriodInfo,
    category: Option<&CategoryFilter>,
    limit: u32,
) -> Result<Leaderboard, String> {
    use rusqlite::types::Value;
    let conn = db.lock().unwrap();

    // The inner subquery picks each user's best run in the period; the outer WHERE
    // re-applies the period filter so users whose best falls outside the window
    // don't leak in via the join. Category filtering applies only to the outer query
    // since the inner is already correlated by user_id.
    let (inner_period_clause, outer_period_clause, mut period_vals): (&str, &str, Vec<Value>) =
        match period {
            Period::All => ("", "", vec![]),
            Period::Range { start, end } => (
                " AND r2.logged_at >= ? AND r2.logged_at < ?",
                " AND r.logged_at >= ? AND r.logged_at < ?",
                vec![
                    Value::Text(start.clone()),
                    Value::Text(end.clone()),
                    Value::Text(start.clone()),
                    Value::Text(end.clone()),
                ],
            ),
        };

    let (category_clause, category_vals): (String, Vec<Value>) = match category {
        None => (String::new(), vec![]),
        Some(cat) => (
            " AND u.birth_year >= ? AND u.birth_year <= ? AND u.gender = ?".to_string(),
            vec![
                Value::Integer(cat.min_birth_year as i64),
                Value::Integer(cat.max_birth_year as i64),
                Value::Text(cat.gender.to_string()),
            ],
        ),
    };

    let sql = format!(
        "SELECT u.id, u.display_name, r.time_seconds, r.logged_at, \
                r.track_id, t.name, t.city \
         FROM runs r \
         JOIN users u  ON u.id = r.user_id \
         JOIN tracks t ON t.id = r.track_id \
         WHERE r.id = ( \
             SELECT r2.id FROM runs r2 \
             WHERE r2.user_id = r.user_id{} \
             ORDER BY r2.time_seconds ASC, r2.logged_at ASC LIMIT 1 \
         ){}{} \
         ORDER BY r.time_seconds ASC, r.logged_at ASC LIMIT {}",
        inner_period_clause, outer_period_clause, category_clause, limit
    );

    let mut vals: Vec<Value> = Vec::new();
    vals.append(&mut period_vals);
    vals.extend(category_vals);

    let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
    let rows: Vec<(i64, String, f64, String, i64, Option<String>, Option<String>)> = stmt
        .query_map(rusqlite::params_from_iter(vals.iter()), |row| {
            Ok((
                row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?,
                row.get(4)?, row.get(5)?, row.get(6)?,
            ))
        })
        .map_err(|e| e.to_string())?
        .filter_map(|r| r.ok())
        .collect();

    let entries: Vec<LeaderboardEntry> = rows
        .into_iter()
        .enumerate()
        .map(|(i, (uid, display_name, time_seconds, logged_at, track_id, track_name, track_city))| {
            LeaderboardEntry {
                rank: (i + 1) as i64,
                user_id: uid,
                display_name,
                time_seconds,
                logged_at,
                track_id,
                track_name,
                track_city,
            }
        })
        .collect();

    let personal_best = if let Some(uid) = user_id {
        let (pb_sql, pb_params): (&str, Vec<&dyn rusqlite::ToSql>) = match period {
            Period::All => (
                "SELECT r.time_seconds, r.logged_at, r.track_id, t.name, t.city \
                 FROM runs r JOIN tracks t ON t.id = r.track_id \
                 WHERE r.user_id = ?1 \
                 ORDER BY r.time_seconds ASC, r.logged_at ASC LIMIT 1",
                vec![&uid as &dyn rusqlite::ToSql],
            ),
            Period::Range { start, end } => (
                "SELECT r.time_seconds, r.logged_at, r.track_id, t.name, t.city \
                 FROM runs r JOIN tracks t ON t.id = r.track_id \
                 WHERE r.user_id = ?1 AND r.logged_at >= ?2 AND r.logged_at < ?3 \
                 ORDER BY r.time_seconds ASC, r.logged_at ASC LIMIT 1",
                vec![&uid as &dyn rusqlite::ToSql,
                     start as &dyn rusqlite::ToSql,
                     end as &dyn rusqlite::ToSql],
            ),
        };
        conn.query_row(
            pb_sql,
            rusqlite::params_from_iter(pb_params.iter()),
            |row| {
                Ok(PersonalBestEntry {
                    time_seconds: row.get(0)?,
                    logged_at: row.get(1)?,
                    track_id: row.get(2)?,
                    track_name: row.get(3)?,
                    track_city: row.get(4)?,
                })
            },
        )
        .ok()
    } else {
        None
    };

    Ok(Leaderboard { entries, personal_best, period_info })
}

// --- Finnish records (Phase 3) ---
//
// Reference data: national / masters records over 400 m. The seed only carries
// the two open-class records (M, N); masters bands are added by a curator
// later. See README / PR description for the data-source caveat — no public
// SUL or Tilastopaja API exists at the time of writing.

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FinnishRecord {
    pub category: String,           // 'OPEN_M', 'OPEN_N', 'M40', 'N55', ...
    pub time_seconds: f64,
    pub holder_name: Option<String>,
    pub set_year: Option<i64>,
    pub set_location: Option<String>,
    pub notes: Option<String>,
    pub updated_at: String,
}

pub fn list_finnish_records(db: &Db) -> Result<Vec<FinnishRecord>, String> {
    let conn = db.lock().unwrap();
    let mut stmt = conn
        .prepare(
            "SELECT category, time_seconds, holder_name, set_year, set_location, notes, updated_at \
             FROM finnish_records \
             ORDER BY \
                 CASE category WHEN 'OPEN_M' THEN 0 WHEN 'OPEN_N' THEN 1 ELSE 2 END, \
                 category",
        )
        .map_err(|e| e.to_string())?;
    let rows: Vec<FinnishRecord> = stmt
        .query_map([], |row| {
            Ok(FinnishRecord {
                category:     row.get(0)?,
                time_seconds: row.get(1)?,
                holder_name:  row.get(2)?,
                set_year:     row.get(3)?,
                set_location: row.get(4)?,
                notes:        row.get(5)?,
                updated_at:   row.get(6)?,
            })
        })
        .map_err(|e| e.to_string())?
        .filter_map(|r| r.ok())
        .collect();
    Ok(rows)
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

// --- User profile (Phase 2) ---

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserProfile {
    pub user_id: i64,
    pub email: String,
    pub display_name: String,
    pub birth_year: Option<i32>,
    pub gender: Option<String>,
}

pub fn get_user_profile(db: &Db, user_id: i64) -> Result<UserProfile, String> {
    let conn = db.lock().unwrap();
    conn.query_row(
        "SELECT id, email, display_name, birth_year, gender FROM users WHERE id = ?1",
        params![user_id],
        |row| {
            Ok(UserProfile {
                user_id:      row.get(0)?,
                email:        row.get(1)?,
                display_name: row.get(2)?,
                birth_year:   row.get(3)?,
                gender:       row.get(4)?,
            })
        },
    )
    .map_err(|e| e.to_string())
}

// Full-replacement update of the editable profile fields. Pass `None` for
// birth_year/gender to clear them. display_name is always required (non-empty).
// The gender input accepts 'M' / 'N' / 'W' / 'F' (case-insensitive) and stores
// 'M' for men and 'N' for women (matching Finnish UI conventions).
pub fn update_user_profile(
    db: &Db,
    user_id: i64,
    display_name: &str,
    birth_year: Option<i32>,
    gender: Option<&str>,
) -> Result<UserProfile, String> {
    let display_name = display_name.trim();
    if display_name.is_empty() {
        return Err("Nimi ei voi olla tyhjä".to_string());
    }
    if display_name.chars().count() > 50 {
        return Err("Nimen enimmäispituus on 50 merkkiä".to_string());
    }
    if let Some(by) = birth_year {
        if !(1900..=2100).contains(&by) {
            return Err(format!("Syntymävuosi alueen ulkopuolella: {}", by));
        }
    }
    let gender_normalized: Option<String> = match gender.map(str::trim).filter(|s| !s.is_empty()) {
        None => None,
        Some(g) => match g.to_ascii_uppercase().as_str() {
            "M" => Some("M".to_string()),
            "N" | "W" | "F" => Some("N".to_string()),
            other => return Err(format!("Tuntematon sukupuoli: {}", other)),
        },
    };

    let conn = db.lock().unwrap();
    let changed = conn
        .execute(
            "UPDATE users SET display_name = ?1, birth_year = ?2, gender = ?3 WHERE id = ?4",
            params![display_name, birth_year, gender_normalized, user_id],
        )
        .map_err(|e| e.to_string())?;
    if changed == 0 {
        return Err("Käyttäjää ei löytynyt".to_string());
    }
    conn.query_row(
        "SELECT id, email, display_name, birth_year, gender FROM users WHERE id = ?1",
        params![user_id],
        |row| {
            Ok(UserProfile {
                user_id:      row.get(0)?,
                email:        row.get(1)?,
                display_name: row.get(2)?,
                birth_year:   row.get(3)?,
                gender:       row.get(4)?,
            })
        },
    )
    .map_err(|e| e.to_string())
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

pub fn make_jwt(user_id: i64, display_name: &str) -> Result<String, String> {
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

#[cfg(test)]
mod leaderboard_tests {
    use super::*;

    fn info_all() -> PeriodInfo {
        PeriodInfo { period: "all".to_string(), month: None, year: None, category: None }
    }

    fn insert_run(conn: &Connection, user_id: i64, track_id: i64, t: f64, ts: &str) {
        conn.execute(
            "INSERT INTO runs (user_id, track_id, time_seconds, logged_at) VALUES (?1, ?2, ?3, ?4)",
            params![user_id, track_id, t, ts],
        )
        .unwrap();
    }

    fn setup() -> (Db, i64, i64, i64, i64) {
        let conn = Connection::open_in_memory().unwrap();
        init_db(&conn).unwrap();
        conn.execute(
            "INSERT INTO users (email, display_name, password_hash, created_at) \
             VALUES ('a@e.c', 'Alice', 'h', '2026-01-01')",
            [],
        )
        .unwrap();
        let alice = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO users (email, display_name, password_hash, created_at) \
             VALUES ('b@e.c', 'Bob', 'h', '2026-01-01')",
            [],
        )
        .unwrap();
        let bob = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO tracks (lipas_id, name, lat, lon, type_code, status, last_synced_at, city) \
             VALUES (1, 'A-rata', 60.0, 24.0, 1220, 'active', '2026-05-05', 'Helsinki')",
            [],
        )
        .unwrap();
        let track_a = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO tracks (lipas_id, name, lat, lon, type_code, status, last_synced_at, city) \
             VALUES (2, 'B-rata', 60.1, 24.1, 1220, 'active', '2026-05-05', 'Espoo')",
            [],
        )
        .unwrap();
        let track_b = conn.last_insert_rowid();
        let db: Db = Arc::new(Mutex::new(conn));
        (db, alice, bob, track_a, track_b)
    }

    #[test]
    fn period_all_returns_records_ordered_by_time() {
        let (db, alice, bob, track_a, _) = setup();
        {
            let conn = db.lock().unwrap();
            insert_run(&conn, alice, track_a, 65.0, "2026-03-01T10:00:00+00:00");
            insert_run(&conn, bob,   track_a, 60.0, "2026-04-01T10:00:00+00:00");
            insert_run(&conn, alice, track_a, 70.0, "2026-05-01T10:00:00+00:00");
        }
        let out = get_records(&db, track_a, None, &Period::All, info_all(), None, 10).unwrap();
        assert_eq!(out.records.len(), 3);
        assert_eq!(out.records[0].display_name, "Bob");
        assert_eq!(out.records[0].time_seconds, 60.0);
        assert_eq!(out.records[0].rank, 1);
        assert_eq!(out.records[2].time_seconds, 70.0);
    }

    #[test]
    fn period_month_filters_records_by_logged_at() {
        let (db, alice, bob, track_a, _) = setup();
        {
            let conn = db.lock().unwrap();
            insert_run(&conn, alice, track_a, 60.0, "2026-04-15T10:00:00+00:00");
            insert_run(&conn, bob,   track_a, 62.0, "2026-05-10T10:00:00+00:00");
            insert_run(&conn, alice, track_a, 64.0, "2026-05-20T10:00:00+00:00");
        }
        let (p, info) = resolve_period(Some("month"), Some("2026-05"), None).unwrap();
        let out = get_records(&db, track_a, None, &p, info, None, 10).unwrap();
        assert_eq!(out.records.len(), 2);
        assert!(out.records.iter().all(|r| r.logged_at.starts_with("2026-05")));
        assert_eq!(out.records[0].time_seconds, 62.0);
    }

    #[test]
    fn period_year_boundary_includes_dec_excludes_jan_next() {
        let (db, alice, _bob, track_a, _) = setup();
        {
            let conn = db.lock().unwrap();
            insert_run(&conn, alice, track_a, 60.0, "2025-12-31T23:59:59+00:00");
            insert_run(&conn, alice, track_a, 61.0, "2026-01-01T00:00:00+00:00");
        }
        let (p, info) = resolve_period(Some("year"), None, Some("2025")).unwrap();
        let out = get_records(&db, track_a, None, &p, info, None, 10).unwrap();
        assert_eq!(out.records.len(), 1);
        assert_eq!(out.records[0].time_seconds, 60.0);
    }

    #[test]
    fn records_tie_break_earlier_logged_at_wins() {
        let (db, alice, bob, track_a, _) = setup();
        {
            let conn = db.lock().unwrap();
            insert_run(&conn, bob,   track_a, 60.0, "2026-05-10T10:00:00+00:00");
            insert_run(&conn, alice, track_a, 60.0, "2026-05-01T10:00:00+00:00");
        }
        let out = get_records(&db, track_a, None, &Period::All, info_all(), None, 10).unwrap();
        assert_eq!(out.records[0].display_name, "Alice");
        assert_eq!(out.records[1].display_name, "Bob");
    }

    #[test]
    fn get_records_personal_best_is_all_time_not_period_scoped() {
        let (db, alice, _bob, track_a, _) = setup();
        {
            let conn = db.lock().unwrap();
            insert_run(&conn, alice, track_a, 55.0, "2025-08-01T10:00:00+00:00");
            insert_run(&conn, alice, track_a, 65.0, "2026-05-10T10:00:00+00:00");
        }
        let (p, info) = resolve_period(Some("month"), Some("2026-05"), None).unwrap();
        let out = get_records(&db, track_a, Some(alice), &p, info, None, 10).unwrap();
        assert_eq!(out.records.len(), 1);
        assert_eq!(out.personal_best, Some(55.0));
    }

    #[test]
    fn leaderboard_picks_best_run_per_user() {
        let (db, alice, bob, track_a, track_b) = setup();
        {
            let conn = db.lock().unwrap();
            insert_run(&conn, alice, track_a, 65.0, "2026-03-01T10:00:00+00:00");
            insert_run(&conn, alice, track_b, 60.0, "2026-04-01T10:00:00+00:00");
            insert_run(&conn, alice, track_a, 70.0, "2026-05-01T10:00:00+00:00");
            insert_run(&conn, bob,   track_a, 62.0, "2026-04-15T10:00:00+00:00");
        }
        let board = get_leaderboard(&db, None, &Period::All, info_all(), None, 25).unwrap();
        assert_eq!(board.entries.len(), 2);
        assert_eq!(board.entries[0].display_name, "Alice");
        assert_eq!(board.entries[0].time_seconds, 60.0);
        assert_eq!(board.entries[0].track_id, track_b);
        assert_eq!(board.entries[0].track_name.as_deref(), Some("B-rata"));
        assert_eq!(board.entries[1].display_name, "Bob");
        assert_eq!(board.entries[1].time_seconds, 62.0);
    }

    #[test]
    fn leaderboard_excludes_users_with_no_runs_in_period() {
        let (db, alice, bob, track_a, _) = setup();
        {
            let conn = db.lock().unwrap();
            insert_run(&conn, alice, track_a, 55.0, "2025-08-01T10:00:00+00:00");
            insert_run(&conn, bob,   track_a, 62.0, "2026-05-10T10:00:00+00:00");
        }
        let (p, info) = resolve_period(Some("month"), Some("2026-05"), None).unwrap();
        let board = get_leaderboard(&db, None, &p, info, None, 25).unwrap();
        assert_eq!(board.entries.len(), 1);
        assert_eq!(board.entries[0].display_name, "Bob");
    }

    #[test]
    fn leaderboard_personal_best_returned_when_authed_and_scoped_to_period() {
        let (db, alice, _bob, track_a, _) = setup();
        {
            let conn = db.lock().unwrap();
            insert_run(&conn, alice, track_a, 55.0, "2025-08-01T10:00:00+00:00");
            insert_run(&conn, alice, track_a, 65.0, "2026-05-10T10:00:00+00:00");
        }
        let (p, info) = resolve_period(Some("year"), None, Some("2026")).unwrap();
        let board = get_leaderboard(&db, Some(alice), &p, info, None, 25).unwrap();
        let pb = board.personal_best.unwrap();
        assert_eq!(pb.time_seconds, 65.0);

        let board_all = get_leaderboard(&db, Some(alice), &Period::All, info_all(), None, 25).unwrap();
        assert_eq!(board_all.personal_best.unwrap().time_seconds, 55.0);

        let board_anon = get_leaderboard(&db, None, &Period::All, info_all(), None, 25).unwrap();
        assert!(board_anon.personal_best.is_none());
    }

    #[test]
    fn resolve_period_defaults_to_all() {
        let (p, info) = resolve_period(None, None, None).unwrap();
        assert!(matches!(p, Period::All));
        assert_eq!(info.period, "all");
    }

    #[test]
    fn resolve_period_rejects_invalid_month_and_year() {
        assert!(resolve_period(Some("month"), Some("2026-13"), None).is_err());
        assert!(resolve_period(Some("month"), Some("not-a-month"), None).is_err());
        assert!(resolve_period(Some("year"), None, Some("1800")).is_err());
        assert!(resolve_period(Some("weekday"), None, None).is_err());
    }

    #[test]
    fn period_year_info_echoes_year_string() {
        let (_, info) = resolve_period(Some("year"), None, Some("2026")).unwrap();
        assert_eq!(info.year.as_deref(), Some("2026"));
        let (_, info_month) = resolve_period(Some("month"), Some("2026-05"), None).unwrap();
        assert_eq!(info_month.month.as_deref(), Some("2026-05"));
    }

    #[test]
    fn records_limit_clamped() {
        let (db, alice, _bob, track_a, _) = setup();
        {
            let conn = db.lock().unwrap();
            for i in 0..30 {
                insert_run(
                    &conn,
                    alice,
                    track_a,
                    60.0 + i as f64,
                    &format!("2026-05-{:02}T10:00:00+00:00", (i % 28) + 1),
                );
            }
        }
        let out = get_records(&db, track_a, None, &Period::All, info_all(), None, 5).unwrap();
        assert_eq!(out.records.len(), 5);
        assert_eq!(clamp_limit(Some(9999)), LEADERBOARD_MAX_LIMIT);
        assert_eq!(clamp_limit(Some(0)), 1);
        assert_eq!(clamp_limit(None), LEADERBOARD_DEFAULT_LIMIT);
    }
}

#[cfg(test)]
mod phase2_tests {
    use super::*;

    fn info_all() -> PeriodInfo {
        PeriodInfo { period: "all".to_string(), month: None, year: None, category: None }
    }

    fn insert_run(conn: &Connection, user_id: i64, track_id: i64, t: f64, ts: &str) {
        conn.execute(
            "INSERT INTO runs (user_id, track_id, time_seconds, logged_at) VALUES (?1, ?2, ?3, ?4)",
            params![user_id, track_id, t, ts],
        )
        .unwrap();
    }

    fn insert_user(
        conn: &Connection,
        email: &str,
        name: &str,
        birth_year: Option<i32>,
        gender: Option<&str>,
    ) -> i64 {
        conn.execute(
            "INSERT INTO users (email, display_name, password_hash, created_at, birth_year, gender) \
             VALUES (?1, ?2, 'h', '2026-01-01', ?3, ?4)",
            params![email, name, birth_year, gender],
        )
        .unwrap();
        conn.last_insert_rowid()
    }

    fn fresh_db() -> Db {
        let conn = Connection::open_in_memory().unwrap();
        init_db(&conn).unwrap();
        Arc::new(Mutex::new(conn))
    }

    fn fresh_db_with_track() -> (Db, i64) {
        let db = fresh_db();
        let track_id;
        {
            let conn = db.lock().unwrap();
            conn.execute(
                "INSERT INTO tracks (lipas_id, name, lat, lon, type_code, status, last_synced_at, city) \
                 VALUES (1, 'A-rata', 60.0, 24.0, 1220, 'active', '2026-05-05', 'Helsinki')",
                [],
            )
            .unwrap();
            track_id = conn.last_insert_rowid();
        }
        (db, track_id)
    }

    // --- parse_age_category / category_filter ---

    #[test]
    fn parse_age_category_accepts_m_and_n_bands() {
        assert_eq!(parse_age_category("M40").unwrap(), AgeCategory { gender: 'M', band: 40 });
        assert_eq!(parse_age_category("N55").unwrap(), AgeCategory { gender: 'N', band: 55 });
        // WMA-style 'W' alias maps to 'N' internally so we match users.gender='N'.
        assert_eq!(parse_age_category("W30").unwrap(), AgeCategory { gender: 'N', band: 30 });
        assert_eq!(parse_age_category("m65").unwrap(), AgeCategory { gender: 'M', band: 65 });
    }

    #[test]
    fn parse_age_category_rejects_out_of_range_or_garbled_input() {
        assert!(parse_age_category("M29").is_err());
        assert!(parse_age_category("M95").is_err());
        assert!(parse_age_category("M42").is_err()); // not a 5-year band
        assert!(parse_age_category("X40").is_err()); // unknown gender
        assert!(parse_age_category("").is_err());
        assert!(parse_age_category("M").is_err());
        assert!(parse_age_category("Mfoo").is_err());
    }

    #[test]
    fn category_filter_computes_5_year_birth_year_window() {
        // M40 in 2026 → ages 40..45 → born 1982..=1986
        let f = category_filter(&AgeCategory { gender: 'M', band: 40 }, 2026);
        assert_eq!(f.min_birth_year, 1982);
        assert_eq!(f.max_birth_year, 1986);
        assert_eq!(f.gender, 'M');
    }

    #[test]
    fn resolve_age_category_returns_none_for_empty_input() {
        assert!(resolve_age_category(None).unwrap().is_none());
        assert!(resolve_age_category(Some("")).unwrap().is_none());
        assert!(resolve_age_category(Some("   ")).unwrap().is_none());
    }

    // --- get_records / get_leaderboard with category filter ---

    #[test]
    fn get_records_category_filter_keeps_only_matching_users() {
        let (db, track_id) = fresh_db_with_track();
        let ref_year = Utc::now().year();
        let m40_birth = ref_year - 42; // squarely in M40
        let m50_birth = ref_year - 52;
        let (a_id, b_id);
        {
            let conn = db.lock().unwrap();
            a_id = insert_user(&conn, "a@e.c", "M40 Alice", Some(m40_birth), Some("M"));
            b_id = insert_user(&conn, "b@e.c", "M50 Bob",   Some(m50_birth), Some("M"));
            insert_run(&conn, a_id, track_id, 65.0, "2026-04-10T10:00:00+00:00");
            insert_run(&conn, b_id, track_id, 60.0, "2026-04-12T10:00:00+00:00");
        }
        let (cat, filter) = resolve_age_category(Some("M40")).unwrap().unwrap();
        assert_eq!(cat.band, 40);
        let mut info = info_all();
        info.category = Some(cat.as_code());
        let out = get_records(&db, track_id, None, &Period::All, info, Some(&filter), 25).unwrap();
        assert_eq!(out.records.len(), 1);
        assert_eq!(out.records[0].display_name, "M40 Alice");
    }

    #[test]
    fn get_records_category_filter_excludes_users_missing_profile_fields() {
        let (db, track_id) = fresh_db_with_track();
        let ref_year = Utc::now().year();
        let (with_profile, without_profile);
        {
            let conn = db.lock().unwrap();
            with_profile    = insert_user(&conn, "p@e.c", "Full", Some(ref_year - 42), Some("M"));
            without_profile = insert_user(&conn, "x@e.c", "Anon", None, None);
            insert_run(&conn, with_profile,    track_id, 60.0, "2026-04-10T10:00:00+00:00");
            insert_run(&conn, without_profile, track_id, 55.0, "2026-04-12T10:00:00+00:00");
        }
        let (_, filter) = resolve_age_category(Some("M40")).unwrap().unwrap();
        let out = get_records(&db, track_id, None, &Period::All, info_all(), Some(&filter), 25).unwrap();
        assert_eq!(out.records.len(), 1);
        assert_eq!(out.records[0].display_name, "Full");
    }

    #[test]
    fn get_leaderboard_category_filter_restricts_user_set() {
        let (db, track_id) = fresh_db_with_track();
        let ref_year = Utc::now().year();
        let (m40, n40);
        {
            let conn = db.lock().unwrap();
            m40 = insert_user(&conn, "m@e.c", "Mies",   Some(ref_year - 41), Some("M"));
            n40 = insert_user(&conn, "n@e.c", "Nainen", Some(ref_year - 41), Some("N"));
            insert_run(&conn, m40, track_id, 62.0, "2026-04-10T10:00:00+00:00");
            insert_run(&conn, n40, track_id, 58.0, "2026-04-11T10:00:00+00:00");
        }
        let (_, n40_filter) = resolve_age_category(Some("N40")).unwrap().unwrap();
        let board = get_leaderboard(&db, None, &Period::All, info_all(), Some(&n40_filter), 25).unwrap();
        assert_eq!(board.entries.len(), 1);
        assert_eq!(board.entries[0].display_name, "Nainen");
    }

    #[test]
    fn get_leaderboard_category_combines_with_period_filter() {
        let (db, track_id) = fresh_db_with_track();
        let ref_year = Utc::now().year();
        let m40_id;
        {
            let conn = db.lock().unwrap();
            m40_id = insert_user(&conn, "m@e.c", "M40", Some(ref_year - 42), Some("M"));
            insert_run(&conn, m40_id, track_id, 70.0, "2025-06-01T10:00:00+00:00");
            insert_run(&conn, m40_id, track_id, 65.0, "2026-04-10T10:00:00+00:00");
        }
        let (p, mut info) = resolve_period(Some("year"), None, Some("2026")).unwrap();
        let (_, filter) = resolve_age_category(Some("M40")).unwrap().unwrap();
        info.category = Some("M40".to_string());
        let board = get_leaderboard(&db, None, &p, info, Some(&filter), 25).unwrap();
        assert_eq!(board.entries.len(), 1);
        assert_eq!(board.entries[0].time_seconds, 65.0);
    }

    // --- get_user_profile / update_user_profile ---

    #[test]
    fn user_profile_round_trip_set_and_clear() {
        let db = fresh_db();
        let uid;
        {
            let conn = db.lock().unwrap();
            uid = insert_user(&conn, "u@e.c", "Uula", None, None);
        }
        let p0 = get_user_profile(&db, uid).unwrap();
        assert_eq!(p0.display_name, "Uula");
        assert!(p0.birth_year.is_none());
        assert!(p0.gender.is_none());

        // Set everything
        let p1 = update_user_profile(&db, uid, "Uula Uimari", Some(1984), Some("M")).unwrap();
        assert_eq!(p1.display_name, "Uula Uimari");
        assert_eq!(p1.birth_year, Some(1984));
        assert_eq!(p1.gender.as_deref(), Some("M"));

        // Clear birth_year + gender (display_name still required)
        let p2 = update_user_profile(&db, uid, "Uula Uimari", None, None).unwrap();
        assert!(p2.birth_year.is_none());
        assert!(p2.gender.is_none());
    }

    #[test]
    fn update_user_profile_normalises_gender_aliases() {
        let db = fresh_db();
        let uid = {
            let conn = db.lock().unwrap();
            insert_user(&conn, "g@e.c", "G", None, None)
        };
        let p = update_user_profile(&db, uid, "G", Some(1990), Some("W")).unwrap();
        assert_eq!(p.gender.as_deref(), Some("N")); // WMA 'W' → Finnish 'N'
        let p2 = update_user_profile(&db, uid, "G", Some(1990), Some("f")).unwrap();
        assert_eq!(p2.gender.as_deref(), Some("N"));
    }

    #[test]
    fn update_user_profile_rejects_invalid_inputs() {
        let db = fresh_db();
        let uid = {
            let conn = db.lock().unwrap();
            insert_user(&conn, "v@e.c", "V", None, None)
        };
        assert!(update_user_profile(&db, uid, "", None, None).is_err());
        assert!(update_user_profile(&db, uid, "   ", None, None).is_err());
        assert!(update_user_profile(&db, uid, "V", Some(1800), None).is_err());
        assert!(update_user_profile(&db, uid, "V", Some(2200), None).is_err());
        assert!(update_user_profile(&db, uid, "V", None, Some("X")).is_err());
    }
}

#[cfg(test)]
mod phase3_tests {
    use super::*;

    fn fresh_db() -> Db {
        let conn = Connection::open_in_memory().unwrap();
        init_db(&conn).unwrap();
        Arc::new(Mutex::new(conn))
    }

    #[test]
    fn init_db_seeds_open_class_records() {
        let db = fresh_db();
        let rows = list_finnish_records(&db).unwrap();
        let cats: Vec<&str> = rows.iter().map(|r| r.category.as_str()).collect();
        // The two seeded entries are present and OPEN_M sorts before OPEN_N.
        assert_eq!(cats, vec!["OPEN_M", "OPEN_N"]);

        let m = &rows[0];
        assert_eq!(m.time_seconds, 45.49);
        assert_eq!(m.holder_name.as_deref(), Some("Markku Kukkoaho"));
        assert_eq!(m.set_year, Some(1972));

        let n = &rows[1];
        assert_eq!(n.time_seconds, 50.14);
        assert_eq!(n.holder_name.as_deref(), Some("Riitta Salin"));
    }

    #[test]
    fn seed_does_not_overwrite_curator_edits() {
        let db = fresh_db();
        // Curator updates the men's open record.
        {
            let conn = db.lock().unwrap();
            conn.execute(
                "UPDATE finnish_records SET time_seconds = 45.10, holder_name = 'Test Curator' \
                 WHERE category = 'OPEN_M'",
                [],
            )
            .unwrap();
        }
        // Re-running init_db must NOT overwrite (INSERT OR IGNORE on the unique
        // `category` column is the guarantee we're relying on).
        {
            let conn = db.lock().unwrap();
            init_db(&conn).unwrap();
        }
        let rows = list_finnish_records(&db).unwrap();
        let m = rows.iter().find(|r| r.category == "OPEN_M").unwrap();
        assert_eq!(m.time_seconds, 45.10);
        assert_eq!(m.holder_name.as_deref(), Some("Test Curator"));
    }

    #[test]
    fn list_returns_masters_record_inserted_by_admin() {
        let db = fresh_db();
        {
            let conn = db.lock().unwrap();
            conn.execute(
                "INSERT INTO finnish_records \
                 (category, time_seconds, holder_name, set_year, set_location, updated_at) \
                 VALUES ('M40', 49.50, 'Aki Aikuinen', 2014, 'Lahti', '2026-05-13T00:00:00+00:00')",
                [],
            )
            .unwrap();
        }
        let rows = list_finnish_records(&db).unwrap();
        let m40 = rows.iter().find(|r| r.category == "M40").unwrap();
        assert_eq!(m40.time_seconds, 49.50);
        assert_eq!(m40.holder_name.as_deref(), Some("Aki Aikuinen"));
        assert_eq!(m40.set_year, Some(2014));
        // OPEN_M / OPEN_N must still sort first.
        assert_eq!(rows[0].category, "OPEN_M");
        assert_eq!(rows[1].category, "OPEN_N");
        assert_eq!(rows[2].category, "M40");
    }
}
