mod qr;

pub(crate) use qr::Module;
pub(crate) use qr::QR;

use crate::common::{
    codec::{encode, encode_with_version},
    ec::Block,
    mask::{apply_best_mask, MaskPattern},
    metadata::{ECLevel, Palette, Version},
    utils::{BitStream, QRError, QRResult},
};

pub struct QRBuilder<'a> {
    data: &'a [u8],
    ver: Option<Version>,
    ecl: ECLevel,
    pal: Palette,
    mask: Option<MaskPattern>,
}

impl<'a> QRBuilder<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self { data, ver: None, ecl: ECLevel::M, pal: Palette::Mono, mask: None }
    }

    pub fn data(&mut self, data: &'a [u8]) -> &mut Self {
        self.data = data;
        self
    }

    pub fn version(&mut self, ver: Version) -> &mut Self {
        self.ver = Some(ver);
        self
    }

    pub fn unset_version(&mut self) -> &mut Self {
        self.ver = None;
        self
    }

    pub fn ec_level(&mut self, ecl: ECLevel) -> &mut Self {
        self.ecl = ecl;
        self
    }

    pub fn palette(&mut self, pal: Palette) -> &mut Self {
        self.pal = pal;
        self
    }

    pub fn mask(&mut self, mask: MaskPattern) -> &mut Self {
        self.mask = Some(mask);
        self
    }

    pub fn metadata(&self) -> String {
        match self.ver {
            Some(v) => format!(
                "{{ Version: {:?}, Ec level: {:?}, Palette: {:?} }}",
                *v, self.ecl, self.pal
            ),
            None => {
                format!("{{ Version: None, Ec level: {:?}, Palette: {:?} }}", self.ecl, self.pal)
            }
        }
    }
}

#[cfg(test)]
mod qrbuilder_util_tests {
    use super::QRBuilder;
    use crate::metadata::{ECLevel, Palette, Version};

    #[test]
    fn test_metadata() {
        let data = "Hello, world!".as_bytes();
        let ver = Version::Normal(1);
        let ecl = ECLevel::L;
        let pal = Palette::Mono;
        let mut qr_bldr = QRBuilder::new(data);
        qr_bldr.version(ver).ec_level(ecl).palette(pal);
        assert_eq!(qr_bldr.metadata(), "{ Version: 1, Ec level: L, Palette: Mono }");
        qr_bldr.unset_version();
        assert_eq!(qr_bldr.metadata(), "{ Version: None, Ec level: L, Palette: Mono }");
    }
}

