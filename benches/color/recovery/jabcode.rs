//! Port of JABCode's hard-decision module classifier (`decodeModuleHD` in the reference
//! `jabcode/src/jabcode/decoder.c`), with the image balancing it runs first (`balanceRGB` in
//! `binarizer.c`).
//!
//! JABCode decides a module in three steps, against a palette of each colour's measured RGB:
//!
//! 1. **Black gate.** Black if every channel is below that channel's palette threshold — the
//!    midpoint between the brightest palette colour with the channel off and the darkest with
//!    it on (`getPaletteThreshold`).
//! 2. **Chromaticity match.** Otherwise divide the module and every palette colour by their own
//!    largest channel, and take the nearest palette colour in that space. Dividing by the max
//!    discards brightness, so the match is on hue and saturation alone.
//! 3. **Neutral split.** Black and white share a chromaticity, so if the match is either, the
//!    channel sum decides: below the midpoint of the black and white palette sums is black.
//!
//! The palette order matches qrism's colour bits (index = R<<2 | G<<1 | B), so colours index
//! directly.
//!
//! Deviations from the reference, forced by qrism's symbol layout:
//! - JABCode reads four palettes, one per corner, each a single sampled module per colour, and
//!   classifies each module against the nearest corner's. No qrism finder carries all eight
//!   colours, so this uses one palette for the whole symbol, each colour the median of its
//!   calibration samples.
//! - JABCode samples a module as the mean of a 3x3 pixel neighbourhood; this reuses the
//!   benchmark's shared per-module sample so every pipeline sees identical input.

use super::DirectRecovery;
use crate::calibration::{median_rgb, GroupedSamples};
use image::RgbImage;
use qrism::Color;

/// The classifier, fitted on (balanced) calibration samples.
pub(crate) struct JabCode {
    /// Each palette colour divided by its largest channel.
    norm: [[f64; 3]; 8],
    /// Per-channel black-gate thresholds.
    ths: [f64; 3],
    /// Channel-sum midpoint between the black and white palette colours.
    neutral_mid: f64,
}

impl JabCode {
    pub(crate) fn fit(groups: &GroupedSamples) -> Self {
        let palette: [[f64; 3]; 8] = std::array::from_fn(|c| median_rgb(&groups[c]));
        let norm = palette.map(max_normalize);

        // A channel is on in colour `c` when bit (2 - ch) of `c` is set.
        let ths = std::array::from_fn(|ch| {
            let on = |c: usize| (c >> (2 - ch)) & 1 == 1;
            let off_max = (0..8).filter(|&c| !on(c)).map(|c| palette[c][ch]).fold(f64::MIN, f64::max);
            let on_min = (0..8).filter(|&c| on(c)).map(|c| palette[c][ch]).fold(f64::MAX, f64::min);
            (off_max + on_min) / 2.0
        });

        let sum = |c: [f64; 3]| c.iter().sum::<f64>();
        let neutral_mid = (sum(palette[0]) + sum(palette[7])) / 2.0;
        JabCode { norm, ths, neutral_mid }
    }
}

impl DirectRecovery for JabCode {
    fn classify(&self, x: [f64; 3]) -> Color {
        if (0..3).all(|ch| x[ch] < self.ths[ch]) {
            return Color::Black;
        }

        let n = max_normalize(x);
        let dist2 = |p: [f64; 3]| (0..3).map(|k| (p[k] - n[k]).powi(2)).sum::<f64>();
        let mut nearest = (0..8usize)
            .min_by(|&a, &b| dist2(self.norm[a]).partial_cmp(&dist2(self.norm[b])).unwrap())
            .unwrap();

        if nearest == 0 || nearest == 7 {
            nearest = if x.iter().sum::<f64>() < self.neutral_mid { 0 } else { 7 };
        }
        Color::try_from(nearest as u8).unwrap()
    }
}

/// Divides by the largest channel. JABCode divides unguarded; an all-zero pixel never reaches
/// it there because the black gate catches it first, but the guard keeps a degenerate palette
/// colour from producing NaNs.
fn max_normalize(c: [f64; 3]) -> [f64; 3] {
    let m = c[0].max(c[1]).max(c[2]).max(1.0);
    c.map(|v| v / m)
}

/// JABCode's `balanceRGB`: a per-channel histogram stretch over the whole image, mapping each
/// channel's darkest and brightest well-populated levels to 0 and 255. A level counts once
/// more than 20 pixels hold it, so a few specular or dead pixels do not set the range.
pub(crate) struct BalanceRgb {
    range: [(f64, f64); 3],
}

impl BalanceRgb {
    const COUNT_THS: u32 = 20;

    pub(crate) fn fit(img: &RgbImage) -> Self {
        let mut hist = [[0u32; 256]; 3];
        for p in img.pixels() {
            for ch in 0..3 {
                hist[ch][p[ch] as usize] += 1;
            }
        }
        let range = hist.map(|h| {
            let min = (0..256).find(|&i| h[i] > Self::COUNT_THS).unwrap_or(0);
            let max = (0..256).rev().find(|&i| h[i] > Self::COUNT_THS).unwrap_or(255);
            (min as f64, max as f64)
        });
        BalanceRgb { range }
    }

    /// Applies the stretch to one sampled value, truncating to a byte as the reference does.
    pub(crate) fn apply(&self, x: [f64; 3]) -> [f64; 3] {
        std::array::from_fn(|ch| {
            let (lo, hi) = self.range[ch];
            if x[ch] < lo {
                0.0
            } else if x[ch] > hi {
                255.0
            } else {
                ((x[ch] - lo) / (hi - lo).max(1.0) * 255.0).floor()
            }
        })
    }
}
