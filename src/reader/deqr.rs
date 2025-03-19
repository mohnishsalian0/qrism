use std::ops::{Deref, Not};

use image::{GrayImage, Luma};

use crate::common::{
    ec::rectify_info,
    error::{QRError, QRResult},
    iter::EncRegionIter,
    mask::MaskPattern,
    metadata::{
        parse_format_info_qr, Color, ECLevel, Metadata, Palette, Version, FORMAT_ERROR_CAPACITY,
        FORMAT_INFOS_QR, FORMAT_INFO_COORDS_QR_MAIN, FORMAT_INFO_COORDS_QR_SIDE, FORMAT_MASK,
        VERSION_ERROR_BIT_LEN, VERSION_ERROR_CAPACITY, VERSION_INFOS, VERSION_INFO_COORDS_BL,
        VERSION_INFO_COORDS_TR,
    },
};

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum DeModule {
    Unmarked(Color),
    Marked,
}

impl Deref for DeModule {
    type Target = Color;
    fn deref(&self) -> &Self::Target {
        match self {
            DeModule::Unmarked(c) => c,
            DeModule::Marked => &Color::Dark,
        }
    }
}

impl Not for DeModule {
    type Output = DeModule;
    fn not(self) -> Self::Output {
        match self {
            DeModule::Unmarked(c) => DeModule::Unmarked(!c),
            DeModule::Marked => DeModule::Marked,
        }
    }
}

// QR type for reader
//------------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct DeQR {
    width: usize,
    grid: Vec<DeModule>,
    version: Version,
    ec_level: Option<ECLevel>,
    palette: Option<Palette>,
    mask_pattern: Option<MaskPattern>,
}

impl DeQR {
    pub fn from_image(qr: &GrayImage, version: Version) -> Self {
        let qr_width = version.width();
        let (w, h) = qr.dimensions();
        let (w, h) = (w as i16, h as i16);
        let mod_size = w / qr_width as i16;
        let qz_size = if let Version::Normal(_) = version { 4 } else { 2 } * mod_size;

        debug_assert!(w == h, "Image is not perfect square");
        debug_assert!(
            (w - 2 * qz_size) % qr_width as i16 == 0,
            "Image width is not a multiple of qr size"
        );

        let half_area = mod_size * mod_size / 2;

        let mut black_count = vec![0; qr_width * qr_width];
        for (c, r, pixel) in qr.enumerate_pixels() {
            let (r, c) = (r as i16, c as i16);
            if r < qz_size || r >= w - qz_size || c < qz_size || c >= w - qz_size {
                continue;
            }
            let index =
                Self::coord_to_index((r - qz_size) / mod_size, (c - qz_size) / mod_size, qr_width);
            let Luma([luma]) = *pixel;
            black_count[index] += if luma < 128 { 1 } else { 0 };
        }

        let grid = black_count
            .iter()
            .map(|&bc| DeModule::Unmarked(if bc > half_area { Color::Dark } else { Color::Light }))
            .collect();

        Self { width: qr_width, grid, version, ec_level: None, palette: None, mask_pattern: None }
    }

    pub fn from_str(qr: &str, version: Version) -> Self {
        let qr_width = version.width();
        let qz_size = if let Version::Normal(_) = version { 4 } else { 2 };
        let full_width = qz_size + qr_width + qz_size;

        let grid = qr
            .chars()
            .filter(|clr| *clr != '\n')
            .enumerate()
            .filter(|(i, clr)| {
                let (r, c) = (i / full_width, i % full_width);
                r >= qz_size && r < qz_size + qr_width && c >= qz_size && c < qz_size + qr_width
            })
            .map(|(i, clr)| DeModule::Unmarked(if clr == ' ' { Color::Dark } else { Color::Light }))
            .collect();

        Self { width: qr_width, grid, version, ec_level: None, palette: None, mask_pattern: None }
    }

    pub fn metadata(&self) -> Metadata {
        Metadata::new(Some(self.version), self.ec_level, self.palette, self.mask_pattern)
    }

    pub fn count_dark_modules(&self) -> usize {
        self.grid.iter().filter(|&m| matches!(**m, Color::Dark)).count()
    }

    #[cfg(test)]
    pub fn to_debug_str(&self) -> String {
        let w = self.width as i16;
        let mut res = String::with_capacity((w * (w + 1)) as usize);
        res.push('\n');
        for i in 0..w {
            for j in 0..w {
                let c = match self.get(i, j) {
                    DeModule::Unmarked(Color::Dark) => 'u',
                    DeModule::Unmarked(Color::Light | Color::Hue(_)) => 'U',
                    DeModule::Marked => '.',
                };
                res.push(c);
            }
            res.push('\n');
        }
        res
    }

    fn coord_to_index(r: i16, c: i16, width: usize) -> usize {
        let w = width as i16;
        debug_assert!(-w <= r && r < w, "row should be greater than or equal to width");
        debug_assert!(-w <= c && c < w, "column should be greater than or equal to width");

        let r = if r < 0 { r + w } else { r };
        let c = if c < 0 { c + w } else { c };
        (r * w + c) as _
    }

    pub fn get(&self, r: i16, c: i16) -> DeModule {
        self.grid[Self::coord_to_index(r, c, self.width)]
    }

