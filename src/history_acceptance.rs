//! Deterministic integration acceptance for the agent-owned history path.
//!
//! This stays at the observation-to-client-history seam so it can exercise a
//! full late join without a receiver, UDP timing, or a renderer window. The
//! renderer and chart adapters consume the same client samples at the end of
//! the scenario.

#[cfg(test)]
mod tests {
    use std::collections::HashSet;
    use std::fs;

    use airjedi_core::{
        AltitudeReference, DisplayHistoryInput, DisplayHistorySample, DisplayProvenance,
        DisplayValueSource, FieldFreshness, FieldProvenance, HeadingReference, HistoryBreakReason,
        HistorySessionId, ObservationIdentity, PositionSource, RawOverride, TimeSourceQuality,
        TrackId, TrackStatus, VerticalRateReference,
    };
    use airjedi_fusion::{FusionClock, HistoryConfig, HistoryRecorder};
    use airjedi_net::{
        ClientHistoryStore, HistoryApplyResult, HistoryOperationMessage, HistoryRequest,
        HistoryRequestPriority, HistoryServerMessage, HistorySnapshotChunk,
        HistorySnapshotComplete, HISTORY_CHUNK_SAMPLES,
    };
    use chrono::{DateTime, Duration, TimeZone, Utc};

    use crate::aircraft::history_chart::{build_chart_series, ChartMetric, HistoryChartWindow};
    use crate::aircraft::trails::{
        contiguous_trail_pairs, SessionClock, TrailConfig, TrailHistory, TrailRenderer,
    };
    use crate::recording::{PlaybackState, RecordedAircraftState, RecordedFrame};

    const PRE_CONNECTION_SECONDS: i64 = 20 * 60;
    const RETENTION_BOUNDARY_SECONDS: i64 = 36 * 60;
    const DISCONNECT_SECONDS: i64 = 22 * 60;

    struct AcceptanceClock {
        start: DateTime<Utc>,
        seconds: i64,
        fusion: FusionClock,
    }

    impl AcceptanceClock {
        fn new(start: DateTime<Utc>) -> Self {
            Self {
                start,
                seconds: 0,
                fusion: FusionClock::fixed(start),
            }
        }

        fn advance_to(&mut self, seconds: i64) {
            assert!(seconds >= self.seconds);
            self.fusion.advance_to(
                self.start + Duration::seconds(seconds),
                (seconds - self.seconds) as f64,
            );
            self.seconds = seconds;
        }

        fn now(&self) -> DateTime<Utc> {
            self.fusion.now_utc()
        }
    }

    fn field(
        at: DateTime<Utc>,
        frame_sequence: u64,
        source: DisplayValueSource,
        sensor_id: &str,
    ) -> FieldProvenance {
        FieldProvenance {
            source,
            freshness: FieldFreshness::Fresh,
            raw_override: RawOverride::Applied,
            observation_time: Some(at),
            receipt_time: Some(at + Duration::milliseconds(100)),
            time_source: Some(TimeSourceQuality::ProtocolTimestamp),
            observation_id: Some(ObservationIdentity {
                frame_sequence,
                payload_index: 0,
            }),
            sensor_id: Some(sensor_id.to_string()),
        }
    }

    fn unknown_field() -> FieldProvenance {
        FieldProvenance::unknown()
    }

    fn trajectory_input(
        at: DateTime<Utc>,
        sequence: u64,
        status: TrackStatus,
    ) -> DisplayHistoryInput {
        let seconds = (at - DateTime::<Utc>::UNIX_EPOCH).num_seconds();
        let missing_altitude = seconds % 120 == 0 && seconds > 0;
        let altitude_ft = (!missing_altitude).then_some(30_000 + (seconds % 1_000) as i32);
        let altitude_reference = if missing_altitude {
            AltitudeReference::Unknown
        } else {
            AltitudeReference::Barometric
        };
        let ground_speed_kts = if seconds % 180 == 0 && seconds > 0 {
            Some(0.0)
        } else {
            Some(390.0 + (seconds % 60) as f64)
        };
        DisplayHistoryInput {
            state_time: at,
            latitude: 37.0 + seconds as f64 * 0.0002,
            longitude: -97.0 + seconds as f64 * 0.0001,
            altitude_ft,
            altitude_reference,
            ground_speed_kts,
            heading: Some(90.0),
            heading_reference: HeadingReference::GroundTrack,
            vertical_rate: Some(100),
            vertical_rate_reference: VerticalRateReference::FeetPerMinute,
            position_source: Some(PositionSource::AdsbIcao),
            status,
            estimated: status == TrackStatus::Coasting,
            provenance: DisplayProvenance {
                position: field(at, sequence, DisplayValueSource::RawObservation, "adsb"),
                altitude: if missing_altitude {
                    unknown_field()
                } else {
                    field(at, sequence, DisplayValueSource::FusedEstimate, "fusion")
                },
                ground_speed: field(
                    at,
                    sequence,
                    DisplayValueSource::RawObservation,
                    "adsb-velocity",
                ),
                airspeed: unknown_field(),
                vertical_rate: field(at, sequence, DisplayValueSource::FusedEstimate, "fusion"),
                heading: field(at, sequence, DisplayValueSource::RawObservation, "adsb"),
            },
        }
    }

