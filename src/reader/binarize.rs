use std::{cmp, collections::VecDeque, num::NonZeroUsize};

use image::{Rgb, RgbImage};
use lru::LruCache;

use crate::metadata::Color;

use super::utils::{
    accumulate::{Accumulator, Area, Row},
    geometry::Point,
};

// #[cfg(test)]
use std::path::Path;

// #[cfg(test)]
use image::ImageResult;

// Pixel
//------------------------------------------------------------------------------

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum Pixel {
    Visited(u8, Color), // Contains id of associated region
    Unvisited(Color),
    Temporary(Color), // Temporary tag before converting to candidate
    Candidate(Color), // Candidate for finder
    Reserved(Color),  // Reserved pixels for functional patterns and infos
}
impl From<Pixel> for Color {
    fn from(p: Pixel) -> Self {
        match p {
            Pixel::Visited(_, c) => c,
            Pixel::Unvisited(c) => c,
            Pixel::Temporary(c) => c,
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

impl From<Pixel> for Rgb<u8> {
    fn from(p: Pixel) -> Self {
        match p {
            Pixel::Visited(_, c)
            | Pixel::Unvisited(c)
            | Pixel::Temporary(c)
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

// QR type for reader
//------------------------------------------------------------------------------

#[derive(Debug)]
pub struct BinaryImage {
    pub buffer: Vec<Pixel>,
    regions: LruCache<u8, Region>, // Areas of visited regions. Index is id
    pub w: u32,
    pub h: u32,
}

impl BinaryImage {
    pub fn new(img: RgbImage) -> Self {
        let (w, h) = img.dimensions();
        let mut buffer = Vec::with_capacity((w * h) as usize);
        for &px in img.pixels() {
            buffer.push(px.into());
        }
        Self { buffer, regions: LruCache::new(NonZeroUsize::new(250).unwrap()), w, h }
    }

    /// Performs adaptive binarization on an RGB image using a sliding window
    /// and per-channel average filtering.
    pub fn prepare(img: RgbImage) -> Self {
        let (w, h) = img.dimensions();
        let win_sz = cmp::max(w / 8, 1);
        let mut u_avg = [0, 0, 0];
        let mut v_avg = [0, 0, 0];
        let mut row_avg = vec![[0, 0, 0]; w as usize];
        let mut buffer = Vec::with_capacity((w * h) as usize);

        for y in 0..h {
            for x in 0..w {
                let (u, v) = if y & 1 == 0 { (x, w - 1 - x) } else { (w - 1 - x, x) };
                let (u_usize, v_usize) = (u as usize, v as usize);
                let (pu, pv) = (img.get_pixel(u, y), img.get_pixel(v, y));

                for i in 0..3 {
                    u_avg[i] = u_avg[i] * (win_sz - 1) / win_sz + pu[i] as u32;
                    v_avg[i] = v_avg[i] * (win_sz - 1) / win_sz + pv[i] as u32;
                    row_avg[u_usize][i] += u_avg[i];
                    row_avg[v_usize][i] += v_avg[i];
                }
            }

            let den = 200 * win_sz;
            for x in 0..w {
                let mut px = *img.get_pixel(x, y);
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
        Self { buffer, regions: LruCache::new(NonZeroUsize::new(250).unwrap()), w, h }
    }

    /// Performs absolute binarization
    pub fn binarize(img: RgbImage) -> Self {
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

    pub fn get(&self, x: u32, y: u32) -> Pixel {
        let x = i32::try_from(x).expect("x coordinate exceeds i32::MAX");
        let y = i32::try_from(y).expect("y coordinate exceeds i32::MAX");

        let idx = self.coord_to_index(x, y);
        self.buffer[idx]
    }

    fn coord_to_index(&self, x: i32, y: i32) -> usize {
        let w = self.w as i32;
        let h = self.h as i32;
        debug_assert!(-w <= x && x < w, "row shouldn't be greater than or equal to w");
        debug_assert!(-h <= y && y < h, "column shouldn't be greater than or equal to w");

        let x = if x < 0 { x + w } else { x };
        let y = if y < 0 { y + h } else { y };
        (y * w + x) as _
    }

    pub fn get_at_point(&self, pt: &Point) -> &Pixel {
        let idx = self.coord_to_index(pt.x, pt.y);
        &self.buffer[idx]
    }

    pub fn get_mut(&mut self, x: u32, y: u32) -> &mut Pixel {
        let x = i32::try_from(x).expect("x coordinate exceeds i32::MAX");
        let y = i32::try_from(y).expect("y coordinate exceeds i32::MAX");

        let idx = self.coord_to_index(x, y);
        &mut self.buffer[idx]
    }

    pub fn get_mut_at_point(&mut self, pt: &Point) -> &mut Pixel {
        let idx = self.coord_to_index(pt.x, pt.y);
        &mut self.buffer[idx]
    }

    pub fn set(&mut self, x: u32, y: u32, px: Pixel) {
        *self.get_mut(x, y) = px;
    }

    pub fn set_at_point(&mut self, pt: &Point, px: Pixel) {
        *self.get_mut_at_point(pt) = px;
    }

    // #[cfg(test)]
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
        let px = self.get(src.0, src.1);

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
        let from = self.get(src.0, src.1);

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
            while left > 0 && self.get(left - 1, y) == from {
                left -= 1;
                self.set(left, y, target);
            }

            // Traverse right till boundary
            while right < w - 1 && self.get(right + 1, y) == from {
                right += 1;
                self.set(right, y, target);
            }

            acc.accumulate(Row { left, right, y });

            for ny in [y.saturating_sub(1), y + 1] {
                if ny != y && ny < h {
                    let mut seg_len = 0;
                    for x in left..=right {
                        let px = self.get(x, ny);
                        if px == from {
                            seg_len += 1;
                        } else if seg_len > 1 {
                            queue.push_back((x - 1, ny));
                            seg_len = 0;
                        }
                    }
                    if seg_len > 1 {
                        queue.push_back((right, ny));
                    }
                }
            }
        }
        acc
    }
}