    pub fn get_mut(&mut self, r: i16, c: i16) -> &mut DeModule {
        let index = Self::coord_to_index(r, c, self.width);
        &mut self.grid[index]
    }

    pub fn set(&mut self, r: i16, c: i16, module: DeModule) {
        *self.get_mut(r, c) = module;
    }
}

#[cfg(test)]
mod deqr_util_tests {
    use super::DeQR;
    use crate::builder::QRBuilder;
    use crate::common::metadata::{ECLevel, Version};

    #[test]
    fn test_from_str() {
        let data = "Hello, world! 🌎";
        let version = Version::Normal(2);
        let size = version.width() as i16;
        let ec_level = ECLevel::L;

        let qr =
            QRBuilder::new(data.as_bytes()).version(version).ec_level(ec_level).build().unwrap();
        let qr_str = qr.to_str(1);

        let deqr = DeQR::from_str(&qr_str, version);

        for r in 0..size {
            for c in 0..size {
                assert_eq!(*qr.get(r, c), *deqr.get(r, c), "{r} {c}");
            }
        }
    }

    #[test]
    fn test_from_image() {
        let data = "Hello, world! 🌎";
        let version = Version::Normal(2);
        let size = version.width() as i16;
        let ec_level = ECLevel::L;

        let qr =
            QRBuilder::new(data.as_bytes()).version(version).ec_level(ec_level).build().unwrap();
        let qr_str = qr.render(1);

        let deqr = DeQR::from_image(&qr_str, version);

        for r in 0..size {
            for c in 0..size {
                assert_eq!(*qr.get(r, c), *deqr.get(r, c), "{r} {c}");
            }
        }
    }
}

// Format & version info
//------------------------------------------------------------------------------

impl DeQR {
    pub fn read_format_info(&mut self) -> QRResult<(ECLevel, MaskPattern)> {
        let main = self.get_number(&FORMAT_INFO_COORDS_QR_MAIN);
        let mut f = rectify_info(main, &FORMAT_INFOS_QR, FORMAT_ERROR_CAPACITY)
            .or_else(|_| {
                let side = self.get_number(&FORMAT_INFO_COORDS_QR_SIDE);
                rectify_info(side, &FORMAT_INFOS_QR, FORMAT_ERROR_CAPACITY)
            })
            .or(Err(QRError::InvalidFormatInfo))?;

        self.mark_coords(&FORMAT_INFO_COORDS_QR_MAIN);
        self.mark_coords(&FORMAT_INFO_COORDS_QR_SIDE);
        self.set(-8, 8, DeModule::Marked);

        f ^= FORMAT_MASK;
        let (ec_level, mask_pattern) = parse_format_info_qr(f);
        self.ec_level = Some(ec_level);
        self.mask_pattern = Some(mask_pattern);
        Ok((ec_level, mask_pattern))
    }

    pub fn read_version_info(&mut self) -> QRResult<Version> {
        debug_assert!(
            !matches!(self.version, Version::Micro(_) | Version::Normal(1..=6)),
            "Version is too small to read version info"
        );
        let bl = self.get_number(&VERSION_INFO_COORDS_BL);
        let v = rectify_info(bl, &VERSION_INFOS, VERSION_ERROR_CAPACITY)
            .or_else(|_| {
                let tr = self.get_number(&VERSION_INFO_COORDS_TR);
                rectify_info(tr, &VERSION_INFOS, VERSION_ERROR_CAPACITY)
            })
            .or(Err(QRError::InvalidVersionInfo))?;
        self.mark_coords(&VERSION_INFO_COORDS_BL);
        self.mark_coords(&VERSION_INFO_COORDS_TR);
        Ok(Version::Normal(v as usize >> VERSION_ERROR_BIT_LEN))
    }

    pub fn get_number(&mut self, coords: &[(i16, i16)]) -> u32 {
        let mut number = 0;
        for (r, c) in coords {
            let m = self.get_mut(*r, *c);
            number = (number << 1) | u32::from(**m);
        }
        number
    }

    pub fn mark_coords(&mut self, coords: &[(i16, i16)]) {
        for (r, c) in coords {
            self.set(*r, *c, DeModule::Marked);
        }
    }
}

#[cfg(test)]
mod deqr_infos_test {

    use super::DeQR;
    use crate::builder::Module;
    use crate::builder::QRBuilder;
    use crate::common::{
        mask::MaskPattern,
        metadata::{Color, ECLevel, Version},
    };

    #[test]
    fn test_read_format_info() {
        let data = "Hello, world! 🌎";
        let version = Version::Normal(2);
        let size = version.width() as i16;
        let ec_level = ECLevel::L;
        let mask_pattern = MaskPattern::new(1);

        let qr = QRBuilder::new(data.as_bytes())
            .version(version)
            .ec_level(ec_level)
            .mask(mask_pattern)
            .build()
            .unwrap();
        let qr_str = qr.to_str(1);

        let mut deqr = DeQR::from_str(&qr_str, version);

        let format_info = deqr.read_format_info().unwrap();
        assert_eq!(format_info, (ec_level, mask_pattern));
    }