impl QRBuilder<'_> {
    pub fn build(&mut self) -> QRResult<QR> {
        println!("\nConstructing QR {}...", self.metadata());
        if self.data.is_empty() {
            return Err(QRError::EmptyData);
        }

        // Encode data optimally
        println!("Encoding data...");
        let (enc, ver) = match self.ver {
            Some(v) => (encode_with_version(self.data, v, self.ecl, self.pal)?, v),
            None => {
                println!("Finding best version...");
                encode(self.data, self.ecl, self.pal)?
            }
        };

        let data_len = self.data.len();
        let tot_cwds = ver.total_codewords(self.pal);
        let data_cap = ver.data_capacity(self.ecl, self.pal);
        let ec_cap = Self::ec_capacity(ver, self.ecl);

        println!("Constructing payload with ecc & interleaving...");
        let mut pld = BitStream::new(tot_cwds << 3);
        let chan_data_cap = ver.channel_data_capacity(self.ecl);

        debug_assert!(
            enc.len() % chan_data_cap == 0,
            "Encoded data length {} is not divisible by channel codewords {chan_data_cap}",
            enc.len()
        );

        enc.data().chunks_exact(chan_data_cap).for_each(|c| {
            // Splits the data into EC block. The blocks will auto compute ecc
            let blks = Self::blockify(c, ver, self.ecl);

            // Interleave data & error correction codewords, and write into payload
            Self::interleave_into(&blks, &mut pld);
        });

        // Construct QR
        println!("Constructing QR...");
        let mut qr = QR::new(ver, self.ecl, self.pal);

        println!("Drawing functional patterns...");
        qr.draw_all_function_patterns();

        println!("Drawing encoding region...");
        qr.draw_encoding_region(pld);

        let mask = match self.mask {
            Some(m) => {
                println!("Apply mask {m:?}...");
                qr.apply_mask(m);
                m
            }
            None => {
                println!("Finding & applying best mask...");
                apply_best_mask(&mut qr)
            }
        };
        self.mask(mask);

        println!("\x1b[1;32mQR generated successfully!\n \x1b[0m");

        let tot_mods = ver.width() * ver.width();
        let dark_mods = qr.count_dark_modules();
        let lt_mods = tot_mods - dark_mods;

        println!("Report:");
        println!("{}", qr.metadata());
        println!("Data capacity: {}, Error Capacity: {}", data_cap, ec_cap);
        println!(
            "Data size: {}, Encoded size: {}, Compression: {}%",
            data_len,
            enc.len() >> 3,
            (enc.len() >> 3) * 100 / data_len
        );
        println!(
            "Dark Cells: {}, Light Cells: {}, Balance: {}\n",
            dark_mods,
            lt_mods,
            dark_mods * 100 / tot_mods
        );

        Ok(qr)
    }

    pub(crate) fn blockify(data: &[u8], ver: Version, ecl: ECLevel) -> [Option<Block>; 256] {
        // b1s = block1_size, b1c = block1_count
        let (b1s, b1c, b2s, b2c) = ver.data_codewords_per_block(ecl);
        let ec_len = ver.ecc_per_block(ecl);

        let b1_tot_sz = b1s * b1c;
        let tot_sz = b1_tot_sz + b2s * b2c;

        debug_assert!(
            tot_sz == data.len(),
            "Data len doesn't match total size of blocks: Data len {}, Total block size {}",
            data.len(),
            tot_sz
        );

        let mut blks: [Option<Block>; 256] = [None; 256];
        data[..b1_tot_sz]
            .chunks(b1s)
            .enumerate()
            .for_each(|(i, d)| blks[i] = Some(Block::new(d, b1s + ec_len)));

        if b2s > 0 {
            data[b1_tot_sz..]
                .chunks(b2s)
                .enumerate()
                .for_each(|(i, d)| blks[i + b1c] = Some(Block::new(d, b2s + ec_len)));
        }

        blks
    }

    pub fn ec_capacity(ver: Version, ecl: ECLevel) -> usize {
        let p = match (ver, ecl) {
            (Version::Micro(2) | Version::Normal(1), ECLevel::L) => 3,
            (Version::Micro(_) | Version::Normal(2), ECLevel::L)
            | (Version::Micro(2) | Version::Normal(1), ECLevel::M) => 2,
            (Version::Normal(1), _) | (Version::Normal(3), ECLevel::L) => 1,
            _ => 0,
        };

        let ec_bpb = ver.ecc_per_block(ecl);
        let (_, cnt1, _, cnt2) = ver.data_codewords_per_block(ecl);
        let ec_bytes = (cnt1 + cnt2) * ec_bpb;

        (ec_bytes - p) / 2
    }

    pub(crate) fn interleave_into(blks: &[Option<Block>], out: &mut BitStream) {
        // Interleaving data codewords
        let max_len = blks
            .iter()
            .filter_map(Option::as_ref)
            .map(Block::data_len)
            .max()
            .expect("Blocks is empty");
        for i in 0..max_len {
            for bl in blks.iter().filter_map(Option::as_ref) {
                if let Some(b) = bl.data().get(i) {
                    out.push_byte(*b)
                }
            }
        }

        // Interleaving ec codewords
        let ec_len = blks[0].unwrap().ec_len();
        for i in 0..ec_len {
            for bl in blks.iter().filter_map(Option::as_ref) {
                if let Some(b) = bl.ecc().get(i) {
                    out.push_byte(*b)
                }
            }
        }
    }
}

