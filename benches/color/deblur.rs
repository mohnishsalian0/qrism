//! Phase 0 — spatial deblur (inter-module interference cancellation).
//!
//! Every stage after this one treats a module's sampled RGB as if it measured that module.
//! It does not: lens blur, ink spread on paper and JPEG chroma subsampling all mix a module's
//! reading with its neighbours'. The mixing is fixed in *pixels* while a module shrinks with
//! version, so it eats a growing share of each module — which is what the accuracy fall-off
//! from v21 to v40 looks like.
//!
//! This models the leak as a 5-point stencil, each of the four edge neighbours contributing a
//! fraction `beta` of its own value:
//!
//! ```text
//!   measured(p) = (1 - n*beta) * true(p) + beta * sum(measured neighbours of p)
//! ```
//!
//! and inverts it per module, `n` being how many neighbours that module actually has so the
//! symbol's border is handled without a special case. This is the spatial analogue of
//! [`crate::recovery::per_colorant`], which cancels the same kind of linear mixing across
//! *colour channels* within one module rather than across *positions*.
//!
//! # Fitting `beta`
//!
//! The obvious gauge would be the timing pattern: a one-module-period stripe is the finest
//! detail the symbol contains, so the contrast it loses measures the blur directly. That does
//! not work on HiQ. Checking the reference renders, the timing pattern's odd positions are
//! always White but the even positions cycle through Black, Blue, Green, Cyan, Red, Pink and
//! Yellow — HiQ does not draw a per-layer-valid timing line (which is why
//! [`crate::hiq`] has to stamp one), so there is no known two-colour alternation to measure
//! contrast against.
//!
//! Scoring candidates on the *calibration* modules does not work either, and the reason is
//! worth recording: the calibration set is the finder and alignment patterns, which are solid
//! blocks. A 5-point deblur is the identity inside a uniform region — it only bites at an
//! edge — so on that set the correction contributes nothing but amplified noise, and any
//! separability criterion evaluated there is maximised at `beta = 0` regardless of how much
//! blur the photo actually has. Fitting on the function patterns measures blur precisely
//! where the symbol has none.
//!
//! So `beta` is scored over *every* module instead, by how tightly the whole grid collapses
//! onto the eight palette centroids: mean distance from a module to its nearest centroid,
//! divided by the mean distance between centroids. Un-mixing a genuinely blurred grid pulls
//! modules back toward the palette points; over-sharpening scatters them. The ratio cancels
//! the `1/(1 - n*beta)` gain, so a candidate cannot win by inflating the scale, and no ground
//! truth is used beyond the function-pattern colours every other stage calibrates on.

use image::RgbImage;
use qrism::symbol::Symbol;
use qrism::Version;

use crate::calibration::GroupedSamples;

/// Largest `beta` considered. The inversion gain is `1/(1 - 4*beta)`, so this caps how much
/// measurement noise a correction is allowed to amplify — well below the `0.25` where the
/// gain diverges.
const BETA_MAX: f64 = 0.20;

/// Candidate steps between 0 and [`BETA_MAX`].
const BETA_STEPS: usize = 10;

/// Sub-module spacing of the optical edge profile, in modules.
const PROFILE_STEP: f64 = 0.05;

/// Minimum peak-to-peak a finder traverse must show before its slope is trusted; below this
/// the profile is noise and the traverse is dropped.
const MIN_CONTRAST: f64 = 20.0;

/// Empirical gain applied to the measured blur width before converting it to a leak fraction.
///
/// The raw measurement is real but reads about a factor of two sharp, for two reasons that
/// both bias the same way. The finders are the highest-contrast features in the frame *and*
/// the patterns the localizer fits its homography to, so they are the best-registered and
/// sharpest part of the symbol — the data modules, whose leak actually matters, sit further
/// out where the homography extrapolates and where residual misregistration blends in
/// neighbouring modules on top of the lens blur. On top of that, the peak-to-peak used to
/// normalise the gradient is taken from one-module-wide bars, which blur already attenuates,
/// so the step height is under-read and the width with it.
///
/// This constant is fitted on the `hiq` dataset — it places the median estimate in the range
/// the fixed-beta sweep found best — and is the weakest part of the estimator. Removing it
/// means modelling the bar attenuation rather than measuring a slope, or probing edges out in
/// the data region instead of at the registration anchor.
const SIGMA_GAIN: f64 = 2.4;

// Optical estimate
//------------------------------------------------------------------------------

