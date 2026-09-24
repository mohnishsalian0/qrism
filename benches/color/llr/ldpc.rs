//! A binary LDPC code rate-matched to a QR layer, decoded by belief propagation.
//!
//! The construction is irregular repeat-accumulate (IRA): every information bit joins
//! [`INFO_DEGREE`] checks chosen at random, and the parity bits form a dual-diagonal
//! accumulator chain, so check `j` covers parity bits `j` and `j - 1`. That makes encoding a
//! running XOR rather than a matrix inversion, and is the same shape 802.11n and 5G NR use.
//! Codeword bits are scattered over the layer's data region by a fixed random permutation, so
//! a spatially clustered defect does not land on a run of the accumulator chain.
//!
//! This is a plain, unoptimized code: a regular information degree and no girth
//! conditioning. A code with a tuned degree distribution does a few tenths of a dB better, so
//! the gain this reports over Reed-Solomon is a lower bound on what LDPC could deliver.
//!
//! Decoding is flooding sum-product in the log domain, stopping as soon as the hard decisions
//! satisfy every check. It uses the calibrated LLRs at face value, so it is the decoder most
//! sensitive to the calibration the replay applies.

use rand::rngs::StdRng;
use rand::seq::SliceRandom;
use rand::{Rng, SeedableRng};

use super::BlockOutcome;

const INFO_DEGREE: usize = 3;
const MAX_ITERS: usize = 60;

/// LLR magnitudes are clamped here. Keeps `phi` finite at both ends.
const LLR_CAP: f64 = 30.0;

pub(crate) struct Ldpc {
    n: usize,
    k: usize,
    /// Checks in CSR form: the variables of check `j` are `edge_var[chk_ptr[j]..chk_ptr[j+1]]`.
    chk_ptr: Vec<usize>,
    edge_var: Vec<u32>,
    /// Codeword bit -> stream position in the layer's data region.
    perm: Vec<u32>,
}

impl Ldpc {
    /// Builds the code for `n` channel bits carrying `k` data bits. Deterministic in `(n, k)`.
    pub(crate) fn new(n: usize, k: usize) -> Self {
        let m = n - k;
        let mut rng = StdRng::seed_from_u64(((n as u64) << 32) | k as u64);

        // Information sockets dealt round-robin over the checks so their degrees stay level,
        // then shuffled and taken `INFO_DEGREE` at a time. A repeated check within one column
        // is swapped away with a random socket elsewhere.
        let mut sock: Vec<usize> = (0..k * INFO_DEGREE).map(|i| i % m).collect();
        sock.shuffle(&mut rng);
        for v in 0..k {
            for d in 1..INFO_DEGREE {
                let at = v * INFO_DEGREE + d;
                for _ in 0..64 {
                    let col = &sock[v * INFO_DEGREE..at];
                    if !col.contains(&sock[at]) {
                        break;
                    }
                    let other = rng.random_range(0..sock.len());
                    sock.swap(at, other);
                }
            }
        }

        let mut rows: Vec<Vec<u32>> = vec![Vec::new(); m];
        for (i, &c) in sock.iter().enumerate() {
            let v = (i / INFO_DEGREE) as u32;
            // A duplicate the repair missed would cancel in GF(2); drop it instead.
            if !rows[c].contains(&v) {
                rows[c].push(v);
            }
        }
        for (j, row) in rows.iter_mut().enumerate() {
            row.push((k + j) as u32);
            if j > 0 {
                row.push((k + j - 1) as u32);
            }
        }

        let mut chk_ptr = Vec::with_capacity(m + 1);
        let mut edge_var = Vec::new();
        chk_ptr.push(0);
        for row in &rows {
            edge_var.extend_from_slice(row);
            chk_ptr.push(edge_var.len());
        }

        let mut perm: Vec<u32> = (0..n as u32).collect();
        perm.shuffle(&mut rng);

        Ldpc { n, k, chk_ptr, edge_var, perm }
    }

    fn m(&self) -> usize {
        self.n - self.k
    }

    fn row(&self, j: usize) -> &[u32] {
        &self.edge_var[self.chk_ptr[j]..self.chk_ptr[j + 1]]
    }

    /// Systematic encode: the data bits, then the accumulator parity.
    fn encode(&self, data: &[bool]) -> Vec<bool> {
        let k = self.k;
        let mut cw = data.to_vec();
        cw.resize(self.n, false);
        let mut prev = false;
        for j in 0..self.m() {
            let s = self.row(j).iter().filter(|&&v| (v as usize) < k).fold(false, |a, &v| a ^ cw[v as usize]);
            prev ^= s;
            cw[k + j] = prev;
        }
        cw
    }

    fn syndrome_ok(&self, hard: &[bool]) -> bool {
        (0..self.m()).all(|j| !self.row(j).iter().fold(false, |a, &v| a ^ hard[v as usize]))
    }

    /// Sends random data through the recorded channel and decodes it. `y` is the truth-relative
    /// LLR at each stream position: positive means the observation favoured whatever was sent.
    pub(crate) fn replay(&self, y: &[f64], rng: &mut StdRng) -> BlockOutcome {
        let data: Vec<bool> = (0..self.k).map(|_| rng.random()).collect();
        let cw = self.encode(&data);
        let ch: Vec<f64> = (0..self.n)
            .map(|i| {
                let l = y[self.perm[i] as usize].clamp(-LLR_CAP, LLR_CAP);
                if cw[i] {
                    -l
                } else {
                    l
                }
            })
            .collect();

        match self.decode(&ch) {
            Some(hard) if hard[..self.k] == data[..] => BlockOutcome::Ok,
            Some(_) => BlockOutcome::Wrong,
            None => BlockOutcome::Failed,
        }
    }

    /// Flooding sum-product. Returns the hard decisions once every check is satisfied, or
    /// `None` if that never happens within [`MAX_ITERS`].
    fn decode(&self, ch: &[f64]) -> Option<Vec<bool>> {
        let mut c2v = vec![0.0f64; self.edge_var.len()];
        let mut tot = vec![0.0f64; self.n];
        let mut hard = vec![false; self.n];
        let mut v2c: Vec<f64> = Vec::new();

        for _ in 0..=MAX_ITERS {
            tot.copy_from_slice(ch);
            for (e, &v) in self.edge_var.iter().enumerate() {
                tot[v as usize] += c2v[e];
            }
            for (h, &t) in hard.iter_mut().zip(&tot) {
                *h = t < 0.0;
            }
            if self.syndrome_ok(&hard) {
                return Some(hard);
            }

            for j in 0..self.m() {
                let (lo, hi) = (self.chk_ptr[j], self.chk_ptr[j + 1]);
                v2c.clear();
                let mut sign = false;
                let mut sum = 0.0;
                for e in lo..hi {
                    let m = (tot[self.edge_var[e] as usize] - c2v[e]).clamp(-LLR_CAP, LLR_CAP);
                    sign ^= m < 0.0;
                    let p = phi(m.abs());
                    sum += p;
                    v2c.push(m);
                }
                for (i, e) in (lo..hi).enumerate() {
                    let m = v2c[i];
                    let mag = phi((sum - phi(m.abs())).max(0.0));
                    c2v[e] = if sign ^ (m < 0.0) { -mag } else { mag };
                }
            }
        }
        None
    }
}

/// `phi(x) = -ln tanh(x / 2)`, its own inverse on `x > 0`.
fn phi(x: f64) -> f64 {
    let x = x.clamp(1e-12, LLR_CAP);
    -((x / 2.0).tanh()).ln()
}
