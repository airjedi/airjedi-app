use airjedi_core::{
    AltitudeReference, DisplayHistorySample, DisplayProvenance, DisplayTrail, HeadingReference,
    HistoryCoverage, HistoryOperation, HistoryOperationKind, HistorySessionId, PositionSource,
    TrackId, TrackStatus, VerticalRateReference,
};
use airjedi_net::{
    ClientHistoryStore, HistoryApplyResult, HistoryOperationMessage, HistoryRejectionReason,
    HistoryRequestPriority, HistoryRequestRejection, HistoryServerMessage, HistorySnapshotChunk,
    HistorySnapshotComplete, HISTORY_MAX_CLIENT_SAMPLES,
};
use chrono::{Duration, TimeZone, Utc};

fn time(seconds: i64) -> chrono::DateTime<Utc> {
    Utc.timestamp_opt(1_700_000_000 + seconds, 0)
        .single()
        .unwrap()
}

fn sample(sequence: u64, seconds: i64, altitude: i32, speed: f64) -> DisplayHistorySample {
    DisplayHistorySample {
        sample_sequence: sequence,
        state_time: time(seconds),
        latitude: 37.0 + seconds as f64 / 100.0,
        longitude: -97.0,
        altitude_ft: Some(altitude),
        altitude_reference: AltitudeReference::Barometric,
        ground_speed_kts: Some(speed),
        heading: Some(90.0),
        heading_reference: HeadingReference::GroundTrack,
        vertical_rate: Some(100),
        vertical_rate_reference: VerticalRateReference::FeetPerMinute,
        position_source: Some(PositionSource::AdsbIcao),
        status: TrackStatus::Confirmed,
        estimated: false,
        provenance: DisplayProvenance::default(),
        segment_id: 0,
        break_reason: None,
    }
}

fn coverage(samples: &[DisplayHistorySample]) -> HistoryCoverage {
    HistoryCoverage {
        first_seen: samples.first().map(|sample| sample.state_time),
        retained_from: samples.first().map(|sample| sample.state_time),
        retained_to: samples.last().map(|sample| sample.state_time),
        retained_sample_count: samples.len(),
        retained_duration: samples
            .first()
            .zip(samples.last())
            .map_or_else(Duration::zero, |(first, last)| {
                last.state_time - first.state_time
            }),
        sampling_interval: Duration::seconds(2),
        truncation_reason: None,
    }
}

fn preview(
    session_id: HistorySessionId,
    track_id: TrackId,
    samples: Vec<DisplayHistorySample>,
    revision: u64,
) -> DisplayTrail {
    let coverage = coverage(&samples);
    DisplayTrail {
        session_id,
        track_id,
        server_time: time(10),
        history_revision: revision,
        coverage,
        sample_sequence_start: samples.first().map(|sample| sample.sample_sequence),
        sample_sequence_end: samples.last().map(|sample| sample.sample_sequence),
        samples,
        preview_window: Duration::minutes(5),
        preview_truncated: false,
    }
}

fn begin(
    store: &mut ClientHistoryStore,
    session_id: HistorySessionId,
    track_id: &TrackId,
    preview_samples: Vec<DisplayHistorySample>,
) -> airjedi_net::HistoryRequest {
    store.install_preview(&preview(session_id, track_id.clone(), preview_samples, 1));
    store
        .prepare_request(track_id, HistoryRequestPriority::Selected)
        .request
        .expect("history request")
}

fn chunk(
    request: &airjedi_net::HistoryRequest,
    samples: Vec<DisplayHistorySample>,
    index: u16,
    count: u16,
    revision: u64,
) -> HistoryServerMessage {
    let snapshot_coverage = if count == 1 {
        coverage(&[sample(1, 0, 30_000, 400.0)])
    } else {
        coverage(&[sample(1, 0, 30_000, 400.0), sample(2, 2, 30_500, 410.0)])
    };
    HistoryServerMessage::SnapshotChunk(HistorySnapshotChunk {
        session_id: request.session_id,
        track_id: request.track_id.clone(),
        request_id: request.request_id,
        server_time: time(10),
        sample_cutoff: Some(3),
        snapshot_revision: revision,
        coverage: snapshot_coverage,
        chunk_index: index,
        chunk_count: count,
        samples,
    })
}