/// Measures `beta` from the image itself rather than from the sampled module grid.
///
/// Blur is a property of pixels, not of modules, so the two grid-based criteria above were
/// asking the wrong data. This instead profiles the three finder patterns at sub-module
/// resolution and measures how far a known-sharp edge is smeared.
///
/// Each channel is profiled *separately*, which turned out to be the whole game. Projecting
/// through `min(r, g, b)` measures how sharp the edge is in luma, and JPEG keeps luma at full
/// resolution while subsampling chroma — so a luma edge looks crisp in photographs whose
/// colour information has bled a long way, and the estimate collapses to zero. HiQ paints its
/// finders in complementary colours (green ring against a magenta stone, and so on), so every
/// channel sees a step somewhere along a traverse: red steps at the ring boundary where green
/// does not, and vice versa. Measuring each channel on its own edges recovers the chroma blur
/// that actually drives inter-module colour bleed.
///
/// For a step of height `h` blurred by a Gaussian of width `sigma`, the steepest slope is
/// `h / (sigma * sqrt(2*pi))`, so `sigma` follows from the profile's peak-to-peak and its
/// maximum gradient — both measured off the same traverse, so the attenuation a narrow bar
/// suffers largely divides out. `sigma` lands in module units because the traverse is
/// parameterised in modules, which is exactly what makes the result comparable across
/// versions and camera distances: the same optical blur is a larger fraction of a v40 module
/// than a v21 one, and that is the effect being chased.
pub(crate) fn estimate_beta_optical(sym: &Symbol, img: &RgbImage, ver: Version) -> [f64; 3] {
    let sigma = estimate_sigma_optical(sym, img, ver);
    std::array::from_fn(|k| beta_from_sigma(sigma[k] * SIGMA_GAIN))
}

/// The per-channel Gaussian blur width, in modules, behind [`estimate_beta_optical`]. Split
/// out so the benchmark can report what was measured as well as what it was converted into.
pub(crate) fn estimate_sigma_optical(sym: &Symbol, img: &RgbImage, ver: Version) -> [f64; 3] {
    let grid = ver.width() as f64;
    let origins = [(0.0, 0.0), (grid - 7.0, 0.0), (0.0, grid - 7.0)];

    let mut sigmas: [Vec<f64>; 3] = Default::default();
    for (ox, oy) in origins {
        for horizontal in [true, false] {
            let prof = match traverse(sym, img, ox, oy, horizontal) {
                Some(p) => p,
                None => continue,
            };
            for (k, acc) in sigmas.iter_mut().enumerate() {
                // A channel flat along this traverse carries no edge to measure; the other
                // finders, painted in complementary colours, supply one.
                if let Some(sg) = sigma_of(&prof, k) {
                    acc.push(sg);
                }
            }
        }
    }

    std::array::from_fn(|k| {
        let v = &mut sigmas[k];
        if v.is_empty() {
            return 0.0;
        }
        v.sort_by(|a, b| a.partial_cmp(b).unwrap());
        v[v.len() / 2]
    })
}

/// One traverse across a finder through its centre, as an RGB profile in module steps.
fn traverse(
    sym: &Symbol,
    img: &RgbImage,
    ox: f64,
    oy: f64,
    horizontal: bool,
) -> Option<Vec<[f64; 3]>> {
    // Stay inside the 7x7 so the walk never needs the quiet zone, which has no tile.
    let (lo, hi) = (0.1f64, 6.9f64);
    let n = ((hi - lo) / PROFILE_STEP) as usize;

    let mut prof = Vec::with_capacity(n + 1);
    for i in 0..=n {
        let t = lo + i as f64 * PROFILE_STEP;
        let (px, py) = if horizontal { (ox + t, oy + 3.5) } else { (ox + 3.5, oy + t) };
        prof.push(sample_rgb(sym, img, px, py)?);
    }
    Some(prof)
}

/// The blur width one channel's profile implies, in module units, or `None` if that channel is
/// too flat along this traverse to hold a usable edge.
fn sigma_of(prof: &[[f64; 3]], k: usize) -> Option<f64> {
    let mx = prof.iter().map(|p| p[k]).fold(f64::NEG_INFINITY, f64::max);
    let mn = prof.iter().map(|p| p[k]).fold(f64::INFINITY, f64::min);
    let pp = mx - mn;
    if pp < MIN_CONTRAST {
        return None;
    }

    // Steepest gradient, over a +/-2 sample span to keep single-pixel noise from winning.
    let mut slope: f64 = 0.0;
    for i in 2..prof.len() - 2 {
        let d = (prof[i + 2][k] - prof[i - 2][k]).abs() / (4.0 * PROFILE_STEP);
        slope = slope.max(d);
    }
    if slope <= 0.0 {
        return None;
    }

    Some(pp / (slope * (2.0 * std::f64::consts::PI).sqrt()))
}

