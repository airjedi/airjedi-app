use std::f64::consts::PI;
use std::path::PathBuf;

use bevy::prelude::*;
use chrono::Utc;

use crate::data_ingest::canonical::{AirspaceInfo, CanonicalRecord};
use crate::data_ingest::pipeline::{PipelineData, PipelineError, PipelinePhase, PipelineStage};
use crate::data_ingest::provider::{
    DataProvider, FetchContext, ProviderCategory, ProviderError, ProviderMeta, RawFetchResult,
};

/// Number of points to approximate a circle in polygon form.
const CIRCLE_SEGMENTS: usize = 36;

/// Provider for OpenAIP airspace data in OpenAir format.
///
/// Supports two data sources:
/// 1. HTTP URL (e.g. from OpenAIP or a mirror)
/// 2. Local file path (fallback if HTTP is unavailable or for offline use)
pub struct OpenAipProvider {
    /// URL to fetch OpenAir data from. May require authentication.
    url: Option<String>,
    /// Local file path as fallback.
    local_path: Option<PathBuf>,
}

impl OpenAipProvider {
    pub fn new() -> Self {
        Self {
            url: None,
            local_path: None,
        }
    }

    /// Set the HTTP URL to fetch OpenAir data from.
    pub fn with_url(mut self, url: impl Into<String>) -> Self {
        self.url = Some(url.into());
        self
    }

    /// Set a local file path as fallback data source.
    pub fn with_local_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.local_path = Some(path.into());
        self
    }
}

impl DataProvider for OpenAipProvider {
    fn name(&self) -> &str {
        "openaip"
    }

    /// Run daily at 07:00 UTC (airspace data changes infrequently).
    fn schedule(&self) -> &str {
        "0 0 7 * * *"
    }

    fn fetch(&self, _ctx: &FetchContext) -> Result<RawFetchResult, ProviderError> {
        // Try HTTP first
        if let Some(ref url) = self.url {
            match fetch_from_url(url) {
                Ok(data) => {
                    return Ok(RawFetchResult {
                        data,
                        content_type: Some("text/plain".to_string()),
                        source: url.clone(),
                    });
                }
                Err(e) => {
                    warn!("OpenAIP: HTTP fetch failed ({}), trying local fallback", e);
                }
            }
        }

        // Fallback to local file
        if let Some(ref path) = self.local_path {
            let data = std::fs::read(path).map_err(|e| {
                ProviderError::Other(format!("failed to read local file {:?}: {}", path, e))
            })?;
            return Ok(RawFetchResult {
                data,
                content_type: Some("text/plain".to_string()),
                source: format!("file://{}", path.display()),
            });
        }

        Err(ProviderError::Other(
            "no URL or local path configured for OpenAIP provider".to_string(),
        ))
    }

    fn pipeline_stages(&self) -> Vec<Box<dyn PipelineStage>> {
        vec![Box::new(OpenAirParseStage)]
    }

    fn metadata(&self) -> ProviderMeta {
        ProviderMeta {
            display_name: "Airspace Boundaries",
            category: ProviderCategory::Navigation,
            description: "Airspace boundaries in OpenAir format",
            config_key: "openaip",
        }
    }
}

fn fetch_from_url(url: &str) -> Result<Vec<u8>, ProviderError> {
    // Derive a cache key from the URL filename
    let cache_key = format!("openaip_{}", url.rsplit('/').next().unwrap_or("data.txt"));
    let result = crate::data_ingest::http_cache::fetch_with_cache(url, &cache_key, 60)?;
    Ok(result.into_bytes())
}

// ---------------------------------------------------------------------------
// OpenAir format parser
// ---------------------------------------------------------------------------

/// Pipeline stage that parses OpenAir format text into AirspaceInfo records.
struct OpenAirParseStage;

impl PipelineStage for OpenAirParseStage {
    fn name(&self) -> &str {
        "openair_parse"
    }

    fn phase(&self) -> PipelinePhase {
        PipelinePhase::Parse
    }

    fn execute(&self, data: &mut PipelineData) -> Result<(), PipelineError> {
        let raw = data
            .raw_bytes
            .take()
            .ok_or_else(|| PipelineError::StageError {
                stage: self.name().to_string(),
                message: "no raw bytes".to_string(),
            })?;

        let text = String::from_utf8_lossy(&raw);
        let airspaces = parse_openair(&text);

        info!("OpenAIP: parsed {} airspace records", airspaces.len());
        data.records.extend(airspaces);
        data.metadata
            .insert("source".to_string(), "openaip".to_string());
        Ok(())
    }
}

