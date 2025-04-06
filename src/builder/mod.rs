mod qr;

pub(crate) use qr::{Module, QR};

use std::ops::Deref;

use crate::common::{
    codec::{encode, encode_with_version},
    ec::ecc,
    error::{QRError, QRResult},
    mask::{apply_best_mask, MaskPattern},
    metadata::{ECLevel, Palette, Version},
    BitStream,
};

pub struct QRBuilder<'a> {
    data: &'a [u8],
    version: Option<Version>,
    ec_level: ECLevel,
    palette: Palette,
    mask: Option<MaskPattern>,
}

impl<'a> QRBuilder<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self { data, version: None, ec_level: ECLevel::M, palette: Palette::Mono, mask: None }
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

    pub fn mask(&mut self, mask: MaskPattern) -> &mut Self {
        self.mask = Some(mask);
        self
    }

    pub fn metadata(&self) -> String {
        match self.version {
            Some(v) => format!(
                "{{ Version: {:?}, Ec level: {:?}, Palette: {:?} }}",
                *v, self.ec_level, self.palette
            ),
            None => format!(
                "{{ Version: None, Ec level: {:?}, Palette: {:?} }}",
                self.ec_level, self.palette
            ),
        }
    }
}

#[cfg(test)]
mod qrbuilder_util_tests {
    use super::QRBuilder;
    use crate::common::{ECLevel, Palette, Version};

    #[test]
    fn test_metadata() {
        let data = "Hello, world!".as_bytes();
        let version = Version::Normal(1);
        let ec_level = ECLevel::L;
        let palette = Palette::Mono;
        let mut qr_builder = QRBuilder::new(data);
        qr_builder.version(version).ec_level(ec_level).palette(palette);
        assert_eq!(qr_builder.metadata(), "{ Version: 1, Ec level: L, Palette: Mono }");
        qr_builder.unset_version();
        assert_eq!(qr_builder.metadata(), "{ Version: None, Ec level: L, Palette: Mono }");
    }
}

