use std::io::Read;

use bevy::prelude::*;
use chrono::Utc;

use crate::data_ingest::canonical::{AirwayInfo, CanonicalRecord, FrequencyInfo};
use crate::data_ingest::pipeline::{PipelineData, PipelineError, PipelinePhase, PipelineStage};
use crate::data_ingest::provider::{
    DataProvider, FetchContext, ProviderCategory, ProviderError, ProviderMeta, RawFetchResult,
};

/// FAA NASR subscription index page (scraped for current effective date).
const NASR_INDEX_URL: &str =
    "https://www.faa.gov/air_traffic/flight_info/aeronav/aero_data/NASR_Subscription/";

/// FAA NASR 28-day subscription download URL template.
/// The date is substituted from the current effective date scraped from NASR_INDEX_URL.
const NASR_DOWNLOAD_TEMPLATE: &str =
    "https://nfdc.faa.gov/webContent/28DaySub/28DaySubscription_Effective_{date}.zip";

/// Provider for FAA National Airspace System Resources (NASR) data.
/// Fetches the 28-day subscription ZIP and parses airways and
/// frequencies into canonical records.
pub struct FaaNasrProvider;

impl FaaNasrProvider {
    pub fn new() -> Self {
        Self
    }

    /// Scrape the NASR subscription index page to find the current effective date,
    /// then construct the download URL.
    fn resolve_download_url(&self) -> Result<String, ProviderError> {
        let client = reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .map_err(|e| ProviderError::Network(format!("HTTP client error: {}", e)))?;

        let html = client
            .get(NASR_INDEX_URL)
            .send()
            .map_err(|e| ProviderError::Network(format!("failed to fetch NASR index: {}", e)))?
            .text()
            .map_err(|e| ProviderError::Network(format!("failed to read NASR index: {}", e)))?;

        // Scan for all YYYY-MM-DD date patterns and pick the most recent
        // one that is not in the future.
        let today = chrono::Utc::now().date_naive();
        let mut best_date: Option<chrono::NaiveDate> = None;
        let bytes = html.as_bytes();

        for i in 0..bytes.len().saturating_sub(9) {
            // Look for "20XX-MM-DD" pattern
            if bytes[i] == b'2'
                && bytes[i + 1] == b'0'
                && bytes[i + 4] == b'-'
                && bytes[i + 7] == b'-'
            {
                let candidate = &html[i..i + 10];
                if let Ok(date) = chrono::NaiveDate::parse_from_str(candidate, "%Y-%m-%d") {
                    if date <= today && (best_date.is_none() || date > best_date.unwrap()) {
                        best_date = Some(date);
                    }
                }
            }
        }

        let date = best_date.ok_or_else(|| {
            ProviderError::Parse("could not find current NASR effective date".to_string())
        })?;

        let url = NASR_DOWNLOAD_TEMPLATE.replace("{date}", &date.format("%Y-%m-%d").to_string());
        Ok(url)
    }

    /// Download the NASR ZIP using HTTP conditional caching.
    /// The cache key includes the edition date so a new edition triggers a fresh download.
    fn fetch_zip(&self) -> Result<Vec<u8>, ProviderError> {
        let url = self.resolve_download_url()?;
        // Extract date from URL for cache key (e.g. "nasr_2026-02-19.zip")
        let date = url.rsplit("Effective_").next().unwrap_or("current.zip");
        let cache_key = format!("nasr_{}", date);
        let result = crate::data_ingest::http_cache::fetch_with_cache(&url, &cache_key, 300)?;
        Ok(result.into_bytes())
    }
}

impl DataProvider for FaaNasrProvider {
    fn name(&self) -> &str {
        "faa_nasr"
    }

    /// Run once per day at 06:00 UTC (NASR data updates every 28 days).
    fn schedule(&self) -> &str {
        "0 0 6 * * *"
    }

    fn fetch(&self, _ctx: &FetchContext) -> Result<RawFetchResult, ProviderError> {
        let data = self.fetch_zip()?;
        Ok(RawFetchResult {
            data,
            content_type: Some("application/zip".to_string()),
            source: "faa_nasr_subscription".to_string(),
        })
    }

    fn pipeline_stages(&self) -> Vec<Box<dyn PipelineStage>> {
        vec![Box::new(NasrParseStage)]
    }

    fn metadata(&self) -> ProviderMeta {
        ProviderMeta {
            display_name: "FAA Airways/Freqs",
            category: ProviderCategory::Navigation,
            description: "Airways and communication frequencies from FAA NASR",
            config_key: "faa_nasr",
        }
    }
}