/// Bilinearly interpolated RGB at a sub-module position, or `None` where no tile covers it.
///
/// Sampled bilinearly rather than at the nearest pixel. That matters more here than anywhere
/// else in the benchmark: the profile advances in steps of a twentieth of a module, which at
/// these module sizes is a fraction of a pixel, so nearest-neighbour would return a staircase
/// whose risers are pixel boundaries. Its steepest gradient would then measure the sampling
/// grid rather than the lens, reporting every photo as perfectly sharp.
fn sample_rgb(sym: &Symbol, img: &RgbImage, px: f64, py: f64) -> Option<[f64; 3]> {
    if px < 0.0 || py < 0.0 {
        return None;
    }
    let (gx, gy) = (px.floor(), py.floor());
    let (x, y) = sym.exact_map(gx as usize, gy as usize, px - gx, py - gy)?;
    let (w, h) = img.dimensions();
    let (fx, fy) = (x.floor(), y.floor());
    let (tx, ty) = (x - fx, y - fy);

    let at = |ix: i64, iy: i64, k: usize| -> f64 {
        let cx = ix.clamp(0, w as i64 - 1) as u32;
        let cy = iy.clamp(0, h as i64 - 1) as u32;
        img.get_pixel(cx, cy)[k] as f64
    };

    let (ix, iy) = (fx as i64, fy as i64);
    Some(std::array::from_fn(|k| {
        let top = at(ix, iy, k) * (1.0 - tx) + at(ix + 1, iy, k) * tx;
        let bot = at(ix, iy + 1, k) * (1.0 - tx) + at(ix + 1, iy + 1, k) * tx;
        top * (1.0 - ty) + bot * ty
    }))
}

/// Converts a Gaussian blur width, in modules, into the 5-point stencil's leak fraction.
///
/// A neighbouring module spans 0.5 to 1.5 modules from the sample point, so the share of the
/// blur kernel landing on it is `Phi(1.5/sigma) - Phi(0.5/sigma)`, against `2*Phi(0.5/sigma) - 1`
/// staying home. Normalising those to sum to one over the five taps gives `beta`.
fn beta_from_sigma(sigma: f64) -> f64 {
    if sigma <= 1e-3 {
        return 0.0;
    }
    let phi = |z: f64| 0.5 * (1.0 + erf(z / std::f64::consts::SQRT_2));
    let leak = phi(1.5 / sigma) - phi(0.5 / sigma);
    let keep = 2.0 * phi(0.5 / sigma) - 1.0;
    let denom = keep + 4.0 * leak;
    if denom <= 0.0 {
        return 0.0;
    }
    (leak / denom).clamp(0.0, BETA_MAX)
}

/// Abramowitz & Stegun 7.1.26. Ample for picking a blur bucket.
fn erf(x: f64) -> f64 {
    let sign = if x < 0.0 { -1.0 } else { 1.0 };
    let x = x.abs();
    let t = 1.0 / (1.0 + 0.3275911 * x);
    let y = 1.0
        - (((((1.061405429 * t - 1.453152027) * t) + 1.421413741) * t - 0.284496736) * t
            + 0.254829592)
            * t
            * (-x * x).exp();
    sign * y
}

/// Inter-module deblur parameterised by the per-neighbour leak fraction.
pub(crate) struct Deblur {
    beta: [f64; 3],
}

impl Deblur {
    /// A fixed leak fraction on every channel, for comparing against a per-image fit.
    pub(crate) fn fixed(beta: f64) -> Self {
        Deblur { beta: [beta; 3] }
    }

    /// A separately measured leak fraction per channel, which is what the optical estimate
    /// produces — chroma blurs further than luma, so the channels genuinely differ.
    pub(crate) fn per_channel(beta: [f64; 3]) -> Self {
        Deblur { beta }
    }

    /// Picks the leak fraction under which the whole module grid collapses most tightly onto
    /// the palette learned from `groups`, the calibration coordinates bucketed by colour.
    pub(crate) fn fit(rgb: &[Vec<[f64; 3]>], groups: &[Vec<(i32, i32)>; 8]) -> Self {
        let mut best = (f64::INFINITY, 0.0);
        for s in 0..=BETA_STEPS {
            let beta = BETA_MAX * s as f64 / BETA_STEPS as f64;
            let score = snap_cost(rgb, groups, beta);
            if score < best.0 {
                best = (score, beta);
            }
        }
        Deblur { beta: [best.1; 3] }
    }

    /// The fitted leak fraction per channel.
    pub(crate) fn beta(&self) -> [f64; 3] {
        self.beta
    }

    /// Un-mixes every module in the grid.
    pub(crate) fn apply(&self, rgb: &[Vec<[f64; 3]>]) -> Vec<Vec<[f64; 3]>> {
        if self.beta.iter().all(|&b| b <= 0.0) {
            return rgb.to_vec();
        }
        (0..rgb.len())
            .map(|y| (0..rgb[y].len()).map(|x| at_rgb(rgb, x, y, &self.beta)).collect())
            .collect()
    }

