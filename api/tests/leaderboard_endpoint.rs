//! Integration test for /api/leaderboard and /api/tracks/:id/records?period=...
//!
//! Mirrors the gpx_analyze_endpoint test style: build a minimal axum Router with
//! handlers that delegate to the library functions, then drive HTTP requests
//! through it. We avoid pulling in main.rs (binary, not library) — the lib API
//! is the only thing the wire format depends on.

use axum::{
    body::Body,
    extract::{Extension, Path, Query},
    http::{Request, StatusCode},
    response::{IntoResponse, Json, Response},
    routing::get,
    Router,
};
use ratakierros_api::{
    clamp_limit, get_leaderboard, get_records, init_db, resolve_period, Db,
};
use rusqlite::{params, Connection};
use serde::Deserialize;
use std::sync::{Arc, Mutex};
use tower::ServiceExt;

#[derive(Deserialize)]
struct PeriodQuery {
    period: Option<String>,
    month: Option<String>,
    year: Option<String>,
    limit: Option<u32>,
}

async fn records_handler(
    Extension(db): Extension<Db>,
    Path(id): Path<i64>,
    Query(q): Query<PeriodQuery>,
) -> Response {
    let (period, info) = match resolve_period(
        q.period.as_deref(),
        q.month.as_deref(),
        q.year.as_deref(),
    ) {
        Ok(p) => p,
        Err(e) => return (StatusCode::BAD_REQUEST, e).into_response(),
    };
    match get_records(&db, id, None, &period, info, clamp_limit(q.limit)) {
        Ok(d) => Json(serde_json::to_value(d).unwrap()).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    }
}

async fn leaderboard_handler(
    Extension(db): Extension<Db>,
    Query(q): Query<PeriodQuery>,
) -> Response {
    let (period, info) = match resolve_period(
        q.period.as_deref(),
        q.month.as_deref(),
        q.year.as_deref(),
    ) {
        Ok(p) => p,
        Err(e) => return (StatusCode::BAD_REQUEST, e).into_response(),
    };
    match get_leaderboard(&db, None, &period, info, clamp_limit(q.limit)) {
        Ok(d) => Json(serde_json::to_value(d).unwrap()).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    }
}

fn build_app() -> (Router, i64) {
    let conn = Connection::open_in_memory().unwrap();
    init_db(&conn).unwrap();

    conn.execute(
        "INSERT INTO users (email, display_name, password_hash, created_at) \
         VALUES ('a@e.c', 'Alice', 'h', '2026-01-01')",
        [],
    )
    .unwrap();
    let alice = conn.last_insert_rowid();
    conn.execute(
        "INSERT INTO users (email, display_name, password_hash, created_at) \
         VALUES ('b@e.c', 'Bob', 'h', '2026-01-01')",
        [],
    )
    .unwrap();
    let bob = conn.last_insert_rowid();

    conn.execute(
        "INSERT INTO tracks (lipas_id, name, lat, lon, type_code, status, last_synced_at, city) \
         VALUES (1, 'A-rata', 60.0, 24.0, 1220, 'active', '2026-05-05', 'Helsinki')",
        [],
    )
    .unwrap();
    let track_id = conn.last_insert_rowid();

    // Alice: best is April 60.0, plus runs in March and May.
    // Bob: only April 62.0.
    let runs = [
        (alice, 65.0, "2026-03-01T10:00:00+00:00"),
        (alice, 60.0, "2026-04-01T10:00:00+00:00"),
        (alice, 70.0, "2026-05-15T10:00:00+00:00"),
        (bob,   62.0, "2026-04-15T10:00:00+00:00"),
    ];
    for (uid, t, ts) in runs {
        conn.execute(
            "INSERT INTO runs (user_id, track_id, time_seconds, logged_at) VALUES (?1, ?2, ?3, ?4)",
            params![uid, track_id, t, ts],
        )
        .unwrap();
    }

    let _ = (alice, bob); // ids are referenced only when seeding rows above
    let db: Db = Arc::new(Mutex::new(conn));
    let app = Router::new()
        .route("/api/tracks/:id/records", get(records_handler))
        .route("/api/leaderboard", get(leaderboard_handler))
        .layer(Extension(db));
    (app, track_id)
}

async fn json_of(resp: Response) -> serde_json::Value {
    let bytes = hyper::body::to_bytes(resp.into_body()).await.unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

#[tokio::test]
async fn records_default_period_is_all_back_compat() {
    let (app, track_id) = build_app();
    let req = Request::builder()
        .uri(format!("/api/tracks/{}/records", track_id))
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let v = json_of(resp).await;
    assert_eq!(v["period"], "all");
    assert_eq!(v["records"].as_array().unwrap().len(), 4);
    assert_eq!(v["records"][0]["time_seconds"], 60.0);
    assert_eq!(v["records"][0]["rank"], 1);
}

#[tokio::test]
async fn records_period_month_filters_correctly() {
    let (app, track_id) = build_app();
    let req = Request::builder()
        .uri(format!("/api/tracks/{}/records?period=month&month=2026-04", track_id))
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let v = json_of(resp).await;
    assert_eq!(v["period"], "month");
    assert_eq!(v["month"], "2026-04");
    let rs = v["records"].as_array().unwrap();
    assert_eq!(rs.len(), 2);
    assert_eq!(rs[0]["time_seconds"], 60.0);
    assert_eq!(rs[0]["display_name"], "Alice");
    assert_eq!(rs[1]["time_seconds"], 62.0);
}

#[tokio::test]
async fn records_rejects_invalid_month() {
    let (app, track_id) = build_app();
    let req = Request::builder()
        .uri(format!("/api/tracks/{}/records?period=month&month=2026-13", track_id))
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn leaderboard_all_returns_best_per_user_ordered() {
    let (app, _track_id) = build_app();
    let req = Request::builder()
        .uri("/api/leaderboard?period=all")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let v = json_of(resp).await;
    let entries = v["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0]["display_name"], "Alice");
    assert_eq!(entries[0]["time_seconds"], 60.0);
    assert_eq!(entries[0]["track_name"], "A-rata");
    assert_eq!(entries[1]["display_name"], "Bob");
    assert_eq!(entries[1]["time_seconds"], 62.0);
}

#[tokio::test]
async fn leaderboard_period_year_filter() {
    let (app, _track_id) = build_app();
    let req = Request::builder()
        .uri("/api/leaderboard?period=year&year=2026")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let v = json_of(resp).await;
    assert_eq!(v["period"], "year");
    assert_eq!(v["year"], "2026");
    let entries = v["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 2);
}

#[tokio::test]
async fn leaderboard_period_month_excludes_users_with_no_runs_in_window() {
    let (app, _track_id) = build_app();
    // March only has Alice's 65.0 — Bob should be absent.
    let req = Request::builder()
        .uri("/api/leaderboard?period=month&month=2026-03")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let v = json_of(resp).await;
    let entries = v["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0]["display_name"], "Alice");
    assert_eq!(entries[0]["time_seconds"], 65.0);
}

#[tokio::test]
async fn leaderboard_rejects_unknown_period() {
    let (app, _track_id) = build_app();
    let req = Request::builder()
        .uri("/api/leaderboard?period=decade")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}
