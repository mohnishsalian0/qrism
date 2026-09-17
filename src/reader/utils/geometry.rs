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

// Square spiral iterator for alignment centre & bottom right anchor search
//------------------------------------------------------------------------------

pub struct SquareSpiral {
    start: Point,
    cursor: Point,
    run: i32,
    run_len: i32,
    dir: Direction,
    radius: i32,
}

impl SquareSpiral {
    pub fn new(start: &Point, radius: i32) -> Self {
        debug_assert!(radius >= 0);

        Self { start: *start, cursor: *start, run: 0, run_len: 1, dir: Direction::Right, radius }
    }
}

impl Iterator for SquareSpiral {
    type Item = Point;

    fn next(&mut self) -> Option<Self::Item> {
        let reach = (self.cursor.x - self.start.x).abs().max((self.cursor.y - self.start.y).abs());
        if reach > self.radius {
            return None;
        }

        let res = self.cursor;

        self.cursor.advance(self.dir);
        self.run += 1;

        // Cycle direction
        if self.run == self.run_len {
            self.run = 0;
            self.dir = self.dir.turn_left();
            if self.dir == Direction::Left || self.dir == Direction::Right {
                self.run_len += 1;
            }
        }

        Some(res)
    }
}

#[cfg(test)]
mod square_spiral_tests {
    use super::{Point, SquareSpiral};
    use std::collections::HashSet;

    fn walk(start: Point, radius: i32) -> Vec<Point> {
        SquareSpiral::new(&start, radius).collect()
    }

    fn reach(start: &Point, pt: &Point) -> i32 {
        (pt.x - start.x).abs().max((pt.y - start.y).abs())
    }

    #[test]
    fn test_radius_zero_yields_only_start() {
        let start = Point { x: 7, y: -3 };
        assert_eq!(walk(start, 0), vec![start]);
    }

    #[test]
    fn test_first_point_is_start() {
        let start = Point { x: -12, y: 40 };
        assert_eq!(SquareSpiral::new(&start, 5).next(), Some(start));
    }

    #[test]
    fn test_covers_square_exactly_once() {
        let start = Point { x: 100, y: 250 };

        for radius in 0..=6 {
            let pts = walk(start, radius);
            let uniq: HashSet<Point> = pts.iter().copied().collect();
            let exp: HashSet<Point> = (-radius..=radius)
                .flat_map(|dy| (-radius..=radius).map(move |dx| (dx, dy)))
                .map(|(dx, dy)| Point { x: start.x + dx, y: start.y + dy })
                .collect();
            let side = (2 * radius + 1) as usize;

            assert_eq!(pts.len(), side * side, "Radius = {radius}");
            assert_eq!(uniq.len(), pts.len(), "Radius = {radius}");
            assert_eq!(uniq, exp, "Radius = {radius}");
        }
    }

    #[test]
    fn test_visits_inner_rings_first() {
        let start = Point { x: -5, y: 8 };
        let rings: Vec<i32> = walk(start, 6).iter().map(|p| reach(&start, p)).collect();

        assert!(rings.windows(2).all(|w| w[0] <= w[1]));
    }

    #[test]
    fn test_steps_are_contiguous() {
        let pts = walk(Point { x: 0, y: 0 }, 5);

        assert!(pts.windows(2).all(|w| (w[1].x - w[0].x).abs() + (w[1].y - w[0].y).abs() == 1));
    }
}
