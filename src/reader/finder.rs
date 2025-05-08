use crate::metadata::Color;

use super::{
    prepare::{Pixel, PreparedImage, Region},
    utils::{
        accumulate::{AllCornerFinder, FirstCornerFinder},
        geometry::{Homography, Point, Slope},
    },
};

// Finder line
//------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
struct DatumLine {
    left: usize,
    stone: usize,
    right: usize,
    y: usize,
}

// Finder type
//------------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct Finder {
    pub h: Homography,
    pub corners: [Point; 4],
    pub center: Point,
}

impl Finder {
    #[inline]
    pub fn map(&self, x: f64, y: f64) -> Point {
        self.h.map(x, y)
    }

    #[inline]
    pub fn unmap(&self, p: &Point) -> (f64, f64) {
        self.h.unmap(p)
    }

    pub fn rotate(&mut self, pt: &Point, m: &Slope) {
        let (top_left, _) = self
            .corners
            .iter()
            .enumerate()
            .min_by_key(|(_, c)| (c.y - pt.y) * m.dx - (c.x - pt.x) * m.dy)
            .expect("Corners cannot be empty");
        self.corners.rotate_left(top_left);
        self.h =
            Homography::create(&self.corners, 7.0, 7.0).expect("rotating homography cant fail");
    }
}

// Line scanner to detect finder line
//------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
struct LineScanner {
    buffer: [usize; 6],  // Run length of each transition
    prev: Option<Color>, // Last observed color
    transitions: usize,  // Count of color changes
    pos: usize,          // Current position
    y: usize,
}

impl LineScanner {
    pub fn new() -> Self {
        Self { buffer: [0; 6], prev: None, transitions: 0, pos: 0, y: 0 }
    }

    pub fn reset(&mut self, y: usize) {
        self.prev = None;
        self.transitions = 0;
        self.pos = 0;
        self.y = y;
    }

    pub fn advance(&mut self, color: Option<Color>) -> Option<DatumLine> {
        self.pos += 1;

        if self.prev == color {
            self.buffer[5] += 1;
            return None;
        }

        self.buffer.rotate_left(1);
        self.buffer[5] = 1;
        self.prev = color;
        self.transitions += 1;

        if self.is_finder_line() {
            Some(DatumLine {
                left: self.pos - self.buffer[..5].iter().sum::<usize>(),
                stone: self.pos - self.buffer[2..5].iter().sum::<usize>(),
                right: self.pos - self.buffer[4],
                y: self.y,
            })
        } else {
            None
        }
    }

    fn is_finder_line(&self) -> bool {
        if !(self.prev == Some(Color::Light) && self.transitions >= 5) {
            return false;
        }

        let avg = self.buffer.iter().sum::<usize>() / 7;
        let tol = avg / 2;

        let ratio: [usize; 5] = [1, 1, 3, 1, 1];
        for (i, r) in ratio.iter().enumerate() {
            if self.buffer[i] < r * avg - tol || self.buffer[i] > r * avg + tol {
                return false;
            }
        }

        true
    }
}

// Locate finders
//------------------------------------------------------------------------------

pub fn locate_finders(img: &mut PreparedImage) -> Vec<Finder> {
    let mut finders = Vec::new();
    let w = img.width();
    let h = img.height();
    let mut scanner = LineScanner::new();

    for y in 0..h {
        for x in 0..w {
            let datum = match scanner.advance(img.get(x, y).into()) {
                Some(d) => d,
                None => continue,
            };

            if !is_finder(img, &datum) {
                continue;
            }

            if let Some(f) = construct_finder(img, &datum) {
                finders.push(f);
            }
        }
        scanner.reset(y + 1);
    }

    finders
}

