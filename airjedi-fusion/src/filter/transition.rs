use nalgebra::{DMatrix, DVector};

pub trait TransitionModel: Send + Sync + std::fmt::Debug {
    fn state_dim(&self) -> usize;
    fn f_matrix(&self, dt: f64) -> DMatrix<f64>;
    fn q_matrix(&self, dt: f64) -> DMatrix<f64>;
    fn predict_state(&self, state: &DVector<f64>, dt: f64) -> DVector<f64>;
    fn is_linear(&self) -> bool;
    fn clone_model(&self) -> Box<dyn TransitionModel>;
}

impl Clone for Box<dyn TransitionModel> {
    fn clone(&self) -> Self {
        self.clone_model()
    }
}

#[derive(Debug, Clone)]
pub struct ConstantVelocity {
    pub noise_intensity: f64,
}

impl ConstantVelocity {
    #[must_use]
    pub fn new(noise_intensity: f64) -> Self {
        Self { noise_intensity }
    }
}

impl Default for ConstantVelocity {
    fn default() -> Self {
        Self {
            noise_intensity: 1.0,
        }
    }
}

impl TransitionModel for ConstantVelocity {
    fn state_dim(&self) -> usize {
        2
    }

    fn f_matrix(&self, dt: f64) -> DMatrix<f64> {
        DMatrix::from_row_slice(2, 2, &[1.0, dt, 0.0, 1.0])
    }

    fn q_matrix(&self, dt: f64) -> DMatrix<f64> {
        let q = self.noise_intensity;
        let dt3 = dt * dt * dt / 3.0;
        let dt2 = dt * dt / 2.0;
        DMatrix::from_row_slice(2, 2, &[q * dt3, q * dt2, q * dt2, q * dt])
    }

    fn predict_state(&self, state: &DVector<f64>, dt: f64) -> DVector<f64> {
        let mut result = state.clone();
        result[0] += state[1] * dt;
        result
    }

    fn is_linear(&self) -> bool {
        true
    }

    fn clone_model(&self) -> Box<dyn TransitionModel> {
        Box::new(self.clone())
    }
}

#[derive(Debug, Clone)]
pub struct ConstantAcceleration {
    pub noise_intensity: f64,
}

impl ConstantAcceleration {
    #[must_use]
    pub fn new(noise_intensity: f64) -> Self {
        Self { noise_intensity }
    }
}

impl TransitionModel for ConstantAcceleration {
    fn state_dim(&self) -> usize {
        3
    }

    fn f_matrix(&self, dt: f64) -> DMatrix<f64> {
        let dt2 = 0.5 * dt * dt;
        DMatrix::from_row_slice(3, 3, &[1.0, dt, dt2, 0.0, 1.0, dt, 0.0, 0.0, 1.0])
    }

    fn q_matrix(&self, dt: f64) -> DMatrix<f64> {
        let q = self.noise_intensity;
        let dt5 = dt.powi(5) / 20.0;
        let dt4 = dt.powi(4) / 8.0;
        let dt3 = dt.powi(3) / 6.0;
        let dt3b = dt.powi(3) / 3.0;
        let dt2 = dt * dt / 2.0;
        DMatrix::from_row_slice(
            3,
            3,
            &[
                q * dt5,
                q * dt4,
                q * dt3,
                q * dt4,
                q * dt3b,
                q * dt2,
                q * dt3,
                q * dt2,
                q * dt,
            ],
        )
    }

    fn predict_state(&self, state: &DVector<f64>, dt: f64) -> DVector<f64> {
        let mut result = state.clone();
        result[0] += state[1] * dt + 0.5 * state[2] * dt * dt;
        result[1] += state[2] * dt;
        result
    }

    fn is_linear(&self) -> bool {
        true
    }

    fn clone_model(&self) -> Box<dyn TransitionModel> {
        Box::new(self.clone())
    }
}

#[derive(Debug, Clone)]
pub struct Singer {
    pub noise_intensity: f64,
    pub reciprocal_time_constant: f64,
}

