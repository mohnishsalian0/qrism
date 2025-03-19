use super::Version;

// Iterator for placing data in encoding region of QR
//------------------------------------------------------------------------------

pub struct EncRegionIter {
    r: i16,
    c: i16,
    width: i16,
    vert_timing_col: i16,
}

impl EncRegionIter {
    pub const fn new(version: Version) -> Self {
        let w = version.width() as i16;
        let vert_timing_col = match version {
            Version::Micro(_) => 0,
            Version::Normal(_) => 6,
        };
        Self { r: w - 1, c: w - 1, width: w, vert_timing_col }
    }
}

impl Iterator for EncRegionIter {
    type Item = (i16, i16);
    fn next(&mut self) -> Option<Self::Item> {
        let adjusted_col = if self.c <= self.vert_timing_col { self.c + 1 } else { self.c };
        if self.c < 0 {
            return None;
        }
        let res = (self.r, self.c);
        let col_type = (self.width - adjusted_col) % 4;
        match col_type {
            2 if self.r > 0 => {
                self.r -= 1;
                self.c += 1;
            }
            0 if self.r < self.width - 1 => {
                self.r += 1;
                self.c += 1;
            }
            0 | 2 if self.c == self.vert_timing_col + 1 => {
                self.c -= 2;
            }
            _ => {
                self.c -= 1;
            }
        }
        Some(res)
    }
}

#[cfg(test)]
mod iter_tests {
    use super::EncRegionIter;
    use crate::builder::{Module, QRBuilder};
    use crate::common::metadata::{ECLevel, Version};

    #[test]
    fn test_enc_region_iter() {
        for v in 1..40 {
            let data = "Hello, world!".as_bytes();
            let version = Version::Normal(v);
            let ec_level = ECLevel::L;
            let qr = QRBuilder::new(data).version(version).ec_level(ec_level).build().unwrap();
            let coords = EncRegionIter::new(version);
            let total_codewords = coords
                .into_iter()
                .filter(|(r, c)| matches!(qr.get(*r, *c), Module::Data(_)))
                .count()
                / 8;
            let exp_codewords = version.total_codewords();
            assert_eq!(total_codewords, exp_codewords);
        }
    }
}
