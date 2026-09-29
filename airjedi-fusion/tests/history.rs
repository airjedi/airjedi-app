use std::time::Duration;

use airjedi_core::{
    AltitudeReference, DisplayProvenance, DisplayTrack, HeadingReference, HistoryBreakReason,
    HistoryOperationKind, HistorySessionId, RawOverride, TrackId, TrackStatus,
    VerticalRateReference,
};
use airjedi_fusion::{HistoryConfig, HistoryRecorder};
use chrono::{Duration as ChronoDuration, TimeZone, Utc};

fn display(
    track_id: &TrackId,
    at: chrono::DateTime<Utc>,
    latitude: f64,
    status: TrackStatus,
    predicting: bool,
) -> DisplayTrack {
    DisplayTrack {
        track_id: track_id.clone(),
        icao: "A1B2C3".to_string(),
        callsign: Some("HISTORY".to_string()),
        latitude,
        longitude: -97.0,
        altitude_ft: Some(30_000),
        altitude_reference: AltitudeReference::Barometric,
        position_freshness: None,
        altitude_freshness: None,
        velocity_freshness: None,
        heading: Some(90.0),
        heading_reference: HeadingReference::GroundTrack,
        velocity_kts: Some(100.0),
        airspeed_kts: None,
        vertical_rate: Some(100),
        vertical_rate_reference: VerticalRateReference::FeetPerMinute,
        roll_angle: None,
        track_angle_rate: None,
        squawk: None,
        is_on_ground: Some(false),
        alert: Some(false),
        emergency: Some(false),
        spi: Some(false),
        last_seen: at,
        status,
        position_source: None,
        h_uncertainty_m: Some(10.0),
        predicting,
        filter_type: "EKF".to_string(),
        mode_probabilities: None,
        dominant_mode: None,
        observation_count: 1,
        provenance: DisplayProvenance::default(),
    }
}

fn start() -> chrono::DateTime<Utc> {
    Utc.timestamp_opt(1_700_000_000, 0).single().unwrap()
}

fn recorder(config: HistoryConfig) -> HistoryRecorder {
    HistoryRecorder::with_session(config, HistorySessionId::nil())
}

#[test]
fn accumulates_before_client_connection_and_returns_bounded_preview() {
    let track_id = TrackId::new();
    let start = start();
    let config = HistoryConfig {
        preview_window: Duration::from_secs(5 * 60),
        max_preview_samples: 3,
        ..Default::default()
    };
    let mut recorder = recorder(config);

    for index in 0..5 {
        let at = start + ChronoDuration::seconds(index * 2);
        assert!(recorder.record_display_track(
            &display(
                &track_id,
                at,
                37.0 + index as f64,
                TrackStatus::Confirmed,
                false
            ),
            at,
        ));
    }

    // No client or trail visibility participates in accumulation. The preview
    // is the first read of the recorder after all five authoritative samples.
    let preview = recorder.preview(&track_id, start + ChronoDuration::seconds(8));
    assert_eq!(preview.samples.len(), 3);
    assert!(preview.preview_truncated);
    assert_eq!(preview.coverage.retained_sample_count, 5);
    assert_eq!(
        preview.coverage.sampling_interval,
        ChronoDuration::seconds(2)
    );
    assert_eq!(preview.coverage.first_seen, Some(start));
    assert_eq!(preview.coverage.retained_from, Some(start));
    assert_eq!(
        preview.coverage.retained_to,
        Some(start + ChronoDuration::seconds(8))
    );
    assert_eq!(preview.samples[0].sample_sequence, 3);
    assert_eq!(preview.samples[2].latitude, 41.0);
    assert_eq!(
        preview.samples[2].state_time,
        start + ChronoDuration::seconds(8)
    );
    assert_eq!(preview.session_id, HistorySessionId::nil());
}

