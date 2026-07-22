use nalgebra::DVector;

#[derive(Debug, Clone, Default)]
pub struct OspaResult {
    pub distance: f64,
    pub localization: f64,
    pub cardinality: f64,
    pub order: f64,
    pub cutoff: f64,
}

pub fn ospa(
    tracks: &[DVector<f64>],
    truths: &[DVector<f64>],
    cutoff: f64,
    order: f64,
) -> OspaResult {
    let m = tracks.len();
    let n = truths.len();

    if m == 0 && n == 0 {
        return OspaResult {
            order,
            cutoff,
            ..Default::default()
        };
    }

    if m == 0 || n == 0 {
        return OspaResult {
            distance: cutoff,
            localization: 0.0,
            cardinality: cutoff,
            order,
            cutoff,
        };
    }

    let dim = m.max(n);
    let big_cost = (cutoff.powf(order) * 1_000_000.0) as i64;
    let scale = 1_000_000i64;

    let mut matrix = pathfinding::matrix::Matrix::new(dim, dim, big_cost);

    for (i, track) in tracks.iter().enumerate() {
        for (j, truth) in truths.iter().enumerate() {
            let d = euclidean_distance(track, truth).min(cutoff);
            #[allow(clippy::cast_possible_truncation)]
            let cost = (d.powf(order) * scale as f64) as i64;
            matrix[(i, j)] = cost;
        }
    }

    let (total_cost, col_assignments) = pathfinding::kuhn_munkres::kuhn_munkres_min(&matrix);

    let mut loc_sum = 0.0;
    let mut assigned_count = 0;
    for (i, &j) in col_assignments.iter().enumerate() {
        if i < m && j < n {
            let d = euclidean_distance(&tracks[i], &truths[j]).min(cutoff);
            loc_sum += d.powf(order);
            assigned_count += 1;
        }
    }

    let cardinality_penalty = (dim - assigned_count.min(m.min(n))) as f64 * cutoff.powf(order);
    let total = (loc_sum + cardinality_penalty) / dim as f64;
    let ospa_val = total.powf(1.0 / order);

    let loc_component = if assigned_count > 0 {
        (loc_sum / dim as f64).powf(1.0 / order)
    } else {
        0.0
    };

    let card_component = (cardinality_penalty / dim as f64).powf(1.0 / order);

    OspaResult {
        distance: ospa_val,
        localization: loc_component,
        cardinality: card_component,
        order,
        cutoff,
    }
}

#[derive(Debug, Clone, Default)]
pub struct GospaResult {
    pub distance: f64,
    pub localization: f64,
    pub missed: f64,
    pub false_count: f64,
    pub order: f64,
    pub cutoff: f64,
}

pub fn gospa(
    tracks: &[DVector<f64>],
    truths: &[DVector<f64>],
    cutoff: f64,
    order: f64,
) -> GospaResult {
    let m = tracks.len();
    let n = truths.len();

    if m == 0 && n == 0 {
        return GospaResult {
            order,
            cutoff,
            ..Default::default()
        };
    }

    if m == 0 {
        let missed = n as f64 * (cutoff.powf(order) / 2.0);
        return GospaResult {
            distance: missed.powf(1.0 / order),
            missed: missed.powf(1.0 / order),
            order,
            cutoff,
            ..Default::default()
        };
    }

    if n == 0 {
        let false_c = m as f64 * (cutoff.powf(order) / 2.0);
        return GospaResult {
            distance: false_c.powf(1.0 / order),
            false_count: false_c.powf(1.0 / order),
            order,
            cutoff,
            ..Default::default()
        };
    }

    let dim = m.max(n);
    let half_cutoff_p = cutoff.powf(order) / 2.0;
    let penalty_cost = (half_cutoff_p * 1_000_000.0) as i64;
    let scale = 1_000_000i64;

    let mut matrix = pathfinding::matrix::Matrix::new(dim, dim, penalty_cost);

    for (i, track) in tracks.iter().enumerate() {
        for (j, truth) in truths.iter().enumerate() {
            let d = euclidean_distance(track, truth);
            let dp = d.powf(order);
            if dp < cutoff.powf(order) {
                #[allow(clippy::cast_possible_truncation)]
                let cost = (dp * scale as f64) as i64;
                matrix[(i, j)] = cost;
            }
        }
    }

    let (_, col_assignments) = pathfinding::kuhn_munkres::kuhn_munkres_min(&matrix);

    let mut loc_sum = 0.0;
    let mut n_assigned = 0usize;

    for (i, &j) in col_assignments.iter().enumerate() {
        if i < m && j < n {
            let d = euclidean_distance(&tracks[i], &truths[j]);
            let dp = d.powf(order);
            if dp < cutoff.powf(order) {
                loc_sum += dp;
                n_assigned += 1;
            }
        }
    }

    let n_missed = n.saturating_sub(n_assigned);
    let n_false = m.saturating_sub(n_assigned);

    let missed_cost = n_missed as f64 * half_cutoff_p;
    let false_cost = n_false as f64 * half_cutoff_p;
    let total = loc_sum + missed_cost + false_cost;
    let gospa_val = total.powf(1.0 / order);

    GospaResult {
        distance: gospa_val,
        localization: loc_sum.powf(1.0 / order),
        missed: missed_cost.powf(1.0 / order),
        false_count: false_cost.powf(1.0 / order),
        order,
        cutoff,
    }
}

