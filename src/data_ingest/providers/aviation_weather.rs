use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::Value;

use crate::data_ingest::canonical::{
    AirmetReport, CanonicalRecord, MetarReport, PirepReport, SigmetReport, TafReport,
};
use crate::data_ingest::pipeline::{PipelineData, PipelineError, PipelinePhase, PipelineStage};
use crate::data_ingest::provider::{
    DataProvider, FetchContext, ProviderCategory, ProviderError, ProviderMeta, RawFetchResult,
};

/// Conversion factor: 1 inHg = 33.8639 hPa/mbar
const HPA_TO_INHG: f32 = 33.8639;

const AVIATION_WEATHER_BASE: &str = "https://aviationweather.gov/api/data";

/// Compute a bounding box from center lat/lon and radius in nautical miles.
/// Returns (lat_min, lon_min, lat_max, lon_max).
fn bbox_from_context(ctx: &FetchContext) -> (f64, f64, f64, f64) {
    let half_deg_lat = ctx.radius_nm / 60.0;
    // Adjust longitude degrees for latitude (1 degree of longitude shrinks with latitude)
    let half_deg_lon = ctx.radius_nm / (60.0 * ctx.center_latitude.to_radians().cos().max(0.01));

    let lat_min = ctx.center_latitude - half_deg_lat;
    let lat_max = ctx.center_latitude + half_deg_lat;
    let lon_min = ctx.center_longitude - half_deg_lon;
    let lon_max = ctx.center_longitude + half_deg_lon;

    (lat_min, lon_min, lat_max, lon_max)
}

fn http_get(url: &str) -> Result<Vec<u8>, ProviderError> {
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| ProviderError::Network(e.to_string()))?;

    let resp = client
        .get(url)
        .send()
        .map_err(|e| ProviderError::Network(e.to_string()))?;

    if !resp.status().is_success() {
        return Err(ProviderError::Network(format!(
            "aviationweather.gov returned status {}",
            resp.status()
        )));
    }

    resp.bytes()
        .map(|b| b.to_vec())
        .map_err(|e| ProviderError::Network(e.to_string()))
}

// ---------------------------------------------------------------------------
// METAR
// ---------------------------------------------------------------------------

/// Provider that fetches METAR observations from aviationweather.gov.
pub struct MetarProvider;

impl DataProvider for MetarProvider {
    fn name(&self) -> &str {
        "aviationweather_metar"
    }

    fn schedule(&self) -> &str {
        "0 */5 * * * *"
    }

    fn fetch(&self, ctx: &FetchContext) -> Result<RawFetchResult, ProviderError> {
        let (lat1, lon1, lat2, lon2) = bbox_from_context(ctx);
        let url = format!(
            "{}/metar?format=json&bbox={},{},{},{}",
            AVIATION_WEATHER_BASE, lat1, lon1, lat2, lon2
        );

        let data = http_get(&url)?;
        Ok(RawFetchResult {
            data,
            content_type: Some("application/json".to_string()),
            source: url,
        })
    }

    fn pipeline_stages(&self) -> Vec<Box<dyn PipelineStage>> {
        vec![Box::new(MetarParseStage)]
    }

    fn metadata(&self) -> ProviderMeta {
        ProviderMeta {
            display_name: "METARs",
            category: ProviderCategory::Weather,
            description: "Surface weather observations",
            config_key: "metar",
        }
    }
}

#[derive(Debug, Deserialize)]
struct MetarJson {
    #[serde(rename = "icaoId")]
    icao_id: Option<String>,
    #[serde(rename = "rawOb")]
    raw_ob: Option<String>,
    /// Observation time: may be an epoch integer (seconds) or an RFC3339/ISO 8601 string.
    #[serde(rename = "obsTime")]
    obs_time: Option<Value>,
    /// Fallback observation time as ISO 8601 string.
    #[serde(rename = "reportTime")]
    report_time: Option<String>,
    temp: Option<f32>,
    dewp: Option<f32>,
    /// Wind direction: may be an integer (degrees) or a string ("VRB", "270").
    wdir: Option<Value>,
    wspd: Option<i32>,
    wgst: Option<i32>,
    /// Visibility: may be a number (0.5) or a string ("10+", "1/2").
    visib: Option<Value>,
    /// Altimeter: may be in hPa/mbar (>100) or inHg (<40). Autodetected.
    altim: Option<f32>,
    /// Flight category: real API uses camelCase "fltCat", accept both.
    #[serde(alias = "fltCat", alias = "fltcat")]
    flt_cat: Option<String>,
    clouds: Option<Vec<MetarCloudLayer>>,
}

#[derive(Debug, Deserialize)]
struct MetarCloudLayer {
    cover: Option<String>,
    base: Option<i32>,
}

struct MetarParseStage;

impl PipelineStage for MetarParseStage {
    fn name(&self) -> &str {
        "metar_parse"
    }