#[test]
fn coasting_history_uses_prediction_time_for_each_cadence() {
    let track_id = TrackId::new();
    let start = start();
    let mut recorder = recorder(HistoryConfig {
        sampling_interval: Duration::from_secs(2),
        ..Default::default()
    });

    let display = display(&track_id, start, 37.0, TrackStatus::Coasting, true);
    let first_time = start + ChronoDuration::seconds(2);
    let second_time = first_time + ChronoDuration::seconds(2);
    assert!(recorder.record_display_track_at(&display, first_time, first_time));
    assert!(recorder.record_display_track_at(&display, second_time, second_time));

    let snapshot = recorder.snapshot(&track_id, second_time).unwrap();
    assert_eq!(
        snapshot
            .samples
            .iter()
            .map(|sample| sample.state_time)
            .collect::<Vec<_>>(),
        vec![first_time, second_time]
    );
    assert!(snapshot.samples.iter().all(|sample| sample.estimated));
}

#[test]
fn samples_keep_original_times_and_never_use_interpolation_or_session_time() {
    let track_id = TrackId::new();
    let start = start();
    let mut recorder = recorder(HistoryConfig::default());

    recorder.record_display_track(
        &display(&track_id, start, 37.0, TrackStatus::Confirmed, false),
        start,
    );
    // A one-second call is below the two-second target and must not create a
    // fabricated midpoint sample.
    recorder.record_display_track(
        &display(
            &track_id,
            start + ChronoDuration::seconds(1),
            37.5,
            TrackStatus::Confirmed,
            false,
        ),
        start + ChronoDuration::seconds(1),
    );
    recorder.record_display_track(
        &display(
            &track_id,
            start + ChronoDuration::seconds(2),
            38.0,
            TrackStatus::Confirmed,
            false,
        ),
        start + ChronoDuration::seconds(2),
    );

    let preview = recorder.preview(&track_id, start + ChronoDuration::seconds(2));
    assert_eq!(preview.samples.len(), 2);
    assert_eq!(preview.samples[0].latitude, 37.0);
    assert_eq!(preview.samples[1].latitude, 38.0);
    assert_eq!(preview.samples[0].state_time, start);
    assert_eq!(
        preview.samples[1].state_time,
        start + ChronoDuration::seconds(2)
    );
    assert!(preview
        .samples
        .iter()
        .all(|sample| sample.provenance.position.raw_override == RawOverride::Unavailable));
}

#[test]
fn pruning_reports_actual_coverage_and_deletion_releases_the_lifetime() {
    let track_id = TrackId::new();
    let start = start();
    let mut recorder = recorder(HistoryConfig {
        retention: Duration::from_secs(4),
        max_samples_per_track: 10,
        ..Default::default()
    });

    for index in 0..4 {
        let at = start + ChronoDuration::seconds(index * 2);
        recorder.record_display_track(
            &display(
                &track_id,
                at,
                37.0 + index as f64,
                TrackStatus::Confirmed,
                false,
            ),
            at,
        );
    }
    recorder.prune(start + ChronoDuration::seconds(6));

    let preview = recorder.preview(&track_id, start + ChronoDuration::seconds(6));
    assert_eq!(preview.coverage.first_seen, Some(start));
    assert_eq!(
        preview.coverage.retained_from,
        Some(start + ChronoDuration::seconds(2))
    );
    assert_eq!(preview.coverage.retained_sample_count, 3);
    assert_eq!(
        preview.coverage.truncation_reason,
        Some(airjedi_core::HistoryTruncationReason::Retention)
    );

    assert!(recorder.remove_track(&track_id));
    assert!(!recorder.contains_track(&track_id));
    assert_eq!(recorder.total_sample_count(), 0);
    assert!(recorder.preview(&track_id, start).samples.is_empty());
}