// Sweeps stone and ring regions from datum line and validates finder if:
// Stone area is roughly 37.5% of ring area
// Stone and ring areas arent connected
// Left and right points of row lying inside the ring are connected
fn is_finder(img: &mut PreparedImage, datum: &DatumLine) -> bool {
    let (l, r, s, y) = (datum.left, datum.right, datum.stone, datum.y);
    let ring = img.get_region((r, y));
    let stone = img.get_region((s, y));

    if img.get(l, y) != img.get(r, y) {
        return false;
    }

    if let (Some(Region { id: r_id, area: r_area }), Some(Region { id: s_id, area: s_area })) =
        (ring, stone)
    {
        let ratio = s_area * 100 / r_area;
        r_id != s_id && (20 < ratio && ratio < 50)
    } else {
        false
    }
}

fn construct_finder(img: &mut PreparedImage, datum: &DatumLine) -> Option<Finder> {
    let (left, right, y) = (datum.left, datum.right, datum.y);
    let px = img.get(right, y);
    let refr_pt = Point { x: right as i32, y: y as i32 };

    // Locating first corner
    let mut fcf = FirstCornerFinder::new(refr_pt);
    img.repaint_and_accumulate((right, y), px, Pixel::Temporary, &mut fcf);

    // Locating rest of the corners
    let mut acf = AllCornerFinder::new(refr_pt, fcf.corner);
    img.repaint_and_accumulate((right, y), Pixel::Temporary, Pixel::Finder, &mut acf);

    // Setting up homographic projection
    let h = Homography::create(&acf.corners, 7.0, 7.0)?;
    let corners = acf.corners;
    let center = h.map(3.5, 3.5);

    Some(Finder { h, corners, center })
}

// Groups finders in 3, which form potential symbols
//------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Orientation {
    Horizontal,
    Vertical,
    None,
}

impl Orientation {
    pub fn is_none(&self) -> bool {
        matches!(self, Orientation::None)
    }
}

pub fn group_finders(finders: &[Finder]) -> Vec<[Finder; 3]> {
    let mut groups = Vec::new();
    let len = finders.len();
    let mut is_grouped = vec![false; len];

    for i1 in 0..len {
        if is_grouped[i1] {
            continue;
        }

        let f1 = &finders[i1];
        // Indices of horizontal and vertical neighbor
        let mut ih: Option<usize> = None;
        let mut iv: Option<usize> = None;
        // Equidistance of the 2 finders from datum finder. Lower the better
        let mut best_score = 2.5;

        for i2 in 0..len {
            if i2 == i1 || is_grouped[i2] {
                continue;
            }

            let f2 = &finders[i2];
            let (o2, d2) = get_relative_position(f1, f2);
            if o2.is_none() {
                continue;
            }

            for i3 in 0..len {
                if i3 == i2 || i3 == i1 || is_grouped[i3] {
                    continue;
                }

                let f3 = &finders[i3];
                let (o3, d3) = get_relative_position(f1, f3);

                match (o2, o3) {
                    (Orientation::Horizontal, Orientation::Vertical) => {
                        let score = (1.0f64 - d2 / d3).abs();
                        if score < best_score {
                            (ih, iv) = (Some(i2), Some(i3));
                            best_score = score;
                        }
                    }
                    (Orientation::Vertical, Orientation::Horizontal) => {
                        let score = (1.0f64 - d2 / d3).abs();
                        if score < best_score {
                            (ih, iv) = (Some(i3), Some(i2));
                            best_score = score;
                        }
                    }
                    _ => (),
                }
            }
        }

        if let (Some(ih), Some(iv)) = (ih, iv) {
            groups.push([finders[iv].clone(), f1.clone(), finders[ih].clone()]);
            is_grouped[i1] = true;
            is_grouped[ih] = true;
            is_grouped[iv] = true;
        }
    }

    groups
}

// Returns orientation of 2 finders and distance between their centers
fn get_relative_position(f1: &Finder, f2: &Finder) -> (Orientation, f64) {
    let (mut x, mut y) = f1.h.unmap(&f2.center);
    x = (x - 3.5f64).abs();
    y = (y - 3.5f64).abs();

    if y < 0.2f64 * x {
        (Orientation::Horizontal, x)
    } else if x < 0.2f64 * y {
        (Orientation::Vertical, y)
    } else {
        (Orientation::None, 0.0)
    }
}
