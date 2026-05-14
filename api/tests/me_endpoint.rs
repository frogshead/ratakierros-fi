//! Integration test for GET /api/me and PATCH /api/me.
//!
//! Mounts a minimal router with the profile handlers and verifies the JSON
//! shape, validation, and the JWT-refresh-on-name-change behaviour. We
//! duplicate the AuthUser extractor here because main.rs's copy is private.

use axum::{
    body::Body,
    extract::{Extension, FromRequestParts},
    http::{request::Parts, Request, StatusCode},
    response::{IntoResponse, Json, Response},
    routing::get,
    Router,
};
use ratakierros_api::{
    get_user_profile, init_db, make_jwt, update_user_profile, verify_jwt, Db,
};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};
use tower::ServiceExt;

const JWT_SECRET: &str = "test-secret-me-endpoint";

#[derive(Deserialize)]
struct ProfileUpdateBody {
    display_name: String,
    birth_year: Option<i32>,
    gender: Option<String>,
}

#[derive(Serialize)]
struct ProfileResponse {
    user_id: i64,
    email: String,
    display_name: String,
    birth_year: Option<i32>,
    gender: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    token: Option<String>,
}

struct AuthUser(i64);

impl<S> FromRequestParts<S> for AuthUser
where
    S: Send + Sync,
{
    type Rejection = (StatusCode, String);
    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        let token = parts
            .headers
            .get("Authorization")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "))
            .ok_or((StatusCode::UNAUTHORIZED, "no token".into()))?;
        let claims = verify_jwt(token).map_err(|e| (StatusCode::UNAUTHORIZED, e))?;
        Ok(AuthUser(claims.sub))
    }
}

async fn me_get(Extension(db): Extension<Db>, AuthUser(uid): AuthUser) -> Response {
    match get_user_profile(&db, uid) {
        Ok(p) => Json(ProfileResponse {
            user_id: p.user_id,
            email: p.email,
            display_name: p.display_name,
            birth_year: p.birth_year,
            gender: p.gender,
            token: None,
        })
        .into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    }
}

async fn me_patch(
    Extension(db): Extension<Db>,
    AuthUser(uid): AuthUser,
    Json(body): Json<ProfileUpdateBody>,
) -> Response {
    let prev = match get_user_profile(&db, uid) {
        Ok(p) => p.display_name,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    };
    let updated = match update_user_profile(
        &db,
        uid,
        &body.display_name,
        body.birth_year,
        body.gender.as_deref(),
    ) {
        Ok(p) => p,
        Err(e) => return (StatusCode::BAD_REQUEST, e).into_response(),
    };
    let token = if prev != updated.display_name {
        make_jwt(uid, &updated.display_name).ok()
    } else {
        None
    };
    Json(ProfileResponse {
        user_id: updated.user_id,
        email: updated.email,
        display_name: updated.display_name,
        birth_year: updated.birth_year,
        gender: updated.gender,
        token,
    })
    .into_response()
}

fn build_app() -> (Router, i64, String) {
    std::env::set_var("JWT_SECRET", JWT_SECRET);
    let conn = Connection::open_in_memory().unwrap();
    init_db(&conn).unwrap();
    conn.execute(
        "INSERT INTO users (email, display_name, password_hash, created_at) \
         VALUES ('me@e.c', 'Me User', 'h', '2026-01-01')",
        [],
    )
    .unwrap();
    let uid = conn.last_insert_rowid();
    let db: Db = Arc::new(Mutex::new(conn));
    let token = make_jwt(uid, "Me User").unwrap();
    let app = Router::new()
        .route("/api/me", get(me_get).patch(me_patch))
        .layer(Extension(db));
    (app, uid, token)
}

async fn json_of(resp: Response) -> serde_json::Value {
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

#[tokio::test]
async fn me_get_requires_auth_token() {
    let (app, _, _) = build_app();
    let req = Request::builder().uri("/api/me").body(Body::empty()).unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn me_get_returns_profile_with_nullable_fields() {
    let (app, uid, token) = build_app();
    let req = Request::builder()
        .uri("/api/me")
        .header("Authorization", format!("Bearer {}", token))
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let v = json_of(resp).await;
    assert_eq!(v["user_id"], uid);
    assert_eq!(v["display_name"], "Me User");
    assert!(v["birth_year"].is_null());
    assert!(v["gender"].is_null());
    assert!(v.get("token").is_none() || v["token"].is_null());
}

#[tokio::test]
async fn me_patch_sets_profile_fields() {
    let (app, _, token) = build_app();
    let body = serde_json::json!({
        "display_name": "Me User",
        "birth_year": 1984,
        "gender": "M",
    });
    let req = Request::builder()
        .method("PATCH")
        .uri("/api/me")
        .header("Authorization", format!("Bearer {}", token))
        .header("Content-Type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let v = json_of(resp).await;
    assert_eq!(v["birth_year"], 1984);
    assert_eq!(v["gender"], "M");
    // Display name unchanged → no fresh token.
    assert!(v.get("token").is_none() || v["token"].is_null());
}

#[tokio::test]
async fn me_patch_returns_fresh_token_when_display_name_changes() {
    let (app, _, token) = build_app();
    let body = serde_json::json!({
        "display_name": "Renamed",
        "birth_year": null,
        "gender": null,
    });
    let req = Request::builder()
        .method("PATCH")
        .uri("/api/me")
        .header("Authorization", format!("Bearer {}", token))
        .header("Content-Type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let v = json_of(resp).await;
    assert_eq!(v["display_name"], "Renamed");
    let new_token = v["token"].as_str().expect("fresh token should be present");
    let claims = verify_jwt(new_token).unwrap();
    assert_eq!(claims.display_name, "Renamed");
}

#[tokio::test]
async fn me_patch_rejects_invalid_birth_year() {
    let (app, _, token) = build_app();
    let body = serde_json::json!({
        "display_name": "Me User",
        "birth_year": 1800,
        "gender": null,
    });
    let req = Request::builder()
        .method("PATCH")
        .uri("/api/me")
        .header("Authorization", format!("Bearer {}", token))
        .header("Content-Type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn me_patch_rejects_empty_display_name() {
    let (app, _, token) = build_app();
    let body = serde_json::json!({
        "display_name": "   ",
        "birth_year": null,
        "gender": null,
    });
    let req = Request::builder()
        .method("PATCH")
        .uri("/api/me")
        .header("Authorization", format!("Bearer {}", token))
        .header("Content-Type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}