    fn apply_snapshot(
        store: &mut ClientHistoryStore,
        request: &HistoryRequest,
        snapshot: &airjedi_core::HistorySnapshot,
    ) {
        let chunk_count = snapshot.samples.len().div_ceil(HISTORY_CHUNK_SAMPLES) as u16;
        for (chunk_index, samples) in snapshot.samples.chunks(HISTORY_CHUNK_SAMPLES).enumerate() {
            let message = HistoryServerMessage::SnapshotChunk(HistorySnapshotChunk {
                session_id: snapshot.session_id,
                track_id: snapshot.track_id.clone(),
                request_id: request.request_id,
                server_time: snapshot.server_time,
                sample_cutoff: snapshot.sample_cutoff,
                snapshot_revision: snapshot.revision,
                coverage: snapshot.coverage.clone(),
                chunk_index: chunk_index as u16,
                chunk_count,
                samples: samples.to_vec(),
            });
            assert_eq!(store.apply(&message), HistoryApplyResult::Applied);
        }

        let complete = HistoryServerMessage::SnapshotComplete(HistorySnapshotComplete {
            session_id: snapshot.session_id,
            track_id: snapshot.track_id.clone(),
            request_id: request.request_id,
            server_time: snapshot.server_time,
            sample_cutoff: snapshot.sample_cutoff,
            snapshot_revision: snapshot.revision,
            coverage: snapshot.coverage.clone(),
            chunk_count,
        });
        assert_eq!(store.apply(&complete), HistoryApplyResult::Applied);
    }

    fn connect_selected(
        store: &mut ClientHistoryStore,
        recorder: &HistoryRecorder,
        track_id: &TrackId,
        now: DateTime<Utc>,
    ) -> HistoryRequest {
        let preview = recorder.preview(track_id, now);
        assert_eq!(store.install_preview(&preview), HistoryApplyResult::Applied);
        let request = store
            .prepare_request(track_id, HistoryRequestPriority::Selected)
            .request
            .expect("selected history request");
        let snapshot = recorder
            .snapshot(track_id, now)
            .expect("selected track snapshot");
        apply_snapshot(store, &request, &snapshot);
        request
    }

    fn deliver_operations(
        store: &mut ClientHistoryStore,
        recorder: &HistoryRecorder,
        track_id: &TrackId,
        request: &HistoryRequest,
        cursor: &mut u64,
        server_time: DateTime<Utc>,
    ) {
        let operations = recorder
            .operations_since(*cursor)
            .expect("operation log covers connected client watermark");
        for operation in operations {
            *cursor = (*cursor).max(operation.revision);
            if operation.track_id != *track_id {
                continue;
            }
            let message = HistoryServerMessage::Operation(HistoryOperationMessage {
                session_id: operation.session_id,
                track_id: operation.track_id.clone(),
                request_id: request.request_id,
                server_time,
                sample_cutoff: operation.sample_cutoff,
                operation,
            });
            assert_eq!(store.apply(&message), HistoryApplyResult::Applied);
        }
        *cursor = recorder.history_revision();
    }

