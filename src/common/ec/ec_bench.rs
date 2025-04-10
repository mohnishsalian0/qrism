use std::{error::Error, fmt::Debug};

use g2p::{g2p, GaloisField};

use super::version_db::RSParameters;

g2p!(GF16, 4, modulus: 0b1_0011);
g2p!(GF256, 8, modulus: 0b1_0001_1101);

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub enum DeQRError {
    /// Could not write the output to the output stream/string
    IoError,
    /// Expected more bits to decode
    DataUnderflow,
    /// Expected less bits to decode
    DataOverflow,
    /// Unknown data type in encoding
    UnknownDataType,
    /// Could not correct errors / code corrupt
    DataEcc,
    /// Could not read format information from both locations
    FormatEcc,
    /// Unsupported / non-existent version read
    InvalidVersion,
    /// Unsupported / non-existent grid size read
    InvalidGridSize,
    /// Output was not encoded in expected UTF8
    EncodingError,
}

type DeQRResult<T> = Result<T, DeQRError>;

impl Error for DeQRError {}

impl From<::std::string::FromUtf8Error> for DeQRError {
    fn from(_: ::std::string::FromUtf8Error) -> Self {
        DeQRError::EncodingError
    }
}

impl ::std::fmt::Display for DeQRError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let msg = match self {
            DeQRError::IoError => "IoError(Could not write to output)",
            DeQRError::DataUnderflow => "DataUnderflow(Expected more bits to decode)",
            DeQRError::DataOverflow => "DataOverflow(Expected less bits to decode)",
            DeQRError::UnknownDataType => "UnknownDataType(DataType not known or not implemented)",
            DeQRError::DataEcc => "Ecc(Too many errors to correct)",
            DeQRError::FormatEcc => "Ecc(Version information corrupt)",
            DeQRError::InvalidVersion => "InvalidVersion(Invalid version or corrupt)",
            DeQRError::InvalidGridSize => "InvalidGridSize(Invalid version or corrupt)",
            DeQRError::EncodingError => "Encoding(Not UTF8)",
        };
        write!(f, "{}", msg)
    }
}

fn correct_block(block: &mut [u8], ecc: &RSParameters) -> DeQRResult<()> {
    assert!(ecc.bs > ecc.dw);

    let npar = ecc.bs - ecc.dw;
    let mut sigma_deriv = [GF256::ZERO; 64];

    // Calculate syndromes. If all 0 there is nothing to do.
    let s = match block_syndromes(&block[..ecc.bs], npar) {
        Ok(_) => return Ok(()),
        Err(s) => s,
    };

    let sigma = berlekamp_massey(&s, npar);
    /* Compute derivative of sigma */
    for i in (1..64).step_by(2) {
        sigma_deriv[i - 1] = sigma[i];
    }

    /* Compute error evaluator polynomial */
    let omega = eloc_poly(&s, &sigma, npar - 1);

    /* Find error locations and magnitudes */
    for i in 0..ecc.bs {
        let xinv = GF256::GENERATOR.pow(255 - i);
        if poly_eval(&sigma, xinv) == GF256::ZERO {
            let sd_x = poly_eval(&sigma_deriv, xinv);
            let omega_x = poly_eval(&omega, xinv);
            if sd_x == GF256::ZERO {
                return Err(DeQRError::DataEcc);
            }
            let error = omega_x / sd_x;
            block[ecc.bs - i - 1] = (GF256(block[ecc.bs - i - 1]) + error).0;
        }
    }

    match block_syndromes(&block[..ecc.bs], npar) {
        Ok(_) => Ok(()),
        Err(_) => Err(DeQRError::DataEcc),
    }
}
/* ***********************************************************************
 * Code stream error correction
 *
 * Generator polynomial for GF(2^8) is x^8 + x^4 + x^3 + x^2 + 1
 */
fn block_syndromes(block: &[u8], npar: usize) -> Result<[GF256; 64], [GF256; 64]> {
    let mut nonzero: bool = false;
    let mut s = [GF256::ZERO; 64];

    #[allow(clippy::needless_range_loop)]
    for i in 0..npar {
        for j in 0..block.len() {
            let c = GF256(block[block.len() - 1 - j]);
            s[i] += c * GF256::GENERATOR.pow(i * j);
        }
        if s[i] != GF256::ZERO {
            nonzero = true;
        }
    }
    if nonzero {
        Err(s)
    } else {
        Ok(s)
    }
}