    fn phase(&self) -> PipelinePhase {
        PipelinePhase::Parse
    }

    fn execute(&self, data: &mut PipelineData) -> Result<(), PipelineError> {
        let raw = data.raw_bytes.take().unwrap_or_default();
        if raw.is_empty() {
            return Ok(());
        }

        let items: Vec<MetarJson> =
            serde_json::from_slice(&raw).map_err(|e| PipelineError::StageError {
                stage: self.name().to_string(),
                message: format!("JSON parse error: {}", e),
            })?;

        let now = Utc::now();

        for item in &items {
            if let Some(record) = parse_metar_item(item, now) {
                data.records.push(CanonicalRecord::Metar(record));
            }
        }

        Ok(())
    }
}

fn parse_metar_item(item: &MetarJson, fetched_at: DateTime<Utc>) -> Option<MetarReport> {
    let icao = item.icao_id.as_ref()?.clone();

    // Parse observation time: try obsTime (epoch int or string), fall back to reportTime
    let observation_time = parse_flexible_datetime(&item.obs_time)
        .or_else(|| {
            item.report_time
                .as_ref()
                .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
                .map(|dt| dt.with_timezone(&Utc))
        })
        .unwrap_or(fetched_at);

    // Wind direction: may be integer (90) or string ("VRB", "270")
    let wind_direction_deg = item.wdir.as_ref().and_then(|v| match v {
        Value::Number(n) => n.as_u64().map(|n| n as u16),
        Value::String(s) => {
            if s == "VRB" {
                None
            } else {
                s.parse::<u16>().ok()
            }
        }
        _ => None,
    });

    let wind_speed_kt = item.wspd.map(|v| v as u16);
    let wind_gust_kt = item.wgst.map(|v| v as u16);

    // Visibility: may be a number (0.5) or string ("10+", "1/2")
    let visibility_sm = item.visib.as_ref().and_then(|v| match v {
        Value::Number(n) => n.as_f64().map(|n| n as f32),
        Value::String(s) => parse_visibility(s),
        _ => None,
    });

    let ceiling_ft = item.clouds.as_ref().and_then(|clouds| {
        clouds
            .iter()
            .filter(|c| matches!(c.cover.as_deref(), Some("BKN") | Some("OVC")))
            .filter_map(|c| c.base)
            .min()
    });

    let flight_category = item.flt_cat.as_deref().unwrap_or("VFR").to_string();

    // Altimeter: if > 100, assume hPa and convert to inHg
    let altimeter_inhg = item
        .altim
        .map(|a| if a > 100.0 { a / HPA_TO_INHG } else { a });

    Some(MetarReport {
        icao,
        raw_text: item.raw_ob.clone().unwrap_or_default(),
        observation_time,
        wind_direction_deg,
        wind_speed_kt,
        wind_gust_kt,
        visibility_sm,
        ceiling_ft,
        temperature_c: item.temp,
        dewpoint_c: item.dewp,
        altimeter_inhg,
        flight_category,
        fetched_at,
    })
}

/// Parse a flexible datetime value that may be an epoch integer (seconds),
/// an ISO 8601 / RFC3339 string, or an epoch integer encoded as string.
fn parse_flexible_datetime(value: &Option<Value>) -> Option<DateTime<Utc>> {
    let v = value.as_ref()?;
    match v {
        Value::Number(n) => {
            // Epoch seconds (integer)
            if let Some(secs) = n.as_i64() {
                return DateTime::from_timestamp(secs, 0);
            }
            // Epoch seconds (float)
            if let Some(secs) = n.as_f64() {
                return DateTime::from_timestamp(secs as i64, 0);
            }
            None
        }
        Value::String(s) => {
            // Try RFC3339 / ISO 8601
            if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
                return Some(dt.with_timezone(&Utc));
            }
            // Try epoch seconds as string
            if let Ok(secs) = s.parse::<i64>() {
                return DateTime::from_timestamp(secs, 0);
            }
            None
        }
        _ => None,
    }
}

/// Parse visibility string from aviationweather.gov.
/// Handles "10+", fractions like "1/2", and mixed like "1 1/2".
fn parse_visibility(v: &str) -> Option<f32> {
    if v.contains('+') {
        Some(10.0)
    } else if v.contains('/') {
        let parts: Vec<&str> = v.split_whitespace().collect();
        if parts.len() == 2 {
            let whole: f32 = parts[0].parse().ok()?;
            let frac_parts: Vec<&str> = parts[1].split('/').collect();
            if frac_parts.len() == 2 {
                let num: f32 = frac_parts[0].parse().ok()?;
                let den: f32 = frac_parts[1].parse().ok()?;
                Some(whole + num / den)
            } else {
                Some(whole)
            }
        } else if parts.len() == 1 {
            let frac_parts: Vec<&str> = v.split('/').collect();
            if frac_parts.len() == 2 {
                let num: f32 = frac_parts[0].parse().ok()?;
                let den: f32 = frac_parts[1].parse().ok()?;
                Some(num / den)
            } else {
                None
            }
        } else {
            None
        }
    } else {
        v.parse().ok()
    }
}

