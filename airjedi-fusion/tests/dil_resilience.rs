//! DIL (Disconnected, Intermittent, Limited) resilience tests.
//!
//! These tests prove that:
//! 1. The fusion pipeline works identically with and without NATS transport
//! 2. An unreachable NATS server doesn't affect local fusion
//! 3. The observation buffer and crossbeam channel bridge handle all edge cases
//! 4. OOSM (out-of-sequence measurements) are handled correctly for late arrivals
//! 5. Message serialization survives round-trips without data loss

use airjedi_fusion::config::FusionConfig;
use airjedi_fusion::coord::CoordinateFrame;
use airjedi_fusion::filter::ekf::ProcessNoiseConfig;
use airjedi_fusion::filter::oosm::handle_oosm;
use airjedi_fusion::filter::{FilterResult, OosmConfig, TrackerState};
use airjedi_fusion::sensor::*;
use airjedi_fusion::store::{StoreConfig, TimelineStore};
use airjedi_fusion::systems::ObservationBuffer;
use airjedi_fusion::transport::messages;
use airjedi_fusion::*;
use bevy_app::prelude::*;
use bevy_ecs::prelude::*;
use chrono::{Duration, Utc};
use nalgebra::DMatrix;

// ============================================================================
// Test helpers
// ============================================================================