fn euclidean_distance(a: &DVector<f64>, b: &DVector<f64>) -> f64 {
    (a - b).norm()
}

#[cfg(test)]
mod tests {
    use super::*;
    use approx::assert_relative_eq;

    fn vec3(x: f64, y: f64, z: f64) -> DVector<f64> {
        DVector::from_column_slice(&[x, y, z])
    }

    #[test]
    fn ospa_perfect_match() {
        let tracks = vec![vec3(0.0, 0.0, 0.0)];
        let truths = vec![vec3(0.0, 0.0, 0.0)];
        let result = ospa(&tracks, &truths, 100.0, 2.0);
        assert_relative_eq!(result.distance, 0.0, epsilon = 1e-6);
    }

    #[test]
    fn ospa_with_offset() {
        let tracks = vec![vec3(10.0, 0.0, 0.0)];
        let truths = vec![vec3(0.0, 0.0, 0.0)];
        let result = ospa(&tracks, &truths, 100.0, 2.0);
        assert_relative_eq!(result.distance, 10.0, epsilon = 1e-6);
    }

    #[test]
    fn ospa_cutoff_limits_distance() {
        let tracks = vec![vec3(1000.0, 0.0, 0.0)];
        let truths = vec![vec3(0.0, 0.0, 0.0)];
        let result = ospa(&tracks, &truths, 50.0, 2.0);
        assert!(result.distance <= 50.0 + 1e-6);
    }

    #[test]
    fn ospa_empty_both() {
        let result = ospa(&[], &[], 100.0, 2.0);
        assert_relative_eq!(result.distance, 0.0, epsilon = 1e-12);
    }

    #[test]
    fn ospa_missed_target() {
        let result = ospa(&[], &[vec3(0.0, 0.0, 0.0)], 100.0, 2.0);
        assert_relative_eq!(result.distance, 100.0, epsilon = 1e-6);
    }

    #[test]
    fn ospa_false_track() {
        let result = ospa(&[vec3(0.0, 0.0, 0.0)], &[], 100.0, 2.0);
        assert_relative_eq!(result.distance, 100.0, epsilon = 1e-6);
    }

    #[test]
    fn gospa_perfect_match() {
        let tracks = vec![vec3(0.0, 0.0, 0.0)];
        let truths = vec![vec3(0.0, 0.0, 0.0)];
        let result = gospa(&tracks, &truths, 100.0, 2.0);
        assert_relative_eq!(result.distance, 0.0, epsilon = 1e-6);
        assert_relative_eq!(result.missed, 0.0, epsilon = 1e-6);
        assert_relative_eq!(result.false_count, 0.0, epsilon = 1e-6);
    }

    #[test]
    fn gospa_missed_target() {
        let result = gospa(&[], &[vec3(0.0, 0.0, 0.0)], 100.0, 2.0);
        assert!(result.missed > 0.0);
        assert_relative_eq!(result.false_count, 0.0, epsilon = 1e-6);
    }

    #[test]
    fn gospa_false_track() {
        let result = gospa(&[vec3(0.0, 0.0, 0.0)], &[], 100.0, 2.0);
        assert_relative_eq!(result.missed, 0.0, epsilon = 1e-6);
        assert!(result.false_count > 0.0);
    }

    #[test]
    fn gospa_decomposition() {
        let tracks = vec![vec3(5.0, 0.0, 0.0), vec3(100.0, 100.0, 100.0)];
        let truths = vec![vec3(0.0, 0.0, 0.0)];
        let result = gospa(&tracks, &truths, 50.0, 2.0);
        assert!(result.localization > 0.0);
        assert!(result.false_count > 0.0);
    }

    #[test]
    fn gospa_multiple_perfect() {
        let tracks = vec![vec3(0.0, 0.0, 0.0), vec3(100.0, 0.0, 0.0)];
        let truths = vec![vec3(0.0, 0.0, 0.0), vec3(100.0, 0.0, 0.0)];
        let result = gospa(&tracks, &truths, 200.0, 2.0);
        assert_relative_eq!(result.distance, 0.0, epsilon = 1e-6);
    }
}