    #[test]
    fn test_read_format_info_one_corrupted() {
        let data = "Hello, world! 🌎";
        let version = Version::Normal(2);
        let size = version.width() as i16;
        let ec_level = ECLevel::L;
        let mask_pattern = MaskPattern::new(1);

        let mut qr = QRBuilder::new(data.as_bytes())
            .version(version)
            .ec_level(ec_level)
            .mask(mask_pattern)
            .build()
            .unwrap();
        qr.set(8, 1, Module::Format(Color::Light));
        qr.set(8, 2, Module::Format(Color::Light));
        qr.set(8, 4, Module::Format(Color::Dark));
        let qr_str = qr.to_str(1);

        let mut deqr = DeQR::from_str(&qr_str, version);

        let format_info = deqr.read_format_info().unwrap();
        assert_eq!(format_info, (ec_level, mask_pattern));
    }

    #[test]
    fn test_read_format_info_one_fully_corrupted() {
        let data = "Hello, world! 🌎";
        let version = Version::Normal(2);
        let size = version.width() as i16;
        let ec_level = ECLevel::L;
        let mask_pattern = MaskPattern::new(1);

        let mut qr = QRBuilder::new(data.as_bytes())
            .version(version)
            .ec_level(ec_level)
            .mask(mask_pattern)
            .build()
            .unwrap();
        qr.set(8, 1, Module::Format(Color::Light));
        qr.set(8, 2, Module::Format(Color::Light));
        qr.set(8, 3, Module::Format(Color::Dark));
        qr.set(8, 4, Module::Format(Color::Dark));
        let qr_str = qr.to_str(1);

        let mut deqr = DeQR::from_str(&qr_str, version);

        let format_info = deqr.read_format_info().unwrap();
        assert_eq!(format_info, (ec_level, mask_pattern));
    }

    #[test]
    #[should_panic]
    fn test_read_format_info_both_fully_corrupted() {
        let data = "Hello, world! 🌎";
        let version = Version::Normal(2);
        let size = version.width() as i16;
        let ec_level = ECLevel::L;
        let mask_pattern = MaskPattern::new(1);

        let mut qr = QRBuilder::new(data.as_bytes())
            .version(version)
            .ec_level(ec_level)
            .mask(mask_pattern)
            .build()
            .unwrap();
        qr.set(8, 1, Module::Format(Color::Light));
        qr.set(8, 2, Module::Format(Color::Light));
        qr.set(8, 3, Module::Format(Color::Dark));
        qr.set(8, 4, Module::Format(Color::Dark));
        qr.set(-2, 8, Module::Format(Color::Light));
        qr.set(-3, 8, Module::Format(Color::Light));
        qr.set(-4, 8, Module::Format(Color::Dark));
        qr.set(-5, 8, Module::Format(Color::Dark));
        let qr_str = qr.to_str(1);

        let mut deqr = DeQR::from_str(&qr_str, version);

        let format_info = deqr.read_format_info().unwrap();
        assert_eq!(format_info, (ec_level, mask_pattern));
    }

    #[test]
    fn test_mark_format_info() {
        let data = "Hello, world! 🌎";
        let version = Version::Normal(2);
        let ec_level = ECLevel::L;

        let qr =
            QRBuilder::new(data.as_bytes()).version(version).ec_level(ec_level).build().unwrap();
        let qr_str = qr.to_str(1);

        let mut deqr = DeQR::from_str(&qr_str, version);
        let _ = deqr.read_format_info();

        assert_eq!(
            deqr.to_debug_str(),
            "\n\
            uuuuuuuU.UuUUuuUuUuuuuuuu\n\
            uUUUUUuU.uUUUUuUUUuUUUUUu\n\
            uUuuuUuU.UUuUUuUUUuUuuuUu\n\
            uUuuuUuU.uUUuuUUUUuUuuuUu\n\
            uUuuuUuU.uUUuuuUuUuUuuuUu\n\
            uUUUUUuU.UuuuUuUuUuUUUUUu\n\
            uuuuuuuUuUuUuUuUuUuuuuuuu\n\
            UUUUUUUU.UUuUUUUuUUUUUUUU\n\
            ......u..UUUuUUuU........\n\
            UUUuUUUuUuUuuuUuUUuUUUuuu\n\
            UUUuUuuUUUuuuUuuuUuuUUuuu\n\
            UuuuUuUuuuuUuuuuUuuUuUUUU\n\
            UuUUUuuUuUuuUUuUUuuUUUUuu\n\
            UuUUuuUuUUuuUUuUuuuUUUuuu\n\
            uUUuuuuuuuuUUuUuuuUuUUuuu\n\
            UuUUUUUuuUuuUUUuUuUUUUUuU\n\
            uUUuUUuUUUuUuUUUuuuuuUUUU\n\
            UUUUUUUU.uuuuuUuuUUUuUuuU\n\
            uuuuuuuU.uuuuUuuuUuUuuUuu\n\
            uUUUUUuU.uuUuuuuuUUUuuUuu\n\
            uUuuuUuU.UuuUUuUuuuuuUUUu\n\
            uUuuuUuU.UuuUUuUUUUUuUuUU\n\
            uUuuuUuU.uUUUuUUuUUuuUuUu\n\
            uUUUUUuU.UuuUUUuuUUuUuUuU\n\
            uuuuuuuU.uUUuUUUuUuUuUUuu\n"
        );
    }

