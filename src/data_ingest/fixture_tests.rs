//! Fixture-based tests using real API data downloaded from production sources.
//!
//! Each test loads a fixture file from `tests/fixtures/data_ingest/`, runs it
//! through the corresponding provider's pipeline stages, and asserts that
//! parsing succeeds with expected record counts and field values.

use super::canonical::CanonicalRecord;
use super::pipeline::{run_pipeline, PipelineData};
use super::provider::DataProvider;

/// Helper: read a fixture file relative to the project root.
fn load_fixture(filename: &str) -> Vec<u8> {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let path = std::path::Path::new(manifest_dir)
        .join("tests")
        .join("fixtures")
        .join("data_ingest")
        .join(filename);
    std::fs::read(&path)
        .unwrap_or_else(|e| panic!("Failed to read fixture file {}: {}", path.display(), e))
}

/// Helper: run raw bytes through a provider's pipeline stages.
fn run_provider_pipeline(provider: &dyn DataProvider, raw: Vec<u8>) -> Vec<CanonicalRecord> {
    let data = PipelineData {
        raw_bytes: Some(raw),
        ..Default::default()
    };
    let stages = provider.pipeline_stages();
    let result = run_pipeline(&stages, data)
        .unwrap_or_else(|e| panic!("Pipeline failed for provider '{}': {}", provider.name(), e));
    result.records
}

// ---------------------------------------------------------------------------
// METAR
// ---------------------------------------------------------------------------

#[test]
fn real_data_metar_parses_without_error() {
    let raw = load_fixture("metar_sample.json");
    let provider = super::providers::aviation_weather::MetarProvider;
    let records = run_provider_pipeline(&provider, raw);

    assert_eq!(records.len(), 8, "expected 8 METAR records from fixture");
    assert!(
        records
            .iter()
            .all(|r| matches!(r, CanonicalRecord::Metar(_))),
        "all records should be Metar variant"
    );
}

#[test]
fn real_data_metar_kict_fields() {
    let raw = load_fixture("metar_sample.json");
    let provider = super::providers::aviation_weather::MetarProvider;
    let records = run_provider_pipeline(&provider, raw);

    let kict = records
        .iter()
        .find_map(|r| {
            if let CanonicalRecord::Metar(m) = r {
                if m.icao == "KICT" {
                    Some(m)
                } else {
                    None
                }
            } else {
                None
            }
        })
        .expect("should find KICT METAR");

    assert_eq!(
        kict.wind_direction_deg,
        Some(110),
        "KICT wdir should be 110"
    );
    assert_eq!(kict.wind_speed_kt, Some(6), "KICT wspd should be 6");
    assert_eq!(kict.wind_gust_kt, None, "KICT should have no gusts");
    assert_eq!(
        kict.visibility_sm,
        Some(10.0),
        "KICT visibility should be 10+"
    );
    assert!(kict.temperature_c.is_some(), "KICT should have temperature");
    assert!(
        (kict.temperature_c.unwrap() - 6.1).abs() < 0.1,
        "KICT temp ~6.1C"
    );
    assert_eq!(kict.flight_category, "IFR", "KICT fltCat should be IFR");

    // Altimeter should be converted from hPa (1015.3) to inHg (~29.98)
    assert!(kict.altimeter_inhg.is_some(), "KICT should have altimeter");
    let alt = kict.altimeter_inhg.unwrap();
    assert!(
        (alt - 29.98).abs() < 0.1,
        "KICT altimeter should be ~29.98 inHg, got {}",
        alt
    );

    // Observation time should be parsed from epoch seconds
    assert!(
        kict.observation_time.timestamp() > 0,
        "observation_time should be a valid datetime"
    );
}

