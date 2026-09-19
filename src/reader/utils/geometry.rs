use std::{cmp::Ordering, marker::PhantomData};

#[cfg(test)]
use image::{Rgb, RgbImage};

use crate::reader::binarize::BinaryImage;

// Direction enum
//------------------------------------------------------------------------------

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum Direction {
    Right,
    Down,
    Left,
    Up,
}

impl Direction {
    pub fn turn_right(&self) -> Self {
        match self {
            Self::Right => Self::Down,
            Self::Down => Self::Left,
            Self::Left => Self::Up,
            Self::Up => Self::Right,
        }
    }

    pub fn turn_left(&self) -> Self {
        match self {
            Self::Right => Self::Up,
            Self::Down => Self::Right,
            Self::Left => Self::Down,
            Self::Up => Self::Left,
        }
    }
}

// Point
//------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, Eq, PartialEq, Default, Hash)]
pub struct Point {
    pub x: i32,
    pub y: i32,
}

impl Point {
    pub fn dist_sq(&self, other: &Point) -> u32 {
        let dx = other.x - self.x;
        let dy = other.y - self.y;
        (dx * dx + dy * dy) as _
    }

    #[cfg(test)]
    pub fn highlight(&self, img: &mut RgbImage, color: Rgb<u8>) {
        let (w, h) = img.dimensions();
        for i in [-1, 0, 1] {
            for j in [-1, 0, 1] {
                let nx = ((self.x - i) as u32).min(w - 1);
                let ny = ((self.y - j) as u32).min(h - 1);
                img.put_pixel(nx, ny, color);
            }
        }
    }

    pub fn advance(&mut self, d: Direction) {
        match d {
            Direction::Right => self.x += 1,
            Direction::Down => self.y += 1,
            Direction::Left => self.x -= 1,
            Direction::Up => self.y -= 1,
        }
    }
}

// Slope
//------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, Eq, PartialEq, Default)]
pub struct Slope {
    pub dx: i32,
    pub dy: i32,
}

impl Slope {
    pub fn new(start: &Point, end: &Point) -> Self {
        let dx = end.x - start.x;
        let dy = end.y - start.y;
        Self { dx, dy }
    }
}

// Axis trait to modify functions based on X/Y axis at compile time
//------------------------------------------------------------------------------

pub trait Axis {
    fn bound_check(img: &BinaryImage, pt: &Point) -> bool;
    fn shift(pt: &mut Point, dist: &(i32, i32)); // Shifts point along axis
    fn shift_cross(pt: &mut Point, dist: &(i32, i32)); // Steps point along perpendicular axis
    fn delta(m: &Slope) -> i32; // Returns delta from slope along axis
    fn delta_cross(m: &Slope) -> i32; // Returns delta from slope along perpendicular axis
    fn is_aligned(a: &Point, b: &Point) -> bool; // True if position along axis is the same
}

pub struct X;

impl Axis for X {
    fn bound_check(img: &BinaryImage, pt: &Point) -> bool {
        0 <= pt.x && pt.x < img.w as i32
    }

    fn shift(pt: &mut Point, dist: &(i32, i32)) {
        pt.x += dist.0;
    }

    fn shift_cross(pt: &mut Point, dist: &(i32, i32)) {
        pt.y += dist.1;
    }

    fn delta(m: &Slope) -> i32 {
        m.dx
    }

    fn delta_cross(m: &Slope) -> i32 {
        m.dy
    }

    fn is_aligned(a: &Point, b: &Point) -> bool {
        a.x == b.x
    }
}

pub struct Y;

impl Axis for Y {
    fn bound_check(img: &BinaryImage, pt: &Point) -> bool {
        0 <= pt.y && pt.y < img.h as i32
    }

    fn shift(pt: &mut Point, dist: &(i32, i32)) {
        pt.y += dist.1;
    }

    fn shift_cross(pt: &mut Point, dist: &(i32, i32)) {
        pt.x += dist.0;
    }

    fn delta(m: &Slope) -> i32 {
        m.dy
    }

    fn delta_cross(m: &Slope) -> i32 {
        m.dx
    }

    fn is_aligned(a: &Point, b: &Point) -> bool {
        a.y == b.y
    }
}

// Bresenham line scan algorithm
//------------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct BresenhamLine<A: Axis> {
    cur: Point, // Current position
    end: Point,
    m: Slope,
    inc: (i32, i32), // (xi, yi) unit increment
    err: i32,
    phantom: PhantomData<A>,
}

impl<A: Axis> BresenhamLine<A> {
    pub fn new(from: &Point, to: &Point) -> Self {
        let cur = *from;
        let end = *to;

        // Computing slope
        let dx = (to.x - from.x).abs();
        let dy = (to.y - from.y).abs();
        let m = Slope { dx: 2 * dx, dy: 2 * dy };

        // Computing increment
        let xi = match to.x.cmp(&from.x) {
            Ordering::Greater => 1,
            Ordering::Equal => 0,
            Ordering::Less => -1,
        };
        let yi = match to.y.cmp(&from.y) {
            Ordering::Greater => 1,
            Ordering::Equal => 0,
            Ordering::Less => -1,
        };
        let inc = (xi, yi);

        // Computing error
        let err = if dy < dx { 2 * dy - dx } else { 2 * dx - dy };

        let phantom = PhantomData;

        Self { cur, end, m, inc, err, phantom }
    }
}