/// Intermediate state while parsing an OpenAir airspace block.
#[derive(Default)]
struct AirspaceBuilder {
    class: Option<String>,
    name: Option<String>,
    upper_limit: Option<String>,
    lower_limit: Option<String>,
    points: Vec<(f64, f64)>,
    /// Center for circle/arc definitions.
    center: Option<(f64, f64)>,
}

impl AirspaceBuilder {
    fn build(self, fetched_at: chrono::DateTime<chrono::Utc>) -> Option<CanonicalRecord> {
        let class = self.class?;
        let name = self.name.unwrap_or_else(|| "Unknown".to_string());

        if self.points.is_empty() {
            return None;
        }

        let airspace_type = classify_airspace_type(&class);
        let lower_limit_ft = self.lower_limit.as_deref().and_then(parse_altitude);
        let upper_limit_ft = self.upper_limit.as_deref().and_then(parse_altitude);

        Some(CanonicalRecord::Airspace(AirspaceInfo {
            name,
            airspace_class: class,
            airspace_type,
            lower_limit_ft,
            upper_limit_ft,
            lower_altitude_ref: None,
            upper_altitude_ref: None,
            polygon: self.points,
            fetched_at,
        }))
    }
}

/// Parse an OpenAir format string into canonical airspace records.
pub(crate) fn parse_openair(text: &str) -> Vec<CanonicalRecord> {
    let mut records = Vec::new();
    let mut current: Option<AirspaceBuilder> = None;
    let now = Utc::now();

    for line in text.lines() {
        let line = line.trim();

        // Skip comments and empty lines
        if line.is_empty() || line.starts_with('*') {
            continue;
        }

        // AC = Airspace Class — starts a new block
        if let Some(rest) = line.strip_prefix("AC ") {
            // Finalize previous block
            if let Some(builder) = current.take() {
                if let Some(record) = builder.build(now) {
                    records.push(record);
                }
            }
            current = Some(AirspaceBuilder {
                class: Some(rest.trim().to_string()),
                ..Default::default()
            });
            continue;
        }

        let Some(ref mut builder) = current else {
            continue;
        };

        if let Some(rest) = line.strip_prefix("AN ") {
            builder.name = Some(rest.trim().to_string());
        } else if let Some(rest) = line.strip_prefix("AH ") {
            builder.upper_limit = Some(rest.trim().to_string());
        } else if let Some(rest) = line.strip_prefix("AL ") {
            builder.lower_limit = Some(rest.trim().to_string());
        } else if let Some(rest) = line.strip_prefix("DP ") {
            if let Some(point) = parse_openair_coord(rest.trim()) {
                builder.points.push(point);
            }
        } else if let Some(rest) = line.strip_prefix("V ") {
            // Variable assignment: V X=lat:lon or V D=+ (direction)
            let rest = rest.trim();
            if let Some(coord_str) = rest.strip_prefix("X=") {
                builder.center = parse_openair_coord(coord_str.trim());
            }
        } else if let Some(rest) = line.strip_prefix("DC ") {
            // Circle: DC radius_nm
            if let (Some(center), Some(radius_nm)) =
                (builder.center, rest.trim().parse::<f64>().ok())
            {
                let circle_points = circle_to_polygon(center, radius_nm);
                builder.points.extend(circle_points);
            }
        } else if let Some(rest) = line.strip_prefix("DA ") {
            // Arc: DA radius, start_angle, end_angle
            if let Some(center) = builder.center {
                if let Some(arc_points) = parse_arc(rest.trim(), center) {
                    builder.points.extend(arc_points);
                }
            }
        }
        // DB (arc by two points) and other less common directives are skipped
    }

    // Finalize last block
    if let Some(builder) = current.take() {
        if let Some(record) = builder.build(now) {
            records.push(record);
        }
    }

    records
}

