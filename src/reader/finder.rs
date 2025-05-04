use crate::metadata::Color;

use super::{
    deqr_temp::{Accumulator, DeQR, Pixel, Region, Row},
    utils::geometry::{Homography, Point, Slope},
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
    homography: Homography,
    corners: [Point; 4],
    center: Point,
}

// First corner finder
// First corner is the farthest point w.r.t the reference point
// The points are judged based on Cartesian distance
//------------------------------------------------------------------------------

#[derive(Debug, Eq, PartialEq, Clone)]
struct FirstCornerFinder {
    reference: Point, // Reference point in quadrilateral to locate corner
    corner: Point,    // Best corner point based on perpendicular distance
    distance: i32,    // Cartesian distance of stored corner
}

impl FirstCornerFinder {
    pub fn new(reference: Point) -> Self {
        FirstCornerFinder { reference, corner: Default::default(), distance: -1 }
    }
}

impl Accumulator for FirstCornerFinder {
    fn accumulate(&mut self, row: Row) {
        let y = row.y as i32;
        let Point { x: rx, y: ry } = self.reference;
        for x in [row.left as i32, row.right as i32] {
            let dx = rx - x;
            let dy = ry - y;
            let dist = dx * dx + dy * dy;
            if dist > self.distance {
                self.corner = Point { x, y };
                self.distance = dist;
            }
        }
    }
}

// All corner finder
// Baseline is constructed from ref pt to first corner
// The 2 corners adjacent to the first corner are the farthest pts from baseline
// The corners are judged based on perpendicular distance from baseline
//
// Perpendicular distance formula = (Ax + By + C) / sqrt (A^2 + B^2)
// Ignoring the constants: Perpendicular distance = Ax + By
//
// The last corner is the farthest pt from the normal to baseline
//------------------------------------------------------------------------------

#[derive(Debug, Eq, PartialEq, Clone)]
struct AllCornerFinder {
    baseline: Slope,     // Slope of baseline between ref pt and first corner
    corners: [Point; 4], // Best corner points based on perpendicular distance
    distances: [i32; 4], // Cartesian distances of stored corners
}

impl AllCornerFinder {
    pub fn new(reference: Point, corner: Point) -> Self {
        let Point { x: rx, y: ry } = reference;
        let baseline = Slope { dx: corner.x - rx, dy: corner.y - ry };

        // Parallel & orthogonal scores
        let par_scr = rx * baseline.dx + ry * baseline.dy;
        let ort_scr = -rx * baseline.dy + ry * baseline.dx;

        AllCornerFinder {
            baseline,
            corners: [reference; 4],
            distances: [par_scr, ort_scr, -par_scr, -ort_scr],
        }
    }
}

impl Accumulator for AllCornerFinder {
    fn accumulate(&mut self, row: Row) {
        let y = row.y as i32;
        let Slope { dx, dy } = self.baseline;
        let (ndx, ndy) = (dy, -dx); // Slope of line normal to baseline

        for x in [row.left as i32, row.right as i32] {
            let base_dist = -x * dy + y * dx; // Dist of pt from baseline
            let norm_dist = -x * ndy + y * ndx; // Dist of pt from normal
            let distances = [norm_dist, base_dist, -norm_dist, -base_dist];

            for (i, d) in distances.iter().enumerate() {
                if *d > self.distances[i] {
                    self.corners[i] = Point { x, y };
                    self.distances[i] = *d;
                }
            }
        }
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

pub fn locate_finders(deqr: &mut DeQR) -> Vec<Finder> {
    let mut finders = Vec::new();
    let w = deqr.width();
    let h = deqr.height();
    let mut scanner = LineScanner::new();

    for y in 0..h {
        for x in 0..w {
            let datum = match scanner.advance(deqr.get(x, y).into()) {
                Some(d) => d,
                None => continue,
            };

            if !is_finder(deqr, &datum) {
                continue;
            }

            if let Some(f) = construct_finder(deqr, &datum) {
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
// Left and right points of row are connected
fn is_finder(deqr: &mut DeQR, datum: &DatumLine) -> bool {
    let (l, r, s, y) = (datum.left, datum.right, datum.stone, datum.y);
    let ring = deqr.get_region((r, y));
    let stone = deqr.get_region((s, y));

    if deqr.get(l, y) != deqr.get(r, y) {
        return false;
    }

    match (ring, stone) {
        (
            Region::Visited { id: r_id, area: r_area },
            Region::Visited { id: s_id, area: s_area },
        ) => {
            let ratio = s_area * 100 / r_area;
            r_id != s_id && 20 < ratio && ratio < 50
        }
        _ => false,
    }
}

fn construct_finder(deqr: &mut DeQR, datum: &DatumLine) -> Option<Finder> {
    let (left, right, y) = (datum.left, datum.right, datum.y);
    let px = deqr.get(right, y);
    let refr_pt = Point { x: right as i32, y: y as i32 };

    // Locating first corner
    let mut fcf = FirstCornerFinder::new(refr_pt);
    deqr.repaint_and_accumulate((right, y), px, Pixel::Temporary, &mut fcf);

    // Locating rest of the corners
    let mut acf = AllCornerFinder::new(refr_pt, fcf.corner);
    deqr.repaint_and_accumulate((right, y), Pixel::Temporary, Pixel::Finder, &mut acf);

    // Setting up homographic projection
    let homography = Homography::create(&acf.corners, 7.0, 7.0)?;
    let corners = acf.corners;
    let center = homography.map(3.5, 3.5);

    Some(Finder { homography, corners, center })
}
