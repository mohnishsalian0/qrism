//! Black/white-point normalization: per-channel affine level stretch.
//!
//! Fits two per-channel references from the calibration groups — `k` from the Black group
//! and `w` from the White group, each a per-channel median. Every module is then remapped
//! per channel as `(x_c - k_c) / (w_c - k_c)`, placing black at 0 and white at 1 and
//! cancelling per-channel gain/offset (illumination colour, sensor black level). Degenerate
//! channels (`w_c <= k_c`) fall back to a passthrough so a missing group can't divide by zero.

use super::Normalizer;
use crate::metadata::Color;
use crate::reader::color::calibration::{median_rgb, GroupedSamples};

/// Per-channel affine normalizer parameterised by the black and white references.
pub(crate) struct BlackWhite {
    black: [f64; 3],
    white: [f64; 3],
}

impl BlackWhite {
    /// Fits the black/white references as the per-channel medians of the Black and White
    /// colour groups.
    pub(crate) fn fit(raw: &GroupedSamples) -> Self {
        BlackWhite {
            black: median_rgb(&raw[Color::Black as usize]),
            white: median_rgb(&raw[Color::White as usize]),
        }
    }

    /// The fitted black and white references (per channel).
    pub(crate) fn references(&self) -> ([f64; 3], [f64; 3]) {
        (self.black, self.white)
    }
}

impl Normalizer for BlackWhite {
    fn apply(&self, rgb: [f64; 3]) -> [f64; 3] {
        std::array::from_fn(|k| {
            let span = self.white[k] - self.black[k];
            if span > 0.0 {
                (rgb[k] - self.black[k]) / span
            } else {
                rgb[k]
            }
        })
    }
}
