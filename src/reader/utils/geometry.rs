use std::{
    marker::PhantomData,
    ops::{Index, IndexMut},
};

// Point
//------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, Eq, PartialEq, Default)]
pub struct Point {
    pub x: i32,
    pub y: i32,
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
                    let nx = self.x.saturating_add(i) as u32;
                    let ny = self.y.saturating_add(j) as u32;
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

// Homographic projection matrix to map logical qr onto image qr
//------------------------------------------------------------------------------

#[derive(Debug, PartialEq, Clone)]
pub struct Homography(pub [f64; 8]);

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
    pub fn create(rect: &[Point; 4], w: f64, h: f64) -> Option<Self> {
        let mut c = [0.0; 8];
        let x0 = rect[0].x as f64;
        let y0 = rect[0].y as f64;
        let x1 = rect[1].x as f64;
        let y1 = rect[1].y as f64;
        let x2 = rect[2].x as f64;
        let y2 = rect[2].y as f64;
        let x3 = rect[3].x as f64;
        let y3 = rect[3].y as f64;
        let wden = w * (x2 * y3 - x3 * y2 + (x3 - x2) * y1 + x1 * (y2 - y3));
        let hden = h * (x2 * y3 + x1 * (y2 - y3) - x3 * y2 + (x3 - x2) * y1);

        if wden < f64::EPSILON || hden < f64::EPSILON {
            return None;
        }

        c[0] = (x1 * (x2 * y3 - x3 * y2)
            + x0 * (-x2 * y3 + x3 * y2 + (x2 - x3) * y1)
            + x1 * (x3 - x2) * y0)
            / wden;
        c[1] = -(x0 * (x2 * y3 + x1 * (y2 - y3) - x2 * y1) - x1 * x3 * y2
            + x2 * x3 * y1
            + (x1 * x3 - x2 * x3) * y0)
            / hden;
        c[2] = x0;
        c[3] = (y0 * (x1 * (y3 - y2) - x2 * y3 + x3 * y2)
            + y1 * (x2 * y3 - x3 * y2)
            + x0 * y1 * (y2 - y3))
            / wden;
        c[4] = (x0 * (y1 * y3 - y2 * y3) + x1 * y2 * y3 - x2 * y1 * y3
            + y0 * (x3 * y2 - x1 * y2 + (x2 - x3) * y1))
            / hden;
        c[5] = y0;
        c[6] = (x1 * (y3 - y2) + x0 * (y2 - y3) + (x2 - x3) * y1 + (x3 - x2) * y0) / wden;
        c[7] = (-x2 * y3 + x1 * y3 + x3 * y2 + x0 * (y1 - y2) - x3 * y1 + (x2 - x1) * y0) / hden;

        Some(Homography(c))
    }

    pub fn map(&self, x: f64, y: f64) -> Point {
        let den = self[6] * x + self[7] * y + 1.0f64;
        let resx = (self[0] * x + self[1] * y + self[2]) / den;
        let resy = (self[3] * x + self[4] * y + self[5]) / den;

        let resx = resx.round();
        let resy = resy.round();

        assert!(resx <= i32::MAX as f64);
        assert!(resx >= i32::MIN as f64);
        assert!(resy <= i32::MAX as f64);
        assert!(resy >= i32::MIN as f64);

        Point { x: resx as i32, y: resy as i32 }
    }

    pub fn unmap(&self, p: &Point) -> (f64, f64) {
        let (x, y) = (p.x as f64, p.y as f64);
        let den = (-self[0] * self[7] + self[1] * self[6]) * y
            + (self[3] * self[7] - self[4] * self[6]) * x
            + self[0] * self[4]
            - self[1] * self[3];
        let resx = -(self[1] * (y - self[5]) - self[2] * self[7] * y
            + (self[5] * self[7] - self[4]) * x
            + self[2] * self[4])
            / den;
        let resy = (self[0] * (y - self[5]) - self[2] * self[6] * y
            + (self[5] * self[6] - self[3]) * x
            + self[2] * self[3])
            / den;

        (resx, resy)
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
    s: Point, // Start point
    e: Point, // End point
    a: i32,   // Ax + By + C = 0
    b: i32,
    c: i32,
}

impl Line {
    pub fn new(s: &Point, e: &Point) -> Self {
        let a = -(e.y - s.y);
        let b = e.x - s.x;
        let c = s.x * e.y - s.y * e.x;
        Self { s: *s, e: *e, a, b, c }
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

    use super::{BresenhamLine, Line, X, Y};

    impl Highlight for Line {
        fn highlight(&self, img: &mut RgbImage) {
            let dx = (self.e.x - self.s.x).abs();
            let dy = (self.e.y - self.s.y).abs();
            if dx > dy {
                let line = BresenhamLine::<X>::new(&self.s, &self.e);
                for pt in line {
                    pt.highlight(img);
                }
            } else {
                let line = BresenhamLine::<Y>::new(&self.s, &self.e);
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
        let l1 = Line::new(&Point { x: 0, y: 0 }, &Point { x: 4, y: 4 });
        let l2 = Line::new(&Point { x: 0, y: 4 }, &Point { x: 4, y: 0 });

        let inter = l1.intersection(&l2).unwrap();
        assert_eq!(inter.x, 2);
        assert_eq!(inter.y, 2);
    }

    #[test]
    fn test_parallel_lines() {
        let l1 = Line::new(&Point { x: 0, y: 0 }, &Point { x: 4, y: 4 });
        let l2 = Line::new(&Point { x: 0, y: 1 }, &Point { x: 4, y: 5 });

        assert!(l1.intersection(&l2).is_none());
    }
}
