use geometry::{Axis, Point};

use super::binarize::BinaryImage;

pub mod accumulate;
pub mod geometry;
pub mod homography;

// Util functions to verify a pattern along a line. This is used in 2 places; in finder locator
// to verify 1:1:3:1:1 pattern, and in alignment locator to verify 1:1:1 pattern
//------------------------------------------------------------------------------

pub fn verify_finder_pattern(
    img: &BinaryImage,
    seed: &Point,
    pattern: &[f64],
    max_run: u32,
) -> Option<(u32, u32)> {
    let pat_len = pattern.len();

    let mut run_len = vec![0; pat_len];
    run_len[pat_len / 2] = 1;

    // Count upward
    let mut pos = *seed;
    let mut flips = pat_len / 2;
    let mut initial = img.get_at_point(seed).unwrap();
    while run_len[flips] <= max_run {
        pos.y -= 1;
        if pos.y < 0 {
            break;
        }

        let color = img.get_at_point(&pos).unwrap();
        if initial != color {
            if flips == 0 {
                break;
            }
            initial = color;
            flips -= 1;
        }
        run_len[flips] += 1;
    }
    let top = (pos.y + 1) as u32;

    // Count downward
    let mut pos = *seed;
    let mut flips = pat_len / 2;
    let mut initial = img.get_at_point(seed).unwrap();
    while run_len[flips] <= max_run {
        pos.y += 1;
        if img.h == pos.y as u32 {
            break;
        }

        let color = img.get_at_point(&pos).unwrap();
        if initial != color {
            if flips == pat_len - 1 {
                break;
            }
            initial = color;
            flips += 1;
        }
        run_len[flips] += 1;
    }
    let bottom = (pos.y - 1) as u32;

    if !matches_finder_ratio(&run_len) {
        return None;
    }

    Some((top, bottom))
}

// Verifies 5 run lengths against the finder's 1:1:3:1:1 ratio using decoupled bar/space
// module sizes. Bars (indices 0, 2, 4) span 5 modules and spaces (indices 1, 3) span 2, and
// each gets an independent module-size estimate and tolerance. This lets ink bleed widen the
// dark bars relative to the light spaces without failing the ratio — a case that a single
// shared average (avg = sum / 7) structurally cannot express.
pub fn matches_finder_ratio(runs: &[u32]) -> bool {
    debug_assert!(runs.len() >= 5);

    let bar_mod = (runs[0] + runs[2] + runs[4]) as f64 / 5.0;
    let space_mod = (runs[1] + runs[3]) as f64 / 2.0;

    // Reject if the bar and space module sizes diverge implausibly.
    let (lo, hi) =
        if bar_mod < space_mod { (bar_mod, space_mod) } else { (space_mod, bar_mod) };
    if hi > 4.0 * lo {
        return false;
    }

    let bar_tol = bar_mod * BAR_TOLERANCE + 0.5;
    let space_tol = space_mod * SPACE_TOLERANCE + 0.5;

    const RATIO: [f64; 5] = [1.0, 1.0, 3.0, 1.0, 1.0];
    for i in 0..5 {
        let (module, tol) = if i % 2 == 0 { (bar_mod, bar_tol) } else { (space_mod, space_tol) };
        if (runs[i] as f64 - RATIO[i] * module).abs() > tol {
            return false;
        }
    }

    true
}

pub fn verify_alignment_pattern<A: Axis>(
    img: &BinaryImage,
    seed: &Point,
    pattern: &[f64],
    threshold: f64,
    max_run: u32,
) -> bool {
    let px = img.get_at_point(seed).unwrap();
    let pat_len = pattern.len();

    let mut run_len = vec![0; pat_len];
    run_len[pat_len / 2] = 1;

    // Count backwards
    let mut pos = *seed;
    let dir = (-1, -1);
    let mut flips = pat_len / 2;
    let mut initial = img.get_at_point(seed).unwrap();
    while run_len[flips] <= max_run {
        A::shift(&mut pos, &dir);
        if !A::bound_check(img, &pos) {
            break;
        }

        let color = img.get_at_point(&pos).unwrap();
        if initial != color {
            if flips == 0 {
                break;
            }
            initial = color;
            flips -= 1;
        }
        run_len[flips] += 1;
    }

    // Count forwards
    let mut pos = *seed;
    let dir = (1, 1);
    let mut flips = pat_len / 2;
    let mut initial = img.get_at_point(seed).unwrap();
    while A::bound_check(img, &pos) && run_len[flips] <= max_run {
        A::shift(&mut pos, &dir);
        if !A::bound_check(img, &pos) {
            break;
        }

        let color = img.get_at_point(&pos).unwrap();
        if initial != color {
            if flips == pat_len - 1 {
                break;
            }
            initial = color;
            flips += 1;
        }
        run_len[flips] += 1;
    }

    // Ensure the average run length is roughly equal to the threshold (estimate mod size) with 50%
    // tolerance
    let avg = run_len.iter().sum::<u32>() as f64 / 3.0;
    if avg < threshold * 0.5 || threshold * 1.5 < avg {
        return false;
    }

    // Verify pattern with 80% tolerance. This was tuned to pass maximum number of test images
    let tol = avg * ALIGNMENT_PATTERN_TOLERANCE;
    for (i, r) in pattern.iter().enumerate() {
        let rl = run_len[i] as f64;
        if rl < r * avg - tol || rl > r * avg + tol {
            return false;
        }
    }

    true
}

#[cfg(test)]
pub fn rnd_rgb() -> image::Rgb<u8> {
    let h = rand::random_range(0..360) as f64;
    let s = 1.0f64;
    let l = 0.5f64;
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;

    let h_prime = h / 60.0;
    let x = c * (1.0 - ((h_prime % 2.0) - 1.0).abs());
    let (r1, g1, b1) = match h_prime as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };

    let m = l - c / 2.0;

    image::Rgb([
        ((r1 + m) * 255.0).round() as u8,
        ((g1 + m) * 255.0).round() as u8,
        ((b1 + m) * 255.0).round() as u8,
    ])
}

// Global constants
//------------------------------------------------------------------------------

// Per-module tolerance for the finder's dark bars, as a fraction of the bar module size.
pub const BAR_TOLERANCE: f64 = 0.75;

// Per-module tolerance for the finder's light spaces, as a fraction of the space module size.
// Tighter than bars because ink bleed thins the spaces, so they carry less slack.
pub const SPACE_TOLERANCE: f64 = 1.0 / 3.0;

pub const ALIGNMENT_PATTERN_TOLERANCE: f64 = 0.8;