impl QRBuilder<'_> {
    pub fn build(&self) -> QRResult<QR> {
        let data_len = self.data.len();

        println!("\nGenerating QR {}...", self.metadata());
        if self.data.is_empty() {
            return Err(QRError::EmptyData);
        }

        // Encode data optimally
        println!("Encoding data...");
        let (encoded_data, version) = match self.version {
            Some(v) => (encode_with_version(self.data, self.ec_level, v, self.palette)?, v),
            None => {
                println!("Finding best version...");
                encode(self.data, self.ec_level, self.palette)?
            }
        };

        let total_codewords = version.total_codewords(self.palette);
        let data_len = version.data_bit_capacity(self.ec_level, self.palette) >> 3;
        let ec_capacity = Self::ec_capacity(version, self.ec_level);

        println!("Constructing payload with ecc & interleaving...");
        let mut payload = BitStream::new(total_codewords << 3);
        let channel_data_capacity = version.channel_data_capacity(self.ec_level);
        debug_assert!(
            encoded_data.len() % channel_data_capacity == 0,
            "Encoded data length {} is not divisible by channel_codewords {channel_data_capacity}",
            encoded_data.len()
        );
        encoded_data.data().chunks_exact(channel_data_capacity).for_each(|c| {
            // Compute error correction codewords
            let (data_blocks, ecc_blocks) = Self::compute_ecc(c, version, self.ec_level);

            // Interleave data & error correction codewords, and store in payload
            payload.extend(&Self::interleave(&data_blocks));
            payload.extend(&Self::interleave(&ecc_blocks));
        });

        // Construct QR
        println!("Constructing QR...");
        let mut qr = QR::new(version, self.ec_level, self.palette);

        println!("Drawing functional patterns...");
        qr.draw_all_function_patterns();

        println!("Drawing encoding region...");
        qr.draw_encoding_region(payload);

        let mask = match self.mask {
            Some(m) => {
                println!("Apply mask {m:?}...");
                qr.mask(m);
                m
            }
            None => {
                println!("Finding & applying best mask...");
                apply_best_mask(&mut qr)
            }
        };

        println!("\x1b[1;32mQR generated successfully!\n \x1b[0m");

        let total_modules = version.width() * version.width();
        let dark_modules = qr.count_dark_modules();
        let light_modules = total_modules - dark_modules;

        println!("Report:");
        println!("{}", qr.metadata());
        println!("Data capacity: {}, Error Capacity: {}", data_len, ec_capacity);
        println!(
            "Data size: {}, Encoded size: {}, Compression: {}%",
            data_len,
            encoded_data.len() >> 3,
            (encoded_data.len() >> 3) * 100 / data_len
        );
        println!(
            "Dark Cells: {}, Light Cells: {}, Balance: {}\n",
            dark_modules,
            light_modules,
            dark_modules * 100 / total_modules
        );

        Ok(qr)
    }

    // ECC: Error Correction Codeword generator
    fn compute_ecc(data: &[u8], version: Version, ec_level: ECLevel) -> (Vec<&[u8]>, Vec<Vec<u8>>) {
        let data_blocks = Self::blockify(data, version, ec_level);

        let ecc_size_per_block = version.ecc_per_block(ec_level);
        let ecc_blocks = data_blocks.iter().map(|b| ecc(b, ecc_size_per_block)).collect::<Vec<_>>();

        (data_blocks, ecc_blocks)
    }

    pub(crate) fn blockify(data: &[u8], version: Version, ec_level: ECLevel) -> Vec<&[u8]> {
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

    pub fn ec_capacity(version: Version, ec_level: ECLevel) -> usize {
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

    pub fn interleave<T: Copy, V: Deref<Target = [T]>>(blocks: &[V]) -> Vec<T> {
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

#[cfg(test)]
mod builder_tests {
    use test_case::test_case;

    use super::QRBuilder;
    use crate::common::{ECLevel, Version};

    // TODO: assert data blocks as well
    #[test]
    fn test_add_ec_simple() {
        let msg = b" [\x0bx\xd1r\xdcMC@\xec\x11\xec\x11\xec\x11";
        let expected_ecc = [b"\xc4\x23\x27\x77\xeb\xd7\xe7\xe2\x5d\x17"];
        let (_, ecc) = QRBuilder::compute_ecc(msg, Version::Normal(1), ECLevel::M);
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
        let (_, ecc) = QRBuilder::compute_ecc(msg, Version::Normal(5), ECLevel::Q);
        assert_eq!(&*ecc, &expected_ec[..]);
    }

    #[test]
    fn test_interleave() {
        let blocks = vec![vec![1, 2, 3], vec![4, 5, 6], vec![7, 8, 9, 0]];
        let interleaved = QRBuilder::interleave(&blocks);
        let exp_interleaved = vec![1, 4, 7, 2, 5, 8, 3, 6, 9, 0];
        assert_eq!(interleaved, exp_interleaved);
    }

    #[test_case("Hello, world!🌎".to_string(), Version::Normal(1), ECLevel::L)]
    #[test_case("TEST".to_string(), Version::Normal(1), ECLevel::M)]
    #[test_case("12345".to_string(), Version::Normal(1), ECLevel::Q)]
    #[test_case("OK".to_string(), Version::Normal(1), ECLevel::H)]
    #[test_case("B3@j🎮#Z%8v🍣K!🔑3zC^8📖&r💾F9*🔐b6🌼".repeat(3).to_string(), Version::Normal(7), ECLevel::L)]
    #[test_case("A11111111111111".repeat(11).to_string(), Version::Normal(7), ECLevel::M)]
    #[test_case("aAAAAAA1111111111111AAAAAAa".repeat(3).to_string(), Version::Normal(7), ECLevel::Q)]
    #[test_case("1234567890".repeat(15).to_string(), Version::Normal(7), ECLevel::H)]
    #[test_case( "B3@j🎮#Z%8v🍣K!🔑3zC^8📖&r💾F9*🔐b6🌼".repeat(4).to_string(), Version::Normal(10), ECLevel::L)]
    #[test_case("A11111111111111".repeat(20).to_string(), Version::Normal(10), ECLevel::M)]
    #[test_case("aAAAAAAAAA1111111111111111AAAAAAAAAAa".repeat(4).to_string(), Version::Normal(10), ECLevel::Q)]
    #[test_case("1234567890".repeat(28).to_string(), Version::Normal(10), ECLevel::H)]
    #[test_case("B3@j🎮#Z%8v🍣K!🔑3zC^8📖&r💾F9*🔐b6🌼".repeat(22).to_string(), Version::Normal(27), ECLevel::L)]
    #[test_case("A111111111111111".repeat(100).to_string(), Version::Normal(27), ECLevel::M)]
    #[test_case("aAAAAAAAAA111111111111111111AAAAAAAAAAa".repeat(20).to_string(), Version::Normal(27), ECLevel::Q)]
    #[test_case("1234567890".repeat(145).to_string(), Version::Normal(27), ECLevel::H)]
    #[test_case("B3@j🎮#Z%8v🍣K!🔑3zC^8📖&r💾F9*🔐b6🌼".repeat(57).to_string(), Version::Normal(40), ECLevel::L)]
    #[test_case("A111111111111111".repeat(97).to_string(), Version::Normal(40), ECLevel::M)]
    #[test_case( "aAAAAAAAAA111111111111111111AAAAAAAAAAa".repeat(42).to_string(), Version::Normal(40), ECLevel::Q)]
    #[test_case("1234567890".repeat(305).to_string(), Version::Normal(40), ECLevel::H)]
    fn test_builder(data: String, version: Version, ec_level: ECLevel) {
        let qr = QRBuilder::new(data.as_bytes())
            .version(version)
            .ec_level(ec_level)
            .build()
            .unwrap()
            .render(10);

        let mut img = rqrr::PreparedImage::prepare(qr);
        let grids = img.detect_grids();
        assert_eq!(grids.len(), 1);
        let (meta, content) = grids[0].decode().unwrap();

        assert_eq!(*version, meta.version.0);
        assert_eq!(data, content);
    }

    #[test]
    #[should_panic]
    fn test_builder_data_overflow() {
        let data = "1234567890".repeat(306).to_string();

        QRBuilder::new(data.as_bytes())
            .version(Version::Normal(40))
            .ec_level(ECLevel::H)
            .build()
            .unwrap()
            .render(10);
    }
}