/// Parse an OpenAir coordinate in one of these formats:
/// - `37:30:00 N 097:26:00 W`
/// - `37:30:00N 097:26:00W`
/// - `37.5000 -97.4333`
/// - `37.5000:-97.4333` (colon-separated decimal, used in V X= directives)
fn parse_openair_coord(s: &str) -> Option<(f64, f64)> {
    let parts: Vec<&str> = s.split_whitespace().collect();

    // Try colon-separated decimal degrees: "37.5000:-97.4333"
    // Must check before DMS since DMS also uses colons but has direction letters.
    if parts.len() == 1 && s.contains(':') {
        let colon_parts: Vec<&str> = s.splitn(2, ':').collect();
        if colon_parts.len() == 2 {
            // Only parse as colon-separated decimal if both parts look like decimal numbers
            // (no direction letters like N/S/E/W at end)
            let last_a = colon_parts[0].chars().last().unwrap_or(' ');
            let last_b = colon_parts[1].chars().last().unwrap_or(' ');
            if last_a.is_ascii_digit() && (last_b.is_ascii_digit() || last_b == '.') {
                // Attempt to split on the FIRST colon that separates lat:lon
                // Handle negative coordinates like "39.2975:-94.7139"
                if let (Ok(lat), Ok(lon)) =
                    (colon_parts[0].parse::<f64>(), colon_parts[1].parse::<f64>())
                {
                    if (-90.0..=90.0).contains(&lat) && (-180.0..=180.0).contains(&lon) {
                        return Some((lat, lon));
                    }
                }
            }
        }
    }

    // Try decimal degrees: "37.5000 -97.4333"
    if parts.len() == 2 {
        if let (Ok(lat), Ok(lon)) = (parts[0].parse::<f64>(), parts[1].parse::<f64>()) {
            if (-90.0..=90.0).contains(&lat) && (-180.0..=180.0).contains(&lon) {
                return Some((lat, lon));
            }
        }
    }

    // Try DMS with separate direction: "37:30:00 N 097:26:00 W"
    if parts.len() == 4 {
        let lat = parse_openair_dms(parts[0], parts[1])?;
        let lon = parse_openair_dms(parts[2], parts[3])?;
        return Some((lat, lon));
    }

    // Try DMS with embedded direction: "37:30:00N 097:26:00W"
    if parts.len() == 2 {
        let lat = parse_dms_embedded(parts[0])?;
        let lon = parse_dms_embedded(parts[1])?;
        return Some((lat, lon));
    }

    None
}

/// Parse DMS like "37:30:00" with direction "N"/"S"/"E"/"W".
fn parse_openair_dms(dms: &str, direction: &str) -> Option<f64> {
    let parts: Vec<&str> = dms.split(':').collect();
    if parts.len() < 2 {
        return None;
    }

    let degrees: f64 = parts[0].parse().ok()?;
    let minutes: f64 = parts[1].parse().ok()?;
    let seconds: f64 = if parts.len() > 2 {
        parts[2].parse().unwrap_or(0.0)
    } else {
        0.0
    };

    let mut decimal = degrees + minutes / 60.0 + seconds / 3600.0;

    match direction.trim().to_uppercase().as_str() {
        "S" | "W" => decimal = -decimal,
        "N" | "E" => {}
        _ => return None,
    }

    Some(decimal)
}

/// Parse DMS with embedded direction like "37:30:00N" or "097:26:00W".
fn parse_dms_embedded(s: &str) -> Option<f64> {
    if s.is_empty() {
        return None;
    }

    let (numeric, dir) = s.split_at(s.len() - 1);
    let parts: Vec<&str> = numeric.split(':').collect();
    if parts.len() < 2 {
        return None;
    }

    let degrees: f64 = parts[0].parse().ok()?;
    let minutes: f64 = parts[1].parse().ok()?;
    let seconds: f64 = if parts.len() > 2 {
        parts[2].parse().unwrap_or(0.0)
    } else {
        0.0
    };

    let mut decimal = degrees + minutes / 60.0 + seconds / 3600.0;

    match dir {
        "S" | "s" | "W" | "w" => decimal = -decimal,
        "N" | "n" | "E" | "e" => {}
        _ => return None,
    }

    Some(decimal)
}

/// Parse altitude string into feet.
///
/// Supported formats:
/// - `FL180` → 18000 ft
/// - `FL350` → 35000 ft
/// - `SFC` / `GND` → 0 ft
/// - `UNL` / `UNLIM` → 60000 ft (unlimited)
/// - `1500 MSL` / `1500 ft MSL` → 1500 ft
/// - `1500 AGL` / `1500 ft AGL` → 1500 ft
/// - `1500` → 1500 ft (bare number assumed MSL)
fn parse_altitude(s: &str) -> Option<i32> {
    let s = s.trim().to_uppercase();

    if s == "SFC" || s == "GND" || s == "SURFACE" {
        return Some(0);
    }

    if s == "UNL" || s == "UNLIM" || s == "UNLIMITED" {
        return Some(60000);
    }

    if let Some(fl) = s.strip_prefix("FL") {
        if let Ok(level) = fl.trim().parse::<i32>() {
            return Some(level * 100);
        }
    }

    // Try to extract a number from strings like "1500 MSL", "3000 ft AGL", "5000FT"
    let numeric: String = s
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '-')
        .collect();
    if let Ok(alt) = numeric.parse::<i32>() {
        return Some(alt);
    }

    None
}

