use std::ops::Deref;

use crate::{
    encode::{encode, encode_with_version},
    error_correction::ecc,
    mask::apply_best_mask,
    qr::QR,
    types::{ECLevel, Palette, QRResult, Version},
};

pub struct QRBuilder {
    version: Option<Version>,
    ec_level: ECLevel,
    palette: Palette,
}

impl QRBuilder {
    pub fn new() -> Self {
        Self { version: None, ec_level: ECLevel::L, palette: Palette::Monochrome }
    }

    pub fn new_with_version(version: Version, ec_level: ECLevel, palette: Palette) -> Self {
        Self { version: Some(version), ec_level, palette }
    }

    pub fn version(&self) -> Option<Version> {
        self.version
    }

    pub fn ec_level(&self) -> ECLevel {
        self.ec_level
    }

    pub fn palette(&self) -> Palette {
        self.palette
    }

    pub fn set_version(&mut self, version: Version) {
        self.version = Some(version);
    }

    pub fn unset_version(&mut self) {
        self.version = None;
    }

    pub fn set_ec_level(&mut self, ec_level: ECLevel) {
        self.ec_level = ec_level;
    }

    pub fn set_palette(&mut self, palette: Palette) {
        self.palette = palette;
    }

    pub fn metadata(&self) -> String {
        match self.version {
            Some(v) => format!(
                "Metadata: Version: {:?}, Ec level: {:?}, Palette: {:?}",
                *v, self.ec_level, self.palette
            ),
            None => format!(
                "Metadata: Version: None, Ec level: {:?}, Palette: {:?}",
                self.ec_level, self.palette
            ),
        }
    }
}

#[cfg(test)]
mod qrbuilder_util_tests {
    use super::QRBuilder;
    use crate::types::{ECLevel, Palette, Version};

    #[test]
    fn test_metadata() {
        let version = Version::Normal(1);
        let ec_level = ECLevel::L;
        let palette = Palette::Monochrome;
        let mut qr_builder = QRBuilder::new_with_version(version, ec_level, palette);
        assert_eq!(qr_builder.metadata(), "Metadata: Version: 1, Ec level: L, Palette: Monochrome");
        qr_builder.unset_version();
        assert_eq!(
            qr_builder.metadata(),
            "Metadata: Version: None, Ec level: L, Palette: Monochrome"
        );
    }
}

impl QRBuilder {
    pub fn build(&self, data: &[u8]) -> QRResult<QR> {
        // Encode data optimally
        let encoded_blob = match self.version {
            Some(v) => encode_with_version(data, self.ec_level, v)?,
            None => encode(data, self.ec_level)?,
        };

        let version = encoded_blob.version();
        let encoded_data = encoded_blob.data();

        // Compute error correction codewords
        let (data_blocks, ecc_blocks) = ecc(encoded_data, version, self.ec_level);

        // Interleave data and error correction codewords
        let interleaved_data = Self::interleave(&data_blocks);
        let interleaved_ecc = Self::interleave(&ecc_blocks);

        // Construct QR
        let mut qr = QR::new(version, self.ec_level, self.palette);
        qr.draw_all_function_patterns();
        qr.draw_encoding_region(&interleaved_data, &interleaved_ecc);
        apply_best_mask(&mut qr);

        Ok(qr)
    }

    fn interleave<T: Copy, V: Deref<Target = [T]>>(blocks: &[V]) -> Vec<T> {
        let max_block_size = blocks.iter().map(|b| b.len()).max().expect("Blocks is empty");
        let total_size = blocks.iter().map(|b| b.len()).sum::<usize>();
        let mut res = Vec::with_capacity(total_size);
        for i in 0..max_block_size {
            for b in blocks {
                res.push(b[i]);
            }
        }
        res
    }
}
