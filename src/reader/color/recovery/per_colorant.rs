//! Per-colorant-channel recovery (interference cancellation).
//!
//! From
//!   H. Blasinski, O. Bulan, G. Sharma, "Per-Colorant-Channel Color Barcodes for Mobile
//!   Applications: An Interference Cancellation Framework," IEEE Trans. Image Processing,
//!   vol. 22, no. 4, Apr. 2013.
//!
//! Each module is decoded as three *independent* channel indicators. The normalized value
//! `x` (optical density, if intensity-normalized) is modelled as a linear mix of the
//! printed channels, `x = D * j`, where `j` is the per-channel "darkened" indicator and
//! `D` is a 3x3 interference matrix capturing cross-channel bleed. We estimate `D` from
//! the grouped calibration samples and recover `j = D^-1 * x`; a downstream thresholding
//! stage turns `j` into channel bits.

use super::ChannelRecovery;
use crate::reader::color::calibration::GroupedSamples;

/// Per-colorant recovery parameterised by the inverse interference matrix.
pub(crate) struct PerColorant {
    d_inv: [[f64; 3]; 3],
}

impl PerColorant {
    /// Estimates `D^-1` by a constrained least-squares fit `x = D * j` over all grouped
    /// (already-normalized) samples. Each output channel (row of `D`) is an independent
    /// regression solved via the normal equations, with negative entries clamped to zero
    /// to honour the physical non-negativity constraint (`D >= 0`).
    pub(crate) fn fit(ng: &GroupedSamples) -> Self {
        // Normal equations shared across the three output channels:
        //   ata = sum j j^T   (3x3),   atb[k] = sum j * x_k   (one per output channel).
        let mut ata = [[0.0f64; 3]; 3];
        let mut atb = [[0.0f64; 3]; 3];
        for c in 0..8usize {
            let j = darken_vec(c);
            for x in &ng[c] {
                for r in 0..3 {
                    for col in 0..3 {
                        ata[r][col] += j[r] * j[col];
                    }
                }
                for k in 0..3 {
                    for r in 0..3 {
                        atb[k][r] += j[r] * x[k];
                    }
                }
            }
        }

        let ata_inv = inv3(&ata).unwrap_or(IDENTITY3);
        let mut d = [[0.0f64; 3]; 3];
        for k in 0..3 {
            let row = matvec(&ata_inv, &atb[k]);
            // Physical constraint: a colorant can only add density in a channel.
            d[k] = std::array::from_fn(|i| row[i].max(0.0));
        }
        PerColorant { d_inv: inv3(&d).unwrap_or(IDENTITY3) }
    }
}

impl ChannelRecovery for PerColorant {
    fn recover(&self, x: [f64; 3]) -> [f64; 3] {
        matvec(&self.d_inv, &x)
    }
}

/// The per-channel "darkened" indicator of a palette colour: `j_k = 1 - bit_k`.
/// White -> (0,0,0) (no density), Black -> (1,1,1) (all channels darkened).
fn darken_vec(color: usize) -> [f64; 3] {
    std::array::from_fn(|k| (1 - ((color >> (2 - k)) & 1)) as f64)
}

// Small linear-algebra helpers
//------------------------------------------------------------------------------

const IDENTITY3: [[f64; 3]; 3] = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];

fn matvec(m: &[[f64; 3]; 3], v: &[f64; 3]) -> [f64; 3] {
    std::array::from_fn(|r| (0..3).map(|c| m[r][c] * v[c]).sum())
}

fn inv3(m: &[[f64; 3]; 3]) -> Option<[[f64; 3]; 3]> {
    let det = m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1])
        - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0])
        + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0]);
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
