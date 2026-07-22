use super::{FilterResult, Innovation, ModeInfo, StateHistory, StateSnapshot, TrackFilter};
use crate::sensor::SensorObservation;
use nalgebra::{DMatrix, DVector};

#[derive(Debug, Clone)]
pub struct ImmFilter {
    filters: Vec<Box<dyn TrackFilter>>,
    mode_probabilities: Vec<f64>,
    transition_matrix: DMatrix<f64>,
    history: StateHistory,
}

impl ImmFilter {
    #[must_use]
    pub fn new(
        filters: Vec<Box<dyn TrackFilter>>,
        transition_matrix: DMatrix<f64>,
    ) -> Self {
        let n = filters.len();
        assert!(n >= 2, "IMM requires at least 2 filters");
        assert_eq!(
            transition_matrix.nrows(),
            n,
            "Transition matrix rows must match filter count"
        );
        assert_eq!(
            transition_matrix.ncols(),
            n,
            "Transition matrix must be square"
        );

        let mode_probabilities = vec![1.0 / n as f64; n];

        Self {
            filters,
            mode_probabilities,
            transition_matrix,
            history: StateHistory::new(10),
        }
    }

    #[must_use]
    pub fn mode_probabilities(&self) -> &[f64] {
        &self.mode_probabilities
    }

    #[must_use]
    pub fn dominant_mode(&self) -> usize {
        self.mode_probabilities
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.partial_cmp(b.1).unwrap())
            .map(|(i, _)| i)
            .unwrap_or(0)
    }

    fn mixing_probabilities(&self) -> DMatrix<f64> {
        let n = self.filters.len();
        let mut mixing = DMatrix::zeros(n, n);

        let mut c_bar = vec![0.0; n];
        for j in 0..n {
            for i in 0..n {
                c_bar[j] += self.transition_matrix[(i, j)] * self.mode_probabilities[i];
            }
        }

        for i in 0..n {
            for j in 0..n {
                if c_bar[j] > 1e-30 {
                    mixing[(i, j)] =
                        self.transition_matrix[(i, j)] * self.mode_probabilities[i] / c_bar[j];
                }
            }
        }

        mixing
    }

    fn mix_states(&mut self, mixing: &DMatrix<f64>) {
        let n = self.filters.len();
        let state_dim = self.filters[0].state_vec().len();

        let states: Vec<DVector<f64>> = self.filters.iter().map(|f| f.state_vec()).collect();
        let covs: Vec<DMatrix<f64>> = self.filters.iter().map(|f| f.covariance_mat()).collect();

        for j in 0..n {
            let mut mixed_state = DVector::zeros(state_dim);
            for i in 0..n {
                mixed_state += mixing[(i, j)] * &states[i];
            }

            let mut mixed_cov = DMatrix::zeros(state_dim, state_dim);
            for i in 0..n {
                let diff = &states[i] - &mixed_state;
                let p_plus_spread = &covs[i] + &diff * diff.transpose();
                mixed_cov += mixing[(i, j)] * p_plus_spread;
            }

            self.filters[j].initialize_from_state(mixed_state, mixed_cov);
        }
    }

    fn update_mode_probabilities(&mut self, likelihoods: &[f64]) {
        let n = self.filters.len();

        let mut c_bar = vec![0.0; n];
        for j in 0..n {
            for i in 0..n {
                c_bar[j] += self.transition_matrix[(i, j)] * self.mode_probabilities[i];
            }
        }

        let mut new_probs = vec![0.0; n];
        let mut sum = 0.0;
        for j in 0..n {
            new_probs[j] = likelihoods[j] * c_bar[j];
            sum += new_probs[j];
        }

        if sum > 1e-300 {
            for p in &mut new_probs {
                *p /= sum;
            }
        } else {
            new_probs = vec![1.0 / n as f64; n];
        }

        self.mode_probabilities = new_probs;
    }

    fn gaussian_likelihood(innovation: &Innovation) -> f64 {
        let d = innovation.residual.len() as f64;
        let s = &innovation.covariance;

        let det = s.determinant();
        if det <= 0.0 {
            return 1e-300;
        }

        let exponent = -0.5 * innovation.mahalanobis_distance * innovation.mahalanobis_distance;
        let norm = (2.0 * std::f64::consts::PI).powf(d / 2.0) * det.sqrt();

        (exponent.exp() / norm).max(1e-300)
    }
}

