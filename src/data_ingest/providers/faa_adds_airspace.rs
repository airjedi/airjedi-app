use bevy::prelude::*;
use chrono::Utc;
use serde_json::Value;

use crate::data_ingest::canonical::{AirspaceInfo, CanonicalRecord};
use crate::data_ingest::pipeline::{PipelineData, PipelineError, PipelinePhase, PipelineStage};
use crate::data_ingest::provider::{
    DataProvider, FetchContext, ProviderCategory, ProviderError, ProviderMeta, RawFetchResult,
};

const FAA_ADDS_AIRSPACE_URL: &str = "https://services6.arcgis.com/ssFJjBXIUyZDrSYZ/arcgis/rest/services/Class_Airspace/FeatureServer/0/query";

/// Data provider that fetches Class B/C and Special Use Airspace from
/// the FAA ADDS ArcGIS FeatureServer.
pub struct FaaClassAirspaceProvider;

impl DataProvider for FaaClassAirspaceProvider {
    fn name(&self) -> &str {
        "faa_adds_airspace"
    }

    fn schedule(&self) -> &str {
        "0 0 4 * * *"
    }

    fn metadata(&self) -> ProviderMeta {
        ProviderMeta {
            display_name: "FAA Airspace",
            category: ProviderCategory::Navigation,
            description: "Class B/C and special use airspace from FAA ADDS",
            config_key: "faa_airspace",
        }
    }

    fn fetch(&self, _ctx: &FetchContext) -> Result<RawFetchResult, ProviderError> {
        // Fetch all Class B/C + SUA using pagination (200 features per page)
        // to avoid overwhelming reqwest with the full 28+ MB response.
        let where_clause = "CLASS%20IN%20(%27B%27%2C%27C%27)%20OR%20TYPE_CODE%20IN%20(%27R%27%2C%27MOA%27%2C%27W%27%2C%27A%27)";
        let fields =
            "IDENT,NAME,CLASS,TYPE_CODE,LOCAL_TYPE,UPPER_VAL,UPPER_CODE,LOWER_VAL,LOWER_CODE";
        let page_size = 200;

        let client = reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(120))
            .build()
            .map_err(|e| ProviderError::Network(format!("Failed to create HTTP client: {}", e)))?;

        let mut all_features: Vec<serde_json::Value> = Vec::new();
        let mut offset = 0;

        loop {
            let url = format!(
                "{}?where={}&outFields={}&f=geojson&geometryPrecision=4&resultRecordCount={}&resultOffset={}",
                FAA_ADDS_AIRSPACE_URL, where_clause, fields, page_size, offset,
            );

            let response = client.get(&url).send().map_err(|e| {
                ProviderError::Network(format!("FAA ADDS fetch failed (offset {}): {}", offset, e))
            })?;

            let bytes = response.bytes().map_err(|e| {
                ProviderError::Network(format!(
                    "Failed to read response (offset {}): {}",
                    offset, e
                ))
            })?;

            let page: serde_json::Value = serde_json::from_slice(&bytes).map_err(|e| {
                ProviderError::Parse(format!("Invalid JSON (offset {}): {}", offset, e))
            })?;

            let features = page["features"].as_array();
            let count = features.map(|f| f.len()).unwrap_or(0);

            if let Some(feats) = features {
                all_features.extend(feats.iter().cloned());
            }

            info!(
                "FAA ADDS airspace: fetched {} features (offset {}, total so far {})",
                count,
                offset,
                all_features.len()
            );

            if count < page_size {
                break; // last page
            }
            offset += page_size;
        }

        // Assemble into a single GeoJSON FeatureCollection
        let geojson = serde_json::json!({
            "type": "FeatureCollection",
            "features": all_features,
        });

        let data = serde_json::to_vec(&geojson).map_err(|e| {
            ProviderError::Parse(format!("Failed to serialize combined GeoJSON: {}", e))
        })?;

        let source = format!(
            "{}?where={} ({} features)",
            FAA_ADDS_AIRSPACE_URL,
            where_clause,
            all_features.len()
        );

        Ok(RawFetchResult {
            data,
            content_type: Some("application/json".to_string()),
            source,
        })
    }

    fn pipeline_stages(&self) -> Vec<Box<dyn PipelineStage>> {
        vec![Box::new(FaaAirspaceParseStage)]
    }
}