impl Singer {
    #[must_use]
    pub fn new(noise_intensity: f64, reciprocal_time_constant: f64) -> Self {
        Self {
            noise_intensity,
            reciprocal_time_constant,
        }
    }
}

impl TransitionModel for Singer {
    fn state_dim(&self) -> usize {
        3
    }

    fn f_matrix(&self, dt: f64) -> DMatrix<f64> {
        let alpha = self.reciprocal_time_constant;
        let eat = (-alpha * dt).exp();
        let adt = alpha * dt;
        DMatrix::from_row_slice(
            3,
            3,
            &[
                1.0,
                dt,
                (adt - 1.0 + eat) / (alpha * alpha),
                0.0,
                1.0,
                (1.0 - eat) / alpha,
                0.0,
                0.0,
                eat,
            ],
        )
    }

    fn q_matrix(&self, dt: f64) -> DMatrix<f64> {
        let q = self.noise_intensity;
        let a = self.reciprocal_time_constant;
        let a2 = a * a;
        let a3 = a2 * a;
        let a4 = a3 * a;
        let a5 = a4 * a;
        let eat = (-a * dt).exp();
        let e2at = (-2.0 * a * dt).exp();
        let t = dt;

        let q11 = q
            * (t.powi(5) / 20.0 - t.powi(4) / (4.0 * a)
                + t.powi(3) / (2.0 * a2)
                + (1.0 - e2at) / (2.0 * a5)
                - (2.0 * t * (1.0 - eat)) / a4
                + (2.0 * t * t) / a3
                - t / a4);
        let q12 = q
            * (t.powi(4) / 8.0 - t.powi(3) / (2.0 * a) + t * t / a2 + (1.0 - eat) / a4
                - t / a3
                - (1.0 - e2at) / (2.0 * a4));
        let q13 = q * (t.powi(3) / 6.0 - t * t / (2.0 * a) + t / a2 - (1.0 - eat) / a3);
        let q22 = q
            * (t.powi(3) / 3.0 - t * t / a + t / a2 + (1.0 - e2at) / (2.0 * a3)
                - 2.0 * (1.0 - eat) / a3);
        let q23 = q * (t * t / 2.0 - t / a + (1.0 - eat) / a2 - (1.0 - e2at) / (2.0 * a2));
        let q33 = q * (t - 2.0 * (1.0 - eat) / a + (1.0 - e2at) / (2.0 * a));

        DMatrix::from_row_slice(3, 3, &[q11, q12, q13, q12, q22, q23, q13, q23, q33])
    }

    fn predict_state(&self, state: &DVector<f64>, dt: f64) -> DVector<f64> {
        &self.f_matrix(dt) * state
    }

    fn is_linear(&self) -> bool {
        true
    }

    fn clone_model(&self) -> Box<dyn TransitionModel> {
        Box::new(self.clone())
    }
}

#[derive(Debug, Clone)]
pub struct CoordinatedTurn {
    pub noise_intensity: f64,
}

impl CoordinatedTurn {
    #[must_use]
    pub fn new(noise_intensity: f64) -> Self {
        Self { noise_intensity }
    }
}

impl TransitionModel for CoordinatedTurn {
    fn state_dim(&self) -> usize {
        5
    }

    fn f_matrix(&self, _dt: f64) -> DMatrix<f64> {
        DMatrix::identity(5, 5)
    }

    fn q_matrix(&self, dt: f64) -> DMatrix<f64> {
        let q = self.noise_intensity;
        let dt3 = dt * dt * dt / 3.0;
        let dt2 = dt * dt / 2.0;
        let mut m = DMatrix::zeros(5, 5);
        // x position
        m[(0, 0)] = q * dt3;
        m[(0, 1)] = q * dt2;
        m[(1, 0)] = q * dt2;
        m[(1, 1)] = q * dt;
        // y position
        m[(2, 2)] = q * dt3;
        m[(2, 3)] = q * dt2;
        m[(3, 2)] = q * dt2;
        m[(3, 3)] = q * dt;
        // turn rate noise
        m[(4, 4)] = q * dt;
        m
    }