    #[test]
    fn test_read_version_info() {
        let data = "Hello, world! 🌎";
        let version = Version::Normal(7);
        let ec_level = ECLevel::L;

        let qr =
            QRBuilder::new(data.as_bytes()).version(version).ec_level(ec_level).build().unwrap();
        let qr_str = qr.to_str(1);

        let mut deqr = DeQR::from_str(&qr_str, version);

        let version_info = deqr.read_version_info().unwrap();
        assert_eq!(version_info, version);
    }

    #[test]
    fn test_read_version_info_one_corrupted() {
        let data = "Hello, world! 🌎";
        let version = Version::Normal(7);
        let ec_level = ECLevel::L;

        let mut qr =
            QRBuilder::new(data.as_bytes()).version(version).ec_level(ec_level).build().unwrap();
        qr.set(-9, 5, Module::Format(Color::Dark));
        qr.set(-10, 5, Module::Format(Color::Dark));
        qr.set(-11, 5, Module::Format(Color::Dark));
        let qr_str = qr.to_str(1);

        let mut deqr = DeQR::from_str(&qr_str, version);

        let version_info = deqr.read_version_info().unwrap();
        assert_eq!(version_info, version);
    }

    #[test]
    fn test_read_version_info_one_fully_corrupted() {
        let data = "Hello, world! 🌎";
        let version = Version::Normal(7);
        let ec_level = ECLevel::L;

        let mut qr =
            QRBuilder::new(data.as_bytes()).version(version).ec_level(ec_level).build().unwrap();
        qr.set(-9, 5, Module::Format(Color::Dark));
        qr.set(-10, 5, Module::Format(Color::Dark));
        qr.set(-11, 5, Module::Format(Color::Dark));
        qr.set(-9, 4, Module::Format(Color::Light));
        let qr_str = qr.to_str(1);

        let mut deqr = DeQR::from_str(&qr_str, version);

        let version_info = deqr.read_version_info().unwrap();
        assert_eq!(version_info, version);
    }

    #[test]
    #[should_panic]
    fn test_read_version_info_both_fully_corrupted() {
        let data = "Hello, world! 🌎";
        let version = Version::Normal(7);
        let ec_level = ECLevel::L;

        let mut qr =
            QRBuilder::new(data.as_bytes()).version(version).ec_level(ec_level).build().unwrap();
        qr.set(-9, 5, Module::Format(Color::Dark));
        qr.set(-10, 5, Module::Format(Color::Dark));
        qr.set(-11, 5, Module::Format(Color::Dark));
        qr.set(-9, 4, Module::Format(Color::Light));
        qr.set(5, -9, Module::Format(Color::Dark));
        qr.set(5, -10, Module::Format(Color::Dark));
        qr.set(5, -11, Module::Format(Color::Dark));
        qr.set(4, -9, Module::Format(Color::Light));
        let qr_str = qr.to_str(1);

        let mut deqr = DeQR::from_str(&qr_str, version);

        let version_info = deqr.read_version_info().unwrap();
    }

