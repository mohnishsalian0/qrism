use super::{galois::*, Block, MAX_BLOCK_SIZE, MAX_EC_SIZE};
use crate::utils::{QRError, QRResult};

// Rectifier
//------------------------------------------------------------------------------

impl Block {
    pub fn rectify(&mut self) -> QRResult<&[u8]> {
        // Compute syndromes
        let synd = match self.syndromes() {
            Ok(()) => return Ok(self.data()),
            Err(s) => s,
        };

        // Error locator polynomial
        let sig = self.berlkamp_massey(&synd);
        let err_loc = self.chien_search(&sig);

        // Sigma derivative
        let mut dsig = [G(0); MAX_EC_SIZE];
        for i in (1..MAX_EC_SIZE).step_by(2) {
            dsig[i - 1] = sig[i];
        }

        // Error evaluator
        let omg = self.omega(&synd, &sig);

        // Error magnitude
        let err_mag = self.forney(&omg, &dsig, &err_loc);

        // Rectify errors by XORing data with magnitude
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

    // Sigma polynomial
    fn berlkamp_massey(&self, synd: &[G]) -> [G; MAX_EC_SIZE] {
        let mut l = 0usize;
        let mut m = 1usize;
        let mut b = G(1);
        let mut cx = [G(0); MAX_EC_SIZE];
        let mut bx = [G(0); MAX_EC_SIZE];
        let mut tx = [G(0); MAX_EC_SIZE];
        cx[0] = G(1);
        bx[0] = G(1);
        let deg = self.len - self.dlen;

        for n in 0..deg {
            // Calculate discrepancy
            let mut d = synd[n];
            for i in 1..=l {
                d += cx[i] * synd[n - i];
            }

            if d.0 != 0 {
                // Temporary copy
                tx.copy_from_slice(&cx);

                let scale = d / b;

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
        cx
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

    // Error evaluator polynomial
    fn omega(&self, synd: &[G; MAX_EC_SIZE], sig: &[G; MAX_EC_SIZE]) -> [G; MAX_EC_SIZE] {
        let t = self.len - self.dlen - 1;
        let mut omg = [G(0); MAX_EC_SIZE];
        for i in 0..t {
            let sy = synd[i + 1];
            for j in 0..t - i {
                let si = sig[j];
                omg[i + j] += sy * si;
            }
        }
        omg
    }

    fn forney(
        &self,
        omg: &[G; MAX_EC_SIZE],
        dsig: &[G; MAX_EC_SIZE],
        err_loc: &[bool; MAX_BLOCK_SIZE],
    ) -> [G; MAX_BLOCK_SIZE] {
        let mut mag = [G(0); MAX_BLOCK_SIZE];
        for (i, &is_err) in err_loc.iter().take(self.len).rev().enumerate() {
            if !is_err {
                continue;
            }
            let xinv = G::gen_pow(255 - i);
            let omg_x = eval_poly(omg.iter(), xinv);
            let sig_x = eval_poly(dsig.iter(), xinv);
            mag[self.len - 1 - i] += omg_x / sig_x;
        }
        mag
    }
}

fn eval_poly<'a>(poly: impl Iterator<Item = &'a G>, x: G) -> G {
    let mut res = G(0);
    let mut xpow = G(1);
    for (j, &coeff) in poly.enumerate() {
        res += coeff * xpow;
        xpow *= x;
    }
    res
}

#[cfg(test)]
mod ec_rectifier_tests {
    use super::Block;
    use test_case::test_case;

    #[test_case(&[32, 91, 11, 45, 89, 123, 77, 44, 56, 99, 202], &[32, 91, 11, 45, 89, 46, 77, 44, 56, 99, 202, 0, 0, 0, 0])]
    #[test_case(&[32, 91, 11, 45, 89, 123, 77, 44, 56, 99, 202], &[32, 91, 11, 45, 89, 46, 77, 44, 56, 99, 249, 0, 0, 0, 0])]
    fn test_rectifier(data: &[u8], bad: &[u8]) {
        let mut blk = Block::new(data, 15);
        println!("Ecc {:?}", blk.ecc());
        blk.data[..11].copy_from_slice(&bad[..11]);
        let rect = blk.rectify().unwrap();
        assert_eq!(rect, data, "Rectified data and original data don't match: Rectified {rect:?}, Original data {data:?}");
    }

    #[test_case(&[32, 91, 11, 45, 89, 123, 77, 44, 56, 99, 202], &[138, 91, 161, 45, 243, 46, 231, 44, 146, 99, 202, 0, 0, 0, 0])]
    #[should_panic]
    fn test_rectifier_fail(data: &[u8], bad: &[u8]) {
        let mut blk = Block::new(data, 15);
        blk.data[..11].copy_from_slice(&bad[..11]);
        let _ = blk.rectify().unwrap();
    }
}

// Rectifier for format and version infos
pub fn rectify_info(info: u32, valid_numbers: &[u32], err_capacity: u32) -> QRResult<u32> {
    let res = *valid_numbers.iter().min_by_key(|&n| (info ^ n).count_ones()).unwrap();

    if (info ^ res).count_ones() <= err_capacity {
        Ok(res)
    } else {
        Err(QRError::InvalidInfo)
    }
}
