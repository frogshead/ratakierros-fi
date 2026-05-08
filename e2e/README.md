# End-to-end tests

These tests bring up the full Docker Compose stack (api + frontend + nginx)
and verify the **browser → nginx → api** integration that `cargo test` cannot
cover. They exist because dependabot bump `744b2ff` (`tower-http` 0.4 → 0.5)
broke `GET /api/tracks` from the page even though every unit test still passed.
The reverts (`5d986e4`, `ea2fb64`) confirmed the gap; this suite closes it.

## What runs

1. **`smoke.sh`** — bash + `curl` + `jq` HTTP probes. Hits both the nginx-proxied
   path on `:8080` (matches prod) and the api directly on `:3000` (exercises the
   cross-origin CORS preflight that 744b2ff regressed). Covers every API
   endpoint, including authed flows.
2. **`playwright/`** — headless Chromium loads `/`, waits for the seeded track
   list to render, clicks a card, and fails on any `console.error` /
   `pageerror`. Catches strict-mode browser CORS rejections.

## Local run

```sh
# from repo root
docker compose -f docker-compose.yml -f e2e/docker-compose.e2e.yml build

# Seed first so the api skips the Lipas auto-fetch (tracks_count > 0).
docker compose -f docker-compose.yml -f e2e/docker-compose.e2e.yml \
  run --rm seed-sidecar

docker compose -f docker-compose.yml -f e2e/docker-compose.e2e.yml \
  up -d api frontend nginx

# Wait until the api is reachable through nginx, then run smoke.
until curl -sSf http://localhost:8080/api/health >/dev/null; do sleep 1; done
bash e2e/smoke.sh

# Browser-driven tests
( cd e2e/playwright \
  && npm install \
  && npx playwright install --with-deps chromium \
  && npx playwright test )

# Cleanup (also drops the seeded volume)
docker compose -f docker-compose.yml -f e2e/docker-compose.e2e.yml down -v
```

## CI

The `e2e` job in `.github/workflows/ci.yml` runs the same sequence on every
PR and on push to `main`. On failure it uploads container logs and the
Playwright HTML report as the artifact `e2e-logs-${run_id}`.

`build.yml` (publish + deploy) is gated via `workflow_run` on a green CI
result, so a red `e2e` blocks image publish and the VM deploy. Manual
`workflow_dispatch` still bypasses the gate when needed.

### Branch protection

To make CI's `e2e` job a hard gate on merges to `main`, add it to the list of
required status checks under **Settings → Branches → Branch protection rules
→ main**. The check is named `e2e`.

## Files

| Path | Purpose |
|---|---|
| `docker-compose.e2e.yml` | Overlay: adds `nginx`, sets frontend `API_BASE` to nginx, declares the `seed-sidecar` service. |
| `seed.sql` | Schema (CREATE IF NOT EXISTS) + 3 fixture tracks (Helsinki / Tampere / Oulu). `lipas_id` 999001-999003 are outside the live Lipas range. |
| `smoke.sh` | Curl + jq matrix. `set -euo pipefail`, exits non-zero on first failure. |
| `fixtures/tiny.gpx` | Minimal GPX (~800 m) for `POST /api/gpx/analyze`. |
| `playwright/tests/tracks.spec.ts` | Browser scenario: list → detail. |
| `playwright/playwright.config.ts` | Single `chromium` project, `baseURL` from `E2E_BASE_URL` env. |

## Updating the seed

When `init_db()` in `api/src/lib.rs` gains columns, mirror the changes here so
fresh CI runs (with no pre-existing volume) get a complete schema. The schema
is duplicated by design — `seed-sidecar` runs **before** the api starts so we
can't delegate schema creation to `init_db()` without giving up hermeticity.