// ---------------------------------------------------------------------------
// TAF
// ---------------------------------------------------------------------------

/// Provider that fetches TAF forecasts from aviationweather.gov.
pub struct TafProvider;

impl DataProvider for TafProvider {
    fn name(&self) -> &str {
        "aviationweather_taf"
    }

    fn schedule(&self) -> &str {
        "0 */15 * * * *"
    }

    fn fetch(&self, ctx: &FetchContext) -> Result<RawFetchResult, ProviderError> {
        let (lat1, lon1, lat2, lon2) = bbox_from_context(ctx);
        let url = format!(
            "{}/taf?format=json&bbox={},{},{},{}",
            AVIATION_WEATHER_BASE, lat1, lon1, lat2, lon2
        );

        let data = http_get(&url)?;
        Ok(RawFetchResult {
            data,
            content_type: Some("application/json".to_string()),
            source: url,
        })
    }

    fn pipeline_stages(&self) -> Vec<Box<dyn PipelineStage>> {
        vec![Box::new(TafParseStage)]
    }

    fn metadata(&self) -> ProviderMeta {
        ProviderMeta {
            display_name: "TAFs",
            category: ProviderCategory::Weather,
            description: "Terminal aerodrome forecasts",
            config_key: "taf",
        }
    }
}

#[derive(Debug, Deserialize)]
struct TafJson {
    #[serde(rename = "icaoId")]
    icao_id: Option<String>,
    #[serde(rename = "rawTAF")]
    raw_taf: Option<String>,
    /// Issue time: may be an ISO 8601 string or epoch seconds.
    #[serde(rename = "issueTime")]
    issue_time: Option<Value>,
    /// Valid from: may be an ISO 8601 string or epoch seconds integer.
    #[serde(rename = "validTimeFrom")]
    valid_time_from: Option<Value>,
    /// Valid to: may be an ISO 8601 string or epoch seconds integer.
    #[serde(rename = "validTimeTo")]
    valid_time_to: Option<Value>,
}

struct TafParseStage;

impl PipelineStage for TafParseStage {
    fn name(&self) -> &str {
        "taf_parse"
    }

    fn phase(&self) -> PipelinePhase {
        PipelinePhase::Parse
    }

    fn execute(&self, data: &mut PipelineData) -> Result<(), PipelineError> {
        let raw = data.raw_bytes.take().unwrap_or_default();
        if raw.is_empty() {
            return Ok(());
        }

        let items: Vec<TafJson> =
            serde_json::from_slice(&raw).map_err(|e| PipelineError::StageError {
                stage: self.name().to_string(),
                message: format!("JSON parse error: {}", e),
            })?;

        let now = Utc::now();

        for item in &items {
            if let Some(record) = parse_taf_item(item, now) {
                data.records.push(CanonicalRecord::Taf(record));
            }
        }

        Ok(())
    }
}

fn parse_taf_item(item: &TafJson, fetched_at: DateTime<Utc>) -> Option<TafReport> {
    let icao = item.icao_id.as_ref()?.clone();

    let issue_time = parse_flexible_datetime(&item.issue_time).unwrap_or(fetched_at);

    let valid_from = parse_flexible_datetime(&item.valid_time_from).unwrap_or(fetched_at);

    let valid_to = parse_flexible_datetime(&item.valid_time_to).unwrap_or(fetched_at);

    Some(TafReport {
        icao,
        raw_text: item.raw_taf.clone().unwrap_or_default(),
        issue_time,
        valid_from,
        valid_to,
        fetched_at,
    })
}

// ---------------------------------------------------------------------------
// SIGMET
// ---------------------------------------------------------------------------

/// Provider that fetches international SIGMETs from aviationweather.gov.
pub struct SigmetProvider;

impl DataProvider for SigmetProvider {
    fn name(&self) -> &str {
        "aviationweather_sigmet"
    }

    fn schedule(&self) -> &str {
        "0 */15 * * * *"
    }

    fn fetch(&self, _ctx: &FetchContext) -> Result<RawFetchResult, ProviderError> {
        let url = format!("{}/isigmet?format=json", AVIATION_WEATHER_BASE);
        let data = http_get(&url)?;
        Ok(RawFetchResult {
            data,
            content_type: Some("application/json".to_string()),
            source: url,
        })
    }

    fn pipeline_stages(&self) -> Vec<Box<dyn PipelineStage>> {
        vec![Box::new(SigmetParseStage)]
    }

    fn metadata(&self) -> ProviderMeta {
        ProviderMeta {
            display_name: "SIGMETs",
            category: ProviderCategory::Weather,
            description: "Significant meteorological information",
            config_key: "metar",
        }
    }
}

