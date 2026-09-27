use airjedi_core::{ObservationFreshness, ObservationIdentity, TimeSourceQuality};
use airjedi_fusion::config::FusionConfig;
use airjedi_fusion::coord::CoordinateFrame;
use airjedi_fusion::sensor::*;
use airjedi_fusion::systems::ObservationBuffer;
use airjedi_fusion::*;
use bevy_app::prelude::*;
use chrono::{Duration, Utc};
use nalgebra::DMatrix;

fn make_adsb_obs(lat: f64, lon: f64, alt: f64, icao: &str) -> SensorObservation {
    SensorObservation {
        sensor_id: SensorId {
            id: "test-adsb".to_string(),
            kind: SensorKind::AdsbReceiver,
            tier: FusionTier::Regional,
            coordinate_frame: CoordinateFrame::Wgs84,
        },
        timestamp: Utc::now(),
        receipt_time: Utc::now(),
        target_id: Some(TargetId {
            domain: TargetDomain::Air,
            id: icao.to_string(),
            id_type: IdentifierType::Icao,
        }),
        measurement: Measurement::PositionVelocity3D {
            lat_deg: lat,
            lon_deg: lon,
            alt_m: Some(alt),
            vel_north_mps: Some(100.0),
            vel_east_mps: Some(0.0),
            vel_down_mps: Some(0.0),
            heading_deg: Some(0.0),
        },
        covariance: ObservationCovariance {
            matrix: DMatrix::identity(6, 6) * 100.0,
        },
        classification_hint: Some(TargetCategory::FixedWing),
        metadata: ObservationMetadata::default(),
    }
}

fn freshness(timestamp: chrono::DateTime<Utc>, frame_sequence: u64) -> ObservationFreshness {
    ObservationFreshness {
        observation_time: timestamp,
        receipt_time: timestamp,
        time_source: TimeSourceQuality::ProtocolTimestamp,
        identity: ObservationIdentity {
            frame_sequence,
            payload_index: 0,
        },
    }
}

fn stamp_observation(
    mut observation: SensorObservation,
    timestamp: chrono::DateTime<Utc>,
    frame_sequence: u64,
    position: bool,
    altitude: bool,
    velocity: bool,
) -> SensorObservation {
    let field_freshness = freshness(timestamp, frame_sequence);
    observation.timestamp = timestamp;
    observation.receipt_time = timestamp;
    observation.metadata.observation_id = Some(field_freshness.identity);
    observation.metadata.time_source = Some(field_freshness.time_source);
    observation.metadata.position_freshness = position.then_some(field_freshness);
    observation.metadata.altitude_freshness = altitude.then_some(field_freshness);
    observation.metadata.velocity_freshness = velocity.then_some(field_freshness);
    observation
}

fn build_test_app() -> App {
    let mut app = App::new();
    app.add_plugins(bevy_time::TimePlugin);
    app.insert_resource(FusionConfig::default());
    app.add_plugins(FusionPlugin);
    app
}

#[test]
fn end_to_end_single_aircraft() {
    let mut app = build_test_app();

    app.world_mut()
        .resource_mut::<ObservationBuffer>()
        .observations
        .push(make_adsb_obs(37.6872, -97.3301, 10000.0, "ABC123"));

    for _ in 0..10 {
        app.update();
    }

    let track_count = app.world_mut().query::<&Track>().iter(app.world()).count();
    assert!(
        track_count >= 1,
        "Expected at least 1 track, got {track_count}"
    );
}

#[test]
fn two_aircraft_separate_tracks() {
    let mut app = build_test_app();

    {
        let mut buffer = app.world_mut().resource_mut::<ObservationBuffer>();
        buffer
            .observations
            .push(make_adsb_obs(37.0, -97.0, 10000.0, "AAA111"));
        buffer
            .observations
            .push(make_adsb_obs(40.0, -80.0, 10000.0, "BBB222"));
    }

    for _ in 0..10 {
        app.update();
    }

    let track_count = app.world_mut().query::<&Track>().iter(app.world()).count();
    assert!(
        track_count >= 2,
        "Expected at least 2 tracks, got {track_count}"
    );
}

