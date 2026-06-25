use chrono::Utc;
use serde_json::Value;

use crate::data_ingest::canonical::{CanonicalRecord, NotamInfo};
use crate::data_ingest::pipeline::{PipelineData, PipelineError, PipelinePhase, PipelineStage};
use crate::data_ingest::provider::{
    DataProvider, FetchContext, ProviderCategory, ProviderError, ProviderMeta, RawFetchResult,
};

const FAA_NOTAM_API_URL: &str = "https://external-api.faa.gov/notamapi/v1/notams";

/// Data provider that fetches NOTAMs from the FAA NOTAM API v1.
///
/// Requires `api_key` (client_id) and `api_secret` (client_secret) from
/// the FAA API portal at https://api.faa.gov/s/.
pub struct NotamProvider;

impl DataProvider for NotamProvider {
    fn name(&self) -> &str {
        "faa_notam"
    }

    fn schedule(&self) -> &str {
        "0 */30 * * * *"
    }

    fn fetch(&self, ctx: &FetchContext) -> Result<RawFetchResult, ProviderError> {
        // Read credentials from config
        let app_config = crate::config::load_config();
        let notam_config = &app_config.data_ingest.notam;

        let client_id = notam_config.api_key.as_deref().unwrap_or("").to_string();
        let client_secret = notam_config.api_secret.as_deref().unwrap_or("").to_string();

        if client_id.is_empty() || client_secret.is_empty() {
            return Err(ProviderError::Other(
                "NOTAM provider requires api_key and api_secret from https://api.faa.gov/s/"
                    .to_string(),
            ));
        }

        let half_side_deg = ctx.radius_nm / 60.0;
        let lat_min = ctx.center_latitude - half_side_deg;
        let lat_max = ctx.center_latitude + half_side_deg;
        let lon_min = ctx.center_longitude - half_side_deg;
        let lon_max = ctx.center_longitude + half_side_deg;

        // Use locationLongitude/Latitude and locationRadius for spatial query
        let client = reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .map_err(|e| ProviderError::Network(e.to_string()))?;

        let resp = client
            .get(FAA_NOTAM_API_URL)
            .header("client_id", &client_id)
            .header("client_secret", &client_secret)
            .query(&[
                ("responseFormat", "geoJson"),
                ("locationLongitude", &format!("{:.4}", ctx.center_longitude)),
                ("locationLatitude", &format!("{:.4}", ctx.center_latitude)),
                ("locationRadius", &format!("{:.0}", ctx.radius_nm)),
            ])
            .send()
            .map_err(|e| ProviderError::Network(e.to_string()))?;

        if resp.status().as_u16() == 401 {
            return Err(ProviderError::Other(
                "NOTAM API auth failed: check api_key and api_secret in Ingest settings"
                    .to_string(),
            ));
        }

        if !resp.status().is_success() {
            return Err(ProviderError::Network(format!(
                "FAA NOTAM API returned status {}",
                resp.status()
            )));
        }

        let bytes = resp
            .bytes()
            .map_err(|e| ProviderError::Network(e.to_string()))?;

        Ok(RawFetchResult {
            data: bytes.to_vec(),
            content_type: Some("application/json".to_string()),
            source: FAA_NOTAM_API_URL.to_string(),
        })
    }

    fn pipeline_stages(&self) -> Vec<Box<dyn PipelineStage>> {
        vec![Box::new(NotamParseStage), Box::new(NotamValidateStage)]
    }

    fn metadata(&self) -> ProviderMeta {
        ProviderMeta {
            display_name: "NOTAMs",
            category: ProviderCategory::Notices,
            description: "Notices to Air Missions from FAA",
            config_key: "notam",
        }
    }
}

/// Parse stage: extracts NOTAM records from the FAA JSON response.
struct NotamParseStage;

impl PipelineStage for NotamParseStage {
    fn name(&self) -> &str {
        "notam_parse"
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

        let now = Utc::now();

        // Support both formats:
        // 1. GeoJSON FeatureCollection (v1 API with responseFormat=geoJson)
        // 2. Legacy format with "notamList" array
        let items: Vec<&Value> =
            if let Some(features) = json.get("features").and_then(|f| f.as_array()) {
                // GeoJSON: each feature has properties with NOTAM fields
                features.iter().collect()
            } else if let Some(list) = json.get("notamList").and_then(|l| l.as_array()) {
                list.iter().collect()
            } else {
                bevy::log::warn!("NOTAM response has no 'features' or 'notamList' field");
                return Ok(());
            };

        for item in &items {
            // For GeoJSON, the NOTAM fields are in "properties"
            let props = item.get("properties").unwrap_or(item);
            let geometry = item.get("geometry");

            let notam = match parse_notam_item(props, geometry, now) {
                Some(n) => n,
                None => continue,
            };
            data.records.push(CanonicalRecord::Notam(notam));
        }

        Ok(())
    }
}

