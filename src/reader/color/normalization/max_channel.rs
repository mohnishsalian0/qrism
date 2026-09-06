//! Max-channel normalization: brightness neutralization with a black guard.
//!
//! Unlike the density model, this makes no optical assumptions. It fits a per-channel
//! range over *all* modules — `min_k`/`max_k` of channel `k` across every sampled module —
//! and derives a per-channel midpoint `mid_k = (min_k + max_k) / 2`. Per module:
//!   - if every channel sits below its midpoint the module reads as unlit, output `[0, 0, 0]`;
//!   - otherwise each channel is divided by the module's own brightest channel, forcing the
//!     dominant channel to 1.0 and neutralizing overall brightness.
//!
//! The black guard also avoids amplifying sensor noise on dark modules, where dividing a
//! near-zero channel by a near-zero max would otherwise manufacture spurious "on" channels.

use super::Normalizer;
use crate::reader::color::calibration::GroupedSamples;

/// Brightness-neutralizing normalizer parameterised by a per-channel midpoint, below which
/// (on every channel) a module is treated as black.
pub(crate) struct MaxChannel {
    mid: [f64; 3],
}

impl MaxChannel {
    /// Fits the per-channel midpoint from the min and max of each channel over every module
    /// (all colour groups pooled).
    pub(crate) fn fit(raw: &GroupedSamples) -> Self {
        let mut min = [f64::INFINITY; 3];
        let mut max = [f64::NEG_INFINITY; 3];
        for group in raw {
            for &p in group {
                for k in 0..3 {
                    min[k] = min[k].min(p[k]);
                    max[k] = max[k].max(p[k]);
                }
            }
        }
        // No samples: degrade to a passthrough that never flags black.
        let mid = std::array::from_fn(|k| {
            if min[k].is_finite() && max[k].is_finite() {
                (min[k] + max[k]) / 2.0
            } else {
                f64::NEG_INFINITY
            }
        });
        MaxChannel { mid }
    }

    /// The fitted per-channel midpoints (R, G, B).
    pub(crate) fn midpoints(&self) -> [f64; 3] {
        self.mid
    }
}

impl Normalizer for MaxChannel {
    fn apply(&self, rgb: [f64; 3]) -> [f64; 3] {
        // Unlit if every channel is below its midpoint.
        if (0..3).all(|k| rgb[k] < self.mid[k]) {
            return [0.0; 3];
        }
        let m = rgb[0].max(rgb[1]).max(rgb[2]);
        if m <= 0.0 {
            return [0.0; 3];
        }
        std::array::from_fn(|k| rgb[k] / m)
    }
}