/// Classify airspace type from OpenAir class code.
fn classify_airspace_type(class: &str) -> String {
    match class.to_uppercase().as_str() {
        "A" => "Class A",
        "B" => "Class B",
        "C" => "Class C",
        "D" => "Class D",
        "E" => "Class E",
        "F" => "Class F",
        "G" => "Class G",
        "CTR" => "Control Zone",
        "TMA" => "Terminal Control Area",
        "TMZ" => "Transponder Mandatory Zone",
        "RMZ" => "Radio Mandatory Zone",
        "R" => "Restricted",
        "P" => "Prohibited",
        "Q" => "Danger",
        "W" => "Warning",
        "GP" => "Glider Prohibited",
        "GSEC" => "Glider Sector",
        _ => "Other",
    }
    .to_string()
}

/// Convert a circle (center + radius in NM) to a polygon approximation.
fn circle_to_polygon(center: (f64, f64), radius_nm: f64) -> Vec<(f64, f64)> {
    let (center_lat, center_lon) = center;
    // 1 NM ≈ 1/60 degree latitude
    let radius_deg_lat = radius_nm / 60.0;

    let mut points = Vec::with_capacity(CIRCLE_SEGMENTS);
    for i in 0..CIRCLE_SEGMENTS {
        let angle = 2.0 * PI * (i as f64) / (CIRCLE_SEGMENTS as f64);
        let lat = center_lat + radius_deg_lat * angle.cos();
        // Adjust longitude for latitude (cos correction)
        let lon_scale = (center_lat.to_radians()).cos();
        let lon = if lon_scale.abs() > 1e-10 {
            center_lon + (radius_deg_lat / lon_scale) * angle.sin()
        } else {
            center_lon
        };
        points.push((lat, lon));
    }

    points
}

