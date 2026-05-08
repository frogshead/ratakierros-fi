#!/usr/bin/env bash
# End-to-end smoke tests for ratakierros-fi.
#
# Runs against a stack started via:
#   docker compose -f docker-compose.yml -f e2e/docker-compose.e2e.yml up -d
#   (after seeding via the seed-sidecar profile)
#
# Hits both entry points so we catch:
#   - browser → nginx → api regressions  (NGINX_BASE, port 8080, same-origin)
#   - cross-origin CORS regressions       (API_BASE,   port 3000, what 744b2ff broke)
set -euo pipefail

NGINX_BASE="${NGINX_BASE:-http://localhost:8080}"
API_BASE="${API_BASE:-http://localhost:3000}"

# Unique per-run user so `register` always succeeds idempotently.
RUN_ID="$(date -u +%Y%m%d%H%M%S)-$$"
EMAIL="e2e-${RUN_ID}@ratakierros.test"
PASSWORD="e2e-password-1234"
DISPLAY_NAME="e2e-${RUN_ID}"

pass() { printf '\033[32m✓\033[0m %s\n' "$1"; }
fail() { printf '\033[31m✗ %s\033[0m\n' "$1" >&2; exit 1; }
note() { printf '  %s\n' "$1"; }

# curl wrapper: -sS quiet but show errors, --fail-with-body returns non-zero on 4xx/5xx
# while still printing the response body. Falls back to --fail for older curl.
curl_json() {
  if curl --help all 2>/dev/null | grep -q -- '--fail-with-body'; then
    curl -sS --fail-with-body "$@"
  else
    curl -sS --fail "$@"
  fi
}

require() {
  command -v "$1" >/dev/null 2>&1 || fail "missing required tool: $1"
}

require curl
require jq

echo "=== ratakierros-fi e2e smoke ==="
echo "nginx base: ${NGINX_BASE}"
echo "api base:   ${API_BASE}"
echo "run id:     ${RUN_ID}"
echo

# ---------------------------------------------------------------------------
# 1. Health (via nginx)
# ---------------------------------------------------------------------------
res=$(curl_json "${NGINX_BASE}/api/health")
status=$(echo "$res" | jq -r '.status')
[[ "$status" == "ok" ]] || fail "GET /api/health expected status=ok, got: $res"
pass "GET /api/health → status=ok (via nginx)"

# Also direct, exercising the bind on :3000.
res=$(curl_json "${API_BASE}/api/health")
[[ "$(echo "$res" | jq -r '.status')" == "ok" ]] || fail "GET /api/health (direct) failed: $res"
pass "GET /api/health → status=ok (direct :3000)"

# ---------------------------------------------------------------------------
# 2. List tracks — the regression class. Must return at least the seed rows.
# ---------------------------------------------------------------------------
res=$(curl_json "${NGINX_BASE}/api/tracks")
count=$(echo "$res" | jq 'length')
[[ "$count" -ge 1 ]] || fail "GET /api/tracks expected length>=1, got $count: $res"
pass "GET /api/tracks → ${count} rows (via nginx)"

res=$(curl_json "${API_BASE}/api/tracks")
[[ "$(echo "$res" | jq 'length')" -ge 1 ]] || fail "GET /api/tracks direct returned 0 rows"
pass "GET /api/tracks → non-empty (direct :3000)"

# Capture an id we can use for downstream steps.
TRACK_ID=$(echo "$res" | jq -r '.[0].id')
[[ "$TRACK_ID" =~ ^[0-9]+$ ]] || fail "could not extract track id from list: $res"
note "using track_id=${TRACK_ID}"

# ---------------------------------------------------------------------------
# 3. Filtered list (lat/lon/q)
# ---------------------------------------------------------------------------
res=$(curl_json --get "${NGINX_BASE}/api/tracks" \
  --data-urlencode "lat=60.17" --data-urlencode "lon=24.94" --data-urlencode "q=helsinki")
[[ "$(echo "$res" | jq 'length')" -ge 1 ]] || fail "filtered tracks returned empty: $res"
pass "GET /api/tracks?lat=&lon=&q=helsinki → non-empty"

# ---------------------------------------------------------------------------
# 4. Single track lookup
# ---------------------------------------------------------------------------
res=$(curl_json "${NGINX_BASE}/api/tracks/${TRACK_ID}")
got=$(echo "$res" | jq -r '.id')
[[ "$got" == "$TRACK_ID" ]] || fail "GET /api/tracks/:id mismatched id (got $got, want $TRACK_ID): $res"
pass "GET /api/tracks/${TRACK_ID} → id matches"

# ---------------------------------------------------------------------------
# 5. Records
# ---------------------------------------------------------------------------
res=$(curl_json "${NGINX_BASE}/api/tracks/${TRACK_ID}/records")
echo "$res" | jq -e '.records | type == "array"' >/dev/null \
  || fail "GET /api/tracks/:id/records missing .records array: $res"
pass "GET /api/tracks/${TRACK_ID}/records → records[] present"

# ---------------------------------------------------------------------------
# 6. Register (auth)
# ---------------------------------------------------------------------------
res=$(curl_json -X POST "${NGINX_BASE}/api/auth/register" \
  -H 'Content-Type: application/json' \
  -d "$(jq -n --arg e "$EMAIL" --arg n "$DISPLAY_NAME" --arg p "$PASSWORD" \
        '{email:$e, display_name:$n, password:$p}')")