fn make_obs(lat: f64, lon: f64, icao: &str) -> SensorObservation {
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
            alt_m: Some(10000.0),
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

fn make_obs_at_time(t: Timestamp, lat: f64, icao: &str) -> SensorObservation {
    let mut obs = make_obs(lat, -97.0, icao);
    obs.timestamp = t;
    obs
}

fn build_app() -> App {
    let mut app = App::new();
    app.add_plugins(bevy_time::TimePlugin);
    app.insert_resource(FusionConfig::default());
    app.add_plugins(FusionPlugin);
    app
}

fn build_app_with_nats_config(server_url: &str) -> App {
    let mut app = App::new();
    app.add_plugins(bevy_time::TimePlugin);
    let mut config = FusionConfig::default();
    config.transport = Some(airjedi_fusion::transport::NatsTransportConfig {
        server_url: server_url.to_string(),
        ..Default::default()
    });
    app.insert_resource(config);
    app.add_plugins(FusionPlugin);
    app
}

fn inject_obs(app: &mut App, obs: SensorObservation) {
    app.world_mut()
        .resource_mut::<ObservationBuffer>()
        .observations
        .push(obs);
}

fn track_count(app: &mut App) -> usize {
    app.world_mut()
        .query::<&Track>()
        .iter(app.world())
        .count()
}

fn get_track_positions(app: &mut App) -> Vec<(f64, f64, f64)> {
    app.world_mut()
        .query::<&TrackerState>()
        .iter(app.world())
        .map(|t| t.position_geodetic())
        .collect()
}

fn run_updates(app: &mut App, n: usize) {
    for _ in 0..n {
        app.update();
    }
}

// ============================================================================
// Test 1: Fusion works without any NATS configuration
// ============================================================================

#[test]
fn fusion_works_without_transport_config() {
    let mut app = build_app(); // default config, transport = None

    inject_obs(&mut app, make_obs(37.0, -97.0, "NO_NATS_1"));
    inject_obs(&mut app, make_obs(40.0, -80.0, "NO_NATS_2"));
    run_updates(&mut app, 10);

    assert!(
        track_count(&mut app) >= 2,
        "Pipeline must create tracks without NATS"
    );

    let positions = get_track_positions(&mut app);
    assert!(positions.len() >= 2);
    // Verify positions are reasonable (within 1 degree of input)
    let has_track_near_37 = positions.iter().any(|(lat, _, _)| (lat - 37.0).abs() < 1.0);
    let has_track_near_40 = positions.iter().any(|(lat, _, _)| (lat - 40.0).abs() < 1.0);
    assert!(has_track_near_37, "Should have track near lat 37");
    assert!(has_track_near_40, "Should have track near lat 40");
}

// ============================================================================
// Test 2: Fusion works with NATS configured but server unreachable
// ============================================================================

#[cfg(feature = "nats")]
#[test]
fn fusion_works_with_unreachable_nats() {
    // Point at a port nothing is listening on
    let mut app = build_app_with_nats_config("nats://127.0.0.1:14222");

    // Give the connection attempt time to fail
    std::thread::sleep(std::time::Duration::from_millis(200));

    inject_obs(&mut app, make_obs(37.0, -97.0, "OFFLINE_1"));
    inject_obs(&mut app, make_obs(42.0, -71.0, "OFFLINE_2"));
    run_updates(&mut app, 10);

    assert!(
        track_count(&mut app) >= 2,
        "Pipeline must create tracks even when NATS is unreachable"
    );

    let positions = get_track_positions(&mut app);
    assert!(positions.len() >= 2);
}

// ============================================================================
// Test 3: Fusion pipeline produces identical results with and without NATS
// ============================================================================

#[cfg(feature = "nats")]
#[test]
fn fusion_results_identical_with_and_without_nats() {
    // Run without NATS
    let mut app_no_nats = build_app();
    inject_obs(&mut app_no_nats, make_obs(37.6872, -97.3301, "COMPARE_1"));
    run_updates(&mut app_no_nats, 10);
    let positions_no_nats = get_track_positions(&mut app_no_nats);

    // Run with unreachable NATS
    let mut app_with_nats = build_app_with_nats_config("nats://127.0.0.1:14222");
    std::thread::sleep(std::time::Duration::from_millis(200));
    inject_obs(&mut app_with_nats, make_obs(37.6872, -97.3301, "COMPARE_1"));
    run_updates(&mut app_with_nats, 10);
    let positions_with_nats = get_track_positions(&mut app_with_nats);

    assert_eq!(
        positions_no_nats.len(),
        positions_with_nats.len(),
        "Same number of tracks with and without NATS"
    );

    // Positions should be very close (not identical due to timing, but within 0.1 deg)
    for (p1, p2) in positions_no_nats.iter().zip(positions_with_nats.iter()) {
        assert!(
            (p1.0 - p2.0).abs() < 0.1,
            "Latitude diverged: {} vs {}",
            p1.0,
            p2.0
        );
        assert!(
            (p1.1 - p2.1).abs() < 0.1,
            "Longitude diverged: {} vs {}",
            p1.1,
            p2.1
        );
    }
}

// ============================================================================
// Test 4: Observation buffer continues to drain when NATS is down
// ============================================================================

#[cfg(feature = "nats")]
#[test]
fn buffer_drains_when_nats_down() {
    let mut app = build_app_with_nats_config("nats://127.0.0.1:14222");
    std::thread::sleep(std::time::Duration::from_millis(200));

    inject_obs(&mut app, make_obs(37.0, -97.0, "DRAIN_1"));
    run_updates(&mut app, 5);

    let buffer = app.world().resource::<ObservationBuffer>();
    assert!(
        buffer.observations.is_empty(),
        "Buffer must drain even when NATS is unreachable"
    );
}

// ============================================================================
// Test 5: Multiple update cycles work without NATS
// ============================================================================

#[test]
fn continuous_updates_without_nats() {
    let mut app = build_app();

    // Simulate 50 update cycles with new observations arriving periodically
    for i in 0..50 {
        if i % 5 == 0 {
            let lat = 37.0 + (i as f64) * 0.001;
            inject_obs(&mut app, make_obs(lat, -97.0, "CONT_1"));
        }
        app.update();
    }

    let count = track_count(&mut app);
    assert!(count >= 1, "Should maintain at least 1 track over 50 cycles");
}

// ============================================================================
// Test 6: OOSM - late observation rejected when too old
// ============================================================================

#[test]
fn oosm_rejects_observation_beyond_max_lag() {
    let now = Utc::now();
    let mut tracker = TrackerState::new_6dof(ProcessNoiseConfig::default());
    let init_obs = make_obs_at_time(now, 37.0, "OOSM_OLD");
    tracker.variant.initialize(&init_obs);

    let track_id = TrackId::new();
    let store = TimelineStore::new(StoreConfig::default());
    let config = OosmConfig {
        max_lag: std::time::Duration::from_secs(10),
        history_depth: 10,
    };

    // Observation from 60 seconds ago - exceeds 10s max_lag
    let old_obs = make_obs_at_time(now - Duration::seconds(60), 37.1, "OOSM_OLD");
    let result = handle_oosm(&mut tracker, &old_obs, &track_id, &store, &config, now);
    assert!(
        matches!(result, FilterResult::OutlierRejected { .. }),
        "Observations beyond max_lag must be rejected"
    );
}

// ============================================================================
// Test 7: OOSM - late observation within tolerance is processed
// ============================================================================

#[test]
fn oosm_accepts_observation_within_lag() {
    let now = Utc::now();
    let mut tracker = TrackerState::new_6dof(ProcessNoiseConfig::default());
    let init_obs = make_obs_at_time(now - Duration::seconds(10), 37.0, "OOSM_OK");
    tracker.variant.initialize(&init_obs);

    // Build state history
    for _ in 0..5 {
        tracker.variant.predict(1.0);
    }

    let track_id = TrackId::new();
    let store = TimelineStore::new(StoreConfig::default());
    let config = OosmConfig {
        max_lag: std::time::Duration::from_secs(30),
        history_depth: 10,
    };

    let late_obs = make_obs_at_time(now - Duration::seconds(3), 37.001, "OOSM_OK");
    let result = handle_oosm(&mut tracker, &late_obs, &track_id, &store, &config, now);
    assert!(
        !matches!(result, FilterResult::DivergenceDetected),
        "Late observation within lag should not cause divergence"
    );
}

// ============================================================================
// Test 8: OOSM - filter state recovers after rollback-and-replay
// ============================================================================

#[test]
fn oosm_rollback_preserves_filter_stability() {
    let now = Utc::now();
    let mut tracker = TrackerState::new_6dof(ProcessNoiseConfig::default());
    let init_obs = make_obs_at_time(now - Duration::seconds(10), 37.0, "OOSM_STABLE");
    tracker.variant.initialize(&init_obs);

    // Run several predict steps to build history
    for _ in 0..5 {
        tracker.variant.predict(1.0);
    }

    let pos_before = tracker.position_geodetic();

    let track_id = TrackId::new();
    let store = TimelineStore::new(StoreConfig::default());
    let config = OosmConfig::default();

    // Process a late observation
    let late_obs = make_obs_at_time(now - Duration::seconds(2), 37.0, "OOSM_STABLE");
    let _ = handle_oosm(&mut tracker, &late_obs, &track_id, &store, &config, now);

    let pos_after = tracker.position_geodetic();

    // Position should still be in the same general area (filter didn't diverge)
    assert!(
        (pos_after.0 - pos_before.0).abs() < 1.0,
        "Latitude should be stable after OOSM: before={}, after={}",
        pos_before.0,
        pos_after.0
    );
    assert!(
        (pos_after.1 - pos_before.1).abs() < 1.0,
        "Longitude should be stable after OOSM: before={}, after={}",
        pos_before.1,
        pos_after.1
    );
}

// ============================================================================
// Test 9: Message serialization round-trip preserves fidelity
// ============================================================================

#[test]
fn message_round_trip_preserves_track_data() {
    let mut tracker = TrackerState::new_6dof(ProcessNoiseConfig::default());
    let obs = make_obs(37.6872, -97.3301, "RT_TEST");
    tracker.variant.initialize(&obs);

    let track = Track {
        id: TrackId::new(),
        cooperative_ids: vec![TargetId {
            domain: TargetDomain::Air,
            id: "RT_TEST".to_string(),
            id_type: IdentifierType::Icao,
        }],
        created_at: Utc::now(),
        last_update: Utc::now(),
        is_on_ground: false,
    };
    let quality = TrackQuality {
        status: TrackStatus::Confirmed,
        confidence: 0.85,
        sensor_count: 3,
        ..Default::default()
    };
    let classification = TargetClassification {
        category: TargetCategory::FixedWing,
        ..Default::default()
    };

    // Serialize to wire message
    let msg = messages::track_to_message(
        &track,
        &tracker,
        &quality,
        &classification,
        "dil-test-node",
        FusionTier::Edge,
    );

    // Deserialize back to observation
    let result_obs = messages::message_to_observation(&msg, Utc::now());

    // The observation should carry the fused state
    assert_eq!(result_obs.sensor_id.kind, SensorKind::UpstreamFusedTrack);
    assert_eq!(result_obs.target_id.as_ref().unwrap().id, "RT_TEST");

    if let Measurement::FusedEstimate {
        state,
        covariance,
        track_quality,
        ..
    } = &result_obs.measurement
    {
        assert_eq!(state.len(), 6);
        assert_eq!(covariance.nrows(), 6);
        assert!((track_quality - 0.85).abs() < 0.01);
    } else {
        panic!("Expected FusedEstimate measurement");
    }
}

// ============================================================================
// Test 10: Upstream fused track can be ingested as a sensor observation
// ============================================================================

#[test]
fn upstream_fused_track_enters_local_pipeline() {
    let mut app = build_app();

    // Create a message as if from an upstream tier
    let mut tracker = TrackerState::new_6dof(ProcessNoiseConfig::default());
    let obs = make_obs(37.6872, -97.3301, "UPSTREAM_1");
    tracker.variant.initialize(&obs);

    let track = Track {
        id: TrackId::new(),
        cooperative_ids: vec![TargetId {
            domain: TargetDomain::Air,
            id: "UPSTREAM_1".to_string(),
            id_type: IdentifierType::Icao,
        }],
        created_at: Utc::now(),
        last_update: Utc::now(),
        is_on_ground: false,
    };
    let quality = TrackQuality {
        status: TrackStatus::Confirmed,
        confidence: 0.9,
        sensor_count: 2,
        ..Default::default()
    };
    let classification = TargetClassification::default();

    let msg = messages::track_to_message(
        &track, &tracker, &quality, &classification,
        "upstream-node", FusionTier::Global,
    );

    // Convert to observation and inject into local pipeline
    let upstream_obs = messages::message_to_observation(&msg, Utc::now());
    inject_obs(&mut app, upstream_obs);
    run_updates(&mut app, 10);

    // Should create a local track from the upstream data
    assert!(
        track_count(&mut app) >= 1,
        "Local pipeline should create tracks from upstream fused observations"
    );
}

// ============================================================================
// Test 11: Multiple upstream sources don't conflict with local sensors
// ============================================================================

#[test]
fn mixed_local_and_upstream_observations() {
    let mut app = build_app();

    // Local ADS-B observation
    inject_obs(&mut app, make_obs(37.0, -97.0, "LOCAL_1"));

    // Upstream fused track observation (different aircraft)
    let mut tracker = TrackerState::new_6dof(ProcessNoiseConfig::default());
    let upstream_init = make_obs(42.0, -71.0, "UPSTREAM_2");
    tracker.variant.initialize(&upstream_init);

    let track = Track {
        id: TrackId::new(),
        cooperative_ids: vec![TargetId {
            domain: TargetDomain::Air,
            id: "UPSTREAM_2".to_string(),
            id_type: IdentifierType::Icao,
        }],
        created_at: Utc::now(),
        last_update: Utc::now(),
        is_on_ground: false,
    };
    let quality = TrackQuality::default();
    let classification = TargetClassification::default();

    let msg = messages::track_to_message(
        &track, &tracker, &quality, &classification,
        "remote", FusionTier::Regional,
    );
    let upstream_obs = messages::message_to_observation(&msg, Utc::now());
    inject_obs(&mut app, upstream_obs);

    run_updates(&mut app, 10);

    assert!(
        track_count(&mut app) >= 2,
        "Should have separate tracks for local and upstream sources"
    );
}

// ============================================================================
// Test 12: Track quality is maintained through OOSM processing
// ============================================================================

#[test]
fn track_quality_stable_through_oosm() {
    let now = Utc::now();
    let mut tracker = TrackerState::new_6dof(ProcessNoiseConfig {
        position_noise: 10.0,
        velocity_noise: 1.0,
    });

    // Initialize with position-only observation (no velocity) so filter doesn't
    // predict movement that conflicts with subsequent stationary updates
    let init_obs = SensorObservation {
        sensor_id: SensorId {
            id: "test".to_string(),
            kind: SensorKind::AdsbReceiver,
            tier: FusionTier::Regional,
            coordinate_frame: CoordinateFrame::Wgs84,
        },
        timestamp: now - Duration::seconds(20),
        receipt_time: now,
        target_id: None,
        measurement: Measurement::PositionVelocity3D {
            lat_deg: 37.0,
            lon_deg: -97.0,
            alt_m: Some(10000.0),
            vel_north_mps: None,
            vel_east_mps: None,
            vel_down_mps: None,
            heading_deg: None,
        },
        covariance: ObservationCovariance {
            matrix: DMatrix::identity(3, 3) * 1000.0,
        },
        classification_hint: None,
        metadata: ObservationMetadata::default(),
    };
    tracker.variant.initialize(&init_obs);

    // Run predict + update cycles with larger covariance observations
    for i in 1..=5 {
        tracker.variant.predict(1.0);
        let mut update_obs = init_obs.clone();
        update_obs.timestamp = now - Duration::seconds(20 - i);
        let result = tracker.variant.update(&update_obs);
        assert!(
            matches!(result, FilterResult::Updated),
            "Update {i} should succeed, got {result:?}"
        );
    }

    let cov_before_oosm = tracker.variant.covariance_mat().trace();

    // Process a late observation via OOSM
    let track_id = TrackId::new();
    let store = TimelineStore::new(StoreConfig::default());
    let config = OosmConfig::default();
    let mut late_obs = init_obs.clone();
    late_obs.timestamp = now - Duration::seconds(12);
    let _ = handle_oosm(&mut tracker, &late_obs, &track_id, &store, &config, now);

    let cov_after_oosm = tracker.variant.covariance_mat().trace();

    // Covariance should not have exploded
    assert!(
        cov_after_oosm < cov_before_oosm * 100.0,
        "Covariance exploded after OOSM: before={cov_before_oosm}, after={cov_after_oosm}"
    );
}

// ============================================================================
// Test 13: Pipeline handles empty buffers gracefully every cycle
// ============================================================================

#[test]
fn empty_update_cycles_are_safe() {
    let mut app = build_app();

    // Run 100 empty cycles - no observations, no NATS
    run_updates(&mut app, 100);

    assert_eq!(track_count(&mut app), 0, "No tracks should exist with no input");

    // Now inject one and verify it still works
    inject_obs(&mut app, make_obs(37.0, -97.0, "LATE_START"));
    run_updates(&mut app, 10);
    assert!(track_count(&mut app) >= 1, "Pipeline should still work after idle period");
}