    fn assert_normalized_history_equal(
        left: &ClientHistoryStore,
        right: &ClientHistoryStore,
        track_id: &TrackId,
    ) {
        let left_history = left.track(track_id).expect("left history");
        let right_history = right.track(track_id).expect("right history");
        assert_eq!(left_history.session_id, right_history.session_id);
        assert_eq!(
            left_history.history_revision,
            right_history.history_revision
        );
        assert_eq!(left_history.coverage, right_history.coverage);
        assert_eq!(left_history.samples, right_history.samples);

        let sequences: HashSet<u64> = left_history
            .samples
            .iter()
            .map(|sample| sample.sample_sequence)
            .collect();
        assert_eq!(sequences.len(), left_history.samples.len());
        assert!(left_history
            .samples
            .windows(2)
            .all(|samples| samples[0].state_time <= samples[1].state_time));
    }

    fn playback_round_trip(sample: &DisplayHistorySample) {
        let path = std::env::temp_dir().join(format!(
            "airjedi-history-t9-playback-{}.ndjson",
            std::process::id()
        ));
        let frame = RecordedFrame {
            timestamp_ms: 1_000,
            aircraft: vec![RecordedAircraftState {
                icao: "HISTORY".to_string(),
                callsign: Some("T9".to_string()),
                latitude: sample.latitude,
                longitude: sample.longitude,
                altitude: sample.altitude_ft,
                heading: sample.heading,
                velocity: sample.ground_speed_kts,
                vertical_rate: sample.vertical_rate,
                squawk: None,
            }],
        };
        fs::write(
            &path,
            serde_json::to_string(&frame).expect("recording JSON") + "\n",
        )
        .expect("write playback fixture");

        let mut playback = PlaybackState::default();
        playback
            .load(&path)
            .expect("existing recording format loads");
        let loaded = playback.current_frame().expect("loaded playback frame");
        assert_eq!(loaded.aircraft[0].latitude, sample.latitude);
        assert_eq!(loaded.aircraft[0].altitude, sample.altitude_ft);
        assert_eq!(loaded.aircraft[0].velocity, sample.ground_speed_kts);
        playback.stop();
        fs::remove_file(path).expect("remove playback fixture");
    }