    /// The calibration samples as seen *after* deblurring, so the stages downstream fit in the
    /// same space they will be applied in.
    pub(crate) fn sample_groups(
        &self,
        rgb: &[Vec<[f64; 3]>],
        groups: &[Vec<(i32, i32)>; 8],
    ) -> GroupedSamples {
        std::array::from_fn(|c| {
            groups[c]
                .iter()
                .map(|&(gx, gy)| at_rgb(rgb, gx as usize, gy as usize, &self.beta))
                .collect()
        })
    }
}

/// One module's deblurred value. `n` is its real neighbour count, so border modules invert
/// the mixing they actually received rather than an assumed four-neighbour one.
fn at_rgb(rgb: &[Vec<[f64; 3]>], x: usize, y: usize, beta: &[f64; 3]) -> [f64; 3] {
    let h = rgb.len();
    let w = rgb[0].len();
    let mut sum = [0.0f64; 3];
    let mut n = 0.0f64;
    for (dx, dy) in [(-1i32, 0i32), (1, 0), (0, -1), (0, 1)] {
        let (nx, ny) = (x as i32 + dx, y as i32 + dy);
        if nx < 0 || ny < 0 || nx as usize >= w || ny as usize >= h {
            continue;
        }
        let p = rgb[ny as usize][nx as usize];
        for k in 0..3 {
            sum[k] += p[k];
        }
        n += 1.0;
    }
    std::array::from_fn(|k| {
        let denom = 1.0 - n * beta[k];
        if denom.abs() < 1e-6 {
            rgb[y][x][k]
        } else {
            (rgb[y][x][k] - beta[k] * sum[k]) / denom
        }
    })
}

fn at(rgb: &[Vec<[f64; 3]>], x: usize, y: usize, beta: f64) -> [f64; 3] {
    let h = rgb.len();
    let w = rgb[0].len();
    let mut sum = [0.0f64; 3];
    let mut n = 0.0f64;
    for (dx, dy) in [(-1i32, 0i32), (1, 0), (0, -1), (0, 1)] {
        let (nx, ny) = (x as i32 + dx, y as i32 + dy);
        if nx < 0 || ny < 0 || nx as usize >= w || ny as usize >= h {
            continue;
        }
        let p = rgb[ny as usize][nx as usize];
        for k in 0..3 {
            sum[k] += p[k];
        }
        n += 1.0;
    }
    let denom = 1.0 - n * beta;
    if denom.abs() < 1e-6 {
        return rgb[y][x];
    }
    std::array::from_fn(|k| (rgb[y][x][k] - beta * sum[k]) / denom)
}

/// How tightly the deblurred grid collapses onto its own palette: mean distance from a
/// module to the nearest palette centroid over the mean distance between centroids. Lower is
/// better. Scale-invariant, so the inversion gain cannot flatter a candidate.
fn snap_cost(rgb: &[Vec<[f64; 3]>], groups: &[Vec<(i32, i32)>; 8], beta: f64) -> f64 {
    // Palette from the calibration modules, in the deblurred space.
    let mut centroid = [[0.0f64; 3]; 8];
    let mut present = [false; 8];
    for c in 0..8 {
        if groups[c].is_empty() {
            continue;
        }
        let pts: Vec<[f64; 3]> =
            groups[c].iter().map(|&(gx, gy)| at(rgb, gx as usize, gy as usize, beta)).collect();
        let n = pts.len() as f64;
        present[c] = true;
        for k in 0..3 {
            centroid[c][k] = pts.iter().map(|p| p[k]).sum::<f64>() / n;
        }
    }

    let live: Vec<usize> = (0..8).filter(|&c| present[c]).collect();
    if live.len() < 2 {
        return f64::INFINITY;
    }

    // Mean spacing of the palette, the scale everything is measured against.
    let mut spread = 0.0;
    let mut pairs = 0.0;
    for (i, &a) in live.iter().enumerate() {
        for &b in &live[i + 1..] {
            spread += dist(centroid[a], centroid[b]);
            pairs += 1.0;
        }
    }
    let spread = spread / pairs;
    if spread <= 1e-9 {
        return f64::INFINITY;
    }

    // Mean nearest-centroid distance over every module in the grid.
    let mut total = 0.0f64;
    let mut n = 0.0f64;
    for y in 0..rgb.len() {
        for x in 0..rgb[y].len() {
            let v = at(rgb, x, y, beta);
            let mut best = f64::INFINITY;
            for &c in &live {
                let d = dist(v, centroid[c]);
                if d < best {
                    best = d;
                }
            }
            total += best;
            n += 1.0;
        }
    }

    (total / n.max(1.0)) / spread
}

fn dist(a: [f64; 3], b: [f64; 3]) -> f64 {
    (0..3).map(|k| (a[k] - b[k]).powi(2)).sum::<f64>().sqrt()
}
