use super::{galois::*, Block, MAX_BLOCK_SIZE, MAX_EC_SIZE};
use crate::utils::{QRError, QRResult};

// Rectifier
//------------------------------------------------------------------------------

impl Block {
    /// Corrects up to `ec_len / 2` byte errors at unknown positions.
    pub fn rectify(&mut self) -> QRResult<&[u8]> {
        let synd = match self.syndromes() {
            Ok(()) => return Ok(self.data()),
            Err(s) => s,
        };
        let sig = self.berlkamp_massey(&synd, self.ec_len())?;
        self.correct(&synd, &sig)
    }

    /// Corrects with some byte positions already known to be unreliable.
    ///
    /// An erasure is a byte whose *position* is suspect but whose value carries no information,
    /// so the decoder only has to solve for its magnitude, not its location. That halves its
    /// cost against the parity budget: where a plain error consumes two of the `ec_len` parity
    /// symbols, an erasure consumes one, and the decoder succeeds whenever
    /// `2 * errors + erasures <= ec_len`. Flagging the bytes a soft-decision front end
    /// distrusts therefore roughly doubles how much corruption a block survives — provided the
    /// flags are actually where the corruption is.
    ///
    /// `erased` is indexed by position in the full block, data then parity. Positions at or
    /// past `len` are ignored. With no erasures flagged this is exactly [`Self::rectify`].
    pub fn rectify_with_erasures(&mut self, erased: &[bool]) -> QRResult<&[u8]> {
        let deg = self.ec_len();
        let pos: Vec<usize> =
            (0..self.len).filter(|&i| erased.get(i).copied().unwrap_or(false)).collect();

        // More erasures than parity symbols is unsolvable even with no unknown errors.
        if pos.len() > deg {
            return Err(QRError::TooManyError);
        }
        if pos.is_empty() {
            return self.rectify();
        }

        let synd = match self.syndromes() {
            Ok(()) => return Ok(self.data()),
            Err(s) => s,
        };

        // Erasure locator, whose roots are the known positions.
        let lam = self.erasure_locator(&pos);

        // Modified syndromes T = S * Lambda mod x^deg. Their tail past the erasure count
        // behaves like the syndrome sequence of a code with `deg - e` parity symbols, so
        // Berlekamp-Massey run over it recovers just the *unknown* errors.
        let mut t = [G(0); MAX_EC_SIZE];
        for k in 0..deg {
            let mut acc = G(0);
            for j in 0..=k {
                acc += lam[j] * synd[k - j];
            }
            t[k] = acc;
        }
        let e = pos.len();
        let sig_err = self.berlkamp_massey(&t[e..], deg - e)?;

        // The combined locator vanishes at both the erasures and the located errors, so the
        // magnitude evaluation below is the ordinary Forney step over their union.
        let psi = poly_mul(&lam, &sig_err);
        self.correct(&synd, &psi)
    }

    /// Locates the positions `psi` vanishes at, solves their magnitudes and applies them,
    /// re-checking the syndromes so an over-corrupted block reports failure rather than
    /// returning silently wrong data. Shared by both entry points.
    fn correct(&mut self, synd: &[G; MAX_EC_SIZE], psi: &[G; MAX_EC_SIZE]) -> QRResult<&[u8]> {
        let err_loc = self.chien_search(psi);

        // Formal derivative: in characteristic 2 the even-power terms differentiate away.
        let mut dpsi = [G(0); MAX_EC_SIZE];
        for i in (1..MAX_EC_SIZE).step_by(2) {
            dpsi[i - 1] = psi[i];
        }

        let omg = self.omega(synd, psi);
        let err_mag = self.forney(&omg, &dpsi, &err_loc)?;

        for (i, &g) in err_mag.iter().enumerate() {
            self.data[i] = (G(self.data[i]) + g).into();
        }

        match self.syndromes() {
            Ok(()) => Ok(self.data()),
            Err(_) => Err(QRError::TooManyError),
        }
    }

    /// `Lambda(x) = prod (1 - X_j x)` over the erased positions, where a position `p` in the
    /// block has locator `alpha^(len - 1 - p)` — the same indexing [`Self::chien_search`] and
    /// [`Self::forney`] use.
    fn erasure_locator(&self, pos: &[usize]) -> [G; MAX_EC_SIZE] {
        let mut lam = [G(0); MAX_EC_SIZE];
        lam[0] = G(1);
        for &p in pos {
            let x = G::gen_pow(self.len - 1 - p);
            for k in (1..MAX_EC_SIZE).rev() {
                lam[k] += x * lam[k - 1];
            }
        }
        lam
    }

