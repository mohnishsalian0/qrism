use std::collections::VecDeque;

use image::{GenericImageView, Pixel as ImgPixel, RgbImage};

use crate::metadata::Color;
use crate::utils::BitMatrix;

use super::utils::accumulate::AreaAndCentreLocator;
use super::utils::{
    accumulate::{Accumulator, Row},
    geometry::Point,
};

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
    px_reg: Vec<u16>,     // Region each pixel belongs to
    regions: Vec<Region>, // Areas of visited regions. Index is id
    pub w: u32,
    pub h: u32,
    color_size: u32, // Bit len of color. B&W=1, multicolor=4
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
    pub fn prepare<I>(img: &I) -> Self
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

        let mut stats = vec![[Stat::new(); 4]; len];

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
                    stats[idx][i].accumulate(val);
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
                        stats[idx][i].accumulate(val);
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
                        stats[idx][i].accumulate(val);
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
                        stats[len - 1][i].accumulate(val);
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
        let color_size = chan_count.next_power_of_two() as u32;
        let mut buffer = BitMatrix::new(w, h);
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
                    buffer.put_bits(x, y, color_byte, color_size);
                }
            }
        }

        let px_reg = vec![u16::MAX; (w * h) as usize];
        let regions = Vec::with_capacity(100);
        Self { buffer, px_reg, regions, w, h, color_size }
    }

    /// Performs absolute/naive binarization
    pub fn global_thresholding(img: RgbImage) -> Self {
        let (w, h) = img.dimensions();
        let mut buffer = BitMatrix::new(w, h);
        let px_reg = vec![u16::MAX; (w * h) as usize];
        let color_size = 4;

        for (x, y, p) in img.enumerate_pixels() {
            let r = (p[0] > 127) as u8;
            let g = (p[1] > 127) as u8;
            let b = (p[2] > 127) as u8;
            let color_byte = (r << 2 | g << 1 | b) as u64;
            buffer.put_bits(x, y, color_byte, color_size);
        }
        Self { buffer, px_reg, regions: Vec::with_capacity(100), w, h, color_size }
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
        let color_size = chan_count.next_power_of_two() as u32;
        let mut buffer = BitMatrix::new(w, h);
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
                    buffer.put_bits(x, y, color_byte, color_size);
                }
            }
        }

        let px_reg = vec![u16::MAX; (w * h) as usize];
        let regions = Vec::with_capacity(100);
        Self { buffer, px_reg, regions, w, h, color_size }
    }
}

// Util functions
impl BinaryImage {
    pub fn get(&self, x: u32, y: u32) -> Option<Color> {
        if x >= self.w || y >= self.h {
            return None;
        }
        let bits = self.buffer.get_bits(x, y, self.color_size);
        Some(if self.color_size == 1 { Color::from(bits != 0) } else { bits.try_into().ok()? })
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

    pub fn get_at_point(&self, pt: &Point) -> Option<Color> {
        let (x, y) = self.wrap_coords(pt.x, pt.y)?;
        let bits = self.buffer.get_bits(x, y, self.color_size);
        Some(if self.color_size == 1 { Color::from(bits != 0) } else { bits.try_into().ok()? })
    }

    /// Flood-fill region label at (x, y), or None if unlabeled or out of bounds.
    pub fn get_region_id(&self, x: u32, y: u32) -> Option<u16> {
        if x >= self.w || y >= self.h {
            return None;
        }

        let id = self.px_reg[(y * self.w + x) as usize];
        (id != u16::MAX).then_some(id)
    }

    pub fn set_region_id(&mut self, x: u32, y: u32, reg_id: u16) {
        if x < self.w && y < self.h {
            self.px_reg[(y * self.w + x) as usize] = reg_id;
        }
    }

    #[cfg(test)]
    pub fn save(&self, path: &Path) -> ImageResult<()> {
        let mut img = RgbImage::new(self.w, self.h);
        for y in 0..self.h {
            for x in 0..self.w {
                let bits = self.buffer.get_bits(x, y, self.color_size);
                let rgb = if self.color_size == 1 {
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
    pub(crate) fn get_region(&mut self, src: (u32, u32)) -> &mut Region {
        let color = self.get(src.0, src.1).unwrap();
        let reg = self.get_region_id(src.0, src.1);

        match reg {
            None => {
                let reg_id = self.regions.len();
                debug_assert!(reg_id < u16::MAX as usize, "Number of regions exceed 65,535 (u16)");

                let acl = AreaAndCentreLocator::new();
                let acl = self.fill_and_accumulate(src, reg_id as u16, acl);
                let new_reg = Region {
                    id: reg_id,
                    src,
                    color,
                    area: acl.area,
                    centre: acl.get_centre(),
                    is_finder: false,
                };

                self.regions.push(new_reg);

                self.regions.get_mut(reg_id).expect("Region not found after saving")
            }
            Some(id) => {
                self.regions.get_mut(id as usize).expect("No region found for visited pixel")
            }
        }
    }

    /// Fills region with provided color and accumulates info
    pub fn fill_and_accumulate<A: Accumulator>(
        &mut self,
        src: (u32, u32),
        target: u16,
        mut acc: A,
    ) -> A {
        let clr = self.get(src.0, src.1).unwrap();

        // Flood fill algorithm
        let w = self.w;
        let h = self.h;
        let mut queue = VecDeque::new();
        queue.push_back(src);

        while let Some(pt) = queue.pop_front() {
            let (x, y) = pt;
            let mut left = x;
            let mut right = x;

            // Already claimed by this fill (via another seed) — skip; the run is done.
            if self.get_region_id(x, y).is_some() {
                continue;
            }

            self.set_region_id(x, y, target);

            // Travel left till boundary
            while left > 0 && self.get(left - 1, y).unwrap() == clr {
                left -= 1;
                self.set_region_id(left, y, target);
            }

            // Travel right till boundary
            while right < w - 1 && self.get(right + 1, y).unwrap() == clr {
                right += 1;
                self.set_region_id(right, y, target);
            }

            acc.accumulate(Row { left, right, y });

            for ny in [y.saturating_sub(1), y + 1] {
                if ny != y && ny < h {
                    let mut seg_len = 0;
                    for x in left..=right {
                        let nclr = self.get(x, ny).unwrap();
                        if nclr == clr {
                            seg_len += 1;
                        } else if seg_len > 0 {
                            queue.push_back((x - 1, ny));
                            seg_len = 0;
                        }
                    }
                    if seg_len > 0 {
                        queue.push_back((right, ny));
                    }
                }
            }
        }
        acc
    }
}

// Constants
//------------------------------------------------------------------------------

// Number of blocks the shorter dimension of image should be divided into
const BLOCK_COUNT: f64 = 20.0;

// Number of blocks along row/col in a grid
const BLOCK_GRID_SIZE: usize = 5;
