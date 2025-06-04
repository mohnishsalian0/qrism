use std::collections::VecDeque;

use image::{GrayImage, Luma, Rgb, RgbImage};

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
    Reserved(Color),       // Reserved pixels for functional patterns and infos
}

impl From<Pixel> for Color {
    fn from(p: Pixel) -> Self {
        match p {
            Pixel::Visited(_, c) => c,
            Pixel::Unvisited(c) => c,
            Pixel::Reserved(c) => c,
        }
    }
}

impl From<Rgb<u8>> for Pixel {
    fn from(p: Rgb<u8>) -> Self {
        let color = Color::try_from(p).unwrap();
        Pixel::Unvisited(color)
    }
}

impl From<Luma<u8>> for Pixel {
    fn from(p: Luma<u8>) -> Self {
        let color = Color::try_from(p).unwrap();
        Pixel::Unvisited(color)
    }
}

impl From<Pixel> for Rgb<u8> {
    fn from(p: Pixel) -> Self {
        match p {
            Pixel::Visited(_, c) | Pixel::Unvisited(c) | Pixel::Reserved(c) => c.into(),
        }
    }
}

// Region
//------------------------------------------------------------------------------

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Region {
    pub src: (u32, u32),
    pub centre: Point,
    pub area: u32,
    pub color: Color,
    pub is_finder: bool,
}

// Binarize trait for rgb & grayscale image
// Steps:
// 1. Divides image into blocks of 8x8 pixels. Note: For the last fractional block is, the
//    last 8 pixels are considered. So few pixels might overlap with last 2 blocks
// 2. Calculates average of each block
// 3. Calculates the threshold for each block by averaging 5x5 block around the current block if
//    the block is near an edge or a corner, the window is shifted accordingly.
// 4. Sets pixel value as false if less than or equal to threshold, else true
// Note: If the pixel value is equal to threshold, it is set as false for the edge case when
// threshold is 0 in which case the pixel should be false/black
//------------------------------------------------------------------------------

pub trait Binarize {
    fn binarize(&self) -> Vec<Pixel>;
}

