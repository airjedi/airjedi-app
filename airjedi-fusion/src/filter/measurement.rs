use nalgebra::{DMatrix, DVector};

pub trait MeasurementModel: Send + Sync + std::fmt::Debug {
    fn measurement_dim(&self) -> usize;
    fn state_dim(&self) -> usize;
    fn h_matrix(&self, state: &DVector<f64>) -> DMatrix<f64>;
    fn h_function(&self, state: &DVector<f64>) -> DVector<f64>;
    fn is_linear(&self) -> bool;
    fn clone_model(&self) -> Box<dyn MeasurementModel>;
}

impl Clone for Box<dyn MeasurementModel> {
    fn clone(&self) -> Self {
        self.clone_model()
    }
}

#[derive(Debug, Clone)]
pub struct LinearPosition {
    pub state_dim: usize,
    pub measurement_dim: usize,
}

impl LinearPosition {
    #[must_use]
    pub fn new_3d(state_dim: usize) -> Self {
        Self {
            state_dim,
            measurement_dim: 3,
        }
    }

    #[must_use]
    pub fn new_6d() -> Self {
        Self {
            state_dim: 6,
            measurement_dim: 6,
        }
    }
}

impl MeasurementModel for LinearPosition {
    fn measurement_dim(&self) -> usize {
        self.measurement_dim
    }

    fn state_dim(&self) -> usize {
        self.state_dim
    }

    fn h_matrix(&self, _state: &DVector<f64>) -> DMatrix<f64> {
        let mut h = DMatrix::zeros(self.measurement_dim, self.state_dim);
        for i in 0..self.measurement_dim.min(self.state_dim) {
            h[(i, i)] = 1.0;
        }
        h
    }

    fn h_function(&self, state: &DVector<f64>) -> DVector<f64> {
        state
            .rows(0, self.measurement_dim.min(state.len()))
            .into_owned()
    }

    fn is_linear(&self) -> bool {
        true
    }

    fn clone_model(&self) -> Box<dyn MeasurementModel> {
        Box::new(self.clone())
    }
}

#[derive(Debug, Clone)]
pub struct CartesianToBearingRange {
    pub sensor_position: [f64; 3],
}

impl CartesianToBearingRange {
    #[must_use]
    pub fn new(sensor_position: [f64; 3]) -> Self {
        Self { sensor_position }
    }
}

impl MeasurementModel for CartesianToBearingRange {
    fn measurement_dim(&self) -> usize {
        2
    }

    fn state_dim(&self) -> usize {
        6
    }

    fn h_matrix(&self, state: &DVector<f64>) -> DMatrix<f64> {
        let dx = state[0] - self.sensor_position[0];
        let dy = state[1] - self.sensor_position[1];
        let r2 = dx * dx + dy * dy;
        let r = r2.sqrt().max(1e-10);

        let mut h = DMatrix::zeros(2, 6);

        // d(bearing)/d(x,y)
        h[(0, 0)] = -dy / r2;
        h[(0, 1)] = dx / r2;

        // d(range)/d(x,y)
        h[(1, 0)] = dx / r;
        h[(1, 1)] = dy / r;

        h
    }

    fn h_function(&self, state: &DVector<f64>) -> DVector<f64> {
        let dx = state[0] - self.sensor_position[0];
        let dy = state[1] - self.sensor_position[1];

        let bearing = dy.atan2(dx);
        let range = (dx * dx + dy * dy).sqrt();

        DVector::from_column_slice(&[bearing, range])
    }

    fn is_linear(&self) -> bool {
        false
    }

    fn clone_model(&self) -> Box<dyn MeasurementModel> {
        Box::new(self.clone())
    }
}

#[derive(Debug, Clone)]
pub struct CartesianToElevationBearingRange {
    pub sensor_position: [f64; 3],
}

impl CartesianToElevationBearingRange {
    #[must_use]
    pub fn new(sensor_position: [f64; 3]) -> Self {
        Self { sensor_position }
    }
}