    #[test]
    fn test_mark_version_info() {
        let data = "Hello, world! 🌎";
        let version = Version::Normal(7);
        let ec_level = ECLevel::L;

        let qr =
            QRBuilder::new(data.as_bytes()).version(version).ec_level(ec_level).build().unwrap();
        let qr_str = qr.to_str(1);

        let mut deqr = DeQR::from_str(&qr_str, version);
        let _ = deqr.read_version_info();

        assert_eq!(
            deqr.to_debug_str(),
            "\n\
            uuuuuuuUuuuUuuUuUUuuuUuUuUUUUuUUuu...Uuuuuuuu\n\
            uUUUUUuUUUuUuuUuUuUUUUuUuuuUUuUuUu...UuUUUUUu\n\
            uUuuuUuUuUuUUuuUuuUUuuUUuuUUuuuUUU...UuUuuuUu\n\
            uUuuuUuUuUuUUuuUUUuUUuUuUuUUuUUUUu...UuUuuuUu\n\
            uUuuuUuUuuuUUUuUUUUuuuuuuUuUuUuuuU...UuUuuuUu\n\
            uUUUUUuUUuUUUUuuuUUUuUUUuUUuUUUuUu...UuUUUUUu\n\
            uuuuuuuUuUuUuUuUuUuUuUuUuUuUuUuUuUuUuUuuuuuuu\n\
            UUUUUUUUUUuuUUUUUUUUuUUUuuUUUuUuuuuuUUUUUUUUU\n\
            uuuuUUuUuUUuuUuUuUUUuuuuuuuuUuUuUUUUUuUUuuuUu\n\
            UuUuuUUUuuuuuUUUuuuuuuUuuuuuuUuuUUuUuuUuUuUUu\n\
            uUuUUUuUUUUuuUuUuUuuuuUuUUuuuUUUuUUuuUUuuuuuu\n\
            UUUuuuUuuuuuUUUuUUuuUUuuUUuuuUUuUuUuuUuuuuuuU\n\
            uUUuUUuUUuUUUUUuUuUuUUUUuUuuUuuuuuuUuUuuUUuuu\n\
            uUuUUuUuUUUUUuuuuuUUuUUUUuUuUuUUUuuUuuUuUUuuu\n\
            uUuuUuuUuUUuUuUUUuuUUuuUUuUUuuUUuUuUuuUUuUUuU\n\
            UuUUUuUUUuuUuuUUuUUUUuUuuUUUUuUUUuUuUuUuuuuuU\n\
            UuuUuuuuuUUuUUuuUUuuuuUuUuuUUuuuUuUuUUuuUuuUu\n\
            UUUUUUUuUuuuuUUUuUUuUuUUUuUuuuuuuUuUuuUUUUUuu\n\
            UUuUuuuUuuuUUuuUUUuUUuuuUuUuUUuuuUUUuUuuUuuuU\n\
            UUUuuuUUUUUuUUUUUUUuUuuuuUuUUUuuuUUUuUuUUuUuu\n\
            UUuuuuuuuUUUUUuuuUUuuuuuuUUuUUuuUuuuuuuuuuUUU\n\
            uUUUuUUUuuuUuUUuUuuuuUUUuUUuuuUuUuUUuUUUuuUUU\n\
            uUUuuUuUuUuuUuUuUuUuuUuUuuuuuuUUuuUuuUuUuUuuU\n\
            UuuUuUUUuuUUuuuUuuUUuUUUuUuUuUUUUuUUuUUUuUUuu\n\
            uuuUuuuuuuuuuuuuUUuUuuuuuUuUuuuUUuuuuuuuuUUuu\n\
            UuUuUUUUuuUuUUuuuUUUUUUUuuUUuuUuuuuuUuuuuUuUU\n\
            uuUuUuuUUuuuUuUUUuuUUuUUUuUuUuUuUUuUUuUUUUuUU\n\
            UUUUuUUuUUuUuuUUuUUuuuuuUuuUUUuUUUuUuuUUuuUUU\n\
            uUUUUUuUUUUUUUuUUUuuUuuuuUUUUUUuUUuUuUuuuuUUU\n\
            UuUUUuUuUuuUUUUuUUUuUUuuuUuuuUUuuuUuuUUuuuuUU\n\
            uUUUuUuuUuUuuUUuuuUUuuUuuUUuUuuuuuUuuUUUuUuUU\n\
            uuuUuUUuUUUuUuuuuuuUuuUuUUuuUUuUuUUuuUUuuuUuu\n\
            uUUuuuuuuUUUUuUuuuuuuUuuUUUUuUuUuuuuUUuuuUuuU\n\
            uUUuUuUuuUuuUuUuUUuUUUUUuUUUUuUUuuUuUUuuUUuUU\n\
            ......uUUUUuuuUuUuUUUUUUUuuUUuUuUuUUuuUuUUUuu\n\
            ......UUUuUuuuuUuuUuUuuUUuUUuuuUUUuUUuUUuUUuu\n\
            ......uUuuuUuuuUUUuUuuuuuuUUuUUUUUUuuuuuuuuUU\n\
            UUUUUUUUuuUUuUuUUUUUuUUUuUuUuUuuuUUuuUUUuuUUu\n\
            uuuuuuuUUUUUuUuuuUUuuUuUuUUuUUUuUuuUuUuUuUUUu\n\
            uUUUUUuUUuuuUUuuUuuuuUUUuuuuUUuuUUuUuUUUuuuUu\n\
            uUuuuUuUUuuUUuUUUuUuuuuuuUUuuUUUuUuuuuuuuUuuU\n\
            uUuuuUuUuuUUUuUuUuUuuUuuUUuUUUUUUuUuUuUUUuuUU\n\
            uUuuuUuUuuUuuUUuuuUuuUUUuUUUuuuUUuUUuUUUUUUUU\n\
            uUUUUUuUuuUuUuuuuuuUuuuuUuUuUuUUuuuUUuUuuUuUu\n\
            uuuuuuuUuUuuUuUUuuuUuUUuuuuUuuUUuUUUUUuuuUUUU\n"
        );
    }
}

// All function patterns
//------------------------------------------------------------------------------

// Marks all function pattern so they are ignored while extracting data
impl DeQR {
    pub fn mark_all_function_patterns(&mut self) {
        self.mark_finder_patterns();
        self.mark_timing_patterns();
        self.mark_alignment_patterns();
    }
}

#[cfg(test)]
mod deqr_all_function_tests {
    use super::DeQR;
    use crate::{
        builder::QRBuilder,
        common::metadata::{ECLevel, Version},
    };

