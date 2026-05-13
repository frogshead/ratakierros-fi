-- e2e fixture: schema + 3 hand-picked tracks.
-- Schema mirrors init_db() in api/src/lib.rs. CREATE IF NOT EXISTS keeps it
-- compatible with the API's own init_db() running afterwards.
-- Seeding runs BEFORE the api container starts so tracks_count > 0 skips the
-- Lipas auto-fetch in main.rs; the suite stays hermetic.

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

-- Fixture rows. lipas_id values 999001-999003 are well outside the live
-- Lipas range; admin/refresh-tracks won't collide with these.
INSERT OR REPLACE INTO tracks
    (lipas_id, name, lat, lon, type_code, status, address, postal_code, city, suburb, surface, track_length_m, lanes, geometry_geojson, last_synced_at)
VALUES
    (999001, 'E2E Helsinki Olympic Stadium', 60.1872, 24.9272, 1220, 'active', 'Paavo Nurmen tie 1', '00250', 'Helsinki', 'Taka-Töölö', 'tartan', 400, 8, NULL, '2026-05-08T00:00:00Z'),
    (999002, 'E2E Tampere Tammela',         61.5050, 23.7667, 1220, 'active', 'Tammelan puistokatu 12', '33100', 'Tampere', 'Tammela', 'tartan', 400, 6, NULL, '2026-05-08T00:00:00Z'),
    (999003, 'E2E Oulu Raatti',             65.0186, 25.4789, 1220, 'active', 'Raatintie 5', '90130', 'Oulu', 'Raatti', 'tartan', 400, 8, NULL, '2026-05-08T00:00:00Z');

-- Test users + historical runs for leaderboard tests. password_hash is a stub
-- (login flow is exercised separately by other tests).
INSERT OR IGNORE INTO users (email, display_name, password_hash, created_at) VALUES
    ('e2e-alice@example.com', 'E2E Alice', 'stub', '2025-01-01T00:00:00Z'),
    ('e2e-bob@example.com',   'E2E Bob',   'stub', '2025-01-01T00:00:00Z'),
    ('e2e-carol@example.com', 'E2E Carol', 'stub', '2025-01-01T00:00:00Z');

-- Cross-track runs in fixed past dates so the all-time leaderboard is
-- deterministic. Bob holds the overall best (58.40 at Helsinki).
-- UNION ALL form is used instead of `VALUES (...) AS r(col, ...)` because
-- older SQLite versions don't support column aliases on table-valued VALUES.
INSERT OR IGNORE INTO runs (user_id, track_id, time_seconds, logged_at)
SELECT u.id, t.id, r.time_seconds, r.logged_at
FROM (
        SELECT 'e2e-alice@example.com' AS email, 999001 AS lipas_id, 60.50 AS time_seconds, '2025-06-15T10:00:00+00:00' AS logged_at
        UNION ALL SELECT 'e2e-alice@example.com', 999002, 61.20, '2025-07-20T10:00:00+00:00'
        UNION ALL SELECT 'e2e-bob@example.com',   999001, 58.40, '2025-08-01T10:00:00+00:00'
        UNION ALL SELECT 'e2e-bob@example.com',   999003, 59.10, '2025-09-12T10:00:00+00:00'
        UNION ALL SELECT 'e2e-carol@example.com', 999002, 62.75, '2025-10-05T10:00:00+00:00'
        UNION ALL SELECT 'e2e-carol@example.com', 999001, 63.30, '2025-11-18T10:00:00+00:00'
) AS r
JOIN users  u ON u.email = r.email
JOIN tracks t ON t.lipas_id = r.lipas_id;
