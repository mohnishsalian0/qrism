// Mahalanobis
//------------------------------------------------------------------------------

use std::debug_assert_matches;

use crate::{metadata::Color, QRResult, Version};

pub(crate) struct Mahalanobis {
    centroid: [[f64; 3]; 8],
    inv: [[f64; 9]; 8],
    logdet: [f64; 8],
}

impl Mahalanobis {
    pub(crate) fn fit<F>(ver: Version, sampler: F) -> Self
    where
        F: Fn(i32, i32) -> QRResult<[u8; 3]>,
    {
        debug_assert_matches!(ver, Version::Normal(_));

        let aps = ver.alignment_pattern();
        let w = ver.width() as i32;
        let wsc = 100 + 8 * aps.len() * aps.len(); // White sample count
        let mut samples: [Vec<[u8; 3]>; 8] = std::array::from_fn(|_| Vec::with_capacity(wsc));

        // Sampling finders
        sample_finder(-4, 3, &sampler, &mut samples, Color::Red, Color::Cyan);
        sample_finder(3, 3, &sampler, &mut samples, Color::Green, Color::Magenta);
        sample_finder(3, -4, &sampler, &mut samples, Color::Blue, Color::Yellow);

        // Sampling alignments
        for &x in aps {
            for &y in aps {
                if (x == 6 && (y == 6 || y == w - 7)) || (x == w - 7 && y == 6) {
                    continue;
                }
                sample_alignment(x, y, &sampler, &mut samples);
            }
        }

        // Sampling timing patterns
        if matches!(ver, Version::Micro(_) | Version::Normal(1..=7)) {
            let (off, last) = match ver {
                Version::Micro(_) => (0, w - 1),
                Version::Normal(_) => (6, w - 9),
            };
            sample_timing(8, last, off, true, &sampler, &mut samples);
            sample_timing(8, last, off, false, &sampler, &mut samples);
        }

        // Median of each class
        let mut centroid: [[f64; 3]; 8] = [[f64::default(); 3]; 8];
        for i in 0..8 {
            for j in 0..3 {
                let mut channel_samples: Vec<u8> = samples[i].iter().map(|s| s[j]).collect();
                centroid[i][j] = median(&mut channel_samples);
            }
        }

        // Covariance of each class
        let mut cov = [[0.0; 9]; 8];
        for i in 0..8 {
            cov[i] = compute_covariance(&samples[i], &centroid[i]);
        }

        // Pooled samples
        let pool_samples: Vec<[u8; 3]> = samples.into_iter().flatten().collect();
        let mut pool_centroid: [f64; 3] = [f64::default(); 3];
        for i in 0..3 {
            let mut channel_pool_samples: Vec<u8> = pool_samples.iter().map(|ps| ps[i]).collect();
            pool_centroid[i] = median(&mut channel_pool_samples);
        }

        // Pooled covariance
        let pool_cov = compute_covariance(&pool_samples, &pool_centroid);

        // Blend class and pooled covariances
        for c in &mut cov {
            shrink(c, &pool_cov);
        }

        // Inverse of covariance
        let inv: [[f64; 9]; 8] = std::array::from_fn(|i| inverse(&cov[i]));

        // Logarithm determinant of covariance
        let logdet: [f64; 8] = std::array::from_fn(|i| determinant(&cov[i]).ln());

        Self { centroid, inv, logdet }
    }

    pub(crate) fn classify(&self, color: &[u8; 3]) -> Color {
        let (mut clr_idx, mut min_score) = (0, f64::INFINITY);
        for i in 0..8 {
            let dist_sq = self.dist_squared(color, i);
            let score = dist_sq + self.logdet[i];
            if score < min_score {
                clr_idx = i;
                min_score = score;
            }
        }
        Color::try_from(clr_idx as u8).unwrap()
    }

    fn dist_squared(&self, color: &[u8; 3], class: usize) -> f64 {
        // Normalize
        let clr: [f64; 3] = std::array::from_fn(|i| color[i] as f64 - self.centroid[class][i]);
        let covi = &self.inv[class];

        (clr[0] * covi[0] + clr[1] * covi[3] + clr[2] * covi[6]) * clr[0]
            + (clr[0] * covi[1] + clr[1] * covi[4] + clr[2] * covi[7]) * clr[1]
            + (clr[0] * covi[2] + clr[1] * covi[5] + clr[2] * covi[8]) * clr[2]
    }
}

// Samplers
//------------------------------------------------------------------------------

fn sample_finder<F>(
    x: i32,
    y: i32,
    sampler: &F,
    samples: &mut [Vec<[u8; 3]>; 8],
    ring_clr: Color,
    stone_clr: Color,
) where
    F: Fn(i32, i32) -> QRResult<[u8; 3]>,
{
    let ring_idx = ring_clr as usize;
    let stone_idx = stone_clr as usize;
    let (left, right) = if x > 0 { (-3, 4) } else { (-4, 3) };
    let (top, bottom) = if y > 0 { (-3, 4) } else { (-4, 3) };
    for i in left..=right {
        for j in top..=bottom {
            let Ok(sample) = sampler(x + i, y + j) else { continue };
            let class = match (i, j) {
                (4 | -4, _) | (_, 4 | -4) => 7,
                (3 | -3, _) | (_, 3 | -3) => ring_idx,
                (2 | -2, _) | (_, 2 | -2) => 7,
                _ => stone_idx,
            };
            samples[class].push(sample);
        }
    }
}