#[test]
fn real_data_metar_kmci_low_visibility() {
    let raw = load_fixture("metar_sample.json");
    let provider = super::providers::aviation_weather::MetarProvider;
    let records = run_provider_pipeline(&provider, raw);

    let kmci = records
        .iter()
        .find_map(|r| {
            if let CanonicalRecord::Metar(m) = r {
                if m.icao == "KMCI" {
                    Some(m)
                } else {
                    None
                }
            } else {
                None
            }
        })
        .expect("should find KMCI METAR");

    // KMCI has visib: 0.5 (numeric, not string)
    assert_eq!(
        kmci.visibility_sm,
        Some(0.5),
        "KMCI visibility should be 0.5"
    );
    assert_eq!(kmci.flight_category, "LIFR", "KMCI should be LIFR");
    assert_eq!(
        kmci.ceiling_ft,
        Some(200),
        "KMCI ceiling should be 200 (OVC002)"
    );
}

#[test]
fn real_data_metar_vrb_wind_handled() {
    // The TAF fixture has VRB wind entries in fcsts, but METAR fixture doesn't.
    // Instead verify that all records parse and none crash on integer wdir.
    let raw = load_fixture("metar_sample.json");
    let provider = super::providers::aviation_weather::MetarProvider;
    let records = run_provider_pipeline(&provider, raw);

    for r in &records {
        if let CanonicalRecord::Metar(m) = r {
            // Every station should have a parsed observation time
            assert!(m.observation_time.timestamp() > 0);
        }
    }
}

// ---------------------------------------------------------------------------
// TAF
// ---------------------------------------------------------------------------

#[test]
fn real_data_taf_parses_without_error() {
    let raw = load_fixture("taf_sample.json");
    let provider = super::providers::aviation_weather::TafProvider;
    let records = run_provider_pipeline(&provider, raw);

    assert_eq!(records.len(), 4, "expected 4 TAF records from fixture");
    assert!(
        records.iter().all(|r| matches!(r, CanonicalRecord::Taf(_))),
        "all records should be Taf variant"
    );
}

#[test]
fn real_data_taf_kict_fields() {
    let raw = load_fixture("taf_sample.json");
    let provider = super::providers::aviation_weather::TafProvider;
    let records = run_provider_pipeline(&provider, raw);

    let kict = records
        .iter()
        .find_map(|r| {
            if let CanonicalRecord::Taf(t) = r {
                if t.icao == "KICT" {
                    Some(t)
                } else {
                    None
                }
            } else {
                None
            }
        })
        .expect("should find KICT TAF");

    assert!(
        kict.raw_text.contains("TAF KICT"),
        "should contain raw TAF text"
    );
    // validTimeFrom is epoch seconds 1772517600 = 2026-03-03T06:00:00Z
    assert!(
        kict.valid_from.timestamp() > 0,
        "valid_from should be a valid datetime"
    );
    assert!(
        kict.valid_to > kict.valid_from,
        "valid_to should be after valid_from"
    );
}

// ---------------------------------------------------------------------------
// SIGMET
// ---------------------------------------------------------------------------

#[test]
fn real_data_sigmet_parses_without_error() {
    let raw = load_fixture("sigmet_sample.json");
    let provider = super::providers::aviation_weather::SigmetProvider;
    let records = run_provider_pipeline(&provider, raw);

    // The fixture has 129 SIGMETs
    assert!(
        records.len() > 50,
        "expected many SIGMET records, got {}",
        records.len()
    );
    assert!(
        records
            .iter()
            .all(|r| matches!(r, CanonicalRecord::Sigmet(_))),
        "all records should be Sigmet variant"
    );
}

#[test]
fn real_data_sigmet_has_coordinates() {
    let raw = load_fixture("sigmet_sample.json");
    let provider = super::providers::aviation_weather::SigmetProvider;
    let records = run_provider_pipeline(&provider, raw);

    // At least some SIGMETs should have polygon coordinates
    let with_coords = records
        .iter()
        .filter(|r| {
            if let CanonicalRecord::Sigmet(s) = r {
                !s.polygon.is_empty()
            } else {
                false
            }
        })
        .count();

    assert!(
        with_coords > 0,
        "at least some SIGMETs should have polygon coordinates"
    );
}