    fn predict_state(&self, state: &DVector<f64>, dt: f64) -> DVector<f64> {
        let x = state[0];
        let vx = state[1];
        let y = state[2];
        let vy = state[3];
        let omega = state[4];

        let mut result = DVector::zeros(5);

        if omega.abs() < 1e-10 {
            result[0] = x + vx * dt;
            result[1] = vx;
            result[2] = y + vy * dt;
            result[3] = vy;
        } else {
            let sin_wt = (omega * dt).sin();
            let cos_wt = (omega * dt).cos();
            result[0] = x + (vx * sin_wt - vy * (1.0 - cos_wt)) / omega;
            result[1] = vx * cos_wt - vy * sin_wt;
            result[2] = y + (vy * sin_wt + vx * (1.0 - cos_wt)) / omega;
            result[3] = vy * cos_wt + vx * sin_wt;
        }
        result[4] = omega;

        result
    }

    fn is_linear(&self) -> bool {
        false
    }

    fn clone_model(&self) -> Box<dyn TransitionModel> {
        Box::new(self.clone())
    }
}

#[derive(Debug, Clone)]
pub struct ConstantVelocity3D {
    pub position_noise: f64,
    pub velocity_noise: f64,
}

impl ConstantVelocity3D {
    #[must_use]
    pub fn new(position_noise: f64, velocity_noise: f64) -> Self {
        Self {
            position_noise,
            velocity_noise,
        }
    }
}

impl TransitionModel for ConstantVelocity3D {
    fn state_dim(&self) -> usize {
        6
    }

    fn f_matrix(&self, dt: f64) -> DMatrix<f64> {
        let mut f = DMatrix::identity(6, 6);
        f[(0, 3)] = dt;
        f[(1, 4)] = dt;
        f[(2, 5)] = dt;
        f
    }

    fn q_matrix(&self, dt: f64) -> DMatrix<f64> {
        let qp = self.position_noise;
        let qv = self.velocity_noise;
        let dt3 = dt * dt * dt / 3.0;
        let dt2 = dt * dt / 2.0;
        let mut q = DMatrix::zeros(6, 6);
        for i in 0..3 {
            q[(i, i)] = qp * dt3;
            q[(i, i + 3)] = qp * dt2;
            q[(i + 3, i)] = qp * dt2;
            q[(i + 3, i + 3)] = qv * dt;
        }
        q
    }

    fn predict_state(&self, state: &DVector<f64>, dt: f64) -> DVector<f64> {
        let mut result = state.clone();
        result[0] += state[3] * dt;
        result[1] += state[4] * dt;
        result[2] += state[5] * dt;
        result
    }

    fn is_linear(&self) -> bool {
        true
    }

    fn clone_model(&self) -> Box<dyn TransitionModel> {
        Box::new(self.clone())
    }
}

#[derive(Debug, Clone)]
pub struct CombinedTransitionModel {
    pub models: Vec<Box<dyn TransitionModel>>,
}

impl CombinedTransitionModel {
    #[must_use]
    pub fn new(models: Vec<Box<dyn TransitionModel>>) -> Self {
        Self { models }
    }

    #[must_use]
    pub fn total_dim(&self) -> usize {
        self.models.iter().map(|m| m.state_dim()).sum()
    }
}

impl TransitionModel for CombinedTransitionModel {
    fn state_dim(&self) -> usize {
        self.total_dim()
    }

    fn f_matrix(&self, dt: f64) -> DMatrix<f64> {
        let n = self.total_dim();
        let mut combined = DMatrix::zeros(n, n);
        let mut offset = 0;
        for model in &self.models {
            let dim = model.state_dim();
            let f = model.f_matrix(dt);
            for i in 0..dim {
                for j in 0..dim {
                    combined[(offset + i, offset + j)] = f[(i, j)];
                }
            }
            offset += dim;
        }
        combined
    }

    fn q_matrix(&self, dt: f64) -> DMatrix<f64> {
        let n = self.total_dim();
        let mut combined = DMatrix::zeros(n, n);
        let mut offset = 0;
        for model in &self.models {
            let dim = model.state_dim();
            let q = model.q_matrix(dt);
            for i in 0..dim {
                for j in 0..dim {
                    combined[(offset + i, offset + j)] = q[(i, j)];
                }
            }
            offset += dim;
        }
        combined
    }

