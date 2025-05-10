use super::geometry::{Point, Slope};

// Accumulator trait for flood fill
//------------------------------------------------------------------------------

pub trait Accumulator {
    fn accumulate(&mut self, row: Row);
}

impl<F> Accumulator for F
where
    F: FnMut(Row),
{
    fn accumulate(&mut self, row: Row) {
        self(row)
    }
}

// Region row
//------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct Row {
    pub left: u32,
    pub right: u32,
    pub y: u32,
}

// Area accumulator to estimate region area
//------------------------------------------------------------------------------

pub struct Area(pub u32);

impl Accumulator for Area {
    fn accumulate(&mut self, row: Row) {
        self.0 += row.right - row.left + 1;
    }
}

// First corner finder
// First corner is the farthest point w.r.t the reference point
// The points are judged based on Cartesian distance
//------------------------------------------------------------------------------

#[derive(Debug, Eq, PartialEq, Clone)]
pub struct FirstCornerFinder {
    pub corner: Point, // Best corner point based on perpendicular distance
    reference: Point,  // Reference point in quadrilateral to locate corner
    score: i32,        // Highest Cartesian distance. Belongs to stored corner
}

impl FirstCornerFinder {
    pub fn new(reference: Point) -> Self {
        FirstCornerFinder { reference, corner: Default::default(), score: -1 }
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
            if dist > self.score {
                self.corner = Point { x, y };
                self.score = dist;
            }
        }
    }
}

// All corner finder
// Baseline is constructed from ref pt to first corner
// The 2 corners adjacent to the first corner are the farthest pts from baseline
// The corners are judged based on perpendicular distance from baseline
//
// Perpendicular distance formula = (Ax + By + C) / sqrt (A² + B²)
// Ignoring the constants: Perpendicular distance score = Ax + By
// A is the numerator of the slope, B is negative of the denominator
//
// The last corner is the farthest pt from the normal to baseline
//------------------------------------------------------------------------------

#[derive(Debug, Eq, PartialEq, Clone)]
pub struct AllCornerFinder {
    pub corners: [Point; 4], // Best corner points based on perpendicular distance
    baseline: Slope,         // Slope of baseline between ref pt and first corner
    scores: [i32; 4],        // Highest dist scores. Belongs to stored corners
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
            scores: [par_scr, ort_scr, -par_scr, -ort_scr],
        }
    }
}

impl Accumulator for AllCornerFinder {
    fn accumulate(&mut self, row: Row) {
        let y = row.y as i32;
        let Slope { dx, dy } = self.baseline;
        let (ndx, ndy) = (dy, -dx); // Slope of line normal to baseline

        for x in [row.left as i32, row.right as i32] {
            let base_dist = -x * dy + y * dx; // Dist score of pt from baseline
            let norm_dist = -x * ndy + y * ndx; // Dist score of pt from normal
            let distances = [norm_dist, base_dist, -norm_dist, -base_dist];

            for (i, d) in distances.iter().enumerate() {
                if *d > self.scores[i] {
                    self.corners[i] = Point { x, y };
                    self.scores[i] = *d;
                }
            }
        }
    }
}

// Top left corner finder for alignment pattern
//------------------------------------------------------------------------------

pub struct TopLeftCornerFinder {
    pub corner: Point,
    m: Slope,   // Slope of diagonal line through opposite finders
    score: i32, // Lowest dist score. Belongs to stored corner
}

impl TopLeftCornerFinder {
    pub fn new(seed: &Point, m: &Slope) -> Self {
        let score = -m.dy * seed.x + m.dx * seed.y;
        Self { corner: *seed, m: *m, score }
    }
}

impl Accumulator for TopLeftCornerFinder {
    fn accumulate(&mut self, row: Row) {
        let left_scr = -self.m.dy * (row.left as i32) + self.m.dx * row.y as i32;
        let right_scr = -self.m.dy * (row.right as i32) + self.m.dx * row.y as i32;

        if left_scr < self.score {
            self.score = left_scr;
            self.corner.x = row.left as i32;
            self.corner.y = row.y as i32;
        }

        if right_scr < self.score {
            self.score = right_scr;
            self.corner.x = row.right as i32;
            self.corner.y = row.y as i32;
        }
    }
}
