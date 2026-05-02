use chrono::Utc;
use rusqlite::params;
use serde_json::Value;

use crate::Db;

const LIPAS_BASE: &str = "https://api.lipas.fi/v2";
const PAGE_SIZE: usize = 100;
const USER_AGENT: &str = "ratakierros-fi/1.0 (+https://ratakierros.fi)";

pub struct LipasTrack {
    pub lipas_id: i64,
    pub name: Option<String>,
    pub lat: f64,
    pub lon: f64,
    pub type_code: i64,
    pub status: String,
    pub address: Option<String>,
    pub postal_code: Option<String>,
    pub city: Option<String>,
    pub suburb: Option<String>,
    pub surface: Option<String>,
    pub track_length_m: Option<i64>,
    pub lanes: Option<i64>,
    pub geometry_geojson: Option<String>,
}

pub async fn fetch_and_cache_lipas_tracks(db: Db) -> Result<usize, String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(60))
        .build()
        .map_err(|e| format!("HTTP client error: {}", e))?;

    let mut all_tracks: Vec<LipasTrack> = Vec::new();
    let mut page = 1;

    loop {
        let url = format!(
            "{}/sports-sites?type-codes=1220&statuses=active,out-of-service-temporarily&page-size={}&page={}",
            LIPAS_BASE, PAGE_SIZE, page
        );

        let resp = client
            .get(&url)
            .header("User-Agent", USER_AGENT)
            .send()
            .await
            .map_err(|e| format!("Lipas request failed (page {}): {}", page, e))?;

        let status = resp.status();
        let body = resp
            .text()
            .await
            .map_err(|e| format!("Failed to read Lipas response: {}", e))?;
        if !status.is_success() {
            return Err(format!(
                "Lipas returned HTTP {}: {}",
                status,
                &body[..body.len().min(200)]
            ));
        }

        let data: Value = serde_json::from_str(&body)
            .map_err(|e| format!("Failed to parse Lipas JSON (page {}): {}", page, e))?;

        let items = data["items"]
            .as_array()
            .ok_or_else(|| "Missing 'items' in Lipas response".to_string())?;

        let mut skipped_no_geom = 0;
        for item in items {
            match parse_lipas_item(item) {
                Some(t) => all_tracks.push(t),
                None => skipped_no_geom += 1,
            }
        }

        let total_pages = data["pagination"]["total-pages"].as_i64().unwrap_or(1);
        let total_items = data["pagination"]["total-items"].as_i64().unwrap_or(0);
        println!(
            "Lipas page {}/{}: parsed {} (skipped {} without geometry); total items reported: {}",
            page,
            total_pages,
            items.len() - skipped_no_geom,
            skipped_no_geom,
            total_items
        );

        if (page as i64) >= total_pages {
            break;
        }
        page += 1;
    }

    let inserted = {
        let conn = db.lock().unwrap();
        let now = Utc::now().to_rfc3339();
        let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
        let mut count = 0usize;
        for t in &all_tracks {
            tx.execute(
                "INSERT INTO tracks (lipas_id, name, lat, lon, type_code, status, address, \
                 postal_code, city, suburb, surface, track_length_m, lanes, geometry_geojson, \
                 last_synced_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15) \
                 ON CONFLICT(lipas_id) DO UPDATE SET \
                    name = excluded.name, \
                    lat = excluded.lat, \
                    lon = excluded.lon, \
                    type_code = excluded.type_code, \
                    status = excluded.status, \
                    address = excluded.address, \
                    postal_code = excluded.postal_code, \
                    city = excluded.city, \
                    suburb = excluded.suburb, \
                    surface = excluded.surface, \
                    track_length_m = excluded.track_length_m, \
                    lanes = excluded.lanes, \
                    geometry_geojson = excluded.geometry_geojson, \
                    last_synced_at = excluded.last_synced_at",
                params![
                    t.lipas_id,
                    t.name,
                    t.lat,
                    t.lon,
                    t.type_code,
                    t.status,
                    t.address,
                    t.postal_code,
                    t.city,
                    t.suburb,
                    t.surface,
                    t.track_length_m,
                    t.lanes,
                    t.geometry_geojson,
                    now,
                ],
            )
            .map_err(|e| format!("Insert failed for lipas-id {}: {}", t.lipas_id, e))?;
            count += 1;
        }
        tx.commit().map_err(|e| e.to_string())?;
        count
    };

    Ok(inserted)
}

