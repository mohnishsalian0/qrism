//! Discriminant-analysis recovery: nearest measured centroid under a *learned* metric.
//!
//! [`super::euclid_measured`] picks the nearest palette centroid in plain Euclidean
//! distance, which implicitly asserts that every direction in colour space is equally
//! informative. It is not. The dominant within-class variation in these photographs is
//! shared illumination — shading and exposure move all three channels together — so a blue
//! module in shadow drifts toward the black centroid along a direction that carries almost
//! no colour information. Euclidean distance cannot tell that drift apart from a genuine
//! colour difference, which is what makes Black/Blue the closest and most-confused pair.
//!
//! This measures distance in units of the *observed* scatter instead, scoring each colour by
//! the Gaussian discriminant
//!
//! ```text
//!   g_c(x) = (x - mu_c)^T * Sigma_c^-1 * (x - mu_c) + ln|Sigma_c|
//! ```
//!
//! and taking the smallest. Inverting `Sigma` is what converts raw channel units into
//! noise-widths: the off-diagonal terms encode the shared-brightness direction, so the metric
//! discounts movement along it and sharpens movement across it. With one shared covariance
//! the `ln|Sigma|` term is constant and the rule reduces to nearest centroid under a single
//! learned metric (linear boundaries).
//!
//! Centroids are the same per-colour medians [`super::euclid_measured`] uses, so a benchmark
//! comparison between the two isolates the change of metric rather than mixing in a change of
//! centroid estimator.
//!
//! How `Sigma_c` is estimated is the one real choice, controlled by `shrink`:
//!   - `1.0` — one covariance pooled over all eight classes (LDA). Cheapest to estimate.
//!   - `0.0` — each class its own (QDA). More expressive, but the calibration groups are
//!     wildly unbalanced: Black contributes 17 samples against White's ~300, so its
//!     covariance is estimated from very little.
//!   - between — each class's own scatter blended toward the pooled one (regularized
//!     discriminant analysis), which is the usual answer when some classes are starved.

use super::linalg::{det3, inv3, IDENTITY3};
use super::DirectRecovery;
use crate::calibration::{median_rgb, GroupedSamples};
use qrism::Color;

/// Classes with fewer samples than this borrow the pooled scatter outright rather than
/// contribute a covariance estimated from almost nothing.
const MIN_CLASS: usize = 8;

/// Diagonal ridge added before inversion, as a fraction of the mean variance. Keeps a
/// degenerate group (every sample identical, e.g. a clipped highlight) invertible.
const RIDGE: f64 = 1e-3;

/// Nearest-centroid classifier under a learned covariance metric.
pub(crate) struct Mahalanobis {
    palette: [[f64; 3]; 8],
    inv: [[[f64; 3]; 3]; 8],
    logdet: [f64; 8],
}

impl Mahalanobis {
    /// One shared covariance across every class — linear discriminant analysis.
    pub(crate) fn fit_lda(ng: &GroupedSamples) -> Self {
        Self::fit(ng, 1.0)
    }

    /// A separate covariance per class — quadratic discriminant analysis.
    pub(crate) fn fit_qda(ng: &GroupedSamples) -> Self {
        Self::fit(ng, 0.0)
    }

    /// Per-class covariance shrunk halfway toward the pooled one.
    pub(crate) fn fit_rda(ng: &GroupedSamples) -> Self {
        Self::fit(ng, 0.5)
    }

    /// Fits centroids and covariances. `shrink` blends each class's own scatter toward the
    /// pooled scatter: 1.0 is fully pooled (LDA), 0.0 fully per-class (QDA).
    fn fit(ng: &GroupedSamples, shrink: f64) -> Self {
        let palette: [[f64; 3]; 8] = std::array::from_fn(|c| median_rgb(&ng[c]));

        // Scatter of each class about its own centroid, accumulated per class and pooled.
        let mut scatter = [[[0.0f64; 3]; 3]; 8];
        let mut pooled = [[0.0f64; 3]; 3];
        let mut n = [0usize; 8];
        let mut n_tot = 0usize;
        for c in 0..8 {
            n[c] = ng[c].len();
            n_tot += n[c];
            for x in &ng[c] {
                let d: [f64; 3] = std::array::from_fn(|k| x[k] - palette[c][k]);
                for r in 0..3 {
                    for q in 0..3 {
                        scatter[c][r][q] += d[r] * d[q];
                        pooled[r][q] += d[r] * d[q];
                    }
                }
            }
        }

        let pooled_cov = scaled(&pooled, 1.0 / n_tot.max(1) as f64);

        let mut inv = [IDENTITY3; 8];
        let mut logdet = [0.0f64; 8];
        for c in 0..8 {
            // A starved class gets the pooled scatter; otherwise blend by `shrink`.
            let cov = if n[c] >= MIN_CLASS {
                let own = scaled(&scatter[c], 1.0 / n[c] as f64);
                blend(&own, &pooled_cov, shrink)
            } else {
                pooled_cov
            };
            let cov = ridged(&cov);
            inv[c] = inv3(&cov).unwrap_or(IDENTITY3);
            logdet[c] = det3(&cov).max(f64::MIN_POSITIVE).ln();
        }

        Mahalanobis { palette, inv, logdet }
    }

