use image::{GrayImage, Pixel as ImgPixel};

use crate::metadata::Color;
use crate::reader::utils::contour::{trace, Contour};
use crate::utils::BitMatrix;

use super::utils::geometry::Point;

#[cfg(test)]
use std::path::Path;

#[cfg(test)]
use image::ImageResult;

#[cfg(test)]
use image::RgbImage;

// Region
//------------------------------------------------------------------------------

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Region {
    pub id: usize,
    pub src: (u32, u32),
    pub centre: Point,
    pub area: u32,
    pub color: Color,
    pub is_finder: bool,
}

// Block stats
//------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy)]
struct Stat {
    avg: usize,
    min: u8,
    max: u8,
}

impl Stat {
    pub fn new() -> Self {
        Self { avg: 0, min: u8::MAX, max: u8::MIN }
    }

    pub fn accumulate(&mut self, val: u8) {
        self.avg += val as usize;
        self.min = std::cmp::min(self.min, val);
        self.max = std::cmp::max(self.max, val);
    }
}

// Image type for reader
//------------------------------------------------------------------------------

#[derive(Debug)]
pub struct BinaryImage {
    buffer: BitMatrix,
    px_cont: Vec<u16>,      // Contour each boundary pixel belongs to
    contours: Vec<Contour>, // Visited contours, index is id
    pub w: u32,
    pub h: u32,
    pass: u32, // Used to mark alignment pattern
}