#[derive(Debug, Deserialize)]
struct SigmetJson {
    /// Unique ID: real API uses "seriesId", older format used "isigmetId".
    #[serde(alias = "isigmetId", alias = "seriesId")]
    sigmet_id: Option<String>,
    #[serde(rename = "icaoId")]
    icao_id: Option<String>,
    hazard: Option<String>,
    #[serde(rename = "rawSigmet")]
    raw_sigmet: Option<String>,
    /// Valid from: may be epoch seconds (integer) or ISO 8601 string.
    #[serde(rename = "validTimeFrom")]
    valid_time_from: Option<Value>,
    /// Valid to: may be epoch seconds (integer) or ISO 8601 string.
    #[serde(rename = "validTimeTo")]
    valid_time_to: Option<Value>,
    /// Lower altitude: real API uses "base", older format used "altitudeLo1".
    #[serde(alias = "altitudeLo1", alias = "base")]
    altitude_lo: Option<i32>,
    /// Upper altitude: real API uses "top", older format used "altitudeHi1".
    #[serde(alias = "altitudeHi1", alias = "top")]
    altitude_hi: Option<i32>,
    #[serde(default, deserialize_with = "deserialize_sigmet_coords")]
    coords: Vec<SigmetCoord>,
}

#[derive(Debug, Deserialize, Clone)]
struct SigmetCoord {
    lat: Option<f64>,
    lon: Option<f64>,
}

/// Deserialize coords that may be `[{lat,lon}]` or `[[{lat,lon}]]` (nested polygons).
fn deserialize_sigmet_coords<'de, D>(deserializer: D) -> Result<Vec<SigmetCoord>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value: Option<Value> = Option::deserialize(deserializer)?;
    let Some(Value::Array(arr)) = value else {
        return Ok(Vec::new());
    };

    let mut result = Vec::new();
    for item in arr {
        match item {
            // Flat: {"lat": f64, "lon": f64}
            Value::Object(_) => {
                if let Ok(coord) = serde_json::from_value::<SigmetCoord>(item) {
                    result.push(coord);
                }
            }
            // Nested: [{...}, {...}]
            Value::Array(inner) => {
                for inner_item in inner {
                    if let Ok(coord) = serde_json::from_value::<SigmetCoord>(inner_item) {
                        result.push(coord);
                    }
                }
            }
            _ => {}
        }
    }
    Ok(result)
}

struct SigmetParseStage;

impl PipelineStage for SigmetParseStage {
    fn name(&self) -> &str {
        "sigmet_parse"
    }

    fn phase(&self) -> PipelinePhase {
        PipelinePhase::Parse
    }

    fn execute(&self, data: &mut PipelineData) -> Result<(), PipelineError> {
        let raw = data.raw_bytes.take().unwrap_or_default();
        if raw.is_empty() {
            return Ok(());
        }

        let items: Vec<SigmetJson> =
            serde_json::from_slice(&raw).map_err(|e| PipelineError::StageError {
                stage: self.name().to_string(),
                message: format!("JSON parse error: {}", e),
            })?;

        let now = Utc::now();

        for item in &items {
            if let Some(record) = parse_sigmet_item(item, now) {
                data.records.push(CanonicalRecord::Sigmet(record));
            }
        }

        Ok(())
    }
}

fn parse_sigmet_item(item: &SigmetJson, fetched_at: DateTime<Utc>) -> Option<SigmetReport> {
    let id = item.sigmet_id.as_ref()?.clone();

    let region = item.icao_id.clone().unwrap_or_default();
    let hazard = item.hazard.clone().unwrap_or_default();

    let valid_from = parse_flexible_datetime(&item.valid_time_from).unwrap_or(fetched_at);

    let valid_to = parse_flexible_datetime(&item.valid_time_to).unwrap_or(fetched_at);

    let polygon: Vec<(f64, f64)> = item
        .coords
        .iter()
        .filter_map(|c| Some((c.lat?, c.lon?)))
        .collect();

    Some(SigmetReport {
        id,
        region,
        hazard,
        raw_text: item.raw_sigmet.clone().unwrap_or_default(),
        valid_from,
        valid_to,
        min_altitude_ft: item.altitude_lo,
        max_altitude_ft: item.altitude_hi,
        polygon,
        fetched_at,
    })
}

// ---------------------------------------------------------------------------
// AIRMET
// ---------------------------------------------------------------------------

/// Provider that fetches AIRMETs from aviationweather.gov.
pub struct AirmetProvider;

impl DataProvider for AirmetProvider {
    fn name(&self) -> &str {
        "aviationweather_airmet"
    }

    fn schedule(&self) -> &str {
        "0 */15 * * * *"
    }

    fn fetch(&self, _ctx: &FetchContext) -> Result<RawFetchResult, ProviderError> {
        let url = format!("{}/airmet?format=json", AVIATION_WEATHER_BASE);
        let data = http_get(&url)?;
        Ok(RawFetchResult {
            data,
            content_type: Some("application/json".to_string()),
            source: url,
        })
    }