fn parse_lipas_item(item: &Value) -> Option<LipasTrack> {
    let lipas_id = item["lipas-id"].as_i64()?;
    let type_code = item["type"]["type-code"].as_i64()?;
    let status = item["status"].as_str()?.to_string();
    let name = item["name"].as_str().map(String::from);

    let location = &item["location"];
    let address = location["address"].as_str().map(String::from);
    let postal_code = location["postal-code"].as_str().map(String::from);
    let suburb = location["city"]["neighborhood"].as_str().map(String::from);
    let city = location["postal-office"]
        .as_str()
        .map(title_case_finnish);

    let geometries = &location["geometries"];
    let (lat, lon, geometry_geojson) = extract_geometry(geometries)?;

    let props = &item["properties"];
    let track_length_m = props["inner-lane-length-m"].as_i64();
    let lanes = props["circular-lanes-count"].as_i64();
    let surface = props["running-track-surface-material"]
        .as_str()
        .map(String::from)
        .or_else(|| {
            props["surface-material"]
                .as_array()
                .and_then(|a| a.first())
                .and_then(|v| v.as_str())
                .map(String::from)
        });

    Some(LipasTrack {
        lipas_id,
        name,
        lat,
        lon,
        type_code,
        status,
        address,
        postal_code,
        city,
        suburb,
        surface,
        track_length_m,
        lanes,
        geometry_geojson,
    })
}

// Returns (centroid_lat, centroid_lon, raw_geometries_geojson_string).
// Lipas geometry is a FeatureCollection of features whose geometry can be Point/Polygon/MultiPolygon.
// For Point we use the coords; for Polygon/MultiPolygon we use the bbox center.
fn extract_geometry(geometries: &Value) -> Option<(f64, f64, Option<String>)> {
    let features = geometries["features"].as_array()?;
    if features.is_empty() {
        return None;
    }

    let mut min_lon = f64::INFINITY;
    let mut max_lon = f64::NEG_INFINITY;
    let mut min_lat = f64::INFINITY;
    let mut max_lat = f64::NEG_INFINITY;
    let mut got_any = false;

    for feature in features {
        let geom = &feature["geometry"];
        let gtype = geom["type"].as_str().unwrap_or("");
        let coords = &geom["coordinates"];
        match gtype {
            "Point" => {
                if let Some((lon, lat)) = coord_pair(coords) {
                    update_bbox(&mut min_lon, &mut max_lon, &mut min_lat, &mut max_lat, lon, lat);
                    got_any = true;
                }
            }
            "Polygon" => {
                if let Some(rings) = coords.as_array() {
                    for ring in rings {
                        walk_coords(ring, &mut min_lon, &mut max_lon, &mut min_lat, &mut max_lat, &mut got_any);
                    }
                }
            }
            "MultiPolygon" => {
                if let Some(polys) = coords.as_array() {
                    for poly in polys {
                        if let Some(rings) = poly.as_array() {
                            for ring in rings {
                                walk_coords(ring, &mut min_lon, &mut max_lon, &mut min_lat, &mut max_lat, &mut got_any);
                            }
                        }
                    }
                }
            }
            "LineString" => {
                walk_coords(coords, &mut min_lon, &mut max_lon, &mut min_lat, &mut max_lat, &mut got_any);
            }
            _ => {}
        }
    }

    if !got_any {
        return None;
    }

    let lat = (min_lat + max_lat) / 2.0;
    let lon = (min_lon + max_lon) / 2.0;
    let raw = serde_json::to_string(geometries).ok();
    Some((lat, lon, raw))
}

fn walk_coords(arr: &Value, min_lon: &mut f64, max_lon: &mut f64, min_lat: &mut f64, max_lat: &mut f64, got_any: &mut bool) {
    if let Some(list) = arr.as_array() {
        for c in list {
            if let Some((lon, lat)) = coord_pair(c) {
                update_bbox(min_lon, max_lon, min_lat, max_lat, lon, lat);
                *got_any = true;
            }
        }
    }
}

fn coord_pair(v: &Value) -> Option<(f64, f64)> {
    let arr = v.as_array()?;
    let lon = arr.first()?.as_f64()?;
    let lat = arr.get(1)?.as_f64()?;
    Some((lon, lat))
}

fn update_bbox(min_lon: &mut f64, max_lon: &mut f64, min_lat: &mut f64, max_lat: &mut f64, lon: f64, lat: f64) {
    if lon < *min_lon { *min_lon = lon; }
    if lon > *max_lon { *max_lon = lon; }
    if lat < *min_lat { *min_lat = lat; }
    if lat > *max_lat { *max_lat = lat; }
}