/// Parse a single NOTAM item from the FAA JSON response.
/// Supports both legacy format and GeoJSON v1 API format.
fn parse_notam_item(
    props: &Value,
    geometry: Option<&Value>,
    fetched_at: chrono::DateTime<Utc>,
) -> Option<NotamInfo> {
    // NOTAM ID: v1 uses "coreNOTAMData.notam.number", legacy uses "notamNumber"
    let id = props
        .get("coreNOTAMData")
        .and_then(|c| c.get("notam"))
        .and_then(|n| n.get("number"))
        .and_then(|v| v.as_str())
        .or_else(|| props.get("notamNumber").and_then(|v| v.as_str()))
        .or_else(|| props.get("id").and_then(|v| v.as_str()))
        .unwrap_or("")
        .to_string();

    if id.is_empty() {
        return None;
    }

    let notam_data = props.get("coreNOTAMData").and_then(|c| c.get("notam"));

    let location = notam_data
        .and_then(|n| n.get("location").and_then(|v| v.as_str()))
        .or_else(|| props.get("facilityDesignator").and_then(|v| v.as_str()))
        .or_else(|| props.get("location").and_then(|v| v.as_str()))
        .unwrap_or("")
        .to_string();

    let raw_text = notam_data
        .and_then(|n| n.get("text").and_then(|v| v.as_str()))
        .or_else(|| props.get("traditionalMessage").and_then(|v| v.as_str()))
        .or_else(|| props.get("icaoMessage").and_then(|v| v.as_str()))
        .unwrap_or("")
        .to_string();

    let classification = notam_data
        .and_then(|n| n.get("classification").and_then(|v| v.as_str()))
        .or_else(|| props.get("classification").and_then(|v| v.as_str()))
        .unwrap_or("UNKNOWN")
        .to_string();

    let effective_start = notam_data
        .and_then(|n| n.get("effectiveStart").and_then(|v| v.as_str()))
        .or_else(|| props.get("startDate").and_then(|v| v.as_str()))
        .or_else(|| props.get("effectiveStart").and_then(|v| v.as_str()))
        .and_then(|s| parse_faa_datetime(s))
        .unwrap_or(fetched_at);

    let effective_end = notam_data
        .and_then(|n| n.get("effectiveEnd").and_then(|v| v.as_str()))
        .or_else(|| props.get("endDate").and_then(|v| v.as_str()))
        .or_else(|| props.get("effectiveEnd").and_then(|v| v.as_str()))
        .and_then(|s| {
            if s.contains("PERM") || s.is_empty() {
                None
            } else {
                parse_faa_datetime(s)
            }
        });

    // Coordinates: from GeoJSON geometry (Point), or from properties
    let (latitude, longitude) = if let Some(geo) = geometry {
        let coords = geo.get("coordinates").and_then(|c| c.as_array());
        match coords {
            Some(arr) if arr.len() >= 2 => {
                // GeoJSON: [longitude, latitude]
                (
                    arr.get(1).and_then(|v| v.as_f64()),
                    arr.get(0).and_then(|v| v.as_f64()),
                )
            }
            _ => (None, None),
        }
    } else {
        let lat = props.get("latitude").and_then(|v| v.as_f64());
        let lon = props.get("longitude").and_then(|v| v.as_f64());
        (lat, lon)
    };

    let radius_nm = props.get("radius").and_then(|v| v.as_f64());

    Some(NotamInfo {
        id,
        location,
        raw_text,
        classification,
        effective_start,
        effective_end,
        latitude,
        longitude,
        radius_nm,
        fetched_at,
    })
}

/// Parse FAA datetime strings. Handles common formats:
/// "MM/DD/YYYY HHmm", ISO 8601, and epoch millis.
fn parse_faa_datetime(s: &str) -> Option<chrono::DateTime<Utc>> {
    // Try ISO 8601 first
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(s) {
        return Some(dt.with_timezone(&Utc));
    }

    // Try "MM/DD/YYYY HHmm" format used by FAA
    if let Ok(dt) = chrono::NaiveDateTime::parse_from_str(s, "%m/%d/%Y %H%M") {
        return Some(dt.and_utc());
    }

    // Try epoch milliseconds (numeric string)
    if let Ok(millis) = s.parse::<i64>() {
        return chrono::DateTime::from_timestamp_millis(millis);
    }

    None
}

/// Validate stage: filters out expired NOTAMs.
struct NotamValidateStage;

impl PipelineStage for NotamValidateStage {
    fn name(&self) -> &str {
        "notam_validate"
    }

    fn phase(&self) -> PipelinePhase {
        PipelinePhase::Validate
    }