/// Parse an arc definition: "radius, start_angle, end_angle" with center.
fn parse_arc(s: &str, center: (f64, f64)) -> Option<Vec<(f64, f64)>> {
    let parts: Vec<&str> = s.split(',').collect();
    if parts.len() < 3 {
        return None;
    }

    let radius_nm: f64 = parts[0].trim().parse().ok()?;
    let start_deg: f64 = parts[1].trim().parse().ok()?;
    let end_deg: f64 = parts[2].trim().parse().ok()?;

    let (center_lat, center_lon) = center;
    let radius_deg_lat = radius_nm / 60.0;

    let mut points = Vec::new();
    let start_rad = start_deg.to_radians();
    let end_rad = end_deg.to_radians();

    // Determine sweep direction (always go start → end, wrapping if needed)
    let step = 2.0 * PI / (CIRCLE_SEGMENTS as f64);

    let total = if end_rad > start_rad {
        end_rad - start_rad
    } else {
        2.0 * PI - (start_rad - end_rad)
    };

    let steps = (total / step).ceil() as usize;
    for i in 0..=steps {
        let a = start_rad + step * (i as f64);
        let a = if a > 2.0 * PI { a - 2.0 * PI } else { a };
        let lat = center_lat + radius_deg_lat * a.cos();
        let lon_scale = (center_lat.to_radians()).cos();
        let lon = if lon_scale.abs() > 1e-10 {
            center_lon + (radius_deg_lat / lon_scale) * a.sin()
        } else {
            center_lon
        };
        points.push((lat, lon));
    }

    Some(points)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_altitude_flight_level() {
        assert_eq!(parse_altitude("FL180"), Some(18000));
        assert_eq!(parse_altitude("FL350"), Some(35000));
        assert_eq!(parse_altitude("FL060"), Some(6000));
    }

    #[test]
    fn parse_altitude_surface() {
        assert_eq!(parse_altitude("SFC"), Some(0));
        assert_eq!(parse_altitude("GND"), Some(0));
        assert_eq!(parse_altitude("SURFACE"), Some(0));
    }

    #[test]
    fn parse_altitude_unlimited() {
        assert_eq!(parse_altitude("UNL"), Some(60000));
        assert_eq!(parse_altitude("UNLIM"), Some(60000));
    }

    #[test]
    fn parse_altitude_numeric() {
        assert_eq!(parse_altitude("1500 MSL"), Some(1500));
        assert_eq!(parse_altitude("3000 ft AGL"), Some(3000));
        assert_eq!(parse_altitude("5000FT"), Some(5000));
        assert_eq!(parse_altitude("18000"), Some(18000));
    }

    #[test]
    fn parse_altitude_invalid() {
        assert_eq!(parse_altitude(""), None);
        assert_eq!(parse_altitude("NOTAM"), None);
    }

    #[test]
    fn parse_openair_coord_decimal() {
        let result = parse_openair_coord("37.5000 -97.4333").unwrap();
        assert!((result.0 - 37.5).abs() < 0.001);
        assert!((result.1 - (-97.4333)).abs() < 0.001);
    }

    #[test]
    fn parse_openair_coord_dms_separate() {
        let result = parse_openair_coord("37:30:00 N 097:26:00 W").unwrap();
        assert!((result.0 - 37.5).abs() < 0.001);
        assert!((result.1 - (-97.4333)).abs() < 0.01);
    }

    #[test]
    fn parse_openair_coord_dms_embedded() {
        let result = parse_openair_coord("37:30:00N 097:26:00W").unwrap();
        assert!((result.0 - 37.5).abs() < 0.001);
        assert!((result.1 - (-97.4333)).abs() < 0.01);
    }

    #[test]
    fn parse_openair_coord_invalid() {
        assert!(parse_openair_coord("").is_none());
        assert!(parse_openair_coord("not coords").is_none());
    }

    #[test]
    fn parse_openair_basic_polygon() {
        let text = "\
AC C
AN WICHITA CLASS C
AH FL180
AL SFC
DP 37.500 -97.000
DP 37.500 -97.500
DP 37.750 -97.500
DP 37.750 -97.000
";
        let records = parse_openair(text);
        assert_eq!(records.len(), 1);

        if let CanonicalRecord::Airspace(ref asp) = records[0] {
            assert_eq!(asp.name, "WICHITA CLASS C");
            assert_eq!(asp.airspace_class, "C");
            assert_eq!(asp.airspace_type, "Class C");
            assert_eq!(asp.lower_limit_ft, Some(0));
            assert_eq!(asp.upper_limit_ft, Some(18000));
            assert_eq!(asp.polygon.len(), 4);
            assert!((asp.polygon[0].0 - 37.5).abs() < 0.001);
        } else {
            panic!("expected Airspace record");
        }
    }

    #[test]
    fn parse_openair_multiple_airspaces() {
        let text = "\
AC B
AN CITY CLASS B
AH FL180
AL SFC
DP 37.500 -97.000
DP 37.500 -97.500
DP 37.750 -97.000

AC D
AN SMALL FIELD
AH 3000 MSL
AL SFC
DP 37.600 -97.200
DP 37.600 -97.300
DP 37.650 -97.300
DP 37.650 -97.200
";
        let records = parse_openair(text);
        assert_eq!(records.len(), 2);

        if let CanonicalRecord::Airspace(ref asp) = records[0] {
            assert_eq!(asp.airspace_class, "B");
            assert_eq!(asp.polygon.len(), 3);
        } else {
            panic!("expected Airspace");
        }

        if let CanonicalRecord::Airspace(ref asp) = records[1] {
            assert_eq!(asp.airspace_class, "D");
            assert_eq!(asp.upper_limit_ft, Some(3000));
            assert_eq!(asp.polygon.len(), 4);
        } else {
            panic!("expected Airspace");
        }
    }

    #[test]
    fn parse_openair_circle() {
        let text = "\
AC D
AN SMALL FIELD
AH 3000 MSL
AL SFC
V X=37.600 -97.200
DC 5
";
        let records = parse_openair(text);
        assert_eq!(records.len(), 1);

        if let CanonicalRecord::Airspace(ref asp) = records[0] {
            assert_eq!(asp.polygon.len(), CIRCLE_SEGMENTS);
            // All points should be roughly 5 NM from center
            for (lat, lon) in &asp.polygon {
                let dlat = lat - 37.6;
                let dlon = (lon - (-97.2)) * (37.6_f64.to_radians()).cos();
                let dist_deg = (dlat * dlat + dlon * dlon).sqrt();
                let dist_nm = dist_deg * 60.0;
                assert!(
                    (dist_nm - 5.0).abs() < 0.5,
                    "point ({}, {}) is {} NM from center, expected ~5",
                    lat,
                    lon,
                    dist_nm
                );
            }
        } else {
            panic!("expected Airspace");
        }
    }

    #[test]
    fn parse_openair_comments_and_blanks() {
        let text = "\
* This is a comment
* Another comment

AC C
AN TEST AIRSPACE
AH FL180
AL SFC
DP 37.500 -97.000
DP 37.500 -97.500
DP 37.750 -97.000
";
        let records = parse_openair(text);
        assert_eq!(records.len(), 1);
    }

    #[test]
    fn parse_openair_restricted_types() {
        let text = "\
AC R
AN R-1234
AH FL250
AL SFC
DP 37.500 -97.000
DP 37.500 -97.500
DP 37.750 -97.000

AC P
AN P-56
AH 18000
AL SFC
DP 38.500 -77.000
DP 38.500 -77.100
DP 38.550 -77.100
";
        let records = parse_openair(text);
        assert_eq!(records.len(), 2);

        if let CanonicalRecord::Airspace(ref asp) = records[0] {
            assert_eq!(asp.airspace_class, "R");
            assert_eq!(asp.airspace_type, "Restricted");
        } else {
            panic!("expected Airspace");
        }

        if let CanonicalRecord::Airspace(ref asp) = records[1] {
            assert_eq!(asp.airspace_class, "P");
            assert_eq!(asp.airspace_type, "Prohibited");
        } else {
            panic!("expected Airspace");
        }
    }

    #[test]
    fn parse_openair_no_polygon_skipped() {
        let text = "\
AC C
AN EMPTY AIRSPACE
AH FL180
AL SFC
";
        let records = parse_openair(text);
        assert_eq!(records.len(), 0);
    }

    #[test]
    fn parse_openair_dms_coordinates() {
        let text = "\
AC C
AN DMS TEST
AH FL180
AL SFC
DP 37:30:00 N 097:26:00 W
DP 37:30:00 N 097:00:00 W
DP 37:45:00 N 097:00:00 W
";
        let records = parse_openair(text);
        assert_eq!(records.len(), 1);

        if let CanonicalRecord::Airspace(ref asp) = records[0] {
            assert_eq!(asp.polygon.len(), 3);
            assert!((asp.polygon[0].0 - 37.5).abs() < 0.001);
            assert!((asp.polygon[0].1 - (-97.4333)).abs() < 0.01);
        } else {
            panic!("expected Airspace");
        }
    }

    #[test]
    fn classify_airspace_types() {
        assert_eq!(classify_airspace_type("A"), "Class A");
        assert_eq!(classify_airspace_type("B"), "Class B");
        assert_eq!(classify_airspace_type("C"), "Class C");
        assert_eq!(classify_airspace_type("R"), "Restricted");
        assert_eq!(classify_airspace_type("P"), "Prohibited");
        assert_eq!(classify_airspace_type("CTR"), "Control Zone");
        assert_eq!(classify_airspace_type("TMA"), "Terminal Control Area");
        assert_eq!(classify_airspace_type("UNKNOWN"), "Other");
    }

    #[test]
    fn circle_to_polygon_count() {
        let points = circle_to_polygon((37.6, -97.2), 5.0);
        assert_eq!(points.len(), CIRCLE_SEGMENTS);
    }

    #[test]
    fn pipeline_stage_parses_openair() {
        let openair_data = b"\
AC C
AN TEST AIRSPACE
AH FL180
AL SFC
DP 37.500 -97.000
DP 37.500 -97.500
DP 37.750 -97.000
";
        let stage = OpenAirParseStage;
        let mut data = PipelineData {
            raw_bytes: Some(openair_data.to_vec()),
            records: Vec::new(),
            metadata: std::collections::HashMap::new(),
        };

        stage.execute(&mut data).unwrap();
        assert_eq!(data.records.len(), 1);
        assert!(matches!(data.records[0], CanonicalRecord::Airspace(_)));
    }

    #[test]
    fn pipeline_stage_no_bytes() {
        let stage = OpenAirParseStage;
        let mut data = PipelineData {
            raw_bytes: None,
            records: Vec::new(),
            metadata: std::collections::HashMap::new(),
        };

        let result = stage.execute(&mut data);
        assert!(result.is_err());
    }
}