impl TrackFilter for ImmFilter {
    fn predict(&mut self, dt: f64) {
        self.history.push(StateSnapshot {
            timestamp: chrono::Utc::now(),
            state: self.state_vec(),
            covariance: self.covariance_mat(),
        });

        let mixing = self.mixing_probabilities();
        self.mix_states(&mixing);

        for filter in &mut self.filters {
            filter.predict(dt);
        }
    }

    fn update(&mut self, observation: &SensorObservation) -> FilterResult {
        let n = self.filters.len();
        let mut likelihoods = vec![1e-300; n];
        let mut any_updated = false;

        for (i, filter) in self.filters.iter_mut().enumerate() {
            if let Some(innov) = filter.innovation(observation) {
                likelihoods[i] = Self::gaussian_likelihood(&innov);
            }

            match filter.update(observation) {
                FilterResult::Updated => {
                    any_updated = true;
                }
                FilterResult::OutlierRejected { .. } => {
                    likelihoods[i] = 1e-300;
                }
                FilterResult::DivergenceDetected => {
                    likelihoods[i] = 1e-300;
                }
            }
        }

        self.update_mode_probabilities(&likelihoods);

        if any_updated {
            FilterResult::Updated
        } else {
            FilterResult::OutlierRejected {
                distance: f64::INFINITY,
            }
        }
    }

    fn state_vec(&self) -> DVector<f64> {
        let state_dim = self.filters[0].state_vec().len();
        let mut combined = DVector::zeros(state_dim);
        for (i, filter) in self.filters.iter().enumerate() {
            combined += self.mode_probabilities[i] * filter.state_vec();
        }
        combined
    }

    fn covariance_mat(&self) -> DMatrix<f64> {
        let state_dim = self.filters[0].state_vec().len();
        let combined_state = self.state_vec();
        let mut combined_cov = DMatrix::zeros(state_dim, state_dim);

        for (i, filter) in self.filters.iter().enumerate() {
            let diff = filter.state_vec() - &combined_state;
            let p_plus_spread = filter.covariance_mat() + &diff * diff.transpose();
            combined_cov += self.mode_probabilities[i] * p_plus_spread;
        }

        combined_cov
    }

    fn innovation(&self, observation: &SensorObservation) -> Option<Innovation> {
        self.filters[self.dominant_mode()].innovation(observation)
    }

    fn initialize(&mut self, observation: &SensorObservation) {
        for filter in &mut self.filters {
            filter.initialize(observation);
        }
        let n = self.filters.len();
        self.mode_probabilities = vec![1.0 / n as f64; n];
        self.history = StateHistory::new(10);
    }

    fn initialize_from_state(&mut self, state: DVector<f64>, covariance: DMatrix<f64>) {
        for filter in &mut self.filters {
            filter.initialize_from_state(state.clone(), covariance.clone());
        }
        let n = self.filters.len();
        self.mode_probabilities = vec![1.0 / n as f64; n];
        self.history = StateHistory::new(10);
    }

    fn state_history(&self) -> &StateHistory {
        &self.history
    }

    fn zero_velocity(&mut self) {
        for filter in &mut self.filters {
            filter.zero_velocity();
        }
    }

    fn clone_filter(&self) -> Box<dyn TrackFilter> {
        Box::new(self.clone())
    }

