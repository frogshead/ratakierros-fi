use axum::{extract::Query, response::Json, routing::get, Router};
use serde::Deserialize;
use tower_http::cors::CorsLayer;

use ratakierros_api::{get_closest_track, TrackResult};

#[derive(Deserialize)]
struct ClosestQuery {
    lat: f64,
    lon: f64,
    radius: Option<f64>,
}

async fn closest_handler(Query(params): Query<ClosestQuery>) -> Json<TrackResult> {
    let radius = params.radius.unwrap_or(5000.0);
    match get_closest_track(params.lat, params.lon, radius).await {
        Ok(result) => Json(result),
        Err(e) => {
            eprintln!("Error: {}", e);
            Json(TrackResult {
            found: false,
            track: None,
        })
        }
    }
}

async fn health_handler() -> &'static str {
    "ok"
}

#[tokio::main]
async fn main() {
    let app = Router::new()
        .route("/api/closest", get(closest_handler))
        .route("/health", get(health_handler))
        .layer(CorsLayer::permissive());

    println!("API server running on http://0.0.0.0:3000");
    axum::Server::bind(&"0.0.0.0:3000".parse().unwrap())
        .serve(app.into_make_service())
        .await
        .unwrap();
}