struct FaaAirspaceParseStage;

impl PipelineStage for FaaAirspaceParseStage {
    fn name(&self) -> &str {
        "faa_adds_airspace_parse"
    }

    fn phase(&self) -> PipelinePhase {
        PipelinePhase::Parse
    }

    fn execute(&self, data: &mut PipelineData) -> Result<(), PipelineError> {
        let bytes = data
            .raw_bytes
            .as_ref()
            .ok_or_else(|| PipelineError::StageError {
                stage: "faa_adds_airspace_parse".into(),
                message: "No raw data to parse".into(),
            })?;

        let geojson: Value =
            serde_json::from_slice(bytes).map_err(|e| PipelineError::StageError {
                stage: "faa_adds_airspace_parse".into(),
                message: format!("Invalid JSON: {}", e),
            })?;

        let features = geojson["features"]
            .as_array()
            .ok_or_else(|| PipelineError::StageError {
                stage: "faa_adds_airspace_parse".into(),
                message: "No 'features' array in GeoJSON".into(),
            })?;

        let now = Utc::now();
        let mut records = Vec::new();

        for feature in features {
            let props = &feature["properties"];

            let ident = props["IDENT"].as_str().unwrap_or("UNKNOWN");
            let name = props["NAME"].as_str().unwrap_or(ident);
            let class = props["CLASS"].as_str().unwrap_or("");
            let type_code = props["TYPE_CODE"].as_str().unwrap_or("");
            let local_type = props["LOCAL_TYPE"].as_str().unwrap_or("");

            // Altitude values are already in feet (not hundreds)
            let upper_val = props["UPPER_VAL"].as_i64().map(|v| v as i32);
            let lower_val = props["LOWER_VAL"].as_i64().map(|v| v as i32);
            let upper_code = props["UPPER_CODE"].as_str().map(String::from);
            let lower_code = props["LOWER_CODE"].as_str().map(String::from);

            // Determine airspace class string for canonical record
            // CLASS field: "B", "C", "D", "E", "Other", or null
            // LOCAL_TYPE field: "CLASS_B", "CLASS_C", "CLASS_D", "CLASS_E", "MODE C", etc.
            // TYPE_CODE field: "CLASS", "EXCLUSION", "MODE-C", "TRSA", "R", "MOA", "W", "A", etc.
            let airspace_class = match class {
                "B" => "ClassB",
                "C" => "ClassC",
                "D" => "ClassD",
                _ => match type_code {
                    "R" => "Restricted",
                    "MOA" => "MOA",
                    "W" => "Warning",
                    "A" => "Alert",
                    _ => match local_type {
                        "CLASS_B" => "ClassB",
                        "CLASS_C" => "ClassC",
                        "CLASS_D" => "ClassD",
                        _ => continue, // skip unknown types
                    },
                },
            };

            // Extract polygon coordinates from GeoJSON geometry
            let polygon = extract_polygon_coords(&feature["geometry"]);
            if polygon.is_empty() {
                continue;
            }

            records.push(CanonicalRecord::Airspace(AirspaceInfo {
                name: format!("{} {}", name, ident),
                airspace_class: airspace_class.to_string(),
                airspace_type: if class.is_empty() {
                    type_code.to_string()
                } else {
                    format!("Class {}", class)
                },
                lower_limit_ft: lower_val,
                upper_limit_ft: upper_val,
                lower_altitude_ref: lower_code,
                upper_altitude_ref: upper_code,
                polygon,
                fetched_at: now,
            }));
        }

        data.records = records;
        Ok(())
    }
}

/// Extract (lat, lon) pairs from a GeoJSON geometry (Polygon or MultiPolygon).
fn extract_polygon_coords(geometry: &Value) -> Vec<(f64, f64)> {
    let geo_type = geometry["type"].as_str().unwrap_or("");
    let coords = &geometry["coordinates"];

    match geo_type {
        "Polygon" => {
            // coords[0] = outer ring = [[lon, lat], ...]
            extract_ring(&coords[0])
        }
        "MultiPolygon" => {
            // coords[0][0] = first polygon's outer ring
            extract_ring(&coords[0][0])
        }
        _ => Vec::new(),
    }
}