    fn mode_info(&self) -> Option<ModeInfo> {
        Some(ModeInfo {
            probabilities: self.mode_probabilities.clone(),
            dominant_mode: self.dominant_mode(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coord;
    use crate::coord::CoordinateFrame;
    use crate::filter::ekf::{Ekf6Dof, ProcessNoiseConfig};
    use crate::filter::transition::{ConstantVelocity3D, TransitionModel};
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

    fn make_cv_ekf(noise: f64) -> Box<dyn TrackFilter> {
        Box::new(Ekf6Dof::new(ProcessNoiseConfig {
            position_noise: noise,
            velocity_noise: noise * 0.1,
        }))
    }

    fn make_imm() -> ImmFilter {
        let filters: Vec<Box<dyn TrackFilter>> = vec![
            make_cv_ekf(1.0),
            make_cv_ekf(10.0),
        ];

        let transition_matrix = DMatrix::from_row_slice(
            2,
            2,
            &[0.95, 0.05, 0.05, 0.95],
        );

        ImmFilter::new(filters, transition_matrix)
    }

    #[test]
    fn imm_initializes_equally() {
        let imm = make_imm();
        let probs = imm.mode_probabilities();
        assert_relative_eq!(probs[0], 0.5, epsilon = 1e-12);
        assert_relative_eq!(probs[1], 0.5, epsilon = 1e-12);
    }

    #[test]
    fn imm_predict_does_not_crash() {
        let obs = make_position_obs(37.6872, -97.3301, 10000.0);
        let mut imm = make_imm();
        imm.initialize(&obs);
        imm.predict(1.0);

        let state = imm.state_vec();
        assert_eq!(state.len(), 6);
        let (lat, lon, _) = coord::ecef_to_geodetic(&[state[0], state[1], state[2]]);
        assert_relative_eq!(lat, 37.6872, epsilon = 1.0);
        assert_relative_eq!(lon, -97.3301, epsilon = 1.0);
    }

    #[test]
    fn imm_update_shifts_mode_probabilities() {
        let obs = make_position_obs(37.6872, -97.3301, 10000.0);
        let mut imm = make_imm();
        imm.initialize(&obs);
        imm.predict(1.0);

        let result = imm.update(&obs);
        assert_eq!(result, FilterResult::Updated);

        let sum: f64 = imm.mode_probabilities().iter().sum();
        assert_relative_eq!(sum, 1.0, epsilon = 1e-10);
    }

    #[test]
    fn imm_combined_state_is_weighted_average() {
        let obs = make_position_obs(37.6872, -97.3301, 10000.0);
        let mut imm = make_imm();
        imm.initialize(&obs);

        let combined = imm.state_vec();
        let s0 = imm.filters[0].state_vec();
        let s1 = imm.filters[1].state_vec();

        for i in 0..6 {
            let expected =
                imm.mode_probabilities[0] * s0[i] + imm.mode_probabilities[1] * s1[i];
            assert_relative_eq!(combined[i], expected, epsilon = 1e-9);
        }
    }

    #[test]
    fn imm_outlier_rejected() {
        let obs = make_position_obs(37.6872, -97.3301, 10000.0);
        let mut imm = make_imm();
        imm.initialize(&obs);

        let far_obs = make_position_obs(50.0, -50.0, 10000.0);
        let result = imm.update(&far_obs);
        assert!(matches!(result, FilterResult::OutlierRejected { .. }));
    }

    #[test]
    fn imm_clone_preserves_state() {
        let obs = make_position_obs(37.6872, -97.3301, 10000.0);
        let mut imm = make_imm();
        imm.initialize(&obs);
        imm.predict(1.0);

        let cloned = imm.clone_filter();
        let orig = imm.state_vec();
        let clone_state = cloned.state_vec();
        for i in 0..6 {
            assert_relative_eq!(orig[i], clone_state[i], epsilon = 1e-12);
        }
    }

    #[test]
    fn imm_three_models() {
        let filters: Vec<Box<dyn TrackFilter>> = vec![
            make_cv_ekf(0.5),
            make_cv_ekf(5.0),
            make_cv_ekf(50.0),
        ];

        #[rustfmt::skip]
        let tm = DMatrix::from_row_slice(3, 3, &[
            0.90, 0.05, 0.05,
            0.05, 0.90, 0.05,
            0.05, 0.05, 0.90,
        ]);

        let mut imm = ImmFilter::new(filters, tm);
        let obs = make_position_obs(37.6872, -97.3301, 10000.0);
        imm.initialize(&obs);
        imm.predict(1.0);
        let result = imm.update(&obs);
        assert_eq!(result, FilterResult::Updated);
        assert_eq!(imm.mode_probabilities().len(), 3);
    }
}
