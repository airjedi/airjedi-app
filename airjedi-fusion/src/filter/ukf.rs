use super::transition::TransitionModel;
use super::{FilterResult, Innovation, StateHistory, StateSnapshot, TrackFilter};
use crate::coord::{self, CoordinateFrame};
use crate::sensor::{Measurement, SensorObservation};
use nalgebra::{DMatrix, DVector};

#[derive(Debug, Clone)]
pub struct UkfConfig {
    pub alpha: f64,
    pub beta: f64,
    pub kappa: f64,
    pub gate_threshold: f64,
}

impl Default for UkfConfig {
    fn default() -> Self {
        Self {
            alpha: 1e-3,
            beta: 2.0,
            kappa: 0.0,
            gate_threshold: 16.27,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Ukf {
    x: DVector<f64>,
    p: DMatrix<f64>,
    transition_model: Box<dyn TransitionModel>,
    config: UkfConfig,
    history: StateHistory,
}

impl Ukf {
    #[must_use]
    pub fn new(
        state_dim: usize,
        transition_model: Box<dyn TransitionModel>,
        config: UkfConfig,
    ) -> Self {
        assert_eq!(
            transition_model.state_dim(),
            state_dim,
            "Transition model dimension must match state dimension"
        );
        Self {
            x: DVector::zeros(state_dim),
            p: DMatrix::identity(state_dim, state_dim) * 1e6,
            transition_model,
            config,
            history: StateHistory::new(10),
        }
    }

    fn state_dim(&self) -> usize {
        self.x.len()
    }

    fn sigma_weights(&self) -> (Vec<f64>, Vec<f64>) {
        let n = self.state_dim() as f64;
        let alpha = self.config.alpha;
        let beta = self.config.beta;
        let kappa = self.config.kappa;
        let lambda = alpha * alpha * (n + kappa) - n;
        let count = 2 * self.state_dim() + 1;

        let mut wm = vec![0.0; count];
        let mut wc = vec![0.0; count];

        wm[0] = lambda / (n + lambda);
        wc[0] = lambda / (n + lambda) + (1.0 - alpha * alpha + beta);

        let w = 1.0 / (2.0 * (n + lambda));
        for i in 1..count {
            wm[i] = w;
            wc[i] = w;
        }

        (wm, wc)
    }

    fn generate_sigma_points(&self) -> Option<Vec<DVector<f64>>> {
        let n = self.state_dim();
        let alpha = self.config.alpha;
        let kappa = self.config.kappa;
        let lambda = alpha * alpha * (n as f64 + kappa) - n as f64;
        let scale = (n as f64 + lambda).sqrt();

        let chol = self.p.clone().cholesky()?;
        let l = chol.l();

        let mut points = Vec::with_capacity(2 * n + 1);
        points.push(self.x.clone());

        for i in 0..n {
            let col = l.column(i) * scale;
            points.push(&self.x + &col);
            points.push(&self.x - &col);
        }

        Some(points)
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
                let z_dim = state.len().min(self.state_dim());
                let z = state.rows(0, z_dim).into_owned();
                let r = covariance.view((0, 0), (z_dim, z_dim)).into_owned();
                Some((z, r))
            }
            _ => None,
        }
    }

    fn h_function(&self, state: &DVector<f64>, z_dim: usize) -> DVector<f64> {
        state.rows(0, z_dim.min(state.len())).into_owned()
    }
}

impl TrackFilter for Ukf {
    fn predict(&mut self, dt: f64) {
        self.history.push(StateSnapshot {
            timestamp: chrono::Utc::now(),
            state: self.x.clone(),
            covariance: self.p.clone(),
        });

        let sigma_points = match self.generate_sigma_points() {
            Some(pts) => pts,
            None => {
                self.x = self.transition_model.predict_state(&self.x, dt);
                let f = self.transition_model.f_matrix(dt);
                let q = self.transition_model.q_matrix(dt);
                self.p = &f * &self.p * f.transpose() + q;
                return;
            }
        };

        let (wm, wc) = self.sigma_weights();

        let propagated: Vec<DVector<f64>> = sigma_points
            .iter()
            .map(|sp| self.transition_model.predict_state(sp, dt))
            .collect();

        let n = self.state_dim();
        let mut x_pred = DVector::zeros(n);
        for (i, sp) in propagated.iter().enumerate() {
            x_pred += wm[i] * sp;
        }

        let mut p_pred = self.transition_model.q_matrix(dt);
        for (i, sp) in propagated.iter().enumerate() {
            let diff = sp - &x_pred;
            p_pred += wc[i] * &diff * diff.transpose();
        }

        self.x = x_pred;
        self.p = p_pred;
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

        let z_dim = z.len();

        let sigma_points = match self.generate_sigma_points() {
            Some(pts) => pts,
            None => return FilterResult::DivergenceDetected,
        };

        let (wm, wc) = self.sigma_weights();

        let z_sigmas: Vec<DVector<f64>> = sigma_points
            .iter()
            .map(|sp| self.h_function(sp, z_dim))
            .collect();

        let mut z_pred = DVector::zeros(z_dim);
        for (i, zs) in z_sigmas.iter().enumerate() {
            z_pred += wm[i] * zs;
        }

        let mut s = r.clone();
        for (i, zs) in z_sigmas.iter().enumerate() {
            let diff = zs - &z_pred;
            s += wc[i] * &diff * diff.transpose();
        }

        let s_inv = match s.clone().try_inverse() {
            Some(inv) => inv,
            None => return FilterResult::DivergenceDetected,
        };

        let y = &z - &z_pred;
        let maha2 = (&y.transpose() * &s_inv * &y)[(0, 0)];

        if maha2 > self.config.gate_threshold {
            return FilterResult::OutlierRejected {
                distance: maha2.sqrt(),
            };
        }

        let n = self.state_dim();
        let mut pxz = DMatrix::zeros(n, z_dim);
        for (i, (sp, zs)) in sigma_points.iter().zip(z_sigmas.iter()).enumerate() {
            let dx = sp - &self.x;
            let dz = zs - &z_pred;
            pxz += wc[i] * &dx * dz.transpose();
        }

        let k = &pxz * &s_inv;
        self.x += &k * &y;

        // Joseph form for numerical stability
        let i_kh = DMatrix::identity(n, n) - &k * self.h_matrix(z_dim).transpose().transpose();
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
        let (z, r) = self.observation_to_ecef(observation)?;
        let z_dim = z.len();
        let z_pred = self.h_function(&self.x, z_dim);
        let y = &z - &z_pred;

        let h = self.h_matrix(z_dim);
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
        if let Some((z, _r)) = self.observation_to_ecef(observation) {
            let n = self.state_dim();
            for i in 0..z.len().min(n) {
                self.x[i] = z[i];
            }
            self.p = DMatrix::identity(n, n) * 1e4;
            self.history = StateHistory::new(10);
        }
    }

