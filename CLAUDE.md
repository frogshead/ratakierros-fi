# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

Ratakierros.fi — a Finnish web app that finds the nearest athletics track (juoksurata) based on the user's geolocation. It queries OpenStreetMap via the Overpass API for features tagged `sport=athletics`.

## Architecture

Three Docker services orchestrated via `docker-compose.yml`, proxied through nginx on port 8080:

- **API** (`api/`): Rust (Axum 0.6) server on port 3000. Accepts `GET /api/closest?lat=...&lon=...&radius=...`, queries Overpass API, computes haversine distances, returns the nearest track as JSON. Library code in `lib.rs`, HTTP handlers in `main.rs`.
- **Frontend** (`ratakierros-fi.ts` + `public/`): Deno server on port 8000 serving a single `index.html`. The HTML uses Leaflet.js for the map and vanilla JS to call the API.
- **Nginx**: Reverse proxy — `/api/` → API container, `/` → frontend container.

## Build & Run Commands

```bash
# Run all services locally
docker compose up --build

# Build/run API only (for development)
cd api && cargo build
cd api && cargo run          # listens on :3000

# Run API tests
cd api && cargo test

# Run frontend only (requires Deno)
deno run --allow-net --allow-read ratakierros-fi.ts

# Build container images
docker compose build
```

## Container Registry

Images are published to `ghcr.io/frogshead/ratakierros-fi/api` and `ghcr.io/frogshead/ratakierros-fi/frontend`.

## Key API Details

- The Overpass query searches for nodes, ways, and relations with `sport=athletics` within the given radius (default 5000m)
- Distance calculation uses the Haversine formula via the `geo` crate
- For ways/relations, coordinates come from the `center` field in Overpass output
- The frontend UI is in Finnish