// Binarizing functions
impl BinaryImage {
    // Steps:
    // 1. Divides image into blocks of 8x8 pixels. Note: For the last fractional block, the
    //    last 8 pixels are considered. So few pixels might overlap with last 2 blocks
    // 2. Calculates average of each block
    // 3. Calculates the threshold for each block by averaging 5x5 block around the current block if
    //    the block is near an edge or a corner, the window is shifted accordingly.
    // 4. Sets pixel value as false if less than or equal to threshold, else true
    // Note: If the pixel value is equal to threshold, it is set as false for the edge case when
    // threshold is 0 in which case the pixel should be false/black
    pub fn prepare(img: &GrayImage) -> Self {
        let (w, h) = img.dimensions();
        let (w, h) = (w as usize, h as usize);
        let raw: &[u8] = img.as_raw();
        let block_pow = 4;
        let block_size = (1usize << block_pow).min(w).min(h);
        let mask = (1 << block_pow) - 1;

        let wsteps = (w + mask) >> block_pow;
        let hsteps = (h + mask) >> block_pow;
        let len = wsteps * hsteps;

        let mut stats = vec![Stat::new(); len];

        // Calculate sum of 8x8 pixels for each block
        // Skip last few pixels which form fractional blocks. The last block will be computed later
        // Round w and h to skips these pixels
        let (wr, hr) = (w & !mask, h & !mask);
        let bw = wr >> block_pow; // full block columns
        let bh = hr >> block_pow; // full block rows
        for by in 0..bh {
            let y0 = by << block_pow;
            for bx in 0..bw {
                let x0 = bx << block_pow;
                let idx = by * wsteps + bx;
                let mut local = Stat::new();
                for y in y0..y0 + block_size {
                    let base = y * w + x0;
                    for &px in &raw[base..base + block_size] {
                        local.accumulate(px);
                    }
                }
                stats[idx] = local;
            }
        }

        // Sum of 8x8 pixels for fractional blocks (if exists) on the right edge
        if w & mask != 0 {
            let x0 = w - block_size;
            for by in 0..bh {
                let idx = (by + 1) * wsteps - 1;
                let mut local = Stat::new();
                for y in (by << block_pow)..((by + 1) << block_pow) {
                    let base = y * w + x0;
                    for &px in &raw[base..base + block_size] {
                        local.accumulate(px);
                    }
                }
                stats[idx] = local;
            }
        }

        // Sum of 8x8 pixels for fractional blocks (if exists) on the bottom edge
        if h & mask != 0 {
            let last_row = wsteps * (hsteps - 1);
            let y0 = h - block_size;
            for bx in 0..bw {
                let x0 = bx << block_pow;
                let mut local = Stat::new();
                for y in y0..h {
                    let base = y * w + x0;
                    for &px in &raw[base..base + block_size] {
                        local.accumulate(px);
                    }
                }
                stats[last_row + bx] = local;
            }
        }

        // Sum of 8x8 pixels for fractional blocks (if exists) on the bottom right corner
        if w & mask != 0 && h & mask != 0 {
            let (x0, y0) = (w - block_size, h - block_size);
            let mut local = Stat::new();
            for y in y0..h {
                let base = y * w + x0;
                for &px in &raw[base..base + block_size] {
                    local.accumulate(px);
                }
            }
            stats[len - 1] = local;
        }

        // Take average from the sum calculated for each block
        // If variance is low (<= 25), assume the block is white. Because there is a high chance
        // that the block is outside the qr. Unless the block has top/left neighbors, in which
        // case take average of them.
        let block_area_pow = 2 * block_pow;
        #[allow(clippy::needless_range_loop)]
        for i in 0..len {
            if stats[i].max - stats[i].min <= 25 {
                stats[i].avg = (stats[i].min as usize) / 2;
                if i > wsteps && i % wsteps > 0 {
                    // Average of neighbors 2 * (x-1, y), (x, y-1), (x-1, y-1)
                    let left = stats[i - 1].avg;
                    let top = stats[i - wsteps].avg;
                    let top_left = stats[i - wsteps - 1].avg;
                    let ng_avg = (2 * left + top + top_left) / 4;
                    if stats[i].min < ng_avg as u8 {
                        stats[i].avg = ng_avg;
                    }
                }
            } else {
                // Convert block sum to average (divide by 64)
                stats[i].avg >>= block_area_pow;
            }
        }

        // Calculates threshold for blocks
        let half_grid = BLOCK_GRID_SIZE / 2;
        let (maxx, maxy) = (wsteps.saturating_sub(half_grid), hsteps.saturating_sub(half_grid));
        let mut threshold = vec![0u8; wsteps * hsteps];

        for y in 0..hsteps {
            let row_off = y * wsteps;
            for x in 0..wsteps {
                let i = row_off + x;

                // If y is near any boundary then copy the threshold above
                if y > 0 && (y <= half_grid || y >= maxy) {
                    threshold[i] = threshold[i - wsteps];
                    continue;
                }

                // If x is near any boundary then copy the left threshold
                if x > 0 && (x <= half_grid || x >= maxx) {
                    threshold[i] = threshold[i - 1];
                    continue;
                }

                // The window is clamped to the grid so an image spanning fewer than
                // BLOCK_GRID_SIZE blocks on an axis averages what blocks it has instead of
                // indexing past `stats`. For a grid of BLOCK_GRID_SIZE or more the copy
                // branches above keep the full window in range, so this clamps nothing.
                let cx = std::cmp::max(x, half_grid);
                let cy = std::cmp::max(y, half_grid);
                let (x0, x1) = (cx.saturating_sub(half_grid), (cx + half_grid).min(wsteps - 1));
                let (y0, y1) = (cy.saturating_sub(half_grid), (cy + half_grid).min(hsteps - 1));
                let mut sum = 0usize;
                for ny in y0..=y1 {
                    let ni = ny * wsteps;
                    for px_stat in &stats[ni + x0..=ni + x1] {
                        sum += px_stat.avg;
                    }
                }

                let count = (x1 - x0 + 1) * (y1 - y0 + 1);
                threshold[i] = (sum / count) as u8;
            }
        }

        let mut trow = vec![0u8; w];
        let mut buffer = BitMatrix::new(w as u32, h as u32, 1);
        for by in 0..hsteps {
            for (bx, &t) in threshold[by * wsteps..(by + 1) * wsteps].iter().enumerate() {
                let x0 = bx << block_pow;
                let x_end = std::cmp::min(x0 + block_size, w);
                trow[x0..x_end].fill(t);
            }

            let y0 = by << block_pow;
            let y_end = std::cmp::min(y0 + block_size, h);
            for y in y0..y_end {
                let row = &raw[y * w..y * w + w];

                let mut x = 0;
                while x + 64 <= w {
                    let px: &[u8; 64] = &row[x..x + 64].try_into().unwrap();
                    let th: &[u8; 64] = &trow[x..x + 64].try_into().unwrap();
                    let mut word = 0u64;
                    for i in 0..64 {
                        word |= u64::from(px[i] > th[i]) << i;
                    }
                    buffer.push_bits(word, 64);
                    x += 64;
                }

                if x < w {
                    let mut word = 0u64;
                    for (i, (&px, &th)) in row[x..].iter().zip(&trow[x..]).enumerate() {
                        word |= u64::from(px > th) << i;
                    }
                    buffer.push_bits(word, w - x);
                }
            }
        }

        let px_cont = vec![u16::MAX; w * h];
        let contours = Vec::with_capacity(100);
        Self { buffer, px_cont, contours, w: w as u32, h: h as u32, pass: 0 }
    }

