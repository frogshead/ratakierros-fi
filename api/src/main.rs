use async_trait::async_trait;
use axum::{
    extract::{Extension, FromRequestParts, Multipart, Path, Query},
    http::{request::Parts, StatusCode},
    response::{IntoResponse, Json, Response},
    routing::{get, post},
    Router,
};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};
use tower_http::cors::CorsLayer;

use ratakierros_api::{
    analyze_gpx, fetch_and_cache_lipas_tracks, finalize_legacy_migration, get_records, get_track,
    list_tracks, log_run, login_user, migrate_db, register_user, tracks_count, verify_jwt,
    AnalyzeError, Claims, Db, DEFAULT_TARGET_DISTANCE_M,
};

// --- Error type ---

enum AppError {
    Unauthorized(String),
    BadRequest(String),
    NotFound,
    Internal(String),
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, msg) = match self {
            AppError::Unauthorized(m) => (StatusCode::UNAUTHORIZED, m),
            AppError::BadRequest(m) => (StatusCode::BAD_REQUEST, m),
            AppError::NotFound => (StatusCode::NOT_FOUND, "Not found".to_string()),
            AppError::Internal(m) => (StatusCode::INTERNAL_SERVER_ERROR, m),
        };
        (status, Json(serde_json::json!({ "error": msg }))).into_response()
    }
}

// --- Auth extractors ---

struct AuthUser(i64);

#[async_trait]
impl<S> FromRequestParts<S> for AuthUser
where
    S: Send + Sync,
{
    type Rejection = AppError;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        let token = parts
            .headers
            .get("Authorization")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "))
            .ok_or_else(|| AppError::Unauthorized("No token provided".to_string()))?;

        let claims =
            verify_jwt(token).map_err(|e| AppError::Unauthorized(e))?;

        Ok(AuthUser(claims.sub))
    }
}

struct OptionalAuthUser(Option<i64>);

#[async_trait]
impl<S> FromRequestParts<S> for OptionalAuthUser
where
    S: Send + Sync,
{
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        let user_id = parts
            .headers
            .get("Authorization")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "))
            .and_then(|token| verify_jwt(token).ok())
            .map(|c: Claims| c.sub);

        Ok(OptionalAuthUser(user_id))
    }
}

// --- Request / response types ---

#[derive(Deserialize)]
struct TracksQuery {
    lat: Option<f64>,
    lon: Option<f64>,
    q: Option<String>,
}

#[derive(Deserialize)]
struct RegisterBody {
    email: String,
    display_name: String,
    password: String,
}

#[derive(Deserialize)]
struct LoginBody {
    email: String,
    password: String,
}

#[derive(Deserialize)]
struct LogRunBody {
    track_id: i64,
    time_seconds: f64,
}

#[derive(Serialize)]
struct AuthResponse {
    token: String,
    user_id: i64,
    display_name: String,
}

// --- Handlers ---

async fn health_handler() -> &'static str {
    "ok"
}

async fn tracks_handler(
    Extension(db): Extension<Db>,
    Query(params): Query<TracksQuery>,
) -> impl IntoResponse {
    match list_tracks(&db, params.lat, params.lon, params.q.as_deref()) {
        Ok(tracks) => Json(tracks).into_response(),
        Err(e) => AppError::Internal(e).into_response(),
    }
}

async fn track_handler(
    Extension(db): Extension<Db>,
    Path(id): Path<i64>,
) -> impl IntoResponse {
    match get_track(&db, id) {
        Ok(Some(track)) => Json(track).into_response(),
        Ok(None) => AppError::NotFound.into_response(),
        Err(e) => AppError::Internal(e).into_response(),
    }
}

async fn records_handler(
    Extension(db): Extension<Db>,
    Path(id): Path<i64>,
    OptionalAuthUser(user_id): OptionalAuthUser,
) -> impl IntoResponse {
    match get_records(&db, id, user_id) {
        Ok(data) => Json(data).into_response(),
        Err(e) => AppError::Internal(e).into_response(),
    }
}

async fn log_run_handler(
    Extension(db): Extension<Db>,
    AuthUser(user_id): AuthUser,
    Json(body): Json<LogRunBody>,
) -> impl IntoResponse {
    if body.time_seconds <= 0.0 || body.time_seconds > 600.0 {
        return AppError::BadRequest("Aika on virheellinen (0–600 s)".to_string()).into_response();
    }
    match log_run(&db, user_id, body.track_id, body.time_seconds) {
        Ok(()) => (StatusCode::CREATED, Json(serde_json::json!({ "ok": true }))).into_response(),
        Err(e) => AppError::Internal(e).into_response(),
    }
}

async fn register_handler(
    Extension(db): Extension<Db>,
    Json(body): Json<RegisterBody>,
) -> impl IntoResponse {
    if body.email.is_empty() || body.display_name.is_empty() || body.password.len() < 6 {
        return AppError::BadRequest(
            "Täytä kaikki kentät (salasana vähintään 6 merkkiä)".to_string(),
        )
        .into_response();
    }
    match register_user(&db, &body.email, &body.display_name, &body.password) {
        Ok((token, user_id, display_name)) => {
            Json(AuthResponse { token, user_id, display_name }).into_response()
        }
        Err(e) => AppError::BadRequest(e).into_response(),
    }
}