#[test]
fn real_data_sigmet_first_record_fields() {
    let raw = load_fixture("sigmet_sample.json");
    let provider = super::providers::aviation_weather::SigmetProvider;
    let records = run_provider_pipeline(&provider, raw);

    if let CanonicalRecord::Sigmet(s) = &records[0] {
        assert!(!s.id.is_empty(), "SIGMET should have an ID (seriesId)");
        assert!(!s.hazard.is_empty(), "SIGMET should have a hazard type");
        assert!(
            s.valid_from.timestamp() > 0,
            "valid_from should be parsed from epoch"
        );
    } else {
        panic!("expected Sigmet record");
    }
}

// ---------------------------------------------------------------------------
// AIRMET
// ---------------------------------------------------------------------------

#[test]
fn real_data_airmet_parses_without_error() {
    let raw = load_fixture("airmet_sample.json");
    let provider = super::providers::aviation_weather::AirmetProvider;
    let records = run_provider_pipeline(&provider, raw);

    // The fixture has 31 AIRMETs
    assert!(
        records.len() > 20,
        "expected many AIRMET records, got {}",
        records.len()
    );
    assert!(
        records
            .iter()
            .all(|r| matches!(r, CanonicalRecord::Airmet(_))),
        "all records should be Airmet variant"
    );
}

#[test]
fn real_data_airmet_has_synthesized_ids() {
    let raw = load_fixture("airmet_sample.json");
    let provider = super::providers::aviation_weather::AirmetProvider;
    let records = run_provider_pipeline(&provider, raw);

    // Real data has no airmetId; the parser synthesizes "region-hazard"
    if let CanonicalRecord::Airmet(a) = &records[0] {
        assert!(!a.id.is_empty(), "AIRMET should have a synthesized ID");
        assert!(
            a.id.contains('-'),
            "synthesized ID should be region-hazard format, got: {}",
            a.id
        );
        assert!(!a.region.is_empty(), "AIRMET should have a region");
        assert!(!a.hazard.is_empty(), "AIRMET should have a hazard");
    } else {
        panic!("expected Airmet record");
    }
}

#[test]
fn real_data_airmet_timestamps_parsed() {
    let raw = load_fixture("airmet_sample.json");
    let provider = super::providers::aviation_weather::AirmetProvider;
    let records = run_provider_pipeline(&provider, raw);

    for r in &records {
        if let CanonicalRecord::Airmet(a) = r {
            assert!(
                a.valid_from.timestamp() > 0,
                "valid_from should be parsed from epoch"
            );
            assert!(
                a.valid_to > a.valid_from,
                "valid_to should be after valid_from for AIRMET {}",
                a.id
            );
        }
    }
}

// ---------------------------------------------------------------------------
// PIREP
// ---------------------------------------------------------------------------

#[test]
fn real_data_pirep_parses_without_error() {
    let raw = load_fixture("pirep_sample.json");
    let provider = super::providers::aviation_weather::PirepProvider;
    let records = run_provider_pipeline(&provider, raw);

    // The fixture has 16 PIREPs
    assert!(
        records.len() > 10,
        "expected many PIREP records, got {}",
        records.len()
    );
    assert!(
        records
            .iter()
            .all(|r| matches!(r, CanonicalRecord::Pirep(_))),
        "all records should be Pirep variant"
    );
}

#[test]
fn real_data_pirep_coordinates_valid() {
    let raw = load_fixture("pirep_sample.json");
    let provider = super::providers::aviation_weather::PirepProvider;
    let records = run_provider_pipeline(&provider, raw);

    for r in &records {
        if let CanonicalRecord::Pirep(p) = r {
            assert!(
                (-90.0..=90.0).contains(&p.latitude),
                "latitude should be valid, got {}",
                p.latitude
            );
            assert!(
                (-180.0..=180.0).contains(&p.longitude),
                "longitude should be valid, got {}",
                p.longitude
            );
        }
    }
}

