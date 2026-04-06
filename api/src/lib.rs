use geo::HaversineDistance;
use geo_types::Point;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct TrackResult {
    pub found: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub track: Option<TrackInfo>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TrackInfo {
    pub lat: f64,
    pub lon: f64,
    pub name: Option<String>,
    pub distance_m: f64,
}

pub async fn get_closest_track(lat: f64, lon: f64, radius: f64) -> Result<TrackResult, String> {
    let query = format!(
        r#"[out:json];(node["sport"="athletics"](around:{radius},{lat},{lon});way["sport"="athletics"](around:{radius},{lat},{lon});relation["sport"="athletics"](around:{radius},{lat},{lon}););out center;"#,
        radius = radius,
        lat = lat,
        lon = lon,
    );

    let client = reqwest::Client::new();
    let response = client
        .post("https://overpass-api.de/api/interpreter")
        .form(&[("data", &query)])
        .send()
        .await
        .map_err(|e| format!("Overpass request failed: {}", e))?;

    let data: serde_json::Value = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse Overpass response: {}", e))?;

    let elements = match data["elements"].as_array() {
        Some(elems) => elems,
        None => return Ok(TrackResult { found: false, track: None }),
    };

    let user_point = Point::new(lon, lat);
    let mut closest: Option<(f64, f64, Option<String>, f64)> = None;

    for element in elements {
        let (elat, elon) = if element["type"] == "node" {
            (
                element["lat"].as_f64().unwrap_or(0.0),
                element["lon"].as_f64().unwrap_or(0.0),
            )
        } else if let Some(center) = element.get("center") {
            (
                center["lat"].as_f64().unwrap_or(0.0),
                center["lon"].as_f64().unwrap_or(0.0),
            )
        } else {
            continue;
        };

        if elat == 0.0 && elon == 0.0 {
            continue;
        }

        let track_point = Point::new(elon, elat);
        let distance = user_point.haversine_distance(&track_point);
        let name = element["tags"]["name"].as_str().map(String::from);

        match &closest {
            Some((_, _, _, d)) if distance >= *d => {}
            _ => closest = Some((elat, elon, name, distance)),
        }
    }

    match closest {
        Some((tlat, tlon, name, distance_m)) => Ok(TrackResult {
            found: true,
            track: Some(TrackInfo {
                lat: tlat,
                lon: tlon,
                name,
                distance_m,
            }),
        }),
        None => Ok(TrackResult { found: false, track: None }),
    }
}