    fn predict_state(&self, state: &DVector<f64>, dt: f64) -> DVector<f64> {
        let mut result = DVector::zeros(self.total_dim());
        let mut offset = 0;
        for model in &self.models {
            let dim = model.state_dim();
            let sub_state = state.rows(offset, dim).into_owned();
            let predicted = model.predict_state(&sub_state, dt);
            for i in 0..dim {
                result[offset + i] = predicted[i];
            }
            offset += dim;
        }
        result
    }

    fn is_linear(&self) -> bool {
        self.models.iter().all(|m| m.is_linear())
    }

    fn clone_model(&self) -> Box<dyn TransitionModel> {
        Box::new(self.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use approx::assert_relative_eq;

    #[test]
    fn constant_velocity_identity_at_zero_dt() {
        let cv = ConstantVelocity::new(1.0);
        let f = cv.f_matrix(0.0);
        assert_relative_eq!(f[(0, 0)], 1.0, epsilon = 1e-12);
        assert_relative_eq!(f[(0, 1)], 0.0, epsilon = 1e-12);
        assert_relative_eq!(f[(1, 1)], 1.0, epsilon = 1e-12);
    }

    #[test]
    fn constant_velocity_prediction() {
        let cv = ConstantVelocity::new(1.0);
        let state = DVector::from_column_slice(&[100.0, 10.0]);
        let predicted = cv.predict_state(&state, 1.0);
        assert_relative_eq!(predicted[0], 110.0, epsilon = 1e-12);
        assert_relative_eq!(predicted[1], 10.0, epsilon = 1e-12);
    }

    #[test]
    fn constant_velocity_q_positive_definite() {
        let cv = ConstantVelocity::new(1.0);
        let q = cv.q_matrix(1.0);
        let eigenvalues = q.symmetric_eigenvalues();
        for val in eigenvalues.iter() {
            assert!(*val >= 0.0);
        }
    }

    #[test]
    fn constant_acceleration_prediction() {
        let ca = ConstantAcceleration::new(1.0);
        let state = DVector::from_column_slice(&[0.0, 10.0, 2.0]);
        let predicted = ca.predict_state(&state, 1.0);
        assert_relative_eq!(predicted[0], 11.0, epsilon = 1e-12); // 0 + 10*1 + 0.5*2*1^2
        assert_relative_eq!(predicted[1], 12.0, epsilon = 1e-12); // 10 + 2*1
        assert_relative_eq!(predicted[2], 2.0, epsilon = 1e-12);
    }

    #[test]
    fn singer_reduces_to_ca_at_low_alpha() {
        let singer = Singer::new(1.0, 0.001);
        let state = DVector::from_column_slice(&[0.0, 10.0, 2.0]);
        let predicted = singer.predict_state(&state, 1.0);
        assert_relative_eq!(predicted[0], 11.0, epsilon = 0.1);
        assert_relative_eq!(predicted[1], 12.0, epsilon = 0.1);
    }

    #[test]
    fn coordinated_turn_straight_line() {
        let ct = CoordinatedTurn::new(1.0);
        let state = DVector::from_column_slice(&[0.0, 100.0, 0.0, 0.0, 0.0]);
        let predicted = ct.predict_state(&state, 1.0);
        assert_relative_eq!(predicted[0], 100.0, epsilon = 1e-6);
        assert_relative_eq!(predicted[1], 100.0, epsilon = 1e-6);
        assert_relative_eq!(predicted[2], 0.0, epsilon = 1e-6);
    }

    #[test]
    fn coordinated_turn_90_degree() {
        let ct = CoordinatedTurn::new(1.0);
        let omega = std::f64::consts::FRAC_PI_2;
        let state = DVector::from_column_slice(&[0.0, 100.0, 0.0, 0.0, omega]);
        let predicted = ct.predict_state(&state, 1.0);
        // After 90 degrees: vx should become ~0, vy should become ~100
        assert_relative_eq!(predicted[1], 0.0, epsilon = 1e-6);
        assert_relative_eq!(predicted[3], 100.0, epsilon = 1e-6);
    }

    #[test]
    fn cv3d_dimensions() {
        let model = ConstantVelocity3D::new(1.0, 0.1);
        assert_eq!(model.state_dim(), 6);
        let f = model.f_matrix(1.0);
        assert_eq!(f.nrows(), 6);
        assert_eq!(f.ncols(), 6);
    }

    #[test]
    fn cv3d_prediction() {
        let model = ConstantVelocity3D::new(1.0, 0.1);
        // State: [x, y, z, vx, vy, vz]
        let state = DVector::from_column_slice(&[0.0, 0.0, 0.0, 10.0, 20.0, -5.0]);
        let predicted = model.predict_state(&state, 1.0);
        assert_relative_eq!(predicted[0], 10.0, epsilon = 1e-12);
        assert_relative_eq!(predicted[1], 20.0, epsilon = 1e-12);
        assert_relative_eq!(predicted[2], -5.0, epsilon = 1e-12);
    }

    #[test]
    fn cv3d_q_matches_original_ekf() {
        let model = ConstantVelocity3D::new(1.0, 0.1);
        let q = model.q_matrix(1.0);
        // Position-position block
        assert_relative_eq!(q[(0, 0)], 1.0 / 3.0, epsilon = 1e-12);
        // Position-velocity cross
        assert_relative_eq!(q[(0, 3)], 0.5, epsilon = 1e-12);
        // Velocity-velocity block
        assert_relative_eq!(q[(3, 3)], 0.1, epsilon = 1e-12);
    }

    #[test]
    fn combined_stacked_dimensions() {
        let model = CombinedTransitionModel::new(vec![
            Box::new(ConstantVelocity::new(1.0)),
            Box::new(ConstantVelocity::new(1.0)),
        ]);
        assert_eq!(model.state_dim(), 4);
        let f = model.f_matrix(1.0);
        assert_eq!(f.nrows(), 4);
        assert_eq!(f.ncols(), 4);
    }

    #[test]
    fn combined_stacked_prediction() {
        let model = CombinedTransitionModel::new(vec![
            Box::new(ConstantVelocity::new(1.0)),
            Box::new(ConstantVelocity::new(1.0)),
        ]);
        // Stacked: [x, vx, y, vy]
        let state = DVector::from_column_slice(&[0.0, 10.0, 0.0, 20.0]);
        let predicted = model.predict_state(&state, 1.0);
        assert_relative_eq!(predicted[0], 10.0, epsilon = 1e-12);
        assert_relative_eq!(predicted[2], 20.0, epsilon = 1e-12);
    }

    #[test]
    fn combined_block_diagonal_f() {
        let model = CombinedTransitionModel::new(vec![
            Box::new(ConstantVelocity::new(1.0)),
            Box::new(ConstantVelocity::new(1.0)),
            Box::new(ConstantVelocity::new(0.1)),
        ]);
        let f = model.f_matrix(1.0);
        // Off-diagonal blocks should be zero
        assert_relative_eq!(f[(0, 2)], 0.0, epsilon = 1e-12);
        assert_relative_eq!(f[(0, 4)], 0.0, epsilon = 1e-12);
        assert_relative_eq!(f[(2, 0)], 0.0, epsilon = 1e-12);
        // Diagonal blocks should have CV structure
        assert_relative_eq!(f[(0, 1)], 1.0, epsilon = 1e-12);
        assert_relative_eq!(f[(2, 3)], 1.0, epsilon = 1e-12);
    }

    #[test]
    fn combined_is_linear_when_all_linear() {
        let model = CombinedTransitionModel::new(vec![
            Box::new(ConstantVelocity::new(1.0)),
            Box::new(ConstantVelocity::new(1.0)),
        ]);
        assert!(model.is_linear());
    }

    #[test]
    fn combined_not_linear_with_ct() {
        let model = CombinedTransitionModel::new(vec![
            Box::new(CoordinatedTurn::new(1.0)),
            Box::new(ConstantVelocity::new(1.0)),
        ]);
        assert!(!model.is_linear());
    }
}