    fn initialize_from_state(&mut self, state: DVector<f64>, covariance: DMatrix<f64>) {
        let n = self.state_dim();
        for i in 0..n.min(state.len()) {
            self.x[i] = state[i];
        }
        for i in 0..n {
            for j in 0..n {
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
        let n = self.state_dim();
        for i in n / 2..n {
            self.x[i] = 0.0;
        }
    }

    fn clone_filter(&self) -> Box<dyn TrackFilter> {
        Box::new(self.clone())
    }
}

impl Ukf {
    fn h_matrix(&self, z_dim: usize) -> DMatrix<f64> {
        let n = self.state_dim();
        let mut h = DMatrix::zeros(z_dim, n);
        for i in 0..z_dim.min(n) {
            h[(i, i)] = 1.0;
        }
        h
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coord::CoordinateFrame;
    use crate::filter::transition::ConstantVelocity3D;
    use crate::sensor::*;
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

    fn make_ukf() -> Ukf {
        let model = Box::new(ConstantVelocity3D::new(1.0, 0.1));
        Ukf::new(6, model, UkfConfig::default())
    }

    #[test]
    fn initialize_from_observation() {
        let obs = make_position_obs(37.6872, -97.3301, 10000.0);
        let mut ukf = make_ukf();
        ukf.initialize(&obs);

        let (lat, lon, alt) = coord::ecef_to_geodetic(&[ukf.x[0], ukf.x[1], ukf.x[2]]);
        assert_relative_eq!(lat, 37.6872, epsilon = 0.001);
        assert_relative_eq!(lon, -97.3301, epsilon = 0.001);
        assert_relative_eq!(alt, 10000.0, epsilon = 10.0);
    }

    #[test]
    fn predict_increases_covariance() {
        let obs = make_position_obs(37.6872, -97.3301, 10000.0);
        let mut ukf = make_ukf();
        ukf.initialize(&obs);

        let p_before = ukf.p.trace();
        ukf.predict(1.0);
        assert!(ukf.p.trace() > p_before);
    }

    #[test]
    fn update_reduces_covariance() {
        let obs = make_position_obs(37.6872, -97.3301, 10000.0);
        let mut ukf = make_ukf();
        ukf.initialize(&obs);
        ukf.predict(1.0);

        let p_before = ukf.p.trace();
        let result = ukf.update(&obs);
        assert_eq!(result, FilterResult::Updated);
        assert!(ukf.p.trace() < p_before);
    }

    #[test]
    fn outlier_rejected() {
        let obs = make_position_obs(37.6872, -97.3301, 10000.0);
        let mut ukf = make_ukf();
        ukf.initialize(&obs);

        let far_obs = make_position_obs(50.0, -50.0, 10000.0);
        let result = ukf.update(&far_obs);
        assert!(matches!(result, FilterResult::OutlierRejected { .. }));
    }

    #[test]
    fn innovation_computes() {
        let obs = make_position_obs(37.6872, -97.3301, 10000.0);
        let mut ukf = make_ukf();
        ukf.initialize(&obs);

        let innov = ukf.innovation(&obs);
        assert!(innov.is_some());
        assert!(innov.unwrap().mahalanobis_distance >= 0.0);
    }

    #[test]
    fn clone_roundtrip() {
        let obs = make_position_obs(37.6872, -97.3301, 10000.0);
        let mut ukf = make_ukf();
        ukf.initialize(&obs);
        ukf.predict(1.0);

        let cloned = ukf.clone_filter();
        let orig_state = ukf.state_vec();
        let clone_state = cloned.state_vec();
        for i in 0..6 {
            assert_relative_eq!(orig_state[i], clone_state[i], epsilon = 1e-12);
        }
    }
}