async fn login_handler(
    Extension(db): Extension<Db>,
    Json(body): Json<LoginBody>,
) -> impl IntoResponse {
    match login_user(&db, &body.email, &body.password) {
        Ok((token, user_id, display_name)) => {
            Json(AuthResponse { token, user_id, display_name }).into_response()
        }
        Err(e) => AppError::Unauthorized(e).into_response(),
    }
}

async fn gpx_analyze_handler(
    Query(params): Query<GpxAnalyzeParams>,
    mut multipart: Multipart,
) -> Result<Json<serde_json::Value>, AppError> {
    let mut gpx_bytes: Option<Vec<u8>> = None;
    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| AppError::BadRequest(format!("multipart-virhe: {}", e)))?
    {
        let name = field.name().unwrap_or("").to_string();
        if name == "file" || name == "gpx" {
            let data = field
                .bytes()
                .await
                .map_err(|e| AppError::BadRequest(format!("tiedostoa ei voitu lukea: {}", e)))?;
            gpx_bytes = Some(data.to_vec());
        }
    }
    let bytes = gpx_bytes
        .ok_or_else(|| AppError::BadRequest("kentta 'file' puuttuu pyynnosta".to_string()))?;
    let gpx_str = std::str::from_utf8(&bytes)
        .map_err(|_| AppError::BadRequest("GPX ei ole UTF-8".to_string()))?;
    let target = params.distance_m.unwrap_or(DEFAULT_TARGET_DISTANCE_M);
    if !target.is_finite() || target <= 0.0 {
        return Err(AppError::BadRequest("distance_m taytyy olla > 0".into()));
    }
    let result = analyze_gpx(gpx_str, target).map_err(map_analyze_error)?;
    Ok(Json(serde_json::to_value(result).unwrap()))
}

fn map_analyze_error(e: AnalyzeError) -> AppError {
    AppError::BadRequest(e.to_string())
}

#[derive(Deserialize)]
struct GpxAnalyzeParams {
    distance_m: Option<f64>,
}

async fn refresh_tracks_handler(Extension(db): Extension<Db>) -> impl IntoResponse {
    match fetch_and_cache_lipas_tracks(db.clone()).await {
        Ok(n) => {
            let (remapped, orphaned) = finalize_legacy_migration(&db).unwrap_or((0, 0));
            Json(serde_json::json!({
                "fetched": n,
                "remapped_runs": remapped,
                "orphaned_tracks": orphaned,
            }))
            .into_response()
        }
        Err(e) => AppError::Internal(e).into_response(),
    }
}

// --- Main ---

#[tokio::main]
async fn main() {
    let db_path =
        std::env::var("DATABASE_PATH").unwrap_or_else(|_| "ratakierros.db".to_string());

    let conn = rusqlite::Connection::open(&db_path).expect("Failed to open database");
    // migrate_db must run before init_db so legacy `tracks` is renamed before the new
    // schema is created.
    migrate_db(&conn);
    ratakierros_api::init_db(&conn).expect("Failed to initialize database");

    let db: Db = Arc::new(Mutex::new(conn));

    let needs_lipas_seed = tracks_count(&db) == 0 || {
        let c = db.lock().unwrap();
        c.query_row(
            "SELECT 1 FROM sqlite_master WHERE type='table' AND name='tracks_legacy'",
            [],
            |_| Ok(()),
        )
        .is_ok()
    };

    if needs_lipas_seed {
        println!("Fetching tracks from Lipas API v2...");
        let db_clone = db.clone();
        tokio::spawn(async move {
            match fetch_and_cache_lipas_tracks(db_clone.clone()).await {
                Ok(n) => {
                    println!("Fetched and cached {} tracks from Lipas", n);
                    match finalize_legacy_migration(&db_clone) {
                        Ok((remapped, orphaned)) if remapped > 0 || orphaned > 0 => {
                            println!(
                                "Legacy migration: {} runs rehomed onto Lipas tracks, \
                                 {} orphan tracks preserved (had runs but no Lipas match)",
                                remapped, orphaned
                            );
                        }
                        _ => {}
                    }
                }
                Err(e) => eprintln!("Lipas fetch failed: {}", e),
            }
        });
    } else {
        println!("Loaded {} tracks from database", tracks_count(&db));
    }

    let app = Router::new()
        .route("/health", get(health_handler))
        .route("/api/tracks", get(tracks_handler))
        .route("/api/tracks/:id", get(track_handler))
        .route("/api/tracks/:id/records", get(records_handler))
        .route("/api/runs", post(log_run_handler))
        .route("/api/auth/register", post(register_handler))
        .route("/api/auth/login", post(login_handler))
        .route("/api/gpx/analyze", post(gpx_analyze_handler))
        .route("/api/admin/refresh-tracks", post(refresh_tracks_handler))
        .layer(Extension(db))
        .layer(CorsLayer::permissive());

    println!("API server running on http://0.0.0.0:3000");
    axum::Server::bind(&"0.0.0.0:3000".parse().unwrap())
        .serve(app.into_make_service())
        .await
        .unwrap();
}