fn complete(
    request: &airjedi_net::HistoryRequest,
    count: u16,
    revision: u64,
) -> HistoryServerMessage {
    let samples = if count == 1 {
        vec![sample(1, 0, 30_000, 400.0)]
    } else {
        vec![sample(1, 0, 30_000, 400.0), sample(2, 2, 30_500, 410.0)]
    };
    HistoryServerMessage::SnapshotComplete(HistorySnapshotComplete {
        session_id: request.session_id,
        track_id: request.track_id.clone(),
        request_id: request.request_id,
        server_time: time(10),
        sample_cutoff: Some(3),
        snapshot_revision: revision,
        coverage: coverage(&samples),
        chunk_count: count,
    })
}

fn operation(
    request: &airjedi_net::HistoryRequest,
    revision: u64,
    kind: HistoryOperationKind,
) -> HistoryServerMessage {
    let samples = vec![sample(1, 0, 32_000, 430.0), sample(2, 2, 30_500, 410.0)];
    HistoryServerMessage::Operation(HistoryOperationMessage {
        session_id: request.session_id,
        track_id: request.track_id.clone(),
        request_id: request.request_id,
        server_time: time(10),
        sample_cutoff: match &kind {
            HistoryOperationKind::Append(_) | HistoryOperationKind::Correction(_) => Some(2),
            HistoryOperationKind::Prune { .. } => Some(2),
            HistoryOperationKind::Remove => None,
        },
        operation: HistoryOperation {
            session_id: request.session_id,
            track_id: request.track_id.clone(),
            sample_cutoff: match &kind {
                HistoryOperationKind::Append(_) | HistoryOperationKind::Correction(_) => Some(2),
                HistoryOperationKind::Prune { .. } => Some(2),
                HistoryOperationKind::Remove => None,
            },
            revision,
            coverage: coverage(&samples),
            kind,
        },
    })
}

#[test]
fn selected_history_includes_pre_connection_altitude_and_ground_speed() {
    let session = HistorySessionId::nil();
    let track = TrackId::new();
    let first = sample(1, 0, 28_000, 380.0);
    let second = sample(2, 2, 29_000, 400.0);
    let mut store = ClientHistoryStore::default();
    let request = begin(&mut store, session, &track, vec![first.clone()]);

    assert_eq!(
        store.apply(&chunk(&request, vec![first, second], 0, 1, 2)),
        HistoryApplyResult::Applied
    );
    assert_eq!(
        store.apply(&complete(&request, 1, 2)),
        HistoryApplyResult::Applied
    );

    let history = store.track(&track).expect("selected history");
    assert_eq!(history.samples.len(), 2);
    assert_eq!(
        history.sample(2).map(|sample| sample.altitude_ft),
        Some(Some(29_000))
    );
    assert_eq!(
        history.sample(2).and_then(|sample| sample.ground_speed_kts),
        Some(400.0)
    );
    assert_eq!(history.loading, airjedi_net::HistoryLoadingState::Complete);
}

