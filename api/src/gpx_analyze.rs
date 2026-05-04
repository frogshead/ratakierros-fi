//! GPX trace analysis: find the fastest continuous distance segment.
//!
//! This is a "best-effort" calculation in the Strava sense — for a target
//! distance like 400 m, slide a window over the trace and report the
//! fastest window. It is *not* "split the trace into laps starting from
//! t=0 and pick the fastest." Real GPS traces don't have a nice 400 m
//! boundary at t=0.

use chrono::{DateTime, Utc};
use geo::HaversineDistance;
use geo_types::Point as GeoPoint;
use serde::Serialize;
use std::io::Cursor;

/// Default target distance for "best lap" — a standard outdoor athletics
/// track lap.
pub const DEFAULT_TARGET_DISTANCE_M: f64 = 400.0;

#[derive(Debug, Clone, Serialize)]
pub struct BestLap {
    pub time_seconds: f64,
    pub start_time: DateTime<Utc>,
    pub end_time: DateTime<Utc>,
    pub start_trackpoint_index: usize,
    pub distance_m: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct TraceSummary {
    pub trackpoint_count: usize,
    pub total_distance_m: f64,
    pub duration_seconds: f64,
    pub start_time: DateTime<Utc>,
    pub end_time: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AnalyzeResult {
    pub best_lap: BestLap,
    pub trace: TraceSummary,
}

#[derive(Debug)]
pub enum AnalyzeError {
    InvalidGpx(String),
    NoTrackpoints,
    MissingTimestamps,
    NonMonotonicTime,
    TooShort,
}

impl std::fmt::Display for AnalyzeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AnalyzeError::InvalidGpx(m) => write!(f, "Virheellinen GPX-tiedosto: {}", m),
            AnalyzeError::NoTrackpoints => write!(f, "GPX-tiedostossa ei ole trackpointteja."),
            AnalyzeError::MissingTimestamps => {
                write!(f, "Trackpointeista puuttuu aikaleima.")
            }
            AnalyzeError::NonMonotonicTime => {
                write!(f, "Aikaleimat eivät ole nousevassa järjestyksessä.")
            }
            AnalyzeError::TooShort => {
                write!(f, "Trace on lyhyempi kuin tavoiteltu kierrosmatka.")
            }
        }
    }
}

impl std::error::Error for AnalyzeError {}

#[derive(Debug, Clone)]
struct Trackpoint {
    lat: f64,
    lon: f64,
    time: DateTime<Utc>,
}

/// Top-level analyzer. Pass raw GPX XML and the desired window length in
/// metres (typically 400.0). Returns the best window plus a summary of
/// the whole trace.
pub fn analyze_gpx(gpx_xml: &str, target_distance_m: f64) -> Result<AnalyzeResult, AnalyzeError> {
    let points = extract_trackpoints(gpx_xml)?;
    validate_monotonic_time(&points)?;
    let cum = cumulative_distance(&points);
    let best = best_window(&points, &cum, target_distance_m)?;
    let trace = trace_summary(&points, &cum);
    Ok(AnalyzeResult {
        best_lap: best,
        trace,
    })
}

fn extract_trackpoints(gpx_xml: &str) -> Result<Vec<Trackpoint>, AnalyzeError> {
    if gpx_xml.trim().is_empty() {
        return Err(AnalyzeError::InvalidGpx("empty input".into()));
    }
    let parsed = gpx::read(Cursor::new(gpx_xml.as_bytes()))
        .map_err(|e| AnalyzeError::InvalidGpx(e.to_string()))?;

    let mut out: Vec<Trackpoint> = Vec::new();
    let mut any_seen = false;
    let mut missing_time = false;
    for track in &parsed.tracks {
        for seg in &track.segments {
            for wp in &seg.points {
                any_seen = true;
                let pt = wp.point();
                let time = match wp.time {
                    Some(t) => match offset_to_chrono(t.into()) {
                        Some(c) => c,
                        None => {
                            return Err(AnalyzeError::InvalidGpx(
                                "timestamp out of range".into(),
                            ))
                        }
                    },
                    None => {
                        missing_time = true;
                        continue;
                    }
                };
                out.push(Trackpoint {
                    lat: pt.y(),
                    lon: pt.x(),
                    time,
                });
            }
        }
    }
    if !any_seen {
        return Err(AnalyzeError::NoTrackpoints);
    }
    if missing_time {
        return Err(AnalyzeError::MissingTimestamps);
    }
    if out.is_empty() {
        return Err(AnalyzeError::NoTrackpoints);
    }
    Ok(out)
}

