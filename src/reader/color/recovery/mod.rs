//! Phase 2 — recovery.
//!
//! Recovers the encoded signal from the normalized module value. Two shapes, because the
//! strategies decide differently:
//!   - [`ChannelRecovery`] (per-colorant) separates the signal into per-channel indicators
//!     that still need a [`crate::reader::color::thresholding`] step to become a `Color`.
//!   - [`DirectRecovery`] (euclid-measured) classifies straight to a `Color`, self-
//!     thresholding, so it skips the thresholding phase.
//!
//! Both fit per image on the normalized grouped calibration samples.

pub(crate) mod euclid_measured;
pub(crate) mod per_colorant;

use crate::metadata::Color;
use crate::reader::color::calibration::GroupedSamples;

/// Recovery that yields per-channel indicators requiring downstream thresholding.
pub(crate) trait ChannelRecovery {
    /// Recovers per-channel indicators from a normalized module value.
    fn recover(&self, x: [f64; 3]) -> [f64; 3];

    /// Applies recovery to every grouped sample, preserving grouping so a thresholder can
    /// fit on the recovered values.
    fn recover_groups(&self, ng: &GroupedSamples) -> GroupedSamples {
        std::array::from_fn(|c| ng[c].iter().map(|&x| self.recover(x)).collect())
    }
}

/// Recovery that classifies directly to a `Color` (its own thresholding built in).
pub(crate) trait DirectRecovery {
    /// Classifies a normalized module value into one of the 8 palette colours.
    fn classify(&self, x: [f64; 3]) -> Color;
}