#[test]
fn global_bounds_and_discontinuities_are_active() {
    let first = TrackId::new();
    let second = TrackId::new();
    let start = start();
    let mut recorder = recorder(HistoryConfig {
        max_samples_global: 3,
        discontinuity_gap: Duration::from_secs(5),
        ..Default::default()
    });

    for (track_id, latitude) in [(&first, 37.0), (&second, 47.0)] {
        recorder.record_display_track(
            &display(track_id, start, latitude, TrackStatus::Confirmed, false),
            start,
        );
        recorder.record_display_track(
            &display(
                track_id,
                start + ChronoDuration::seconds(2),
                latitude + 1.0,
                TrackStatus::Confirmed,
                false,
            ),
            start + ChronoDuration::seconds(2),
        );
    }
    assert_eq!(recorder.total_sample_count(), 3);

    let gap_at = start + ChronoDuration::seconds(20);
    recorder.record_display_track(
        &display(&first, gap_at, 60.0, TrackStatus::Coasting, true),
        gap_at,
    );
    let reacquired_at = gap_at + ChronoDuration::seconds(2);
    recorder.record_display_track(
        &display(&first, reacquired_at, 61.0, TrackStatus::Confirmed, false),
        reacquired_at,
    );

    let preview = recorder.preview(&first, reacquired_at);
    let breaks: Vec<HistoryBreakReason> = preview
        .samples
        .iter()
        .filter_map(|sample| sample.break_reason)
        .collect();
    assert!(breaks.contains(&HistoryBreakReason::SamplingGap));
    assert!(breaks.contains(&HistoryBreakReason::Reacquired));
    assert!(preview
        .samples
        .iter()
        .filter(|sample| sample.break_reason.is_some())
        .all(|sample| sample.segment_id > 0));
}

#[test]
fn corrections_preserve_sample_identity_and_advance_revision_with_a_bounded_horizon() {
    let track_id = TrackId::new();
    let start = start();
    let mut recorder = recorder(HistoryConfig::default());
    recorder.record_display_track(
        &display(&track_id, start, 37.0, TrackStatus::Confirmed, false),
        start,
    );
    let before = recorder.history_revision();
    let preview = recorder.preview(&track_id, start);
    let sequence = preview.samples[0].sample_sequence;

    let corrected = display(&track_id, start, 39.0, TrackStatus::Confirmed, false);
    assert!(recorder.correct_sample(
        &track_id,
        sequence,
        airjedi_core::DisplayHistoryInput::from(&corrected),
        start + ChronoDuration::seconds(2),
    ));
    assert!(recorder.history_revision() > before);
    let corrected_preview = recorder.preview(&track_id, start + ChronoDuration::seconds(2));
    assert_eq!(corrected_preview.samples[0].sample_sequence, sequence);
    assert_eq!(corrected_preview.samples[0].latitude, 39.0);

    let revision = recorder.history_revision();
    assert!(!recorder.correct_sample(
        &track_id,
        sequence,
        airjedi_core::DisplayHistoryInput::from(&corrected),
        start + ChronoDuration::seconds(31),
    ));
    assert_eq!(recorder.history_revision(), revision);
}

#[test]
fn snapshot_watermark_exposes_later_append_and_correction_operations() {
    let track_id = TrackId::new();
    let start = start();
    let mut recorder = recorder(HistoryConfig {
        max_operation_log: 8,
        ..Default::default()
    });
    for index in 0..2 {
        let at = start + ChronoDuration::seconds(index * 2);
        recorder.record_display_track(
            &display(
                &track_id,
                at,
                37.0 + index as f64,
                TrackStatus::Confirmed,
                false,
            ),
            at,
        );
    }

    let watermark = recorder.history_revision();
    let snapshot = recorder
        .snapshot(&track_id, start + ChronoDuration::seconds(2))
        .expect("track snapshot");
    assert_eq!(snapshot.revision, watermark);
    assert_eq!(snapshot.sample_cutoff, Some(2));
    assert_eq!(snapshot.samples.len(), 2);

    recorder.record_display_track(
        &display(
            &track_id,
            start + ChronoDuration::seconds(4),
            39.0,
            TrackStatus::Confirmed,
            false,
        ),
        start + ChronoDuration::seconds(4),
    );
    let corrected = display(&track_id, start, 41.0, TrackStatus::Confirmed, false);
    assert!(recorder.correct_sample(
        &track_id,
        1,
        airjedi_core::DisplayHistoryInput::from(&corrected),
        start + ChronoDuration::seconds(4),
    ));

    let operations = recorder
        .operations_since(watermark)
        .expect("watermark remains in operation log");
    assert_eq!(operations.len(), 2);
    assert!(matches!(
        operations[0].kind,
        HistoryOperationKind::Append(ref sample) if sample.sample_sequence == 3
    ));
    assert!(matches!(
        operations[1].kind,
        HistoryOperationKind::Correction(ref sample) if sample.sample_sequence == 1
    ));
}

