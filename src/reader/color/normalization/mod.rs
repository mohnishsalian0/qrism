//! Phase 1 — normalization.
//!
//! Maps raw sampled RGB into the space the recovery stage works in. A normalizer is fit
//! per image from the grouped calibration samples, then applied per module. [`Identity`]
//! is the "no normalization" slot (used e.g. by euclid-measured on raw RGB); real
//! strategies are [`intensity::Intensity`] (optical density),
//! [`absorptance::Absorptance`] (its linear-in-reflectance analog, `1 - rgb/white`),
//! [`max_channel::MaxChannel`] (brightness neutralization with a black guard) and
//! [`black_white::BlackWhite`] (per-channel black/white-point affine stretch).

pub(crate) mod absorptance;
pub(crate) mod black_white;
pub(crate) mod intensity;
pub(crate) mod max_channel;

use crate::reader::color::calibration::GroupedSamples;

/// A per-channel transform of a module's sampled RGB, learned per image.
pub(crate) trait Normalizer {
    /// Transforms a single module's RGB.
    fn apply(&self, rgb: [f64; 3]) -> [f64; 3];

    /// Applies the transform to every grouped calibration sample, preserving grouping so
    /// downstream stages can fit in the normalized space.
    fn normalize_groups(&self, raw: &GroupedSamples) -> GroupedSamples {
        std::array::from_fn(|c| raw[c].iter().map(|&p| self.apply(p)).collect())
    }
}

/// The "no normalization" slot: passes raw RGB straight through.
pub(crate) struct Identity;

impl Normalizer for Identity {
    #[inline]
    fn apply(&self, rgb: [f64; 3]) -> [f64; 3] {
        rgb
    }
}
