use std::collections::VecDeque;

use image::{GenericImageView, Luma, Pixel as ImgPixel, Rgb, RgbImage};

use crate::metadata::Color;

use super::utils::accumulate::AreaAndCentreLocator;
use super::utils::{
    accumulate::{Accumulator, Row},
    geometry::Point,
};

#[cfg(test)]
use std::path::Path;

#[cfg(test)]
use image::ImageResult;

// Pixel
//------------------------------------------------------------------------------

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum Pixel {
    Visited(usize, Color), // Contains id of associated region
    Unvisited(Color),      // Default tag
}

impl From<Pixel> for Rgb<u8> {
    fn from(p: Pixel) -> Self {
        match p {
            Pixel::Visited(_, c) | Pixel::Unvisited(c) => c.into(),
        }
    }
}

impl Pixel {
    pub fn get_id(&self) -> Option<usize> {
        match self {
            Pixel::Visited(id, _) => Some(*id),
            _ => None,
        }
    }

    pub fn get_color(&self) -> Color {
        match self {
            Pixel::Visited(_, c) => *c,
            Pixel::Unvisited(c) => *c,
        }
    }
}

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

// Binarize trait for pixel type in image crate
//------------------------------------------------------------------------------

pub trait Binarize {
    fn binarize(value: u8) -> Color;
}

impl Binarize for Rgb<u8> {
    fn binarize(value: u8) -> Color {
        Color::try_from(value).unwrap()
    }
}

impl Binarize for Luma<u8> {
    fn binarize(value: u8) -> Color {
        let value = value != 0;
        Color::from(value)
    }
}

// Image type for reader
//------------------------------------------------------------------------------