    /// The pooled per-channel standard deviations and correlation matrix, for reporting what
    /// the metric actually learned.
    pub(crate) fn pooled_structure(ng: &GroupedSamples) -> ([f64; 3], [[f64; 3]; 3]) {
        let palette: [[f64; 3]; 8] = std::array::from_fn(|c| median_rgb(&ng[c]));
        let mut pooled = [[0.0f64; 3]; 3];
        let mut n_tot = 0usize;
        for c in 0..8 {
            n_tot += ng[c].len();
            for x in &ng[c] {
                let d: [f64; 3] = std::array::from_fn(|k| x[k] - palette[c][k]);
                for r in 0..3 {
                    for q in 0..3 {
                        pooled[r][q] += d[r] * d[q];
                    }
                }
            }
        }
        let cov = scaled(&pooled, 1.0 / n_tot.max(1) as f64);
        let sd: [f64; 3] = std::array::from_fn(|k| cov[k][k].max(0.0).sqrt());
        let corr = std::array::from_fn(|r| {
            std::array::from_fn(|q| cov[r][q] / (sd[r] * sd[q]).max(f64::MIN_POSITIVE))
        });
        (sd, corr)
    }
}

impl DirectRecovery for Mahalanobis {
    fn classify(&self, x: [f64; 3]) -> Color {
        let best = (0..8usize)
            .min_by(|&a, &b| {
                self.score(x, a).partial_cmp(&self.score(x, b)).unwrap()
            })
            .unwrap();
        Color::try_from(best as u8).unwrap()
    }
}

impl Mahalanobis {
    /// The chosen colour together with how close the call was: the gap between the runner-up
    /// discriminant and the winner. Zero means two colours scored identically; a large gap
    /// means the module sat squarely inside one class. The scale is per-image and arbitrary,
    /// so only the ranking of modules within one symbol is meaningful — which is all an
    /// erasure decoder needs, since it erases the weakest positions in each block.
    pub(crate) fn classify_with_margin(&self, x: [f64; 3]) -> (Color, f64) {
        let mut best = (f64::INFINITY, 0usize);
        let mut second = f64::INFINITY;
        for c in 0..8usize {
            let s = self.score(x, c);
            if s < best.0 {
                second = best.0;
                best = (s, c);
            } else if s < second {
                second = s;
            }
        }
        (Color::try_from(best.1 as u8).unwrap(), second - best.0)
    }

    /// The chosen colour plus, per channel, how safe *that channel's bit* was.
    ///
    /// [`Self::classify_with_margin`] returns one number per module, but a module read as the
    /// wrong colour does not corrupt every layer: mistaking Blue for Black flips only the blue
    /// bit, leaving the red and green layers intact. A single per-module confidence therefore
    /// dilutes the signal across three layers, two of which may be perfectly fine.
    ///
    /// This instead scores each channel separately, as the gap between the best colour whose
    /// bit `k` agrees with the decision and the best colour whose bit `k` disagrees — a
    /// per-bit likelihood ratio. Small means that bit was nearly a coin flip and is the right
    /// thing for an erasure decoder to distrust in *that* layer alone.
    pub(crate) fn classify_with_channel_margins(&self, x: [f64; 3]) -> (Color, [f64; 3]) {
        let mut score = [0.0f64; 8];
        let mut best = (f64::INFINITY, 0usize);
        for c in 0..8usize {
            score[c] = self.score(x, c);
            if score[c] < best.0 {
                best = (score[c], c);
            }
        }

        let win = best.1;
        let margins = std::array::from_fn(|k| {
            let bit = |c: usize| (c >> (2 - k)) & 1;
            let want = bit(win);
            let mut agree = f64::INFINITY;
            let mut differ = f64::INFINITY;
            for c in 0..8usize {
                if bit(c) == want {
                    agree = agree.min(score[c]);
                } else {
                    differ = differ.min(score[c]);
                }
            }
            differ - agree
        });

        (Color::try_from(win as u8).unwrap(), margins)
    }

    /// Per-class Gaussian log-likelihood, `-g_c(x) / 2` up to a shared constant. Normalized
    /// across the eight classes this is the posterior under a uniform prior — the soft output
    /// the LLR recorder stores, from which per-bit LLRs and GF(8) symbol likelihoods follow.
    pub(crate) fn log_likelihoods(&self, x: [f64; 3]) -> [f64; 8] {
        std::array::from_fn(|c| -0.5 * self.score(x, c))
    }

    /// The Gaussian discriminant for one class: squared Mahalanobis distance plus the
    /// log-determinant that makes distances under different covariances comparable. Under a
    /// single pooled covariance the second term is constant and drops out.
    #[inline]
    fn score(&self, x: [f64; 3], c: usize) -> f64 {
        let d: [f64; 3] = std::array::from_fn(|k| x[k] - self.palette[c][k]);
        let m = &self.inv[c];
        let mut q = 0.0;
        for r in 0..3 {
            for s in 0..3 {
                q += d[r] * m[r][s] * d[s];
            }
        }
        q + self.logdet[c]
    }
}

// Matrix helpers local to the covariance fit
//------------------------------------------------------------------------------

fn scaled(m: &[[f64; 3]; 3], k: f64) -> [[f64; 3]; 3] {
    std::array::from_fn(|r| std::array::from_fn(|c| m[r][c] * k))
}

fn blend(own: &[[f64; 3]; 3], pooled: &[[f64; 3]; 3], shrink: f64) -> [[f64; 3]; 3] {
    std::array::from_fn(|r| {
        std::array::from_fn(|c| (1.0 - shrink) * own[r][c] + shrink * pooled[r][c])
    })
}

/// Adds a diagonal ridge proportional to the mean variance, so inversion stays well-posed.
fn ridged(m: &[[f64; 3]; 3]) -> [[f64; 3]; 3] {
    let eps = RIDGE * (m[0][0] + m[1][1] + m[2][2]).max(0.0) / 3.0;
    let mut out = *m;
    for k in 0..3 {
        out[k][k] += eps.max(f64::MIN_POSITIVE);
    }
    out
}
