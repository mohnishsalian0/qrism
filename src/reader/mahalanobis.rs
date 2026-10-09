// Mahalanobis
//------------------------------------------------------------------------------

use std::debug_assert_matches;

use crate::{metadata::Color, QRError, QRResult, Version};

#[derive(Debug, Clone)]
pub(crate) struct Mahalanobis {
    centroid: [[f64; 3]; 8],
    inv: [[f64; 9]; 8],
    logdet: [f64; 8],
}

impl Mahalanobis {
    pub(crate) fn fit<F>(ver: Version, sampler: F) -> QRResult<Self>
    where
        F: Fn(i32, i32) -> QRResult<[u8; 3]>,
    {
        debug_assert_matches!(ver, Version::Normal(_));

        let aps = ver.alignment_pattern();
        let w = ver.width() as i32;
        let mut samples: [Vec<[u8; 3]>; 8] = std::array::from_fn(|_| Vec::new());

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
        let (off, last) = match ver {
            Version::Micro(_) => (0, w - 1),
            Version::Normal(_) => (6, w - 9),
        };
        sample_timing(8, last, off, true, &sampler, &mut samples);
        sample_timing(8, last, off, false, &sampler, &mut samples);

        // Return error if black or white samples are empty
        let black_idx = Color::Black as usize;
        let white_idx = Color::White as usize;
        if samples[black_idx].is_empty() || samples[white_idx].is_empty() {
            return Err(QRError::InsufficientSamples);
        }

        // Median of each class
        let mut centroid: [[f64; 3]; 8] = [[f64::default(); 3]; 8];
        for c in 0..8 {
            for k in 0..3 {
                let mut channel_samples: Vec<u8> = samples[c].iter().map(|s| s[k]).collect();
                centroid[c][k] = median(&mut channel_samples);
            }
        }

        // Infer missing colour centroids from black/white
        let on = |c: usize, k: usize| (c >> (2 - k)) & 1 == 1;
        for c in 1..7 {
            if samples[c].is_empty() {
                centroid[c] = std::array::from_fn(|k| {
                    if on(c, k) {
                        centroid[white_idx][k]
                    } else {
                        centroid[black_idx][k]
                    }
                });
            }
        }

        // Covariance of each class
        let mut class_cov = [[0.0; 9]; 8];
        let mut pool_cov: [f64; 9] = [0.0; 9];
        for i in 0..8 {
            compute_covariance(&samples[i], &centroid[i], &mut class_cov[i], &mut pool_cov);
        }

        // Average pool by dividing by total samples
        let total_sample: usize = samples.iter().map(|s| s.len()).sum();
        for pool_cv in pool_cov.iter_mut() {
            *pool_cv /= total_sample as f64;
        }

        // Blend class and pooled covariances
        for (c, cov) in class_cov.iter_mut().enumerate() {
            blend(cov, &pool_cov, samples[c].len());
        }

        // Ridge the diagonal elements. In an ideal qr, all elements will be near zero.
        // This makes the determinant zero and inverse is invalid
        for cov in &mut class_cov {
            ridged(cov);
        }

        // Inverse of covariance
        let inv: [[f64; 9]; 8] = std::array::from_fn(|i| inverse(&class_cov[i]));

        // Logarithm determinant of covariance
        let logdet: [f64; 8] = std::array::from_fn(|i| determinant(&class_cov[i]).ln());

        Ok(Self { centroid, inv, logdet })
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
        // Centre channel values
        let clr: [f64; 3] = std::array::from_fn(|i| color[i] as f64 - self.centroid[class][i]);
        let covi = &self.inv[class];

        (clr[0] * covi[0] + clr[1] * covi[3] + clr[2] * covi[6]) * clr[0]
            + (clr[0] * covi[1] + clr[1] * covi[4] + clr[2] * covi[7]) * clr[1]
            + (clr[0] * covi[2] + clr[1] * covi[5] + clr[2] * covi[8]) * clr[2]
    }
}

#[cfg(test)]
mod mahalanobis_tests {
    use image::Rgb;
    use rand::{rngs::StdRng, Rng, SeedableRng};

    use super::Mahalanobis;
    use crate::{ECLevel, QRBuilder, Version};

    #[test]
    fn test_classify_clean() {
        let data = "Hello, world! 🌎";
        for v in 1..=40 {
            let ver = Version::Normal(v);
            let qr = QRBuilder::new(data.as_bytes())
                .version(ver)
                .ec_level(ECLevel::L)
                .high_capacity(true)
                .build()
                .unwrap();

            let sampler = |x: i32, y: i32| Ok(Rgb::<u8>::from(*qr.get(x, y)).0);
            let maha = Mahalanobis::fit(ver, sampler).expect("Failed to fit classifier");

            let w = ver.width() as i32;
            for y in 0..w {
                for x in 0..w {
                    let exp = *qr.get(x, y);
                    let rgb = Rgb::<u8>::from(exp).0;
                    assert_eq!(maha.classify(&rgb), exp, "Version {v}: mismatch at ({x}, {y})");
                }
            }
        }
    }