#[derive(Debug)]
pub struct BinaryImage {
    pub buffer: Vec<Pixel>,
    regions: Vec<Region>, // Areas of visited regions. Index is id
    pub w: u32,
    pub h: u32,
}

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
    pub fn binarize<I>(img: &I) -> Self
    where
        I: GenericImageView,
        I::Pixel: ImgPixel<Subpixel = u8> + Binarize,
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
        // If variance is low (<= 24), assume the block is white. Because there is a high chance
        // that the block is outside the qr. Unless the block has top/left neighbors, in which
        // case take average of them.
        let wsteps = wsteps as usize;
        let hsteps = hsteps as usize;
        let block_area_pow = 2 * block_pow;
        for i in 0..len {
            for j in 0..chan_count {
                if stats[i][j].max - stats[i][j].min <= 24 {
                    stats[i][j].avg = (stats[i][j].min as usize) / 2;
                    if i > wsteps && i % wsteps > 0 {
                        // Average of neighbors 2 * (x-1, y), (x, y-1), (x-1, y-1)
                        let left = stats[i - 1][j].avg;
                        let top = stats[i - wsteps][j].avg;
                        let top_left = stats[i - wsteps - 1][j].avg;
                        let ng_avg = (2 * left + top + top_left) / 4;
                        if stats[i][j].min < ng_avg as u8 {
                            stats[i][j].avg = ng_avg;
                        }
                    }
                } else {
                    // Convert block sum to average (divide by 64)
                    stats[i][j].avg >>= block_area_pow;
                }
            }
        }

        // Calculates threshold for blocks
        let half_grid = IMAGE_GRID_SIZE / 2;
        let grid_area = IMAGE_GRID_SIZE * IMAGE_GRID_SIZE;
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
        let mut buffer = vec![Pixel::Unvisited(Color::White); (w * h) as usize];
        for y in 0..h {
            let row_off = y * w;
            let thresh_row_off = (y as usize >> block_pow) * wsteps;
            for x in 0..w {
                let p = img.get_pixel(x, y);

                let idx = (row_off + x) as usize;
                let xsteps = x as usize >> block_pow;
                let thresh_idx = thresh_row_off + xsteps;

                let mut color_byte = 0;
                for (i, &val) in p.channels().iter().rev().enumerate() {
                    if val > threshold[thresh_idx][i] {
                        color_byte |= 1 << i;
                    }
                }

                let color = <I::Pixel>::binarize(color_byte);
                if color != Color::White {
                    buffer[idx] = Pixel::Unvisited(color);
                }
            }
        }

        let regions = Vec::with_capacity(100);
        Self { buffer, regions, w, h }
    }

    /// Performs absolute/naive binarization
    pub fn simple_thresholding(img: RgbImage) -> Self {
        let (w, h) = img.dimensions();
        let mut buffer = Vec::with_capacity((w * h) as usize);

        for p in img.pixels() {
            let r = (p[0] > 127) as u8;
            let g = (p[1] > 127) as u8;
            let b = (p[2] > 127) as u8;
            let np = Color::try_from(r << 2 | g << 1 | b).unwrap();
            buffer.push(Pixel::Unvisited(np));
        }
        Self { buffer, regions: Vec::with_capacity(100), w, h }
    }

    pub fn get(&self, x: u32, y: u32) -> Option<Pixel> {
        let w = self.w;
        let h = self.h;

        if x >= w || y >= h {
            return None;
        }

        let idx = (y * w + x) as usize;
        Some(self.buffer[idx])
    }

    fn coord_to_index(&self, x: i32, y: i32) -> Option<usize> {
        let w = self.w as i32;
        let h = self.h as i32;

        if x < -w || w <= x || y < -h || h <= y {
            return None;
        }

        let x = if x < 0 { x + w } else { x };
        let y = if y < 0 { y + h } else { y };

        Some((y * w + x) as _)
    }

    pub fn get_at_point(&self, pt: &Point) -> Option<&Pixel> {
        let idx = self.coord_to_index(pt.x, pt.y)?;
        Some(&self.buffer[idx])
    }

    pub fn get_mut(&mut self, x: u32, y: u32) -> Option<&mut Pixel> {
        let w = self.w;
        let h = self.h;

        if x >= w || y >= h {
            return None;
        }

        let idx = (y * w + x) as usize;
        Some(&mut self.buffer[idx])
    }

    pub fn get_mut_at_point(&mut self, pt: &Point) -> Option<&mut Pixel> {
        let idx = self.coord_to_index(pt.x, pt.y)?;
        Some(&mut self.buffer[idx])
    }

    pub fn set(&mut self, x: u32, y: u32, px: Pixel) {
        if let Some(pt) = self.get_mut(x, y) {
            *pt = px;
        }
    }

    pub fn set_at_point(&mut self, pt: &Point, px: Pixel) {
        if let Some(pt) = self.get_mut_at_point(pt) {
            *pt = px;
        }
    }

    #[cfg(test)]
    pub fn save(&self, path: &Path) -> ImageResult<()> {
        let w = self.w;
        let mut img = RgbImage::new(w, self.h);
        for (i, p) in self.buffer.iter().enumerate() {
            let i = i as u32;
            let (x, y) = (i % w, i / w);
            img.put_pixel(x, y, (*p).into());
        }
        img.save(path).unwrap();
        Ok(())
    }

    pub(crate) fn get_region(&mut self, src: (u32, u32)) -> &mut Region {
        let px = self.get(src.0, src.1).unwrap();

        match px {
            Pixel::Unvisited(color) => {
                let reg_id = self.regions.len();

                let acl = AreaAndCentreLocator::new();
                let to = Pixel::Visited(reg_id, color);
                let acl = self.fill_and_accumulate(src, to, acl);
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
            Pixel::Visited(id, _) => {
                self.regions.get_mut(id).expect("No region found for visited pixel")
            }
        }
    }

    /// Fills region with provided color and accumulates info
    pub fn fill_and_accumulate<A: Accumulator>(
        &mut self,
        src: (u32, u32),
        target: Pixel,
        mut acc: A,
    ) -> A {
        let from = self.get(src.0, src.1).unwrap();

        debug_assert!(from != target, "Cannot fill same color: From {from:?}, To {target:?}");

        // Flood fill algorithm
        let w = self.w;
        let h = self.h;
        let mut queue = VecDeque::new();
        queue.push_back(src);

        while let Some(pt) = queue.pop_front() {
            let (x, y) = pt;
            let mut left = x;
            let mut right = x;
            self.set(x, y, target);

            // Travel left till boundary
            while left > 0 && self.get(left - 1, y).unwrap() == from {
                left -= 1;
                self.set(left, y, target);
            }

            // Travel right till boundary
            while right < w - 1 && self.get(right + 1, y).unwrap() == from {
                right += 1;
                self.set(right, y, target);
            }

            acc.accumulate(Row { left, right, y });

            for ny in [y.saturating_sub(1), y + 1] {
                if ny != y && ny < h {
                    let mut seg_len = 0;
                    for x in left..=right {
                        let px = self.get(x, ny).unwrap();
                        if px == from {
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

// Number of blocks along shorter dimension of image
const BLOCK_COUNT: f64 = 20.0;

const IMAGE_GRID_SIZE: usize = 5;