    #[test]
    fn full_late_join_experience_converges_through_reconnect_and_retention() {
        let start = Utc.timestamp_opt(1_700_000_000, 0).single().unwrap();
        let mut clock = AcceptanceClock::new(start);
        let session = HistorySessionId::nil();
        let config = HistoryConfig {
            max_preview_samples: 256,
            ..HistoryConfig::default()
        };
        let mut recorder = HistoryRecorder::with_session(config.clone(), session);
        let primary = TrackId::new();
        let background = TrackId::new();

        // The background track gets one stable sample. Its history request is
        // still exercised without introducing unrelated global revisions into
        // the continuously connected selected-track comparison.
        recorder.record_input(
            &background,
            trajectory_input(start, 10_000, TrackStatus::Confirmed),
        );
        recorder.record_input(&primary, trajectory_input(start, 1, TrackStatus::Confirmed));
        recorder.prune(start);

        let mut early = ClientHistoryStore::default();
        let early_request = connect_selected(&mut early, &recorder, &primary, start);
        let mut early_cursor = recorder.history_revision();
        let mut trajectory = Vec::new();

        for seconds in (2..=PRE_CONNECTION_SECONDS).step_by(2) {
            if (482..500).contains(&seconds) {
                continue;
            }
            clock.advance_to(seconds);
            let status = if seconds == 500 {
                TrackStatus::Coasting
            } else {
                TrackStatus::Confirmed
            };
            let input = trajectory_input(clock.now(), seconds as u64, status);
            trajectory.push(input.clone());
            recorder.record_input(&primary, input);
            recorder.prune(clock.now());
            deliver_operations(
                &mut early,
                &recorder,
                &primary,
                &early_request,
                &mut early_cursor,
                clock.now(),
            );
        }

        assert_eq!(clock.seconds, PRE_CONNECTION_SECONDS);
        assert!(recorder.track_sample_count(&primary) > 500);
        assert!(
            recorder.preview(&primary, clock.now()).samples.len()
                < recorder.track_sample_count(&primary)
        );

        // The late client gets the five-minute visible preview first, then the
        // selected full history from the same revision watermark.
        let mut late = ClientHistoryStore::default();
        let late_preview = recorder.preview(&primary, clock.now());
        assert!(late_preview.samples.len() > 100);
        assert!(late_preview.preview_truncated);
        assert_eq!(
            late.install_preview(&late_preview),
            HistoryApplyResult::Applied
        );
        assert_eq!(
            late.track(&primary).expect("late preview").loading,
            airjedi_net::HistoryLoadingState::Preview
        );
        let late_request = late
            .prepare_request(&primary, HistoryRequestPriority::Selected)
            .request
            .expect("late selected history request");
        let late_snapshot = recorder
            .snapshot(&primary, clock.now())
            .expect("late selected snapshot");
        apply_snapshot(&mut late, &late_request, &late_snapshot);

        let initial_late = late.track(&primary).expect("late history");
        assert_eq!(
            initial_late.samples.len(),
            recorder.track_sample_count(&primary)
        );
        assert_eq!(initial_late.samples.first().unwrap().state_time, start);
        assert_eq!(
            initial_late.samples.first().unwrap().altitude_ft,
            Some(30_000)
        );
        assert_eq!(
            initial_late
                .samples
                .first()
                .unwrap()
                .provenance
                .position
                .sensor_id
                .as_deref(),
            Some("adsb")
        );
        assert!(initial_late
            .samples
            .iter()
            .any(|sample| sample.break_reason == Some(HistoryBreakReason::SamplingGap)));
        assert!(initial_late
            .samples
            .iter()
            .any(|sample| sample.break_reason == Some(HistoryBreakReason::Reacquired)));
        let mut late_cursor = recorder.history_revision();

        let background_preview = recorder.preview(&background, clock.now());
        assert_eq!(
            late.install_preview(&background_preview),
            HistoryApplyResult::Applied
        );
        let background_request = late
            .prepare_request(&background, HistoryRequestPriority::Background)
            .request
            .expect("background history request");
        assert_eq!(
            late.active_request_priority(&background),
            Some(HistoryRequestPriority::Background)
        );
        let background_snapshot = recorder
            .snapshot(&background, clock.now())
            .expect("background snapshot");
        apply_snapshot(&mut late, &background_request, &background_snapshot);
        assert_eq!(late.release_completed_background_requests().len(), 1);

        // Continue live ingestion and correct a recent pre-cutoff sample after
        // the late client has completed its initial snapshot.
        let corrected_sequence = recorder
            .snapshot(&primary, clock.now())
            .unwrap()
            .samples
            .last()
            .unwrap()
            .sample_sequence;
        clock.advance_to(PRE_CONNECTION_SECONDS + 2);
        recorder.record_input(
            &primary,
            trajectory_input(clock.now(), clock.seconds as u64, TrackStatus::Confirmed),
        );
        let correction_time = clock.now() - Duration::seconds(2);
        let mut correction = trajectory_input(correction_time, 900_001, TrackStatus::Confirmed);
        correction.latitude += 0.001;
        correction.altitude_ft = Some(31_111);
        correction.ground_speed_kts = Some(444.0);
        correction.provenance.position.sensor_id = Some("late-correction".to_string());
        assert!(recorder.correct_sample(&primary, corrected_sequence, correction, clock.now()));
        recorder.prune(clock.now());
        deliver_operations(
            &mut early,
            &recorder,
            &primary,
            &early_request,
            &mut early_cursor,
            clock.now(),
        );
        deliver_operations(
            &mut late,
            &recorder,
            &primary,
            &late_request,
            &mut late_cursor,
            clock.now(),
        );
        assert_eq!(
            late.track(&primary)
                .unwrap()
                .sample(corrected_sequence)
                .unwrap()
                .altitude_ft,
            Some(31_111)
        );

        // The late client is absent for the interval that crosses the 30-minute
        // retention boundary. The early client continues receiving appends and
        // prune operations throughout the interruption.
        for seconds in ((PRE_CONNECTION_SECONDS + 4)..=RETENTION_BOUNDARY_SECONDS).step_by(2) {
            clock.advance_to(seconds);
            recorder.record_input(
                &primary,
                trajectory_input(clock.now(), seconds as u64, TrackStatus::Confirmed),
            );
            recorder.prune(clock.now());
            deliver_operations(
                &mut early,
                &recorder,
                &primary,
                &early_request,
                &mut early_cursor,
                clock.now(),
            );
            if seconds <= DISCONNECT_SECONDS {
                deliver_operations(
                    &mut late,
                    &recorder,
                    &primary,
                    &late_request,
                    &mut late_cursor,
                    clock.now(),
                );
                if seconds == DISCONNECT_SECONDS {
                    late.invalidate_active_requests();
                }
            }
        }

        let before_reconnect_count = late.track(&primary).unwrap().samples.len();
        let final_snapshot = recorder
            .snapshot(&primary, clock.now())
            .expect("final retained snapshot");
        assert!(before_reconnect_count < final_snapshot.samples.len());
        assert_eq!(
            final_snapshot.coverage.retained_from,
            Some(start + Duration::seconds(RETENTION_BOUNDARY_SECONDS - 30 * 60))
        );
        assert_eq!(
            final_snapshot.coverage.truncation_reason,
            Some(airjedi_core::HistoryTruncationReason::Retention)
        );

        let reconnect_preview = recorder.preview(&primary, clock.now());
        assert_eq!(
            late.install_preview(&reconnect_preview),
            HistoryApplyResult::Applied
        );
        let reconnect_request = late
            .prepare_request(&primary, HistoryRequestPriority::Selected)
            .request
            .expect("reconnect selected history request");
        assert_ne!(reconnect_request.request_id, late_request.request_id);
        apply_snapshot(&mut late, &reconnect_request, &final_snapshot);
        assert_normalized_history_equal(&early, &late, &primary);

        let history = late.track(&primary).expect("reconnected history");
        assert_eq!(
            history.coverage.retained_sample_count,
            history.samples.len()
        );
        assert_eq!(history.coverage.retained_to, Some(clock.now()));
        assert_eq!(history.history_revision, recorder.history_revision());
        assert!(history.samples.iter().any(|sample| {
            sample.altitude_ft.is_none() && sample.ground_speed_kts == Some(0.0)
        }));
        let corrected = history
            .sample(corrected_sequence)
            .expect("recent correction remains inside retention");
        assert_eq!(corrected.altitude_ft, Some(31_111));

        let altitude = build_chart_series(
            &history.samples,
            &history.coverage,
            history.server_time,
            HistoryChartWindow::ThirtyMinutes,
            2_000,
            ChartMetric::Altitude,
        );
        let speed = build_chart_series(
            &history.samples,
            &history.coverage,
            history.server_time,
            HistoryChartWindow::ThirtyMinutes,
            2_000,
            ChartMetric::GroundSpeed,
        );
        assert!(altitude.points.iter().any(|point| point.value.is_none()));
        assert!(speed.points.iter().any(|point| point.value == Some(0.0)));
        assert!(altitude.points.iter().any(|point| point.gap_before));
        assert!(speed.points.iter().any(|point| point.gap_before));
        assert_eq!(altitude.start, history.server_time - Duration::minutes(30));

        let mut trail = TrailHistory::default();
        let trail_clock = SessionClock::default();
        trail.replace_from_samples(&history.samples, history.server_time, &trail_clock);
        let trail_config = TrailConfig::default();
        let pairs_2d =
            contiguous_trail_pairs(&trail.points, &trail_clock, &trail_config, true, false);
        let pairs_3d =
            contiguous_trail_pairs(&trail.points, &trail_clock, &trail_config, true, true);
        assert!(pairs_2d.len() > pairs_3d.len());
        assert!(TrailRenderer::ALL.contains(&TrailRenderer::Gizmo));
        assert!(TrailRenderer::ALL.contains(&TrailRenderer::MeshStrip));
        assert!(pairs_3d.iter().all(|(from, to)| {
            from.altitude.is_some() && to.altitude.is_some() && !to.starts_new_segment(from)
        }));

        // The same timestamped inputs produce identical canonical history in
        // embedded and headless recorder instances.
        let mut embedded = HistoryRecorder::with_session(config.clone(), session);
        let mut headless = HistoryRecorder::with_session(config, session);
        embedded.record_input(&primary, trajectory_input(start, 1, TrackStatus::Confirmed));
        headless.record_input(&primary, trajectory_input(start, 1, TrackStatus::Confirmed));
        for input in trajectory {
            embedded.record_input(&primary, input.clone());
            headless.record_input(&primary, input.clone());
        }
        embedded.prune(clock.now());
        headless.prune(clock.now());
        assert_eq!(
            embedded.preview(&primary, clock.now()),
            headless.preview(&primary, clock.now())
        );

        playback_round_trip(history.samples.last().expect("playback sample"));
    }
}
