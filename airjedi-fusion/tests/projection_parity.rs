use airjedi_core::{
    AltitudeReference, DisplayValueSource, FieldFreshness, HeadingReference, PositionSource,
    RawOverride, TargetCategory, TargetDomain, TargetId, TrackId, VerticalRateReference,
};
use airjedi_fusion::coord::CoordinateFrame;
use airjedi_fusion::sensor::*;
use airjedi_fusion::{
    derive_display_track, raw_observation_hint_for, FusionConfig, Measurement, TimelineStore,
    Track, TrackQuality, TrackStatus,
};
use chrono::{Duration as ChronoDuration, TimeZone, Utc};
use std::time::Duration;

fn observation(at: chrono::DateTime<Utc>, lat: f64, lon: f64) -> SensorObservation {
    SensorObservation {
        sensor_id: SensorId {
            id: "adsb-parity".to_string(),
            kind: SensorKind::AdsbReceiver,
            tier: FusionTier::Regional,
            coordinate_frame: CoordinateFrame::Wgs84,
        },
        timestamp: at,
        receipt_time: at,
        target_id: Some(TargetId {
            domain: TargetDomain::Air,
            id: "A1B2C3".to_string(),
            id_type: airjedi_core::IdentifierType::Icao,
        }),
        measurement: Measurement::PositionVelocity3D {
            lat_deg: lat,
            lon_deg: lon,
            alt_m: Some(3_048.0),
            vel_north_mps: Some(0.0),
            vel_east_mps: Some(51.4444),
            vel_down_mps: Some(-0.508),
            heading_deg: Some(90.0),
        },
        covariance: airjedi_fusion::sensor::ObservationCovariance {
            matrix: airjedi_fusion::nalgebra::DMatrix::identity(6, 6),
        },
        classification_hint: Some(TargetCategory::FixedWing),
        metadata: ObservationMetadata {
            altitude_reference: Some(AltitudeReference::Barometric),
            heading_reference: Some(HeadingReference::GroundTrack),
            vertical_rate_fpm: Some(100),
            airspeed_kts: Some(105.0),
            callsign: Some("PARITY".to_string()),
            observation_id: Some(airjedi_core::ObservationIdentity {
                frame_sequence: 42,
                payload_index: 0,
            }),
            time_source: Some(airjedi_core::TimeSourceQuality::ProtocolTimestamp),
            ..Default::default()
        },
    }
}

fn fixture_state(
    state_time: chrono::DateTime<Utc>,
    observation: SensorObservation,
) -> (
    Track,
    airjedi_fusion::TrackerState,
    TrackQuality,
    TimelineStore,
) {
    let track_id = TrackId::new();
    let target_id = observation.target_id.clone().expect("fixture target id");
    let mut tracker = FusionConfig::default().create_tracker(&TargetCategory::FixedWing);
    tracker.variant.initialize(&observation);
    tracker.last_update = Some(state_time);

    let track = Track {
        id: track_id.clone(),
        cooperative_ids: vec![target_id],
        created_at: state_time,
        last_update: state_time,
        is_on_ground: false,
    };
    let quality = TrackQuality {
        status: TrackStatus::Confirmed,
        observation_count: 1,
        ..Default::default()
    };
    let mut store = TimelineStore::new(FusionConfig::default().store);
    store.insert(observation);
    store.associate(0, &track_id);
    (track, tracker, quality, store)
}

fn project(
    track: &Track,
    tracker: &airjedi_fusion::TrackerState,
    quality: &TrackQuality,
    store: &TimelineStore,
) -> airjedi_core::DisplayTrack {
    let hint = raw_observation_hint_for(store, track);
    derive_display_track(
        track,
        tracker,
        quality,
        hint.as_ref(),
        Some(PositionSource::AdsbIcao),
    )
}

#[test]
fn equivalent_timestamped_observations_project_identically_in_both_modes() {
    let at = Utc.timestamp_opt(1_700_000_000, 0).single().unwrap();
    let raw = observation(at, 37.5, -97.5);
    let (track, tracker, quality, embedded_store) = fixture_state(at, raw.clone());
    let (mut headless_track, headless_tracker, headless_quality, mut headless_store) =
        fixture_state(at, raw.clone());
    headless_track.id = track.id.clone();
    headless_store.insert(raw);

    let embedded = project(&track, &tracker, &quality, &embedded_store);
    let headless = project(
        &headless_track,
        &headless_tracker,
        &headless_quality,
        &headless_store,
    );

    assert_eq!(embedded, headless);
    assert_eq!(embedded.velocity_kts, Some(100.0));
    assert_eq!(embedded.airspeed_kts, Some(105.0));
    assert_eq!(embedded.vertical_rate, Some(100));
    assert_eq!(embedded.altitude_reference, AltitudeReference::Barometric);
    assert_eq!(embedded.heading_reference, HeadingReference::GroundTrack);
    assert_eq!(
        embedded.vertical_rate_reference,
        VerticalRateReference::FeetPerMinute
    );
    assert_eq!(
        embedded.provenance.ground_speed.source,
        DisplayValueSource::RawObservation
    );
    assert_eq!(
        embedded.provenance.ground_speed.freshness,
        FieldFreshness::Fresh
    );
    assert_eq!(
        embedded.provenance.ground_speed.raw_override,
        RawOverride::Applied
    );
}

