//! HiQ's pretrained colour classifiers, ported to run on qrism's samples.
//!
//! The comparison with the HiQ decoder in the paper is end to end: each decoder localizes and
//! samples for itself, so the gap mixes classifier, sampling and localization. These ports
//! run HiQ's shipped models, unchanged, on the module grid every other pipeline here gets,
//! after HiQ's own white-point normalization ([`crate::normalization::hiq_white`]). The only
//! difference from the other pipelines is then the classifier.
//!
//! Parameters come from `hiq_models.txt`, exported verbatim from the HiQ repository by
//! `hiq_export.py`; nothing is refitted. Each classifier is a transcription of its Java class,
//! down to the order of the floating-point sums, so the parity pass (`-- hiq_parity`) can
//! demand identical decisions on HiQ's own inputs. What HiQ consumes, and where this cannot
//! match it, is in `hiq_port.md`.
//!
//!   - [`HiqQda`]  — `QDA.java` with `Qmodel_CMI.txt`, on the centre module alone.
//!   - [`HiqCmi`]  — the same QDA after cross-module interference cancellation over the
//!     centre and its four edge neighbours, with the indexing slip in `CMIModel.cancelCMI`
//!     corrected (all five terms, as the HiQ bench's `CMIModelFixed`).
//!   - [`HiqLsvm`] — `LayeredSVM.java` with `layeredSVMs_ip6plus_mixed_new_aug.txt`.

use std::collections::HashMap;
use std::sync::OnceLock;

use super::DirectRecovery;
use qrism::Color;

const MODELS: &str = include_str!("../hiq_models.txt");

/// Every `key value value …` line of the exported model file.
fn params() -> &'static HashMap<String, Vec<f64>> {
    static P: OnceLock<HashMap<String, Vec<f64>>> = OnceLock::new();
    P.get_or_init(|| {
        MODELS
            .lines()
            .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
            .map(|l| {
                let mut it = l.split_whitespace();
                let key = it.next().unwrap().to_string();
                let vals: Vec<f64> =
                    it.map(|v| v.parse().unwrap_or_else(|_| panic!("{key}: bad value {v}"))).collect();
                (key, vals)
            })
            .collect()
    })
}

fn get(key: &str) -> &'static [f64] {
    params().get(key).unwrap_or_else(|| panic!("hiq_models.txt: missing {key}"))
}

fn arr<const N: usize>(key: &str) -> [f64; N] {
    get(key).try_into().unwrap_or_else(|_| panic!("hiq_models.txt: {key} is not {N} values"))
}

// QDA
//------------------------------------------------------------------------------

/// HiQ's quadratic discriminant: eight Gaussian classes with stored inverse covariances.
pub(crate) struct HiqQda {
    groups: [u8; 8],
    logprior: [f64; 8],
    mean: [[f64; 3]; 8],
    invcov: [[[f64; 3]; 3]; 8],
    halflogdet: [f64; 8],
}

impl HiqQda {
    pub(crate) fn shipped() -> Self {
        HiqQda {
            groups: arr::<8>("qda.groups").map(|g| g as u8),
            logprior: arr("qda.logprior"),
            mean: std::array::from_fn(|c| arr(&format!("qda.mean.{c}"))),
            invcov: std::array::from_fn(|c| {
                let m: [f64; 9] = arr(&format!("qda.invcov.{c}"));
                std::array::from_fn(|r| std::array::from_fn(|q| m[3 * r + q]))
            }),
            halflogdet: arr("qda.halflogdet"),
        }
    }

    /// `QDA.predict_value`: the group of the largest discriminant, first on ties. A
    /// non-finite input that scores no class leaves the group at -1, which `predict` turns
    /// into every channel lit.
    pub(crate) fn group(&self, x: [f64; 3]) -> Option<u8> {
        let mut best = None;
        let mut max = f64::NEG_INFINITY;
        for c in 0..8 {
            let f = self.discriminant(x, c);
            if f > max {
                max = f;
                best = Some(self.groups[c]);
            }
        }
        best
    }

    /// `getDiscriminantFunctionValues` for one class, with HiQ's evaluation order:
    /// `t = M·d` row by row, then `dᵀt`.
    #[inline]
    fn discriminant(&self, x: [f64; 3], c: usize) -> f64 {
        let d: [f64; 3] = std::array::from_fn(|j| x[j] - self.mean[c][j]);
        let m = &self.invcov[c];
        let mut t = [0.0f64; 3];
        for r in 0..3 {
            for j in 0..3 {
                t[r] += d[j] * m[r][j];
            }
        }
        let mut q = 0.0;
        for j in 0..3 {
            q += t[j] * d[j];
        }
        self.logprior[c] - 0.5 * q - self.halflogdet[c]
    }
}

impl DirectRecovery for HiqQda {
    fn classify(&self, x: [f64; 3]) -> Color {
        // The group label is the colour's channel bits, the encoding `Color` uses.
        Color::try_from(self.group(x).unwrap_or(0b111)).unwrap()
    }
}

// Cross-module interference cancellation
//------------------------------------------------------------------------------

/// Which neighbours fill the four non-centre slots (left, right, up, down).
#[derive(Clone, Copy)]
pub(crate) enum Neighbours {
    /// All four edge neighbours, zero where one falls outside the symbol: what the
    /// sampler's slot layout and the CMI weights describe.
    Edge,
    /// What `DefaultGridSampler.colorSampleGrid` actually fetches. Its `else if` takes the
    /// right neighbour only in column 0 and the down neighbour only in row 0; every other
    /// right and down slot is zero. HiQ's own end-to-end CMI numbers ran this way.
    HiqShipped,
}

