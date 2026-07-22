use super::transition::{ConstantVelocity, CombinedTransitionModel, TransitionModel};
use super::{FilterResult, Innovation, StateHistory, StateSnapshot, TrackFilter};
use crate::coord::{self, CoordinateFrame};
use crate::sensor::{Measurement, SensorObservation};
use nalgebra::{DMatrix, DVector};

#[derive(Debug, Clone)]
pub struct SurfaceConfig {
    pub position_noise: f64,
    pub velocity_noise: f64,
    pub gate_threshold: f64,
}

impl Default for SurfaceConfig {
    fn default() -> Self {
        Self {
            position_noise: 10.0,
            velocity_noise: 1.0,
            gate_threshold: 16.27,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Surface4Dof {
    x: DVector<f64>,
    p: DMatrix<f64>,
    transition_model: Box<dyn TransitionModel>,
    config: SurfaceConfig,
    history: StateHistory,
}

impl Surface4Dof {
    #[must_use]
    pub fn new(config: SurfaceConfig) -> Self {
        let model = CombinedTransitionModel::new(vec![
            Box::new(ConstantVelocity::new(config.position_noise)),
            Box::new(ConstantVelocity::new(config.position_noise)),
        ]);

        Self {
            x: DVector::zeros(4),
            p: DMatrix::identity(4, 4) * 1e6,
            transition_model: Box::new(model),
            config,
            history: StateHistory::new(10),
        }
    }

    fn observation_to_local(&self, obs: &SensorObservation) -> Option<(DVector<f64>, DMatrix<f64>)> {
        match &obs.measurement {
            Measurement::PositionVelocity2D {
                lat_deg,
                lon_deg,
                speed_over_ground_mps,
                course_over_ground_deg,
            } => {
                let ecef = coord::geodetic_to_ecef(*lat_deg, *lon_deg, 0.0);

                let has_vel = speed_over_ground_mps.is_some() && course_over_ground_deg.is_some();
                let z_dim = if has_vel { 4 } else { 2 };
                let mut z = DVector::zeros(z_dim);
                z[0] = ecef[0];
                z[1] = ecef[1];

                if let (Some(sog), Some(cog)) = (speed_over_ground_mps, course_over_ground_deg) {
                    let cog_rad = cog.to_radians();
                    let lat_rad = lat_deg.to_radians();
                    let lon_rad = lon_deg.to_radians();
                    let sin_lat = lat_rad.sin();
                    let cos_lat = lat_rad.cos();
                    let sin_lon = lon_rad.sin();
                    let cos_lon = lon_rad.cos();

                    let vn = sog * cog_rad.cos();
                    let ve = sog * cog_rad.sin();

                    z[2] = -sin_lat * cos_lon * vn - sin_lon * ve;
                    z[3] = -sin_lat * sin_lon * vn + cos_lon * ve;
                }

                let r = if obs.covariance.matrix.nrows() >= z_dim {
                    obs.covariance.matrix.view((0, 0), (z_dim, z_dim)).into_owned()
                } else {
                    DMatrix::identity(z_dim, z_dim) * 50.0
                };

                Some((z, r))
            }
            Measurement::PositionVelocity3D {
                lat_deg,
                lon_deg,
                vel_north_mps,
                vel_east_mps,
                ..
            } => {
                let ecef = coord::geodetic_to_ecef(*lat_deg, *lon_deg, 0.0);

                let has_vel = vel_north_mps.is_some() && vel_east_mps.is_some();
                let z_dim = if has_vel { 4 } else { 2 };
                let mut z = DVector::zeros(z_dim);
                z[0] = ecef[0];
                z[1] = ecef[1];

                if let (Some(vn), Some(ve)) = (vel_north_mps, vel_east_mps) {
                    let lat_rad = lat_deg.to_radians();
                    let lon_rad = lon_deg.to_radians();
                    let sin_lat = lat_rad.sin();
                    let cos_lat = lat_rad.cos();
                    let sin_lon = lon_rad.sin();
                    let cos_lon = lon_rad.cos();

                    z[2] = -sin_lat * cos_lon * vn - sin_lon * ve;
                    z[3] = -sin_lat * sin_lon * vn + cos_lon * ve;
                }

                let r = if obs.covariance.matrix.nrows() >= z_dim {
                    obs.covariance.matrix.view((0, 0), (z_dim, z_dim)).into_owned()
                } else {
                    DMatrix::identity(z_dim, z_dim) * 100.0
                };

                Some((z, r))
            }
            _ => None,
        }
    }

    fn build_h_matrix(&self, z_dim: usize) -> DMatrix<f64> {
        let mut h = DMatrix::zeros(z_dim, 4);
        // State is stacked [x, vx, y, vy]
        // Measurement: position-only [ecef_x, ecef_y] or with velocity [ecef_x, ecef_y, vx, vy]
        if z_dim >= 1 {
            h[(0, 0)] = 1.0; // ecef_x -> state[0] (x)
        }
        if z_dim >= 2 {
            h[(1, 2)] = 1.0; // ecef_y -> state[2] (y)
        }
        if z_dim >= 3 {
            h[(2, 1)] = 1.0; // vx -> state[1] (vx)
        }
        if z_dim >= 4 {
            h[(3, 3)] = 1.0; // vy -> state[3] (vy)
        }
        h
    }
}

impl TrackFilter for Surface4Dof {
    fn predict(&mut self, dt: f64) {
        self.history.push(StateSnapshot {
            timestamp: chrono::Utc::now(),
            state: self.x.clone(),
            covariance: self.p.clone(),
        });

        self.x = self.transition_model.predict_state(&self.x, dt);
        let f = self.transition_model.f_matrix(dt);
        let q = self.transition_model.q_matrix(dt);
        self.p = &f * &self.p * f.transpose() + q;
    }

    fn update(&mut self, observation: &SensorObservation) -> FilterResult {
        let (z, r) = match self.observation_to_local(observation) {
            Some(pair) => pair,
            None => {
                return FilterResult::OutlierRejected {
                    distance: f64::INFINITY,
                }
            }
        };

        let z_dim = z.len();
        let h = self.build_h_matrix(z_dim);
        let z_pred = &h * &self.x;
        let y = &z - &z_pred;

        let s = &h * &self.p * h.transpose() + &r;
        let s_inv = match s.clone().try_inverse() {
            Some(inv) => inv,
            None => return FilterResult::DivergenceDetected,
        };

        let maha2 = (&y.transpose() * &s_inv * &y)[(0, 0)];
        if maha2 > self.config.gate_threshold {
            return FilterResult::OutlierRejected {
                distance: maha2.sqrt(),
            };
        }

        let k = &self.p * h.transpose() * &s_inv;
        self.x += &k * &y;

        let i_kh = DMatrix::identity(4, 4) - &k * &h;
        self.p = &i_kh * &self.p * i_kh.transpose() + &k * &r * k.transpose();

        FilterResult::Updated
    }

    fn state_vec(&self) -> DVector<f64> {
        self.x.clone()
    }

    fn covariance_mat(&self) -> DMatrix<f64> {
        self.p.clone()
    }

    fn innovation(&self, observation: &SensorObservation) -> Option<Innovation> {
        let (z, r) = self.observation_to_local(observation)?;
        let z_dim = z.len();
        let h = self.build_h_matrix(z_dim);
        let z_pred = &h * &self.x;
        let y = &z - &z_pred;
        let s = &h * &self.p * h.transpose() + &r;
        let s_inv = s.clone().try_inverse()?;
        let maha2 = (&y.transpose() * &s_inv * &y)[(0, 0)];

        Some(Innovation {
            residual: y,
            covariance: s,
            mahalanobis_distance: maha2.sqrt(),
        })
    }

    fn initialize(&mut self, observation: &SensorObservation) {
        if let Some((z, _)) = self.observation_to_local(observation) {
            // z = [ecef_x, ecef_y, vx, vy], state = [x, vx, y, vy]
            self.x[0] = z[0]; // ecef_x -> x
            if z.len() >= 2 {
                self.x[2] = z[1]; // ecef_y -> y
            }
            if z.len() >= 3 {
                self.x[1] = z[2]; // vx -> vx
            }
            if z.len() >= 4 {
                self.x[3] = z[3]; // vy -> vy
            }
            self.p = DMatrix::identity(4, 4) * 1e4;
            self.history = StateHistory::new(10);
        }
    }

    fn initialize_from_state(&mut self, state: DVector<f64>, covariance: DMatrix<f64>) {
        for i in 0..4.min(state.len()) {
            self.x[i] = state[i];
        }
        for i in 0..4 {
            for j in 0..4 {
                if i < covariance.nrows() && j < covariance.ncols() {
                    self.p[(i, j)] = covariance[(i, j)];
                }
            }
        }
        self.history = StateHistory::new(10);
    }

    fn state_history(&self) -> &StateHistory {
        &self.history
    }

    fn zero_velocity(&mut self) {
        self.x[2] = 0.0;
        self.x[3] = 0.0;
    }

    fn clone_filter(&self) -> Box<dyn TrackFilter> {
        Box::new(self.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coord::CoordinateFrame;
    use crate::sensor::*;
    use crate::types::*;
    use approx::assert_relative_eq;
    use chrono::Utc;

    fn make_surface_obs(lat: f64, lon: f64, sog: Option<f64>, cog: Option<f64>) -> SensorObservation {
        SensorObservation {
            sensor_id: SensorId {
                id: "ais".to_string(),
                kind: SensorKind::AisReceiver,
                tier: FusionTier::Regional,
                coordinate_frame: CoordinateFrame::Wgs84,
            },
            timestamp: Utc::now(),
            receipt_time: Utc::now(),
            target_id: Some(TargetId {
                domain: TargetDomain::Maritime,
                id: "211234567".to_string(),
                id_type: IdentifierType::Mmsi,
            }),
            measurement: Measurement::PositionVelocity2D {
                lat_deg: lat,
                lon_deg: lon,
                speed_over_ground_mps: sog,
                course_over_ground_deg: cog,
            },
            covariance: ObservationCovariance {
                matrix: DMatrix::identity(4, 4) * 50.0,
            },
            classification_hint: Some(TargetCategory::SurfaceVessel),
            metadata: ObservationMetadata::default(),
        }
    }

    #[test]
    fn initialize_from_ais() {
        let obs = make_surface_obs(36.85, -75.98, Some(7.7), Some(45.0));
        let mut filter = Surface4Dof::new(SurfaceConfig::default());
        filter.initialize(&obs);

        let state = filter.state_vec();
        assert_eq!(state.len(), 4);
        // Stacked layout: [x, vx, y, vy]
        assert!(state[0].abs() > 1e5); // ECEF x position
        assert!(state[2].abs() > 1e5); // ECEF y position
    }

    #[test]
    fn predict_moves_position() {
        let obs = make_surface_obs(36.85, -75.98, Some(7.7), Some(45.0));
        let mut filter = Surface4Dof::new(SurfaceConfig::default());
        filter.initialize(&obs);

        let x_before = filter.state_vec();
        filter.predict(1.0);
        let x_after = filter.state_vec();

        let pos_moved = (x_after[0] - x_before[0]).abs() > 0.0
            || (x_after[1] - x_before[1]).abs() > 0.0;
        assert!(pos_moved || (x_before[2].abs() < 1e-10 && x_before[3].abs() < 1e-10));
    }

    #[test]
    fn update_reduces_covariance() {
        let obs = make_surface_obs(36.85, -75.98, Some(7.7), Some(45.0));
        let mut filter = Surface4Dof::new(SurfaceConfig::default());
        filter.initialize(&obs);
        filter.predict(1.0);

        let p_before = filter.p.trace();
        let result = filter.update(&obs);
        assert_eq!(result, FilterResult::Updated);
        assert!(filter.p.trace() < p_before);
    }

    #[test]
    fn position_only_ais() {
        let obs = make_surface_obs(36.85, -75.98, None, None);
        let mut filter = Surface4Dof::new(SurfaceConfig::default());
        filter.initialize(&obs);
        filter.predict(1.0);
        let result = filter.update(&obs);
        assert_eq!(result, FilterResult::Updated);
    }

    #[test]
    fn clone_preserves_state() {
        let obs = make_surface_obs(36.85, -75.98, Some(7.7), Some(45.0));
        let mut filter = Surface4Dof::new(SurfaceConfig::default());
        filter.initialize(&obs);

        let cloned = filter.clone_filter();
        let orig = filter.state_vec();
        let clone_state = cloned.state_vec();
        for i in 0..4 {
            assert_relative_eq!(orig[i], clone_state[i], epsilon = 1e-12);
        }
    }
}
