//! Phase 4 — layer decode.
//!
//! Turns a predicted grid of `Color` into the three messages a HiQ symbol carries, so a
//! pipeline can be scored on what a decoder actually delivers rather than on how many
//! modules it got right. Module accuracy and decode success are only loosely related: errors
//! that cluster inside one Reed-Solomon block sink a layer that a scattered error budget of
//! the same size would survive.
//!
//! HiQ multiplexes three *independent* standard QR codes, one per colour channel — unlike
//! this library's own high-capacity symbols, which spread a single message across the three
//! channels contiguously (see `Symbol::extract_payload`). So each layer is decoded on its
//! own and yields its own message.
//!
//! Each layer is recovered by re-rendering it as a clean monochrome QR and handing it back to
//! the library's ordinary [`detect_qr`] path. That costs a redundant localization on a
//! synthetic, perfectly-gridded image, but it reuses the production decoder end to end rather
//! than reimplementing deinterleaving, Reed-Solomon and the codec — all of which are
//! crate-private.
//!
//! # What gets rewritten before the layer is decoded
//!
//! A layer is *not* a verbatim bitplane of the predicted grid, because HiQ does not draw its
//! function patterns the way a standard QR does. Three cases:
//!
//! - **Spec-derivable patterns** — finders with their separators, alignment patterns, the
//!   timing lines and the always-dark module — are stamped canonically. HiQ paints the
//!   finders in complementary colours (green ring / magenta stone at the top left, and so on)
//!   and each alignment pattern in a colour that cycles with its position, so no single
//!   channel holds a valid copy of either. Their layout follows from the version alone, which
//!   localization already reported, so a real decoder knows them without reading anything —
//!   stamping them removes a confound rather than granting an advantage.
//!
//! - **Achromatic shared regions** — format info and, from version 7, the version info blocks
//!   — are resolved by majority vote across the three channels. HiQ draws them in pure black
//!   or white, identical in every layer, so a decoder that knows the region is neutral can
//!   read one decision from three measurements. The bits still come from the pipeline's
//!   predictions; only the redundancy the format intends is being used. Format info carries
//!   the EC level and mask, which are data rather than spec constants, so they are never
//!   stamped from ground truth.
//!
//! - **Everything else** — the payload — passes through as that channel's own bit.
//!
//! Polarity: a layer module is dark where its channel bit is 0, since HiQ's black is all
//! channels off and its white is all channels on.

use image::{DynamicImage, GrayImage, Luma};

use qrism::{detect_qr, Color, ECLevel, Version};

/// Pixels per module in the synthetic render. Three is enough for the localizer to lock on
/// — the image is noiseless and axis-aligned — and measurably cheaper than four.
const SCALE: i32 = 3;

/// Quiet zone around the synthetic render, in modules. The spec asks for four.
const QUIET: i32 = 4;

/// What one layer yielded. Failures are split so the report can tell a layer that never
/// localized from one that lost its format info from one the error correction gave up on.
pub(crate) enum Layer {
    Decoded(String),
    /// The synthetic render did not localize, or localized at the wrong version.
    NoSymbol,
    /// Localized, but neither copy of the format info survived BCH rectification.
    BadFormat,
    /// Format info read, but Reed-Solomon could not rectify the payload.
    BadPayload,
}

// Module classification
//------------------------------------------------------------------------------

/// How a module's value is decided when rendering a layer.
enum Source {
    /// Fixed by the spec given the version: written as-is.
    Canonical(bool),
    /// Drawn achromatically by HiQ and shared by all three layers: resolved by majority vote
    /// across the predicted channels.
    Majority,
    /// Payload: the channel's own bit.
    Predicted,
}

/// True where a canonical 7x7 finder pattern is dark, in finder-local coordinates.
fn finder_dark(fx: i32, fy: i32) -> bool {
    let ring = fx == 0 || fx == 6 || fy == 0 || fy == 6;
    let stone = (2..=4).contains(&fx) && (2..=4).contains(&fy);
    ring || stone
}