TOKEN=$(echo "$res" | jq -r '.token')
[[ -n "$TOKEN" && "$TOKEN" != "null" ]] || fail "register did not return a token: $res"
pass "POST /api/auth/register → token issued"

# ---------------------------------------------------------------------------
# 7. Login with the same credentials
# ---------------------------------------------------------------------------
res=$(curl_json -X POST "${NGINX_BASE}/api/auth/login" \
  -H 'Content-Type: application/json' \
  -d "$(jq -n --arg e "$EMAIL" --arg p "$PASSWORD" '{email:$e, password:$p}')")
TOKEN=$(echo "$res" | jq -r '.token')
[[ -n "$TOKEN" && "$TOKEN" != "null" ]] || fail "login did not return a token: $res"
pass "POST /api/auth/login → token issued"

# ---------------------------------------------------------------------------
# 8. Log a run (authed)
# ---------------------------------------------------------------------------
curl_json -X POST "${NGINX_BASE}/api/runs" \
  -H 'Content-Type: application/json' \
  -H "Authorization: Bearer ${TOKEN}" \
  -d "$(jq -n --argjson tid "$TRACK_ID" '{track_id:$tid, time_seconds:75.4}')" >/dev/null
pass "POST /api/runs (authed) → 2xx"

# ---------------------------------------------------------------------------
# 9. Add then remove favorite (authed)
# ---------------------------------------------------------------------------
curl_json -X POST "${NGINX_BASE}/api/favorites/${TRACK_ID}" \
  -H "Authorization: Bearer ${TOKEN}" >/dev/null
pass "POST /api/favorites/${TRACK_ID} → 2xx"

curl_json -X DELETE "${NGINX_BASE}/api/favorites/${TRACK_ID}" \
  -H "Authorization: Bearer ${TOKEN}" >/dev/null
pass "DELETE /api/favorites/${TRACK_ID} → 2xx"

# ---------------------------------------------------------------------------
# 10. GPX analyze
# ---------------------------------------------------------------------------
GPX_FIXTURE="$(dirname "$0")/fixtures/tiny.gpx"
[[ -f "$GPX_FIXTURE" ]] || fail "missing GPX fixture at $GPX_FIXTURE"
res=$(curl_json -X POST "${NGINX_BASE}/api/gpx/analyze" \
  -F "file=@${GPX_FIXTURE};type=application/gpx+xml")
echo "$res" | jq -e '.best_lap' >/dev/null \
  || fail "POST /api/gpx/analyze missing .best_lap: $res"
pass "POST /api/gpx/analyze → best_lap present"

# ---------------------------------------------------------------------------
# 11. CORS preflight — THE 744b2ff regression test.
# Browser sends OPTIONS with Origin + Access-Control-Request-Method before
# a cross-origin GET. tower-http 0.5 changed CorsLayer::permissive() behavior
# in a way that broke this for the dev (localhost:8000 → :3000) setup.
# ---------------------------------------------------------------------------
preflight=$(curl -sS -i -X OPTIONS "${API_BASE}/api/tracks" \
  -H 'Origin: http://localhost:8000' \
  -H 'Access-Control-Request-Method: GET' \
  -H 'Access-Control-Request-Headers: authorization,content-type')
# NOTE: use here-strings, not `echo … | grep -q`. Under `set -o pipefail`,
# `grep -q` closes the pipe as soon as it matches, sending SIGPIPE to echo
# and tripping pipefail. Locally the timing usually wins; CI exposes the race.
status_line=$(head -n1 <<< "$preflight")
grep -qE 'HTTP/[0-9.]+ (200|204)' <<< "$status_line" \
  || fail "CORS preflight expected 200/204, got: $status_line"
grep -qi '^access-control-allow-origin:' <<< "$preflight" \
  || fail "CORS preflight missing access-control-allow-origin header.\nFull response:\n$preflight"
grep -qi '^access-control-allow-methods:' <<< "$preflight" \
  || fail "CORS preflight missing access-control-allow-methods header"
pass "OPTIONS /api/tracks (CORS preflight) → headers present"

# Real cross-origin GET should also include the ACAO header.
acao=$(curl -sS -D - -o /dev/null \
  -H 'Origin: http://localhost:8000' \
  "${API_BASE}/api/tracks" | grep -i '^access-control-allow-origin:' || true)
[[ -n "$acao" ]] || fail "GET /api/tracks (with Origin) missing ACAO header"
pass "GET /api/tracks (cross-origin) → ACAO header present"

# ---------------------------------------------------------------------------
# 12. Admin refresh-tracks — best-effort. Hits external Lipas; tolerate
# upstream slowness / outages so unrelated builds don't go red.
# ---------------------------------------------------------------------------
if curl -sS --max-time 30 -X POST "${NGINX_BASE}/api/admin/refresh-tracks" >/dev/null 2>&1; then
  pass "POST /api/admin/refresh-tracks → 2xx (Lipas reachable)"
else
  note "POST /api/admin/refresh-tracks could not reach Lipas (tolerated)"
fi

# ---------------------------------------------------------------------------
# 13. Frontend served via nginx
# ---------------------------------------------------------------------------
html=$(curl_json "${NGINX_BASE}/")
[[ "$html" == *"window.API_BASE"* ]] \
  || fail "frontend HTML missing injected window.API_BASE"
pass "GET / → frontend HTML with API_BASE injected"

echo
echo "all smoke checks passed."
