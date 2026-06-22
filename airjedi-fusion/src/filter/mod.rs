pub mod ekf;
pub mod oosm;

use crate::prelude_imports::*;
use nalgebra::{DMatrix, DVector};
use std::collections::VecDeque;
use crate::coord;
use crate::sensor::SensorObservation;
use crate::types::{StateVectorType, Timestamp};

#[derive(Debug, Clone)]
pub struct Innovation {
    pub residual: DVector<f64>,
    pub covariance: DMatrix<f64>,
    pub mahalanobis_distance: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub enum FilterResult {
    Updated,
    OutlierRejected { distance: f64 },
    DivergenceDetected,
}

#[derive(Debug, Clone)]
pub struct StateSnapshot {
    pub timestamp: Timestamp,
    pub state: DVector<f64>,
    pub covariance: DMatrix<f64>,
}

#[derive(Debug, Clone)]
pub struct StateHistory {
    pub snapshots: VecDeque<StateSnapshot>,
    max_depth: usize,
}

impl StateHistory {
    #[must_use]
    pub fn new(max_depth: usize) -> Self {
        Self {
            snapshots: VecDeque::with_capacity(max_depth),
            max_depth,
        }
    }

    pub fn push(&mut self, snapshot: StateSnapshot) {
        if self.snapshots.len() >= self.max_depth {
            self.snapshots.pop_front();
        }
        self.snapshots.push_back(snapshot);
    }

    #[must_use]
    pub fn find_before(&self, timestamp: Timestamp) -> Option<&StateSnapshot> {
        self.snapshots
            .iter()
            .rev()
            .find(|s| s.timestamp <= timestamp)
    }

    #[must_use]
    pub fn latest_timestamp(&self) -> Option<Timestamp> {
        self.snapshots.back().map(|s| s.timestamp)
    }
}

#[derive(Debug, Clone)]
pub struct OosmConfig {
    pub max_lag: std::time::Duration,
    pub history_depth: usize,
}

impl Default for OosmConfig {
    fn default() -> Self {
        Self {
            max_lag: std::time::Duration::from_secs(30),
            history_depth: 10,
        }
    }
}

pub trait TrackFilter: Send + Sync {
    fn predict(&mut self, dt: f64);
    fn update(&mut self, observation: &SensorObservation) -> FilterResult;
    fn state_vec(&self) -> DVector<f64>;
    fn covariance_mat(&self) -> DMatrix<f64>;
    fn innovation(&self, observation: &SensorObservation) -> Option<Innovation>;
    fn initialize(&mut self, observation: &SensorObservation);
    fn initialize_from_state(&mut self, state: DVector<f64>, covariance: DMatrix<f64>);
    fn state_history(&self) -> &StateHistory;
    fn zero_velocity(&mut self);
}

#[derive(Debug, Clone)]
pub enum FilterVariant {
    Ekf6Dof(ekf::Ekf6Dof),
}

impl FilterVariant {
    pub fn predict(&mut self, dt: f64) {
        match self {
            Self::Ekf6Dof(f) => f.predict(dt),
        }
    }

    pub fn update(&mut self, observation: &SensorObservation) -> FilterResult {
        match self {
            Self::Ekf6Dof(f) => f.update(observation),
        }
    }

    #[must_use]
    pub fn state_vec(&self) -> DVector<f64> {
        match self {
            Self::Ekf6Dof(f) => TrackFilter::state_vec(f),
        }
    }

    #[must_use]
    pub fn covariance_mat(&self) -> DMatrix<f64> {
        match self {
            Self::Ekf6Dof(f) => TrackFilter::covariance_mat(f),
        }
    }

    #[must_use]
    pub fn innovation(&self, observation: &SensorObservation) -> Option<Innovation> {
        match self {
            Self::Ekf6Dof(f) => f.innovation(observation),
        }
    }

    pub fn initialize(&mut self, observation: &SensorObservation) {
        match self {
            Self::Ekf6Dof(f) => TrackFilter::initialize(f, observation),
        }
    }

    pub fn initialize_from_state(&mut self, state: DVector<f64>, covariance: DMatrix<f64>) {
        match self {
            Self::Ekf6Dof(f) => TrackFilter::initialize_from_state(f, state, covariance),
        }
    }

    #[must_use]
    pub fn state_history(&self) -> &StateHistory {
        match self {
            Self::Ekf6Dof(f) => TrackFilter::state_history(f),
        }
    }

    pub fn zero_velocity(&mut self) {
        match self {
            Self::Ekf6Dof(f) => TrackFilter::zero_velocity(f),
        }
    }
}

#[derive(Component, Debug, Clone)]
pub struct TrackerState {
    pub variant: FilterVariant,
    pub state_type: StateVectorType,
    pub last_update: Option<Timestamp>,
}

impl TrackerState {
    #[must_use]
    pub fn new_6dof(config: ekf::ProcessNoiseConfig) -> Self {
        Self {
            variant: FilterVariant::Ekf6Dof(ekf::Ekf6Dof::new(config)),
            state_type: StateVectorType::Cartesian6Dof,
            last_update: None,
        }
    }

    #[must_use]
    pub fn position_ecef(&self) -> [f64; 3] {
        let s = self.variant.state_vec();
        [s[0], s[1], s[2]]
    }

    #[must_use]
    pub fn velocity_ecef(&self) -> [f64; 3] {
        let s = self.variant.state_vec();
        [s[3], s[4], s[5]]
    }

    #[must_use]
    pub fn position_geodetic(&self) -> (f64, f64, f64) {
        let ecef = self.position_ecef();
        coord::ecef_to_geodetic(&ecef)
    }

    pub fn zero_velocity(&mut self) {
        self.variant.zero_velocity();
    }
}