    #[test]
    fn test_mark_all_function_pattern() {
        let data = "Hello, world! 🌎";
        let version = Version::Normal(7);
        let ec_level = ECLevel::L;

        let qr =
            QRBuilder::new(data.as_bytes()).version(version).ec_level(ec_level).build().unwrap();
        let qr_str = qr.to_str(1);

        let mut deqr = DeQR::from_str(&qr_str, version);
        deqr.mark_all_function_patterns();

        println!("{}", deqr.to_debug_str());

        assert_eq!(
            deqr.to_debug_str(),
            "\n\
            ........uuuUuuUuUUuuuUuUuUUUUuUUuuUUu........\n\
            ........UUuUuuUuUuUUUUuUuuuUUuUuUuUuU........\n\
            ........uUuUUuuUuuUUuuUUuuUUuuuUUUUuU........\n\
            ........uUuUUuuUUUuUUuUuUuUUuUUUUuUuu........\n\
            ........uuuUUUuUUUUu.....UuUuUuuuUuuu........\n\
            ........UuUUUUuuuUUU.....UUuUUUuUuUUU........\n\
            .............................................\n\
            ........UUuuUUUUUUUU.....uUUUuUuuuuuU........\n\
            uuuuUU.UuUUuuUuUuUUU.....uuuUuUuUUUUUuUUuuuUu\n\
            UuUuuU.UuuuuuUUUuuuuuuUuuuuuuUuuUUuUuuUuUuUUu\n\
            uUuUUU.UUUUuuUuUuUuuuuUuUUuuuUUUuUUuuUUuuuuuu\n\
            UUUuuu.uuuuuUUUuUUuuUUuuUUuuuUUuUuUuuUuuuuuuU\n\
            uUUuUU.UUuUUUUUuUuUuUUUUuUuuUuuuuuuUuUuuUUuuu\n\
            uUuUUu.uUUUUUuuuuuUUuUUUUuUuUuUUUuuUuuUuUUuuu\n\
            uUuuUu.UuUUuUuUUUuuUUuuUUuUUuuUUuUuUuuUUuUUuU\n\
            UuUUUu.UUuuUuuUUuUUUUuUuuUUUUuUUUuUuUuUuuuuuU\n\
            UuuUuu.uuUUuUUuuUUuuuuUuUuuUUuuuUuUuUUuuUuuUu\n\
            UUUUUU.uUuuuuUUUuUUuUuUUUuUuuuuuuUuUuuUUUUUuu\n\
            UUuUuu.UuuuUUuuUUUuUUuuuUuUuUUuuuUUUuUuuUuuuU\n\
            UUUuuu.UUUUuUUUUUUUuUuuuuUuUUUuuuUUUuUuUUuUuu\n\
            UUuu.....UUUUUuuuUUu.....UUuUUuuUuuu.....uUUU\n\
            uUUU.....uuUuUUuUuuu.....UUuuuUuUuUU.....uUUU\n\
            uUUu.....UuuUuUuUuUu.....uuuuuUUuuUu.....UuuU\n\
            UuuU.....uUUuuuUuuUU.....UuUuUUUUuUU.....UUuu\n\
            uuuU.....uuuuuuuUUuU.....UuUuuuUUuuu.....UUuu\n\
            UuUuUU.UuuUuUUuuuUUUUUUUuuUUuuUuuuuuUuuuuUuUU\n\
            uuUuUu.UUuuuUuUUUuuUUuUUUuUuUuUuUUuUUuUUUUuUU\n\
            UUUUuU.uUUuUuuUUuUUuuuuuUuuUUUuUUUuUuuUUuuUUU\n\
            uUUUUU.UUUUUUUuUUUuuUuuuuUUUUUUuUUuUuUuuuuUUU\n\
            UuUUUu.uUuuUUUUuUUUuUUuuuUuuuUUuuuUuuUUuuuuUU\n\
            uUUUuU.uUuUuuUUuuuUUuuUuuUUuUuuuuuUuuUUUuUuUU\n\
            uuuUuU.uUUUuUuuuuuuUuuUuUUuuUUuUuUUuuUUuuuUuu\n\
            uUUuuu.uuUUUUuUuuuuuuUuuUUUUuUuUuuuuUUuuuUuuU\n\
            uUUuUu.uuUuuUuUuUUuUUUUUuUUUUuUUuuUuUUuuUUuUU\n\
            UUUUuU.UUUUuuuUuUuUUUUUUUuuUUuUuUuUUuuUuUUUuu\n\
            UuuuuU.UUuUuuuuUuuUuUuuUUuUUuuuUUUuUUuUUuUUuu\n\
            uUUuuU.UuuuUuuuUUUuU.....uUUuUUUUUUu.....uuUU\n\
            ........uuUUuUuUUUUU.....UuUuUuuuUUu.....uUUu\n\
            ........UUUUuUuuuUUu.....UUuUUUuUuuU.....UUUu\n\
            ........UuuuUUuuUuuu.....uuuUUuuUUuU.....uuUu\n\
            ........UuuUUuUUUuUu.....UUuuUUUuUuu.....UuuU\n\
            ........uuUUUuUuUuUuuUuuUUuUUUUUUuUuUuUUUuuUU\n\
            ........uuUuuUUuuuUuuUUUuUUUuuuUUuUUuUUUUUUUU\n\
            ........uuUuUuuuuuuUuuuuUuUuUuUUuuuUUuUuuUuUu\n\
            ........uUuuUuUUuuuUuUUuuuuUuuUUuUUUUUuuuUUUU\n"
        );
    }
}

// Finder pattern
//------------------------------------------------------------------------------

impl DeQR {
    pub fn mark_finder_patterns(&mut self) {
        self.mark_finder_pattern_at(3, 3);
        match self.version {
            Version::Micro(_) => {}
            Version::Normal(_) => {
                self.mark_finder_pattern_at(3, -4);
                self.mark_finder_pattern_at(-4, 3);
            }
        }
    }