impl Binarize for RgbImage {
    fn binarize(&self) -> Vec<Pixel> {
        let (w, h) = self.dimensions();

        let wsteps = (w + 7) >> 3;
        let hsteps = (h + 7) >> 3;
        let len = (wsteps * hsteps) as usize;

        let mut avg = vec![[0usize; 3]; len];
        let mut min = vec![[u8::MAX; 3]; len];
        let mut max = vec![[u8::MIN; 3]; len];

        // Calculate sum of 8x8 pixels for each block
        // Skip last few pixels which form fractional blocks. The last block will be computed later
        // Round w and h to skips these pixels
        let (wr, hr) = (w & !0b111, h & !0b111);
        for y in 0..hr {
            let row_off = (y >> 3) * wsteps;
            for x in 0..wr {
                let idx = (row_off + (x >> 3)) as usize;
                let px = self.get_pixel(x, y);
                for c in 0..3 {
                    avg[idx][c] += px[c] as usize;
                    min[idx][c] = std::cmp::min(min[idx][c], px[c]);
                    max[idx][c] = std::cmp::max(max[idx][c], px[c]);
                }
            }
        }

        // Sum of 8x8 pixels for fractional blocks (if exists) on the right edge
        if w & 0b111 != 0 {
            for y in 0..hr {
                let idx = (((y >> 3) + 1) * wsteps - 1) as usize;
                for x in w - 8..w {
                    let px = self.get_pixel(x, y);
                    for c in 0..3 {
                        avg[idx][c] += px[c] as usize;
                        min[idx][c] = std::cmp::min(min[idx][c], px[c]);
                        max[idx][c] = std::cmp::max(max[idx][c], px[c]);
                    }
                }
            }
        }

        // Sum of 8x8 pixels for fractional blocks (if exists) on the bottom edge
        if h & 0b111 != 0 {
            let last_row = wsteps * (hsteps - 1);
            for y in h - 8..h {
                for x in 0..wr {
                    let idx = (last_row + (x >> 3)) as usize;
                    let px = self.get_pixel(x, y);
                    for c in 0..3 {
                        avg[idx][c] += px[c] as usize;
                        min[idx][c] = std::cmp::min(min[idx][c], px[c]);
                        max[idx][c] = std::cmp::max(max[idx][c], px[c]);
                    }
                }
            }
        }

        // Sum of 8x8 pixels for fractional blocks (if exists) on the bottom right corner
        if w & 0b111 != 0 && h & 0b111 != 0 {
            for y in h - 8..h {
                for x in w - 8..w {
                    let px = self.get_pixel(x, y);
                    for c in 0..3 {
                        avg[len - 1][c] += px[c] as usize;
                        min[len - 1][c] = std::cmp::min(min[len - 1][c], px[c]);
                        max[len - 1][c] = std::cmp::max(max[len - 1][c], px[c]);
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
        for i in 0..len {
            let (mn, mx) = (min[i], max[i]);
            for c in 0..3 {
                if mx[c] - mn[c] <= 24 {
                    avg[i][c] = (mn[c] as usize) / 2;
                    if i > wsteps && i % wsteps > 0 {
                        // Average of neighbors 2 * (x-1, y), (x, y-1), (x-1, y-1)
                        let ng_avg =
                            (2 * avg[i - 1][c] + avg[i - wsteps][c] + avg[i - wsteps - 1][c]) / 4;
                        if mn[c] < ng_avg as u8 {
                            avg[i][c] = ng_avg;
                        }
                    }
                } else {
                    // Convert 8×8 sum to average (divide by 64)
                    avg[i][c] >>= 6;
                }
            }
        }

        // Calculates threshold for blocks
        let (maxx, maxy) = (wsteps - 2, hsteps - 2);
        let mut threshold = vec![[0u8; 3]; wsteps * hsteps];

        for y in 0..hsteps {
            let row_off = y * wsteps;
            for x in 0..wsteps {
                let i = row_off + x;

                // If y is near any boundary then copy the threshold above
                if y > 0 && (y <= 2 || y >= maxy) {
                    threshold[i] = threshold[i - wsteps];
                    continue;
                }

                // If x is near any boundary then copy the left threshold
                if x > 0 && (x <= 2 || x >= maxx) {
                    threshold[i] = threshold[i - 1];
                    continue;
                }

                let cx = std::cmp::max(x, 2);
                let cy = std::cmp::max(y, 2);
                let mut sum = [0usize; 3];
                for ny in cy - 2..=cy + 2 {
                    let ni = ny * wsteps + cx;
                    for a in &avg[ni - 2..=ni + 2] {
                        for c in 0..3 {
                            sum[c] += a[c];
                        }
                    }
                }

                for (c, t) in threshold[i].iter_mut().enumerate() {
                    *t = (sum[c] / 25) as u8;
                }
            }
        }

        // Initially mark all pixels as unvisited; will be used for flood fill later.
        let mut res = vec![Pixel::Unvisited(Color::Black); (w * h) as usize];

        for y in 0..h {
            let row_off = y * w;
            let thresh_row_off = (y as usize >> 3) * wsteps;
            for x in 0..w {
                let p = self.get_pixel(x, y);

                let idx = (row_off + x) as usize;

                let xsteps = x as usize >> 3;
                let thresh_idx = thresh_row_off + xsteps;

                let mut color = Color::Black;
                for c in 0..3 {
                    if p[c] > threshold[thresh_idx][c] {
                        let byte = color as u8 | 1 << (2 - c);
                        color = Color::try_from(byte).unwrap();
                    }
                }

                if color != Color::Black {
                    res[idx] = Pixel::Unvisited(color);
                }
            }
        }

        res
    }
}

impl Binarize for GrayImage {
    fn binarize(&self) -> Vec<Pixel> {
        let (w, h) = self.dimensions();

        let wsteps = (w + 7) >> 3;
        let hsteps = (h + 7) >> 3;
        let len = (wsteps * hsteps) as usize;

        // Calculates block average
        let mut avg = vec![0usize; len];
        let mut min_max = vec![(u8::MAX, u8::MIN); len];

        // Divide image into blocks of 8x8 pixels and calculate sum of values for each block.
        // Skip last few pixels which form fractional blocks. The last block will be computed later
        // Round w and h to skips these pixels
        let (wr, hr) = (w & !0b111, h & !0b111);
        for y in 0..hr {
            let row_off = (y >> 3) * wsteps;
            for x in 0..wr {
                let xsteps = x >> 3;
                let idx = (row_off + xsteps) as usize;

                let p = self.get_pixel(x, y)[0];
                avg[idx] += p as usize;
                min_max[idx].0 = std::cmp::min(min_max[idx].0, p);
                min_max[idx].1 = std::cmp::max(min_max[idx].1, p);
            }
        }

        // Sum of 8x8 pixels for fractional blocks (if exists) on the right edge
        if w & 0b111 != 0 {
            for y in 0..hr {
                let idx = (((y >> 3) + 1) * wsteps - 1) as usize;
                for x in w - 8..w {
                    let p = self.get_pixel(x, y)[0];

                    avg[idx] += p as usize;
                    min_max[idx].0 = std::cmp::min(min_max[idx].0, p);
                    min_max[idx].1 = std::cmp::max(min_max[idx].1, p);
                }
            }
        }

        // Sum of 8x8 pixels for fractional blocks (if exists) on the bottom edge
        if h & 0b111 != 0 {
            let last_row = wsteps * (hsteps - 1);
            for y in h - 8..h {
                for x in 0..wr {
                    let xsteps = x >> 3;
                    let idx = (last_row + xsteps) as usize;

                    let p = self.get_pixel(x, y)[0];
                    avg[idx] += p as usize;
                    min_max[idx].0 = std::cmp::min(min_max[idx].0, p);
                    min_max[idx].1 = std::cmp::max(min_max[idx].1, p);
                }
            }
        }

        // Sum of 8x8 pixels for fractional blocks (if exists) on the bottom right corner
        if w & 0b111 != 0 && h & 0b111 != 0 {
            for y in h - 8..h {
                for x in w - 8..w {
                    let p = self.get_pixel(x, y)[0];

                    avg[len - 1] += p as usize;
                    min_max[len - 1].0 = std::cmp::min(min_max[len - 1].0, p);
                    min_max[len - 1].1 = std::cmp::max(min_max[len - 1].1, p);
                }
            }
        }

        // Take average from the sum calculated for each block
        // If variance is low (<= 24), assume the block is white. Because there is a high chance
        // that the block is outside the qr. Unless the block has top/left neighbors, in which
        // case take average of them.
        let wsteps = wsteps as usize;
        let hsteps = hsteps as usize;
        for i in 0..len {
            let (mn, mx) = min_max[i];
            if mx - mn <= 24 {
                avg[i] = (mn as usize) / 2;
                if i > wsteps && i % wsteps > 0 {
                    // Average of neighbors 2 * (x-1, y), (x, y-1), (x-1, y-1)
                    let ng_avg = (2 * avg[i - 1] + avg[i - wsteps] + avg[i - wsteps - 1]) / 4;
                    if mn < ng_avg as u8 {
                        avg[i] = ng_avg;
                    }
                }
            } else {
                // Convert 8×8 sum to average (divide by 64)
                avg[i] >>= 6;
            }
        }

        // Calculates threshold for each block
        let (maxx, maxy) = (wsteps - 2, hsteps - 2);
        let mut threshold = vec![0u8; wsteps * hsteps];

        for y in 0..hsteps {
            let row_off = y * wsteps;
            for x in 0..wsteps {
                let i = row_off + x;

                // If y is near any boundary then copy the threshold above
                if y > 0 && (y <= 2 || y >= maxy) {
                    threshold[i] = threshold[i - wsteps];
                    continue;
                }

                // If x is near any boundary then copy the left threshold
                if x > 0 && (x <= 2 || x >= maxx) {
                    threshold[i] = threshold[i - 1];
                    continue;
                }

                let cx = std::cmp::max(x, 2);
                let cy = std::cmp::max(y, 2);
                let mut sum = 0usize;
                for ny in cy - 2..=cy + 2 {
                    let ni = ny * wsteps + cx;
                    sum += avg[ni - 2..=ni + 2].iter().sum::<usize>();
                }

                threshold[i] = (sum / 25) as u8;
            }
        }

        // Initially mark all pixels as unvisited; will be used for flood fill later.
        let mut res = vec![Pixel::Unvisited(Color::White); (w * h) as usize];
        for y in 0..h {
            let row_off = y * w;
            let thresh_row_off = (y as usize >> 3) * wsteps;
            for x in 0..w {
                let p = self.get_pixel(x, y)[0];

                let idx = (row_off + x) as usize;

                let xsteps = x as usize >> 3;
                let thresh_idx = thresh_row_off + xsteps;

                if p <= threshold[thresh_idx] {
                    res[idx] = Pixel::Unvisited(Color::Black);
                }
            }
        }

        res
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
    /// Performs adaptive binarization on an RGB image using a sliding window
    /// and per-channel average filtering.
    pub fn prepare(img: &GrayImage) -> Self {
        let (w, h) = img.dimensions();
        let buffer = img.binarize();
        let regions = Vec::with_capacity(100);
        Self { buffer, regions, w, h }
    }

    pub fn prepare_rgb(img: &RgbImage) -> Self {
        let (w, h) = img.dimensions();
        let buffer = img.binarize();
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

    pub(crate) fn get_region(&mut self, src: (u32, u32)) -> Option<&mut Region> {
        let px = self.get(src.0, src.1).unwrap();

        match px {
            Pixel::Unvisited(color) => {
                let reg_id = self.regions.len();

                let acl = AreaAndCentreLocator::new();
                let to = Pixel::Visited(reg_id, color);
                let acl = self.fill_and_accumulate(src, to, acl);
                let new_reg = Region {
                    src,
                    color,
                    area: acl.area,
                    centre: acl.get_centre(),
                    is_finder: false,
                };

                self.regions.push(new_reg);

                Some(self.regions.get_mut(reg_id).expect("Region not found after saving"))
            }
            Pixel::Visited(id, _) => {
                Some(self.regions.get_mut(id).expect("No region found for visited pixel"))
            }
            _ => None,
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
