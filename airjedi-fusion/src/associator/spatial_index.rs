use crate::prelude_imports::*;
use crate::types::TrackId;
use kiddo::float::kdtree::KdTree;
use std::collections::HashMap;
use std::time::Instant;

type Tree = KdTree<f64, u64, 3, 1024, u32>;

#[derive(Debug, Resource)]
pub struct SpatialIndex {
    tree: Tree,
    track_to_item: HashMap<TrackId, u64>,
    item_to_track: HashMap<u64, TrackId>,
    positions: HashMap<u64, [f64; 3]>,
    next_item: u64,
    search_radius_deg: f64,
    last_rebuild: Instant,
}

const MIN_REBUILD_INTERVAL: std::time::Duration = std::time::Duration::from_secs(30);

impl SpatialIndex {
    #[must_use]
    pub fn new(search_radius_deg: f64) -> Self {
        Self {
            tree: Tree::new(),
            track_to_item: HashMap::new(),
            item_to_track: HashMap::new(),
            positions: HashMap::new(),
            next_item: 0,
            search_radius_deg,
            last_rebuild: Instant::now(),
        }
    }

    pub fn update_track(&mut self, track_id: &TrackId, lat_deg: f64, lon_deg: f64) {
        if let Some(&item_id) = self.track_to_item.get(track_id) {
            if let Some(old_pos) = self.positions.get(&item_id) {
                let dlat = (old_pos[0] - lat_deg).abs();
                let dlon = (old_pos[1] - lon_deg).abs();
                if dlat < 0.001 && dlon < 0.001 {
                    return;
                }
            }
            self.item_to_track.remove(&item_id);
            self.positions.remove(&item_id);
            self.track_to_item.remove(track_id);
        }

        let item_id = self.next_item;
        self.next_item += 1;

        let point = [lat_deg, lon_deg, 0.0];
        self.tree.add(&point, item_id);
        self.track_to_item.insert(track_id.clone(), item_id);
        self.item_to_track.insert(item_id, track_id.clone());
        self.positions.insert(item_id, point);
    }

    pub fn remove_track(&mut self, track_id: &TrackId) {
        if let Some(item_id) = self.track_to_item.remove(track_id) {
            self.item_to_track.remove(&item_id);
            self.positions.remove(&item_id);
        }
    }

    #[must_use]
    pub fn nearby_tracks(&self, lat_deg: f64, lon_deg: f64) -> Vec<TrackId> {
        let query = [lat_deg, lon_deg, 0.0];
        let radius_sq = self.search_radius_deg * self.search_radius_deg;

        self.tree
            .within_unsorted::<kiddo::SquaredEuclidean>(&query, radius_sq)
            .iter()
            .filter_map(|entry| self.item_to_track.get(&entry.item).cloned())
            .collect()
    }

    #[must_use]
    pub fn track_count(&self) -> usize {
        self.track_to_item.len()
    }

    #[must_use]
    pub fn needs_compaction(&self) -> bool {
        let live = self.track_to_item.len() as u64;
        live > 0
            && self.next_item > live * 3
            && self.last_rebuild.elapsed() >= MIN_REBUILD_INTERVAL
    }

    pub fn rebuild(&mut self) {
        let mut new_tree = Tree::new();
        let mut new_item_to_track = HashMap::new();
        let mut new_track_to_item = HashMap::new();
        let mut new_positions = HashMap::new();
        let mut next = 0u64;

        for (track_id, &old_item) in &self.track_to_item {
            if let Some(&pos) = self.positions.get(&old_item) {
                new_tree.add(&pos, next);
                new_track_to_item.insert(track_id.clone(), next);
                new_item_to_track.insert(next, track_id.clone());
                new_positions.insert(next, pos);
                next += 1;
            }
        }

        self.tree = new_tree;
        self.track_to_item = new_track_to_item;
        self.item_to_track = new_item_to_track;
        self.positions = new_positions;
        self.next_item = next;
        self.last_rebuild = Instant::now();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insert_and_find_nearby() {
        let mut idx = SpatialIndex::new(0.5);
        let t1 = TrackId::new();
        let t2 = TrackId::new();
        idx.update_track(&t1, 37.0, -97.0);
        idx.update_track(&t2, 37.1, -97.1);
        let nearby = idx.nearby_tracks(37.05, -97.05);
        assert!(nearby.contains(&t1));
        assert!(nearby.contains(&t2));
    }

    #[test]
    fn distant_track_not_found() {
        let mut idx = SpatialIndex::new(0.5);
        let t1 = TrackId::new();
        let t2 = TrackId::new();
        idx.update_track(&t1, 37.0, -97.0);
        idx.update_track(&t2, 50.0, -50.0);
        let nearby = idx.nearby_tracks(37.0, -97.0);
        assert!(nearby.contains(&t1));
        assert!(!nearby.contains(&t2));
    }

    #[test]
    fn remove_track() {
        let mut idx = SpatialIndex::new(0.5);
        let t1 = TrackId::new();
        idx.update_track(&t1, 37.0, -97.0);
        idx.remove_track(&t1);
        idx.rebuild();
        let nearby = idx.nearby_tracks(37.0, -97.0);
        assert!(nearby.is_empty());
        assert_eq!(idx.track_count(), 0);
    }

    #[test]
    fn track_moves_between_positions() {
        let mut idx = SpatialIndex::new(0.5);
        let t1 = TrackId::new();
        idx.update_track(&t1, 37.0, -97.0);
        idx.update_track(&t1, 50.0, -50.0);
        idx.rebuild();
        let near_old = idx.nearby_tracks(37.0, -97.0);
        let near_new = idx.nearby_tracks(50.0, -50.0);
        assert!(!near_old.contains(&t1));
        assert!(near_new.contains(&t1));
    }

    #[test]
    fn small_move_no_churn() {
        let mut idx = SpatialIndex::new(0.5);
        let t1 = TrackId::new();
        idx.update_track(&t1, 37.0, -97.0);
        idx.update_track(&t1, 37.0001, -97.0001);
        assert_eq!(idx.track_count(), 1);
    }

    #[test]
    fn rebuild_compacts() {
        let mut idx = SpatialIndex::new(0.5);
        let t1 = TrackId::new();
        let t2 = TrackId::new();
        idx.update_track(&t1, 37.0, -97.0);
        idx.update_track(&t2, 37.1, -97.1);
        idx.remove_track(&t1);
        idx.rebuild();
        assert_eq!(idx.track_count(), 1);
        let nearby = idx.nearby_tracks(37.1, -97.1);
        assert!(nearby.contains(&t2));
    }
}
