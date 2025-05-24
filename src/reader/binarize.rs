use std::{cmp, collections::VecDeque, num::NonZeroUsize};

use image::Pixel as ImgPixel;
use image::{GenericImageView, GrayImage, Luma, Rgb, RgbImage};
use lru::LruCache;

use crate::metadata::Color;

use super::utils::{
    accumulate::{Accumulator, Area, Row},
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
    Visited(u8, Color), // Contains id of associated region
    Unvisited(Color),   // Default tag
    Candidate(Color),   // Candidate for finder
    Reserved(Color),    // Reserved pixels for functional patterns and infos
}

impl From<Pixel> for Color {
    fn from(p: Pixel) -> Self {
        match p {
            Pixel::Visited(_, c) => c,
            Pixel::Unvisited(c) => c,
            Pixel::Candidate(c) => c,
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
            Pixel::Visited(_, c)
            | Pixel::Unvisited(c)
            | Pixel::Reserved(c)
            | Pixel::Candidate(c) => c.into(),
        }
    }
}

// Region
//------------------------------------------------------------------------------

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct Region {
    pub src: (u32, u32),
    pub color: Color,
    pub area: u32,
}

// Binarize trait for rgb & grayscale image
// Steps:
// 1. Divides image into blocks of 8x8 pixels. Note: The last fractional block average is
//    calculated based on average of last 8 pixels. So few pixels might overlap into 2 blocks
// 2. Calculates average of each block
// 3. Calculates the threshold for each block by averaging 5x5 block around the current block
// 4. Sets binary pixel as false if pixel value less than or equal to threshold, else true
// Note: If the pixel value is equal to threshold, it is set as false for the edge case when
// threshold is 0 in which case the pixel should be false/black
//------------------------------------------------------------------------------

pub trait Binarize {
    fn binarize(&self) -> Vec<Pixel>;
    fn calculate_block_average(&self) -> Vec<usize>;
    fn calculate_threshold(&self, avg: &[usize]) -> Vec<u8>;
}

impl Binarize for RgbImage {
    fn binarize(&self) -> Vec<Pixel> {
        let (w, h) = self.dimensions();
        let win_sz = cmp::max(w / 8, 1);
        let mut u_avg = [0, 0, 0];
        let mut v_avg = [0, 0, 0];
        let mut row_avg = vec![[0, 0, 0]; w as usize];
        let mut buffer = Vec::with_capacity((w * h) as usize);

        for y in 0..h {
            for x in 0..w {
                let (u, v) = if y & 1 == 0 { (x, w - 1 - x) } else { (w - 1 - x, x) };
                let (u_usize, v_usize) = (u as usize, v as usize);
                let (pu, pv) = (self.get_pixel(u, y), self.get_pixel(v, y));

                for i in 0..3 {
                    u_avg[i] = u_avg[i] * (win_sz - 1) / win_sz + pu[i] as u32;
                    v_avg[i] = v_avg[i] * (win_sz - 1) / win_sz + pv[i] as u32;
                    row_avg[u_usize][i] += u_avg[i];
                    row_avg[v_usize][i] += v_avg[i];
                }
            }

            let den = 200 * win_sz;
            for x in 0..w {
                let mut px = *self.get_pixel(x, y);
                for (i, p) in px.0.iter_mut().enumerate() {
                    let thresh = row_avg[x as usize][i] * (100 - 5) / den;
                    if *p as u32 >= thresh {
                        *p = 255;
                    } else {
                        *p = 0;
                    }
                }
                buffer.push(px.into());
            }

            row_avg.fill([0, 0, 0]);
        }
        buffer
    }

    fn calculate_block_average(&self) -> Vec<usize> {
        todo!()
    }

    fn calculate_threshold(&self, avg: &[usize]) -> Vec<u8> {
        todo!()
    }
}

