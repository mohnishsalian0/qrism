use std::ops::Deref;

use crate::{
    encode::{encode, encode_with_version},
    error_correction::{ecc, error_correction_capacity},
    mask::apply_best_mask,
    qr::QR,
    types::{ECLevel, Palette, QRError, QRResult, Version},
};

pub struct QRBuilder<'a> {
    data: &'a [u8],
    version: Option<Version>,
    ec_level: ECLevel,
    palette: Palette,
}

impl<'a> QRBuilder<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self { data, version: None, ec_level: ECLevel::M, palette: Palette::Monochrome }
    }

    pub fn new_with_version(
        data: &'a [u8],
        version: Version,
        ec_level: ECLevel,
        palette: Palette,
    ) -> Self {
        Self { data, version: Some(version), ec_level, palette }
    }

    pub fn data(&mut self, data: &'a [u8]) -> &mut Self {
        self.data = data;
        self
    }

    pub fn version(&mut self, version: Version) -> &mut Self {
        self.version = Some(version);
        self
    }

    pub fn unset_version(&mut self) -> &mut Self {
        self.version = None;
        self
    }

    pub fn ec_level(&mut self, ec_level: ECLevel) -> &mut Self {
        self.ec_level = ec_level;
        self
    }

    pub fn palette(&mut self, palette: Palette) -> &mut Self {
        self.palette = palette;
        self
    }

    pub fn get_metadata(&self) -> String {
        match self.version {
            Some(v) => format!(
                "Metadata {{ Version: {:?}, Ec level: {:?}, Palette: {:?} }}",
                *v, self.ec_level, self.palette
            ),
            None => format!(
                "Metadata {{ Version: None, Ec level: {:?}, Palette: {:?} }}",
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
        let data = "Hello, world!".as_bytes();
        let version = Version::Normal(1);
        let ec_level = ECLevel::L;
        let palette = Palette::Monochrome;
        let mut qr_builder = QRBuilder::new_with_version(data, version, ec_level, palette);
        assert_eq!(
            qr_builder.get_metadata(),
            "Metadata { Version: 1, Ec level: L, Palette: Monochrome }"
        );
        qr_builder.unset_version();
        assert_eq!(
            qr_builder.get_metadata(),
            "Metadata { Version: None, Ec level: L, Palette: Monochrome }"
        );
    }
}

impl<'a> QRBuilder<'a> {
    pub fn build(&self) -> QRResult<QR> {
        let data_len = self.data.len();

        println!("\nGenerating QR with {}...", self.get_metadata());
        if self.data.is_empty() {
            return Err(QRError::EmptyData);
        }

        // Encode data optimally
        println!("Encoding data...");
        let (encoded_blob, encoded_len) = match self.version {
            Some(v) => encode_with_version(self.data, self.ec_level, v)?,
            None => encode(self.data, self.ec_level)?,
        };

        let version = encoded_blob.version();
        let version_capacity = version.bit_capacity(self.ec_level) / 8;
        let err_corr_cap = error_correction_capacity(version, self.ec_level);
        let encoded_data = encoded_blob.data();

        // Compute error correction codewords
        println!("Computing ecc...");
        let (data_blocks, ecc_blocks) = ecc(encoded_data, version, self.ec_level);

        // Interleave data and error correction codewords
        println!("Interleaving and chaining data & ecc...");
        let mut payload = Self::interleave(&data_blocks);
        payload.extend(Self::interleave(&ecc_blocks));

        // Construct QR
        println!("Constructing QR...");
        let mut qr = QR::new(version, self.ec_level, self.palette);

        println!("Drawing functional patterns...");
        qr.draw_all_function_patterns();

        println!("Drawing encoding region...");
        qr.draw_encoding_region(&payload);

        println!("Finding & applying best mask...");
        let best_mask = apply_best_mask(&mut qr);

        println!("\x1b[1;32mQR generated successfully!\n \x1b[0m");

        let total_modules = version.width() * version.width();
        let dark_modules = qr.count_dark_modules();
        let light_modules = total_modules - dark_modules;

        println!("Report:");
        println!(
            "Version: {version:?}, EC Level: {:?}, Palette: {:?}, Masking pattern: {}",
            self.ec_level, self.palette, *best_mask
        );
        println!("Data capacity: {}, Error Capacity: {}", version_capacity, err_corr_cap);
        println!(
            "Data size: {}, Encoded size: {}, Compression: {}%",
            data_len,
            encoded_len,
            encoded_len * 100 / data_len
        );
        println!(
            "Dark Cells: {}, Light Cells: {}, Balance: {}\n",
            dark_modules,
            light_modules,
            dark_modules * 100 / total_modules
        );

        Ok(qr)
    }

    fn interleave<T: Copy, V: Deref<Target = [T]>>(blocks: &[V]) -> Vec<T> {
        let max_block_size = blocks.iter().map(|b| b.len()).max().expect("Blocks is empty");
        let total_size = blocks.iter().map(|b| b.len()).sum::<usize>();
        let mut res = Vec::with_capacity(total_size);
        for i in 0..max_block_size {
            for b in blocks {
                if i < b.len() {
                    res.push(b[i]);
                }
            }
        }
        res
    }
}