    pub fn prepare_discard<P>(img: &image::ImageBuffer<P, Vec<u8>>) -> Self
    where
        P: ImgPixel<Subpixel = u8>,
    {
        let (w, h) = img.dimensions();
        let chan_count = P::CHANNEL_COUNT as usize;
        let raw: &[u8] = img.as_raw();
        let px = |x: u32, y: u32, c: usize| raw[((y * w + x) as usize) * chan_count + c];
        let block_pow = (std::cmp::min(w, h) as f64 / BLOCK_COUNT).log2() as usize;
        let block_size = 1 << block_pow;
        let mask = (1 << block_pow) - 1;

        let wsteps = (w + mask) >> block_pow;
        let hsteps = (h + mask) >> block_pow;
        let len = (wsteps * hsteps) as usize;

        let mut stats = vec![[Stat::new(); 4]; len];

        // Calculate sum of 8x8 pixels for each block
        // Skip last few pixels which form fractional blocks. The last block will be computed later
        // Round w and h to skips these pixels
        let (wr, hr) = (w & !mask, h & !mask);
        let bw = wr >> block_pow; // full block columns
        let bh = hr >> block_pow; // full block rows
        for by in 0..bh {
            let y0 = by << block_pow;
            for bx in 0..bw {
                let x0 = bx << block_pow;
                let idx = (by * wsteps + bx) as usize;
                let mut local = [Stat::new(); 4];
                for yy in 0..block_size {
                    let y = y0 + yy;
                    let base = ((y * w + x0) as usize) * chan_count;
                    for xx in 0..block_size as usize {
                        let poff = base + xx * chan_count;
                        for i in 0..chan_count {
                            local[i].accumulate(raw[poff + i]);
                        }
                    }
                }
                stats[idx] = local;
            }
        }

        // Sum of 8x8 pixels for fractional blocks (if exists) on the right edge
        if w & mask != 0 {
            for y in 0..hr {
                let idx = (((y >> block_pow) + 1) * wsteps - 1) as usize;
                for x in w - block_size..w {
                    for i in 0..chan_count {
                        stats[idx][i].accumulate(px(x, y, i));
                    }
                }
            }
        }

        // Sum of 8x8 pixels for fractional blocks (if exists) on the bottom edge
        if h & mask != 0 {
            let last_row = wsteps * (hsteps - 1);
            for y in h - block_size..h {
                for x in 0..wr {
                    let idx = (last_row + (x >> block_pow)) as usize;

                    for i in 0..chan_count {
                        stats[idx][i].accumulate(px(x, y, i));
                    }
                }
            }
        }

        // Sum of 8x8 pixels for fractional blocks (if exists) on the bottom right corner
        if w & mask != 0 && h & mask != 0 {
            for y in h - block_size..h {
                for x in w - block_size..w {
                    for i in 0..chan_count {
                        stats[len - 1][i].accumulate(px(x, y, i));
                    }
                }
            }
        }

        // Take average from the sum calculated for each block
        // If variance is low (<= 25), assume the block is white. Because there is a high chance
        // that the block is outside the qr. Unless the block has top/left neighbors, in which
        // case take average of them.
        let wsteps = wsteps as usize;
        let hsteps = hsteps as usize;
        let block_area_pow = 2 * block_pow;
        #[allow(clippy::needless_range_loop)]
        for i in 0..len {
            for j in 0..chan_count {
                // FIXME:
                // if stats[i][j].max - stats[i][j].min <= 25 {
                //     stats[i][j].avg = (stats[i][j].min as usize) / 2;
                //     if i > wsteps && i % wsteps > 0 {
                //         // Average of neighbors 2 * (x-1, y), (x, y-1), (x-1, y-1)
                //         let left = stats[i - 1][j].avg;
                //         let top = stats[i - wsteps][j].avg;
                //         let top_left = stats[i - wsteps - 1][j].avg;
                //         let ng_avg = (2 * left + top + top_left) / 4;
                //         if stats[i][j].min < ng_avg as u8 {
                //             stats[i][j].avg = ng_avg;
                //         }
                //     }
                // } else {
                //     // Convert block sum to average (divide by 64)
                //     stats[i][j].avg >>= block_area_pow;
                // }
                stats[i][j].avg >>= block_area_pow;
            }
        }

        // Calculates threshold for blocks
        let half_grid = BLOCK_GRID_SIZE / 2;
        let grid_area = BLOCK_GRID_SIZE * BLOCK_GRID_SIZE;
        let (maxx, maxy) = (wsteps.saturating_sub(half_grid), hsteps.saturating_sub(half_grid));
        let mut threshold = vec![[0u8; 4]; wsteps * hsteps];

        for y in 0..hsteps {
            let row_off = y * wsteps;
            for x in 0..wsteps {
                let i = row_off + x;

                // If y is near any boundary then copy the threshold above
                if y > 0 && (y <= half_grid || y >= maxy) {
                    threshold[i] = threshold[i - wsteps];
                    continue;
                }

                // If x is near any boundary then copy the left threshold
                if x > 0 && (x <= half_grid || x >= maxx) {
                    threshold[i] = threshold[i - 1];
                    continue;
                }

                let cx = std::cmp::max(x, half_grid);
                let cy = std::cmp::max(y, half_grid);
                let mut sum = [0usize; 4];
                for ny in cy - half_grid..=cy + half_grid {
                    let ni = ny * wsteps + cx;
                    for px_stat in &stats[ni - half_grid..=ni + half_grid] {
                        for (i, chan_stat) in px_stat.iter().take(chan_count).enumerate() {
                            sum[i] += chan_stat.avg;
                        }
                    }
                }

                for (c, t) in threshold[i].iter_mut().take(chan_count).enumerate() {
                    *t = (sum[c] / grid_area) as u8;
                }
            }
        }

        // Initially mark all pixels as unvisited; will be used for flood fill later.
        // Colour plane packs `color_size` bits per pixel; the matrix strides columns by it.
        let color_size = chan_count.next_power_of_two() as u32;
        let mut buffer = BitMatrix::new(w, h, color_size);
        for by in 0..hsteps {
            let y0 = (by << block_pow) as u32;
            let y_end = std::cmp::min(y0 + block_size, h);
            for bx in 0..wsteps {
                let x0 = (bx << block_pow) as u32;
                let x_end = std::cmp::min(x0 + block_size, w);
                let t = threshold[by * wsteps + bx];

                for y in y0..y_end {
                    for x in x0..x_end {
                        let mut color_byte = 0u64;
                        for (i, &th) in t.iter().take(chan_count).enumerate() {
                            color_byte = (color_byte << 1) | u64::from(px(x, y, i) > th);
                        }

                        if color_byte != 0 {
                            buffer.put(x, y, color_byte);
                        }
                    }
                }
            }
        }

        let px_cont = vec![u16::MAX; (w * h) as usize];
        let contours = Vec::with_capacity(100);
        Self { buffer, px_cont, contours, w, h, pass: 0 }
    }
}

