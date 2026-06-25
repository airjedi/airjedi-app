use crate::prelude_imports::*;
use crate::types::TrackId;
use std::collections::{HashMap, HashSet};

#[derive(Debug, Resource)]
pub struct SpatialIndex {
    grid: HashMap<(i32, i32), HashSet<TrackId>>,
    track_cells: HashMap<TrackId, (i32, i32)>,
    cell_size_deg: f64,
}

impl SpatialIndex {
    #[must_use]
    pub fn new(cell_size_deg: f64) -> Self {
        Self {
            grid: HashMap::new(),
            track_cells: HashMap::new(),
            cell_size_deg,
        }
    }

    fn cell_for(&self, lat_deg: f64, lon_deg: f64) -> (i32, i32) {
        #[allow(clippy::cast_possible_truncation)]
        let lat_bin = (lat_deg / self.cell_size_deg).floor() as i32;
        #[allow(clippy::cast_possible_truncation)]
        let lon_bin = (lon_deg / self.cell_size_deg).floor() as i32;
        (lat_bin, lon_bin)
    }

    pub fn update_track(&mut self, track_id: &TrackId, lat_deg: f64, lon_deg: f64) {
        let new_cell = self.cell_for(lat_deg, lon_deg);

        if let Some(old_cell) = self.track_cells.get(track_id) {
            if *old_cell == new_cell {
                return;
            }
            if let Some(set) = self.grid.get_mut(old_cell) {
                set.remove(track_id);
                if set.is_empty() {
                    self.grid.remove(old_cell);
                }
            }
        }

        self.grid
            .entry(new_cell)
            .or_default()
            .insert(track_id.clone());
        self.track_cells.insert(track_id.clone(), new_cell);
    }

    pub fn remove_track(&mut self, track_id: &TrackId) {
        if let Some(cell) = self.track_cells.remove(track_id) {
            if let Some(set) = self.grid.get_mut(&cell) {
                set.remove(track_id);
                if set.is_empty() {
                    self.grid.remove(&cell);
                }
            }
        }
    }

    #[must_use]
    pub fn nearby_tracks(&self, lat_deg: f64, lon_deg: f64) -> Vec<TrackId> {
        let (cy, cx) = self.cell_for(lat_deg, lon_deg);
        let mut result = Vec::new();
        for dy in -1..=1 {
            for dx in -1..=1 {
                if let Some(set) = self.grid.get(&(cy + dy, cx + dx)) {
                    result.extend(set.iter().cloned());
                }
            }
        }
        result
    }

    #[must_use]
    pub fn track_count(&self) -> usize {
        self.track_cells.len()
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
        let nearby = idx.nearby_tracks(37.0, -97.0);
        assert!(nearby.is_empty());
        assert_eq!(idx.track_count(), 0);
    }

    #[test]
    fn track_moves_between_cells() {
        let mut idx = SpatialIndex::new(0.5);
        let t1 = TrackId::new();
        idx.update_track(&t1, 37.0, -97.0);
        idx.update_track(&t1, 50.0, -50.0);
        let near_old = idx.nearby_tracks(37.0, -97.0);
        let near_new = idx.nearby_tracks(50.0, -50.0);
        assert!(!near_old.contains(&t1));
        assert!(near_new.contains(&t1));
    }

    #[test]
    fn same_cell_no_churn() {
        let mut idx = SpatialIndex::new(0.5);
        let t1 = TrackId::new();
        idx.update_track(&t1, 37.0, -97.0);
        idx.update_track(&t1, 37.01, -97.01); // same 0.5 deg cell
        assert_eq!(idx.track_count(), 1);
    }
}