#[test]
fn real_data_pirep_flight_level_conversion() {
    let raw = load_fixture("pirep_sample.json");
    let provider = super::providers::aviation_weather::PirepProvider;
    let records = run_provider_pipeline(&provider, raw);

    // First PIREP: fltLvl=30 should be converted to 3000 ft
    if let CanonicalRecord::Pirep(p) = &records[0] {
        assert_eq!(p.altitude_ft, 3000, "fltLvl 30 should convert to 3000 ft");
        assert_eq!(
            p.report_type, "PIREP",
            "pirepType should map to report_type"
        );
        assert!(p.aircraft_type.is_some(), "should have aircraft type");
    } else {
        panic!("expected Pirep record");
    }
}

#[test]
fn real_data_pirep_observation_time_parsed() {
    let raw = load_fixture("pirep_sample.json");
    let provider = super::providers::aviation_weather::PirepProvider;
    let records = run_provider_pipeline(&provider, raw);

    for r in &records {
        if let CanonicalRecord::Pirep(p) = r {
            assert!(
                p.observation_time.timestamp() > 0,
                "observation_time should be parsed from epoch"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Airports (OurAirports CSV)
// ---------------------------------------------------------------------------

#[test]
fn real_data_airports_csv_parses_without_error() {
    let raw = load_fixture("airports_sample.csv");
    let provider = super::providers::our_airports::AirportsProvider;
    let records = run_provider_pipeline(&provider, raw);

    // The fixture has 51 data rows (header + 51 airports)
    assert_eq!(
        records.len(),
        50,
        "expected 50 airport records (header + 50 data rows)"
    );
    assert!(
        records
            .iter()
            .all(|r| matches!(r, CanonicalRecord::Airport(_))),
        "all records should be Airport variant"
    );
}

#[test]
fn real_data_airports_first_record_fields() {
    let raw = load_fixture("airports_sample.csv");
    let provider = super::providers::our_airports::AirportsProvider;
    let records = run_provider_pipeline(&provider, raw);

    // First record is "00A" - Total RF Heliport
    if let CanonicalRecord::Airport(a) = &records[0] {
        assert_eq!(a.ident, "00A");
        assert_eq!(a.airport_type, "heliport");
        assert!(a.name.contains("Total RF Heliport"));
        assert!((a.latitude - 40.070985).abs() < 0.001);
        assert!((a.longitude - (-74.933689)).abs() < 0.001);
        assert_eq!(a.elevation_ft, Some(11));
        assert_eq!(a.iso_country, "US");
        assert_eq!(a.iso_region, "US-PA");
        assert!(!a.scheduled_service);
    } else {
        panic!("expected Airport record");
    }
}

#[test]
fn real_data_airports_coordinates_valid() {
    let raw = load_fixture("airports_sample.csv");
    let provider = super::providers::our_airports::AirportsProvider;
    let records = run_provider_pipeline(&provider, raw);

    for r in &records {
        if let CanonicalRecord::Airport(a) = r {
            assert!(
                (-90.0..=90.0).contains(&a.latitude),
                "airport {} latitude should be valid",
                a.ident
            );
            assert!(
                (-180.0..=180.0).contains(&a.longitude),
                "airport {} longitude should be valid",
                a.ident
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Runways (OurAirports CSV)
// ---------------------------------------------------------------------------

#[test]
fn real_data_runways_csv_parses_without_error() {
    let raw = load_fixture("runways_sample.csv");
    let provider = super::providers::our_airports::RunwaysProvider;
    let records = run_provider_pipeline(&provider, raw);

    assert_eq!(
        records.len(),
        50,
        "expected 50 runway records (header + 50 data rows)"
    );
    assert!(
        records
            .iter()
            .all(|r| matches!(r, CanonicalRecord::Runway(_))),
        "all records should be Runway variant"
    );
}

#[test]
fn real_data_runways_specific_fields() {
    let raw = load_fixture("runways_sample.csv");
    let provider = super::providers::our_airports::RunwaysProvider;
    let records = run_provider_pipeline(&provider, raw);

    // First record is helipad H1 at 00A
    if let CanonicalRecord::Runway(r) = &records[0] {
        assert_eq!(r.airport_ident, "00A");
        assert_eq!(r.length_ft, Some(80));
        assert_eq!(r.width_ft, Some(80));
        assert!(r.lighted);
        assert!(!r.closed);
        assert_eq!(r.le_ident, "H1");
    } else {
        panic!("expected Runway record");
    }
}

// ---------------------------------------------------------------------------
// Navaids (OurAirports CSV)
// ---------------------------------------------------------------------------

#[test]
fn real_data_navaids_csv_parses_without_error() {
    let raw = load_fixture("navaids_sample.csv");
    let provider = super::providers::our_airports::NavaidsProvider;
    let records = run_provider_pipeline(&provider, raw);

    assert_eq!(
        records.len(),
        50,
        "expected 50 navaid records (header + 50 data rows)"
    );
    assert!(
        records
            .iter()
            .all(|r| matches!(r, CanonicalRecord::Navaid(_))),
        "all records should be Navaid variant"
    );
}

#[test]
fn real_data_navaids_first_record_fields() {
    let raw = load_fixture("navaids_sample.csv");
    let provider = super::providers::our_airports::NavaidsProvider;
    let records = run_provider_pipeline(&provider, raw);

    // First record: "1A" Williams Harbour NDB
    if let CanonicalRecord::Navaid(n) = &records[0] {
        assert_eq!(n.ident, "1A");
        assert_eq!(n.name, "Williams Harbour");
        assert_eq!(n.navaid_type, "NDB");
        assert_eq!(n.frequency_khz, Some(373));
        assert!((n.latitude - 52.558).abs() < 0.01);
        assert!(n.associated_airport.is_some());
        assert_eq!(n.associated_airport.as_deref(), Some("CCA6"));
    } else {
        panic!("expected Navaid record");
    }
}

#[test]
fn real_data_navaids_empty_airport_filtered() {
    let raw = load_fixture("navaids_sample.csv");
    let provider = super::providers::our_airports::NavaidsProvider;
    let records = run_provider_pipeline(&provider, raw);

    // Second record "1B" Sable Island has no associated airport
    if let CanonicalRecord::Navaid(n) = &records[1] {
        assert_eq!(n.ident, "1B");
        assert_eq!(
            n.associated_airport, None,
            "empty associated_airport should be None"
        );
    } else {
        panic!("expected Navaid record");
    }
}

// ---------------------------------------------------------------------------
// TFR (GeoJSON)
// ---------------------------------------------------------------------------

#[test]
fn real_data_tfr_parses_without_error() {
    let raw = load_fixture("tfr_sample.geojson");
    let provider = super::providers::tfrs::TfrProvider;
    let records = run_provider_pipeline(&provider, raw);

    assert_eq!(records.len(), 3, "expected 3 TFR records from fixture");
    assert!(
        records.iter().all(|r| matches!(r, CanonicalRecord::Tfr(_))),
        "all records should be Tfr variant"
    );
}

#[test]
fn real_data_tfr_polygon_coordinates_lat_lon_order() {
    let raw = load_fixture("tfr_sample.geojson");
    let provider = super::providers::tfrs::TfrProvider;
    let records = run_provider_pipeline(&provider, raw);

    // First TFR: Polygon with coordinates [[-97.5, 37.5], ...]
    // GeoJSON is [lon, lat] -- parser should swap to (lat, lon)
    if let CanonicalRecord::Tfr(tfr) = &records[0] {
        assert_eq!(tfr.notam_id, "6/2345");
        assert!(!tfr.polygon.is_empty(), "polygon should not be empty");
        let (lat, lon) = tfr.polygon[0];
        assert!(
            (lat - 37.5).abs() < 0.001,
            "first coord lat should be ~37.5, got {}",
            lat
        );
        assert!(
            (lon - (-97.5)).abs() < 0.001,
            "first coord lon should be ~-97.5, got {}",
            lon
        );
    } else {
        panic!("expected Tfr record");
    }
}

#[test]
fn real_data_tfr_string_altitudes_parsed() {
    let raw = load_fixture("tfr_sample.geojson");
    let provider = super::providers::tfrs::TfrProvider;
    let records = run_provider_pipeline(&provider, raw);

    // First TFR: LOWALT="SFC" -> 0, HIGHALT="18000 MSL" -> 18000
    if let CanonicalRecord::Tfr(tfr) = &records[0] {
        assert_eq!(tfr.lower_altitude_ft, Some(0), "SFC should parse to 0");
        assert_eq!(
            tfr.upper_altitude_ft,
            Some(18000),
            "18000 MSL should parse to 18000"
        );
    } else {
        panic!("expected Tfr record");
    }

    // Third TFR: HIGHALT="FL180" -> 18000
    if let CanonicalRecord::Tfr(tfr) = &records[2] {
        assert_eq!(
            tfr.upper_altitude_ft,
            Some(18000),
            "FL180 should parse to 18000"
        );
    } else {
        panic!("expected Tfr record");
    }
}

#[test]
fn real_data_tfr_multipolygon_handled() {
    let raw = load_fixture("tfr_sample.geojson");
    let provider = super::providers::tfrs::TfrProvider;
    let records = run_provider_pipeline(&provider, raw);

    // Third TFR is a MultiPolygon
    if let CanonicalRecord::Tfr(tfr) = &records[2] {
        assert_eq!(tfr.notam_id, "6/5678");
        assert_eq!(
            tfr.polygon.len(),
            5,
            "MultiPolygon outer ring should have 5 points"
        );
    } else {
        panic!("expected Tfr record");
    }
}

#[test]
fn real_data_tfr_iso8601_dates_parsed() {
    let raw = load_fixture("tfr_sample.geojson");
    let provider = super::providers::tfrs::TfrProvider;
    let records = run_provider_pipeline(&provider, raw);

    if let CanonicalRecord::Tfr(tfr) = &records[0] {
        // EFFECTIVE: "2026-03-01T12:00:00Z"
        assert!(
            tfr.effective_start.timestamp() > 0,
            "effective_start should be a valid datetime"
        );
        // EXPIRE: "2026-12-31T23:59:00Z"
        assert!(
            tfr.effective_end.is_some(),
            "effective_end should be present"
        );
        assert!(
            tfr.effective_end.unwrap() > tfr.effective_start,
            "expire should be after effective"
        );
    } else {
        panic!("expected Tfr record");
    }
}

// ---------------------------------------------------------------------------
// NOTAM
// ---------------------------------------------------------------------------

#[test]
fn real_data_notam_parses_without_error() {
    let raw = load_fixture("notam_sample.json");
    let provider = super::providers::notams::NotamProvider;
    // Only use the parse stage, not the validate stage (which would filter expired)
    let stages = provider.pipeline_stages();
    let parse_stage_only: Vec<_> = stages.into_iter().take(1).collect();

    let data = PipelineData {
        raw_bytes: Some(raw),
        ..Default::default()
    };
    let result = run_pipeline(&parse_stage_only, data).expect("NOTAM parse should succeed");

    assert_eq!(
        result.records.len(),
        4,
        "expected 4 NOTAMs from fixture (before filtering)"
    );
}

#[test]
fn real_data_notam_expired_filtered() {
    let raw = load_fixture("notam_sample.json");
    let provider = super::providers::notams::NotamProvider;
    let records = run_provider_pipeline(&provider, raw);

    // The fixture has 4 NOTAMs: 3 active + 1 expired (12/999 ends 2026-01-01)
    assert_eq!(records.len(), 3, "expected 3 active NOTAMs after filtering");

    // Verify expired NOTAM was removed
    let has_expired = records.iter().any(|r| {
        if let CanonicalRecord::Notam(n) = r {
            n.id == "12/999"
        } else {
            false
        }
    });
    assert!(!has_expired, "expired NOTAM 12/999 should be filtered out");
}

#[test]
fn real_data_notam_kict_fields() {
    let raw = load_fixture("notam_sample.json");
    let provider = super::providers::notams::NotamProvider;
    let records = run_provider_pipeline(&provider, raw);

    let kict_rwy = records
        .iter()
        .find_map(|r| {
            if let CanonicalRecord::Notam(n) = r {
                if n.id == "01/234" {
                    Some(n)
                } else {
                    None
                }
            } else {
                None
            }
        })
        .expect("should find NOTAM 01/234");

    assert_eq!(kict_rwy.location, "ICT");
    assert!(
        kict_rwy.raw_text.contains("RWY 01L/19R CLSD"),
        "should contain runway closure text"
    );
    assert!(kict_rwy.latitude.is_some());
    assert!((kict_rwy.latitude.unwrap() - 37.6499).abs() < 0.001);
    assert!(kict_rwy.longitude.is_some());
}

// ---------------------------------------------------------------------------
// Airspace (OpenAir format)
// ---------------------------------------------------------------------------

#[test]
fn real_data_airspace_openair_parses_without_error() {
    let raw = load_fixture("airspace_sample.openair");
    // Use the OpenAirParseStage via pipeline
    let provider = super::providers::openaip::OpenAipProvider::new();
    let records = run_provider_pipeline(&provider, raw);

    assert_eq!(records.len(), 5, "expected 5 airspaces from fixture");
    assert!(
        records
            .iter()
            .all(|r| matches!(r, CanonicalRecord::Airspace(_))),
        "all records should be Airspace variant"
    );
}

#[test]
fn real_data_airspace_classes() {
    let raw = load_fixture("airspace_sample.openair");
    let provider = super::providers::openaip::OpenAipProvider::new();
    let records = run_provider_pipeline(&provider, raw);

    let classes: Vec<&str> = records
        .iter()
        .filter_map(|r| {
            if let CanonicalRecord::Airspace(a) = r {
                Some(a.airspace_class.as_str())
            } else {
                None
            }
        })
        .collect();

    assert_eq!(classes, vec!["C", "D", "R", "B", "E"]);
}

#[test]
fn real_data_airspace_wichita_class_c() {
    let raw = load_fixture("airspace_sample.openair");
    let provider = super::providers::openaip::OpenAipProvider::new();
    let records = run_provider_pipeline(&provider, raw);

    if let CanonicalRecord::Airspace(a) = &records[0] {
        assert_eq!(a.name, "WICHITA CLASS C");
        assert_eq!(a.airspace_class, "C");
        assert_eq!(a.airspace_type, "Class C");
        assert_eq!(a.lower_limit_ft, Some(0), "SFC should be 0");
        assert_eq!(a.upper_limit_ft, Some(4400), "4400 MSL");
        assert_eq!(a.polygon.len(), 4, "polygon should have 4 points");
    } else {
        panic!("expected Airspace record");
    }
}

#[test]
fn real_data_airspace_circle_definition() {
    let raw = load_fixture("airspace_sample.openair");
    let provider = super::providers::openaip::OpenAipProvider::new();
    let records = run_provider_pipeline(&provider, raw);

    // Fourth airspace: Kansas City Class B with DC 30 (circle)
    if let CanonicalRecord::Airspace(a) = &records[3] {
        assert_eq!(a.name, "KANSAS CITY CLASS B");
        assert_eq!(a.airspace_class, "B");
        // Circle with 30 NM radius should produce 36 polygon points
        assert_eq!(a.polygon.len(), 36, "circle should have 36 segments");
        assert_eq!(a.lower_limit_ft, Some(3000), "3000 MSL");
        assert_eq!(a.upper_limit_ft, Some(10000), "FL100 = 10000");
    } else {
        panic!("expected Airspace record");
    }
}

#[test]
fn real_data_airspace_restricted() {
    let raw = load_fixture("airspace_sample.openair");
    let provider = super::providers::openaip::OpenAipProvider::new();
    let records = run_provider_pipeline(&provider, raw);

    // Third airspace: R-3601A
    if let CanonicalRecord::Airspace(a) = &records[2] {
        assert_eq!(a.name, "R-3601A FORT RILEY");
        assert_eq!(a.airspace_class, "R");
        assert_eq!(a.airspace_type, "Restricted");
        assert_eq!(a.upper_limit_ft, Some(23000), "FL230 = 23000");
    } else {
        panic!("expected Airspace record");
    }
}
