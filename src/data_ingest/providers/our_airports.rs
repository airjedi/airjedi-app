use chrono::Utc;
use serde::Deserialize;

use crate::data_ingest::canonical::{AirportInfo, CanonicalRecord, NavaidInfo, RunwayInfo};
use crate::data_ingest::pipeline::{PipelineData, PipelineError, PipelinePhase, PipelineStage};
use crate::data_ingest::provider::{
    DataProvider, FetchContext, ProviderCategory, ProviderError, ProviderMeta, RawFetchResult,
};

const OURAIRPORTS_BASE: &str = "https://davidmegginson.github.io/ourairports-data";

/// Fetch a file from OurAirports using HTTP conditional caching.
fn fetch_ourairports_file(filename: &str) -> Result<Vec<u8>, ProviderError> {
    let url = format!("{}/{}", OURAIRPORTS_BASE, filename);
    let cache_key = format!("ourairports_{}", filename);
    let result = crate::data_ingest::http_cache::fetch_with_cache(&url, &cache_key, 60)?;
    Ok(result.into_bytes())
}

// ---------------------------------------------------------------------------
// Airports
// ---------------------------------------------------------------------------

/// Provider that fetches airport data from OurAirports.
pub struct AirportsProvider;

impl DataProvider for AirportsProvider {
    fn name(&self) -> &str {
        "ourairports_airports"
    }

    fn schedule(&self) -> &str {
        "0 0 3 * * *"
    }

    fn fetch(&self, _ctx: &FetchContext) -> Result<RawFetchResult, ProviderError> {
        let data = fetch_ourairports_file("airports.csv")?;
        Ok(RawFetchResult {
            data,
            content_type: Some("text/csv".to_string()),
            source: format!("{}/airports.csv", OURAIRPORTS_BASE),
        })
    }

    fn pipeline_stages(&self) -> Vec<Box<dyn PipelineStage>> {
        vec![Box::new(AirportParseStage)]
    }

    fn metadata(&self) -> ProviderMeta {
        ProviderMeta {
            display_name: "Airports",
            category: ProviderCategory::Navigation,
            description: "Airport locations and details",
            config_key: "ourairports",
        }
    }
}

#[derive(Debug, Deserialize)]
struct AirportCsv {
    #[allow(dead_code)]
    id: Option<i64>,
    ident: Option<String>,
    #[serde(rename = "type")]
    airport_type: Option<String>,
    name: Option<String>,
    latitude_deg: Option<f64>,
    longitude_deg: Option<f64>,
    elevation_ft: Option<i32>,
    #[allow(dead_code)]
    continent: Option<String>,
    iso_country: Option<String>,
    iso_region: Option<String>,
    municipality: Option<String>,
    scheduled_service: Option<String>,
    #[allow(dead_code)]
    gps_code: Option<String>,
    iata_code: Option<String>,
    #[allow(dead_code)]
    local_code: Option<String>,
    #[allow(dead_code)]
    home_link: Option<String>,
    #[allow(dead_code)]
    wikipedia_link: Option<String>,
    #[allow(dead_code)]
    keywords: Option<String>,
}

struct AirportParseStage;

impl PipelineStage for AirportParseStage {
    fn name(&self) -> &str {
        "airport_parse"
    }

    fn phase(&self) -> PipelinePhase {
        PipelinePhase::Parse
    }

    fn execute(&self, data: &mut PipelineData) -> Result<(), PipelineError> {
        let raw = data.raw_bytes.take().unwrap_or_default();
        if raw.is_empty() {
            return Ok(());
        }

        let mut rdr = csv::ReaderBuilder::new()
            .has_headers(true)
            .flexible(true)
            .from_reader(raw.as_slice());

        let now = Utc::now();

        for result in rdr.deserialize::<AirportCsv>() {
            let row = result.map_err(|e| PipelineError::StageError {
                stage: self.name().to_string(),
                message: format!("CSV parse error: {}", e),
            })?;

            if let Some(record) = parse_airport_row(&row, now) {
                data.records.push(CanonicalRecord::Airport(record));
            }
        }

        Ok(())
    }
}

