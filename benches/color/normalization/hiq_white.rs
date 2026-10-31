//! HiQ's white-point normalization, reproduced on qrism's samples.
//!
//! HiQ divides each module's raw RGB by a white point and feeds the ratio straight to its
//! pretrained classifiers; nothing else is fitted per image. The white point is the mean of
//! twelve single pixels at the corners of the finders' white rings, skipping any that HiQ's
//! block binarizer calls black. Both steps are transcribed from the Java, including the
//! single-precision arithmetic (`white` is a `float[]` there), so a module value here is the
//! same number HiQ would compute from the same pixel. See `hiq_port.md`.
//!
//! The binarizer runs on the photograph, so [`white_candidates`] is called once per photo
//! during sampling; the per-image "fit" is then only the mean in [`HiqWhite::fit`].

use std::collections::HashMap;

use image::RgbImage;

use super::Normalizer;

/// Grid coordinates `(x, y)` of the twelve white samples, in the order
/// `DefaultGridSampler.getWhiteSamplePoints` lists them. The order matters: the mean is
/// accumulated in `f32`.
pub(crate) fn white_points(dim: usize) -> [(usize, usize); 12] {
    let d = dim;
    [
        (1, 1),
        (1, 5),
        (5, 1),
        (5, 5),
        (d - 2, 1),
        (d - 2, 5),
        (d - 6, 1),
        (d - 6, 5),
        (1, d - 2),
        (5, d - 2),
        (1, d - 6),
        (5, d - 6),
    ]
}

/// Number of binarizer blocks along each axis (`RGBColorWrapper.rgb2Binary`).
const BLOCKS: u32 = 8;

/// HiQ's colour binarizer, evaluated lazily: block thresholds are computed only for the
/// blocks a query lands in, since twelve pixels touch at most twelve of the 64 blocks.
pub(crate) struct Binarizer<'a> {
    img: &'a RgbImage,
    step: (u32, u32),
    thresholds: HashMap<(u32, u32), [i32; 3]>,
}

impl<'a> Binarizer<'a> {
    pub(crate) fn new(img: &'a RgbImage) -> Self {
        let (w, h) = img.dimensions();
        Binarizer { img, step: (w / BLOCKS + 1, h / BLOCKS + 1), thresholds: HashMap::new() }
    }

    /// Whether `rgb2Binary` marks pixel `(x, y)` black.
    pub(crate) fn is_black(&mut self, x: u32, y: u32) -> bool {
        let key = (x / self.step.0, y / self.step.1);
        let (img, step) = (self.img, self.step);
        let t = *self.thresholds.entry(key).or_insert_with(|| block_threshold(img, key, step));
        let p = img.get_pixel(x, y);
        p[0] as i32 <= t[0] || p[1] as i32 <= t[1] || p[2] as f64 <= 0.95 * t[2] as f64
    }
}

/// `getLocalThreshold` for each channel: the rounded midpoint of the block's range.
fn block_threshold(img: &RgbImage, (bx, by): (u32, u32), (sw, sh): (u32, u32)) -> [i32; 3] {
    let (w, h) = img.dimensions();
    let (x1, y1) = (bx * sw, by * sh);
    let (x2, y2) = ((x1 + sw).min(w), (y1 + sh).min(h));
    let (mut lo, mut hi) = ([255u8; 3], [0u8; 3]);
    for y in y1..y2 {
        for x in x1..x2 {
            let p = img.get_pixel(x, y);
            for k in 0..3 {
                lo[k] = lo[k].min(p[k]);
                hi[k] = hi[k].max(p[k]);
            }
        }
    }
    std::array::from_fn(|k| ((hi[k] as f64 + lo[k] as f64) / 2.0 + 0.5) as i32)
}

/// The white samples that survive the binarizer, in [`white_points`] order. `coords` maps a
/// grid cell to the pixel qrism sampled it at, so the samples are the same pixels the module
/// grid holds.
pub(crate) fn white_candidates(
    img: &RgbImage,
    dim: usize,
    coords: impl Fn(usize, usize) -> (u32, u32),
) -> Vec<[f64; 3]> {
    let mut bin = Binarizer::new(img);
    white_points(dim)
        .into_iter()
        .filter_map(|(gx, gy)| {
            let (x, y) = coords(gx, gy);
            if bin.is_black(x, y) {
                return None;
            }
            let p = img.get_pixel(x, y);
            Some([p[0] as f64, p[1] as f64, p[2] as f64])
        })
        .collect()
}

/// Division by HiQ's white point, in single precision as HiQ does it.
pub(crate) struct HiqWhite {
    white: [f32; 3],
}

impl HiqWhite {
    /// The mean of the surviving white samples, accumulated in `f32` in sample order, or
    /// `(255, 255, 255)` when none survived (`estimateWhiteRGB`).
    pub(crate) fn fit(candidates: &[[f64; 3]]) -> Self {
        if candidates.is_empty() {
            return HiqWhite { white: [255.0; 3] };
        }
        let mut white = [0.0f32; 3];
        for c in candidates {
            for k in 0..3 {
                white[k] += c[k] as f32;
            }
        }
        let n = candidates.len() as f32;
        HiqWhite { white: white.map(|w| w / n) }
    }

    /// A given white point, for checking the port against one HiQ computed.
    pub(crate) fn from_white(white: [f32; 3]) -> Self {
        HiqWhite { white }
    }

    pub(crate) fn white(&self) -> [f32; 3] {
        self.white
    }
}

impl Normalizer for HiqWhite {
    #[inline]
    fn apply(&self, rgb: [f64; 3]) -> [f64; 3] {
        std::array::from_fn(|k| (rgb[k] as f32 / self.white[k]) as f64)
    }
}
