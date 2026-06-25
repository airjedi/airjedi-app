use chrono::Utc;
use serde_json::Value;

use crate::data_ingest::canonical::{CanonicalRecord, TfrInfo};
use crate::data_ingest::pipeline::{PipelineData, PipelineError, PipelinePhase, PipelineStage};
use crate::data_ingest::provider::{
    DataProvider, FetchContext, ProviderCategory, ProviderError, ProviderMeta, RawFetchResult,
};

const FAA_TFR_WFS_URL: &str = "https://tfr.faa.gov/geoserver/TFR/ows";

/// Data provider that fetches TFRs from the FAA GeoServer WFS endpoint.
pub struct TfrProvider;

impl DataProvider for TfrProvider {
    fn name(&self) -> &str {
        "faa_tfr"
    }

    fn schedule(&self) -> &str {
        "0 */15 * * * *"
    }

    fn fetch(&self, _ctx: &FetchContext) -> Result<RawFetchResult, ProviderError> {
        let url = format!(
            "{}?service=WFS&version=1.1.0&request=GetFeature&typeName=TFR:V_TFR_LOC&maxFeatures=300&outputFormat=application%2Fjson&srsname=EPSG:4326",
            FAA_TFR_WFS_URL
        );
        let result = crate::data_ingest::http_cache::fetch_with_cache(&url, "faa_tfr.geojson", 30)?;
        Ok(RawFetchResult {
            data: result.into_bytes(),
            content_type: Some("application/json".to_string()),
            source: FAA_TFR_WFS_URL.to_string(),
        })
    }

    fn pipeline_stages(&self) -> Vec<Box<dyn PipelineStage>> {
        vec![Box::new(TfrParseStage), Box::new(TfrValidateStage)]
    }

    fn metadata(&self) -> ProviderMeta {
        ProviderMeta {
            display_name: "TFRs",
            category: ProviderCategory::Notices,
            description: "Temporary Flight Restrictions from FAA",
            config_key: "tfr",
        }
    }
}

/// Parse stage: extracts TFR records from GeoJSON features.
struct TfrParseStage;

impl PipelineStage for TfrParseStage {
    fn name(&self) -> &str {
        "tfr_parse"
    }

    fn phase(&self) -> PipelinePhase {
        PipelinePhase::Parse
    }

    fn execute(&self, data: &mut PipelineData) -> Result<(), PipelineError> {
        let raw = data.raw_bytes.take().unwrap_or_default();
        if raw.is_empty() {
            return Ok(());
        }

        let json: Value = serde_json::from_slice(&raw).map_err(|e| PipelineError::StageError {
            stage: self.name().to_string(),
            message: format!("JSON parse error: {}", e),
        })?;

        // Check for ArcGIS error response
        if json.get("error").is_some() {
            bevy::log::warn!(
                "TFR ArcGIS API returned error: {}",
                json.get("error")
                    .and_then(|e| e.get("message"))
                    .and_then(|m| m.as_str())
                    .unwrap_or("unknown")
            );
            return Ok(());
        }

        let features = match json.get("features").and_then(|f| f.as_array()) {
            Some(arr) => arr,
            None => {
                bevy::log::warn!("TFR response missing 'features' array");
                return Ok(());
            }
        };

        let now = Utc::now();

        for feature in features {
            let tfr = match parse_tfr_feature(feature, now) {
                Some(t) => t,
                None => continue,
            };
            data.records.push(CanonicalRecord::Tfr(tfr));
        }

        Ok(())
    }
}

