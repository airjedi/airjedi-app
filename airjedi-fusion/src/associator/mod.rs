pub mod gnn;
pub mod spatial_index;

use std::collections::HashMap;
use crate::prelude_imports::*;
use crate::types::TargetCategory;

#[derive(Debug, Clone)]
pub struct Assignment {
    pub observation_idx: usize,
    pub track_idx: usize,
    pub distance: f64,
    pub confidence: f32,
}

#[derive(Debug, Clone)]
pub struct AssociationResult {
    pub assignments: Vec<Assignment>,
    pub unassigned_observations: Vec<usize>,
    pub unassigned_tracks: Vec<usize>,
}

#[derive(Debug, Clone)]
pub struct GateParams {
    pub chi_squared_threshold: f64,
}

impl Default for GateParams {
    fn default() -> Self {
        Self {
            chi_squared_threshold: 16.27,
        }
    }
}

#[derive(Debug, Clone, Resource)]
pub struct AssociatorConfig {
    pub gate_profiles: HashMap<TargetCategory, GateParams>,
    pub default_gate: GateParams,
    pub cooperative_id_boost: f64,
}

impl Default for AssociatorConfig {
    fn default() -> Self {
        let mut gate_profiles = HashMap::new();
        gate_profiles.insert(
            TargetCategory::Person,
            GateParams {
                chi_squared_threshold: 11.34,
            },
        );
        gate_profiles.insert(
            TargetCategory::Missile,
            GateParams {
                chi_squared_threshold: 16.27,
            },
        );

        Self {
            gate_profiles,
            default_gate: GateParams::default(),
            cooperative_id_boost: 0.01,
        }
    }
}

impl AssociatorConfig {
    #[must_use]
    pub fn gate_for(&self, category: &TargetCategory) -> &GateParams {
        self.gate_profiles.get(category).unwrap_or(&self.default_gate)
    }
}