    fn syndromes(&self) -> Result<(), [G; MAX_EC_SIZE]> {
        let ec_len = self.len - self.dlen;
        let mut synd = [G(0); MAX_EC_SIZE];

        let mut gdata = [G(0); MAX_BLOCK_SIZE];
        for (i, &b) in self.data.iter().take(self.len).enumerate() {
            gdata[i] = G(b);
        }
        for (i, e) in synd.iter_mut().take(ec_len).enumerate() {
            let eval = eval_poly(gdata.iter().take(self.len).rev(), G::gen_pow(i));
            *e += eval;
        }

        if synd.iter().all(|&s| s.0 == 0) {
            Ok(())
        } else {
            Err(synd)
        }
    }

    // Sigma polynomial. `count` is how many leading syndromes to consume, which is the full
    // parity length for an error-only decode and the reduced length once erasures have paid
    // for part of the budget.
    fn berlkamp_massey(&self, synd: &[G], count: usize) -> QRResult<[G; MAX_EC_SIZE]> {
        let mut l = 0usize;
        let mut m = 1usize;
        let mut b = G(1);
        let mut cx = [G(0); MAX_EC_SIZE];
        let mut bx = [G(0); MAX_EC_SIZE];
        let mut tx = [G(0); MAX_EC_SIZE];
        cx[0] = G(1);
        bx[0] = G(1);

        for n in 0..count {
            // Calculate discrepancy
            let mut d = synd[n];
            for i in 1..=l {
                d += cx[i] * synd[n - i];
            }

            if d.0 != 0 {
                // Temporary copy
                tx.copy_from_slice(&cx);

                let scale = d.div(b)?;

                for i in 0..MAX_EC_SIZE - m {
                    cx[i + m] += scale * bx[i];
                }

                if 2 * l <= n {
                    bx.copy_from_slice(&tx);
                    l = n + 1 - l;
                    b = d;
                    m = 1;
                } else {
                    m += 1;
                }
            } else {
                m += 1;
            }
        }
        Ok(cx)
    }

    // Error location polynomial
    fn chien_search(&self, sig: &[G; MAX_EC_SIZE]) -> [bool; MAX_BLOCK_SIZE] {
        // A combined error-and-erasure locator can reach degree `ec_len`, one more coefficient
        // than an error-only sigma needs; the extra term is zero in that case.
        let deg = (self.len - self.dlen + 1).min(MAX_EC_SIZE);
        let mut err_loc = [false; MAX_BLOCK_SIZE];
        for (i, e) in err_loc[..self.len].iter_mut().rev().enumerate() {
            *e = eval_poly(sig.iter().take(deg), G::gen_pow(255 - i)).0 == 0;
        }
        err_loc
    }

    // Error evaluator polynomial, `Omega = S1 * sigma` truncated, with `S1` the syndromes from
    // the first power on — the convention `forney` below is paired with.
    fn omega(&self, synd: &[G; MAX_EC_SIZE], sig: &[G; MAX_EC_SIZE]) -> [G; MAX_EC_SIZE] {
        let t = self.len - self.dlen - 1;
        let mut omg = [G(0); MAX_EC_SIZE];
        for k in 0..t {
            let mut acc = G(0);
            for j in 0..=k {
                acc += sig[j] * synd[k - j + 1];
            }
            omg[k] = acc;
        }
        omg
    }

    fn forney(
        &self,
        omg: &[G; MAX_EC_SIZE],
        dsig: &[G; MAX_EC_SIZE],
        err_loc: &[bool; MAX_BLOCK_SIZE],
    ) -> QRResult<[G; MAX_BLOCK_SIZE]> {
        let mut mag = [G(0); MAX_BLOCK_SIZE];
        for (i, &is_err) in err_loc.iter().take(self.len).rev().enumerate() {
            if !is_err {
                continue;
            }
            let xinv = G::gen_pow(255 - i);
            let omg_x = eval_poly(omg.iter(), xinv);
            let sig_x = eval_poly(dsig.iter(), xinv);
            mag[self.len - 1 - i] += omg_x.div(sig_x)?;
        }
        Ok(mag)
    }
}

/// Polynomial product, truncated to the coefficient budget the locators live in.
fn poly_mul(a: &[G; MAX_EC_SIZE], b: &[G; MAX_EC_SIZE]) -> [G; MAX_EC_SIZE] {
    let mut out = [G(0); MAX_EC_SIZE];
    for i in 0..MAX_EC_SIZE {
        if a[i].0 == 0 {
            continue;
        }
        for j in 0..MAX_EC_SIZE - i {
            if b[j].0 == 0 {
                continue;
            }
            out[i + j] += a[i] * b[j];
        }
    }
    out
}

fn eval_poly<'a>(poly: impl Iterator<Item = &'a G>, x: G) -> G {
    let mut res = G(0);
    let mut xpow = G(1);
    for &coeff in poly {
        res += coeff * xpow;
        xpow *= x;
    }
    res
}

#[cfg(test)]
mod ec_rectifier_tests {
    use super::Block;
    use test_case::test_case;