    fn pipeline_stages(&self) -> Vec<Box<dyn PipelineStage>> {
        vec![Box::new(AirmetParseStage)]
    }

    fn metadata(&self) -> ProviderMeta {
        ProviderMeta {
            display_name: "AIRMETs",
            category: ProviderCategory::Weather,
            description: "Airmen meteorological information",
            config_key: "metar",
        }
    }
}

#[derive(Debug, Deserialize)]
struct AirmetJson {
    /// Unique ID: may be "airmetId" or synthesized from region + hazard.
    #[serde(rename = "airmetId")]
    airmet_id: Option<String>,
    /// Region code: real API uses "region" instead of "icaoId".
    #[serde(alias = "icaoId", alias = "region")]
    region_id: Option<String>,
    hazard: Option<String>,
    #[serde(rename = "rawAirmet")]
    raw_airmet: Option<String>,
    /// Valid from: may be epoch seconds (integer) or ISO 8601 string.
    #[serde(rename = "validTimeFrom")]
    valid_time_from: Option<Value>,
    /// Valid to: may be epoch seconds (integer) or ISO 8601 string.
    #[serde(rename = "validTimeTo")]
    valid_time_to: Option<Value>,
    #[serde(default, deserialize_with = "deserialize_sigmet_coords")]
    coords: Vec<SigmetCoord>,
}

struct AirmetParseStage;

impl PipelineStage for AirmetParseStage {
    fn name(&self) -> &str {
        "airmet_parse"
    }

    fn phase(&self) -> PipelinePhase {
        PipelinePhase::Parse
    }

    fn execute(&self, data: &mut PipelineData) -> Result<(), PipelineError> {
        let raw = data.raw_bytes.take().unwrap_or_default();
        if raw.is_empty() {
            return Ok(());
        }

        let items: Vec<AirmetJson> =
            serde_json::from_slice(&raw).map_err(|e| PipelineError::StageError {
                stage: self.name().to_string(),
                message: format!("JSON parse error: {}", e),
            })?;

        let now = Utc::now();

        for item in &items {
            if let Some(record) = parse_airmet_item(item, now) {
                data.records.push(CanonicalRecord::Airmet(record));
            }
        }

        Ok(())
    }
}

fn parse_airmet_item(item: &AirmetJson, fetched_at: DateTime<Utc>) -> Option<AirmetReport> {
    let region = item.region_id.clone().unwrap_or_default();
    let hazard = item.hazard.clone().unwrap_or_default();

    // Use airmetId if present, otherwise synthesize from region + hazard
    let id = item.airmet_id.clone().unwrap_or_else(|| {
        if region.is_empty() && hazard.is_empty() {
            return String::new();
        }
        format!("{}-{}", region, hazard)
    });

    // Need at least region or id to produce a valid record
    if id.is_empty() && region.is_empty() {
        return None;
    }

    let valid_from = parse_flexible_datetime(&item.valid_time_from).unwrap_or(fetched_at);

    let valid_to = parse_flexible_datetime(&item.valid_time_to).unwrap_or(fetched_at);

    let polygon: Vec<(f64, f64)> = item
        .coords
        .iter()
        .filter_map(|c| Some((c.lat?, c.lon?)))
        .collect();

    Some(AirmetReport {
        id,
        region,
        hazard,
        raw_text: item.raw_airmet.clone().unwrap_or_default(),
        valid_from,
        valid_to,
        polygon,
        fetched_at,
    })
}

// ---------------------------------------------------------------------------
// PIREP
// ---------------------------------------------------------------------------

/// Provider that fetches PIREPs from aviationweather.gov.
pub struct PirepProvider;

impl DataProvider for PirepProvider {
    fn name(&self) -> &str {
        "aviationweather_pirep"
    }

    fn schedule(&self) -> &str {
        "0 */5 * * * *"
    }

    fn fetch(&self, ctx: &FetchContext) -> Result<RawFetchResult, ProviderError> {
        let (lat1, lon1, lat2, lon2) = bbox_from_context(ctx);
        let url = format!(
            "{}/pirep?format=json&bbox={},{},{},{}",
            AVIATION_WEATHER_BASE, lat1, lon1, lat2, lon2
        );

        let data = http_get(&url)?;
        Ok(RawFetchResult {
            data,
            content_type: Some("application/json".to_string()),
            source: url,
        })
    }

    fn pipeline_stages(&self) -> Vec<Box<dyn PipelineStage>> {
        vec![Box::new(PirepParseStage)]
    }

    fn metadata(&self) -> ProviderMeta {
        ProviderMeta {
            display_name: "PIREPs",
            category: ProviderCategory::Weather,
            description: "Pilot weather reports",
            config_key: "metar",
        }
    }
}

