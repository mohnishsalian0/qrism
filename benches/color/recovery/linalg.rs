//! Small fixed-size linear algebra shared by the recovery strategies.
//!
//! Both [`super::per_colorant`] (inverting the interference matrix) and
//! [`super::mahalanobis`] (inverting a covariance) need a 3x3 inverse, so it lives here
//! rather than being duplicated per strategy.

pub(crate) const IDENTITY3: [[f64; 3]; 3] = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];

pub(crate) fn matvec(m: &[[f64; 3]; 3], v: &[f64; 3]) -> [f64; 3] {
    std::array::from_fn(|r| (0..3).map(|c| m[r][c] * v[c]).sum())
}

pub(crate) fn det3(m: &[[f64; 3]; 3]) -> f64 {
    m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1])
        - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0])
        + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0])
}

/// Inverse by cofactors, or `None` when the matrix is singular to within `1e-9`.
pub(crate) fn inv3(m: &[[f64; 3]; 3]) -> Option<[[f64; 3]; 3]> {
    let det = det3(m);
    if det.abs() < 1e-9 {
        return None;
    }
    let inv_det = 1.0 / det;
    let mut out = [[0.0f64; 3]; 3];
    out[0][0] = (m[1][1] * m[2][2] - m[1][2] * m[2][1]) * inv_det;
    out[0][1] = (m[0][2] * m[2][1] - m[0][1] * m[2][2]) * inv_det;
    out[0][2] = (m[0][1] * m[1][2] - m[0][2] * m[1][1]) * inv_det;
    out[1][0] = (m[1][2] * m[2][0] - m[1][0] * m[2][2]) * inv_det;
    out[1][1] = (m[0][0] * m[2][2] - m[0][2] * m[2][0]) * inv_det;
    out[1][2] = (m[0][2] * m[1][0] - m[0][0] * m[1][2]) * inv_det;
    out[2][0] = (m[1][0] * m[2][1] - m[1][1] * m[2][0]) * inv_det;
    out[2][1] = (m[0][1] * m[2][0] - m[0][0] * m[2][1]) * inv_det;
    out[2][2] = (m[0][0] * m[1][1] - m[0][1] * m[1][0]) * inv_det;
    Some(out)
}
