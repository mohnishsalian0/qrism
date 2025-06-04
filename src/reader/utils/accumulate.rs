use crate::utils::f64_to_i32;

use super::geometry::Point;

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

// Centre locator for finder
// Uses the centroid formula:
// CX = Sum of X / Total points
// CY = Sum of Y / Total points
//------------------------------------------------------------------------------

pub struct AreaAndCentreLocator {
    sum_x: u32,
    sum_y: u32,
    pub area: u32,
}

impl AreaAndCentreLocator {
    pub fn new() -> Self {
        Self { sum_x: 0, sum_y: 0, area: 0 }
    }

    pub fn get_centre(&self) -> Point {
        let x = self.sum_x as f64 / (2 * self.area) as f64;
        let y = self.sum_y as f64 / self.area as f64;

        let x = x.round();
        let y = y.round();

        let x = f64_to_i32(&x).unwrap();
        let y = f64_to_i32(&y).unwrap();

        Point { x, y }
    }
}

impl Accumulator for AreaAndCentreLocator {
    fn accumulate(&mut self, row: Row) {
        let Row { left, right, y } = row;
        let width = right - left + 1;
        let mid = left + right; // Not divided by 2 to avoid FP arithmetic. This 2 is accounted for
                                // when calculating x and y in get_centre()

        self.sum_x += mid * width;
        self.sum_y += y * width;
        self.area += width;
    }
}