fn extract_ring(ring: &Value) -> Vec<(f64, f64)> {
    ring.as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|coord| {
                    let lon = coord[0].as_f64()?;
                    let lat = coord[1].as_f64()?;
                    Some((lat, lon))
                })
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_faa_airspace_geojson() {
        let geojson = r#"{
            "type": "FeatureCollection",
            "features": [{
                "type": "Feature",
                "properties": {
                    "IDENT": "ICT",
                    "NAME": "WICHITA MID-CONTINENT AIRPORT CLASS C",
                    "CLASS": "C",
                    "UPPER_VAL": 5300,
                    "UPPER_CODE": "MSL",
                    "LOWER_VAL": 0,
                    "LOWER_CODE": "SFC",
                    "TYPE_CODE": "CLASS",
                    "LOCAL_TYPE": "CLASS_C"
                },
                "geometry": {
                    "type": "Polygon",
                    "coordinates": [[
                        [-97.43, 37.65],
                        [-97.20, 37.65],
                        [-97.20, 37.72],
                        [-97.43, 37.72],
                        [-97.43, 37.65]
                    ]]
                }
            }]
        }"#;

        let mut data = PipelineData {
            raw_bytes: Some(geojson.as_bytes().to_vec()),
            records: Vec::new(),
            metadata: std::collections::HashMap::new(),
        };

        let stage = FaaAirspaceParseStage;
        stage.execute(&mut data).unwrap();

        assert_eq!(data.records.len(), 1);
        if let CanonicalRecord::Airspace(ref info) = data.records[0] {
            assert!(info.name.contains("ICT"));
            assert_eq!(info.airspace_class, "ClassC");
            assert_eq!(info.upper_limit_ft, Some(5300));
            assert_eq!(info.lower_limit_ft, Some(0));
            assert_eq!(info.upper_altitude_ref.as_deref(), Some("MSL"));
            assert_eq!(info.lower_altitude_ref.as_deref(), Some("SFC"));
            assert_eq!(info.polygon.len(), 5);
        } else {
            panic!("Expected Airspace record");
        }
    }

    #[test]
    fn test_parse_restricted_airspace() {
        let geojson = r#"{
            "type": "FeatureCollection",
            "features": [{
                "type": "Feature",
                "properties": {
                    "IDENT": "R-2508",
                    "NAME": "CHINA LAKE",
                    "CLASS": "",
                    "UPPER_VAL": 99900,
                    "UPPER_CODE": "MSL",
                    "LOWER_VAL": 0,
                    "LOWER_CODE": "SFC",
                    "TYPE_CODE": "R",
                    "LOCAL_TYPE": "R"
                },
                "geometry": {
                    "type": "Polygon",
                    "coordinates": [[
                        [-117.8, 35.6],
                        [-117.5, 35.6],
                        [-117.5, 35.9],
                        [-117.8, 35.9],
                        [-117.8, 35.6]
                    ]]
                }
            }]
        }"#;

        let mut data = PipelineData {
            raw_bytes: Some(geojson.as_bytes().to_vec()),
            records: Vec::new(),
            metadata: std::collections::HashMap::new(),
        };

        let stage = FaaAirspaceParseStage;
        stage.execute(&mut data).unwrap();

        assert_eq!(data.records.len(), 1);
        if let CanonicalRecord::Airspace(ref info) = data.records[0] {
            assert_eq!(info.airspace_class, "Restricted");
            assert_eq!(info.upper_limit_ft, Some(99900));
        } else {
            panic!("Expected Airspace record");
        }
    }

    #[test]
    fn test_extract_multipolygon() {
        let geo = serde_json::json!({
            "type": "MultiPolygon",
            "coordinates": [[[
                [-97.43, 37.65],
                [-97.20, 37.65],
                [-97.20, 37.72],
                [-97.43, 37.72],
                [-97.43, 37.65]
            ]]]
        });

        let coords = extract_polygon_coords(&geo);
        assert_eq!(coords.len(), 5);
        assert!((coords[0].0 - 37.65).abs() < 0.001); // lat
        assert!((coords[0].1 - (-97.43)).abs() < 0.001); // lon
    }

    #[test]
    fn test_metadata() {
        let provider = FaaClassAirspaceProvider;
        let meta = provider.metadata();
        assert_eq!(meta.config_key, "faa_airspace");
        assert_eq!(meta.category, ProviderCategory::Navigation);
    }
}