#[test]
fn exhausted_operation_log_requires_a_fresh_snapshot() {
    let track_id = TrackId::new();
    let start = start();
    let mut recorder = recorder(HistoryConfig {
        max_operation_log: 1,
        ..Default::default()
    });
    recorder.record_display_track(
        &display(&track_id, start, 37.0, TrackStatus::Confirmed, false),
        start,
    );
    let watermark = recorder.history_revision();
    recorder.record_display_track(
        &display(
            &track_id,
            start + ChronoDuration::seconds(2),
            38.0,
            TrackStatus::Confirmed,
            false,
        ),
        start + ChronoDuration::seconds(2),
    );
    recorder.record_display_track(
        &display(
            &track_id,
            start + ChronoDuration::seconds(4),
            39.0,
            TrackStatus::Confirmed,
            false,
        ),
        start + ChronoDuration::seconds(4),
    );

    assert!(recorder.operations_since(watermark).is_none());
}

#[test]
fn equivalent_embedded_and_headless_inputs_have_identical_history() {
    let track_id = TrackId::new();
    let start = start();
    let inputs: Vec<DisplayTrack> = (0..4)
        .map(|index| {
            display(
                &track_id,
                start + ChronoDuration::seconds(index * 2),
                37.0 + index as f64,
                TrackStatus::Confirmed,
                false,
            )
        })
        .collect();
    let config = HistoryConfig {
        max_preview_samples: 32,
        ..Default::default()
    };
    let mut embedded = recorder(config.clone());
    let mut headless = recorder(config);
    for input in &inputs {
        embedded.record_display_track(input, input.last_seen);
        headless.record_display_track(input, input.last_seen);
    }

    assert_eq!(
        embedded.preview(&track_id, start + ChronoDuration::seconds(6)),
        headless.preview(&track_id, start + ChronoDuration::seconds(6))
    );
}

#[test]
fn declared_track_profiles_keep_recorder_bounds_and_report_memory() {
    for profile in [100, 500, 1_000] {
        let start = start();
        let mut recorder = recorder(HistoryConfig {
            max_samples_global: profile,
            ..Default::default()
        });
        for index in 0..profile {
            let track_id = TrackId::new();
            assert!(recorder.record_display_track(
                &display(
                    &track_id,
                    start,
                    37.0 + index as f64 / 100.0,
                    TrackStatus::Confirmed,
                    false,
                ),
                start,
            ));
        }

        let diagnostics = recorder.diagnostics();
        assert_eq!(diagnostics.track_count, profile);
        assert_eq!(diagnostics.retained_samples, profile);
        assert!(diagnostics.retained_sample_bytes > 0);
        assert_eq!(diagnostics.operation_count, profile);
        assert!(diagnostics.operation_bytes > 0);
    }
}

#[test]
fn global_profile_limit_reports_truncation_without_exceeding_the_cap() {
    let start = start();
    let mut recorder = recorder(HistoryConfig {
        max_samples_global: 2,
        ..Default::default()
    });
    for index in 0..3 {
        let track_id = TrackId::new();
        recorder.record_display_track(
            &display(
                &track_id,
                start + ChronoDuration::seconds(index),
                37.0,
                TrackStatus::Confirmed,
                false,
            ),
            start + ChronoDuration::seconds(index),
        );
    }

    let diagnostics = recorder.diagnostics();
    assert_eq!(diagnostics.retained_samples, 2);
    assert!(diagnostics.truncation_events > 0);
}