    #[test]
    fn test_classify_noisy() {
        let data = "Hello, world! 🌎";
        let mut rng = StdRng::seed_from_u64(42);
        for v in 1..=40 {
            let ver = Version::Normal(v);
            let qr = QRBuilder::new(data.as_bytes())
                .version(ver)
                .ec_level(ECLevel::L)
                .high_capacity(true)
                .build()
                .unwrap();

            // Distort every module once up front, so fit and classify see the same image.
            // Brightness falls linearly from 1.0 at top-left to MIN_GAIN at bottom-right.
            let w = ver.width() as i32;
            let diag = (2 * (w - 1)) as f64;
            let grid: Vec<[u8; 3]> = (0..w * w)
                .map(|i| {
                    let (x, y) = (i % w, i / w);
                    let gain = 1.0 - (1.0 - MIN_GAIN) * (x + y) as f64 / diag;
                    let ideal = Rgb::<u8>::from(*qr.get(x, y)).0;
                    std::array::from_fn(|k| {
                        let val = ideal[k] as f64 * gain + gaussian(&mut rng) * NOISE_SD;
                        val.round().clamp(0.0, 255.0) as u8
                    })
                })
                .collect();

            let sampler = |x: i32, y: i32| {
                let x = if x < 0 { x + w } else { x };
                let y = if y < 0 { y + w } else { y };
                Ok(grid[(y * w + x) as usize])
            };
            let maha = Mahalanobis::fit(ver, sampler).expect("Failed to fit classifier");

            let mut errors = 0;
            for y in 0..w {
                for x in 0..w {
                    if maha.classify(&sampler(x, y).unwrap()) != *qr.get(x, y) {
                        errors += 1;
                    }
                }
            }

            let total = (w * w) as f64;
            let rate = errors as f64 / total;
            assert!(
                rate <= MAX_ERROR_RATE,
                "Version {v}: {errors}/{total} modules misclassified ({:.2}%)",
                rate * 100.0
            );
        }
    }

    /// Standard normal sample via the Box-Muller transform.
    fn gaussian(rng: &mut impl Rng) -> f64 {
        let u1 = 1.0 - rng.random::<f64>(); // (0, 1], keeps ln finite
        let u2 = rng.random::<f64>();
        (-2.0 * u1.ln()).sqrt() * (std::f64::consts::TAU * u2).cos()
    }

    const MIN_GAIN: f64 = 0.6;

    const NOISE_SD: f64 = 12.0;

    const MAX_ERROR_RATE: f64 = 0.01;
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
    let white_idx = Color::White as usize;

    let (left, right) = if x > 0 { (-3, 4) } else { (-4, 3) };
    let (top, bottom) = if y > 0 { (-3, 4) } else { (-4, 3) };

