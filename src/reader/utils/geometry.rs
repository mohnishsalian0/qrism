use std::{cmp::Ordering, marker::PhantomData};

#[cfg(test)]
use image::{Rgb, RgbImage};

use crate::reader::binarize::BinaryImage;

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
    pub fn highlight(&self, img: &mut RgbImage) {
        let (w, h) = img.dimensions();
        for i in [-1, 0, 1] {
            for j in [-1, 0, 1] {
                let nx = ((self.x - i) as u32).min(w - 1);
                let ny = ((self.y - j) as u32).min(h - 1);
                img.put_pixel(nx, ny, Rgb([255, 0, 0]));
            }
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

    pub fn cross(&self, other: &Self) -> i32 {
        self.dx * other.dy - self.dy * other.dx
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

// Line represented as Ax + By + C = 0
//------------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct Line {
    a: i32,
    b: i32,
    c: i32,
}

impl Line {
    pub fn from_points(s: &Point, e: &Point) -> Self {
        let a = -(e.y - s.y);
        let b = e.x - s.x;
        let c = s.x * e.y - s.y * e.x;
        Self { a, b, c }
    }

    pub fn from_point_slope(p: &Point, m: &Slope) -> Self {
        let a = m.dy;
        let b = -m.dx;
        let c = -(a * p.x + b * p.y);
        Self { a, b, c }
    }

    pub fn intersection(&self, other: &Line) -> Option<Point> {
        let (a, b, c) = (self.a as i64, self.b as i64, self.c as i64);
        let (ao, bo, co) = (other.a as i64, other.b as i64, other.c as i64);

        let den = a * bo - b * ao;
        if den == 0 {
            return None;
        }

        let x = (b * co - c * bo) / den;
        let y = (c * ao - a * co) / den;

        let x = x.try_into().expect("Value out of i32 bounds");
        let y = y.try_into().expect("Value out of i32 bounds");
        Some(Point { x, y })
    }

    #[cfg(test)]
    pub fn highlight(&self, img: &mut RgbImage) {
        let dx = -self.b;
        let dy = self.a;
        let (w, h) = img.dimensions();
        let mut isecs = Vec::new();
        let corners = [(0, 0), (0, h), (w, h), (w, 0)];
        for (x1, y1) in corners.iter() {
            let p1 = Point { x: *x1 as i32, y: *y1 as i32 };
            for (x2, y2) in corners.iter() {
                let p2 = Point { x: *x2 as i32, y: *y2 as i32 };
                if p1 != p2 {
                    let line = Line::from_points(&p1, &p2);
                    if let Some(pt) = self.intersection(&line) {
                        isecs.push(pt);
                        if isecs.len() == 2 {
                            break;
                        }
                    };
                }
            }
        }
        if dx > dy {
            let line = BresenhamLine::<X>::new(&isecs[0], &isecs[1]);
            for pt in line {
                pt.highlight(img);
            }
        } else {
            let line = BresenhamLine::<Y>::new(&isecs[0], &isecs[1]);
            for pt in line {
                pt.highlight(img);
            }
        }
    }
}

#[cfg(test)]
mod line_tests {
    use super::*;

    #[test]
    fn test_line_intersection() {
        let l1 = Line::from_points(&Point { x: 0, y: 0 }, &Point { x: 4, y: 4 });
        let l2 = Line::from_points(&Point { x: 0, y: 4 }, &Point { x: 4, y: 0 });

        let inter = l1.intersection(&l2).unwrap();
        assert_eq!(inter.x, 2);
        assert_eq!(inter.y, 2);
    }

    #[test]
    fn test_parallel_lines() {
        let l1 = Line::from_points(&Point { x: 0, y: 0 }, &Point { x: 4, y: 4 });
        let l2 = Line::from_points(&Point { x: 0, y: 1 }, &Point { x: 4, y: 5 });

        assert!(l1.intersection(&l2).is_none());
    }
}