/// Parse a single GeoJSON feature into a TfrInfo.
/// Supports both the old ArcGIS format and the new GeoServer WFS format.
fn parse_tfr_feature(feature: &Value, fetched_at: chrono::DateTime<Utc>) -> Option<TfrInfo> {
    let props = feature.get("properties")?;

    // NOTAM ID: WFS uses NOTAM_KEY (e.g. "6/3691-1-FDC-F"), ArcGIS used NOTAM
    let notam_id = props
        .get("NOTAM_KEY")
        .or_else(|| props.get("NOTAM"))
        .or_else(|| props.get("notam"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    if notam_id.is_empty() {
        return None;
    }

    // Name/title: WFS uses TITLE, ArcGIS used NAME
    let name = props
        .get("TITLE")
        .or_else(|| props.get("NAME"))
        .or_else(|| props.get("name"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    // Type: WFS uses LEGAL, ArcGIS used TYPE
    let tfr_type = props
        .get("LEGAL")
        .or_else(|| props.get("TYPE"))
        .or_else(|| props.get("type"))
        .and_then(|v| v.as_str())
        .unwrap_or("UNKNOWN")
        .to_string();

    // Effective start: WFS uses LAST_MODIFICATION_DATETIME (yyyyMMddHHmm),
    // ArcGIS used EFFECTIVE (epoch millis)
    let effective_start = props
        .get("LAST_MODIFICATION_DATETIME")
        .and_then(|v| parse_tfr_compact_datetime(v))
        .or_else(|| {
            props
                .get("EFFECTIVE")
                .or_else(|| props.get("effective"))
                .and_then(|v| parse_tfr_datetime(v))
        })
        .unwrap_or(fetched_at);

    // Effective end: only in ArcGIS format
    let effective_end = props
        .get("EXPIRE")
        .or_else(|| props.get("expire"))
        .and_then(|v| parse_tfr_datetime(v));

    let lower_altitude_ft = props
        .get("LOWALT")
        .or_else(|| props.get("lowalt"))
        .and_then(|v| parse_tfr_altitude(v));

    let upper_altitude_ft = props
        .get("HIGHALT")
        .or_else(|| props.get("highalt"))
        .and_then(|v| parse_tfr_altitude(v));

    let polygon = extract_polygon(feature);

    Some(TfrInfo {
        notam_id,
        name,
        tfr_type,
        effective_start,
        effective_end,
        lower_altitude_ft,
        upper_altitude_ft,
        polygon,
        fetched_at,
    })
}

/// Parse compact datetime format "yyyyMMddHHmm" used by FAA GeoServer WFS.
fn parse_tfr_compact_datetime(v: &Value) -> Option<chrono::DateTime<Utc>> {
    let s = v.as_str()?;
    if s.len() < 12 {
        return None;
    }
    let naive = chrono::NaiveDateTime::parse_from_str(s, "%Y%m%d%H%M").ok()?;
    Some(naive.and_utc())
}

/// Parse a TFR datetime value. Handles epoch millis (as number or string)
/// and ISO 8601 strings.
fn parse_tfr_datetime(v: &Value) -> Option<chrono::DateTime<Utc>> {
    if let Some(millis) = v.as_i64() {
        return chrono::DateTime::from_timestamp_millis(millis);
    }
    if let Some(millis) = v.as_f64() {
        return chrono::DateTime::from_timestamp_millis(millis as i64);
    }
    if let Some(s) = v.as_str() {
        if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(s) {
            return Some(dt.with_timezone(&Utc));
        }
        if let Ok(millis) = s.parse::<i64>() {
            return chrono::DateTime::from_timestamp_millis(millis);
        }
    }
    None
}

/// Parse a TFR altitude value. Handles:
/// - Integer (0, 18000)
/// - String: "SFC" / "GND" → 0, "FL180" → 18000, "18000 MSL" → 18000, "5000 AGL" → 5000, bare number
fn parse_tfr_altitude(v: &Value) -> Option<i32> {
    if let Some(n) = v.as_i64() {
        return Some(n as i32);
    }
    if let Some(s) = v.as_str() {
        let s = s.trim().to_uppercase();
        if s == "SFC" || s == "GND" || s == "SURFACE" {
            return Some(0);
        }
        if let Some(fl) = s.strip_prefix("FL") {
            if let Ok(level) = fl.trim().parse::<i32>() {
                return Some(level * 100);
            }
        }
        // Extract leading numeric value from strings like "18000 MSL", "5000 AGL", "5000FT"
        let numeric: String = s
            .chars()
            .take_while(|c| c.is_ascii_digit() || *c == '-')
            .collect();
        if let Ok(alt) = numeric.parse::<i32>() {
            return Some(alt);
        }
    }
    None
}

/// Extract polygon coordinates from GeoJSON geometry.
/// Handles Point (returns empty), Polygon, and MultiPolygon (first ring only).
fn extract_polygon(feature: &Value) -> Vec<(f64, f64)> {
    let geometry = match feature.get("geometry") {
        Some(g) => g,
        None => return Vec::new(),
    };

    let geo_type = geometry.get("type").and_then(|t| t.as_str()).unwrap_or("");
    let coords = match geometry.get("coordinates") {
        Some(c) => c,
        None => return Vec::new(),
    };

    match geo_type {
        "Polygon" => extract_ring(coords.get(0)),
        "MultiPolygon" => {
            // Use first polygon's outer ring
            coords
                .get(0)
                .and_then(|first_poly| extract_ring(first_poly.get(0)).into())
                .unwrap_or_default()
        }
        "Point" => {
            // Single point — no polygon boundary
            Vec::new()
        }
        _ => Vec::new(),
    }
}

/// Extract (lat, lon) pairs from a GeoJSON coordinate ring.
/// GeoJSON uses [longitude, latitude] order — we swap to (lat, lon).
fn extract_ring(ring: Option<&Value>) -> Vec<(f64, f64)> {
    let ring = match ring {
        Some(r) => r,
        None => return Vec::new(),
    };

    let arr = match ring.as_array() {
        Some(a) => a,
        None => return Vec::new(),
    };

    arr.iter()
        .filter_map(|coord| {
            let pair = coord.as_array()?;
            let lon = pair.first()?.as_f64()?;
            let lat = pair.get(1)?.as_f64()?;
            Some((lat, lon))
        })
        .collect()
}

/// Validate stage: filters out expired TFRs.
struct TfrValidateStage;

impl PipelineStage for TfrValidateStage {
    fn name(&self) -> &str {
        "tfr_validate"
    }

    fn phase(&self) -> PipelinePhase {
        PipelinePhase::Validate
    }

    fn execute(&self, data: &mut PipelineData) -> Result<(), PipelineError> {
        let now = Utc::now();
        data.records.retain(|record| {
            if let CanonicalRecord::Tfr(tfr) = record {
                match tfr.effective_end {
                    Some(end) if end < now => false,
                    _ => true,
                }
            } else {
                true
            }
        });
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data_ingest::pipeline::run_pipeline;

    fn sample_tfr_geojson() -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({
            "type": "FeatureCollection",
            "features": [
                {
                    "type": "Feature",
                    "properties": {
                        "NOTAM": "1/2345",
                        "NAME": "VIP TFR - Wichita",
                        "TYPE": "VIP",
                        "EFFECTIVE": 1767225600000_i64,
                        "EXPIRE": 1893456000000_i64,
                        "LOWALT": 0,
                        "HIGHALT": 18000
                    },
                    "geometry": {
                        "type": "Polygon",
                        "coordinates": [[
                            [-97.5, 37.5],
                            [-97.3, 37.5],
                            [-97.3, 37.7],
                            [-97.5, 37.7],
                            [-97.5, 37.5]
                        ]]
                    }
                },
                {
                    "type": "Feature",
                    "properties": {
                        "NOTAM": "1/6789",
                        "NAME": "Hazards - Stadium",
                        "TYPE": "HAZARDS",
                        "EFFECTIVE": 1767225600000_i64,
                        "EXPIRE": 1893456000000_i64,
                        "LOWALT": 0,
                        "HIGHALT": 3000
                    },
                    "geometry": {
                        "type": "Point",
                        "coordinates": [-97.4, 37.6]
                    }
                }
            ]
        }))
        .unwrap()
    }

    fn sample_tfr_with_expired() -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({
            "type": "FeatureCollection",
            "features": [
                {
                    "type": "Feature",
                    "properties": {
                        "NOTAM": "1/0001",
                        "NAME": "Active TFR",
                        "TYPE": "SECURITY",
                        "EFFECTIVE": 1767225600000_i64,
                        "EXPIRE": 1893456000000_i64,
                        "LOWALT": 0,
                        "HIGHALT": 18000
                    },
                    "geometry": {
                        "type": "Polygon",
                        "coordinates": [[
                            [-97.5, 37.5],
                            [-97.3, 37.5],
                            [-97.3, 37.7],
                            [-97.5, 37.7],
                            [-97.5, 37.5]
                        ]]
                    }
                },
                {
                    "type": "Feature",
                    "properties": {
                        "NOTAM": "1/0002",
                        "NAME": "Expired TFR",
                        "TYPE": "VIP",
                        "EFFECTIVE": 1577836800000_i64,
                        "EXPIRE": 1609459200000_i64,
                        "LOWALT": 0,
                        "HIGHALT": 5000
                    },
                    "geometry": {
                        "type": "Polygon",
                        "coordinates": [[
                            [-97.5, 37.5],
                            [-97.3, 37.5],
                            [-97.3, 37.7],
                            [-97.5, 37.7],
                            [-97.5, 37.5]
                        ]]
                    }
                }
            ]
        }))
        .unwrap()
    }

    #[test]
    fn test_parse_tfr_geojson() {
        let data = PipelineData {
            raw_bytes: Some(sample_tfr_geojson()),
            ..Default::default()
        };

        let stages: Vec<Box<dyn PipelineStage>> = vec![Box::new(TfrParseStage)];
        let result = run_pipeline(&stages, data).unwrap();

        assert_eq!(result.records.len(), 2);

        if let CanonicalRecord::Tfr(tfr) = &result.records[0] {
            assert_eq!(tfr.notam_id, "1/2345");
            assert_eq!(tfr.name, "VIP TFR - Wichita");
            assert_eq!(tfr.tfr_type, "VIP");
            assert_eq!(tfr.lower_altitude_ft, Some(0));
            assert_eq!(tfr.upper_altitude_ft, Some(18000));
            // Polygon should have 5 points (closed ring)
            assert_eq!(tfr.polygon.len(), 5);
            // Verify GeoJSON [lon, lat] → (lat, lon) swap
            let (lat, lon) = tfr.polygon[0];
            assert!((lat - 37.5).abs() < 0.001);
            assert!((lon - (-97.5)).abs() < 0.001);
        } else {
            panic!("expected Tfr record");
        }

        // Point geometry → empty polygon
        if let CanonicalRecord::Tfr(tfr) = &result.records[1] {
            assert_eq!(tfr.notam_id, "1/6789");
            assert!(tfr.polygon.is_empty());
        } else {
            panic!("expected Tfr record");
        }
    }

    #[test]
    fn test_validate_filters_expired_tfrs() {
        let data = PipelineData {
            raw_bytes: Some(sample_tfr_with_expired()),
            ..Default::default()
        };

        let stages: Vec<Box<dyn PipelineStage>> =
            vec![Box::new(TfrParseStage), Box::new(TfrValidateStage)];
        let result = run_pipeline(&stages, data).unwrap();

        assert_eq!(result.records.len(), 1);
        if let CanonicalRecord::Tfr(tfr) = &result.records[0] {
            assert_eq!(tfr.notam_id, "1/0001");
        } else {
            panic!("expected Tfr record");
        }
    }

    #[test]
    fn test_empty_response() {
        let data = PipelineData {
            raw_bytes: Some(b"{}".to_vec()),
            ..Default::default()
        };

        let stages: Vec<Box<dyn PipelineStage>> = vec![Box::new(TfrParseStage)];
        let result = run_pipeline(&stages, data).unwrap();
        assert!(result.records.is_empty());
    }

    #[test]
    fn test_arcgis_error_response() {
        let error_json = serde_json::to_vec(&serde_json::json!({
            "error": {
                "code": 400,
                "message": "Invalid query"
            }
        }))
        .unwrap();

        let data = PipelineData {
            raw_bytes: Some(error_json),
            ..Default::default()
        };

        let stages: Vec<Box<dyn PipelineStage>> = vec![Box::new(TfrParseStage)];
        let result = run_pipeline(&stages, data).unwrap();
        assert!(result.records.is_empty());
    }

    #[test]
    fn test_multipolygon_geometry() {
        let geojson = serde_json::to_vec(&serde_json::json!({
            "type": "FeatureCollection",
            "features": [{
                "type": "Feature",
                "properties": {
                    "NOTAM": "1/9999",
                    "NAME": "MultiPoly TFR",
                    "TYPE": "SECURITY",
                    "EFFECTIVE": 1767225600000_i64,
                    "EXPIRE": 1893456000000_i64
                },
                "geometry": {
                    "type": "MultiPolygon",
                    "coordinates": [
                        [[
                            [-97.5, 37.5],
                            [-97.3, 37.5],
                            [-97.3, 37.7],
                            [-97.5, 37.7],
                            [-97.5, 37.5]
                        ]],
                        [[
                            [-96.5, 36.5],
                            [-96.3, 36.5],
                            [-96.3, 36.7],
                            [-96.5, 36.7],
                            [-96.5, 36.5]
                        ]]
                    ]
                }
            }]
        }))
        .unwrap();

        let data = PipelineData {
            raw_bytes: Some(geojson),
            ..Default::default()
        };

        let stages: Vec<Box<dyn PipelineStage>> = vec![Box::new(TfrParseStage)];
        let result = run_pipeline(&stages, data).unwrap();

        assert_eq!(result.records.len(), 1);
        if let CanonicalRecord::Tfr(tfr) = &result.records[0] {
            // Should use first polygon only
            assert_eq!(tfr.polygon.len(), 5);
            let (lat, _lon) = tfr.polygon[0];
            assert!((lat - 37.5).abs() < 0.001);
        } else {
            panic!("expected Tfr record");
        }
    }

    #[test]
    fn test_provider_metadata() {
        let provider = TfrProvider;
        assert_eq!(provider.name(), "faa_tfr");
        assert_eq!(provider.schedule(), "0 */15 * * * *");
        assert!(!provider.supports_on_demand());
    }
}