impl Neighbours {
    /// Grid cells for the left, right, up and down slots of module `(gx, gy)`, `None` for a
    /// zero slot.
    pub(crate) fn cells(self, gx: usize, gy: usize, dim: usize) -> [Option<(usize, usize)>; 4] {
        let left = (gx > 0).then(|| (gx - 1, gy));
        let up = (gy > 0).then(|| (gx, gy - 1));
        let (right, down) = match self {
            Neighbours::Edge => {
                ((gx + 1 < dim).then_some((gx + 1, gy)), (gy + 1 < dim).then_some((gx, gy + 1)))
            }
            Neighbours::HiqShipped => (
                (gx == 0 && dim > 1).then_some((gx + 1, gy)),
                (gy == 0 && dim > 1).then_some((gx, gy + 1)),
            ),
        };
        [left, right, up, down]
    }
}

/// HiQ's QDA behind the corrected interference cancellation.
pub(crate) struct HiqCmi {
    qda: HiqQda,
    weights: [f64; 5],
}

impl HiqCmi {
    pub(crate) fn shipped() -> Self {
        HiqCmi { qda: HiqQda::shipped(), weights: arr("cmi.weights") }
    }

    /// `CMIModelFixed.cancel` for a single-row model: per channel, the weighted sum over
    /// the five slots, in slot order.
    #[inline]
    pub(crate) fn cancel(&self, v: &[f64; 15]) -> [f64; 3] {
        std::array::from_fn(|k| {
            let mut s = 0.0;
            for m in 0..5 {
                s += self.weights[m] * v[3 * m + k];
            }
            s
        })
    }

    pub(crate) fn classify15(&self, v: &[f64; 15]) -> Color {
        self.qda.classify(self.cancel(v))
    }

    /// The 15-value input for module `(gx, gy)` of a normalized grid: centre, then the four
    /// neighbour slots, a zero triple where a slot is empty.
    pub(crate) fn features(
        grid: &[Vec<[f64; 3]>],
        gx: usize,
        gy: usize,
        nb: Neighbours,
    ) -> [f64; 15] {
        let mut v = [0.0f64; 15];
        v[..3].copy_from_slice(&grid[gy][gx]);
        for (slot, cell) in nb.cells(gx, gy, grid.len()).into_iter().enumerate() {
            if let Some((x, y)) = cell {
                v[3 * (slot + 1)..3 * (slot + 2)].copy_from_slice(&grid[y][x]);
            }
        }
        v
    }

    /// Classifies every module of a normalized grid.
    pub(crate) fn decide_grid(&self, grid: &[Vec<[f64; 3]>], nb: Neighbours) -> Vec<Vec<Color>> {
        (0..grid.len())
            .map(|gy| {
                (0..grid.len()).map(|gx| self.classify15(&Self::features(grid, gx, gy, nb))).collect()
            })
            .collect()
    }
}

// Layered SVM
//------------------------------------------------------------------------------

/// HiQ's layered SVM: one linear SVM per channel on a (possibly polynomial) expansion.
pub(crate) struct HiqLsvm {
    layers: [(Vec<f64>, f64); 3],
}

impl HiqLsvm {
    pub(crate) fn shipped() -> Self {
        HiqLsvm {
            layers: std::array::from_fn(|i| {
                (get(&format!("lsvm.w.{i}")).to_vec(), arr::<1>(&format!("lsvm.b.{i}"))[0])
            }),
        }
    }

    /// `svm_predict_value`: `w·φ(x) − b`, the channel dark when negative.
    #[inline]
    pub(crate) fn value(&self, x: [f64; 3], layer: usize) -> f64 {
        let (w, b) = &self.layers[layer];
        let mut y = 0.0;
        match w.len() {
            10 => poly2(x).iter().zip(w).for_each(|(a, c)| y += c * a),
            20 => poly3(x).iter().zip(w).for_each(|(a, c)| y += c * a),
            _ => x.iter().zip(w).for_each(|(a, c)| y += c * a),
        }
        y - b
    }
}

impl DirectRecovery for HiqLsvm {
    fn classify(&self, x: [f64; 3]) -> Color {
        let lit = |l: usize| !(self.value(x, l) < 0.0) as u8;
        Color::try_from((lit(0) << 2) | (lit(1) << 1) | lit(2)).unwrap()
    }
}

/// `LayeredSVM.convt2poly`.
fn poly2(x: [f64; 3]) -> [f64; 10] {
    let mut o = [0.0; 10];
    o[0] = 1.0;
    for i in 1..4 {
        o[i] = x[i - 1];
        o[i + 3] = x[i - 1] * x[i - 1];
    }
    o[7] = x[0] * x[1];
    o[8] = x[1] * x[2];
    o[9] = x[0] * x[2];
    o
}

/// `LayeredSVM.convt2poly3`.
fn poly3(x: [f64; 3]) -> [f64; 20] {
    let mut o = [0.0; 20];
    o[0] = 1.0;
    for i in 1..4 {
        o[i] = x[i - 1];
        o[i + 3] = x[i - 1] * x[i - 1];
        o[i + 6] = x[i - 1] * x[i - 1] * x[i - 1];
    }
    o[10] = x[0] * x[1];
    o[11] = x[1] * x[2];
    o[12] = x[0] * x[2];
    o[13] = x[0] * x[0] * x[1];
    o[14] = x[1] * x[1] * x[2];
    o[15] = x[0] * x[0] * x[2];
    o[16] = x[0] * x[1] * x[1];
    o[17] = x[1] * x[2] * x[2];
    o[18] = x[0] * x[2] * x[2];
    o[19] = x[0] * x[1] * x[2];
    o
}
