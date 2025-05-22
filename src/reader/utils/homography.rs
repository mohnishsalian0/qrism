use std::ops::{Index, IndexMut};

use crate::utils::{f64_to_i32, QRError, QRResult};

use super::geometry::Point;

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
    /// Compute homography matrix from 4 point pairs:
    /// source[i] -> destination[i]
    /// Returns homography matrix to project points from logical QR to image QR
    pub fn compute(src: [(f64, f64); 4], dst: [(f64, f64); 4]) -> QRResult<Self> {
        // Build matrix A (8x8) & B (8x1)
        // Rows: 2 per point, total 8 rows
        // Columns: 9 columns (h11..h33)
        let mut a = [[0.0_f64; 8]; 8];
        let mut b = [0.0_f64; 8];

        for i in 0..4 {
            let (x, y) = src[i];
            let (xp, yp) = dst[i];

            // row 2*i
            a[2 * i][0] = -x;
            a[2 * i][1] = -y;
            a[2 * i][2] = -1.0;
            a[2 * i][3] = 0.0;
            a[2 * i][4] = 0.0;
            a[2 * i][5] = 0.0;
            a[2 * i][6] = xp * x;
            a[2 * i][7] = xp * y;
            b[2 * i] = -xp;

            // row 2*i + 1
            a[2 * i + 1][0] = 0.0;
            a[2 * i + 1][1] = 0.0;
            a[2 * i + 1][2] = 0.0;
            a[2 * i + 1][3] = -x;
            a[2 * i + 1][4] = -y;
            a[2 * i + 1][5] = -1.0;
            a[2 * i + 1][6] = yp * x;
            a[2 * i + 1][7] = yp * y;
            b[2 * i + 1] = -yp;
        }

        let h = Self::solve_linear_system(a, b)?;

        Ok(Self(h))
    }

    /// Solve 8x8 linear system Ax = b by Gaussian elimination
    fn solve_linear_system(mut a: [[f64; 8]; 8], mut b: [f64; 8]) -> QRResult<[f64; 8]> {
        // Forward elimination
        for i in 0..8 {
            // Partial pivot
            let mut max_row = i;
            let mut max_val = a[i][i].abs();
            #[allow(clippy::needless_range_loop)]
            for r in (i + 1)..8 {
                if a[r][i].abs() > max_val {
                    max_val = a[r][i].abs();
                    max_row = r;
                }
            }
            if max_row != i {
                a.swap(i, max_row);
                b.swap(i, max_row);
            }

            // Check for zero pivot (singular matrix)
            if a[i][i].abs() < f64::EPSILON {
                return Err(QRError::SingularMatrix); // No unique solution
            }

            // Normalize row
            let pivot = a[i][i];
            for c in i..8 {
                a[i][c] /= pivot;
            }
            b[i] /= pivot;

            // Eliminate other rows
            for r in (i + 1)..8 {
                let factor = a[r][i];
                for c in i..8 {
                    a[r][c] -= factor * a[i][c];
                }
                b[r] -= factor * b[i];
            }
        }

        // Back substitution
        let mut x = [0.0; 8];
        for r in (0..8).rev() {
            let mut sum = 0.0;
            #[allow(clippy::needless_range_loop)]
            for c in (r + 1)..8 {
                sum += a[r][c] * x[c];
            }
            x[r] = (b[r] - sum) / a[r][r];
        }
        Ok(x)
    }

    /// Map a point (x,y) using homography H (3x3)
    pub fn map(&self, x: f64, y: f64) -> QRResult<Point> {
        let xp = self[0] * x + self[1] * y + self[2];
        let yp = self[3] * x + self[4] * y + self[5];
        let w = self[6] * x + self[7] * y + 1.0;

        if w.abs() <= f64::EPSILON {
            return Err(QRError::PointAtInfinity);
        }

        let xp = (xp / w).round();
        let yp = (yp / w).round();

        let x = f64_to_i32(&xp);
        let y = f64_to_i32(&yp);

        Ok(Point { x, y })
    }
}

#[cfg(test)]
mod homography_tests {
    use crate::reader::utils::geometry::Point;

    use super::Homography;

    #[test]
    fn test_homography() {
        let src = [(3.5, 3.5), (21.5, 3.5), (18.5, 18.5), (3.5, 21.5)];
        let dst = [(75.0, 75.0), (255.0, 75.0), (225.0, 225.0), (75.0, 255.0)];
        let h = Homography::compute(src, dst).unwrap();
        let pts = [(7.0, 7.0), (25.0, 0.0), (25.0, 25.0), (0.0, 25.0)];
        let expected = [(110, 110), (290, 40), (290, 290), (40, 290)];
        for (i, pt) in pts.iter().enumerate() {
            let proj_pt = h.map(pt.0, pt.1).unwrap();
            let exp_pt = Point { x: expected[i].0, y: expected[i].1 };
            assert_eq!(proj_pt, exp_pt);
        }
    }
}