/// Pipeline stage that extracts airway and frequency data from the NASR ZIP.
///
/// The outer ZIP contains `CSV_Data/<date>_CSV.zip` with CSV files inside.
/// We parse `AWY_SEG_ALT.csv` for airway segment data and `FRQ.csv` for
/// airport/facility frequencies.
struct NasrParseStage;

impl PipelineStage for NasrParseStage {
    fn name(&self) -> &str {
        "nasr_parse"
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

        let cursor = std::io::Cursor::new(raw);
        let mut outer = zip::ZipArchive::new(cursor).map_err(|e| PipelineError::StageError {
            stage: self.name().to_string(),
            message: format!("invalid outer ZIP: {}", e),
        })?;

        // Find and extract the inner CSV ZIP from CSV_Data/
        let inner_bytes =
            extract_inner_csv_zip(&mut outer).ok_or_else(|| PipelineError::StageError {
                stage: self.name().to_string(),
                message: "CSV_Data/*.zip not found in NASR archive".to_string(),
            })?;

        let inner_cursor = std::io::Cursor::new(inner_bytes);
        let mut inner =
            zip::ZipArchive::new(inner_cursor).map_err(|e| PipelineError::StageError {
                stage: self.name().to_string(),
                message: format!("invalid inner CSV ZIP: {}", e),
            })?;

        let now = Utc::now();

        // Parse airway segments
        if let Some(content) = read_csv_file(&mut inner, "AWY_SEG_ALT.csv") {
            let airways = parse_awy_seg_csv(&content, now);
            info!(
                "FAA NASR: parsed {} airway segment records from CSV",
                airways.len()
            );
            data.records.extend(airways);
        }

        // Parse frequencies
        if let Some(content) = read_csv_file(&mut inner, "FRQ.csv") {
            let freqs = parse_frq_csv(&content, now);
            info!(
                "FAA NASR: parsed {} frequency records from CSV",
                freqs.len()
            );
            data.records.extend(freqs);
        }

        data.metadata
            .insert("source".to_string(), "faa_nasr".to_string());
        Ok(())
    }
}

/// Find the CSV ZIP inside `CSV_Data/` in the outer archive and return its bytes.
fn extract_inner_csv_zip(
    archive: &mut zip::ZipArchive<std::io::Cursor<Vec<u8>>>,
) -> Option<Vec<u8>> {
    for i in 0..archive.len() {
        let mut file = archive.by_index(i).ok()?;
        let name = file.name().to_string();
        if name.starts_with("CSV_Data/") && name.ends_with(".zip") && !name.contains("CHG") {
            let mut buf = Vec::new();
            file.read_to_end(&mut buf).ok()?;
            return Some(buf);
        }
    }
    None
}

/// Read a specific CSV file from the inner ZIP archive.
fn read_csv_file(
    archive: &mut zip::ZipArchive<std::io::Cursor<Vec<u8>>>,
    filename: &str,
) -> Option<String> {
    let mut file = archive.by_name(filename).ok()?;
    let mut buf = Vec::new();
    file.read_to_end(&mut buf).ok()?;
    Some(String::from_utf8_lossy(&buf).into_owned())
}