fn parse_airport_row(row: &AirportCsv, fetched_at: chrono::DateTime<Utc>) -> Option<AirportInfo> {
    let ident = row.ident.as_ref()?.clone();
    let latitude = row.latitude_deg?;
    let longitude = row.longitude_deg?;

    Some(AirportInfo {
        ident,
        name: row.name.clone().unwrap_or_default(),
        airport_type: row.airport_type.clone().unwrap_or_default(),
        latitude,
        longitude,
        elevation_ft: row.elevation_ft,
        iso_country: row.iso_country.clone().unwrap_or_default(),
        iso_region: row.iso_region.clone().unwrap_or_default(),
        municipality: row.municipality.clone(),
        scheduled_service: row.scheduled_service.as_deref() == Some("yes"),
        iata_code: row.iata_code.clone().filter(|s| !s.is_empty()),
        fetched_at,
    })
}

// ---------------------------------------------------------------------------
// Runways
// ---------------------------------------------------------------------------

/// Provider that fetches runway data from OurAirports.
pub struct RunwaysProvider;

impl DataProvider for RunwaysProvider {
    fn name(&self) -> &str {
        "ourairports_runways"
    }

    fn schedule(&self) -> &str {
        "0 0 3 * * *"
    }

    fn fetch(&self, _ctx: &FetchContext) -> Result<RawFetchResult, ProviderError> {
        let data = fetch_ourairports_file("runways.csv")?;
        Ok(RawFetchResult {
            data,
            content_type: Some("text/csv".to_string()),
            source: format!("{}/runways.csv", OURAIRPORTS_BASE),
        })
    }

    fn pipeline_stages(&self) -> Vec<Box<dyn PipelineStage>> {
        vec![Box::new(RunwayParseStage)]
    }

    fn metadata(&self) -> ProviderMeta {
        ProviderMeta {
            display_name: "Runways",
            category: ProviderCategory::Navigation,
            description: "Runway dimensions and surfaces",
            config_key: "ourairports",
        }
    }
}

#[derive(Debug, Deserialize)]
struct RunwayCsv {
    #[allow(dead_code)]
    id: Option<i64>,
    #[allow(dead_code)]
    airport_ref: Option<i64>,
    airport_ident: Option<String>,
    length_ft: Option<i32>,
    width_ft: Option<i32>,
    surface: Option<String>,
    lighted: Option<i32>,
    closed: Option<i32>,
    le_ident: Option<String>,
    le_latitude_deg: Option<f64>,
    le_longitude_deg: Option<f64>,
    #[allow(dead_code)]
    le_elevation_ft: Option<i32>,
    #[serde(rename = "le_heading_degT")]
    le_heading_deg_t: Option<f64>,
    #[allow(dead_code)]
    le_displaced_threshold_ft: Option<i32>,
    he_ident: Option<String>,
    he_latitude_deg: Option<f64>,
    he_longitude_deg: Option<f64>,
    #[allow(dead_code)]
    he_elevation_ft: Option<i32>,
    #[serde(rename = "he_heading_degT")]
    he_heading_deg_t: Option<f64>,
    #[allow(dead_code)]
    he_displaced_threshold_ft: Option<i32>,
}

struct RunwayParseStage;

impl PipelineStage for RunwayParseStage {
    fn name(&self) -> &str {
        "runway_parse"
    }

    fn phase(&self) -> PipelinePhase {
        PipelinePhase::Parse
    }

    fn execute(&self, data: &mut PipelineData) -> Result<(), PipelineError> {
        let raw = data.raw_bytes.take().unwrap_or_default();
        if raw.is_empty() {
            return Ok(());
        }

        let mut rdr = csv::ReaderBuilder::new()
            .has_headers(true)
            .flexible(true)
            .from_reader(raw.as_slice());

        let now = Utc::now();

        for result in rdr.deserialize::<RunwayCsv>() {
            let row = result.map_err(|e| PipelineError::StageError {
                stage: self.name().to_string(),
                message: format!("CSV parse error: {}", e),
            })?;

            if let Some(record) = parse_runway_row(&row, now) {
                data.records.push(CanonicalRecord::Runway(record));
            }
        }

        Ok(())
    }
}