#[test]
fn interleaved_chunks_and_live_operations_converge_without_duplicates() {
    let session = HistorySessionId::nil();
    let track = TrackId::new();
    let samples = vec![sample(1, 0, 30_000, 400.0), sample(2, 2, 30_500, 410.0)];
    let mut store = ClientHistoryStore::default();
    let request = begin(&mut store, session, &track, vec![samples[0].clone()]);

    assert_eq!(
        store.apply(&chunk(&request, vec![samples[0].clone()], 0, 2, 2)),
        HistoryApplyResult::Applied
    );
    assert_eq!(
        store.apply(&operation(
            &request,
            3,
            HistoryOperationKind::Correction(sample(1, 0, 32_000, 430.0)),
        )),
        HistoryApplyResult::Applied
    );
    assert_eq!(
        store.apply(&chunk(&request, vec![samples[1].clone()], 1, 2, 2)),
        HistoryApplyResult::Applied
    );
    assert_eq!(
        store.apply(&complete(&request, 2, 2)),
        HistoryApplyResult::Applied
    );
    assert_eq!(
        store.apply(&operation(
            &request,
            3,
            HistoryOperationKind::Correction(sample(1, 0, 32_000, 430.0))
        )),
        HistoryApplyResult::IgnoredStale
    );

    let history = store.track(&track).expect("history installed");
    assert_eq!(history.samples.len(), 2);
    assert_eq!(
        history.sample(1).and_then(|sample| sample.altitude_ft),
        Some(32_000)
    );
    assert_eq!(
        history.sample(1).and_then(|sample| sample.ground_speed_kts),
        Some(430.0)
    );
}

#[test]
fn correction_to_pre_cutoff_sample_survives_snapshot_handoff() {
    let session = HistorySessionId::nil();
    let track = TrackId::new();
    let original = sample(1, 0, 30_000, 400.0);
    let mut store = ClientHistoryStore::default();
    let request = begin(&mut store, session, &track, vec![original.clone()]);
    assert_eq!(
        store.apply(&chunk(&request, vec![original], 0, 1, 5)),
        HistoryApplyResult::Applied
    );
    assert_eq!(
        store.apply(&operation(
            &request,
            6,
            HistoryOperationKind::Correction(sample(1, 0, 33_000, 440.0)),
        )),
        HistoryApplyResult::Applied
    );
    assert_eq!(
        store.apply(&complete(&request, 1, 5)),
        HistoryApplyResult::Applied
    );
    let history = store.track(&track).expect("history installed");
    assert_eq!(
        history.sample(1).and_then(|sample| sample.altitude_ft),
        Some(33_000)
    );
}

#[test]
#[ignore = "known history regression: newer windowed preview drops buffered correction"]
fn newer_windowed_preview_preserves_buffered_correction_outside_preview() {
    let session = HistorySessionId::nil();
    let track = TrackId::new();
    let original = sample(1, 0, 30_000, 400.0);
    let newer_preview_sample = sample(3, 600, 31_000, 420.0);
    let mut store = ClientHistoryStore::default();
    store.install_preview(&preview(
        session,
        track.clone(),
        vec![newer_preview_sample],
        12,
    ));
    let request = store
        .prepare_request(&track, HistoryRequestPriority::Selected)
        .request
        .expect("history request");

    assert_eq!(
        store.apply(&chunk(&request, vec![original], 0, 1, 10)),
        HistoryApplyResult::Applied
    );
    assert_eq!(
        store.apply(&operation(
            &request,
            11,
            HistoryOperationKind::Correction(sample(1, 0, 33_000, 440.0)),
        )),
        HistoryApplyResult::Applied
    );
    assert_eq!(
        store.apply(&complete(&request, 1, 10)),
        HistoryApplyResult::Applied
    );

    let history = store.track(&track).expect("history installed");
    assert_eq!(
        history.sample(1).and_then(|sample| sample.altitude_ft),
        Some(33_000),
        "a newer preview window must not suppress a buffered correction to an older snapshot sample"
    );
}

#[test]
fn retention_during_transfer_is_applied_after_snapshot_installation() {
    let session = HistorySessionId::nil();
    let track = TrackId::new();
    let samples = vec![sample(1, 0, 30_000, 400.0), sample(2, 2, 30_500, 410.0)];
    let mut store = ClientHistoryStore::default();
    let request = begin(&mut store, session, &track, samples.clone());
    assert_eq!(
        store.apply(&chunk(&request, samples, 0, 1, 5)),
        HistoryApplyResult::Applied
    );
    assert_eq!(
        store.apply(&operation(
            &request,
            6,
            HistoryOperationKind::Prune {
                through_sequence: 1
            },
        )),
        HistoryApplyResult::Applied
    );
    assert_eq!(
        store.apply(&complete(&request, 1, 5)),
        HistoryApplyResult::Applied
    );

    let history = store.track(&track).expect("history after retention");
    assert!(history.sample(1).is_none());
    assert!(history.sample(2).is_some());
}