#[cfg(test)]
mod builder_tests {

    use test_case::test_case;

    use super::QRBuilder;
    use crate::ec::Block;
    use crate::metadata::{ECLevel, Version};
    use crate::utils::BitStream;

    // TODO: assert data blocks as well
    #[test]
    fn test_add_ec_simple() {
        let msg = b" [\x0bx\xd1r\xdcMC@\xec\x11\xec\x11\xec\x11";
        let exp_ecc = [b"\xc4\x23\x27\x77\xeb\xd7\xe7\xe2\x5d\x17"];
        let blks = QRBuilder::blockify(msg, Version::Normal(1), ECLevel::M);
        assert_eq!(blks.iter().filter(|b| Option::is_some(b)).count(), exp_ecc.len());
        for (i, b) in blks.iter().filter_map(Option::as_ref).enumerate() {
            assert_eq!(b.ecc(), *exp_ecc[i]);
        }
    }

    #[test]
    fn test_add_ec_complex() {
        let msg = b"CUF\x86W&U\xc2w2\x06\x12\x06g&\xf6\xf6B\x07v\x86\xf2\x07&V\x16\xc6\xc7\x92\x06\
                    \xb6\xe6\xf7w2\x07v\x86W&R\x06\x86\x972\x07F\xf7vV\xc2\x06\x972\x10\xec\x11\xec\
                    \x11\xec\x11\xec";
        let exp_ecc = [
            b"\xd5\xc7\x0b\x2d\x73\xf7\xf1\xdf\xe5\xf8\x9a\x75\x9a\x6f\x56\xa1\x6f\x27",
            b"\x57\xcc\x60\x3c\xca\xb6\x7c\x9d\xc8\x86\x1b\x81\xd1\x11\xa3\xa3\x78\x85",
            b"\x94\x74\xb1\xd4\x4c\x85\x4b\xf2\xee\x4c\xc3\xe6\xbd\x0a\x6c\xf0\xc0\x8d",
            b"\xeb\x9f\x05\xad\x18\x93\x3b\x21\x6a\x28\xff\xac\x52\x02\x83\x20\xb2\xec",
        ];
        let blks = QRBuilder::blockify(msg, Version::Normal(5), ECLevel::Q);
        assert_eq!(blks.iter().filter(|b| Option::is_some(b)).count(), exp_ecc.len());
        for (i, b) in blks.iter().filter_map(Option::as_ref).enumerate() {
            assert_eq!(b.ecc(), *exp_ecc[i]);
        }
    }

    #[test]
    fn test_interleave() {
        let data = [vec![1, 2, 3], vec![4, 5, 6], vec![7, 8, 9, 0]];
        let mut blks: [Option<Block>; 256] = [None; 256];
        data.iter().enumerate().for_each(|(i, b)| blks[i] = Some(Block::new(b, 6)));
        let mut ilvd = BitStream::new(256);
        QRBuilder::interleave_into(&blks, &mut ilvd);
        let exp_ilvd = vec![1, 4, 7, 2, 5, 8, 3, 6, 9, 0];
        assert_eq!(ilvd.data()[..10], exp_ilvd);
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
    fn test_builder(data: String, ver: Version, ecl: ECLevel) {
        let qr = QRBuilder::new(data.as_bytes())
            .version(ver)
            .ec_level(ecl)
            .build()
            .unwrap()
            .to_gray_image(10);

        let mut img = rqrr::PreparedImage::prepare(qr);
        let grids = img.detect_grids();
        assert_eq!(grids.len(), 1);
        let (meta, msg) = grids[0].decode().unwrap();

        assert_eq!(*ver, meta.version.0);
        assert_eq!(data, msg);
    }

    #[test]
    #[should_panic]
    fn test_builder_data_overflow() {
        let data = "1234567890".repeat(306).to_string();

        QRBuilder::new(data.as_bytes())
            .version(Version::Normal(40))
            .ec_level(ECLevel::H)
            .build()
            .unwrap();
    }
}