#[test]
fn track_has_correct_position() {
    let mut app = build_test_app();

    app.world_mut()
        .resource_mut::<ObservationBuffer>()
        .observations
        .push(make_adsb_obs(37.6872, -97.3301, 10000.0, "POS001"));

    for _ in 0..10 {
        app.update();
    }

    let mut query = app.world_mut().query::<(&Track, &TrackerState)>();
    let positions: Vec<(f64, f64, f64)> = query
        .iter(app.world())
        .map(|(_, tracker)| tracker.position_geodetic())
        .collect();

    assert!(!positions.is_empty());
    let (lat, lon, _alt) = positions[0];
    assert!(
        (lat - 37.6872_f64).abs() < 1.0,
        "Latitude {lat} too far from 37.6872"
    );
    assert!(
        (lon - (-97.3301_f64)).abs() < 1.0,
        "Longitude {lon} too far from -97.3301"
    );
}

#[test]
fn track_has_classification() {
    let mut app = build_test_app();

    app.world_mut()
        .resource_mut::<ObservationBuffer>()
        .observations
        .push(make_adsb_obs(37.0, -97.0, 10000.0, "CLS001"));

    for _ in 0..10 {
        app.update();
    }

    let mut query = app.world_mut().query::<&TargetClassification>();
    let classifications: Vec<_> = query.iter(app.world()).collect();

    assert!(!classifications.is_empty());
    assert_eq!(classifications[0].category, TargetCategory::FixedWing);
}

#[test]
fn coasting_track_reacquires_after_maneuver_gap() {
    // Regression: an aircraft that maneuvers while its signal is lost must
    // reacquire when it reappears. The constant-velocity filter dead-reckons
    // straight during the gap, so the returning (turned) observation fails the
    // Mahalanobis gate on both position and velocity. Before the fix this
    // observation was silently discarded, stranding the track in Coasting until
    // it was cleaned up and re-initiated far away. It must now re-seed instead.
    let mut app = build_test_app();

    // Establish a confirmed track flying north.
    for _ in 0..5 {
        app.world_mut()
            .resource_mut::<ObservationBuffer>()
            .observations
            .push(make_adsb_obs(37.0, -97.0, 10000.0, "TURN01"));
        app.update();
    }

    // Simulate a long signal gap through a turn: force the track into Coasting
    // and let the CV filter dead-reckon ~30s north so its predicted state is far
    // from where the aircraft actually is.
    {
        let mut q = app
            .world_mut()
            .query::<(&mut TrackQuality, &mut TrackerState)>();
        for (mut quality, mut tracker) in q.iter_mut(app.world_mut()) {
            quality.status = TrackStatus::Coasting;
            quality.staleness = std::time::Duration::from_secs(20);
            for _ in 0..30 {
                tracker.variant.predict(1.0);
            }
        }
    }

    // The aircraft reappears well off the dead-reckoned path, now heading east.
    // This observation fails the gate against the coasted state.
    let mut turned = make_adsb_obs(37.0, -96.9, 10000.0, "TURN01");
    if let Measurement::PositionVelocity3D {
        vel_north_mps,
        vel_east_mps,
        ..
    } = &mut turned.measurement
    {
        *vel_north_mps = Some(0.0);
        *vel_east_mps = Some(200.0);
    }
    app.world_mut()
        .resource_mut::<ObservationBuffer>()
        .observations
        .push(turned);

    app.update();

    let mut query = app.world_mut().query::<(&TrackerState, &TrackQuality)>();
    let (tracker, quality) = query.iter(app.world()).next().expect("track exists");
    let (lat, lon, _) = tracker.position_geodetic();
    let status = quality.status;

    assert!(
        (lat - 37.0).abs() < 0.05,
        "latitude should snap to reacquired observation, got {lat}"
    );
    assert!(
        (lon - (-96.9)).abs() < 0.05,
        "longitude should snap to reacquired observation, got {lon}"
    );
    assert_eq!(
        status,
        TrackStatus::Confirmed,
        "track must reacquire after a maneuvering gap, not stay coasting"
    );
}