#[test]
fn stale_request_and_session_responses_are_rejected() {
    let session = HistorySessionId::nil();
    let track = TrackId::new();
    let mut store = ClientHistoryStore::default();
    let old_request = begin(
        &mut store,
        session,
        &track,
        vec![sample(1, 0, 30_000, 400.0)],
    );
    let new_request = store
        .prepare_request(&track, HistoryRequestPriority::Background)
        .request
        .expect("superseding request");

    assert_eq!(
        store.apply(&chunk(
            &old_request,
            vec![sample(1, 0, 30_000, 400.0)],
            0,
            1,
            2
        )),
        HistoryApplyResult::Rejected
    );

    let other_session = HistorySessionId::new();
    let mut old_session_message = chunk(&new_request, vec![sample(1, 0, 30_000, 400.0)], 0, 1, 2);
    if let HistoryServerMessage::SnapshotChunk(chunk) = &mut old_session_message {
        chunk.session_id = other_session;
    }
    assert_eq!(
        store.apply(&old_session_message),
        HistoryApplyResult::Rejected
    );
    let other_track = TrackId::new();
    assert_eq!(
        store.install_preview(&preview(other_session, other_track, Vec::new(), 1)),
        HistoryApplyResult::Applied
    );
    assert!(
        store.track(&track).is_none(),
        "session change clears old lifetime"
    );
}

#[test]
fn client_history_cache_remains_bounded() {
    let session = HistorySessionId::nil();
    let track = TrackId::new();
    let samples: Vec<_> = (0..5)
        .map(|index| sample(index + 1, (index * 2) as i64, 30_000 + index as i32, 400.0))
        .collect();
    let mut store = ClientHistoryStore::new(3, 4);
    store.install_preview(&preview(session, track.clone(), samples, 5));
    assert_eq!(store.total_sample_count(), 3);
    assert!(store
        .track(&track)
        .unwrap()
        .samples
        .windows(2)
        .all(|pair| { pair[0].sample_sequence < pair[1].sample_sequence }));
}

#[test]
fn history_received_before_visual_initialization_remains_retryable_and_readable() {
    let session = HistorySessionId::nil();
    let track = TrackId::new();
    let sample = sample(1, 0, 30_000, 400.0);
    let mut store = ClientHistoryStore::default();
    store.install_preview(&preview(session, track.clone(), vec![sample.clone()], 1));
    let request = begin(&mut store, session, &track, vec![sample.clone()]);
    store.apply(&chunk(&request, vec![sample], 0, 1, 2));
    store.apply(&complete(&request, 1, 2));

    let history = store.track(&track).expect("history outlives visual setup");
    assert!(!history.samples.is_empty());
    assert_eq!(history.request_id, Some(request.request_id));
}

#[test]
fn reconnect_keeps_received_history_and_reissues_a_fresh_request() {
    let session = HistorySessionId::nil();
    let track = TrackId::new();
    let first = sample(1, 0, 30_000, 400.0);
    let mut store = ClientHistoryStore::default();
    let request = begin(&mut store, session, &track, vec![first.clone()]);
    store.apply(&chunk(&request, vec![first], 0, 1, 2));
    store.apply(&complete(&request, 1, 2));

    store.invalidate_active_requests();
    assert_eq!(store.active_request_count(), 0);
    assert_eq!(store.track(&track).unwrap().samples.len(), 1);

    let retry = store
        .prepare_request(&track, HistoryRequestPriority::Selected)
        .request
        .expect("reconnect request");
    assert_ne!(retry.request_id, request.request_id);
    assert_eq!(store.diagnostics().retained_samples, 1);
}