impl Binarize for GrayImage {
    // fn binarize(&self) -> Vec<Pixel> {
    //     let (w, h) = self.dimensions();
    //     let win_sz = cmp::max(w / 8, 1);
    //     let mut u_avg = 0;
    //     let mut v_avg = 0;
    //     let mut row_avg = vec![0; w as usize];
    //     let mut buffer = Vec::with_capacity((w * h) as usize);
    //
    //     for y in 0..h {
    //         for x in 0..w {
    //             let (u, v) = if y & 1 == 0 { (x, w - 1 - x) } else { (w - 1 - x, x) };
    //             let (u_usize, v_usize) = (u as usize, v as usize);
    //             let (pu, pv) = (self.get_pixel(u, y), self.get_pixel(v, y));
    //
    //             u_avg = u_avg * (win_sz - 1) / win_sz + pu[0] as u32;
    //             v_avg = v_avg * (win_sz - 1) / win_sz + pv[0] as u32;
    //             row_avg[u_usize] += u_avg;
    //             row_avg[v_usize] += v_avg;
    //         }
    //
    //         let den = 200 * win_sz;
    //         for x in 0..w {
    //             let mut px = *self.get_pixel(x, y);
    //             let thresh = row_avg[x as usize] * (100 - 5) / den;
    //             if px[0] as u32 >= thresh {
    //                 px[0] = 255;
    //             } else {
    //                 px[0] = 0;
    //             }
    //             buffer.push(px.into());
    //         }
    //
    //         row_avg.fill(0);
    //     }
    //     buffer
    // }

    fn binarize(&self) -> Vec<Pixel> {
        let (w, h) = self.dimensions();

        let mut wsteps = w >> 3;
        if w & 0b111 != 0 {
            wsteps += 1;
        }

        // Initially mark all pixels as unvisited; will be used for flood fill later.
        let mut res = vec![Pixel::Unvisited(Color::White); (w * h) as usize];

        let blk_avg = self.calculate_block_average();

        let thresh = self.calculate_threshold(&blk_avg);

        for y in 0..h {
            let row_off = y * w;
            let thresh_row_off = (y >> 3) * wsteps;
            for x in 0..w {
                let p = self.get_pixel(x, y)[0];

                let idx = (row_off + x) as usize;

                let xsteps = x >> 3;
                let thresh_idx = (thresh_row_off + xsteps) as usize;

                if p <= thresh[thresh_idx] {
                    res[idx] = Pixel::Unvisited(Color::Black);
                }
            }
        }

        res
    }

    fn calculate_block_average(&self) -> Vec<usize> {
        let (w, h) = self.dimensions();

        let mut wsteps = w >> 3;
        if w & 0b111 != 0 {
            wsteps += 1;
        }
        let mut hsteps = h >> 3;
        if h & 0b111 != 0 {
            hsteps += 1;
        }
        let len = (wsteps * hsteps) as usize;

        let mut avg = vec![0usize; len];
        let mut min_max = vec![(0u8, 0u8); len];

        // Calculate sum of 8x8 pixels for each block
        // Skip last few pixels which form fractional blocks. The last block will be computed later
        // Round w and h to skips these pixels
        let (wr, hr) = (w & !0b111, h & !0b111);
        for y in 0..hr {
            let row_off = (y >> 3) * wsteps;
            for x in 0..wr {
                let p = self.get_pixel(x, y)[0];

                let xsteps = x >> 3;
                let idx = (row_off + xsteps) as usize;

                avg[idx] += p as usize;
                min_max[idx].0 = std::cmp::min(min_max[idx].0, p);
                min_max[idx].1 = std::cmp::max(min_max[idx].1, p);
            }
        }

        // Calculate sum of 8x8 pixels for fractional blocks on the right edge
        for y in 0..h {
            let idx = (((y >> 3) + 1) * wsteps - 1) as usize;
            for x in w - 8..w {
                let p = self.get_pixel(x, y)[0];

                avg[idx] += p as usize;
                min_max[idx].0 = std::cmp::min(min_max[idx].0, p);
                min_max[idx].1 = std::cmp::max(min_max[idx].1, p);
            }
        }

        // Calculate sum of 8x8 pixels for fractional blocks on the bottom edge
        let last_row = wsteps * (hsteps - 1);
        for y in h - 8..h {
            for x in 0..w - 8 {
                let p = self.get_pixel(x, y)[0];

                let xsteps = x >> 3;
                let idx = (last_row + xsteps) as usize;

                avg[idx] += p as usize;
                min_max[idx].0 = std::cmp::min(min_max[idx].0, p);
                min_max[idx].1 = std::cmp::max(min_max[idx].1, p);
            }
        }

        // Take average from the sum calculated for each block
        // If variance is low (<= 24), assume the block is white. Unless the block has top/left
        // neighbor, in which case take average of them.
        let w = w as usize;
        for i in 0..len {
            let (mn, mx) = min_max[i];
            if mx - mn <= 24 {
                avg[i] = (mn as usize) / 2;
                if i > w {
                    // Average of neighbors (x-1, y), (x, y-1), (x-1, y-1)
                    let ng_avg = (2 * avg[i - 1] + avg[i - w] + avg[i - w - 1]) / 4;
                    if mn < ng_avg as u8 {
                        avg[i] = ng_avg;
                    }
                }
            } else {
                // Convert 8×8 sum to average (divide by 64)
                avg[i] >>= 6;
            }
        }

        avg
    }

