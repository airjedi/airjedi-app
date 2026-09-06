//! The agent-side projection: fused track -> replicated `DisplayTrack`.
//!
//! This is the server half of the design-b boundary. It reuses the exact same
//! [`derive_display_track`] the fat-mode app uses, then maintains one
//! `(Replicated, DisplayTrack)` entity per fused track. `bevy_replicon` copies
//! those entities to every connected thin client, where the renderers read
//! `DisplayTrack` unchanged.

use std::collections::{HashMap, HashSet};

use airjedi_core::{DisplayTrack, PositionSource, TrackId};
use airjedi_fusion::{
    derive_display_track, IdentifierType, Track, TrackQuality, TrackStatus, TrackerState,
};
use bevy::prelude::*;
use bevy_replicon::prelude::Replicated;

/// Maps each fused track's stable [`TrackId`] to the replicated display entity.
#[derive(Resource, Default)]
pub struct TrackEntityMap(pub HashMap<TrackId, Entity>);

/// ICAOs readsb tagged `type:"mlat"`, used to tag `DisplayTrack.position_source`.
#[derive(Resource, Default)]
pub struct MlatSet(pub HashSet<u32>);

fn track_icao_u32(track: &Track) -> Option<u32> {
    track
        .cooperative_ids
        .iter()
        .find(|id| id.id_type == IdentifierType::Icao)
        .and_then(|id| adsb_client::Icao::from_hex(&id.id))
        .map(|i| i.0)
}

/// Upsert a replicated `DisplayTrack` per changed fused track, and despawn
/// display entities whose fused track has gone away (or been marked `Lost`).
pub fn sync_replicated_tracks(
    mut commands: Commands,
    changed: Query<(&Track, &TrackerState, &TrackQuality), Changed<TrackerState>>,
    all_tracks: Query<&Track>,
    mut display: Query<&mut DisplayTrack>,
    mut map: ResMut<TrackEntityMap>,
    mlat: Res<MlatSet>,
) {
    for (track, tracker, quality) in &changed {
        let track_id = track.id.clone();

        // Lost tracks are gone too long to display; drop the replicated entity.
        if quality.status == TrackStatus::Lost {
            if let Some(entity) = map.0.remove(&track_id) {
                commands.entity(entity).despawn();
            }
            continue;
        }

        let position_source = match track_icao_u32(track) {
            Some(icao) if mlat.0.contains(&icao) => Some(PositionSource::Mlat),
            Some(_) => Some(PositionSource::AdsbIcao),
            None => None,
        };

        // Same projection the fat-mode app uses; no raw-observation hint here
        // (fixture replay feeds the filter directly), so this is the pure
        // filter estimate merged with the enrichment source tag.
        let dt = derive_display_track(track, tracker, quality, None, position_source);

        match map.0.get(&track_id).copied() {
            Some(entity) if display.get_mut(entity).is_ok() => {
                if let Ok(mut existing) = display.get_mut(entity) {
                    *existing = dt;
                }
            }
            _ => {
                let entity = commands.spawn((Replicated, dt)).id();
                map.0.insert(track_id, entity);
            }
        }
    }

    // Reap display entities whose fused track no longer exists (track cleanup
    // despawns the fused entity; the changed-query above never sees that).
    let live: HashSet<TrackId> = all_tracks.iter().map(|t| t.id.clone()).collect();
    let stale: Vec<TrackId> = map
        .0
        .keys()
        .filter(|id| !live.contains(*id))
        .cloned()
        .collect();
    for track_id in stale {
        if let Some(entity) = map.0.remove(&track_id) {
            commands.entity(entity).despawn();
        }
    }
}