#[test]
fn snapshot_gap_rejection_is_retryable_and_counted() {
    let session = HistorySessionId::nil();
    let track = TrackId::new();
    let mut store = ClientHistoryStore::default();
    let request = begin(
        &mut store,
        session,
        &track,
        vec![sample(1, 0, 30_000, 400.0)],
    );

    assert_eq!(
        store.apply(&HistoryServerMessage::Rejected(HistoryRequestRejection {
            session_id: session,
            track_id: track.clone(),
            request_id: request.request_id,
            reason: HistoryRejectionReason::SnapshotGap,
            retryable: true,
        })),
        HistoryApplyResult::Rejected
    );
    assert_eq!(
        store.track(&track).unwrap().loading,
        airjedi_net::HistoryLoadingState::RetryableError
    );
    assert!(store
        .retry(&track, HistoryRequestPriority::Selected)
        .request
        .is_some());
    assert_eq!(store.diagnostics().retries, 1);
    assert_eq!(store.diagnostics().rejected_responses, 1);
}

#[test]
fn duplicate_sample_identity_across_chunks_forces_resync() {
    let session = HistorySessionId::nil();
    let track = TrackId::new();
    let first = sample(1, 0, 30_000, 400.0);
    let mut store = ClientHistoryStore::default();
    let request = begin(&mut store, session, &track, vec![first.clone()]);

    assert_eq!(
        store.apply(&chunk(&request, vec![first.clone()], 0, 2, 2)),
        HistoryApplyResult::Applied
    );
    assert_eq!(
        store.apply(&chunk(&request, vec![first], 1, 2, 2)),
        HistoryApplyResult::Rejected
    );
    assert_eq!(
        store.track(&track).unwrap().loading,
        airjedi_net::HistoryLoadingState::RetryableError
    );
}

#[test]
fn completed_background_requests_are_released_for_fair_cache_fill() {
    let session = HistorySessionId::nil();
    let track = TrackId::new();
    let first = sample(1, 0, 30_000, 400.0);
    let mut store = ClientHistoryStore::default();
    store.install_preview(&preview(session, track.clone(), vec![first.clone()], 1));
    let request = store
        .prepare_request(&track, HistoryRequestPriority::Background)
        .request
        .expect("background request");
    store.apply(&chunk(&request, vec![first], 0, 1, 2));
    store.apply(&complete(&request, 1, 2));

    let cancellations = store.release_completed_background_requests();
    assert_eq!(cancellations.len(), 1);
    assert_eq!(store.active_request_count(), 0);
    assert_eq!(
        store.track(&track).unwrap().loading,
        airjedi_net::HistoryLoadingState::Complete
    );
}

#[test]
fn declared_track_profiles_keep_client_cache_bounded() {
    for profile in [100, 500, 1_000] {
        let session = HistorySessionId::nil();
        let mut store = ClientHistoryStore::new(HISTORY_MAX_CLIENT_SAMPLES, 4);
        for index in 0..profile {
            let track = TrackId::new();
            store.install_preview(&preview(
                session,
                track,
                vec![sample(1, index as i64, 30_000, 400.0)],
                1,
            ));
        }
        assert_eq!(store.track_count(), profile);
        assert!(store.total_sample_count() <= HISTORY_MAX_CLIENT_SAMPLES);
        assert!(store.diagnostics().retained_bytes > 0);
    }
}

#[test]
fn client_request_limit_allows_selected_history_to_displace_background_work() {
    let session = HistorySessionId::nil();
    let tracks: Vec<_> = (0..5).map(|_| TrackId::new()).collect();
    let mut store = ClientHistoryStore::default();
    for track in &tracks {
        store.install_preview(&preview(
            session,
            track.clone(),
            vec![sample(1, 0, 30_000, 400.0)],
            1,
        ));
    }
    for track in tracks.iter().take(4) {
        assert!(store
            .prepare_request(track, HistoryRequestPriority::Background)
            .request
            .is_some());
    }
    assert!(store
        .prepare_request(&tracks[4], HistoryRequestPriority::Background)
        .request
        .is_none());

    let selected = store.prepare_request(&tracks[4], HistoryRequestPriority::Selected);
    assert!(selected.request.is_some());
    assert!(selected.cancel.is_some());
    assert_eq!(store.active_request_count(), 4);
}
