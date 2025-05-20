use std::{
    marker::PhantomData,
    ops::{Index, IndexMut},
};

use nalgebra::{DMatrix, Matrix3, Vector3};

// Point
//------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, Eq, PartialEq, Default)]
pub struct Point {
    pub x: i32,
    pub y: i32,
}

impl Point {
    pub fn squared_distance(&self, other: &Point) -> u32 {
        let dx = other.x - self.x;
        let dy = other.y - self.y;
        (dx * dx + dy * dy) as _
    }
}

#[cfg(test)]
mod point_highlight {
    use image::{Rgb, RgbImage};

    use crate::reader::utils::Highlight;

    use super::Point;

    impl Highlight for Point {
        fn highlight(&self, img: &mut RgbImage) {
            for i in [-1, 0, 1] {
                for j in [-1, 0, 1] {
                    let nx = (self.x - i).max(0) as u32;
                    let ny = (self.y - j).max(0) as u32;
                    img.put_pixel(nx, ny, Rgb([255, 0, 0]));
                }
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

// Homographic projection matrix to map logical qr onto image qr
//------------------------------------------------------------------------------

#[derive(Debug, PartialEq, Clone)]
pub struct Homography(pub Matrix3<f64>);

impl Index<usize> for Homography {
    type Output = f64;

    #[inline]
    fn index(&self, index: usize) -> &Self::Output {
        &self.0[index]
    }
}

impl IndexMut<usize> for Homography {
    #[inline]
    fn index_mut(&mut self, index: usize) -> &mut Self::Output {
        &mut self.0[index]
    }
}

impl Homography {
    pub fn compute(src: [(f64, f64); 4], dst: [(f64, f64); 4]) -> Option<Self> {
        let mut a_data = Vec::with_capacity(8 * 9);

        for i in 0..4 {
            let (x, y) = src[i];
            let (xp, yp) = dst[i];

            #[rustfmt::skip]
            a_data.extend_from_slice(&[
                -x, -y, -1.0, 0.0, 0.0, 0.0, x * xp, y * xp, xp,
                0.0, 0.0, 0.0, -x, -y, -1.0, x * yp, y * yp, yp,
            ]);
        }

        let a_matrix = DMatrix::from_row_slice(8, 9, &a_data);

        // Solve using Singular Value Decomposition to get the last column of V
        // corresponding to smallest singular value
        let svd = a_matrix.svd(true, true);
        let v = svd.v_t?;
        let h = v.row(8);

        Some(Self(Matrix3::from_iterator(h.iter().cloned())))
    }

    pub fn map(&self, x: f64, y: f64) -> Point {
        let p = Vector3::new(x, y, 1.0);
        let hp = self.0 * p;

        debug_assert!(hp.z.abs() <= 1e-10, "Homography denominator is too small");

        let resx = (hp.x / hp.z).round();
        let resy = (hp.y / hp.z).round();

        assert!(resx <= i32::MAX as f64);
        assert!(resx >= i32::MIN as f64);
        assert!(resy <= i32::MAX as f64);
        assert!(resy >= i32::MIN as f64);

        Point { x: resx as i32, y: resy as i32 }
    }
}

// Bresenham line scan algorithm
//------------------------------------------------------------------------------

pub trait Axis {}

pub struct X;
impl Axis for X {}

pub struct Y;
impl Axis for Y {}

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
        let xi = if to.x > from.x { 1 } else { -1 };
        let yi = if to.y > from.y { 1 } else { -1 };
        let inc = (xi, yi);

        // Computing error
        let err = if dy < dx { 2 * dy - dx } else { 2 * dx - dy };

        let phantom = PhantomData;

        Self { cur, end, m, inc, err, phantom }
    }
}

impl Iterator for BresenhamLine<X> {
    type Item = Point;

    fn next(&mut self) -> Option<Self::Item> {
        if self.cur.x == self.end.x {
            return None;
        }

        let res = Some(self.cur);

        if self.err > 0 {
            self.cur.y += self.inc.1;
            self.err -= self.m.dx;
        }
        self.err += self.m.dy;
        self.cur.x += self.inc.0;

        res
    }
}

impl Iterator for BresenhamLine<Y> {
    type Item = Point;

    fn next(&mut self) -> Option<Self::Item> {
        if self.cur.y == self.end.y {
            return None;
        }

        let res = Some(self.cur);

        if self.err > 0 {
            self.cur.x += self.inc.0;
            self.err -= self.m.dy;
        }
        self.err += self.m.dx;
        self.cur.y += self.inc.1;

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
        let den = self.a * other.b - self.b * other.a;
        if den == 0 {
            return None;
        }
        let x = (self.b * other.c - self.c * other.b) / den;
        let y = (self.c * other.a - self.a * other.c) / den;

        Some(Point { x, y })
    }
}

#[cfg(test)]
mod line_highlight {
    use image::RgbImage;

    use crate::reader::utils::Highlight;

    use super::{BresenhamLine, Line, Point, X, Y};

    impl Highlight for Line {
        fn highlight(&self, img: &mut RgbImage) {
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