/// Parse AWY_SEG_ALT.csv for airway segment records.
/// Key fields: AWY_ID, POINT_SEQ, FROM_POINT, MIN_ENROUTE_ALT, MAX_AUTH_ALT
fn parse_awy_seg_csv(content: &str, fetched_at: chrono::DateTime<Utc>) -> Vec<CanonicalRecord> {
    let mut records = Vec::new();
    let mut reader = csv::ReaderBuilder::new()
        .has_headers(true)
        .flexible(true)
        .from_reader(content.as_bytes());

    let headers = match reader.headers() {
        Ok(h) => h.clone(),
        Err(_) => return records,
    };

    let col = |name: &str| headers.iter().position(|h| h == name);
    let i_awy_id = match col("AWY_ID") {
        Some(i) => i,
        None => return records,
    };
    let i_seq = col("POINT_SEQ");
    let i_from = col("FROM_POINT");
    let i_mea = col("MIN_ENROUTE_ALT");
    let i_maa = col("MAX_AUTH_ALT");

    for result in reader.records() {
        let row = match result {
            Ok(r) => r,
            Err(_) => continue,
        };

        let designator = row.get(i_awy_id).unwrap_or("").trim().to_string();
        if designator.is_empty() {
            continue;
        }

        let sequence: u32 = i_seq
            .and_then(|i| row.get(i))
            .and_then(|s| s.trim().parse().ok())
            .unwrap_or(0);

        let fix_ident = i_from
            .map(|i| row.get(i).unwrap_or("").trim().to_string())
            .unwrap_or_default();

        let min_alt: Option<i32> = i_mea
            .and_then(|i| row.get(i))
            .and_then(|s| s.trim().parse().ok());

        let max_alt: Option<i32> = i_maa
            .and_then(|i| row.get(i))
            .and_then(|s| s.trim().parse().ok());

        let airway_type = if designator.starts_with('J') {
            "Jet"
        } else if designator.starts_with('V') {
            "Victor"
        } else if designator.starts_with('T') {
            "RNAV"
        } else if designator.starts_with('Q') {
            "Q-Route"
        } else {
            "Other"
        }
        .to_string();

        records.push(CanonicalRecord::Airway(AirwayInfo {
            designator,
            airway_type,
            sequence,
            fix_ident,
            fix_latitude: 0.0, // Coordinates require FIX_BASE.csv join
            fix_longitude: 0.0,
            min_altitude_ft: min_alt,
            max_altitude_ft: max_alt,
            fetched_at,
        }));
    }

    records
}

/// Parse FRQ.csv for airport/facility frequency records.
/// Key fields: FACILITY, FAC_NAME, FREQ, FREQ_USE, LAT_DECIMAL, LONG_DECIMAL
fn parse_frq_csv(content: &str, fetched_at: chrono::DateTime<Utc>) -> Vec<CanonicalRecord> {
    let mut records = Vec::new();
    let mut reader = csv::ReaderBuilder::new()
        .has_headers(true)
        .flexible(true)
        .from_reader(content.as_bytes());

    let headers = match reader.headers() {
        Ok(h) => h.clone(),
        Err(_) => return records,
    };

    let col = |name: &str| headers.iter().position(|h| h == name);
    let i_facility = match col("FACILITY") {
        Some(i) => i,
        None => return records,
    };
    let i_freq = match col("FREQ") {
        Some(i) => i,
        None => return records,
    };
    let i_use = col("FREQ_USE");
    let i_fac_name = col("FAC_NAME");

    for result in reader.records() {
        let row = match result {
            Ok(r) => r,
            Err(_) => continue,
        };

        let airport_ident = row.get(i_facility).unwrap_or("").trim().to_string();
        if airport_ident.is_empty() {
            continue;
        }

        let freq_str = row.get(i_freq).unwrap_or("").trim();
        let freq_mhz: f64 = match freq_str.parse() {
            Ok(f) => f,
            Err(_) => continue,
        };

        // Filter to valid aviation VHF/UHF ranges
        if !((108.0..=137.0).contains(&freq_mhz) || (225.0..=400.0).contains(&freq_mhz)) {
            continue;
        }

        let frequency_type = i_use
            .map(|i| row.get(i).unwrap_or("").trim().to_string())
            .unwrap_or_default();

        let description = i_fac_name
            .map(|i| row.get(i).unwrap_or("").trim().to_string())
            .unwrap_or_else(|| frequency_type.clone());

        records.push(CanonicalRecord::Frequency(FrequencyInfo {
            airport_ident,
            frequency_type,
            description,
            frequency_mhz: freq_mhz,
            fetched_at,
        }));
    }

    records
}

// ---------------------------------------------------------------------------
// AWY (Airway) parsing
// ---------------------------------------------------------------------------

