mod deqr;

use image::RgbImage;

use crate::common::{
    codec::decode,
    ec::rectify,
    error::{QRError, QRResult},
    metadata::{Palette, Version},
};
use deqr::DeQR;

pub trait QRReadable {
    fn to_deqr(&self, version: Version) -> DeQR;
}

impl QRReadable for String {
    fn to_deqr(&self, version: Version) -> DeQR {
        DeQR::from_str(self, version)
    }
}

impl QRReadable for RgbImage {
    fn to_deqr(&self, version: Version) -> DeQR {
        DeQR::from_clr_img(self, version)
    }
}

pub struct QRReader();

impl QRReader {
    // TODO: Remove version
    pub fn read<T: QRReadable>(qr: &T, version: Version) -> QRResult<String> {
        println!("Reading QR...");
        let mut deqr = qr.to_deqr(version);

        println!("Reading format info...");
        let (ec_level, mask_pattern) = deqr.read_format_info()?;

        println!("Reading version info...");
        let version = match version {
            Version::Normal(7..=40) => deqr.read_version_info()?,
            _ => version,
        };

        println!("Marking all function patterns...");
        deqr.mark_all_function_patterns();

        println!("Unmasking payload...");
        deqr.unmask(mask_pattern);

        println!("Extracting payload...");
        let payload = deqr.extract_payload(version);

        let data_size = version.bit_capacity(ec_level, Palette::Mono) >> 3;
        let block_info = version.data_codewords_per_block(ec_level);
        let total_blocks = block_info.1 + block_info.3;
        let epb = version.ecc_per_block(ec_level);

        // Extracting encoded data from payload
        let mut encoded_data = Vec::with_capacity(payload.len());
        let chunk_size = payload.len() / 3;

        println!("Separating channels, deinterleaving & rectifying payload...");
        payload.chunks_exact(chunk_size).for_each(|c| {
            let data_blocks: Vec<Vec<u8>> = Self::deinterleave(&c[..data_size], block_info);
            let ecc_blocks: Vec<Vec<u8>> =
                Self::deinterleave(&c[data_size..], (epb, total_blocks, 0, 0));

            let rectified_data = rectify(&data_blocks, &ecc_blocks);

            encoded_data.extend(rectified_data);
        });

        println!("Decoding data blocks...");
        let message = decode(&encoded_data, version);

        println!("\n{}\n", deqr.metadata());

        String::from_utf8(message).or(Err(QRError::InvalidUTF8Sequence))
    }

    fn deinterleave(data: &[u8], block_info: (usize, usize, usize, usize)) -> Vec<Vec<u8>> {
        let len = data.len();
        let (block1_size, block1_count, block2_size, block2_count) = block_info;

        let total_blocks = block1_count + block2_count;
        let partition = block1_size * total_blocks;
        let total_size = block1_size * block1_count + block2_size * block2_count;

        debug_assert!(len == total_size, "Data size doesn't match chunk total size: Data size {len}, Chunks total size {total_size}");

        let mut res = vec![Vec::with_capacity(block2_size); total_blocks];
        data[..partition]
            .chunks(total_blocks)
            .for_each(|ch| ch.iter().enumerate().for_each(|(i, v)| res[i].push(*v)));
        if block2_count > 0 {
            data[partition..].chunks(block2_count).for_each(|ch| {
                ch.iter().enumerate().for_each(|(i, v)| res[block1_count + i].push(*v))
            });
        }
        res
    }
}

#[cfg(test)]
mod reader_tests {
    use test_case::test_case;

    use super::QRReader;
    use crate::builder::QRBuilder;
    use crate::common::metadata::{ECLevel, Version};

    #[test]
    fn test_deinterleave() {
        // Data length has to match version capacity
        let data = "Hello, world!!!🌍".as_bytes();
        let version = Version::Normal(1);
        let ec_level = ECLevel::L;

        let data_blocks = QRBuilder::blockify(data, version, ec_level);

        let interleaved = QRBuilder::interleave(&data_blocks);

        let block_info = version.data_codewords_per_block(ec_level);
        let deinterleaved = QRReader::deinterleave(&interleaved, block_info);
        assert_eq!(data_blocks, deinterleaved);
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
    fn test_reader(data: String, version: Version, ec_level: ECLevel) {
        let qr = QRBuilder::new(data.as_bytes())
            .version(version)
            .ec_level(ec_level)
            .build()
            .unwrap()
            .to_str(1);

        let decoded_data = QRReader::read(&qr, version).unwrap();

        assert_eq!(decoded_data, data);
    }
}