    fn mark_finder_pattern_at(&mut self, r: i16, c: i16) {
        let (dr_left, dr_right) = if r > 0 { (-3, 4) } else { (-4, 3) };
        let (dc_top, dc_bottom) = if c > 0 { (-3, 4) } else { (-4, 3) };
        for i in dr_left..=dr_right {
            for j in dc_top..=dc_bottom {
                self.set(r + i, c + j, DeModule::Marked);
            }
        }
    }
}

#[cfg(test)]
mod deqr_finder_tests {
    use super::DeQR;
    use crate::{
        builder::QRBuilder,
        common::metadata::{ECLevel, Version},
    };

    #[test]
    fn test_mark_finder_pattern() {
        let data = "Hello, world! 🌎";
        let version = Version::Normal(2);
        let ec_level = ECLevel::L;

        let qr =
            QRBuilder::new(data.as_bytes()).version(version).ec_level(ec_level).build().unwrap();
        let qr_str = qr.to_str(1);

        let mut deqr = DeQR::from_str(&qr_str, version);
        deqr.mark_finder_patterns();

        assert_eq!(
            deqr.to_debug_str(),
            "\n\
            ........UUuUUuuUu........\n\
            ........UuUUUUuUU........\n\
            ........uUUuUUuUU........\n\
            ........UuUUuuUUU........\n\
            ........UuUUuuuUu........\n\
            ........UUuuuUuUu........\n\
            ........uUuUuUuUu........\n\
            ........uUUuUUUUu........\n\
            uuuUuuuuuUUUuUUuUuuUUUuUU\n\
            UUUuUUUuUuUuuuUuUUuUUUuuu\n\
            UUUuUuuUUUuuuUuuuUuuUUuuu\n\
            UuuuUuUuuuuUuuuuUuuUuUUUU\n\
            UuUUUuuUuUuuUUuUUuuUUUUuu\n\
            UuUUuuUuUUuuUUuUuuuUUUuuu\n\
            uUUuuuuuuuuUUuUuuuUuUUuuu\n\
            UuUUUUUuuUuuUUUuUuUUUUUuU\n\
            uUUuUUuUUUuUuUUUuuuuuUUUU\n\
            ........uuuuuuUuuUUUuUuuU\n\
            ........uuuuuUuuuUuUuuUuu\n\
            ........uuuUuuuuuUUUuuUuu\n\
            ........uUuuUUuUuuuuuUUUu\n\
            ........UUuuUUuUUUUUuUuUU\n\
            ........uuUUUuUUuUUuuUuUu\n\
            ........uUuuUUUuuUUuUuUuU\n\
            ........uuUUuUUUuUuUuUUuu\n"
        );
    }
}

// Timing pattern
//------------------------------------------------------------------------------

impl DeQR {
    pub fn mark_timing_patterns(&mut self) {
        let w = self.width as i16;
        let (offset, last) = match self.version {
            Version::Micro(_) => (0, w - 1),
            Version::Normal(_) => (6, w - 9),
        };
        self.mark_line(offset, 8, offset, last);
        self.mark_line(8, offset, last, offset);
    }

    fn mark_line(&mut self, r1: i16, c1: i16, r2: i16, c2: i16) {
        debug_assert!(r1 == r2 || c1 == c2, "Line is neither vertical nor horizontal");

        if r1 == r2 {
            for j in c1..=c2 {
                self.set(r1, j, DeModule::Marked);
            }
        } else {
            for i in r1..=r2 {
                self.set(i, c1, DeModule::Marked);
            }
        }
    }
}

#[cfg(test)]
mod deqr_timing_tests {
    use super::DeQR;
    use crate::{
        builder::QRBuilder,
        common::metadata::{ECLevel, Version},
    };

    #[test]
    fn test_mark_timing_pattern() {
        let data = "Hello, world! 🌎";
        let version = Version::Normal(2);
        let ec_level = ECLevel::L;

        let qr =
            QRBuilder::new(data.as_bytes()).version(version).ec_level(ec_level).build().unwrap();
        let qr_str = qr.to_str(1);

        let mut deqr = DeQR::from_str(&qr_str, version);
        deqr.mark_timing_patterns();

        assert_eq!(
            deqr.to_debug_str(),
            "\n\
            uuuuuuuUUUuUUuuUuUuuuuuuu\n\
            uUUUUUuUUuUUUUuUUUuUUUUUu\n\
            uUuuuUuUuUUuUUuUUUuUuuuUu\n\
            uUuuuUuUUuUUuuUUUUuUuuuUu\n\
            uUuuuUuUUuUUuuuUuUuUuuuUu\n\
            uUUUUUuUUUuuuUuUuUuUUUUUu\n\
            uuuuuuuU.........Uuuuuuuu\n\
            UUUUUUUUuUUuUUUUuUUUUUUUU\n\
            uuuUuu.uuUUUuUUuUuuUUUuUU\n\
            UUUuUU.uUuUuuuUuUUuUUUuuu\n\
            UUUuUu.UUUuuuUuuuUuuUUuuu\n\
            UuuuUu.uuuuUuuuuUuuUuUUUU\n\
            UuUUUu.UuUuuUUuUUuuUUUUuu\n\
            UuUUuu.uUUuuUUuUuuuUUUuuu\n\
            uUUuuu.uuuuUUuUuuuUuUUuuu\n\
            UuUUUU.uuUuuUUUuUuUUUUUuU\n\
            uUUuUU.UUUuUuUUUuuuuuUUUU\n\
            UUUUUUUUuuuuuuUuuUUUuUuuU\n\
            uuuuuuuUuuuuuUuuuUuUuuUuu\n\
            uUUUUUuUuuuUuuuuuUUUuuUuu\n\
            uUuuuUuUuUuuUUuUuuuuuUUUu\n\
            uUuuuUuUUUuuUUuUUUUUuUuUU\n\
            uUuuuUuUuuUUUuUUuUUuuUuUu\n\
            uUUUUUuUuUuuUUUuuUUuUuUuU\n\
            uuuuuuuUuuUUuUUUuUuUuUUuu\n"
        );
    }
}