#[derive(Debug, Deserialize)]
struct PirepJson {
    #[serde(rename = "rawOb")]
    raw_ob: Option<String>,
    lat: Option<f64>,
    lon: Option<f64>,
    /// Altitude: may be "altFt" (feet MSL) or "fltLvl" (flight level in hundreds of feet).
    #[serde(alias = "altFt")]
    alt_ft: Option<i32>,
    /// Flight level (hundreds of feet). Used when altFt is absent.
    #[serde(rename = "fltLvl")]
    flt_lvl: Option<i32>,
    /// Observation time: may be epoch seconds (integer) or ISO 8601 string.
    #[serde(rename = "obsTime")]
    obs_time: Option<Value>,
    #[serde(rename = "acType")]
    ac_type: Option<String>,
    /// Report type: real API uses "pirepType", older format used "repType".
    #[serde(alias = "repType", alias = "pirepType")]
    rep_type: Option<String>,
}

struct PirepParseStage;

impl PipelineStage for PirepParseStage {
    fn name(&self) -> &str {
        "pirep_parse"
    }

    fn phase(&self) -> PipelinePhase {
        PipelinePhase::Parse
    }

    fn execute(&self, data: &mut PipelineData) -> Result<(), PipelineError> {
        let raw = data.raw_bytes.take().unwrap_or_default();
        if raw.is_empty() {
            return Ok(());
        }

        let items: Vec<PirepJson> =
            serde_json::from_slice(&raw).map_err(|e| PipelineError::StageError {
                stage: self.name().to_string(),
                message: format!("JSON parse error: {}", e),
            })?;

        let now = Utc::now();

        for item in &items {
            if let Some(record) = parse_pirep_item(item, now) {
                data.records.push(CanonicalRecord::Pirep(record));
            }
        }

        Ok(())
    }
}