    for i in left..=right {
        for j in top..=bottom {
            let Ok(sample) = sampler(x + i, y + j) else { continue };
            let class = match (i, j) {
                (4 | -4, _) | (_, 4 | -4) => white_idx,
                (3 | -3, _) | (_, 3 | -3) => ring_idx,
                (2 | -2, _) | (_, 2 | -2) => white_idx,
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
    let black_idx = Color::Black as usize;
    let white_idx = Color::White as usize;

    for i in -2..=2 {
        for j in -2..=2 {
            let Ok(sample) = sampler(x + i, y + j) else { continue };
            let class = match (i, j) {
                (-2 | 2, _) | (_, -2 | 2) | (0, 0) => black_idx,
                _ => white_idx,
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
    let black_idx = Color::Black as usize;
    let white_idx = Color::White as usize;

    if hor {
        for i in start..=end {
            let Ok(sample) = sampler(i, fixed) else { continue };
            let class = if i & 1 == 0 { black_idx } else { white_idx };
            samples[class].push(sample);
        }
    } else {
        for j in start..=end {
            let Ok(sample) = sampler(fixed, j) else { continue };
            let class = if j & 1 == 0 { black_idx } else { white_idx };
            samples[class].push(sample);
        }
    }
}

#[cfg(test)]
mod sampler_tests {
    use image::Rgb;

    use super::{sample_alignment, sample_finder, sample_timing};
    use crate::{metadata::Color, ECLevel, QRBuilder, Version};

    #[test]
    fn test_sample_finder() {
        let data = "Hello, world! 🌎";
        let ver = Version::Normal(7);
        let qr = QRBuilder::new(data.as_bytes())
            .version(ver)
            .ec_level(ECLevel::L)
            .high_capacity(true)
            .build()
            .unwrap();

        let sampler = |x: i32, y: i32| Ok(Rgb::<u8>::from(*qr.get(x, y)).0);

        // Finder centre, ring colour, stone colour
        let finders = [
            (-4, 3, Color::Red, Color::Cyan),
            (3, 3, Color::Green, Color::Magenta),
            (3, -4, Color::Blue, Color::Yellow),
        ];

        for (x, y, ring, stone) in finders {
            let mut samples: [Vec<[u8; 3]>; 8] = std::array::from_fn(|_| Vec::new());
            sample_finder(x, y, &sampler, &mut samples, ring, stone);

            // 8x8 block: 24 ring, 9 stone, 16 inner white ring + 15 separator
            for (c, sample) in samples.iter().enumerate() {
                let clr = Color::try_from(c as u8).unwrap();
                let exp_len = match clr {
                    _ if clr == ring => 24,
                    _ if clr == stone => 9,
                    Color::White => 31,
                    _ => 0,
                };
                assert_eq!(samples[c].len(), exp_len, "Finder ({x}, {y}): {clr:?} count");

                let exp_rgb = Rgb::<u8>::from(clr).0;
                for s in sample {
                    assert_eq!(*s, exp_rgb, "Finder ({x}, {y}): {clr:?} colour");
                }
            }
        }
    }

    #[test]
    fn test_sample_alignment() {
        let data = "Hello, world! 🌎";
        for v in 2..=40 {
            let ver = Version::Normal(v);
            let qr = QRBuilder::new(data.as_bytes())
                .version(ver)
                .ec_level(ECLevel::L)
                .high_capacity(true)
                .build()
                .unwrap();

            let sampler = |x: i32, y: i32| Ok(Rgb::<u8>::from(*qr.get(x, y)).0);

            let aps = ver.alignment_pattern();
            let w = ver.width() as i32;
            for &x in aps {
                for &y in aps {
                    // Skip centres that overlap finders
                    if (x == 6 && (y == 6 || y == w - 7)) || (x == w - 7 && y == 6) {
                        continue;
                    }

                    let mut samples: [Vec<[u8; 3]>; 8] = std::array::from_fn(|_| Vec::new());
                    sample_alignment(x, y, &sampler, &mut samples);

                    // 5x5 block: 16 outer ring + 1 centre black, 8 inner white ring
                    for (c, sample) in samples.iter().enumerate() {
                        let clr = Color::try_from(c as u8).unwrap();
                        let exp_len = match clr {
                            Color::Black => 17,
                            Color::White => 8,
                            _ => 0,
                        };
                        assert_eq!(
                            samples[c].len(),
                            exp_len,
                            "Version {v}, alignment ({x}, {y}): {clr:?} count"
                        );

                        let exp_rgb = Rgb::<u8>::from(clr).0;
                        for s in sample {
                            assert_eq!(
                                *s, exp_rgb,
                                "Version {v}, alignment ({x}, {y}): {clr:?} colour"
                            );
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn test_sample_timing() {
        let data = "Hello, world! 🌎";
        for v in 1..=40 {
            let ver = Version::Normal(v);
            let qr = QRBuilder::new(data.as_bytes())
                .version(ver)
                .ec_level(ECLevel::L)
                .high_capacity(true)
                .build()
                .unwrap();

            let sampler = |x: i32, y: i32| Ok(Rgb::<u8>::from(*qr.get(x, y)).0);

            let w = ver.width() as i32;
            for hor in [true, false] {
                let mut samples: [Vec<[u8; 3]>; 8] = std::array::from_fn(|_| Vec::new());
                sample_timing(8, w - 9, 6, hor, &sampler, &mut samples);

                // w - 16 modules from 8 to w - 9, both ends black
                for (c, sample) in samples.iter().enumerate() {
                    let clr = Color::try_from(c as u8).unwrap();
                    let exp_len = match clr {
                        Color::Black => (w - 15) / 2,
                        Color::White => (w - 17) / 2,
                        _ => 0,
                    } as usize;
                    assert_eq!(
                        samples[c].len(),
                        exp_len,
                        "Version {v}, horizontal {hor}: {clr:?} count"
                    );

                    let exp_rgb = Rgb::<u8>::from(clr).0;
                    for s in sample {
                        assert_eq!(*s, exp_rgb, "Version {v}, horizontal {hor}: {clr:?} colour");
                    }
                }
            }
        }
    }
}

// Covariance matrix
//------------------------------------------------------------------------------

fn compute_covariance(
    samples: &[[u8; 3]],
    centroid: &[f64; 3],
    class_cov: &mut [f64; 9],
    pool_cov: &mut [f64; 9],
) {
    if samples.is_empty() {
        return;
    }

    let len = samples.len();

    // Covariance matrix
    for px in samples {
        for (i, (cv, pool_cv)) in class_cov.iter_mut().zip(pool_cov.iter_mut()).enumerate() {
            let (r, c) = (i / 3, i % 3);
            let val = (px[r] as f64 - centroid[r]) * (px[c] as f64 - centroid[c]);
            *cv += val;
            *pool_cv += val;
        }
    }

    for cv in class_cov.iter_mut() {
        *cv /= len as f64;
    }
}

// 0.0 median is overwritten in fit by inferring from black/white
fn median(data: &mut [u8]) -> f64 {
    if data.is_empty() {
        return f64::default();
    }

    let len = data.len();
    let mid = len / 2;

    if len & 1 == 1 {
        let (_, &mut median, _) = data.select_nth_unstable(mid);
        median as f64
    } else {
        let (left_arr, &mut mid_right, _) = data.select_nth_unstable(mid);
        let mid_left = *left_arr.iter().max().unwrap();
        (mid_left as f64 + mid_right as f64) / 2.0
    }
}

/// Adds a diagonal ridge proportional to the mean variance, so inversion stays well-posed.
fn ridged(data: &mut [f64; 9]) {
    let eps = (RIDGE_FACTOR * (data[0] + data[4] + data[8]) / 3.0).max(VARIANCE_FLOOR);
    data[0] += eps;
    data[4] += eps;
    data[8] += eps;
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

fn blend(class: &mut [f64; 9], pooled: &[f64; 9], n: usize) {
    let w = if n < MIN_CLASS_SAMPLE { 1.0 } else { SHRINK };
    for i in 0..9 {
        class[i] = class[i] * (1.0 - w) + pooled[i] * w;
    }
}

#[cfg(test)]
mod math_tests {
    use super::{determinant, inverse, median};

    #[test]
    fn test_determinant() {
        let identity = [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0];
        assert_eq!(determinant(&identity), 1.0);

        let diagonal = [2.0, 0.0, 0.0, 0.0, 3.0, 0.0, 0.0, 0.0, 4.0];
        assert_eq!(determinant(&diagonal), 24.0);

        let general = [2.0, -3.0, 1.0, 2.0, 0.0, -1.0, 1.0, 4.0, 5.0];
        assert_eq!(determinant(&general), 49.0);

        let singular = [1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0];
        assert_eq!(determinant(&singular), 0.0);
    }

    #[test]
    fn test_inverse() {
        let diagonal = [2.0, 0.0, 0.0, 0.0, 4.0, 0.0, 0.0, 0.0, 8.0];
        assert_eq!(inverse(&diagonal), [0.5, 0.0, 0.0, 0.0, 0.25, 0.0, 0.0, 0.0, 0.125]);

        // Non-symmetric catches transposed cofactors; symmetric is the covariance case
        let general = [2.0, -3.0, 1.0, 2.0, 0.0, -1.0, 1.0, 4.0, 5.0];
        let covariance = [400.0, 120.0, -60.0, 120.0, 250.0, 30.0, -60.0, 30.0, 90.0];
        for m in [general, covariance] {
            let inv = inverse(&m);
            for r in 0..3 {
                for c in 0..3 {
                    let prod: f64 = (0..3).map(|k| m[r * 3 + k] * inv[k * 3 + c]).sum();
                    let exp = if r == c { 1.0 } else { 0.0 };
                    assert!((prod - exp).abs() < 1e-12, "M * M^-1 at ({r}, {c}) is {prod}");
                }
            }
        }
    }

    #[test]
    fn test_median() {
        assert_eq!(median(&mut []), 0.0);
        assert_eq!(median(&mut [7]), 7.0);
        assert_eq!(median(&mut [3, 1, 2]), 2.0);
        assert_eq!(median(&mut [4, 1, 3, 2]), 2.5);
        assert_eq!(median(&mut [9, 9, 1, 9]), 9.0);

        // Sum of middle pair exceeds u8
        assert_eq!(median(&mut [255, 0, 255, 200]), 227.5);
    }
}

// Global constants
//------------------------------------------------------------------------------

const RIDGE_FACTOR: f64 = 1e-3;

const SHRINK: f64 = 0.5;

const VARIANCE_FLOOR: f64 = 1.0;

const MIN_CLASS_SAMPLE: usize = 8;
