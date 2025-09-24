mod qr;

pub(crate) use qr::QR;

use crate::{
    common::{
        codec::{encode, encode_with_version},
        ec::Block,
        mask::{apply_best_mask, MaskPattern},
        metadata::{ECLevel, Version},
        utils::{BitStream, QRError, QRResult},
    },
    debug_println,
};

#[cfg(test)]
pub(crate) use qr::Module;

pub struct QRBuilder<'a> {
    data: &'a [u8],
    ver: Option<Version>,
    ecl: ECLevel,
    hi_cap: bool,
    mask: Option<MaskPattern>,
}

impl<'a> QRBuilder<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self { data, ver: None, ecl: ECLevel::M, hi_cap: false, mask: None }
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

    pub fn high_capacity(&mut self, enabled: bool) -> &mut Self {
        self.hi_cap = enabled;
        self
    }

    pub fn mask(&mut self, mask: MaskPattern) -> &mut Self {
        self.mask = Some(mask);
        self
    }

    pub fn metadata(&self) -> String {
        match self.ver {
            Some(v) => format!(
                "{{ Version: {:?}, Ec level: {:?}, High Capacity: {:?} }}",
                *v, self.ecl, self.hi_cap
            ),
            None => {
                format!(
                    "{{ Version: None, Ec level: {:?}, High Capacity: {:?} }}",
                    self.ecl, self.hi_cap
                )
            }
        }
    }
}

#[cfg(test)]
mod qrbuilder_util_tests {
    use super::QRBuilder;
    use crate::metadata::{ECLevel, Version};

    #[test]
    fn test_metadata() {
        let data = "Hello, world!".as_bytes();
        let ver = Version::Normal(1);
        let ecl = ECLevel::L;
        let mut qr_bldr = QRBuilder::new(data);
        qr_bldr.version(ver).ec_level(ecl).high_capacity(false);
        assert_eq!(qr_bldr.metadata(), "{ Version: 1, Ec level: L, High Capacity: false }");
        qr_bldr.unset_version();
        assert_eq!(qr_bldr.metadata(), "{ Version: None, Ec level: L, High Capacity: false }");
    }
}

impl QRBuilder<'_> {
    pub fn build(&mut self) -> QRResult<QR> {
        debug_println!("\nConstructing QR {}...", self.metadata());
        if self.data.is_empty() {
            return Err(QRError::EmptyData);
        }

        // Encode data optimally
        debug_println!("Encoding data...");
        let (enc, ver) = match self.ver {
            Some(v) => (encode_with_version(self.data, v, self.ecl, self.hi_cap)?, v),
            None => {
                debug_println!("Finding best version...");
                encode(self.data, self.ecl, self.hi_cap)?
            }
        };

        let _data_len = self.data.len();
        let _data_cap = ver.data_capacity(self.ecl, self.hi_cap);
        let _ec_cap = Self::ec_capacity(ver, self.ecl);
        let tot_cwds = ver.total_codewords(self.hi_cap);

        debug_println!("Constructing payload with ecc & interleaving...");
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
        debug_println!("Constructing QR...");
        let mut qr = QR::new(ver, self.ecl, self.hi_cap);

        debug_println!("Drawing functional patterns...");
        qr.draw_all_function_patterns();

        debug_println!("Drawing encoding region...");
        qr.draw_encoding_region(pld);

        let mask = match self.mask {
            Some(m) => {
                debug_println!("Apply mask {m:?}...");
                qr.apply_mask(m);
                m
            }
            None => {
                debug_println!("Finding & applying best mask...");
                apply_best_mask(&mut qr)
            }
        };
        self.mask(mask);

        debug_println!("\x1b[1;32mQR generated successfully!\n \x1b[0m");

        let tot_mods = ver.width() * ver.width();
        let dark_mods = qr.count_dark_modules();
        let _lt_mods = tot_mods - dark_mods;

        debug_println!("Report:");
        debug_println!("{}", qr.metadata());
        debug_println!("Data capacity: {}, Error Capacity: {}", _data_cap, _ec_cap);
        debug_println!(
            "Data size: {}, Encoded size: {}, Compression: {}%",
            _data_len,
            enc.len() >> 3,
            (enc.len() >> 3) * 100 / _data_len
        );
        debug_println!(
            "Dark Cells: {}, Light Cells: {}, Balance: {}\n",
            dark_mods,
            _lt_mods,
            dark_mods * 100 / tot_mods
        );

        Ok(qr)
    }

    pub(crate) fn blockify(data: &[u8], ver: Version, ecl: ECLevel) -> Vec<Block> {
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

        let mut blks = Vec::with_capacity(256);
        data[..b1_tot_sz].chunks(b1s).for_each(|d| blks.push(Block::new(d, b1s + ec_len)));

        if b2s > 0 {
            data[b1_tot_sz..].chunks(b2s).for_each(|d| blks.push(Block::new(d, b2s + ec_len)));
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

    pub(crate) fn interleave_into(blks: &[Block], out: &mut BitStream) {
        // Interleaving data codewords
        let max_len = blks.iter().map(Block::data_len).max().expect("Blocks is empty");
        for i in 0..max_len {
            for bl in blks.iter() {
                if let Some(b) = bl.data().get(i) {
                    out.push_byte(*b)
                }
            }
        }

        // Interleaving ec codewords
        let ec_len = blks[0].ec_len();
        for i in 0..ec_len {
            for bl in blks.iter() {
                if let Some(b) = bl.ecc().get(i) {
                    out.push_byte(*b)
                }
            }
        }
    }
}

#[cfg(test)]
mod builder_tests {

    use super::QRBuilder;
    use crate::ec::Block;
    use crate::metadata::{ECLevel, Version};
    use crate::utils::BitStream;

    #[test]
    fn test_add_ec_simple() {
        let msg = b" [\x0bx\xd1r\xdcMC@\xec\x11\xec\x11\xec\x11";
        let exp_ecc = [b"\xc4\x23\x27\x77\xeb\xd7\xe7\xe2\x5d\x17"];
        let blks = QRBuilder::blockify(msg, Version::Normal(1), ECLevel::M);
        assert_eq!(blks.len(), exp_ecc.len());
        for (i, b) in blks.iter().enumerate() {
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
        assert_eq!(blks.len(), exp_ecc.len());
        for (i, b) in blks.iter().enumerate() {
            assert_eq!(b.ecc(), *exp_ecc[i]);
        }
    }

    #[test]
    fn test_interleave() {
        let data = [vec![1, 2, 3], vec![4, 5, 6], vec![7, 8, 9, 0]];
        let mut blks: Vec<Block> = Vec::with_capacity(256);
        data.iter().for_each(|b| blks.push(Block::new(b, 6)));
        let mut ilvd = BitStream::new(256);
        QRBuilder::interleave_into(&blks, &mut ilvd);
        let exp_ilvd = vec![1, 4, 7, 2, 5, 8, 3, 6, 9, 0];
        assert_eq!(ilvd.data()[..10], exp_ilvd);
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