    #[test_case(&[32, 91, 11, 45, 89, 123, 77, 44, 56, 99, 202], &[32, 91, 11, 45, 89, 46, 77, 44, 56, 99, 202, 0, 0, 0, 0]; "test_rectfier_1")]
    #[test_case(&[32, 91, 11, 45, 89, 123, 77, 44, 56, 99, 202], &[32, 91, 11, 45, 89, 46, 77, 44, 56, 99, 249, 0, 0, 0, 0]; "test_rectfier_2")]
    fn test_rectifier(data: &[u8], bad: &[u8]) {
        let mut blk = Block::new(data, 15);
        blk.data[..11].copy_from_slice(&bad[..11]);
        let rect = blk.rectify().unwrap();
        assert_eq!(rect, data, "Rectified data and original data don't match: Rectified {rect:?}, Original data {data:?}");
    }

    #[test_case(&[32, 91, 11, 45, 89, 123, 77, 44, 56, 99, 202], &[138, 91, 161, 45, 243, 46, 231, 44, 146, 99, 202, 0, 0, 0, 0]; "test_rectifier_panic")]
    #[should_panic]
    fn test_rectifier_fail(data: &[u8], bad: &[u8]) {
        let mut blk = Block::new(data, 15);
        blk.data[..11].copy_from_slice(&bad[..11]);
        let _ = blk.rectify().unwrap();
    }
}

#[cfg(test)]
mod ec_erasure_tests {
    use super::Block;

    /// Builds a block, corrupts `positions`, and flags `flagged` as erasures.
    fn run(dlen: usize, ec_len: usize, corrupt: &[usize], flagged: &[usize]) -> bool {
        let data: Vec<u8> = (0..dlen).map(|i| (i * 7 + 13) as u8).collect();
        let len = dlen + ec_len;
        let mut blk = Block::new(&data, len);
        for &i in corrupt {
            blk.data[i] ^= 0xFF;
        }
        let mut erased = vec![false; len];
        for &i in flagged {
            erased[i] = true;
        }
        matches!(blk.rectify_with_erasures(&erased), Ok(d) if d == data.as_slice())
    }

    #[test]
    fn test_pure_erasures_up_to_full_parity() {
        // With every corrupt position flagged, the budget is one parity symbol per erasure, so
        // all `ec_len` of them must come back.
        let (dlen, ec_len) = (40usize, 16usize);
        let all: Vec<usize> = (0..ec_len).collect();
        assert!(run(dlen, ec_len, &all, &all), "{ec_len} flagged erasures should be correctable");
    }

    #[test]
    fn test_one_erasure_past_parity_fails() {
        let (dlen, ec_len) = (40usize, 16usize);
        let all: Vec<usize> = (0..ec_len + 1).collect();
        assert!(!run(dlen, ec_len, &all, &all), "more erasures than parity must not decode");
    }

    #[test]
    fn test_erasures_beat_unflagged_errors_two_to_one() {
        // `ec_len` corruptions is twice what an error-only decode can fix, so it must fail
        // unflagged and succeed once flagged. This is the whole point of erasure decoding.
        let (dlen, ec_len) = (40usize, 16usize);
        let corrupt: Vec<usize> = (0..ec_len).collect();
        let none: Vec<usize> = vec![];
        assert!(!run(dlen, ec_len, &corrupt, &none), "unflagged: should exceed error capacity");
        assert!(run(dlen, ec_len, &corrupt, &corrupt), "flagged: should be within erasure budget");
    }

    #[test]
    fn test_mixed_errors_and_erasures() {
        // 2 * errors + erasures <= ec_len. Here 4 unflagged errors and 8 erasures against 16.
        let (dlen, ec_len) = (60usize, 16usize);
        let corrupt: Vec<usize> = (0..12).collect();
        let flagged: Vec<usize> = (4..12).collect();
        assert!(run(dlen, ec_len, &corrupt, &flagged), "4 errors + 8 erasures within 16 parity");
    }

    #[test]
    fn test_flagging_clean_positions_is_harmless() {
        // Erasures that turn out to be correct only spend budget; they must not corrupt data.
        let (dlen, ec_len) = (40usize, 16usize);
        let corrupt: Vec<usize> = vec![0, 5, 9];
        let flagged: Vec<usize> = vec![0, 5, 9, 20, 21, 22, 30];
        assert!(run(dlen, ec_len, &corrupt, &flagged), "over-flagging should still decode");
    }

    #[test]
    fn test_no_erasures_matches_plain_rectify() {
        let (dlen, ec_len) = (40usize, 16usize);
        let corrupt: Vec<usize> = vec![3, 17, 29];
        assert!(run(dlen, ec_len, &corrupt, &[]), "empty erasure set falls back to rectify");
    }
}

// Rectifier for format and version infos
pub fn rectify_info(info: u32, valid_numbers: &[u32], err_capacity: u32) -> QRResult<(u32, u32)> {
    let res = *valid_numbers.iter().min_by_key(|&n| (info ^ n).count_ones()).unwrap();
    let err = (info ^ res).count_ones();

    if err <= err_capacity {
        Ok((res, err))
    } else {
        Err(QRError::InvalidInfo)
    }
}