/// Parse NASR AWY records from pipe-delimited text.
///
/// The AWY file contains two record types:
/// - AWY1: airway header (designator, type, effective date)
/// - AWY2: fix/point records with coordinates and altitudes
///
/// We only extract AWY2 records since they contain the actual fix data.
/// Fields are pipe-delimited. Key AWY2 fields (0-indexed):
///   0: record type ("AWY2")
///   1: airway designator (e.g. "V16", "J60")
///   2: sequence number
///   4: fix identifier
///   Fields containing lat/lon in DMS: varies by format version
///
/// The NASR format can vary between releases, so we parse defensively.
// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// Create a nested ZIP matching NASR structure:
    /// outer.zip -> CSV_Data/test_CSV.zip -> AWY_SEG_ALT.csv, FRQ.csv
    fn create_test_nasr_zip(awy_csv: &str, frq_csv: &str) -> Vec<u8> {
        // Build inner CSV ZIP
        let mut inner_buf = std::io::Cursor::new(Vec::new());
        {
            let mut zip = zip::ZipWriter::new(&mut inner_buf);
            let opts = zip::write::SimpleFileOptions::default();

            zip.start_file("AWY_SEG_ALT.csv", opts).unwrap();
            zip.write_all(awy_csv.as_bytes()).unwrap();

            zip.start_file("FRQ.csv", opts).unwrap();
            zip.write_all(frq_csv.as_bytes()).unwrap();

            zip.finish().unwrap();
        }

        // Build outer ZIP containing CSV_Data/<date>_CSV.zip
        let mut outer_buf = std::io::Cursor::new(Vec::new());
        {
            let mut zip = zip::ZipWriter::new(&mut outer_buf);
            let opts = zip::write::SimpleFileOptions::default();

            zip.start_file("CSV_Data/01_Jan_2026_CSV.zip", opts)
                .unwrap();
            zip.write_all(&inner_buf.into_inner()).unwrap();

            zip.finish().unwrap();
        }

        outer_buf.into_inner()
    }

    #[test]
    fn parse_awy_seg_csv_basic() {
        let csv = "EFF_DATE,REGULATORY,AWY_LOCATION,AWY_ID,POINT_SEQ,FROM_POINT,FROM_PT_TYPE,NAV_NAME,NAV_CITY,ARTCC,ICAO_REGION_CODE,STATE_CODE,COUNTRY_CODE,TO_POINT,MAG_COURSE,OPP_MAG_COURSE,MAG_COURSE_DIST,CHGOVR_PT,CHGOVR_PT_NAME,CHGOVR_PT_DIST,AWY_SEG_GAP_FLAG,SIGNAL_GAP_FLAG,DOGLEG,NEXT_MEA_PT,MIN_ENROUTE_ALT,MIN_ENROUTE_ALT_DIR,MIN_ENROUTE_ALT_OPPOSITE,MIN_ENROUTE_ALT_OPPOSITE_DIR,GPS_MIN_ENROUTE_ALT,GPS_MIN_ENROUTE_ALT_DIR,GPS_MIN_ENROUTE_ALT_OPPOSITE,GPS_MEA_OPPOSITE_DIR,DD_IRU_MEA,DD_IRU_MEA_DIR,DD_I_MEA_OPPOSITE,DD_I_MEA_OPPOSITE_DIR,MIN_OBSTN_CLNC_ALT,MIN_CROSS_ALT,MIN_CROSS_ALT_DIR,MIN_CROSS_ALT_NAV_PT,MIN_CROSS_ALT_OPPOSITE,MIN_CROSS_ALT_OPPOSITE_DIR,MIN_RECEP_ALT,MAX_AUTH_ALT,MEA_GAP,REQD_NAV_PERFORMANCE,REMARK\n\
2026/01/22,N,C,V16,10,WHING,WP,,,ZKC,K6,KS,US,ICT,180,360,15,,,,N,N,N,ICT,4000,,,,,,,,,,,,,,,,,,,,60000,,\n\
2026/01/22,N,C,J60,10,TUL,VOR,,,ZKC,K6,OK,US,ICT,180,360,30,,,,N,N,N,ICT,18000,,,,,,,,,,,,,,,,,,,,45000,,\n";

        let records = parse_awy_seg_csv(csv, Utc::now());
        assert_eq!(records.len(), 2);

        if let CanonicalRecord::Airway(ref awy) = records[0] {
            assert_eq!(awy.designator, "V16");
            assert_eq!(awy.airway_type, "Victor");
            assert_eq!(awy.sequence, 10);
            assert_eq!(awy.fix_ident, "WHING");
            assert_eq!(awy.min_altitude_ft, Some(4000));
        } else {
            panic!("expected Airway record");
        }

        if let CanonicalRecord::Airway(ref awy) = records[1] {
            assert_eq!(awy.designator, "J60");
            assert_eq!(awy.airway_type, "Jet");
        } else {
            panic!("expected Airway record");
        }
    }

    #[test]
    fn parse_frq_csv_basic() {
        let csv = "EFF_DATE,FACILITY,FAC_NAME,FACILITY_TYPE,ARTCC_OR_FSS_ID,CPDLC,TOWER_HRS,SERVICED_FACILITY,SERVICED_FAC_NAME,SERVICED_SITE_TYPE,LAT_DECIMAL,LONG_DECIMAL,SERVICED_CITY,SERVICED_STATE,SERVICED_COUNTRY,TOWER_OR_COMM_CALL,PRIMARY_APPROACH_RADIO_CALL,FREQ,SECTORIZATION,FREQ_USE,REMARK\n\
2026/02/19,ICT,WICHITA TOWER,ATCT,,,,ICT,WICHITA TOWER,AIRPORT,37.649,-97.433,WICHITA,KS,US,WICHITA TOWER,,118.7,,TWR,\n\
2026/02/19,ICT,WICHITA TOWER,ATCT,,,,ICT,WICHITA TOWER,AIRPORT,37.649,-97.433,WICHITA,KS,US,WICHITA GROUND,,121.9,,GND,\n\
2026/02/19,ICT,WICHITA TOWER,ATCT,,,,ICT,WICHITA TOWER,AIRPORT,37.649,-97.433,WICHITA,KS,US,,,122.9,,CTAF,\n";

        let records = parse_frq_csv(csv, Utc::now());
        assert_eq!(records.len(), 3);

        if let CanonicalRecord::Frequency(ref freq) = records[0] {
            assert_eq!(freq.airport_ident, "ICT");
            assert_eq!(freq.frequency_type, "TWR");
            assert!((freq.frequency_mhz - 118.7).abs() < 0.001);
            assert_eq!(freq.description, "WICHITA TOWER");
        } else {
            panic!("expected Frequency record");
        }
    }

    #[test]
    fn parse_frq_csv_filters_invalid_frequencies() {
        let csv = "EFF_DATE,FACILITY,FAC_NAME,FACILITY_TYPE,ARTCC_OR_FSS_ID,CPDLC,TOWER_HRS,SERVICED_FACILITY,SERVICED_FAC_NAME,SERVICED_SITE_TYPE,LAT_DECIMAL,LONG_DECIMAL,SERVICED_CITY,SERVICED_STATE,SERVICED_COUNTRY,TOWER_OR_COMM_CALL,PRIMARY_APPROACH_RADIO_CALL,FREQ,SECTORIZATION,FREQ_USE,REMARK\n\
2026/02/19,ICT,TEST,ATCT,,,,ICT,TEST,AIRPORT,37.649,-97.433,WICHITA,KS,US,,,50.0,,BAD,\n\
2026/02/19,ICT,TEST,ATCT,,,,ICT,TEST,AIRPORT,37.649,-97.433,WICHITA,KS,US,,,not_a_freq,,BAD,\n";

        let records = parse_frq_csv(csv, Utc::now());
        assert_eq!(records.len(), 0);
    }

    #[test]
    fn pipeline_stage_parses_nested_zip() {
        let awy_csv = "EFF_DATE,REGULATORY,AWY_LOCATION,AWY_ID,POINT_SEQ,FROM_POINT,FROM_PT_TYPE,NAV_NAME,NAV_CITY,ARTCC,ICAO_REGION_CODE,STATE_CODE,COUNTRY_CODE,TO_POINT,MAG_COURSE,OPP_MAG_COURSE,MAG_COURSE_DIST,CHGOVR_PT,CHGOVR_PT_NAME,CHGOVR_PT_DIST,AWY_SEG_GAP_FLAG,SIGNAL_GAP_FLAG,DOGLEG,NEXT_MEA_PT,MIN_ENROUTE_ALT,MIN_ENROUTE_ALT_DIR,MIN_ENROUTE_ALT_OPPOSITE,MIN_ENROUTE_ALT_OPPOSITE_DIR,GPS_MIN_ENROUTE_ALT,GPS_MIN_ENROUTE_ALT_DIR,GPS_MIN_ENROUTE_ALT_OPPOSITE,GPS_MEA_OPPOSITE_DIR,DD_IRU_MEA,DD_IRU_MEA_DIR,DD_I_MEA_OPPOSITE,DD_I_MEA_OPPOSITE_DIR,MIN_OBSTN_CLNC_ALT,MIN_CROSS_ALT,MIN_CROSS_ALT_DIR,MIN_CROSS_ALT_NAV_PT,MIN_CROSS_ALT_OPPOSITE,MIN_CROSS_ALT_OPPOSITE_DIR,MIN_RECEP_ALT,MAX_AUTH_ALT,MEA_GAP,REQD_NAV_PERFORMANCE,REMARK\n\
2026/01/22,N,C,V16,10,WHING,WP,,,ZKC,K6,KS,US,ICT,180,360,15,,,,N,N,N,ICT,4000,,,,,,,,,,,,,,,,,,,,,60000,,\n";

        let frq_csv = "EFF_DATE,FACILITY,FAC_NAME,FACILITY_TYPE,ARTCC_OR_FSS_ID,CPDLC,TOWER_HRS,SERVICED_FACILITY,SERVICED_FAC_NAME,SERVICED_SITE_TYPE,LAT_DECIMAL,LONG_DECIMAL,SERVICED_CITY,SERVICED_STATE,SERVICED_COUNTRY,TOWER_OR_COMM_CALL,PRIMARY_APPROACH_RADIO_CALL,FREQ,SECTORIZATION,FREQ_USE,REMARK\n\
2026/02/19,ICT,WICHITA TOWER,ATCT,,,,ICT,WICHITA TOWER,AIRPORT,37.649,-97.433,WICHITA,KS,US,WICHITA TOWER,,118.7,,TWR,\n";

        let zip_bytes = create_test_nasr_zip(awy_csv, frq_csv);

        let stage = NasrParseStage;
        let mut data = PipelineData {
            raw_bytes: Some(zip_bytes),
            records: Vec::new(),
            metadata: std::collections::HashMap::new(),
        };

        stage.execute(&mut data).unwrap();
        assert_eq!(data.records.len(), 2);

        let has_airway = data
            .records
            .iter()
            .any(|r| matches!(r, CanonicalRecord::Airway(_)));
        let has_freq = data
            .records
            .iter()
            .any(|r| matches!(r, CanonicalRecord::Frequency(_)));
        assert!(has_airway, "should have an airway record");
        assert!(has_freq, "should have a frequency record");
    }

    #[test]
    fn pipeline_stage_fails_on_invalid_zip() {
        let stage = NasrParseStage;
        let mut data = PipelineData {
            raw_bytes: Some(b"not a zip file".to_vec()),
            records: Vec::new(),
            metadata: std::collections::HashMap::new(),
        };

        let result = stage.execute(&mut data);
        assert!(result.is_err());
    }

    #[test]
    fn airway_type_detection_csv() {
        let csv = "EFF_DATE,REGULATORY,AWY_LOCATION,AWY_ID,POINT_SEQ,FROM_POINT,FROM_PT_TYPE,NAV_NAME,NAV_CITY,ARTCC,ICAO_REGION_CODE,STATE_CODE,COUNTRY_CODE,TO_POINT,MAG_COURSE,OPP_MAG_COURSE,MAG_COURSE_DIST,CHGOVR_PT,CHGOVR_PT_NAME,CHGOVR_PT_DIST,AWY_SEG_GAP_FLAG,SIGNAL_GAP_FLAG,DOGLEG,NEXT_MEA_PT,MIN_ENROUTE_ALT,MIN_ENROUTE_ALT_DIR,MIN_ENROUTE_ALT_OPPOSITE,MIN_ENROUTE_ALT_OPPOSITE_DIR,GPS_MIN_ENROUTE_ALT,GPS_MIN_ENROUTE_ALT_DIR,GPS_MIN_ENROUTE_ALT_OPPOSITE,GPS_MEA_OPPOSITE_DIR,DD_IRU_MEA,DD_IRU_MEA_DIR,DD_I_MEA_OPPOSITE,DD_I_MEA_OPPOSITE_DIR,MIN_OBSTN_CLNC_ALT,MIN_CROSS_ALT,MIN_CROSS_ALT_DIR,MIN_CROSS_ALT_NAV_PT,MIN_CROSS_ALT_OPPOSITE,MIN_CROSS_ALT_OPPOSITE_DIR,MIN_RECEP_ALT,MAX_AUTH_ALT,MEA_GAP,REQD_NAV_PERFORMANCE,REMARK\n\
2026/01/22,N,C,V16,10,A,WP,,,ZKC,,,US,B,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,\n\
2026/01/22,N,C,J60,10,B,VOR,,,ZKC,,,US,C,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,\n\
2026/01/22,N,C,T270,10,C,WP,,,ZKC,,,US,D,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,\n\
2026/01/22,N,C,Q100,10,D,WP,,,ZKC,,,US,E,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,\n";

        let records = parse_awy_seg_csv(csv, Utc::now());
        assert_eq!(records.len(), 4);

        let types: Vec<&str> = records
            .iter()
            .map(|r| match r {
                CanonicalRecord::Airway(a) => a.airway_type.as_str(),
                _ => panic!("expected airway"),
            })
            .collect();
        assert_eq!(types, vec!["Victor", "Jet", "RNAV", "Q-Route"]);
    }
}
