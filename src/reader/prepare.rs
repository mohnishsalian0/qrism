use std::{cmp, collections::VecDeque, num::NonZeroUsize};

use image::{Rgb, RgbImage};
use lru::LruCache;

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
    Visited(u8), // Contains id of associated region
    Unvisited,
    Temporary,
    Reserved,
    White,
}

impl From<Rgb<u8>> for Pixel {
    fn from(p: Rgb<u8>) -> Self {
        for c in p.0 {
            if c != 255 {
                return match c {
                    0 => Self::Unvisited,
                    1 => Self::Temporary,
                    2 => Self::Reserved,
                    id => Self::Visited(id - 3),
                };
            }
        }
        Self::White
    }
}

impl Pixel {
    // Takes current Rgb color and updates redundant channel with Pixel value
    pub fn to_rgb(self, color: &Rgb<u8>) -> Rgb<u8> {
        let val = match self {
            Self::Visited(id) => id + 3,
            Self::Unvisited => return *color,
            Self::Temporary => 1,
            Self::Reserved => return Rgb([2, 2, 2]),
            Self::White => return Rgb([255, 255, 255]),
        };
        let mut res = *color;
        if res[0] != 255 {
            res[0] = val;
        }
        if res[1] != 255 {
            res[1] = val;
        }
        if res[2] != 255 {
            res[2] = val;
        }
        res
    }
}

// Region
//------------------------------------------------------------------------------

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct Region {
    pub src: (u32, u32),
    pub color: Rgb<u8>,
    pub area: u32,
}

// QR type for reader
//------------------------------------------------------------------------------

#[derive(Debug)]
pub struct PreparedImage {
    pub buffer: RgbImage,
    regions: LruCache<u8, Region>, // Areas of visited regions. Index is id
}

impl PreparedImage {
    /// Performs adaptive binarization on an RGB image using a sliding window
    /// and per-channel average filtering.
    pub fn prepare(mut img: RgbImage) -> Self {
        let (w, h) = img.dimensions();
        let win_sz = cmp::max(w / 8, 1);
        let den = 200 * win_sz;
        let mut u_avg = [0, 0, 0];
        let mut v_avg = [0, 0, 0];
        let mut row_avg = vec![[0, 0, 0]; w as usize];

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

            for x in 0..w {
                let px = img.get_pixel_mut(x, y);
                for (i, p) in px.0.iter_mut().enumerate() {
                    let thresh = row_avg[x as usize][i] * (100 - 5) / den;
                    if *p as u32 >= thresh {
                        *p = 255;
                    } else {
                        *p = 0;
                    }
                }
            }

            row_avg.fill([0, 0, 0]);
        }
        Self { buffer: img, regions: LruCache::new(NonZeroUsize::new(250).unwrap()) }
    }

    #[inline]
    pub fn width(&self) -> u32 {
        self.buffer.width()
    }

    #[inline]
    pub fn height(&self) -> u32 {
        self.buffer.height()
    }

    pub fn get(&self, x: u32, y: u32) -> Rgb<u8> {
        *self.buffer.get_pixel(x, y)
    }

    fn coord_to_index(&self, x: i32, y: i32) -> (u32, u32) {
        let w = self.width() as i32;
        let h = self.height() as i32;
        debug_assert!(-w <= x && x < w, "row shouldn't be greater than or equal to w");
        debug_assert!(-h <= y && y < h, "column shouldn't be greater than or equal to w");

        let x = if x < 0 { x + w } else { x };
        let y = if y < 0 { y + h } else { y };
        (x as u32, y as u32)
    }

    pub fn get_at_point(&self, pt: &Point) -> &Rgb<u8> {
        let (x, y) = self.coord_to_index(pt.x, pt.y);
        self.buffer.get_pixel(x, y)
    }

    pub fn get_mut(&mut self, x: u32, y: u32) -> &mut Rgb<u8> {
        self.buffer.get_pixel_mut(x, y)
    }

    pub fn get_mut_at_point(&mut self, pt: &Point) -> &mut Rgb<u8> {
        let (x, y) = self.coord_to_index(pt.x, pt.y);
        self.buffer.get_pixel_mut(x, y)
    }

    pub fn set(&mut self, x: u32, y: u32, px: Rgb<u8>) {
        *self.get_mut(x, y) = px;
    }

    pub fn set_at_point(&mut self, pt: &Point, px: Rgb<u8>) {
        *self.get_mut_at_point(pt) = px;
    }

    #[cfg(test)]
    pub fn save(&self, path: &Path) -> ImageResult<()> {
        self.buffer.save(path)
    }

    pub(crate) fn get_region(&mut self, src: (u32, u32)) -> Option<Region> {
        let color = self.get(src.0, src.1);
        let px: Pixel = color.into();

        match px {
            Pixel::Unvisited => {
                let reg_count = self.regions.len() as u8;

                let reg_id = if reg_count == self.regions.cap().get() as u8 {
                    let (id, reg) = self.regions.pop_lru().expect("Cache is full");
                    let Region { src, color, .. } = reg;

                    let _ = self.fill_and_accumulate(src, color, |_| ());

                    id
                } else {
                    reg_count
                };

                let area = Area(0);
                let to = Pixel::Visited(reg_id).to_rgb(&color);
                let acc = self.fill_and_accumulate(src, to, area);
                let new_reg = Region { src, color, area: acc.0 };

                self.regions.put(reg_id, new_reg);

                Some(new_reg)
            }
            Pixel::Visited(id) => {
                Some(*self.regions.get(&id).expect("No region found for visited pixel"))
            }
            _ => None,
        }
    }

    /// Fills region with provided color and accumulates info
    pub fn fill_and_accumulate<A: Accumulator>(
        &mut self,
        src: (u32, u32),
        target: Rgb<u8>,
        mut acc: A,
    ) -> A {
        let from = *self.buffer.get_pixel(src.0, src.1);

        debug_assert!(
            from != target && from != Rgb([255, 255, 255]),
            "Cannot fill white or same color"
        );

        // Flood fill algorithm
        let w = self.width();
        let h = self.height();
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

#[cfg(test)]
mod prepare_tests {

    use image::Rgb;
    use std::path::Path;

    use super::PreparedImage;

    #[test]
    fn test_flood_fill() {
        let path = Path::new("assets/test1.png");
        let img = image::open(path).unwrap().to_rgb8();
        let mut img = PreparedImage::prepare(img);
        let _ = img.fill_and_accumulate((101, 60), Rgb([127, 127, 127]), |_| ());
        let out_path = Path::new("assets/test_flood_fill.png");
        img.save(out_path).expect("Failed to save image");
    }
}