/// Resolves a module inside one of the three 8x8 finder blocks — the 7x7 pattern plus the
/// separator along the block's two inner edges. `(ox, oy)` is the block's origin.
fn finder_block(gx: i32, gy: i32, ox: i32, oy: i32) -> Option<bool> {
    let (dx, dy) = (gx - ox, gy - oy);
    if !(0..8).contains(&dx) || !(0..8).contains(&dy) {
        return None;
    }
    // The 7x7 sits at the corner of the block nearest the symbol corner; the separator takes
    // the remaining row and column on the inner side.
    let fx = if ox == 0 { dx } else { dx - 1 };
    let fy = if oy == 0 { dy } else { dy - 1 };
    Some((0..7).contains(&fx) && (0..7).contains(&fy) && finder_dark(fx, fy))
}

/// Centres of every alignment pattern, minus the three the finders occupy. Mirrors
/// `calibration::alignment_coords`, which exists for the same reason — the library's own copy
/// is crate-private, but the coordinate table on `Version` is public.
fn alignment_centres(ver: Version) -> impl Iterator<Item = (i32, i32)> {
    let aps = ver.alignment_pattern();
    let len = aps.len();
    let last = len.saturating_sub(1);
    (0..len)
        .flat_map(move |i| (0..len).map(move |j| (i, j)))
        .filter(move |ij| ![(0, 0), (0, last), (last, 0)].contains(ij))
        .map(move |(i, j)| (aps[i], aps[j]))
}

/// Decides how module `(gx, gy)` is sourced. Checked in layout order: the finder blocks and
/// alignment patterns first, since an alignment pattern overrides the timing line it crosses.
fn source_of(gx: i32, gy: i32, ver: Version, grid: i32) -> Source {
    for (ox, oy) in [(0, 0), (grid - 8, 0), (0, grid - 8)] {
        if let Some(dark) = finder_block(gx, gy, ox, oy) {
            return Source::Canonical(dark);
        }
    }

    for (cx, cy) in alignment_centres(ver) {
        let (dx, dy) = ((gx - cx).abs(), (gy - cy).abs());
        if dx <= 2 && dy <= 2 {
            // Ring and centre dark, the gap between them light.
            return Source::Canonical(dx.max(dy) != 1);
        }
    }

    // Timing lines, running between the finder blocks along row and column 6. Dark on even
    // coordinates, which is what makes the first module adjacent to each finder dark.
    if gy == 6 && (8..=grid - 9).contains(&gx) {
        return Source::Canonical(gx % 2 == 0);
    }
    if gx == 6 && (8..=grid - 9).contains(&gy) {
        return Source::Canonical(gy % 2 == 0);
    }

    // The always-dark module, just above the bottom-left finder's separator.
    if gx == 8 && gy == grid - 8 {
        return Source::Canonical(true);
    }

    // Format info: row 8 and column 8, both copies. The timing crossings at (6, 8) and (8, 6)
    // were claimed above, so they are already excluded here.
    let fmt_row = gy == 8 && ((0..=8).contains(&gx) || gx >= grid - 8);
    let fmt_col = gx == 8 && ((0..=8).contains(&gy) || gy >= grid - 8);
    if fmt_row || fmt_col {
        return Source::Majority;
    }

    // Version info, from version 7 on: a 3x6 block left of the top-right finder and its
    // transpose above the bottom-left one.
    if matches!(ver, Version::Normal(7..)) {
        let left = (0..6).contains(&gy) && (grid - 11..grid - 8).contains(&gx);
        let above = (0..6).contains(&gx) && (grid - 11..grid - 8).contains(&gy);
        if left || above {
            return Source::Majority;
        }
    }

    Source::Predicted
}

/// Majority vote over a predicted colour's three channel bits. Valid only where HiQ draws
/// black or white, so the truth is all-on or all-off and two agreeing channels outvote a
/// single flipped one.
#[inline]
fn majority_dark(c: Color) -> bool {
    let b = c as u8;
    let on = (b & 1) + ((b >> 1) & 1) + ((b >> 2) & 1);
    on < 2
}

// Rendering
//------------------------------------------------------------------------------