impl<A: Axis> Iterator for BresenhamLine<A> {
    type Item = Point;

    fn next(&mut self) -> Option<Self::Item> {
        if A::is_aligned(&self.cur, &self.end) {
            return None;
        }

        let res = Some(self.cur);

        if self.err > 0 {
            A::shift_cross(&mut self.cur, &self.inc);
            self.err -= A::delta(&self.m);
        }
        A::shift(&mut self.cur, &self.inc);
        self.err += A::delta_cross(&self.m);

        res
    }
}

// Square spiral leg iterator for alignment centre & bottom right anchor search
//------------------------------------------------------------------------------

pub struct SquareSpiralLeg {
    len: i32,
    dx: i32,
    dy: i32,
    steps: i32,
    max_steps: i32,
}

impl SquareSpiralLeg {
    pub fn new(radius: i32) -> Self {
        debug_assert!(radius >= 0);

        Self { len: 1, dx: -1, dy: 0, steps: 0, max_steps: (2 * radius + 1).pow(2) }
    }
}

impl Iterator for SquareSpiralLeg {
    type Item = (i32, i32, i32);

    fn next(&mut self) -> Option<Self::Item> {
        let res = (self.len, self.dx, self.dy);

        self.steps += self.len;
        if self.steps > self.max_steps {
            return None;
        }

        // Cycle direction
        (self.dx, self.dy) = (-self.dy, self.dx);
        if self.dy == 0 {
            self.len += 1;
        }

        Some(res)
    }
}

#[cfg(test)]
mod square_spiral_tests {
    use std::collections::HashSet;

    use super::SquareSpiralLeg;

    fn walk(radius: i32) -> Vec<(i32, i32, i32)> {
        SquareSpiralLeg::new(radius).collect()
    }

    // Replays the legs the way `pinpoint_alignment_centre` does: start at the origin, step first,
    // then look at where the cursor landed. The origin itself is never yielded.
    fn trace(radius: i32) -> Vec<(i32, i32)> {
        let (mut cx, mut cy) = (0, 0);
        let mut pts = Vec::new();
        for (leg, dx, dy) in SquareSpiralLeg::new(radius) {
            for _ in 0..leg {
                cx += dx;
                cy += dy;
                pts.push((cx, cy));
            }
        }
        pts
    }

    #[test]
    fn test_radius_zero_yields_only_start() {
        assert_eq!(walk(0), vec![(1, -1, 0)]);
    }

    #[test]
    fn test_first_point_is_start() {
        assert_eq!(SquareSpiralLeg::new(5).next(), Some((1, -1, 0)));
    }

    #[test]
    fn test_radius_one_leg_sequence() {
        let legs = walk(1);
        assert_eq!(legs, vec![(1, -1, 0), (1, 0, -1), (2, 1, 0), (2, 0, 1), (3, -1, 0)]);
    }

    #[test]
    fn test_leg_lengths_grow_in_pairs() {
        let lens: Vec<i32> = walk(4).iter().map(|l| l.0).collect();
        assert_eq!(lens, vec![1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9]);
    }

    #[test]
    fn test_directions_cycle_left_up_right_down() {
        let dirs: Vec<(i32, i32)> = walk(4).iter().map(|l| (l.1, l.2)).collect();
        let cycle = [(-1, 0), (0, -1), (1, 0), (0, 1)];
        for (i, d) in dirs.iter().enumerate() {
            assert_eq!(*d, cycle[i % 4], "leg {i} turned the wrong way");
        }
    }

    #[test]
    fn test_total_steps_fill_bounding_square() {
        for radius in 0..=6 {
            let steps: i32 = walk(radius).iter().map(|l| l.0).sum();
            assert_eq!(steps, (2 * radius + 1).pow(2), "radius {radius} walked the wrong length");
        }
    }

    #[test]
    fn test_spiral_covers_neighbourhood_exactly_once() {
        for radius in 1..=5 {
            let pts = trace(radius);

            let unique: HashSet<(i32, i32)> = pts.iter().copied().collect();
            assert_eq!(unique.len(), pts.len(), "radius {radius} revisited a cell");
            assert!(!unique.contains(&(0, 0)), "radius {radius} stepped back onto the seed");

            // Every cell within the radius, bar the seed, must be visited. The spiral overshoots
            // its bounding square by exactly one cell -- the consumer bound checks it away.
            let inside = |&(x, y): &(i32, i32)| x.abs() <= radius && y.abs() <= radius;
            let covered = unique.iter().filter(|p| inside(p)).count() as i32;
            assert_eq!(covered, (2 * radius + 1).pow(2) - 1, "radius {radius} left a gap");
            assert_eq!(unique.iter().filter(|p| !inside(p)).count(), 1);
        }
    }

    #[test]
    fn test_every_leg_steps_one_cell_along_an_axis() {
        for (_, dx, dy) in walk(3) {
            assert_eq!(dx.abs() + dy.abs(), 1, "({dx}, {dy}) is not a unit axis step");
        }
    }
}
