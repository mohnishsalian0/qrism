//! Phase 3 — thresholding.
//!
//! Turns the per-channel indicators from a [`ChannelRecovery`] stage into a `Color`.
//! Strategies:
//!   - [`adaptive::Adaptive`] — one per-channel threshold per image, fit from the grouped
//!     calibration samples; each module decided independently (paper eq. 6).
//!   - [`local::Local`] — spatially-varying block thresholds computed from the recovered
//!     grid itself, a mirror of `BinaryImage::prepare`; needs no calibration fit.
//!
//! Because a local threshold depends on a module's neighbours, the trait operates on the
//! whole recovered grid rather than a single module. Recovery strategies that classify
//! directly (euclid-measured) bypass this phase entirely.
//!
//! [`ChannelRecovery`]: crate::reader::color::recovery::ChannelRecovery

pub(crate) mod adaptive;
pub(crate) mod local;

use crate::metadata::Color;

/// Maps a grid of recovered per-channel indicators to a grid of decoded colours.
pub(crate) trait Thresholder {
    fn decide_grid(&self, recovered: &[Vec<[f64; 3]>]) -> Vec<Vec<Color>>;
}

/// Packs three channel-on booleans (R,G,B) into a `Color`.
pub(crate) fn bits_to_color(on: [bool; 3]) -> Color {
    let bits = (on[0] as u8) << 2 | (on[1] as u8) << 1 | on[2] as u8;
    Color::try_from(bits).unwrap()
}