    fn calculate_threshold(&self, avg: &[usize]) -> Vec<u8> {
        let (w, h) = self.dimensions();

        let mut wsteps = (w >> 3) as usize;
        if w & 0b111 != 0 {
            wsteps += 1;
        }
        let mut hsteps = (h >> 3) as usize;
        if h & 0b111 != 0 {
            hsteps += 1;
        }

        let (maxx, maxy) = (wsteps - 3, hsteps - 3);
        let mut res = vec![0u8; wsteps * hsteps];

        for y in 0..hsteps {
            let row_off = y * wsteps;
            let cy = y.clamp(2, maxy);
            for x in 0..wsteps {
                let i = row_off + x;
                let cx = x.clamp(2, maxx);
                let mut sum = 0usize;

                for ny in cy - 2..=cy + 2 {
                    let ni = ny * wsteps + cx;
                    sum += avg[ni - 2..=ni + 2].iter().sum::<usize>();
                }

                res[i] = (sum / 25) as u8;
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
    regions: LruCache<u8, Region>, // Areas of visited regions. Index is id
    pub w: u32,
    pub h: u32,
}

impl BinaryImage {
    /// Performs adaptive binarization on an RGB image using a sliding window
    /// and per-channel average filtering.
    pub fn prepare<I>(img: &I) -> Self
    where
        I: GenericImageView + Binarize,
        I::Pixel: ImgPixel<Subpixel = u8>,
    {
        let (w, h) = img.dimensions();
        let buffer = img.binarize();
        Self { buffer, regions: LruCache::new(NonZeroUsize::new(250).unwrap()), w, h }
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
        Self { buffer, regions: LruCache::new(NonZeroUsize::new(250).unwrap()), w, h }
    }

    pub fn get(&self, x: u32, y: u32) -> Option<Pixel> {
        let x = i32::try_from(x).expect("x coordinate exceeds i32::MAX");
        let y = i32::try_from(y).expect("y coordinate exceeds i32::MAX");

        let idx = self.coord_to_index(x, y)?;
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
        let x = i32::try_from(x).expect("x coordinate exceeds i32::MAX");
        let y = i32::try_from(y).expect("y coordinate exceeds i32::MAX");

        let idx = self.coord_to_index(x, y)?;
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

    pub(crate) fn get_region(&mut self, src: (u32, u32)) -> Option<Region> {
        let px = self.get(src.0, src.1).unwrap();

        match px {
            Pixel::Unvisited(color) => {
                let reg_count = self.regions.len() as u8;

                let reg_id = if reg_count == self.regions.cap().get() as u8 {
                    let (id, reg) = self.regions.pop_lru().expect("Cache is full");
                    let Region { src, color, .. } = reg;

                    let _ = self.fill_and_accumulate(src, Pixel::Unvisited(color), |_| ());

                    id
                } else {
                    reg_count
                };

                let area = Area(0);
                let to = Pixel::Visited(reg_id, color);
                let acc = self.fill_and_accumulate(src, to, area);
                let new_reg = Region { src, color, area: acc.0 };

                self.regions.put(reg_id, new_reg);

                Some(new_reg)
            }
            Pixel::Visited(id, _) => {
                Some(*self.regions.get(&id).expect("No region found for visited pixel"))
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

            // Traverse left till boundary
            while left > 0 && self.get(left - 1, y).unwrap() == from {
                left -= 1;
                self.set(left, y, target);
            }

            // Traverse right till boundary
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
