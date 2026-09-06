//! Adaptive per-channel thresholding (paper eq. 6), applied to recovered indicators.
//!
//! The interference-cancelled analogue of the paper's adaptive threshold: instead of a
//! fixed 0.5 cut on each recovered indicator `j_k`, the cut is calibrated from this
//! image's separation. Each of the 8 grouped colours acts as one "pilot block" — we take
//! its median recovered `j_k`, then split the eight medians by whether channel `k` is ON
//! (colour bit set, indicator low) or OFF (bit clear, indicator high). The threshold sits
//! midway between the two clusters' facing extremes: `max` over the ON medians and `min`
//! over the OFF medians.

use super::{bits_to_color, Thresholder};
use crate::metadata::Color;
use crate::reader::color::calibration::{median_f, GroupedSamples};

/// Per-channel adaptive threshold on the recovered indicator; a channel is ON when
/// `recovered_k < thresh_k`.
pub(crate) struct Adaptive {
    thresh: [f64; 3],
}

impl Adaptive {
    /// Fits per-channel thresholds from the recovered indicators grouped by colour.
    pub(crate) fn fit(recovered: &GroupedSamples) -> Self {
        // Median recovered indicator per colour, per channel.
        let mut med = [[0.0f64; 3]; 8];
        for c in 0..8usize {
            let mut ch: [Vec<f64>; 3] = Default::default();
            for j in &recovered[c] {
                for k in 0..3 {
                    ch[k].push(j[k]);
                }
            }
            for k in 0..3 {
                med[c][k] = median_f(&mut ch[k]);
            }
        }

        let thresh = std::array::from_fn(|k| {
            let bit = |c: usize| (c >> (2 - k)) & 1;
            let on_max = (0..8)
                .filter(|&c| bit(c) == 1 && !recovered[c].is_empty())
                .map(|c| med[c][k])
                .fold(f64::NEG_INFINITY, f64::max);
            let off_min = (0..8)
                .filter(|&c| bit(c) == 0 && !recovered[c].is_empty())
                .map(|c| med[c][k])
                .fold(f64::INFINITY, f64::min);
            let t = (on_max + off_min) / 2.0;
            // Fall back to a fixed cut if a class was absent or the clusters degenerate.
            if t.is_finite() {
                t
            } else {
                0.5
            }
        });

        Adaptive { thresh }
    }

    /// The fitted per-channel thresholds (R, G, B).
    pub(crate) fn thresholds(&self) -> [f64; 3] {
        self.thresh
    }

    fn decide(&self, recovered: [f64; 3]) -> Color {
        bits_to_color(std::array::from_fn(|k| recovered[k] < self.thresh[k]))
    }
}

impl Thresholder for Adaptive {
    fn decide_grid(&self, recovered: &[Vec<[f64; 3]>]) -> Vec<Vec<Color>> {
        recovered.iter().map(|row| row.iter().map(|&c| self.decide(c)).collect()).collect()
    }
}
