mod deqr;

use image::RgbImage;

use crate::common::{
    codec::decode,
    error::{QRError, QRResult},
    metadata::{Palette, Version},
    BitStream, Block,
};
use deqr::DeQR;

pub trait QRReadable {
    fn to_deqr(&self, ver: Version) -> DeQR;
}

impl QRReadable for String {
    fn to_deqr(&self, ver: Version) -> DeQR {
        DeQR::from_str(self, ver)
    }
}

impl QRReadable for RgbImage {
    fn to_deqr(&self, ver: Version) -> DeQR {
        DeQR::from_clr_img(self, ver)
    }
}

pub struct QRReader();

impl QRReader {
    // TODO: Remove version
    pub fn read<T: QRReadable>(qr: &T, ver: Version) -> QRResult<String> {
        println!("Reading QR...");
        let mut deqr = qr.to_deqr(ver);

        println!("Reading format info...");
        let (ecl, mask) = deqr.read_format_info()?;

        println!("Reading version info...");
        let ver = match ver {
            Version::Normal(7..=40) => deqr.read_version_info()?,
            _ => ver,
        };

        println!("Marking all function patterns...");
        deqr.mark_all_function_patterns();

        println!("Unmasking payload...");
        deqr.unmask(mask);

        println!("Extracting payload...");
        let pld = deqr.extract_payload(ver);

        let data_len = ver.data_bit_capacity(ecl, Palette::Mono) >> 3;
        let blk_info = ver.data_codewords_per_block(ecl);
        let ec_len = ver.ecc_per_block(ecl);

        // Extracting encoded data from payload
        let mut enc = BitStream::new(pld.len() << 3);
        let chan_cap = ver.channel_codewords();

        println!("Separating channels, deinterleaving & rectifying payload...");
        pld.data().chunks_exact(chan_cap).for_each(|c| {
            let mut blocks = Self::deinterleave(c, blk_info, ec_len);
            let _ = blocks.iter_mut().map(Block::rectify);
            blocks.iter().for_each(|b| enc.extend(b.data()));
        });

        println!("Decoding data blocks...");
        let msg = decode(&mut enc, ver);

        println!("\n{}\n", deqr.metadata());

        String::from_utf8(msg).or(Err(QRError::InvalidUTF8Sequence))
    }

    fn deinterleave(
        data: &[u8],
        blk_info: (usize, usize, usize, usize),
        ec_len: usize,
    ) -> Vec<Block> {
        let len = data.len();
        // b1s = block1_size, b1c = block1_count
        let (b1s, b1c, b2s, b2c) = blk_info;

        let total_blks = b1c + b2c;
        let spl = b1s * total_blks;
        let data_sz = b1s * b1c + b2s * b2c;

        let mut blks = vec![Vec::with_capacity(b2s); total_blks];

        // Deinterleaving data
        data[..spl]
            .chunks(total_blks)
            .for_each(|ch| ch.iter().enumerate().for_each(|(i, v)| blks[i].push(*v)));
        if b2c > 0 {
            data[spl..data_sz]
                .chunks(b2c)
                .for_each(|ch| ch.iter().enumerate().for_each(|(i, v)| blks[b1c + i].push(*v)));
        }

        // Deinterleaving ecc
        data[data_sz..]
            .chunks(total_blks)
            .for_each(|ch| ch.iter().enumerate().for_each(|(i, v)| blks[i].push(*v)));

        blks.iter().map(|b| Block::with_encoded(b, b.len() - ec_len)).collect()
    }
}

#[cfg(test)]
mod reader_tests {
    use test_case::test_case;

    use super::QRReader;
    use crate::builder::QRBuilder;
    use crate::common::metadata::{ECLevel, Version};
    use crate::common::BitStream;
    use crate::Palette;

    #[test]
    fn test_deinterleave() {
        // Data length has to match version capacity
        let data = "Hello, world!!!🌍".as_bytes();
        let ver = Version::Normal(1);
        let ecl = ECLevel::L;

        let exp_blks = QRBuilder::blockify(data, ver, ecl);

        let mut bs = BitStream::new(ver.total_codewords(Palette::Mono) << 3);
        QRBuilder::interleave_into(&exp_blks, &mut bs);

        let blk_info = ver.data_codewords_per_block(ecl);
        let ec_len = ver.ecc_per_block(ecl);
        let blks = QRReader::deinterleave(bs.data(), blk_info, ec_len);
        assert_eq!(blks, exp_blks);
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
    fn test_reader(data: String, ver: Version, ecl: ECLevel) {
        let qr =
            QRBuilder::new(data.as_bytes()).version(ver).ec_level(ecl).build().unwrap().to_str(1);

        let decoded_data = QRReader::read(&qr, ver).unwrap();

        assert_eq!(decoded_data, data);
    }
}