fn parse_pirep_item(item: &PirepJson, fetched_at: DateTime<Utc>) -> Option<PirepReport> {
    let latitude = item.lat?;
    let longitude = item.lon?;

    let observation_time = parse_flexible_datetime(&item.obs_time).unwrap_or(fetched_at);

    // Altitude: prefer altFt, fall back to fltLvl * 100
    let altitude_ft = item
        .alt_ft
        .or_else(|| item.flt_lvl.map(|fl| fl * 100))
        .unwrap_or(0);

    Some(PirepReport {
        raw_text: item.raw_ob.clone().unwrap_or_default(),
        latitude,
        longitude,
        altitude_ft,
        observation_time,
        aircraft_type: item.ac_type.clone(),
        report_type: item.rep_type.clone().unwrap_or_else(|| "PIREP".to_string()),
        fetched_at,
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data_ingest::pipeline::run_pipeline;

    // -- METAR tests --

    fn sample_metar_json() -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!([
            {
                "icaoId": "KICT",
                "rawOb": "KICT 021753Z 27010G15KT 10SM FEW050 BKN120 15/10 A2992",
                "obsTime": "2026-03-02T17:53:00Z",
                "temp": 15.0,
                "dewp": 10.0,
                "wdir": "270",
                "wspd": 10,
                "wgst": 15,
                "visib": "10+",
                "altim": 29.92,
                "fltcat": "VFR",
                "clouds": [
                    {"cover": "FEW", "base": 5000},
                    {"cover": "BKN", "base": 12000}
                ]
            },
            {
                "icaoId": "KJFK",
                "rawOb": "KJFK 021753Z VRB05KT 1/2SM OVC005 02/01 A3010",
                "obsTime": "2026-03-02T17:53:00Z",
                "temp": 2.0,
                "dewp": 1.0,
                "wdir": "VRB",
                "wspd": 5,
                "wgst": null,
                "visib": "1/2",
                "altim": 30.10,
                "fltcat": "LIFR",
                "clouds": [
                    {"cover": "OVC", "base": 500}
                ]
            }
        ]))
        .unwrap()
    }

    #[test]
    fn test_parse_metar_response() {
        let data = PipelineData {
            raw_bytes: Some(sample_metar_json()),
            ..Default::default()
        };

        let stages: Vec<Box<dyn PipelineStage>> = vec![Box::new(MetarParseStage)];
        let result = run_pipeline(&stages, data).unwrap();

        assert_eq!(result.records.len(), 2);

        if let CanonicalRecord::Metar(m) = &result.records[0] {
            assert_eq!(m.icao, "KICT");
            assert_eq!(m.wind_direction_deg, Some(270));
            assert_eq!(m.wind_speed_kt, Some(10));
            assert_eq!(m.wind_gust_kt, Some(15));
            assert_eq!(m.visibility_sm, Some(10.0));
            assert_eq!(m.ceiling_ft, Some(12000));
            assert_eq!(m.temperature_c, Some(15.0));
            assert_eq!(m.dewpoint_c, Some(10.0));
            assert_eq!(m.altimeter_inhg, Some(29.92));
            assert_eq!(m.flight_category, "VFR");
        } else {
            panic!("expected Metar record");
        }

        if let CanonicalRecord::Metar(m) = &result.records[1] {
            assert_eq!(m.icao, "KJFK");
            assert_eq!(m.wind_direction_deg, None); // VRB
            assert_eq!(m.wind_gust_kt, None);
            assert_eq!(m.visibility_sm, Some(0.5));
            assert_eq!(m.ceiling_ft, Some(500));
            assert_eq!(m.flight_category, "LIFR");
        } else {
            panic!("expected Metar record");
        }
    }

    #[test]
    fn test_parse_visibility_formats() {
        assert_eq!(parse_visibility("10+"), Some(10.0));
        assert_eq!(parse_visibility("10"), Some(10.0));
        assert_eq!(parse_visibility("1/2"), Some(0.5));
        assert_eq!(parse_visibility("1 1/2"), Some(1.5));
        assert_eq!(parse_visibility("3"), Some(3.0));
        assert_eq!(parse_visibility("1/4"), Some(0.25));
    }

    #[test]
    fn test_metar_empty_response() {
        let data = PipelineData {
            raw_bytes: Some(b"[]".to_vec()),
            ..Default::default()
        };

        let stages: Vec<Box<dyn PipelineStage>> = vec![Box::new(MetarParseStage)];
        let result = run_pipeline(&stages, data).unwrap();
        assert!(result.records.is_empty());
    }

    #[test]
    fn test_metar_provider_metadata() {
        let provider = MetarProvider;
        assert_eq!(provider.name(), "aviationweather_metar");
        assert_eq!(provider.schedule(), "0 */5 * * * *");
    }

    // -- TAF tests --

    fn sample_taf_json() -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!([
            {
                "icaoId": "KICT",
                "rawTAF": "TAF KICT 021730Z 0218/0318 27010KT P6SM FEW050",
                "issueTime": "2026-03-02T17:30:00Z",
                "validTimeFrom": "2026-03-02T18:00:00Z",
                "validTimeTo": "2026-03-03T18:00:00Z"
            }
        ]))
        .unwrap()
    }

    #[test]
    fn test_parse_taf_response() {
        let data = PipelineData {
            raw_bytes: Some(sample_taf_json()),
            ..Default::default()
        };

        let stages: Vec<Box<dyn PipelineStage>> = vec![Box::new(TafParseStage)];
        let result = run_pipeline(&stages, data).unwrap();

        assert_eq!(result.records.len(), 1);

        if let CanonicalRecord::Taf(t) = &result.records[0] {
            assert_eq!(t.icao, "KICT");
            assert!(t.raw_text.contains("TAF KICT"));
        } else {
            panic!("expected Taf record");
        }
    }

    #[test]
    fn test_taf_provider_metadata() {
        let provider = TafProvider;
        assert_eq!(provider.name(), "aviationweather_taf");
        assert_eq!(provider.schedule(), "0 */15 * * * *");
    }

    // -- SIGMET tests --

    fn sample_sigmet_json() -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!([
            {
                "isigmetId": "SIG-001",
                "icaoId": "KKCI",
                "hazard": "TURB",
                "rawSigmet": "SIGMET TANGO 1 VALID ...",
                "validTimeFrom": "2026-03-02T18:00:00Z",
                "validTimeTo": "2026-03-02T22:00:00Z",
                "altitudeLo1": 25000,
                "altitudeHi1": 45000,
                "coords": [
                    {"lat": 37.0, "lon": -97.0},
                    {"lat": 38.0, "lon": -97.0},
                    {"lat": 38.0, "lon": -96.0},
                    {"lat": 37.0, "lon": -96.0}
                ]
            }
        ]))
        .unwrap()
    }

    #[test]
    fn test_parse_sigmet_response() {
        let data = PipelineData {
            raw_bytes: Some(sample_sigmet_json()),
            ..Default::default()
        };

        let stages: Vec<Box<dyn PipelineStage>> = vec![Box::new(SigmetParseStage)];
        let result = run_pipeline(&stages, data).unwrap();

        assert_eq!(result.records.len(), 1);

        if let CanonicalRecord::Sigmet(s) = &result.records[0] {
            assert_eq!(s.id, "SIG-001");
            assert_eq!(s.region, "KKCI");
            assert_eq!(s.hazard, "TURB");
            assert_eq!(s.min_altitude_ft, Some(25000));
            assert_eq!(s.max_altitude_ft, Some(45000));
            assert_eq!(s.polygon.len(), 4);
            assert!((s.polygon[0].0 - 37.0).abs() < 0.001);
        } else {
            panic!("expected Sigmet record");
        }
    }

    #[test]
    fn test_sigmet_provider_metadata() {
        let provider = SigmetProvider;
        assert_eq!(provider.name(), "aviationweather_sigmet");
        assert_eq!(provider.schedule(), "0 */15 * * * *");
    }

    // -- AIRMET tests --

    fn sample_airmet_json() -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!([
            {
                "airmetId": "AIRMET-001",
                "icaoId": "KKCI",
                "hazard": "IFR",
                "rawAirmet": "AIRMET SIERRA FOR IFR ...",
                "validTimeFrom": "2026-03-02T18:00:00Z",
                "validTimeTo": "2026-03-02T22:00:00Z",
                "coords": [
                    {"lat": 36.0, "lon": -98.0},
                    {"lat": 37.0, "lon": -98.0},
                    {"lat": 37.0, "lon": -97.0},
                    {"lat": 36.0, "lon": -97.0}
                ]
            }
        ]))
        .unwrap()
    }

    #[test]
    fn test_parse_airmet_response() {
        let data = PipelineData {
            raw_bytes: Some(sample_airmet_json()),
            ..Default::default()
        };

        let stages: Vec<Box<dyn PipelineStage>> = vec![Box::new(AirmetParseStage)];
        let result = run_pipeline(&stages, data).unwrap();

        assert_eq!(result.records.len(), 1);

        if let CanonicalRecord::Airmet(a) = &result.records[0] {
            assert_eq!(a.id, "AIRMET-001");
            assert_eq!(a.region, "KKCI");
            assert_eq!(a.hazard, "IFR");
            assert_eq!(a.polygon.len(), 4);
        } else {
            panic!("expected Airmet record");
        }
    }

    #[test]
    fn test_airmet_provider_metadata() {
        let provider = AirmetProvider;
        assert_eq!(provider.name(), "aviationweather_airmet");
        assert_eq!(provider.schedule(), "0 */15 * * * *");
    }

    // -- PIREP tests --

    fn sample_pirep_json() -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!([
            {
                "rawOb": "ICT UA /OV ICT/TM 1800/FL350/TP B737/TB MOD",
                "lat": 37.65,
                "lon": -97.43,
                "altFt": 35000,
                "obsTime": "2026-03-02T18:00:00Z",
                "acType": "B737",
                "repType": "PIREP"
            },
            {
                "rawOb": "MCI UUA /OV MCI/TM 1815/FL280/TP CRJ2/TB SEV",
                "lat": 39.30,
                "lon": -94.71,
                "altFt": 28000,
                "obsTime": "2026-03-02T18:15:00Z",
                "acType": "CRJ2",
                "repType": "URGENT PIREP"
            }
        ]))
        .unwrap()
    }

    #[test]
    fn test_parse_pirep_response() {
        let data = PipelineData {
            raw_bytes: Some(sample_pirep_json()),
            ..Default::default()
        };

        let stages: Vec<Box<dyn PipelineStage>> = vec![Box::new(PirepParseStage)];
        let result = run_pipeline(&stages, data).unwrap();

        assert_eq!(result.records.len(), 2);

        if let CanonicalRecord::Pirep(p) = &result.records[0] {
            assert!((p.latitude - 37.65).abs() < 0.001);
            assert!((p.longitude - (-97.43)).abs() < 0.001);
            assert_eq!(p.altitude_ft, 35000);
            assert_eq!(p.aircraft_type, Some("B737".to_string()));
            assert_eq!(p.report_type, "PIREP");
        } else {
            panic!("expected Pirep record");
        }

        if let CanonicalRecord::Pirep(p) = &result.records[1] {
            assert_eq!(p.report_type, "URGENT PIREP");
        } else {
            panic!("expected Pirep record");
        }
    }

    #[test]
    fn test_pirep_missing_coords_skipped() {
        let json = serde_json::to_vec(&serde_json::json!([
            {
                "rawOb": "no coords",
                "lat": null,
                "lon": null,
                "altFt": 10000,
                "obsTime": "2026-03-02T18:00:00Z",
                "repType": "PIREP"
            }
        ]))
        .unwrap();

        let data = PipelineData {
            raw_bytes: Some(json),
            ..Default::default()
        };

        let stages: Vec<Box<dyn PipelineStage>> = vec![Box::new(PirepParseStage)];
        let result = run_pipeline(&stages, data).unwrap();
        assert!(result.records.is_empty());
    }

    #[test]
    fn test_pirep_provider_metadata() {
        let provider = PirepProvider;
        assert_eq!(provider.name(), "aviationweather_pirep");
        assert_eq!(provider.schedule(), "0 */5 * * * *");
    }

    // -- Bounding box tests --

    #[test]
    fn test_bbox_from_context() {
        let ctx = FetchContext {
            center_latitude: 37.6872,
            center_longitude: -97.3301,
            radius_nm: 120.0,
        };
        let (lat_min, lon_min, lat_max, lon_max) = bbox_from_context(&ctx);

        assert!(lat_min < ctx.center_latitude);
        assert!(lat_max > ctx.center_latitude);
        assert!(lon_min < ctx.center_longitude);
        assert!(lon_max > ctx.center_longitude);
        // Lat range should be about 4 degrees (120nm / 60nm per degree on each side = 2 deg each side)
        assert!((lat_max - lat_min - 4.0).abs() < 0.01);
    }
}