/// Renders one channel of a predicted grid as a clean monochrome QR, rewriting the function
/// patterns as described in the module docs.
fn render_layer(pred: &[Vec<Color>], chan: usize, ver: Version) -> DynamicImage {
    let grid = ver.width() as i32;
    let side = ((grid + 2 * QUIET) * SCALE) as u32;
    let mut img = GrayImage::from_pixel(side, side, Luma([255]));

    for gy in 0..grid {
        for gx in 0..grid {
            let c = pred[gy as usize][gx as usize];
            let dark = match source_of(gx, gy, ver, grid) {
                Source::Canonical(d) => d,
                Source::Majority => majority_dark(c),
                // HiQ's black is all channels off, so a layer is dark where its bit is 0.
                Source::Predicted => (c as u8 >> (2 - chan)) & 1 == 0,
            };
            if !dark {
                continue;
            }
            for dy in 0..SCALE {
                for dx in 0..SCALE {
                    let px = ((QUIET + gx) * SCALE + dx) as u32;
                    let py = ((QUIET + gy) * SCALE + dy) as u32;
                    img.put_pixel(px, py, Luma([0]));
                }
            }
        }
    }

    DynamicImage::ImageLuma8(img)
}

// Decode
//------------------------------------------------------------------------------

/// The EC level each layer of a clean grid was encoded at, in R, G, B order. HiQ picks it per
/// layer, and it fixes the Reed-Solomon block layout a replay has to reproduce.
pub(crate) fn layer_ec_levels(truth: &[Vec<Color>], ver: Version) -> [ECLevel; 3] {
    std::array::from_fn(|chan| {
        let img = render_layer(truth, chan, ver);
        let mut res = detect_qr(&img);
        let sym = res.symbols().first_mut().expect("clean layer must localize");
        sym.read_format_info().expect("clean layer must have format info").0
    })
}

/// Decodes all three layers of a predicted grid, in R, G, B order.
pub(crate) fn decode_layers(pred: &[Vec<Color>], ver: Version) -> [Layer; 3] {
    decode_layers_inner(pred, ver, None)
}

/// As [`decode_layers`], but handing the decoder a per-module confidence so each block is
/// rectified as errors-and-erasures. `steps` are fractions of the block's parity to erase,
/// tried in order.
///
/// `conf` holds one grid per channel, since each layer is decoded as its own symbol and a
/// module's bit can be safe in one channel while being marginal in another. Grids are indexed
/// in the predicted grid's own module coordinates, which lines up with the re-detected
/// synthetic symbol because [`render_layer`] draws a canonical, axis-aligned QR at a fixed
/// scale, so localization recovers the same grid it was given.
pub(crate) fn decode_layers_conf(
    pred: &[Vec<Color>],
    conf: &[Vec<Vec<f64>>; 3],
    ver: Version,
    steps: &[f64],
) -> [Layer; 3] {
    decode_layers_inner(pred, ver, Some((conf, steps)))
}

fn decode_layers_inner(
    pred: &[Vec<Color>],
    ver: Version,
    soft: Option<(&[Vec<Vec<f64>>; 3], &[f64])>,
) -> [Layer; 3] {
    std::array::from_fn(|chan| {
        let img = render_layer(pred, chan, ver);
        let mut res = detect_qr(&img);

        let Some(sym) = res.symbols().first_mut() else {
            return Layer::NoSymbol;
        };
        // A noisy layer can grow spurious finder candidates and localize at some other
        // version; that is a failed layer, not a symbol to score against the wrong reference.
        if sym.version() != ver {
            return Layer::NoSymbol;
        }
        // Read format info first so a format failure is distinguishable from a payload one;
        // `decode` folds both into the same error.
        if sym.read_format_info().is_err() {
            return Layer::BadFormat;
        }
        let decoded = match soft {
            Some((conf, steps)) => sym.decode_with_confidence(&conf[chan], steps),
            None => sym.decode(),
        };
        match decoded {
            Ok((_meta, msg)) => Layer::Decoded(msg),
            Err(_) => Layer::BadPayload,
        }
    })
}