// "UUSIKAUPUNKI" -> "Uusikaupunki", "Helsinki" -> "Helsinki"
fn title_case_finnish(s: &str) -> String {
    if s.chars().any(|c| c.is_lowercase()) {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut at_word_start = true;
    for c in s.chars() {
        if c.is_whitespace() || c == '-' {
            out.push(c);
            at_word_start = true;
        } else if at_word_start {
            out.extend(c.to_uppercase());
            at_word_start = false;
        } else {
            out.extend(c.to_lowercase());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn title_case_uppercase() {
        assert_eq!(title_case_finnish("UUSIKAUPUNKI"), "Uusikaupunki");
        assert_eq!(title_case_finnish("VESANTO"), "Vesanto");
    }

    #[test]
    fn title_case_keeps_mixed() {
        assert_eq!(title_case_finnish("Helsinki"), "Helsinki");
        assert_eq!(title_case_finnish("Nurmes"), "Nurmes");
    }

    #[test]
    fn title_case_compound() {
        assert_eq!(title_case_finnish("PIETARSAARI-JAKOBSTAD"), "Pietarsaari-Jakobstad");
    }

    #[test]
    fn parse_full_record() {
        let json = serde_json::json!({
            "lipas-id": 12345,
            "name": "Testikenttä",
            "type": { "type-code": 1220 },
            "status": "active",
            "location": {
                "address": "Testitie 1",
                "postal-code": "00100",
                "postal-office": "HELSINKI",
                "city": { "city-code": 91, "neighborhood": "Töölö" },
                "geometries": {
                    "type": "FeatureCollection",
                    "features": [{
                        "type": "Feature",
                        "geometry": { "type": "Point", "coordinates": [24.93, 60.17] }
                    }]
                }
            },
            "properties": {
                "inner-lane-length-m": 400,
                "circular-lanes-count": 8,
                "running-track-surface-material": "synthetic"
            }
        });

        let t = parse_lipas_item(&json).expect("should parse");
        assert_eq!(t.lipas_id, 12345);
        assert_eq!(t.name.as_deref(), Some("Testikenttä"));
        assert_eq!(t.type_code, 1220);
        assert_eq!(t.status, "active");
        assert_eq!(t.lat, 60.17);
        assert_eq!(t.lon, 24.93);
        assert_eq!(t.city.as_deref(), Some("Helsinki"));
        assert_eq!(t.suburb.as_deref(), Some("Töölö"));
        assert_eq!(t.track_length_m, Some(400));
        assert_eq!(t.lanes, Some(8));
        assert_eq!(t.surface.as_deref(), Some("synthetic"));
    }

    #[test]
    fn parse_skips_no_geometry() {
        let json = serde_json::json!({
            "lipas-id": 999,
            "type": { "type-code": 1220 },
            "status": "active",
            "location": {
                "geometries": { "type": "FeatureCollection", "features": [] }
            },
            "properties": {}
        });
        assert!(parse_lipas_item(&json).is_none());
    }

    // Live integration test — hits api.lipas.fi. Run with `cargo test -- --ignored`.
    #[tokio::test]
    #[ignore]
    async fn live_fetch_populates_db() {
        use rusqlite::Connection;
        use std::sync::{Arc, Mutex};

        let conn = Connection::open_in_memory().expect("open in-mem db");
        crate::init_db(&conn).expect("init schema");
        let db: crate::Db = Arc::new(Mutex::new(conn));

        let n = super::fetch_and_cache_lipas_tracks(db.clone())
            .await
            .expect("fetch should succeed");
        assert!(n >= 200, "expected at least 200 tracks, got {}", n);

        let conn = db.lock().unwrap();
        let with_lanes: i64 = conn
            .query_row("SELECT COUNT(*) FROM tracks WHERE lanes IS NOT NULL", [], |r| r.get(0))
            .unwrap();
        let in_finland: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM tracks WHERE lat BETWEEN 59 AND 71 AND lon BETWEEN 19 AND 32",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(in_finland as usize, n, "all tracks should be inside Finland bbox");
        assert!(with_lanes > 0, "at least some tracks should have lane count");

        let sample: (i64, Option<String>, Option<String>, Option<i64>, Option<i64>) = conn
            .query_row(
                "SELECT lipas_id, name, city, track_length_m, lanes FROM tracks LIMIT 1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
            )
            .unwrap();
        eprintln!("Sample row: {:?}", sample);
    }

    #[test]
    fn parse_polygon_uses_bbox_center() {
        let json = serde_json::json!({
            "lipas-id": 7,
            "type": { "type-code": 1220 },
            "status": "active",
            "location": {
                "geometries": {
                    "type": "FeatureCollection",
                    "features": [{
                        "type": "Feature",
                        "geometry": {
                            "type": "Polygon",
                            "coordinates": [[
                                [25.0, 60.0],
                                [25.2, 60.0],
                                [25.2, 60.2],
                                [25.0, 60.2],
                                [25.0, 60.0]
                            ]]
                        }
                    }]
                }
            },
            "properties": {}
        });
        let t = parse_lipas_item(&json).expect("should parse");
        assert!((t.lat - 60.1).abs() < 1e-9);
        assert!((t.lon - 25.1).abs() < 1e-9);
    }
}
