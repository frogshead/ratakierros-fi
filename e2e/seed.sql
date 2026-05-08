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
