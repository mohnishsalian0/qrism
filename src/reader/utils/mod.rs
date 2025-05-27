use geometry::{Axis, Point};

use crate::metadata::Color;

use super::binarize::BinaryImage;

pub mod accumulate;
pub mod geometry;
pub mod homography;

// Util function to validate a pattern along a line. This is used in 2 places; in finder locator
// to verify 1:1:3:1:1 pattern, and in alignment locator to verify 1:1:1 pattern
//------------------------------------------------------------------------------

pub fn verify_pattern<A: Axis>(
    img: &BinaryImage,
    seed: &Point,
    pattern: &[f64],
    max_run: u32,
) -> bool {
    let px = img.get_at_point(seed).unwrap();
    let pat_len = pattern.len();

    let mut run_len = vec![0; pat_len];
    run_len[pat_len / 2] = 1;

    // Count backwards
    let mut pos = *seed;
    let mut flips = pat_len / 2;
    let mut initial = Color::from(*px);
    while run_len[flips] <= max_run {
        A::shift(&mut pos, -1);
        if !A::bound_check(img, &pos) {
            break;
        }

        let color = Color::from(*img.get_at_point(&pos).unwrap());
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
    let mut flips = pat_len / 2;
    let mut initial = Color::from(*px);
    while A::bound_check(img, &pos) && run_len[flips] <= max_run {
        A::shift(&mut pos, 1);
        if !A::bound_check(img, &pos) {
            break;
        }

        let color = Color::from(*img.get_at_point(&pos).unwrap());
        if initial != color {
            if flips == pat_len - 1 {
                break;
            }
            initial = color;
            flips += 1;
        }
        run_len[flips] += 1;
    }

    // Verify pattern
    let avg = (run_len.iter().sum::<u32>() as f64) / pattern.iter().sum::<f64>();
    let tol = avg * 3.0 / 4.0;

    for (i, r) in pattern.iter().enumerate() {
        let rl = run_len[i] as f64;
        if rl < r * avg - tol || rl > r * avg + tol {
            return false;
        }
    }

    true
}
