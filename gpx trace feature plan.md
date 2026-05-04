# Feature Plan: GPS Trace — Best 400 m Effort (backend-side)

## Overview

User uploads a GPX file from a running watch / phone. The **Rust API**
parses it, finds the **fastest continuous 400 m segment** (Strava
"Best Effort" style — sliding window, not fixed-start laps), and
returns the result. The user can then log that time against a track
through the existing `POST /api/runs` flow.

## Why sliding window, not fixed laps

The original draft talked about "lap = 400 m, take min." That's wrong
for a real GPS trace: GPS noise means cumulative distance won't land
exactly on 400 m boundaries, and a runner's fastest 400 m is rarely
the one that starts at the trace's t=0. Strava and Garmin both expose
this as "best effort over distance X" — pick the fastest *any* 400 m
window in the trace. We do the same.

## API surface

```
POST /api/gpx/analyze
Content-Type: multipart/form-data
Body: field name "file", value = raw .gpx bytes
Auth: optional (analyze is read-only; saving comes via existing /api/runs)

200 OK
{
  "best_lap": {
    "time_seconds": 78.42,
    "start_time": "2026-04-15T16:21:03Z",
    "end_time":   "2026-04-15T16:22:21.42Z",
    "start_trackpoint_index": 142,
    "distance_m": 400.0
  },
  "trace": {
    "trackpoint_count": 612,
    "total_distance_m": 2415.7,
    "duration_seconds": 489.0,
    "start_time": "2026-04-15T16:18:00Z",
    "end_time":   "2026-04-15T16:26:09Z"
  }
}

400 Bad Request — invalid GPX, no trackpoints, missing timestamps,
                  trace shorter than the target distance.
```

The endpoint does **not** write to the database — it's a pure
analyzer. The frontend gets the result, lets the user pick the right
track + confirm, then POSTs to the existing `/api/runs` with the
returned `time_seconds`.

## Algorithm

Pure function in `api/src/gpx_analyze.rs`:

```rust
pub fn find_best_lap(gpx_xml: &str, target_distance_m: f64)
    -> Result<BestLap, AnalyzeError>;
```

1. Parse GPX with the [`gpx`](https://crates.io/crates/gpx) crate.
   Flatten all tracks → segments → points into a single ordered
   `Vec<Trackpoint>` of `{lat, lon, time}`. Reject trackpoints
   without `time`.
2. Compute cumulative distance using `geo::HaversineDistance`
   (already a dep) between consecutive points.
3. Two-pointer sweep over indices `i`:
   - For each `i`, advance `j` forward until
     `cumdist[j] - cumdist[i] >= target_distance_m`.
   - Linearly interpolate the exact moment the runner crossed the
     target distance between `j-1` and `j`:
     `frac = (target - (cumdist[j-1] - cumdist[i])) / (cumdist[j] - cumdist[j-1])`,
     `t_cross = time[j-1] + frac * (time[j] - time[j-1])`.
   - Lap candidate = `t_cross - time[i]`.
4. Return the minimum candidate, plus its `start_time` (= `time[i]`)
   and `end_time` (= `t_cross`). O(n) total.
5. If no `i` finds a valid `j`, return `AnalyzeError::TooShort`.

## Errors

```rust
pub enum AnalyzeError {
    InvalidGpx(String),     // parser rejected the bytes
    NoTrackpoints,          // empty trace
    MissingTimestamps,      // at least one trackpoint had no time
    NonMonotonicTime,       // times go backward → garbage trace
    TooShort,               // total distance < target_distance_m
}
```

These map to HTTP 400 with a Finnish-language error message in the
existing `AppError::BadRequest` style used by the rest of the API.

## TDD plan

Unit tests live in `api/src/gpx_analyze.rs` (inline `#[cfg(test)] mod tests`).
Each test builds a synthetic GPX string — no fixture files on disk.
Helper:

```rust
fn synth_gpx(points: &[(f64, f64, &str)]) -> String { ... }
```

builds a minimal valid GPX from `(lat, lon, iso_timestamp)` triples.

Order of tests, each one driving a piece of the implementation:

1. `parse_empty_gpx_errors` → returns `InvalidGpx` on `""` and
   `<gpx></gpx>` (no trackpoints).
2. `parse_extracts_trackpoints_in_order` → handcrafted GPX with 3
   points → `extract_trackpoints` returns a `Vec` of length 3 with
   matching lat/lon/time.
3. `missing_time_rejected` → trackpoint without `<time>` → returns
   `MissingTimestamps`.
4. `non_monotonic_time_rejected` → time goes backward between two
   points → returns `NonMonotonicTime`.
5. `cumulative_distance_two_points` → two points 100 m apart on the
   equator → `cumulative_distance` matches `100 ± 1 m`.
6. `too_short_returns_error` → 200 m trace, target 400 m → returns
   `TooShort`.
7. `single_uniform_lap_correct_time` → straight line, 800 m at
   constant 4 m/s → best 400 m = 100 s, start_time = trace start.
8. `second_lap_faster_returns_second_lap` → first 400 m at 100 s,
   second 400 m at 80 s → returns 80 s and start_time matches the
   start of the second lap.
9. `interpolation_between_trackpoints` → trackpoints at 380 m and
   420 m → interpolate the exact 400 m crossing.

Once the pure function is green, integration test for the HTTP
handler:

10. `gpx_analyze_endpoint_happy_path` — multipart upload of a small
    synthetic GPX, expects 200 + JSON shape above. Uses
    `axum::http::Request::builder()` and `tower::ServiceExt::oneshot`
    against the router (the existing test pattern — verify by reading
    `api/src/lib.rs` or any existing integration test).

## Files modified

- `api/Cargo.toml` — add `gpx = "0.10"` (or whatever resolves) and
  enable axum's `multipart` feature.
- `api/src/gpx_analyze.rs` — new module with pure function + tests.
- `api/src/lib.rs` — expose the module + types, re-export
  `find_best_lap` and `BestLap`.
- `api/src/main.rs` — new handler `gpx_analyze_handler`,
  wired at `POST /api/gpx/analyze`.
- This plan file — already updated above.

## Out of scope

- Front-end UI for the upload (web app gets it later in a separate PR).
- Per-track best efforts on the leaderboard (would need DB schema
  change to store full traces).
- Distances other than 400 m (the algorithm takes a parameter; a
  query-string override is a one-line follow-up).
- Auto-detecting the track from GPX coordinates (user picks track
  manually before logging).