// Util functions
impl BinaryImage {
    pub fn get(&self, x: u32, y: u32) -> Option<Color> {
        if x >= self.w || y >= self.h {
            return None;
        }
        let bits = self.buffer.get(x, y);
        Some(if self.buffer.elem_bits() == 1 {
            Color::from(bits != 0)
        } else {
            bits.try_into().ok()?
        })
    }

    pub fn get_bit(&self, x: u32, y: u32) -> Option<bool> {
        if x >= self.w || y >= self.h {
            return None;
        }
        let bit = self.buffer.get_bit(x, y);
        Some(bit)
    }

    // Colour at `(x, y)` plus the length of the run of that colour reaching to the row's right
    // edge. Lets a row scan step by whole runs; see `BitMatrix::run`.
    pub fn run(&self, x: u32, y: u32) -> Option<(bool, u32)> {
        if x >= self.w || y >= self.h {
            return None;
        }
        let (bit, len) = self.buffer.run(x, y);
        Some((bit, len))
    }

    // Bit at `(x, y)`, for callers that have already bounds-checked the coordinate.
    #[inline]
    pub(super) fn get_bit_unbounded(&self, x: u32, y: u32) -> bool {
        self.buffer.get_bit(x, y)
    }

    pub fn get_bit_bounded(&self, x: i32, y: i32) -> Option<bool> {
        if x < 0 || y < 0 {
            return None;
        }

        let (x, y) = (x as u32, y as u32);
        self.get_bit(x, y)
    }