#[test]
fn stale_raw_and_coasting_states_do_not_override_the_fused_projection() {
    let observed_at = Utc.timestamp_opt(1_700_000_000, 0).single().unwrap();
    let state_at = observed_at + ChronoDuration::seconds(30);
    let raw = observation(observed_at, 37.5, -97.5);
    let (track, tracker, mut quality, store) = fixture_state(state_at, raw);

    quality.staleness = Duration::from_secs(30);
    let stale = project(&track, &tracker, &quality, &store);
    assert_eq!(
        stale.provenance.position.raw_override,
        RawOverride::IgnoredStale
    );
    assert_eq!(
        stale.provenance.position.source,
        DisplayValueSource::FusedEstimate
    );
    assert_eq!(stale.provenance.position.freshness, FieldFreshness::Fresh);

    quality.status = TrackStatus::Coasting;
    let coasting = project(&track, &tracker, &quality, &store);
    assert_eq!(
        coasting.provenance.position.source,
        DisplayValueSource::PredictedEstimate
    );
    assert_eq!(
        coasting.provenance.position.freshness,
        FieldFreshness::Stale
    );
    assert_eq!(
        coasting.provenance.ground_speed.raw_override,
        RawOverride::IgnoredStale
    );
}

#[test]
fn raw_zero_ground_speed_is_distinct_from_missing_velocity_telemetry() {
    let at = Utc.timestamp_opt(1_700_000_000, 0).single().unwrap();
    let mut zero = observation(at, 37.5, -97.5);
    if let Measurement::PositionVelocity3D {
        vel_north_mps,
        vel_east_mps,
        ..
    } = &mut zero.measurement
    {
        *vel_north_mps = Some(0.0);
        *vel_east_mps = Some(0.0);
    }
    let (zero_track, zero_tracker, zero_quality, zero_store) = fixture_state(at, zero);
    let zero_display = project(&zero_track, &zero_tracker, &zero_quality, &zero_store);
    assert_eq!(zero_display.velocity_kts, Some(0.0));
    assert_eq!(
        zero_display.provenance.ground_speed.source,
        DisplayValueSource::RawObservation
    );

    let mut unknown = observation(at, 37.5, -97.5);
    if let Measurement::PositionVelocity3D {
        vel_north_mps,
        vel_east_mps,
        ..
    } = &mut unknown.measurement
    {
        *vel_north_mps = None;
        *vel_east_mps = None;
    }
    let (unknown_track, unknown_tracker, unknown_quality, unknown_store) =
        fixture_state(at, unknown);
    let unknown_display = project(
        &unknown_track,
        &unknown_tracker,
        &unknown_quality,
        &unknown_store,
    );
    assert_eq!(unknown_display.velocity_kts, Some(0.0));
    assert_eq!(
        unknown_display.provenance.ground_speed.source,
        DisplayValueSource::FusedEstimate
    );
    assert_eq!(
        unknown_display.provenance.ground_speed.raw_override,
        RawOverride::Unavailable
    );
}

#[test]
fn reacquisition_applies_a_new_timestamped_raw_observation() {
    let old_at = Utc.timestamp_opt(1_700_000_000, 0).single().unwrap();
    let new_at = old_at + ChronoDuration::seconds(30);
    let old = observation(old_at, 37.5, -97.5);
    let new = observation(new_at, 38.5, -98.5);
    let (track, tracker, mut quality, _) = fixture_state(new_at, old);
    let (_, _, _, mut new_store) = fixture_state(new_at, new.clone());
    new_store.insert(new);
    quality.staleness = Duration::ZERO;

    let reacquired = project(&track, &tracker, &quality, &new_store);
    assert_eq!(reacquired.latitude, 38.5);
    assert_eq!(
        reacquired.provenance.position.raw_override,
        RawOverride::Applied
    );
    assert_eq!(
        reacquired.provenance.position.source,
        DisplayValueSource::RawObservation
    );
}
