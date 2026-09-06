//! Euclidean nearest measured-palette recovery (baseline).
//!
//! Classifies a module as whichever of the 8 palette colours it is closest to, where each
//! palette colour is the median appearance measured in this image (in whatever space the
//! normalizer produced). Self-thresholding: the nearest-colour decision is the whole
//! decode, so it skips the thresholding phase. This is the reference the per-colorant
//! recovery is benchmarked against.

use super::DirectRecovery;
use crate::metadata::Color;
use crate::reader::color::calibration::{median_rgb, GroupedSamples};

/// Nearest-palette classifier parameterised by the measured per-colour palette.
pub(crate) struct EuclidMeasured {
    palette: [[f64; 3]; 8],
}

impl EuclidMeasured {
    /// Fits the palette as the median normalized value of each colour group.
    pub(crate) fn fit(ng: &GroupedSamples) -> Self {
        EuclidMeasured { palette: std::array::from_fn(|c| median_rgb(&ng[c])) }
    }
}

impl DirectRecovery for EuclidMeasured {
    fn classify(&self, x: [f64; 3]) -> Color {
        let nearest = (0..8usize)
            .min_by(|&a, &b| {
                dist2(x, self.palette[a]).partial_cmp(&dist2(x, self.palette[b])).unwrap()
            })
            .unwrap();
        Color::try_from(nearest as u8).unwrap()
    }
}

fn dist2(a: [f64; 3], b: [f64; 3]) -> f64 {
    (0..3).map(|k| (a[k] - b[k]).powi(2)).sum()
}
