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

    /// Locates the positions `psi` vanishes at, solves their magnitudes and applies them,
    /// re-checking the syndromes so an over-corrupted block reports failure rather than
    /// returning silently wrong data.
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

    // Sigma polynomial. `count` is how many leading syndromes to consume.
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
        let deg = self.len - self.dlen;
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