fn sample_alignment<F>(x: i32, y: i32, sampler: &F, samples: &mut [Vec<[u8; 3]>; 8])
where
    F: Fn(i32, i32) -> QRResult<[u8; 3]>,
{
    for i in -2..=2 {
        for j in -2..=2 {
            let Ok(sample) = sampler(x + i, y + j) else { continue };
            let class = match (i, j) {
                (-2 | 2, _) | (_, -2 | 2) | (0, 0) => 0,
                _ => 7,
            };
            samples[class].push(sample);
        }
    }
}

fn sample_timing<F>(
    start: i32,
    end: i32,
    fixed: i32,
    hor: bool,
    sampler: &F,
    samples: &mut [Vec<[u8; 3]>; 8],
) where
    F: Fn(i32, i32) -> QRResult<[u8; 3]>,
{
    if hor {
        for i in start..=end {
            let Ok(sample) = sampler(i, fixed) else { continue };
            let class = (i as usize & 1) * 7;
            samples[class].push(sample);
        }
    } else {
        for j in start..=end {
            let Ok(sample) = sampler(fixed, j) else { continue };
            let class = (j as usize & 1) * 7;
            samples[class].push(sample);
        }
    }
}

// Covariance matrix
//------------------------------------------------------------------------------

fn compute_covariance(data: &[[u8; 3]], centroid: &[f64; 3]) -> [f64; 9] {
    let len = data.len();

    // Centring data by subtracting median
    let centred: Vec<[f64; 3]> =
        data.iter().map(|row| std::array::from_fn(|i| (row[i] as f64) - centroid[i])).collect();

    // Covariance matrix
    let mut cov: [f64; 9] = [0.0; 9];
    for px in centred {
        for (i, cv) in cov.iter_mut().enumerate() {
            let (r, c) = (i / 3, i % 3);
            *cv += px[r] * px[c];
        }
    }

    for cv in &mut cov {
        *cv /= len as f64;
    }

    // Add ridge to diagonal so inversion is possible
    ridged(&mut cov);

    cov
}

fn median(data: &mut [u8]) -> f64 {
    let len = data.len();
    let mid = len / 2;

    if len & 1 == 1 {
        let (_, &mut median, _) = data.select_nth_unstable(mid);
        median as f64
    } else {
        let (_, &mut mid_left, _) = data.select_nth_unstable(mid - 1);
        let (_, &mut mid_right, _) = data.select_nth_unstable(mid);
        (mid_left as f64 + mid_right as f64) / 2.0
    }
}

/// Adds a diagonal ridge proportional to the mean variance, so inversion stays well-posed.
fn ridged(data: &mut [f64; 9]) {
    let eps = RIDGE_FACTOR * (data[0] + data[4] + data[8]).max(0.0) / 3.0;
    data[0] += eps.max(f64::MIN_POSITIVE);
    data[4] += eps.max(f64::MIN_POSITIVE);
    data[8] += eps.max(f64::MIN_POSITIVE);
}

fn inverse(m: &[f64; 9]) -> [f64; 9] {
    let det = determinant(m);

    let mut inv = [0.0; 9];
    inv[0] = (m[4] * m[8] - m[5] * m[7]) / det;
    inv[3] = (m[5] * m[6] - m[3] * m[8]) / det;
    inv[6] = (m[3] * m[7] - m[4] * m[6]) / det;
    inv[1] = (m[2] * m[7] - m[1] * m[8]) / det;
    inv[4] = (m[0] * m[8] - m[2] * m[6]) / det;
    inv[7] = (m[1] * m[6] - m[0] * m[7]) / det;
    inv[2] = (m[1] * m[5] - m[2] * m[4]) / det;
    inv[5] = (m[2] * m[3] - m[0] * m[5]) / det;
    inv[8] = (m[0] * m[4] - m[1] * m[3]) / det;

    inv
}

fn determinant(m: &[f64; 9]) -> f64 {
    m[0] * (m[4] * m[8] - m[5] * m[7]) - m[1] * (m[3] * m[8] - m[5] * m[6])
        + m[2] * (m[3] * m[7] - m[4] * m[6])
}

fn shrink(class: &mut [f64; 9], pooled: &[f64; 9]) {
    for i in 0..9 {
        class[i] = class[i] * (1.0 - SHRINK) + pooled[i] * SHRINK;
    }
}

// Global constants
//------------------------------------------------------------------------------

const RIDGE_FACTOR: f64 = 1e-3;

const SHRINK: f64 = 0.5;