    pub fn get_at_point(&self, pt: &Point) -> Option<Color> {
        let (x, y) = self.wrap_coords(pt.x, pt.y)?;
        let bits = self.buffer.get(x, y);
        Some(if self.buffer.elem_bits() == 1 {
            Color::from(bits != 0)
        } else {
            bits.try_into().ok()?
        })
    }

    pub fn get_bit_at_point(&self, pt: &Point) -> Option<bool> {
        let (x, y) = self.wrap_coords(pt.x, pt.y)?;
        let bit = self.buffer.get_bit(x, y);
        Some(bit)
    }

    fn wrap_coords(&self, x: i32, y: i32) -> Option<(u32, u32)> {
        let w = self.w as i32;
        let h = self.h as i32;

        if x < -w || w <= x || y < -h || h <= y {
            return None;
        }

        let x = if x < 0 { x + w } else { x };
        let y = if y < 0 { y + h } else { y };

        Some((x as u32, y as u32))
    }

    pub fn contains(&self, x: i32, y: i32) -> bool {
        0 <= x && (x as u32) < self.w && 0 <= y && (y as u32) < self.h
    }

    pub fn matches_bit(&self, x: i32, y: i32, bit: bool) -> bool {
        if x < 0 || y < 0 {
            return false;
        }
        let (x, y) = (x as u32, y as u32);
        x < self.w && y < self.h && self.buffer.get_bit(x, y) == bit
    }

    pub fn get_px_contour(&self, x: u32, y: u32) -> Option<u16> {
        if self.w <= x || self.h <= y {
            return None;
        }

        let id = self.px_cont[(y * self.w + x) as usize];
        (id < UNLABELED).then_some(id)
    }

    pub fn get_contours(&self) -> &[Contour] {
        &self.contours
    }

    pub fn get_contours_mut(&mut self) -> &mut Vec<Contour> {
        &mut self.contours
    }

    pub fn set_px_contour(&mut self, x: u32, y: u32, cont_id: u16) {
        if x < self.w && y < self.h {
            self.px_cont[(y * self.w + x) as usize] = cont_id;
        }
    }

    pub fn next_pass(&mut self) -> u32 {
        self.pass += 1;
        self.pass
    }

    #[cfg(test)]
    pub fn save(&self, path: &Path) -> ImageResult<()> {
        let mut img = RgbImage::new(self.w, self.h);
        for y in 0..self.h {
            for x in 0..self.w {
                let bits = self.buffer.get(x, y);
                let rgb = if self.buffer.elem_bits() == 1 {
                    // B&W: 1 = light/white, 0 = dark/black
                    if bits == 0 {
                        image::Rgb([0, 0, 0])
                    } else {
                        image::Rgb([255, 255, 255])
                    }
                } else {
                    // Multicolor: low 3 bits are R<<2 | G<<1 | B, i.e. a Color
                    Color::try_from(bits as u8).unwrap_or(Color::White).into()
                };
                img.put_pixel(x, y, rgb);
            }
        }
        img.save(path)?;
        Ok(())
    }
}

#[cfg(test)]
mod bit_accessor_tests {
    use super::{BinaryImage, BitMatrix, Color, Contour, Point, UNLABELED};