#[test]
fn observation_buffer_drains() {
    let mut app = build_test_app();

    {
        let mut buffer = app.world_mut().resource_mut::<ObservationBuffer>();
        buffer
            .observations
            .push(make_adsb_obs(37.0, -97.0, 10000.0, "DRN001"));
    }

    for _ in 0..5 {
        app.update();
    }

    let buffer = app.world().resource::<ObservationBuffer>();
    assert!(
        buffer.observations.is_empty(),
        "Buffer should be drained after updates"
    );
}

#[test]
fn telemetry_only_update_reaches_display_without_refreshing_position_or_fusing_twice() {
    let mut app = build_test_app();
    let start = Utc::now();

    for frame_sequence in 1..=3 {
        let observation = stamp_observation(
            make_adsb_obs(37.6872, -97.3301, 10_000.0, "TEL001"),
            start + Duration::seconds(frame_sequence),
            frame_sequence as u64,
            true,
            true,
            true,
        );
        app.world_mut()
            .resource_mut::<ObservationBuffer>()
            .observations
            .push(observation);
        app.update();
    }
    app.update();

    let (before_count, before_last_update, before_position) = {
        let mut query = app
            .world_mut()
            .query::<(&Track, &TrackerState, &TrackQuality)>();
        let (track, tracker, quality) = query.iter(app.world()).next().expect("track exists");
        (
            quality.observation_count,
            track.last_update,
            tracker.position_geodetic(),
        )
    };

    let telemetry_time = start + Duration::seconds(4);
    let telemetry_freshness = freshness(telemetry_time, 4);
    let telemetry = stamp_observation(
        make_adsb_obs(0.0, 0.0, 35_000.0, "TEL001"),
        telemetry_time,
        4,
        false,
        true,
        true,
    );
    app.world_mut()
        .resource_mut::<ObservationBuffer>()
        .observations
        .push(telemetry);
    app.update();

    let (track, tracker, quality) = {
        let mut query = app
            .world_mut()
            .query::<(&Track, &TrackerState, &TrackQuality)>();
        query
            .iter(app.world())
            .next()
            .map(|(track, tracker, quality)| (track.clone(), tracker.clone(), quality.clone()))
            .expect("track exists after telemetry-only update")
    };
    assert_eq!(quality.observation_count, before_count);
    assert_eq!(track.last_update, before_last_update);

    let hint = RawObservationHint {
        latitude: Some(0.0),
        longitude: Some(0.0),
        altitude_ft: Some(35_000),
        track_deg: Some(90.0),
        velocity_kts: Some(250.0),
        position_freshness: None,
        altitude_freshness: Some(telemetry_freshness),
        velocity_freshness: Some(telemetry_freshness),
        ..Default::default()
    };
    let display = derive_display_track(&track, &tracker, &quality, Some(&hint), None);

    assert!((display.latitude - before_position.0).abs() < 1e-6);
    assert!((display.longitude - before_position.1).abs() < 1e-6);
    assert_eq!(display.altitude_ft, Some(35_000));
    assert_eq!(display.velocity_kts, Some(250.0));
    assert!(display.position_freshness.is_none());
    assert_eq!(
        display.altitude_freshness.unwrap().identity.frame_sequence,
        4
    );
    assert_eq!(
        display.velocity_freshness.unwrap().identity.frame_sequence,
        4
    );
}

#[test]
fn exact_duplicate_observation_is_fused_once() {
    let mut config = FusionConfig::default();
    config.initiation.required_detections = 1;

    let mut app = App::new();
    app.add_plugins(bevy_time::TimePlugin);
    app.insert_resource(config);
    app.add_plugins(FusionPlugin);

    let observation = stamp_observation(
        make_adsb_obs(37.6872, -97.3301, 10_000.0, "DUP001"),
        Utc::now(),
        1,
        true,
        true,
        true,
    );
    {
        let mut buffer = app.world_mut().resource_mut::<ObservationBuffer>();
        buffer.observations.push(observation.clone());
        buffer.observations.push(observation);
    }

    for _ in 0..5 {
        app.update();
    }

    let mut query = app.world_mut().query::<&TrackQuality>();
    let quality = query
        .iter(app.world())
        .next()
        .expect("duplicate observation should create a track");
    assert_eq!(quality.observation_count, 1);
}
