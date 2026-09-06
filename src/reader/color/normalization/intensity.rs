//! Intensity normalization: optical density relative to paper white.
//!
//! The interference model of Blasinski/Bulan/Sharma is linear in *optical density*, not
//! raw intensity, so per-colorant recovery is normally fed this. Density per channel is
//! `d_k = -log10(clamp(rgb_k, 1, white_k) / white_k)`, floored at 0; the paper-white
//! reference is fit from the White colour group.

use super::Normalizer;
use crate::metadata::Color;
use crate::reader::color::calibration::{median_rgb, GroupedSamples};

/// Intensity normalizer parameterised by the per-channel paper-white reference.
pub(crate) struct Intensity {
    white: [f64; 3],
}

impl Intensity {
    /// Fits the white reference as the median RGB of the White colour group.
    pub(crate) fn fit(raw: &GroupedSamples) -> Self {
        Intensity { white: median_rgb(&raw[Color::White as usize]) }
    }

    /// The fitted paper-white reference (per channel).
    pub(crate) fn white(&self) -> [f64; 3] {
        self.white
    }
}

impl Normalizer for Intensity {
    fn apply(&self, rgb: [f64; 3]) -> [f64; 3] {
        std::array::from_fn(|k| {
            let w = self.white[k].max(1.0);
            let ratio = rgb[k].clamp(1.0, w) / w;
            (-ratio.log10()).max(0.0)
        })
    }
}