    fn sketch(rows: &[&str]) -> BinaryImage {
        let h = rows.len() as u32;
        let w = rows[0].len() as u32;
        let mut buffer = BitMatrix::new(w, h, 1);
        for (y, row) in rows.iter().enumerate() {
            assert_eq!(row.len() as u32, w, "sketch rows must all be the same width");
            for (x, c) in row.chars().enumerate() {
                let light = match c {
                    '.' => true,
                    '#' => false,
                    other => panic!("sketch takes '.' and '#', got {other:?}"),
                };
                buffer.put(x as u32, y as u32, light as u64);
            }
        }
        let px_cont = vec![UNLABELED; (w * h) as usize];
        let contours: Vec<Contour> = Vec::new();
        BinaryImage { buffer, px_cont, contours, w, h, pass: 0 }
    }

    const ROWS: [&str; 4] = [
        "..###...", //
        "#..##..#", //
        "########", //
        ".......#", //
    ];

    fn light(x: u32, y: u32) -> bool {
        ROWS[y as usize].as_bytes()[x as usize] == b'.'
    }

    #[test]
    fn test_bit_is_set_for_light_pixels() {
        let img = sketch(&ROWS);
        for y in 0..img.h {
            for x in 0..img.w {
                let expected = light(x, y);
                assert_eq!(img.get_bit(x, y), Some(expected), "bit at ({x}, {y})");
                assert_eq!(
                    img.get(x, y),
                    Some(if expected { Color::White } else { Color::Black }),
                    "colour at ({x}, {y})"
                );
                // The bit accessors and the colour accessors must never disagree.
                let pt = Point { x: x as i32, y: y as i32 };
                assert_eq!(
                    img.get_bit_at_point(&pt),
                    Some(img.get_at_point(&pt) == Some(Color::White)),
                    "get_bit_at_point disagrees with get_at_point at ({x}, {y})"
                );
            }
        }
    }

    #[test]
    fn test_get_bit_rejects_out_of_bounds() {
        let img = sketch(&ROWS);
        assert_eq!(img.get_bit(img.w, 0), None, "one past the right edge");
        assert_eq!(img.get_bit(0, img.h), None, "one past the bottom edge");
        assert_eq!(img.get_bit(img.w - 1, img.h - 1), Some(false), "the last pixel is in bounds");
    }

    #[test]
    fn test_get_bit_unbounded_agrees_with_get_bit() {
        let img = sketch(&ROWS);
        for y in 0..img.h {
            for x in 0..img.w {
                assert_eq!(img.get_bit_unbounded(x, y), img.get_bit(x, y).unwrap(), "({x}, {y})");
            }
        }
    }

    #[test]
    fn test_get_bit_bounded_rejects_negatives() {
        let img = sketch(&ROWS);
        let (w, h) = (img.w as i32, img.h as i32);

        assert_eq!(img.get_bit_bounded(-1, 0), None);
        assert_eq!(img.get_bit_bounded(0, -1), None);
        assert_eq!(img.get_bit_bounded(-1, -1), None);
        assert_eq!(img.get_bit_bounded(w, 0), None);
        assert_eq!(img.get_bit_bounded(0, h), None);

        assert_eq!(img.get_bit_bounded(0, 0), Some(true));
        assert_eq!(img.get_bit_bounded(w - 1, h - 1), Some(false));
    }

    #[test]
    fn test_get_bit_at_point_wraps_negatives() {
        let img = sketch(&ROWS);
        let (w, h) = (img.w as i32, img.h as i32);
        let at = |x, y| img.get_bit_at_point(&Point { x, y });

        assert_eq!(at(-1, -1), img.get_bit(img.w - 1, img.h - 1), "(-1, -1) is the last pixel");
        assert_eq!(at(-1, -1), Some(false), "and that pixel is dark");
        assert_eq!(at(-w, -h), img.get_bit(0, 0), "(-w, -h) is the first pixel");
        assert_eq!(at(-w, -h), Some(true));
        assert_eq!(at(-3, 0), img.get_bit(img.w - 3, 0), "wraps per axis, independently");

        // One step past the wrap window on either side is out of bounds.
        assert_eq!(at(-w - 1, 0), None);
        assert_eq!(at(0, -h - 1), None);
        assert_eq!(at(w, 0), None);
        assert_eq!(at(0, h), None);
    }

