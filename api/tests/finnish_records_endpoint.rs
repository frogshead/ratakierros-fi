//! Integration test for GET /api/finnish-records.
//!
//! The endpoint is anonymous (no auth required) — it returns curated reference
//! data with the two seeded open-class records and any masters records an
//! admin has inserted.

use axum::{
    body::Body,
    extract::Extension,
    http::{Request, StatusCode},
    response::{IntoResponse, Json, Response},
    routing::get,
    Router,
};
use ratakierros_api::{init_db, list_finnish_records, Db};
use rusqlite::Connection;
use std::sync::{Arc, Mutex};
use tower::ServiceExt;

async fn handler(Extension(db): Extension<Db>) -> Response {
    match list_finnish_records(&db) {
        Ok(rs) => Json(serde_json::json!({ "records": rs })).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    }
}

fn build_app() -> Router {
    let conn = Connection::open_in_memory().unwrap();
    init_db(&conn).unwrap();
    let db: Db = Arc::new(Mutex::new(conn));
    Router::new()
        .route("/api/finnish-records", get(handler))
        .layer(Extension(db))
}

#[tokio::test]
async fn returns_seeded_open_class_records() {
    let app = build_app();
    let req = Request::builder()
        .uri("/api/finnish-records")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let records = v["records"].as_array().expect("records[] array");
    assert!(records.len() >= 2, "expected at least 2 seeded records, got {}", records.len());

    let m = &records[0];
    assert_eq!(m["category"], "OPEN_M");
    assert_eq!(m["time_seconds"], 45.49);
    assert_eq!(m["holder_name"], "Markku Kukkoaho");
    assert_eq!(m["set_year"], 1972);

    let n = &records[1];
    assert_eq!(n["category"], "OPEN_N");
    assert_eq!(n["time_seconds"], 50.14);
}