fn parse_runway_row(row: &RunwayCsv, fetched_at: chrono::DateTime<Utc>) -> Option<RunwayInfo> {
    let airport_ident = row.airport_ident.as_ref()?.clone();

    Some(RunwayInfo {
        airport_ident,
        length_ft: row.length_ft,
        width_ft: row.width_ft,
        surface: row.surface.clone(),
        lighted: row.lighted == Some(1),
        closed: row.closed == Some(1),
        le_ident: row.le_ident.clone().unwrap_or_default(),
        le_latitude: row.le_latitude_deg,
        le_longitude: row.le_longitude_deg,
        le_heading_deg: row.le_heading_deg_t.map(|v| v as f32),
        he_ident: row.he_ident.clone().unwrap_or_default(),
        he_latitude: row.he_latitude_deg,
        he_longitude: row.he_longitude_deg,
        he_heading_deg: row.he_heading_deg_t.map(|v| v as f32),
        fetched_at,
    })
}

// ---------------------------------------------------------------------------
// Navaids
// ---------------------------------------------------------------------------

/// Provider that fetches navaid data from OurAirports.
pub struct NavaidsProvider;

impl DataProvider for NavaidsProvider {
    fn name(&self) -> &str {
        "ourairports_navaids"
    }

    fn schedule(&self) -> &str {
        "0 0 3 * * *"
    }

    fn fetch(&self, _ctx: &FetchContext) -> Result<RawFetchResult, ProviderError> {
        let data = fetch_ourairports_file("navaids.csv")?;
        Ok(RawFetchResult {
            data,
            content_type: Some("text/csv".to_string()),
            source: format!("{}/navaids.csv", OURAIRPORTS_BASE),
        })
    }

    fn pipeline_stages(&self) -> Vec<Box<dyn PipelineStage>> {
        vec![Box::new(NavaidParseStage)]
    }

    fn metadata(&self) -> ProviderMeta {
        ProviderMeta {
            display_name: "Navaids",
            category: ProviderCategory::Navigation,
            description: "VOR, NDB, and other navigation aids",
            config_key: "ourairports",
        }
    }
}

#[derive(Debug, Deserialize)]
struct NavaidCsv {
    #[allow(dead_code)]
    id: Option<i64>,
    #[allow(dead_code)]
    filename: Option<String>,
    ident: Option<String>,
    name: Option<String>,
    #[serde(rename = "type")]
    navaid_type: Option<String>,
    frequency_khz: Option<u32>,
    latitude_deg: Option<f64>,
    longitude_deg: Option<f64>,
    elevation_ft: Option<i32>,
    #[allow(dead_code)]
    iso_country: Option<String>,
    #[allow(dead_code)]
    dme_frequency_khz: Option<i32>,
    #[allow(dead_code)]
    dme_channel: Option<String>,
    #[allow(dead_code)]
    dme_latitude_deg: Option<f64>,
    #[allow(dead_code)]
    dme_longitude_deg: Option<f64>,
    #[allow(dead_code)]
    dme_elevation_ft: Option<i32>,
    #[allow(dead_code)]
    slaved_variation_deg: Option<f64>,
    #[allow(dead_code)]
    magnetic_variation_deg: Option<f64>,
    #[allow(dead_code)]
    #[serde(rename = "usageType")]
    usage_type: Option<String>,
    #[allow(dead_code)]
    power: Option<String>,
    associated_airport: Option<String>,
}

struct NavaidParseStage;

impl PipelineStage for NavaidParseStage {
    fn name(&self) -> &str {
        "navaid_parse"
    }

    fn phase(&self) -> PipelinePhase {
        PipelinePhase::Parse
    }

    fn execute(&self, data: &mut PipelineData) -> Result<(), PipelineError> {
        let raw = data.raw_bytes.take().unwrap_or_default();
        if raw.is_empty() {
            return Ok(());
        }

        let mut rdr = csv::ReaderBuilder::new()
            .has_headers(true)
            .flexible(true)
            .from_reader(raw.as_slice());

        let now = Utc::now();

        for result in rdr.deserialize::<NavaidCsv>() {
            let row = result.map_err(|e| PipelineError::StageError {
                stage: self.name().to_string(),
                message: format!("CSV parse error: {}", e),
            })?;

            if let Some(record) = parse_navaid_row(&row, now) {
                data.records.push(CanonicalRecord::Navaid(record));
            }
        }

        Ok(())
    }
}