    #[test]
    fn test_signed_accessors_disagree_on_negatives() {
        let img = sketch(&ROWS);
        assert_eq!(img.get_bit_bounded(-1, -1), None);
        assert_eq!(img.get_bit_at_point(&Point { x: -1, y: -1 }), Some(false));
    }

    #[test]
    fn test_matches_bit() {
        let img = sketch(&ROWS);
        let (w, h) = (img.w as i32, img.h as i32);

        assert!(img.matches_bit(0, 0, true), "(0, 0) is light");
        assert!(!img.matches_bit(0, 0, false));
        assert!(img.matches_bit(0, 1, false), "(0, 1) is dark");
        assert!(!img.matches_bit(0, 1, true));

        for (x, y) in [(-1, 0), (0, -1), (-1, -1), (w, 0), (0, h), (w, h)] {
            assert!(!img.matches_bit(x, y, true), "({x}, {y}) is outside and must not match true");
            assert!(
                !img.matches_bit(x, y, false),
                "({x}, {y}) is outside and must not match false"
            );
        }
    }

    #[test]
    fn test_run_matches_naive_scan() {
        let img = sketch(&ROWS);
        for y in 0..img.h {
            for x in 0..img.w {
                let (bit, len) = img.run(x, y).unwrap();
                assert_eq!(bit, img.get_bit(x, y).unwrap(), "run bit at ({x}, {y})");

                let mut naive = 0;
                while x + naive < img.w && img.get_bit(x + naive, y) == Some(bit) {
                    naive += 1;
                }
                assert_eq!(len, naive, "run length at ({x}, {y})");
            }
        }
    }

    #[test]
    fn test_run_lengths_and_bounds() {
        let img = sketch(&ROWS);
        assert_eq!(img.run(0, 0), Some((true, 2)), "two light pixels, then a flip");
        assert_eq!(img.run(2, 0), Some((false, 3)), "the three dark pixels");
        assert_eq!(img.run(5, 0), Some((true, 3)), "clamped at the row edge");
        assert_eq!(img.run(0, 2), Some((false, 8)), "a fully dark row is one run");
        assert_eq!(img.run(0, 3), Some((true, 7)), "stops at the dark last pixel");

        assert_eq!(img.run(img.w, 0), None, "out of bounds");
        assert_eq!(img.run(0, img.h), None);
    }
}

// Flood fill related functions
impl BinaryImage {
    pub(crate) fn get_contour_capped(
        &mut self,
        src: (u32, u32),
        probe: (u32, u32),
        max_width: u32,
    ) -> Option<&mut Contour> {
        if self.w <= src.0 || self.h <= src.1 || self.w <= probe.0 || self.h <= probe.1 {
            return None;
        }

        let seed = Point { x: src.0 as i32, y: src.1 as i32 };
        let probe = Point { x: probe.0 as i32, y: probe.1 as i32 };
        let max_perimeter = max_width * 4;

        match self.get_px_contour(src.0, src.1) {
            None => trace(self, None, seed, probe, max_width),
            Some(id) => {
                let contour =
                    self.contours.get(id as usize).expect("No contour found for visited pixel");

                // Regardless of whether the contour bailed or not, if its perimeter is over limit
                // we exit
                if contour.perimeter() > max_perimeter || contour.extent() > max_width {
                    return None;
                }

                // If perimeter is within limit and contour bailed then we retrace it
                if contour.bailed {
                    return trace(self, Some(id), seed, probe, max_width);
                }

                // Lastly, if perimeter is within limits and the contour completed the loop, we check
                // whether the probe is within the contour bounds
                if !contour.contains(&probe) {
                    return None;
                }

                let contour =
                    self.contours.get_mut(id as usize).expect("No contour found for visited pixel");
                (contour.area() > 0).then_some(contour)
            }
        }
    }
}

// Constants
//------------------------------------------------------------------------------

// Number of blocks the shorter dimension of image should be divided into
const BLOCK_COUNT: f64 = 20.0;

pub const UNLABELED: u16 = u16::MAX;

// Number of blocks along row/col in a grid
const BLOCK_GRID_SIZE: usize = 5;