impl MeasurementModel for CartesianToElevationBearingRange {
    fn measurement_dim(&self) -> usize {
        3
    }

    fn state_dim(&self) -> usize {
        6
    }

    fn h_matrix(&self, state: &DVector<f64>) -> DMatrix<f64> {
        let dx = state[0] - self.sensor_position[0];
        let dy = state[1] - self.sensor_position[1];
        let dz = state[2] - self.sensor_position[2];

        let rxy2 = dx * dx + dy * dy;
        let rxy = rxy2.sqrt().max(1e-10);
        let r2 = rxy2 + dz * dz;
        let r = r2.sqrt().max(1e-10);

        let mut h = DMatrix::zeros(3, 6);

        // d(elevation)/d(x,y,z)
        h[(0, 0)] = -dx * dz / (r2 * rxy);
        h[(0, 1)] = -dy * dz / (r2 * rxy);
        h[(0, 2)] = rxy / r2;

        // d(bearing)/d(x,y)
        h[(1, 0)] = -dy / rxy2;
        h[(1, 1)] = dx / rxy2;

        // d(range)/d(x,y,z)
        h[(2, 0)] = dx / r;
        h[(2, 1)] = dy / r;
        h[(2, 2)] = dz / r;

        h
    }

    fn h_function(&self, state: &DVector<f64>) -> DVector<f64> {
        let dx = state[0] - self.sensor_position[0];
        let dy = state[1] - self.sensor_position[1];
        let dz = state[2] - self.sensor_position[2];

        let rxy = (dx * dx + dy * dy).sqrt();
        let range = (dx * dx + dy * dy + dz * dz).sqrt();
        let elevation = dz.atan2(rxy);
        let bearing = dy.atan2(dx);

        DVector::from_column_slice(&[elevation, bearing, range])
    }

    fn is_linear(&self) -> bool {
        false
    }

    fn clone_model(&self) -> Box<dyn MeasurementModel> {
        Box::new(self.clone())
    }
}

#[derive(Debug, Clone)]
pub struct BearingOnly {
    pub sensor_position: [f64; 2],
}

impl BearingOnly {
    #[must_use]
    pub fn new(sensor_position: [f64; 2]) -> Self {
        Self { sensor_position }
    }
}

impl MeasurementModel for BearingOnly {
    fn measurement_dim(&self) -> usize {
        1
    }

    fn state_dim(&self) -> usize {
        4
    }

    fn h_matrix(&self, state: &DVector<f64>) -> DMatrix<f64> {
        let dx = state[0] - self.sensor_position[0];
        let dy = state[1] - self.sensor_position[1];
        let r2 = (dx * dx + dy * dy).max(1e-10);

        let mut h = DMatrix::zeros(1, 4);
        h[(0, 0)] = -dy / r2;
        h[(0, 1)] = dx / r2;
        h
    }

    fn h_function(&self, state: &DVector<f64>) -> DVector<f64> {
        let dx = state[0] - self.sensor_position[0];
        let dy = state[1] - self.sensor_position[1];
        DVector::from_column_slice(&[dy.atan2(dx)])
    }

    fn is_linear(&self) -> bool {
        false
    }

