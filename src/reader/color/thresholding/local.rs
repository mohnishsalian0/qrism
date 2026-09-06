//! Local (block-adaptive) thresholding — a mirror of [`BinaryImage::prepare`].
//!
//! This ports the exact block-thresholding scheme that
//! [`crate::reader::binarize::BinaryImage::prepare`] runs over image pixels, but onto the
//! per-module recovered-indicator grid: each module here plays the role of a pixel there.
//!   1. Group the grid into power-of-2 blocks whose side is `2^floor(log2(min(grid)/20))`,
//!      handling the fractional right/bottom/corner blocks by folding in the last
//!      `block_size` rows/cols (as `prepare` does).
//!   2. Average each block per channel.
//!   3. Threshold each block by averaging the block means over its 5x5 block neighbourhood,
//!      copying the nearest computed threshold along the borders.
//!   4. Every module in a block is decided against that block's threshold.
//!
//! Values are kept in `f64` rather than quantised to `u8`. The per-module comparison
//! polarity is a mode, because the same block machinery is useful on two kinds of input:
//!   - [`Local::intensity`] — native `prepare` polarity, a channel is on when its value is
//!     *above* the local threshold (brighter = on). Use when running standalone on the
//!     sampled RGB grid — the most direct mirror of `prepare` on the module image.
//!   - [`Local::indicator`] — inverted, a channel is on when *below* the threshold, for a
//!     recovered *darkening* indicator or optical density (low = on). Matches the polarity
//!     the sibling [`adaptive`] thresholder uses when composed after per-colorant recovery.
//!
//! [`BinaryImage::prepare`]: crate::reader::binarize::BinaryImage::prepare
//! [`adaptive`]: crate::reader::color::thresholding::adaptive

use super::{bits_to_color, Thresholder};
use crate::metadata::Color;

/// Number of blocks the shorter grid dimension is divided into (mirrors `prepare`).
const BLOCK_COUNT: f64 = 20.0;
/// Side of the square block neighbourhood used to average a block's threshold.
const BLOCK_GRID_SIZE: usize = 5;

/// Block-adaptive thresholder. Stateless: every threshold is derived from the grid it is
/// handed, so there is nothing to calibrate. The polarity mode (see the module docs) picks
/// which side of the local threshold counts as a channel being on.
pub(crate) struct Local {
    on_above: bool,
}

impl Local {
    /// Native `prepare` polarity: a channel is on where its value is *above* the local
    /// threshold (brighter = on). For raw/normalized intensity grids run standalone.
    pub(crate) fn intensity() -> Self {
        Local { on_above: true }
    }

    /// Inverted polarity: a channel is on where its value is *below* the local threshold
    /// (low = on). For recovered darkening indicators or optical density.
    pub(crate) fn indicator() -> Self {
        Local { on_above: false }
    }
}

impl Thresholder for Local {
    fn decide_grid(&self, recovered: &[Vec<[f64; 3]>]) -> Vec<Vec<Color>> {
        let h = recovered.len();
        let w = if h == 0 { 0 } else { recovered[0].len() };
        if w == 0 || h == 0 {
            return vec![vec![Color::White; w]; h];
        }

        let block_pow = (std::cmp::min(w, h) as f64 / BLOCK_COUNT).log2().max(0.0) as usize;
        let block_size = 1usize << block_pow;
        let mask = block_size - 1;

        let wsteps = (w + mask) >> block_pow;
        let hsteps = (h + mask) >> block_pow;
        let len = wsteps * hsteps;

        // --- Block means (sum then divide by block area) ---
        let mut sum = vec![[0.0f64; 3]; len];

        // Full blocks; the trailing fractional row/col is folded in afterwards.
        let (wr, hr) = (w & !mask, h & !mask);
        let (bw, bh) = (wr >> block_pow, hr >> block_pow);
        for by in 0..bh {
            let y0 = by << block_pow;
            for bx in 0..bw {
                let x0 = bx << block_pow;
                let idx = by * wsteps + bx;
                for yy in 0..block_size {
                    for xx in 0..block_size {
                        let px = recovered[y0 + yy][x0 + xx];
                        for i in 0..3 {
                            sum[idx][i] += px[i];
                        }
                    }
                }
            }
        }

        // Fractional blocks on the right edge (fold in the last `block_size` columns).
        if w & mask != 0 {
            for y in 0..hr {
                let idx = ((y >> block_pow) + 1) * wsteps - 1;
                for x in w - block_size..w {
                    for i in 0..3 {
                        sum[idx][i] += recovered[y][x][i];
                    }
                }
            }
        }

        // Fractional blocks on the bottom edge (fold in the last `block_size` rows).
        if h & mask != 0 {
            let last_row = wsteps * (hsteps - 1);
            for y in h - block_size..h {
                for x in 0..wr {
                    let idx = last_row + (x >> block_pow);
                    for i in 0..3 {
                        sum[idx][i] += recovered[y][x][i];
                    }
                }
            }
        }

        // Fractional block on the bottom-right corner.
        if w & mask != 0 && h & mask != 0 {
            for y in h - block_size..h {
                for x in w - block_size..w {
                    for i in 0..3 {
                        sum[len - 1][i] += recovered[y][x][i];
                    }
                }
            }
        }

        let block_area = (block_size * block_size) as f64;
        let avg: Vec<[f64; 3]> =
            sum.iter().map(|s| std::array::from_fn(|i| s[i] / block_area)).collect();

        // --- Per-block threshold: mean of block means over a 5x5 block neighbourhood ---
        let half = BLOCK_GRID_SIZE / 2;
        let grid_area = (BLOCK_GRID_SIZE * BLOCK_GRID_SIZE) as f64;
        let (maxx, maxy) = (wsteps - half, hsteps - half);
        let mut threshold = vec![[0.0f64; 3]; len];

        for y in 0..hsteps {
            let row_off = y * wsteps;
            for x in 0..wsteps {
                let i = row_off + x;

                // Near a vertical boundary: copy the threshold above.
                if y > 0 && (y <= half || y >= maxy) {
                    threshold[i] = threshold[i - wsteps];
                    continue;
                }
                // Near a horizontal boundary: copy the threshold to the left.
                if x > 0 && (x <= half || x >= maxx) {
                    threshold[i] = threshold[i - 1];
                    continue;
                }

                let cx = std::cmp::max(x, half);
                let cy = std::cmp::max(y, half);
                let mut s = [0.0f64; 3];
                for ny in cy - half..=cy + half {
                    let ni = ny * wsteps + cx;
                    for blk in &avg[ni - half..=ni + half] {
                        for c in 0..3 {
                            s[c] += blk[c];
                        }
                    }
                }
                threshold[i] = std::array::from_fn(|c| s[c] / grid_area);
            }
        }

        // --- Decide every module against its block's threshold ---
        let mut out = vec![vec![Color::White; w]; h];
        for by in 0..hsteps {
            let y0 = by << block_pow;
            let y_end = std::cmp::min(y0 + block_size, h);
            for bx in 0..wsteps {
                let x0 = bx << block_pow;
                let x_end = std::cmp::min(x0 + block_size, w);
                let t = threshold[by * wsteps + bx];

                for (y, row) in out.iter_mut().enumerate().take(y_end).skip(y0) {
                    for (x, cell) in row.iter_mut().enumerate().take(x_end).skip(x0) {
                        let px = recovered[y][x];
                        *cell = bits_to_color(std::array::from_fn(|k| {
                            if self.on_above {
                                px[k] > t[k]
                            } else {
                                px[k] < t[k]
                            }
                        }));
                    }
                }
            }
        }
        out
    }
}
