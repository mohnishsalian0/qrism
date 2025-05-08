use std::{cmp, collections::VecDeque};

use image::RgbImage;

use crate::metadata::Color;

use super::utils::{
    accumulate::{Accumulator, Area, Row},
    geometry::Point,
};

// Pixel
//------------------------------------------------------------------------------

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum Pixel {
    Color([bool; 3]),
    Visited(usize), // Contains id of associated region
    Temporary,
    Finder,
    Alignment,
    Format,
    Version,
    Timing,
}

impl From<Pixel> for Option<Color> {
    fn from(p: Pixel) -> Self {
        match p {
            Pixel::Color(clr) => Some(Color::Rgb(clr)),
            _ => None,
        }
    }
}

impl From<Pixel> for Option<u32> {
    fn from(value: Pixel) -> Self {
        match value {
            Pixel::Color([r, g, b]) => Some(!(r | g | b) as u32),
            _ => None,
        }
    }
}

// Direction
//------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
enum Direction {
    Up,
    Down,
}

// Region
//------------------------------------------------------------------------------

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct Region {
    pub id: usize,
    pub area: usize,
}

// QR type for reader
//------------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct PreparedImage {
    grid: Vec<Pixel>,
    regions: Vec<usize>, // Areas of visited regions. Index is id
    w: usize,
    h: usize,
}

impl PreparedImage {
    /// Performs adaptive binarization on an RGB image using a sliding window
    /// and per-channel average filtering.
    pub fn from_image(img: &RgbImage) -> Self {
        let (w, h) = img.dimensions();
        let mut grid = Vec::with_capacity(w.checked_mul(h).expect("Image too large") as usize);
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
                let mut out = [false, false, false];
                let px = img.get_pixel(x, y);
                for (i, p) in out.iter_mut().enumerate() {
                    let thresh = row_avg[x as usize][i] * (100 - 5) / den;
                    *p = px[i] as u32 >= thresh;
                }
                grid.push(Pixel::Color(out));
            }

            row_avg.fill([0, 0, 0]);
        }
        Self { grid, regions: vec![], w: w as usize, h: h as usize }
    }

    pub fn grid(&self) -> &[Pixel] {
        &self.grid
    }

    pub fn width(&self) -> usize {
        self.w
    }

    pub fn height(&self) -> usize {
        self.h
    }

    pub fn get(&self, r: usize, c: usize) -> Pixel {
        self.grid[r * self.w + c]
    }

    fn coord_to_index(&self, r: i32, c: i32) -> usize {
        let w = self.w as i32;
        let h = self.h as i32;
        debug_assert!(-w <= r && r < w, "row shouldn't be greater than or equal to w");
        debug_assert!(-h <= c && c < h, "column shouldn't be greater than or equal to w");

        let r = if r < 0 { r + w } else { r };
        let c = if c < 0 { c + h } else { c };
        (r * w + c) as _
    }

    pub fn get_at_point(&self, p: &Point) -> Pixel {
        let idx = self.coord_to_index(p.y, p.x);
        self.grid[idx]
    }

    pub fn get_mut(&mut self, r: usize, c: usize) -> &mut Pixel {
        &mut self.grid[r * self.w + c]
    }

    pub fn get_mut_at_point(&mut self, pt: &Point) -> &mut Pixel {
        let idx = self.coord_to_index(pt.y, pt.x);
        &mut self.grid[idx]
    }

    pub fn set(&mut self, r: usize, c: usize, px: Pixel) {
        *self.get_mut(r, c) = px;
    }

    pub fn set_at_point(&mut self, pt: &Point, px: Pixel) {
        *self.get_mut_at_point(pt) = px;
    }

    pub(crate) fn get_region(&mut self, src: (usize, usize)) -> Option<Region> {
        let px = self.get(src.0, src.1);
        match px {
            Pixel::Color(clr) => {
                let id = self.regions.len();
                let mut area = Area(0);
                self.repaint_and_accumulate(src, px, Pixel::Visited(id), &mut area);
                self.regions.push(area.0);
                Some(Region { id, area: area.0 })
            }
            Pixel::Visited(id) => Some(Region { id, area: self.regions[id] }),
            _ => None,
        }
    }

    /// Repaints region and accumulates info
    pub fn repaint_and_accumulate<A: Accumulator>(
        &mut self,
        src: (usize, usize),
        from: Pixel,
        to: Pixel,
        acc: &mut A,
    ) {
        debug_assert!(
            from != to && from != Pixel::Color([true, true, true]),
            "Cannot repaint white or same color"
        );

        // Flood fill algorithm
        let w = self.w;
        let mut queue = VecDeque::new();
        let mut left = src.0;
        let mut right = src.0;
        let y = src.1;

        // Traverse left until boundary
        while left > 0 && self.get(left - 1, y) == from {
            left -= 1;
            self.set(left, y, to);
        }

        // Traverse right until boundary
        while right < w - 1 && self.get(right + 1, y) == from {
            right += 1;
            self.set(right, y, to);
        }

        queue.push_back(Row { left, right, y });

        while let Some(row) = queue.pop_front() {
            let Row { left, right, y } = row;
            acc.accumulate(row);

            for ny in [y.saturating_sub(1), y + 1] {
                if ny != y && y < self.h {
                    let mut rl = left;
                    let mut reg_found = false;
                    for x in left..=right {
                        let px = self.get(x, y);
                        if px == from {
                            self.set(x, y, to);
                            if !reg_found {
                                rl = x;
                                reg_found = true;
                            }
                        } else if reg_found {
                            queue.push_back(Row { left: rl, right: x - 1, y });
                            reg_found = false;
                        }
                    }
                }
            }
        }
    }
}