    fn clone_model(&self) -> Box<dyn MeasurementModel> {
        Box::new(self.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use approx::assert_relative_eq;

    #[test]
    fn linear_position_3d_identity() {
        let model = LinearPosition::new_3d(6);
        let state = DVector::from_column_slice(&[1.0, 2.0, 3.0, 4.0, 5.0, 6.0]);
        let z = model.h_function(&state);
        assert_eq!(z.len(), 3);
        assert_relative_eq!(z[0], 1.0);
        assert_relative_eq!(z[1], 2.0);
        assert_relative_eq!(z[2], 3.0);
    }

    #[test]
    fn linear_position_h_matrix() {
        let model = LinearPosition::new_3d(6);
        let state = DVector::zeros(6);
        let h = model.h_matrix(&state);
        assert_eq!(h.nrows(), 3);
        assert_eq!(h.ncols(), 6);
        assert_relative_eq!(h[(0, 0)], 1.0);
        assert_relative_eq!(h[(0, 3)], 0.0);
    }

    #[test]
    fn bearing_range_at_known_position() {
        let model = CartesianToBearingRange::new([0.0, 0.0, 0.0]);
        let state = DVector::from_column_slice(&[100.0, 0.0, 0.0, 0.0, 0.0, 0.0]);
        let z = model.h_function(&state);
        assert_relative_eq!(z[0], 0.0, epsilon = 1e-10); // bearing = atan2(0, 100) = 0
        assert_relative_eq!(z[1], 100.0, epsilon = 1e-10); // range = 100
    }

    #[test]
    fn bearing_range_45_degrees() {
        let model = CartesianToBearingRange::new([0.0, 0.0, 0.0]);
        let state = DVector::from_column_slice(&[100.0, 100.0, 0.0, 0.0, 0.0, 0.0]);
        let z = model.h_function(&state);
        assert_relative_eq!(z[0], std::f64::consts::FRAC_PI_4, epsilon = 1e-10);
        assert_relative_eq!(z[1], (100.0f64 * 2.0_f64.sqrt()), epsilon = 1e-6);
    }

    #[test]
    fn elevation_bearing_range_straight_up() {
        let model = CartesianToElevationBearingRange::new([0.0, 0.0, 0.0]);
        let state = DVector::from_column_slice(&[0.0, 0.0, 100.0, 0.0, 0.0, 0.0]);
        let z = model.h_function(&state);
        assert_relative_eq!(z[0], std::f64::consts::FRAC_PI_2, epsilon = 1e-10); // elevation = 90 deg
        assert_relative_eq!(z[2], 100.0, epsilon = 1e-10); // range = 100
    }

    #[test]
    fn bearing_only_measurement() {
        let model = BearingOnly::new([0.0, 0.0]);
        let state = DVector::from_column_slice(&[100.0, 100.0, 0.0, 0.0]);
        let z = model.h_function(&state);
        assert_eq!(z.len(), 1);
        assert_relative_eq!(z[0], std::f64::consts::FRAC_PI_4, epsilon = 1e-10);
    }

    #[test]
    fn jacobian_numerical_check_bearing_range() {
        let model = CartesianToBearingRange::new([0.0, 0.0, 0.0]);
        let state = DVector::from_column_slice(&[500.0, 300.0, 0.0, 0.0, 0.0, 0.0]);
        let h_analytic = model.h_matrix(&state);
        let eps = 1e-6;

        let z0 = model.h_function(&state);
        for col in 0..2 {
            let mut perturbed = state.clone();
            perturbed[col] += eps;
            let z1 = model.h_function(&perturbed);
            for row in 0..2 {
                let numerical = (z1[row] - z0[row]) / eps;
                assert_relative_eq!(h_analytic[(row, col)], numerical, epsilon = 1e-4);
            }
        }
    }

    #[test]
    fn jacobian_numerical_check_elevation_bearing_range() {
        let model = CartesianToElevationBearingRange::new([0.0, 0.0, 0.0]);
        let state = DVector::from_column_slice(&[500.0, 300.0, 200.0, 0.0, 0.0, 0.0]);
        let h_analytic = model.h_matrix(&state);
        let eps = 1e-6;

        let z0 = model.h_function(&state);
        for col in 0..3 {
            let mut perturbed = state.clone();
            perturbed[col] += eps;
            let z1 = model.h_function(&perturbed);
            for row in 0..3 {
                let numerical = (z1[row] - z0[row]) / eps;
                assert_relative_eq!(h_analytic[(row, col)], numerical, epsilon = 1e-3);
            }
        }
    }
}
