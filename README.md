# Ratakierros.fi

Finnish web app for discovering athletics tracks (juoksuradat) in Finland and logging personal 400 m runs.

## Features

- **Track directory** — Browse all athletics tracks in Finland sourced from OpenStreetMap
- **Search** — Filter tracks by name or city; sort by distance from your location
- **Map & directions** — View each track on an embedded map and open directions in Google Maps
- **Track records (kenttäennätykset)** — See the all-time best 400 m times logged at each track
- **Run logging** — Register an account and log your own 400 m times; personal bests tracked per track
- **Mobile-first** — Responsive layout that works on both phone and desktop browsers

## Architecture

Three Docker services orchestrated via `docker-compose.yml`, proxied through nginx on port 8080:

```
User (port 8080)
       ↓
   Nginx (reverse proxy)
   ↙          ↘
API (3000)   Frontend (8000)
```

| Service | Stack | Role |
|---------|-------|------|
| **api** | Rust (Axum 0.6) | REST API — tracks, auth, run records; SQLite database |
| **frontend** | Deno + static HTML | Serves `public/index.html` |
| **nginx** | Alpine nginx | Routes `/api/` → API, `/` → frontend |

## Data Model

SQLite database (`/data/ratakierros.db` in production, `./ratakierros.db` in dev):

```sql
tracks (id, osm_id, name, lat, lon, city)
  -- Cached from OpenStreetMap via Overpass API (sport=athletics, Finland bbox)
  -- Refreshed on startup if empty; POST /api/admin/refresh-tracks to force refresh

users  (id, email, display_name, password_hash, created_at)
  -- Argon2 password hashing; JWT sessions (30-day tokens)

runs   (id, user_id, track_id, time_seconds, logged_at)
  -- Each row is one 400 m run logged by a user at a specific track
```

## API Endpoints

| Method | Path | Auth | Description |
|--------|------|------|-------------|
| GET | `/api/tracks` | — | List all tracks; `?lat=&lon=` to sort by distance; `?q=` to search |
| GET | `/api/tracks/:id` | — | Single track details |
| GET | `/api/tracks/:id/records` | optional | Top 10 times for a track + personal best if authenticated |
| POST | `/api/runs` | required | Log a 400 m run `{ track_id, time_seconds }` |
| POST | `/api/auth/register` | — | Register `{ email, display_name, password }` → JWT |
| POST | `/api/auth/login` | — | Login `{ email, password }` → JWT |
| POST | `/api/admin/refresh-tracks` | — | Re-fetch all Finnish tracks from Overpass |
| GET | `/health` | — | Health check |

## Build & Run

```bash
# Run all services locally
docker compose up --build

# API development (Rust)
cd api && cargo build
cd api && cargo run          # listens on :3000
cd api && cargo test

# Frontend development (Deno)
deno run --allow-net --allow-read ratakierros-fi.ts   # listens on :8000
```

## Configuration

Environment variables for the API container:

| Variable | Default | Description |
|----------|---------|-------------|
| `DATABASE_PATH` | `./ratakierros.db` | SQLite database file path |
| `JWT_SECRET` | dev fallback | Secret key for JWT signing — **change in production** |

## Container Registry

Images are published to:
- `ghcr.io/frogshead/ratakierros-fi/api`
- `ghcr.io/frogshead/ratakierros-fi/frontend`
