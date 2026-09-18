use image::{GenericImageView, Pixel as ImgPixel, RgbImage};

use crate::metadata::Color;
use crate::reader::utils::contour::{trace, Contour};
use crate::utils::BitMatrix;

use super::utils::geometry::Point;

#[cfg(test)]
use std::path::Path;

#[cfg(test)]
use image::ImageResult;

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
    pub buffer: BitMatrix,
    px_cont: Vec<u16>,      // Contour each boundary pixel belongs to
    contours: Vec<Contour>, // Visited contours, index is id
    pub w: u32,
    pub h: u32,
    pass: u32, // Used to mark alignment pattern
}

// Binarizing functions
impl BinaryImage {
    // Steps:
    // 1. Divides image into blocks of 8x8 pixels. Note: For the last fractional block is, the
    //    last 8 pixels are considered. So few pixels might overlap with last 2 blocks
    // 2. Calculates average of each block
    // 3. Calculates the threshold for each block by averaging 5x5 block around the current block if
    //    the block is near an edge or a corner, the window is shifted accordingly.
    // 4. Sets pixel value as false if less than or equal to threshold, else true
    // Note: If the pixel value is equal to threshold, it is set as false for the edge case when
    // threshold is 0 in which case the pixel should be false/black
    pub fn prepare<P>(img: &image::ImageBuffer<P, Vec<u8>>) -> Self
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
        let (maxx, maxy) = (wsteps - half_grid, hsteps - half_grid);
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

    /// Performs absolute/naive binarization
    pub fn global_thresholding(img: RgbImage) -> Self {
        let (w, h) = img.dimensions();
        // Colour plane packs 4 bits per pixel; the matrix strides columns by it.
        let mut buffer = BitMatrix::new(w, h, 4);

        for (x, y, p) in img.enumerate_pixels() {
            let r = (p[0] > 127) as u8;
            let g = (p[1] > 127) as u8;
            let b = (p[2] > 127) as u8;
            let color_byte = (r << 2 | g << 1 | b) as u64;
            buffer.put(x, y, color_byte);
        }

        let px_cont = vec![u16::MAX; (w * h) as usize];
        let contours = Vec::with_capacity(100);
        Self { buffer, px_cont, contours, w, h, pass: 0 }
    }
}

// Otsu binarizing
//------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy)]
struct Histogram {
    h: [u32; 256],
    total: u32,
    min: u8,
    max: u8,
    is_block: bool,
}

impl Histogram {
    pub fn new(is_block: bool) -> Self {
        Histogram { h: [0; 256], total: 0, min: u8::MAX, max: u8::MIN, is_block }
    }

    pub fn accumulate(&mut self, val: u8) {
        self.h[val as usize] += 1;
        self.total += 1;
        self.min = self.min.min(val);
        self.max = self.max.max(val);
    }

    // Computes Otsu threshold
    fn threshold(&self) -> u8 {
        let dlen = 256.0;
        let min = self.min as usize;
        let max = self.max as usize;

        // Compute sum of normalized intensities
        let mut sum = 0.0;
        for i in min..=max {
            let f = i as f64 / dlen;
            sum += f * self.h[i] as f64;
        }

        let mut sumb = 0.0;
        let mut wb = 0; // Count of background pixels
        let mut max_variance = 0.0;
        let mut best_mb = 0.0; // Best background mean
        let mut best_mf = 0.0; // Best foreground mean

        for i in min..max {
            wb += self.h[i];
            let wf = self.total - wb;

            let f = i as f64 / dlen;
            sumb += f * self.h[i] as f64;

            let mb = sumb / (wb as f64);
            let mf = (sum - sumb) / (wf as f64);

            let var_between = (wb as f64) * (wf as f64) * (mb - mf).powi(2);

            if var_between > max_variance {
                max_variance = var_between;
                best_mb = mb;
                best_mf = mf;
            }
        }

        // FIXME:
        // if !self.is_block && max >= min && max - min <= 60 {
        //     let avg = (max + min) / 2;
        //     if avg > 127 {
        //         return 0;
        //     }
        //     return 255;
        // }

        // Final threshold is average of both means, scaled back to 0..255
        let threshold_f = (best_mb + best_mf) / 2.0;

        (threshold_f * dlen).round() as u8
    }
}