fn parse_navaid_row(row: &NavaidCsv, fetched_at: chrono::DateTime<Utc>) -> Option<NavaidInfo> {
    let ident = row.ident.as_ref()?.clone();
    let latitude = row.latitude_deg?;
    let longitude = row.longitude_deg?;

    Some(NavaidInfo {
        ident,
        name: row.name.clone().unwrap_or_default(),
        navaid_type: row.navaid_type.clone().unwrap_or_default(),
        latitude,
        longitude,
        elevation_ft: row.elevation_ft,
        frequency_khz: row.frequency_khz,
        associated_airport: row.associated_airport.clone().filter(|s| !s.is_empty()),
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

    // -- Airport tests --

    fn sample_airports_csv() -> Vec<u8> {
        b"id,ident,type,name,latitude_deg,longitude_deg,elevation_ft,continent,iso_country,iso_region,municipality,scheduled_service,gps_code,iata_code,local_code,home_link,wikipedia_link,keywords\n\
        3580,KICT,large_airport,\"Wichita Dwight D Eisenhower National Airport\",37.6499,-97.4331,1333,NA,US,US-KS,Wichita,yes,KICT,ICT,ICT,,,\n\
        3448,KJFK,large_airport,\"John F Kennedy International Airport\",40.6399,-73.7787,13,NA,US,US-NY,New York,yes,KJFK,JFK,JFK,,,\n\
        26430,5KS9,small_airport,\"Selby Aerodrome\",37.8,-97.5,1450,NA,US,US-KS,Wichita,no,,,5KS9,,,\n"
            .to_vec()
    }

    #[test]
    fn test_parse_airports_csv() {
        let data = PipelineData {
            raw_bytes: Some(sample_airports_csv()),
            ..Default::default()
        };

        let stages: Vec<Box<dyn PipelineStage>> = vec![Box::new(AirportParseStage)];
        let result = run_pipeline(&stages, data).unwrap();

        assert_eq!(result.records.len(), 3);

        if let CanonicalRecord::Airport(a) = &result.records[0] {
            assert_eq!(a.ident, "KICT");
            assert_eq!(a.airport_type, "large_airport");
            assert!(a.name.contains("Eisenhower"));
            assert!((a.latitude - 37.6499).abs() < 0.001);
            assert!((a.longitude - (-97.4331)).abs() < 0.001);
            assert_eq!(a.elevation_ft, Some(1333));
            assert_eq!(a.iso_country, "US");
            assert_eq!(a.iso_region, "US-KS");
            assert_eq!(a.municipality, Some("Wichita".to_string()));
            assert!(a.scheduled_service);
            assert_eq!(a.iata_code, Some("ICT".to_string()));
        } else {
            panic!("expected Airport record");
        }

        if let CanonicalRecord::Airport(a) = &result.records[2] {
            assert_eq!(a.ident, "5KS9");
            assert_eq!(a.airport_type, "small_airport");
            assert!(!a.scheduled_service);
            assert_eq!(a.iata_code, None);
        } else {
            panic!("expected Airport record");
        }
    }

    #[test]
    fn test_airports_empty_csv() {
        let data = PipelineData {
            raw_bytes: Some(b"id,ident,type,name,latitude_deg,longitude_deg,elevation_ft,continent,iso_country,iso_region,municipality,scheduled_service,gps_code,iata_code,local_code,home_link,wikipedia_link,keywords\n".to_vec()),
            ..Default::default()
        };

        let stages: Vec<Box<dyn PipelineStage>> = vec![Box::new(AirportParseStage)];
        let result = run_pipeline(&stages, data).unwrap();
        assert!(result.records.is_empty());
    }

    #[test]
    fn test_airports_provider_metadata() {
        let provider = AirportsProvider;
        assert_eq!(provider.name(), "ourairports_airports");
        assert_eq!(provider.schedule(), "0 0 3 * * *");
    }

    // -- Runway tests --

    fn sample_runways_csv() -> Vec<u8> {
        b"id,airport_ref,airport_ident,length_ft,width_ft,surface,lighted,closed,le_ident,le_latitude_deg,le_longitude_deg,le_elevation_ft,le_heading_degT,le_displaced_threshold_ft,he_ident,he_latitude_deg,he_longitude_deg,he_elevation_ft,he_heading_degT,he_displaced_threshold_ft\n\
        234550,3580,KICT,10301,150,CON,1,0,01L,37.6399,-97.4431,1329,12.3,,19R,37.6612,-97.4391,1321,192.3,\n\
        234551,3580,KICT,7302,150,ASP,1,0,14,37.6520,-97.4210,1333,144.0,,32,37.6430,-97.4450,1330,324.0,\n"
            .to_vec()
    }

    #[test]
    fn test_parse_runways_csv() {
        let data = PipelineData {
            raw_bytes: Some(sample_runways_csv()),
            ..Default::default()
        };

        let stages: Vec<Box<dyn PipelineStage>> = vec![Box::new(RunwayParseStage)];
        let result = run_pipeline(&stages, data).unwrap();

        assert_eq!(result.records.len(), 2);

        if let CanonicalRecord::Runway(r) = &result.records[0] {
            assert_eq!(r.airport_ident, "KICT");
            assert_eq!(r.length_ft, Some(10301));
            assert_eq!(r.width_ft, Some(150));
            assert_eq!(r.surface, Some("CON".to_string()));
            assert!(r.lighted);
            assert!(!r.closed);
            assert_eq!(r.le_ident, "01L");
            assert!((r.le_heading_deg.unwrap() - 12.3).abs() < 0.1);
            assert_eq!(r.he_ident, "19R");
        } else {
            panic!("expected Runway record");
        }
    }

    #[test]
    fn test_runways_provider_metadata() {
        let provider = RunwaysProvider;
        assert_eq!(provider.name(), "ourairports_runways");
        assert_eq!(provider.schedule(), "0 0 3 * * *");
    }

    // -- Navaid tests --

    fn sample_navaids_csv() -> Vec<u8> {
        b"id,filename,ident,name,type,frequency_khz,latitude_deg,longitude_deg,elevation_ft,iso_country,dme_frequency_khz,dme_channel,dme_latitude_deg,dme_longitude_deg,dme_elevation_ft,slaved_variation_deg,magnetic_variation_deg,usageType,power,associated_airport\n\
        85727,,ICT,WICHITA,VORTAC,113900,37.6499,-97.4331,1340,US,113900,086X,37.6499,-97.4331,1340,,,BOTH,HIGH,KICT\n\
        85729,,MCI,KANSAS CITY,VOR-DME,113600,39.1200,-94.5800,1050,US,113600,083X,39.1200,-94.5800,1050,,,BOTH,HIGH,\n"
            .to_vec()
    }

    #[test]
    fn test_parse_navaids_csv() {
        let data = PipelineData {
            raw_bytes: Some(sample_navaids_csv()),
            ..Default::default()
        };

        let stages: Vec<Box<dyn PipelineStage>> = vec![Box::new(NavaidParseStage)];
        let result = run_pipeline(&stages, data).unwrap();

        assert_eq!(result.records.len(), 2);

        if let CanonicalRecord::Navaid(n) = &result.records[0] {
            assert_eq!(n.ident, "ICT");
            assert_eq!(n.name, "WICHITA");
            assert_eq!(n.navaid_type, "VORTAC");
            assert_eq!(n.frequency_khz, Some(113900));
            assert!((n.latitude - 37.6499).abs() < 0.001);
            assert_eq!(n.elevation_ft, Some(1340));
            assert_eq!(n.associated_airport, Some("KICT".to_string()));
        } else {
            panic!("expected Navaid record");
        }

        if let CanonicalRecord::Navaid(n) = &result.records[1] {
            assert_eq!(n.ident, "MCI");
            assert_eq!(n.navaid_type, "VOR-DME");
            assert_eq!(n.associated_airport, None); // empty string filtered to None
        } else {
            panic!("expected Navaid record");
        }
    }

    #[test]
    fn test_navaids_provider_metadata() {
        let provider = NavaidsProvider;
        assert_eq!(provider.name(), "ourairports_navaids");
        assert_eq!(provider.schedule(), "0 0 3 * * *");
    }
}