fn poly_eval<G>(s: &[G; 64], x: G) -> G
where
    G: GaloisField + Debug,
{
    let mut sum = G::ZERO;
    let mut x_pow = G::ONE;

    #[allow(clippy::needless_range_loop)]
    for i in 0..64 {
        sum += s[i] * x_pow;
        x_pow *= x;
    }
    sum
}

fn eloc_poly(s: &[GF256; 64], sigma: &[GF256; 64], npar: usize) -> [GF256; 64] {
    let mut omega = [GF256::ZERO; 64];
    for i in 0..npar {
        let a = sigma[i];
        for j in 0..(npar - i) {
            let b = s[j + 1];
            omega[i + j] += a * b;
        }
    }
    omega
}
/* ***********************************************************************
 * Berlekamp-Massey algorithm for finding error locator polynomials.
 */
fn berlekamp_massey<G>(s: &[G; 64], n: usize) -> [G; 64]
where
    G: GaloisField,
{
    let mut ts: [G; 64] = [G::ZERO; 64];
    let mut cs: [G; 64] = [G::ZERO; 64];
    let mut bs: [G; 64] = [G::ZERO; 64];
    let mut l: usize = 0;
    let mut m: usize = 1;
    let mut b = G::ONE;
    bs[0] = G::ONE;
    cs[0] = G::ONE;

    for n in 0..n {
        let mut d = s[n];

        // Calculate in GF(p):
        // d = s[n] + \Sum_{i=1}^{l} c[i] * s[n - i]
        for i in 1..=l {
            d += cs[i] * s[n - i];
        }
        // Pre-calculate d * b^-1 in GF(p)
        let mult = d / b;

        if d == G::ZERO {
            m += 1
        } else if l * 2 <= n {
            ts.copy_from_slice(&cs);
            poly_add(&mut cs, &bs, mult, m);
            bs.copy_from_slice(&ts);
            l = n + 1 - l;
            b = d;
            m = 1
        } else {
            poly_add(&mut cs, &bs, mult, m);
            m += 1
        }
    }
    cs
}
/* ***********************************************************************
 * Polynomial operations
 */
fn poly_add<G>(dst: &mut [G; 64], src: &[G; 64], c: G, shift: usize)
where
    G: GaloisField,
{
    if c == G::ZERO {
        return;
    }

    #[allow(clippy::needless_range_loop)]
    for i in 0..64 {
        let p = i + shift;
        if p >= 64 {
            break;
        }
        let v = src[i];
        dst[p] += v * c;
    }
}

// #[cfg(test)]
// mod ec_rectifier_correct_block_tests {
//     use test_case::test_case;
//
//     use crate::common::version_db::RSParameters;
//
//     use super::correct_block;
//
//     #[test_case(&mut [32, 91, 11, 45, 89, 123, 77, 44, 56, 99, 202], &mut [32, 91, 11, 45, 89, 46, 77, 44, 56, 99, 202, 21, 197, 229, 186])]
//     #[test_case(&mut [32, 91, 11, 45, 89, 123, 77, 44, 56, 99, 202], &mut [32, 91, 11, 45, 89, 46, 77, 44, 56, 99, 249, 21, 197, 229, 186])]
//     fn test_ec_correct_block(data: &mut [u8], bad: &mut [u8]) {
//         correct_block(bad, &RSParameters { bs: 15, dw: 11, ns: 1 }).unwrap();
//         assert_eq!(data, &bad[..11]);
//     }
//
//     #[should_panic]
//     #[test_case(&mut [32, 91, 11, 45, 89, 123, 77, 44, 56, 99, 202], &mut [138, 91, 161, 45, 243, 46, 231, 44, 146, 99, 202, 0, 0, 0, 0])]
//     fn test_ec_correct_block_panic(data: &mut [u8], bad: &mut [u8]) {
//         correct_block(bad, &RSParameters { bs: 15, dw: 11, ns: 1 }).unwrap();
//     }
// }
