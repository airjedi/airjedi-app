use airjedi_fusion::config::FusionConfig;
use airjedi_fusion::coord::CoordinateFrame;
use airjedi_fusion::sensor::*;
use airjedi_fusion::systems::ObservationBuffer;
use airjedi_fusion::*;
use bevy_app::prelude::*;
use bevy_ecs::prelude::*;
use chrono::Utc;
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
