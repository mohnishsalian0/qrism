//! Absorptance normalization: linear darkness relative to paper white.
//!
//! The linear-in-intensity analog of [`super::intensity::Intensity`]. Where the density
//! model takes `-log10(rgb/white)`, this takes `d_k = 1 - clamp(rgb_k, 0, white_k)/white_k`
//! — absorptance (1 - reflectance) — floored at 0. It keeps the property that makes the
//! intercept-free per-colorant fit `x = D * j` well-posed (paper white maps to `[0,0,0]`,
//! darker channels grow toward 1) but drops the log, so cross-channel mixing is modelled as
//! linear in reflectance rather than in optical density. The two agree to first order for
//! light tints (`-log10(r) ~= (1-r)/ln10`) and diverge for dark, saturated colours.

use super::Normalizer;
use crate::metadata::Color;
use crate::reader::color::calibration::{median_rgb, GroupedSamples};

/// Absorptance normalizer parameterised by the per-channel paper-white reference.
pub(crate) struct Absorptance {
    white: [f64; 3],
}

impl Absorptance {
    /// Fits the white reference as the median RGB of the White colour group.
    pub(crate) fn fit(raw: &GroupedSamples) -> Self {
        Absorptance { white: median_rgb(&raw[Color::White as usize]) }
    }

    /// The fitted paper-white reference (per channel).
    pub(crate) fn white(&self) -> [f64; 3] {
        self.white
    }
}

impl Normalizer for Absorptance {
    fn apply(&self, rgb: [f64; 3]) -> [f64; 3] {
        std::array::from_fn(|k| {
            let w = self.white[k].max(1.0);
            let ratio = rgb[k].clamp(0.0, w) / w;
            (1.0 - ratio).max(0.0)
        })
    }
}
