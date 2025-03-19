use crate::common::{
    ec::*,
    metadata::{ECLevel, Version},
};

// ECC: Error Correction Codeword generator
pub fn ecc(data: &[u8], version: Version, ec_level: ECLevel) -> (Vec<&[u8]>, Vec<Vec<u8>>) {
    let data_blocks = blockify(data, version, ec_level);

    let ecc_size_per_block = version.ecc_per_block(ec_level);
    let ecc_blocks =
        data_blocks.iter().map(|b| ecc_per_block(b, ecc_size_per_block)).collect::<Vec<_>>();

    (data_blocks, ecc_blocks)
}

pub fn blockify(data: &[u8], version: Version, ec_level: ECLevel) -> Vec<&[u8]> {
    let (block1_size, block1_count, block2_size, block2_count) =
        version.data_codewords_per_block(ec_level);

    let total_blocks = block1_count + block2_count;
    let total_block1_size = block1_size * block1_count;
    let total_size = total_block1_size + block2_size * block2_count;

    debug_assert!(
        total_size == data.len(),
        "Data len doesn't match total size of blocks: Data len {}, Total block size {}",
        data.len(),
        total_size
    );

    let mut data_blocks = Vec::with_capacity(total_blocks);
    data_blocks.extend(data[..total_block1_size].chunks(block1_size));
    if block2_size > 0 {
        data_blocks.extend(data[total_block1_size..].chunks(block2_size));
    }
    data_blocks
}

// Performs polynomial long division with data polynomial(num)
// and generator polynomial(den) to compute remainder polynomial,
// the coefficients of which are the ecc
fn ecc_per_block(block: &[u8], ecc_count: usize) -> Vec<u8> {
    let len = block.len();
    let gen_poly = GENERATOR_POLYNOMIALS[ecc_count];

    let mut res = block.to_vec();
    res.resize(len + ecc_count, 0);

    for i in 0..len {
        let lead_coeff = res[i] as usize;
        if lead_coeff == 0 {
            continue;
        }

        let log_lead_coeff = LOG_TABLE[lead_coeff] as usize;
        for (u, v) in res[i + 1..].iter_mut().zip(gen_poly.iter()) {
            let mut log_sum = *v as usize + log_lead_coeff;
            debug_assert!(log_sum < 510, "Log sum has crossed 510: {log_sum}");
            if log_sum >= 255 {
                log_sum -= 255;
            }
            *u ^= EXP_TABLE[log_sum];
        }
    }

    res.split_off(len)
}

pub fn error_correction_capacity(version: Version, ec_level: ECLevel) -> usize {
    let p = match (version, ec_level) {
        (Version::Micro(2) | Version::Normal(1), ECLevel::L) => 3,
        (Version::Micro(_) | Version::Normal(2), ECLevel::L)
        | (Version::Micro(2) | Version::Normal(1), ECLevel::M) => 2,
        (Version::Normal(1), _) | (Version::Normal(3), ECLevel::L) => 1,
        _ => 0,
    };

    let ec_bytes_per_block = version.ecc_per_block(ec_level);
    let (_, count1, _, count2) = version.data_codewords_per_block(ec_level);
    let ec_bytes = (count1 + count2) * ec_bytes_per_block;

    (ec_bytes - p) / 2
}

#[cfg(test)]
mod ec_tests {

    use crate::{
        builder::ec::{ecc, ecc_per_block},
        common::metadata::{ECLevel, Version},
    };

    #[test]
    fn test_poly_mod_1() {
        let res = ecc_per_block(b" [\x0bx\xd1r\xdcMC@\xec\x11\xec\x11\xec\x11", 10);
        assert_eq!(&*res, b"\xc4#'w\xeb\xd7\xe7\xe2]\x17");
    }

    #[test]
    fn test_poly_mod_2() {
        let res = ecc_per_block(b" [\x0bx\xd1r\xdcMC@\xec\x11\xec", 13);
        assert_eq!(&*res, b"\xa8H\x16R\xd96\x9c\x00.\x0f\xb4z\x10");
    }

    #[test]
    fn test_poly_mod_3() {
        let res = ecc_per_block(b"CUF\x86W&U\xc2w2\x06\x12\x06g&", 18);
        assert_eq!(&*res, b"\xd5\xc7\x0b-s\xf7\xf1\xdf\xe5\xf8\x9au\x9aoV\xa1o'");
    }

    // TODO: assert data blocks as well
    #[test]
    fn test_add_ec_simple() {
        let msg = b" [\x0bx\xd1r\xdcMC@\xec\x11\xec\x11\xec\x11";
        let expected_ecc = [b"\xc4\x23\x27\x77\xeb\xd7\xe7\xe2\x5d\x17"];
        let (_, ecc) = ecc(msg, Version::Normal(1), ECLevel::M);
        assert_eq!(&*ecc, expected_ecc);
    }

    #[test]
    fn test_add_ec_complex() {
        let msg = b"CUF\x86W&U\xc2w2\x06\x12\x06g&\xf6\xf6B\x07v\x86\xf2\x07&V\x16\xc6\xc7\x92\x06\
                    \xb6\xe6\xf7w2\x07v\x86W&R\x06\x86\x972\x07F\xf7vV\xc2\x06\x972\x10\xec\x11\xec\
                    \x11\xec\x11\xec";
        let expected_ec = [
            b"\xd5\xc7\x0b\x2d\x73\xf7\xf1\xdf\xe5\xf8\x9a\x75\x9a\x6f\x56\xa1\x6f\x27",
            b"\x57\xcc\x60\x3c\xca\xb6\x7c\x9d\xc8\x86\x1b\x81\xd1\x11\xa3\xa3\x78\x85",
            b"\x94\x74\xb1\xd4\x4c\x85\x4b\xf2\xee\x4c\xc3\xe6\xbd\x0a\x6c\xf0\xc0\x8d",
            b"\xeb\x9f\x05\xad\x18\x93\x3b\x21\x6a\x28\xff\xac\x52\x02\x83\x20\xb2\xec",
        ];
        let (_, ecc) = ecc(msg, Version::Normal(5), ECLevel::Q);
        assert_eq!(&*ecc, &expected_ec[..]);
    }
}
