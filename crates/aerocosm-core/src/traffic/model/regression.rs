//! Least squares for the traffic model's fallback (plan M5 D109): departures
//! from airport and country figures, fitted in logarithms on the observed
//! airports.

/// Coefficients minimising the squared error of `x · β ≈ y` over `samples`
/// (each a row of regressors and the target), or `None` when the regressors
/// are degenerate (fewer samples than coefficients, or collinear).
pub(super) fn least_squares(samples: &[(Vec<f64>, f64)]) -> Option<Vec<f64>> {
    let width = samples.first()?.0.len();
    if samples.len() < width || samples.iter().any(|(x, _)| x.len() != width) {
        return None;
    }
    // Normal equations XᵀX β = Xᵀy, solved by Gaussian elimination with
    // partial pivoting.
    let mut matrix = vec![vec![0.0; width + 1]; width];
    for (x, y) in samples {
        for (i, row) in matrix.iter_mut().enumerate() {
            for (j, cell) in row.iter_mut().take(width).enumerate() {
                *cell += x[i] * x[j];
            }
            row[width] += x[i] * y;
        }
    }
    let scale = matrix
        .iter()
        .flat_map(|row| row.iter().take(width))
        .fold(0.0_f64, |m, v| m.max(v.abs()));
    for column in 0..width {
        let pivot = (column..width)
            .max_by(|&a, &b| matrix[a][column].abs().total_cmp(&matrix[b][column].abs()))?;
        if matrix[pivot][column].abs() <= scale * 1e-12 {
            return None;
        }
        matrix.swap(column, pivot);
        let lead = matrix[column].clone();
        for (r, row) in matrix.iter_mut().enumerate() {
            if r != column {
                let factor = row[column] / lead[column];
                for (cell, value) in row.iter_mut().zip(&lead) {
                    *cell -= factor * value;
                }
            }
        }
    }
    Some(
        matrix
            .iter()
            .enumerate()
            .map(|(i, row)| row[width] / row[i])
            .collect(),
    )
}

/// `x · β`.
pub(super) fn predict(coefficients: &[f64], x: &[f64]) -> f64 {
    coefficients.iter().zip(x).map(|(b, v)| b * v).sum()
}

#[cfg(test)]
mod tests {
    use anyhow::{Context, Result};
    use proptest::prelude::*;

    use super::*;

    #[test]
    fn test_least_squares_recovers_a_plane() -> Result<()> {
        let samples: Vec<(Vec<f64>, f64)> = (0..20)
            .map(|i| {
                let (a, b) = (f64::from(i), f64::from(i * i % 7));
                (vec![1.0, a, b], 2.0 + 0.5 * a - 3.0 * b)
            })
            .collect();
        let beta = least_squares(&samples).context("solvable")?;
        for (got, want) in beta.iter().zip([2.0, 0.5, -3.0]) {
            assert!((got - want).abs() < 1e-9, "{beta:?}");
        }
        assert!((predict(&beta, &[1.0, 2.0, 1.0]) - 0.0).abs() < 1e-9);
        Ok(())
    }

    #[test]
    fn test_least_squares_rejects_degenerate_regressors() {
        let collinear: Vec<(Vec<f64>, f64)> = (0..5)
            .map(|i| (vec![1.0, f64::from(i), 2.0 * f64::from(i)], 1.0))
            .collect();
        assert!(least_squares(&collinear).is_none());
        assert!(least_squares(&[(vec![1.0, 2.0], 1.0)]).is_none(), "too few");
        assert!(least_squares(&[]).is_none());
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(1000))]

        /// The residuals of a least-squares fit are orthogonal to every
        /// regressor.
        #[test]
        fn test_least_squares_residuals_are_orthogonal(
            rows in prop::collection::vec((-10.0_f64..10.0, -10.0_f64..10.0, -50.0_f64..50.0), 6..30)
        ) {
            let samples: Vec<(Vec<f64>, f64)> =
                rows.iter().map(|&(a, b, y)| (vec![1.0, a, b], y)).collect();
            if let Some(beta) = least_squares(&samples) {
                for k in 0..3 {
                    let dot: f64 = samples
                        .iter()
                        .map(|(x, y)| x[k] * (y - predict(&beta, x)))
                        .sum();
                    let size: f64 = samples.iter().map(|(x, y)| (x[k] * y).abs()).sum::<f64>() + 1.0;
                    prop_assert!(dot.abs() < 1e-6 * size, "{dot}");
                }
            }
        }
    }
}
