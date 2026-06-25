use super::{FilterResult, Innovation, StateHistory, StateSnapshot, TrackFilter};
use crate::coord::{self, CoordinateFrame};
use crate::sensor::{Measurement, SensorObservation};
use nalgebra::{DMatrix, DVector, SMatrix, SVector};

#[derive(Debug, Clone)]
pub struct ProcessNoiseConfig {
    pub position_noise: f64,
    pub velocity_noise: f64,
}

impl Default for ProcessNoiseConfig {
    fn default() -> Self {
        Self {
            position_noise: 1.0,
            velocity_noise: 0.1,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Ekf6Dof {
    x: SVector<f64, 6>,
    p: SMatrix<f64, 6, 6>,
    q_config: ProcessNoiseConfig,
    history: StateHistory,
    gate_threshold: f64,
}

impl Ekf6Dof {
    #[must_use]
    pub fn new(q_config: ProcessNoiseConfig) -> Self {
        Self {
            x: SVector::zeros(),
            p: SMatrix::identity() * 1e6,
            q_config,
            history: StateHistory::new(10),
            gate_threshold: 16.27,
        }
    }

    fn observation_to_ecef(&self, obs: &SensorObservation) -> Option<(DVector<f64>, DMatrix<f64>)> {
        match &obs.measurement {
            Measurement::PositionVelocity3D {
                lat_deg,
                lon_deg,
                alt_m,
                vel_north_mps,
                vel_east_mps,
                vel_down_mps,
                ..
            } => {
                let alt = alt_m.unwrap_or(0.0);
                let ecef = coord::geodetic_to_ecef(*lat_deg, *lon_deg, alt);

                let has_vel =
                    vel_north_mps.is_some() && vel_east_mps.is_some() && vel_down_mps.is_some();
                let z_dim = if has_vel { 6 } else { 3 };
                let mut z = DVector::zeros(z_dim);
                z[0] = ecef[0];
                z[1] = ecef[1];
                z[2] = ecef[2];

                if let (Some(vn), Some(ve), Some(vd)) = (vel_north_mps, vel_east_mps, vel_down_mps)
                {
                    let lat_rad = lat_deg.to_radians();
                    let lon_rad = lon_deg.to_radians();
                    let sin_lat = lat_rad.sin();
                    let cos_lat = lat_rad.cos();
                    let sin_lon = lon_rad.sin();
                    let cos_lon = lon_rad.cos();

                    // NED to ECEF velocity rotation
                    z[3] = -sin_lat * cos_lon * vn - sin_lon * ve - cos_lat * cos_lon * vd;
                    z[4] = -sin_lat * sin_lon * vn + cos_lon * ve - cos_lat * sin_lon * vd;
                    z[5] = cos_lat * vn - sin_lat * vd;
                }

                let r = if obs.covariance.matrix.nrows() >= z_dim
                    && obs.covariance.matrix.ncols() >= z_dim
                {
                    obs.covariance
                        .matrix
                        .view((0, 0), (z_dim, z_dim))
                        .into_owned()
                } else {
                    DMatrix::identity(z_dim, z_dim) * 100.0
                };

                Some((z, r))
            }
            Measurement::PositionVelocity2D {
                lat_deg, lon_deg, ..
            } => {
                let ecef = coord::geodetic_to_ecef(*lat_deg, *lon_deg, 0.0);
                let mut z = DVector::zeros(3);
                z[0] = ecef[0];
                z[1] = ecef[1];
                z[2] = ecef[2];
                let r = if obs.covariance.matrix.nrows() >= 3 {
                    obs.covariance.matrix.view((0, 0), (3, 3)).into_owned()
                } else {
                    DMatrix::identity(3, 3) * 100.0
                };
                Some((z, r))
            }
            Measurement::Spherical {
                range_m,
                azimuth_rad,
                elevation_rad,
                ..
            } => {
                if let CoordinateFrame::SensorSpherical {
                    sensor_lat_deg,
                    sensor_lon_deg,
                    sensor_alt_m,
                } = &obs.sensor_id.coordinate_frame
                {
                    let sensor_ecef =
                        coord::geodetic_to_ecef(*sensor_lat_deg, *sensor_lon_deg, *sensor_alt_m);
                    let el = elevation_rad.unwrap_or(0.0);
                    let target_ecef = coord::spherical_to_ecef(
                        *range_m,
                        *azimuth_rad,
                        el,
                        &sensor_ecef,
                        *sensor_lat_deg,
                        *sensor_lon_deg,
                    );
                    let mut z = DVector::zeros(3);
                    z[0] = target_ecef[0];
                    z[1] = target_ecef[1];
                    z[2] = target_ecef[2];
                    let r = if obs.covariance.matrix.nrows() >= 3 {
                        obs.covariance.matrix.view((0, 0), (3, 3)).into_owned()
                    } else {
                        DMatrix::identity(3, 3) * 500.0
                    };
                    Some((z, r))
                } else {
                    None
                }
            }
            Measurement::FusedEstimate {
                state, covariance, ..
            } => {
                let z_dim = state.len().min(6);
                let z = state.rows(0, z_dim).into_owned();
                let r = covariance.view((0, 0), (z_dim, z_dim)).into_owned();
                Some((z, r))
            }
            _ => None,
        }
    }

    fn build_h_matrix(&self, z_dim: usize) -> DMatrix<f64> {
        let mut h = DMatrix::zeros(z_dim, 6);
        for i in 0..z_dim.min(6) {
            h[(i, i)] = 1.0;
        }
        h
    }

    fn compute_innovation(
        &self,
        z: &DVector<f64>,
        r: &DMatrix<f64>,
    ) -> Option<(DVector<f64>, DMatrix<f64>, f64)> {
        let z_dim = z.len();
        let h = self.build_h_matrix(z_dim);
        let x_dyn = DVector::from_iterator(6, self.x.iter().copied());
        let z_pred = &h * &x_dyn;
        let y = z - &z_pred;

        let p_dyn = DMatrix::from_iterator(6, 6, self.p.iter().copied());
        let s = &h * &p_dyn * h.transpose() + r;

        let s_inv = s.clone().try_inverse()?;
        let maha2 = (&y.transpose() * &s_inv * &y)[(0, 0)];

        Some((y, s, maha2))
    }
}

impl TrackFilter for Ekf6Dof {
    fn predict(&mut self, dt: f64) {
        self.history.push(StateSnapshot {
            timestamp: chrono::Utc::now(),
            state: DVector::from_iterator(6, self.x.iter().copied()),
            covariance: DMatrix::from_iterator(6, 6, self.p.iter().copied()),
        });

        // State transition: constant velocity in ECEF
        self.x[0] += self.x[3] * dt;
        self.x[1] += self.x[4] * dt;
        self.x[2] += self.x[5] * dt;

        let mut f = SMatrix::<f64, 6, 6>::identity();
        f[(0, 3)] = dt;
        f[(1, 4)] = dt;
        f[(2, 5)] = dt;

        let qp = self.q_config.position_noise;
        let qv = self.q_config.velocity_noise;
        let mut q = SMatrix::<f64, 6, 6>::zeros();
        let dt3 = dt * dt * dt / 3.0;
        let dt2 = dt * dt / 2.0;
        for i in 0..3 {
            q[(i, i)] = qp * dt3;
            q[(i, i + 3)] = qp * dt2;
            q[(i + 3, i)] = qp * dt2;
            q[(i + 3, i + 3)] = qv * dt;
        }

        self.p = f * self.p * f.transpose() + q;
    }

    fn update(&mut self, observation: &SensorObservation) -> FilterResult {
        let (z, r) = match self.observation_to_ecef(observation) {
            Some(pair) => pair,
            None => {
                return FilterResult::OutlierRejected {
                    distance: f64::INFINITY,
                }
            }
        };

        let (y, s, maha2) = match self.compute_innovation(&z, &r) {
            Some(t) => t,
            None => return FilterResult::DivergenceDetected,
        };

        if maha2 > self.gate_threshold {
            return FilterResult::OutlierRejected {
                distance: maha2.sqrt(),
            };
        }

        let s_inv = match s.try_inverse() {
            Some(inv) => inv,
            None => return FilterResult::DivergenceDetected,
        };

        let z_dim = z.len();
        let h = self.build_h_matrix(z_dim);
        let p_dyn = DMatrix::from_iterator(6, 6, self.p.iter().copied());
        let k = &p_dyn * h.transpose() * &s_inv;

        let dx = &k * &y;
        for i in 0..6 {
            self.x[i] += dx[i];
        }

        // Joseph form for numerical stability
        let i_kh = DMatrix::identity(6, 6) - &k * &h;
        let p_new = &i_kh * &p_dyn * i_kh.transpose() + &k * &r * k.transpose();
        for i in 0..6 {
            for j in 0..6 {
                self.p[(i, j)] = p_new[(i, j)];
            }
        }

        FilterResult::Updated
    }

    fn state_vec(&self) -> DVector<f64> {
        DVector::from_iterator(6, self.x.iter().copied())
    }

    fn covariance_mat(&self) -> DMatrix<f64> {
        DMatrix::from_iterator(6, 6, self.p.iter().copied())
    }

    fn innovation(&self, observation: &SensorObservation) -> Option<Innovation> {
        let (z, r) = self.observation_to_ecef(observation)?;
        let (y, s, maha2) = self.compute_innovation(&z, &r)?;
        Some(Innovation {
            residual: y,
            covariance: s,
            mahalanobis_distance: maha2.sqrt(),
        })
    }

    fn initialize(&mut self, observation: &SensorObservation) {
        if let Some((z, _r)) = self.observation_to_ecef(observation) {
            for i in 0..z.len().min(6) {
                self.x[i] = z[i];
            }
            self.p = SMatrix::identity() * 1e4;
            self.history = StateHistory::new(10);
        }
    }

    fn initialize_from_state(&mut self, state: DVector<f64>, covariance: DMatrix<f64>) {
        for i in 0..6.min(state.len()) {
            self.x[i] = state[i];
        }
        for i in 0..6 {
            for j in 0..6 {
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
        self.x[3] = 0.0;
        self.x[4] = 0.0;
        self.x[5] = 0.0;
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

    fn make_position_obs(lat: f64, lon: f64, alt: f64) -> SensorObservation {
        SensorObservation {
            sensor_id: SensorId {
                id: "test".to_string(),
                kind: SensorKind::AdsbReceiver,
                tier: FusionTier::Regional,
                coordinate_frame: CoordinateFrame::Wgs84,
            },
            timestamp: Utc::now(),
            receipt_time: Utc::now(),
            target_id: None,
            measurement: Measurement::PositionVelocity3D {
                lat_deg: lat,
                lon_deg: lon,
                alt_m: Some(alt),
                vel_north_mps: Some(100.0),
                vel_east_mps: Some(0.0),
                vel_down_mps: Some(0.0),
                heading_deg: None,
            },
            covariance: ObservationCovariance {
                matrix: DMatrix::identity(6, 6) * 100.0,
            },
            classification_hint: None,
            metadata: ObservationMetadata::default(),
        }
    }

    #[test]
    fn initialize_from_observation() {
        let obs = make_position_obs(37.6872, -97.3301, 10000.0);
        let mut ekf = Ekf6Dof::new(ProcessNoiseConfig::default());
        ekf.initialize(&obs);

        let (lat, lon, alt) = coord::ecef_to_geodetic(&[ekf.x[0], ekf.x[1], ekf.x[2]]);
        assert_relative_eq!(lat, 37.6872, epsilon = 0.001);
        assert_relative_eq!(lon, -97.3301, epsilon = 0.001);
        assert_relative_eq!(alt, 10000.0, epsilon = 10.0);
    }

    #[test]
    fn predict_constant_velocity() {
        let obs = make_position_obs(37.6872, -97.3301, 10000.0);
        let mut ekf = Ekf6Dof::new(ProcessNoiseConfig::default());
        ekf.initialize(&obs);

        let x_before = ekf.x;
        ekf.predict(1.0);
        assert_relative_eq!(ekf.x[0], x_before[0] + x_before[3], epsilon = 1e-6);
        assert_relative_eq!(ekf.x[1], x_before[1] + x_before[4], epsilon = 1e-6);
        assert_relative_eq!(ekf.x[2], x_before[2] + x_before[5], epsilon = 1e-6);
        assert_relative_eq!(ekf.x[3], x_before[3], epsilon = 1e-6);
    }

    #[test]
    fn predict_increases_covariance() {
        let obs = make_position_obs(37.6872, -97.3301, 10000.0);
        let mut ekf = Ekf6Dof::new(ProcessNoiseConfig::default());
        ekf.initialize(&obs);

        let p_before = ekf.p.trace();
        ekf.predict(1.0);
        assert!(ekf.p.trace() > p_before);
    }

    #[test]
    fn update_reduces_covariance() {
        let obs = make_position_obs(37.6872, -97.3301, 10000.0);
        let mut ekf = Ekf6Dof::new(ProcessNoiseConfig::default());
        ekf.initialize(&obs);
        ekf.predict(1.0);

        let p_before_trace = ekf.p.trace();
        let result = ekf.update(&obs);
        assert_eq!(result, FilterResult::Updated);
        assert!(ekf.p.trace() < p_before_trace);
    }

    #[test]
    fn outlier_rejected() {
        let obs = make_position_obs(37.6872, -97.3301, 10000.0);
        let mut ekf = Ekf6Dof::new(ProcessNoiseConfig::default());
        ekf.initialize(&obs);

        let far_obs = make_position_obs(50.0, -50.0, 10000.0);
        let result = ekf.update(&far_obs);
        assert!(matches!(result, FilterResult::OutlierRejected { .. }));
    }

    #[test]
    fn innovation_returns_distance() {
        let obs = make_position_obs(37.6872, -97.3301, 10000.0);
        let mut ekf = Ekf6Dof::new(ProcessNoiseConfig::default());
        ekf.initialize(&obs);

        let innov = ekf.innovation(&obs);
        assert!(innov.is_some());
        let innov = innov.unwrap();
        assert!(innov.mahalanobis_distance >= 0.0);
    }

    #[test]
    fn state_history_records_snapshots() {
        let obs = make_position_obs(37.6872, -97.3301, 10000.0);
        let mut ekf = Ekf6Dof::new(ProcessNoiseConfig::default());
        ekf.initialize(&obs);
        ekf.predict(1.0);
        ekf.predict(1.0);
        assert!(ekf.history.snapshots.len() >= 2);
    }

    #[test]
    fn position_only_observation_works() {
        let obs = SensorObservation {
            sensor_id: SensorId {
                id: "test".to_string(),
                kind: SensorKind::AdsbReceiver,
                tier: FusionTier::Regional,
                coordinate_frame: CoordinateFrame::Wgs84,
            },
            timestamp: Utc::now(),
            receipt_time: Utc::now(),
            target_id: None,
            measurement: Measurement::PositionVelocity3D {
                lat_deg: 37.6872,
                lon_deg: -97.3301,
                alt_m: Some(10000.0),
                vel_north_mps: None,
                vel_east_mps: None,
                vel_down_mps: None,
                heading_deg: None,
            },
            covariance: ObservationCovariance {
                matrix: DMatrix::identity(3, 3) * 100.0,
            },
            classification_hint: None,
            metadata: ObservationMetadata::default(),
        };

        let mut ekf = Ekf6Dof::new(ProcessNoiseConfig::default());
        ekf.initialize(&obs);
        ekf.predict(1.0);
        let result = ekf.update(&obs);
        assert_eq!(result, FilterResult::Updated);
    }

    #[test]
    fn initialize_from_state_roundtrip() {
        let obs = make_position_obs(37.6872, -97.3301, 10000.0);
        let mut ekf = Ekf6Dof::new(ProcessNoiseConfig::default());
        ekf.initialize(&obs);

        let saved_state = ekf.state_vec();
        let saved_cov = ekf.covariance_mat();

        ekf.predict(10.0);
        ekf.initialize_from_state(saved_state.clone(), saved_cov.clone());

        let restored = ekf.state_vec();
        for i in 0..6 {
            assert_relative_eq!(restored[i], saved_state[i], epsilon = 1e-9);
        }
    }
}