    fn execute(&self, data: &mut PipelineData) -> Result<(), PipelineError> {
        let now = Utc::now();
        data.records.retain(|record| {
            if let CanonicalRecord::Notam(notam) = record {
                match notam.effective_end {
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
    use chrono::{Datelike, Timelike};

    fn sample_notam_response() -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({
            "notamList": [
                {
                    "notamNumber": "01/234",
                    "facilityDesignator": "KICT",
                    "traditionalMessage": "!KICT 01/234 ICT RWY 01R/19L CLSD",
                    "classification": "AERODROME",
                    "startDate": "01/15/2026 1400",
                    "endDate": "12/31/2026 2359",
                    "latitude": 37.6499,
                    "longitude": -97.4331,
                    "radius": 5.0
                },
                {
                    "notamNumber": "02/567",
                    "facilityDesignator": "KICT",
                    "traditionalMessage": "!KICT 02/567 ICT TWY A CLSD",
                    "classification": "AERODROME",
                    "startDate": "02/01/2026 0800",
                    "endDate": "PERM",
                    "latitude": 37.6499,
                    "longitude": -97.4331
                }
            ]
        }))
        .unwrap()
    }

    fn sample_notam_response_with_expired() -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({
            "notamList": [
                {
                    "notamNumber": "01/001",
                    "facilityDesignator": "KICT",
                    "traditionalMessage": "!KICT active notam",
                    "classification": "AERODROME",
                    "startDate": "01/01/2026 0000",
                    "endDate": "12/31/2030 2359"
                },
                {
                    "notamNumber": "01/002",
                    "facilityDesignator": "KICT",
                    "traditionalMessage": "!KICT expired notam",
                    "classification": "AERODROME",
                    "startDate": "01/01/2020 0000",
                    "endDate": "01/01/2021 0000"
                }
            ]
        }))
        .unwrap()
    }

    #[test]
    fn test_parse_notam_response() {
        let data = PipelineData {
            raw_bytes: Some(sample_notam_response()),
            ..Default::default()
        };

        let stages: Vec<Box<dyn PipelineStage>> = vec![Box::new(NotamParseStage)];
        let result = run_pipeline(&stages, data).unwrap();

        assert_eq!(result.records.len(), 2);

        if let CanonicalRecord::Notam(notam) = &result.records[0] {
            assert_eq!(notam.id, "01/234");
            assert_eq!(notam.location, "KICT");
            assert!(notam.raw_text.contains("RWY 01R/19L CLSD"));
            assert_eq!(notam.classification, "AERODROME");
            assert!(notam.latitude.is_some());
            assert!(notam.longitude.is_some());
            assert_eq!(notam.radius_nm, Some(5.0));
        } else {
            panic!("expected Notam record");
        }

        // Second NOTAM has "PERM" end date → effective_end should be None
        if let CanonicalRecord::Notam(notam) = &result.records[1] {
            assert_eq!(notam.id, "02/567");
            assert!(notam.effective_end.is_none());
        } else {
            panic!("expected Notam record");
        }
    }

    #[test]
    fn test_validate_filters_expired() {
        let data = PipelineData {
            raw_bytes: Some(sample_notam_response_with_expired()),
            ..Default::default()
        };

        let stages: Vec<Box<dyn PipelineStage>> =
            vec![Box::new(NotamParseStage), Box::new(NotamValidateStage)];
        let result = run_pipeline(&stages, data).unwrap();

        // Only the active NOTAM should remain
        assert_eq!(result.records.len(), 1);
        if let CanonicalRecord::Notam(notam) = &result.records[0] {
            assert_eq!(notam.id, "01/001");
        } else {
            panic!("expected Notam record");
        }
    }

    #[test]
    fn test_empty_response() {
        let data = PipelineData {
            raw_bytes: Some(b"{}".to_vec()),
            ..Default::default()
        };

        let stages: Vec<Box<dyn PipelineStage>> = vec![Box::new(NotamParseStage)];
        let result = run_pipeline(&stages, data).unwrap();
        assert!(result.records.is_empty());
    }

    #[test]
    fn test_empty_notam_list() {
        let data = PipelineData {
            raw_bytes: Some(serde_json::to_vec(&serde_json::json!({"notamList": []})).unwrap()),
            ..Default::default()
        };

        let stages: Vec<Box<dyn PipelineStage>> = vec![Box::new(NotamParseStage)];
        let result = run_pipeline(&stages, data).unwrap();
        assert!(result.records.is_empty());
    }

    #[test]
    fn test_parse_faa_datetime_formats() {
        // MM/DD/YYYY HHmm
        let dt = parse_faa_datetime("03/15/2026 1430").unwrap();
        assert_eq!(dt.month(), 3);
        assert_eq!(dt.day(), 15);
        assert_eq!(dt.hour(), 14);
        assert_eq!(dt.minute(), 30);

        // ISO 8601
        let dt = parse_faa_datetime("2026-03-15T14:30:00Z").unwrap();
        assert_eq!(dt.month(), 3);

        // PERM-like strings should fail
        assert!(parse_faa_datetime("PERM").is_none());
        assert!(parse_faa_datetime("").is_none());
    }

    #[test]
    fn test_provider_metadata() {
        let provider = NotamProvider;
        assert_eq!(provider.name(), "faa_notam");
        assert_eq!(provider.schedule(), "0 */30 * * * *");
        assert!(!provider.supports_on_demand());
    }
}