// Alignment pattern
//------------------------------------------------------------------------------

impl DeQR {
    pub fn mark_alignment_patterns(&mut self) {
        let positions = self.version.alignment_pattern();
        for &r in positions {
            for &c in positions {
                self.mark_alignment_pattern_at(r, c);
            }
        }
    }

    fn mark_alignment_pattern_at(&mut self, r: i16, c: i16) {
        let w = self.width as i16;
        if (r == 6 && (c == 6 || c - w == -7)) || (r - w == -7 && c == 6) {
            return;
        }
        for i in -2..=2 {
            for j in -2..=2 {
                self.set(r + i, c + j, DeModule::Marked);
            }
        }
    }
}

#[cfg(test)]
mod deqr_alignement_tests {
    use super::DeQR;
    use crate::{
        builder::QRBuilder,
        common::metadata::{ECLevel, Version},
    };

    #[test]
    fn test_mark_alignment_pattern() {
        let data = "Hello, world! 🌎";
        let version = Version::Normal(2);
        let ec_level = ECLevel::L;

        let qr =
            QRBuilder::new(data.as_bytes()).version(version).ec_level(ec_level).build().unwrap();
        let qr_str = qr.to_str(1);

        let mut deqr = DeQR::from_str(&qr_str, version);
        deqr.mark_alignment_patterns();

        assert_eq!(
            deqr.to_debug_str(),
            "\n\
            uuuuuuuUUUuUUuuUuUuuuuuuu\n\
            uUUUUUuUUuUUUUuUUUuUUUUUu\n\
            uUuuuUuUuUUuUUuUUUuUuuuUu\n\
            uUuuuUuUUuUUuuUUUUuUuuuUu\n\
            uUuuuUuUUuUUuuuUuUuUuuuUu\n\
            uUUUUUuUUUuuuUuUuUuUUUUUu\n\
            uuuuuuuUuUuUuUuUuUuuuuuuu\n\
            UUUUUUUUuUUuUUUUuUUUUUUUU\n\
            uuuUuuuuuUUUuUUuUuuUUUuUU\n\
            UUUuUUUuUuUuuuUuUUuUUUuuu\n\
            UUUuUuuUUUuuuUuuuUuuUUuuu\n\
            UuuuUuUuuuuUuuuuUuuUuUUUU\n\
            UuUUUuuUuUuuUUuUUuuUUUUuu\n\
            UuUUuuUuUUuuUUuUuuuUUUuuu\n\
            uUUuuuuuuuuUUuUuuuUuUUuuu\n\
            UuUUUUUuuUuuUUUuUuUUUUUuU\n\
            uUUuUUuUUUuUuUUU.....UUUU\n\
            UUUUUUUUuuuuuuUu.....UuuU\n\
            uuuuuuuUuuuuuUuu.....uUuu\n\
            uUUUUUuUuuuUuuuu.....uUuu\n\
            uUuuuUuUuUuuUUuU.....UUUu\n\
            uUuuuUuUUUuuUUuUUUUUuUuUU\n\
            uUuuuUuUuuUUUuUUuUUuuUuUu\n\
            uUUUUUuUuUuuUUUuuUUuUuUuU\n\
            uuuuuuuUuuUUuUUUuUuUuUUuu\n"
        );
    }
}

// Unmask
//------------------------------------------------------------------------------

impl DeQR {
    pub fn unmask(&mut self, pattern: MaskPattern) {
        let mask_function = pattern.mask_functions();
        let w = self.width as i16;
        for r in 0..w {
            for c in 0..w {
                if mask_function(r, c) {
                    self.set(r, c, !self.get(r, c))
                }
            }
        }
    }
}

// Encoding region
//------------------------------------------------------------------------------

impl DeQR {
    pub fn extract_payload(&mut self, version: Version) -> Vec<u8> {
        let total_codewords = version.total_codewords();
        let mut codewords = Vec::with_capacity(total_codewords);
        let mut coords = EncRegionIter::new(version);
        for _ in 0..total_codewords {
            let mut codeword = 0;
            for _ in 0..8 {
                for (r, c) in coords.by_ref() {
                    if matches!(self.get(r, c), DeModule::Unmarked(_)) {
                        codeword = (codeword << 1) | u8::from(*self.get(r, c));
                        break;
                    }
                }
            }
            codewords.push(codeword);
        }
        codewords
    }
}
