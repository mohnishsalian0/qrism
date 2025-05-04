use std::ops::Index;

#[derive(Debug, Clone, Copy, Eq, PartialEq, Default)]
pub struct Point {
    pub x: i32,
    pub y: i32,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, Default)]
pub struct Slope {
    pub dx: i32,
    pub dy: i32,
}

// Homographic projection matrix to map reference qr onto image qr
//------------------------------------------------------------------------------

#[derive(Debug, PartialEq, Clone)]
pub struct Homography(pub [f64; 8]);

impl Index<usize> for Homography {
    type Output = f64;

    fn index(&self, index: usize) -> &Self::Output {
        &self.0[index]
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
}