fn offset_to_chrono(odt: time::OffsetDateTime) -> Option<DateTime<Utc>> {
    let utc = odt.to_offset(time::UtcOffset::UTC);
    DateTime::<Utc>::from_timestamp(utc.unix_timestamp(), utc.nanosecond())
}

fn validate_monotonic_time(points: &[Trackpoint]) -> Result<(), AnalyzeError> {
    for w in points.windows(2) {
        if w[1].time < w[0].time {
            return Err(AnalyzeError::NonMonotonicTime);
        }
    }
    Ok(())
}

fn cumulative_distance(points: &[Trackpoint]) -> Vec<f64> {
    let mut cum = Vec::with_capacity(points.len());
    cum.push(0.0);
    for pair in points.windows(2) {
        let a = GeoPoint::new(pair[0].lon, pair[0].lat);
        let b = GeoPoint::new(pair[1].lon, pair[1].lat);
        let d = a.haversine_distance(&b);
        cum.push(cum.last().copied().unwrap() + d);
    }
    cum
}

fn best_window(
    points: &[Trackpoint],
    cum: &[f64],
    target: f64,
) -> Result<BestLap, AnalyzeError> {
    if points.len() < 2 {
        return Err(AnalyzeError::TooShort);
    }
    let total = *cum.last().unwrap();
    if total < target {
        return Err(AnalyzeError::TooShort);
    }

    let mut best: Option<(f64, usize, DateTime<Utc>, DateTime<Utc>)> = None;
    let mut j: usize = 0;
    for i in 0..points.len() {
        if j < i {
            j = i;
        }
        // Advance j until cum[j] - cum[i] >= target, or we run out of points.
        while j < points.len() && cum[j] - cum[i] < target {
            j += 1;
        }
        if j >= points.len() {
            // No more candidates from this i onwards.
            break;
        }
        // j-1..j brackets the exact target crossing. j is at least i+1 here
        // (since at j == i the gap is 0 < target).
        debug_assert!(j > i);
        let prev = j - 1;
        let span = cum[j] - cum[prev];
        let frac = if span > 0.0 {
            ((target - (cum[prev] - cum[i])) / span).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let t_prev = points[prev].time;
        let t_next = points[j].time;
        let dt_nanos =
            (t_next - t_prev).num_nanoseconds().unwrap_or(0) as f64 * frac;
        let t_cross = t_prev + chrono::Duration::nanoseconds(dt_nanos as i64);
        let lap_dur = (t_cross - points[i].time)
            .num_nanoseconds()
            .unwrap_or(0) as f64
            / 1_000_000_000.0;
        if lap_dur <= 0.0 {
            continue;
        }
        match best {
            None => best = Some((lap_dur, i, points[i].time, t_cross)),
            Some((cur, _, _, _)) if lap_dur < cur => {
                best = Some((lap_dur, i, points[i].time, t_cross))
            }
            _ => {}
        }
    }

    match best {
        Some((time_seconds, start_idx, start_time, end_time)) => Ok(BestLap {
            time_seconds,
            start_time,
            end_time,
            start_trackpoint_index: start_idx,
            distance_m: target,
        }),
        None => Err(AnalyzeError::TooShort),
    }
}

fn trace_summary(points: &[Trackpoint], cum: &[f64]) -> TraceSummary {
    let start_time = points.first().map(|p| p.time).unwrap_or_else(Utc::now);
    let end_time = points.last().map(|p| p.time).unwrap_or(start_time);
    let duration_seconds = (end_time - start_time)
        .num_nanoseconds()
        .unwrap_or(0) as f64
        / 1_000_000_000.0;
    TraceSummary {
        trackpoint_count: points.len(),
        total_distance_m: *cum.last().unwrap_or(&0.0),
        duration_seconds,
        start_time,
        end_time,
    }
}

// --------------------------------------------------------------------------
// Tests
// --------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    /// Build a minimal GPX XML string from `(lat, lon, time)` triples.
    /// `time` is `Some(iso8601)` or `None` (the latter omits `<time>`).
    fn synth_gpx(points: &[(f64, f64, Option<&str>)]) -> String {
        let mut s = String::new();
        s.push_str(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<gpx version="1.1" creator="test" xmlns="http://www.topografix.com/GPX/1/1">
  <trk><trkseg>
"#,
        );
        for (lat, lon, t) in points {
            s.push_str(&format!("    <trkpt lat=\"{}\" lon=\"{}\">", lat, lon));
            if let Some(ts) = t {
                s.push_str(&format!("<time>{}</time>", ts));
            }
            s.push_str("</trkpt>\n");
        }
        s.push_str("  </trkseg></trk>\n</gpx>\n");
        s
    }

    /// At the equator, 1 degree of longitude ≈ 111_320 m. We use that to
    /// place trackpoints at known distances east of (0, 0).
    fn lon_for_metres_east(metres: f64) -> f64 {
        // Haversine returns ~111_195 m per degree at the equator with the
        // earth radius the geo crate uses; using that constant keeps the
        // tests self-consistent.
        metres / 111_195.0
    }

    #[test]
    fn parse_empty_input_errors() {
        let err = analyze_gpx("", 400.0).unwrap_err();
        assert!(matches!(err, AnalyzeError::InvalidGpx(_)));
    }

    #[test]
    fn parse_no_trackpoints_errors() {
        let xml = r#"<?xml version="1.0"?><gpx version="1.1" creator="t" xmlns="http://www.topografix.com/GPX/1/1"></gpx>"#;
        let err = analyze_gpx(xml, 400.0).unwrap_err();
        assert!(matches!(err, AnalyzeError::NoTrackpoints), "got {:?}", err);
    }

    #[test]
    fn parse_extracts_trackpoints_in_order() {
        let xml = synth_gpx(&[
            (60.17, 24.94, Some("2026-04-15T16:00:00Z")),
            (60.18, 24.94, Some("2026-04-15T16:00:30Z")),
            (60.19, 24.94, Some("2026-04-15T16:01:00Z")),
        ]);
        let pts = extract_trackpoints(&xml).unwrap();
        assert_eq!(pts.len(), 3);
        assert!((pts[0].lat - 60.17).abs() < 1e-9);
        assert_eq!(
            pts[1].time,
            Utc.with_ymd_and_hms(2026, 4, 15, 16, 0, 30).unwrap()
        );
    }

    #[test]
    fn missing_time_rejected() {
        let xml = synth_gpx(&[
            (0.0, 0.0, Some("2026-04-15T16:00:00Z")),
            (0.0, lon_for_metres_east(100.0), None),
        ]);
        let err = analyze_gpx(&xml, 400.0).unwrap_err();
        assert!(matches!(err, AnalyzeError::MissingTimestamps), "got {:?}", err);
    }

    #[test]
    fn non_monotonic_time_rejected() {
        let xml = synth_gpx(&[
            (0.0, 0.0, Some("2026-04-15T16:00:30Z")),
            (0.0, lon_for_metres_east(100.0), Some("2026-04-15T16:00:00Z")),
        ]);
        let err = analyze_gpx(&xml, 400.0).unwrap_err();
        assert!(matches!(err, AnalyzeError::NonMonotonicTime), "got {:?}", err);
    }

    #[test]
    fn cumulative_distance_two_points_about_100m() {
        let pts = vec![
            Trackpoint {
                lat: 0.0,
                lon: 0.0,
                time: Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap(),
            },
            Trackpoint {
                lat: 0.0,
                lon: lon_for_metres_east(100.0),
                time: Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 30).unwrap(),
            },
        ];
        let cum = cumulative_distance(&pts);
        assert_eq!(cum.len(), 2);
        assert!(cum[0] == 0.0);
        let d = cum[1];
        assert!((d - 100.0).abs() < 1.0, "cum[1] was {}", d);
    }

    #[test]
    fn too_short_returns_error() {
        let xml = synth_gpx(&[
            (0.0, 0.0, Some("2026-04-15T16:00:00Z")),
            (0.0, lon_for_metres_east(200.0), Some("2026-04-15T16:01:00Z")),
        ]);
        let err = analyze_gpx(&xml, 400.0).unwrap_err();
        assert!(matches!(err, AnalyzeError::TooShort), "got {:?}", err);
    }

    /// Straight-line 800 m at constant 4 m/s. Best 400 m anywhere should
    /// be 100 s, and the start time should be the trace start.
    #[test]
    fn single_uniform_lap_correct_time() {
        let xml = synth_gpx(&[
            (0.0, 0.0, Some("2026-04-15T16:00:00Z")),
            (0.0, lon_for_metres_east(200.0), Some("2026-04-15T16:00:50Z")),
            (0.0, lon_for_metres_east(400.0), Some("2026-04-15T16:01:40Z")),
            (0.0, lon_for_metres_east(600.0), Some("2026-04-15T16:02:30Z")),
            (0.0, lon_for_metres_east(800.0), Some("2026-04-15T16:03:20Z")),
        ]);
        let result = analyze_gpx(&xml, 400.0).unwrap();
        assert!(
            (result.best_lap.time_seconds - 100.0).abs() < 0.5,
            "best lap was {}",
            result.best_lap.time_seconds
        );
        assert_eq!(
            result.best_lap.start_time,
            Utc.with_ymd_and_hms(2026, 4, 15, 16, 0, 0).unwrap()
        );
        assert!(result.trace.total_distance_m > 790.0);
        assert!(result.trace.total_distance_m < 810.0);
    }

    /// First 400 m at 100 s, second 400 m at 80 s (faster). Best window
    /// should land on the second lap.
    #[test]
    fn second_lap_faster_returns_second_lap() {
        let xml = synth_gpx(&[
            (0.0, 0.0, Some("2026-04-15T16:00:00Z")),
            (0.0, lon_for_metres_east(400.0), Some("2026-04-15T16:01:40Z")),
            (0.0, lon_for_metres_east(800.0), Some("2026-04-15T16:03:00Z")),
        ]);
        let result = analyze_gpx(&xml, 400.0).unwrap();
        assert!(
            (result.best_lap.time_seconds - 80.0).abs() < 1.0,
            "best lap was {}",
            result.best_lap.time_seconds
        );
        // The second lap starts at the second trackpoint (16:01:40).
        assert_eq!(
            result.best_lap.start_time,
            Utc.with_ymd_and_hms(2026, 4, 15, 16, 1, 40).unwrap()
        );
    }

    /// Trackpoints land at 380 m and 420 m — the 400 m crossing is
    /// between them. Linear time interpolation should give 50% of the
    /// 380→420 segment time.
    #[test]
    fn interpolation_between_trackpoints() {
        // 0 m at t=0, 380 m at t=95s, 420 m at t=105s. Window from
        // (0, 0) advances to 400 m at frac=(400-380)/(420-380)=0.5,
        // i.e. t=95+0.5*(105-95)=100s. So best lap = 100 s.
        let xml = synth_gpx(&[
            (0.0, 0.0, Some("2026-04-15T16:00:00Z")),
            (0.0, lon_for_metres_east(380.0), Some("2026-04-15T16:01:35Z")),
            (0.0, lon_for_metres_east(420.0), Some("2026-04-15T16:01:45Z")),
        ]);
        let result = analyze_gpx(&xml, 400.0).unwrap();
        assert!(
            (result.best_lap.time_seconds - 100.0).abs() < 0.5,
            "best lap was {}",
            result.best_lap.time_seconds
        );
    }
}
