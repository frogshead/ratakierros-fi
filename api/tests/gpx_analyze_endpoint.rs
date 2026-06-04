//! Integration test for POST /api/gpx/analyze.
//!
//! Builds a minimal axum Router with just the analyze handler (no DB
//! needed since the endpoint doesn't touch one), sends a hand-crafted
//! multipart upload, and asserts the response shape.

use axum::{
    body::Body,
    extract::{Multipart, Query},
    http::{Request, StatusCode},
    response::Json,
    routing::post,
    Router,
};
use ratakierros_api::{analyze_gpx, AnalyzeError, DEFAULT_TARGET_DISTANCE_M};
use serde::Deserialize;
use tower::ServiceExt;

#[derive(Deserialize)]
struct Params {
    distance_m: Option<f64>,
}

async fn handler(
    Query(params): Query<Params>,
    mut multipart: Multipart,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let mut bytes: Option<Vec<u8>> = None;
    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?
    {
        let name = field.name().unwrap_or("").to_string();
        if name == "file" || name == "gpx" {
            let b = field
                .bytes()
                .await
                .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
            bytes = Some(b.to_vec());
        }
    }
    let bytes = bytes.ok_or((StatusCode::BAD_REQUEST, "missing file".into()))?;
    let xml = std::str::from_utf8(&bytes)
        .map_err(|_| (StatusCode::BAD_REQUEST, "not utf-8".into()))?;
    let target = params.distance_m.unwrap_or(DEFAULT_TARGET_DISTANCE_M);
    let result = analyze_gpx(xml, target).map_err(map_err)?;
    Ok(Json(serde_json::to_value(result).unwrap()))
}

fn map_err(e: AnalyzeError) -> (StatusCode, String) {
    (StatusCode::BAD_REQUEST, e.to_string())
}

fn router() -> Router {
    Router::new().route("/api/gpx/analyze", post(handler))
}

const BOUNDARY: &str = "----testboundary";

fn multipart_body(field_name: &str, filename: &str, content: &str) -> Vec<u8> {
    let mut body = Vec::new();
    body.extend_from_slice(format!("--{}\r\n", BOUNDARY).as_bytes());
    body.extend_from_slice(
        format!(
            "Content-Disposition: form-data; name=\"{}\"; filename=\"{}\"\r\n",
            field_name, filename
        )
        .as_bytes(),
    );
    body.extend_from_slice(b"Content-Type: application/gpx+xml\r\n\r\n");
    body.extend_from_slice(content.as_bytes());
    body.extend_from_slice(b"\r\n");
    body.extend_from_slice(format!("--{}--\r\n", BOUNDARY).as_bytes());
    body
}

fn synth_gpx() -> String {
    // 800 m at 4 m/s — best 400 m = 100 s.
    let dx = |m: f64| m / 111_195.0;
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<gpx version="1.1" creator="test" xmlns="http://www.topografix.com/GPX/1/1">
<trk><trkseg>
<trkpt lat="0.0" lon="{}"><time>2026-04-15T16:00:00Z</time></trkpt>
<trkpt lat="0.0" lon="{}"><time>2026-04-15T16:00:50Z</time></trkpt>
<trkpt lat="0.0" lon="{}"><time>2026-04-15T16:01:40Z</time></trkpt>
<trkpt lat="0.0" lon="{}"><time>2026-04-15T16:02:30Z</time></trkpt>
<trkpt lat="0.0" lon="{}"><time>2026-04-15T16:03:20Z</time></trkpt>
</trkseg></trk>
</gpx>
"#,
        dx(0.0),
        dx(200.0),
        dx(400.0),
        dx(600.0),
        dx(800.0),
    )
}

#[tokio::test]
async fn analyze_endpoint_happy_path() {
    let body = multipart_body("file", "run.gpx", &synth_gpx());
    let req = Request::builder()
        .method("POST")
        .uri("/api/gpx/analyze")
        .header(
            "Content-Type",
            format!("multipart/form-data; boundary={}", BOUNDARY),
        )
        .body(Body::from(body))
        .unwrap();

    let resp = router().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body_bytes = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let v: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
    let best = &v["best_lap"];
    let secs = best["time_seconds"].as_f64().expect("time_seconds f64");
    assert!((secs - 100.0).abs() < 0.5, "best_lap was {}", secs);
    assert!(best["start_time"].is_string());
    assert!(best["end_time"].is_string());
    assert!((best["distance_m"].as_f64().unwrap() - 400.0).abs() < 1e-6);
    let trace = &v["trace"];
    assert_eq!(trace["trackpoint_count"].as_i64().unwrap(), 5);
}

#[tokio::test]
async fn analyze_endpoint_missing_file_400() {
    let mut body = Vec::new();
    body.extend_from_slice(format!("--{}\r\n", BOUNDARY).as_bytes());
    body.extend_from_slice(
        b"Content-Disposition: form-data; name=\"other\"\r\n\r\nirrelevant\r\n",
    );
    body.extend_from_slice(format!("--{}--\r\n", BOUNDARY).as_bytes());

    let req = Request::builder()
        .method("POST")
        .uri("/api/gpx/analyze")
        .header(
            "Content-Type",
            format!("multipart/form-data; boundary={}", BOUNDARY),
        )
        .body(Body::from(body))
        .unwrap();
    let resp = router().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn analyze_endpoint_too_short_400() {
    let dx = |m: f64| m / 111_195.0;
    let xml = format!(
        r#"<?xml version="1.0"?><gpx version="1.1" creator="t" xmlns="http://www.topografix.com/GPX/1/1">
<trk><trkseg>
<trkpt lat="0.0" lon="{}"><time>2026-04-15T16:00:00Z</time></trkpt>
<trkpt lat="0.0" lon="{}"><time>2026-04-15T16:00:30Z</time></trkpt>
</trkseg></trk></gpx>"#,
        dx(0.0),
        dx(100.0),
    );
    let body = multipart_body("file", "short.gpx", &xml);
    let req = Request::builder()
        .method("POST")
        .uri("/api/gpx/analyze")
        .header(
            "Content-Type",
            format!("multipart/form-data; boundary={}", BOUNDARY),
        )
        .body(Body::from(body))
        .unwrap();
    let resp = router().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let body_bytes = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let s = std::str::from_utf8(&body_bytes).unwrap();
    assert!(s.contains("lyhyempi"), "body: {}", s);
}

#[tokio::test]
async fn analyze_endpoint_custom_distance() {
    let body = multipart_body("file", "run.gpx", &synth_gpx());
    let req = Request::builder()
        .method("POST")
        .uri("/api/gpx/analyze?distance_m=200")
        .header(
            "Content-Type",
            format!("multipart/form-data; boundary={}", BOUNDARY),
        )
        .body(Body::from(body))
        .unwrap();
    let resp = router().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body_bytes = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let v: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
    let secs = v["best_lap"]["time_seconds"].as_f64().unwrap();
    // 200 m at 4 m/s = 50 s.
    assert!((secs - 50.0).abs() < 0.5, "got {}", secs);
    assert!((v["best_lap"]["distance_m"].as_f64().unwrap() - 200.0).abs() < 1e-6);
}