impl BinaryImage {
    pub fn otsu<I>(img: &I) -> Self
    where
        I: GenericImageView,
        I::Pixel: ImgPixel<Subpixel = u8>,
    {
        let (w, h) = img.dimensions();
        let chan_count = I::Pixel::CHANNEL_COUNT as usize;
        let block_pow = (std::cmp::min(w, h) as f64 / BLOCK_COUNT).log2() as usize;
        let block_size = 1 << block_pow;
        let mask = (1 << block_pow) - 1;

        let wsteps = (w + mask) >> block_pow;
        let hsteps = (h + mask) >> block_pow;
        let len = (wsteps * hsteps) as usize;

        let mut histogram = vec![[Histogram::new(true); 4]; len];

        // Calculate sum of 8x8 pixels for each block
        // Skip last few pixels which form fractional blocks. The last block will be computed later
        // Round w and h to skips these pixels
        let (wr, hr) = (w & !mask, h & !mask);
        for y in 0..hr {
            let row_off = (y >> block_pow) * wsteps;
            for x in 0..wr {
                let idx = (row_off + (x >> block_pow)) as usize;

                let px = img.get_pixel(x, y);
                for (i, &val) in px.channels().iter().enumerate() {
                    histogram[idx][i].accumulate(val);
                }
            }
        }

        // Sum of 8x8 pixels for fractional blocks (if exists) on the right edge
        if w & mask != 0 {
            for y in 0..hr {
                let idx = (((y >> block_pow) + 1) * wsteps - 1) as usize;
                for x in w - block_size..w {
                    let px = img.get_pixel(x, y);
                    for (i, &val) in px.channels().iter().enumerate() {
                        histogram[idx][i].accumulate(val);
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

                    let px = img.get_pixel(x, y);
                    for (i, &val) in px.channels().iter().enumerate() {
                        histogram[idx][i].accumulate(val);
                    }
                }
            }
        }

        // Sum of 8x8 pixels for fractional blocks (if exists) on the bottom right corner
        if w & mask != 0 && h & mask != 0 {
            for y in h - block_size..h {
                for x in w - block_size..w {
                    let px = img.get_pixel(x, y);
                    for (i, &val) in px.channels().iter().enumerate() {
                        histogram[len - 1][i].accumulate(val);
                    }
                }
            }
        }

        // Calculates threshold for blocks
        let wsteps = wsteps as usize;
        let hsteps = hsteps as usize;
        let half_grid = BLOCK_GRID_SIZE / 2;
        let (maxx, maxy) = (wsteps - half_grid, hsteps - half_grid);
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
                let mut grid_hist = [Histogram::new(false); 4];
                for ny in cy - half_grid..=cy + half_grid {
                    let ni = ny * wsteps + cx;
                    for px_stat in &histogram[ni - half_grid..=ni + half_grid] {
                        for (i, chan_hist) in px_stat.iter().take(chan_count).enumerate() {
                            let block_thresh = chan_hist.threshold();
                            grid_hist[i].accumulate(block_thresh);
                        }
                    }
                }

                for (c, t) in threshold[i].iter_mut().take(chan_count).enumerate() {
                    let grid_thresh = grid_hist[c].threshold();
                    *t = grid_thresh;
                }
            }
        }

        // Initially mark all pixels as unvisited; will be used for flood fill later.
        // Colour plane packs `color_size` bits per pixel; the matrix strides columns by it.
        let color_size = chan_count.next_power_of_two() as u32;
        let mut buffer = BitMatrix::new(w, h, color_size);
        for y in 0..h {
            let thresh_row_off = (y as usize >> block_pow) * wsteps;
            for x in 0..w {
                let p = img.get_pixel(x, y);

                let xsteps = x as usize >> block_pow;
                let thresh_idx = thresh_row_off + xsteps;

                let mut color_byte = 0u64;
                for (i, &val) in p.channels().iter().enumerate() {
                    color_byte = (color_byte << 1) | u64::from(val > threshold[thresh_idx][i]);
                }

                if color_byte != 0 {
                    buffer.put(x, y, color_byte);
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

    // Colour at `(x, y)` plus the length of the run of that colour reaching to the row's right
    // edge. Lets a row scan step by whole runs; see `BitMatrix::run`.
    pub fn run(&self, x: u32, y: u32) -> Option<(Color, u32)> {
        if x >= self.w || y >= self.h {
            return None;
        }
        let (bits, len) = self.buffer.run(x, y);
        let color = if self.buffer.elem_bits() == 1 {
            Color::from(bits != 0)
        } else {
            bits.try_into().ok()?
        };
        Some((color, len))
    }

    pub fn get_bounded(&self, x: i32, y: i32) -> Option<Color> {
        if x < 0 || y < 0 {
            return None;
        }

        let (x, y) = (x as u32, y as u32);
        if self.w <= x || self.h <= y {
            return None;
        }

        let bits = self.buffer.get(x, y);
        Some(if self.buffer.elem_bits() == 1 {
            Color::from(bits != 0)
        } else {
            bits.try_into().ok()?
        })
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

    pub fn matches_bits(&self, x: i32, y: i32, bits: u64) -> bool {
        if x < 0 || y < 0 {
            return false;
        }
        let (x, y) = (x as u32, y as u32);
        x < self.w && y < self.h && self.buffer.get(x, y) == bits
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
                if contour.perimeter() > max_perimeter {
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
