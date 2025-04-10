use image::{GrayImage, Luma, Rgb, RgbImage};
use std::ops::{Deref, Not};

use crate::ec::rectify_info;
use crate::metadata::*;
use crate::utils::{BitArray, EncRegionIter, QRError, QRResult};
use crate::MaskPattern;

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
    w: usize,
    grid: Box<[DeModule; MAX_QR_SIZE]>,
    ver: Version,
    ecl: Option<ECLevel>,
    mask: Option<MaskPattern>,
}

impl DeQR {
    pub fn from_clr_img(qr: &RgbImage, ver: Version) -> Self {
        let qr_w = ver.width();
        let (w, h) = qr.dimensions();
        let (w, h) = (w as i16, h as i16);
        let qz_sz = if let Version::Normal(_) = ver { 4 } else { 2 };
        let mod_w = w / (qr_w + 2 * qz_sz) as i16;
        let qz_w = qz_sz as i16 * mod_w;

        debug_assert!(w == h, "Image is not perfect square");
        let img_w = w - 2 * qz_w;
        debug_assert!(
            img_w % qr_w as i16 == 0,
            "Image w {img_w} is not a multiple of qr size {qr_w}"
        );

        let thresh = (mod_w * mod_w * 255 / 2) as u32;

        let mut clr_grid = vec![(0u32, 0u32, 0u32); qr_w * qr_w];
        for (c, r, pixel) in qr.enumerate_pixels() {
            let (r, c) = (r as i16, c as i16);
            if r < qz_w || r >= w - qz_w || c < qz_w || c >= w - qz_w {
                continue;
            }
            let idx = Self::coord_to_index((r - qz_w) / mod_w, (c - qz_w) / mod_w, qr_w);
            let Rgb([r, g, b]) = *pixel;
            clr_grid[idx].0 += r as u32;
            clr_grid[idx].1 += g as u32;
            clr_grid[idx].2 += b as u32;
        }

        let mut grid = Box::new([DeModule::Marked; MAX_QR_SIZE]);
        clr_grid.iter().enumerate().for_each(|(i, &m)| {
            let r = if m.0 < thresh { 0 } else { 255 };
            let g = if m.1 < thresh { 0 } else { 255 };
            let b = if m.2 < thresh { 0 } else { 255 };
            grid[i] = DeModule::Unmarked(Color::Hue(r, g, b));
        });

        Self { w: qr_w, grid, ver, ecl: None, mask: None }
    }

    pub fn from_image(qr: &GrayImage, ver: Version) -> Self {
        let qr_w = ver.width();
        let (w, h) = qr.dimensions();
        let (w, h) = (w as i16, h as i16);
        let mod_sz = w / qr_w as i16;
        let qz_sz = if let Version::Normal(_) = ver { 4 } else { 2 } * mod_sz;

        debug_assert!(w == h, "Image is not perfect square");
        debug_assert!((w - 2 * qz_sz) % qr_w as i16 == 0, "Image w is not a multiple of qr size");

        let half_area = mod_sz * mod_sz / 2;

        let mut black_cnt = vec![0; qr_w * qr_w];
        for (c, r, pixel) in qr.enumerate_pixels() {
            let (r, c) = (r as i16, c as i16);
            if r < qz_sz || r >= w - qz_sz || c < qz_sz || c >= w - qz_sz {
                continue;
            }
            let index = Self::coord_to_index((r - qz_sz) / mod_sz, (c - qz_sz) / mod_sz, qr_w);
            let Luma([luma]) = *pixel;
            black_cnt[index] += if luma < 128 { 1 } else { 0 };
        }

        let mut grid = Box::new([DeModule::Marked; MAX_QR_SIZE]);
        black_cnt.iter().enumerate().for_each(|(i, &bc)| {
            grid[i] = DeModule::Unmarked(if bc > half_area { Color::Dark } else { Color::Light })
        });
        Self { w: qr_w, grid, ver, ecl: None, mask: None }
    }

    pub fn from_str(qr: &str, ver: Version) -> Self {
        let qr_w = ver.width();
        let qz_sz = if let Version::Normal(_) = ver { 4 } else { 2 };
        let full_w = qz_sz + qr_w + qz_sz;

        let mut grid = Box::new([DeModule::Marked; MAX_QR_SIZE]);
        qr.chars()
            .filter(|clr| *clr != '\n')
            .enumerate()
            .filter(|(i, _)| {
                let (r, c) = (i / full_w, i % full_w);
                r >= qz_sz && r < qz_sz + qr_w && c >= qz_sz && c < qz_sz + qr_w
            })
            .enumerate()
            .for_each(|(i, (_, clr))| {
                grid[i] = DeModule::Unmarked(if clr == ' ' { Color::Dark } else { Color::Light })
            });

        Self { w: qr_w, grid, ver, ecl: None, mask: None }
    }

    pub fn metadata(&self) -> Metadata {
        Metadata::new(Some(self.ver), self.ecl, self.mask)
    }

    pub fn count_dark_modules(&self) -> usize {
        self.grid.iter().filter(|&m| matches!(**m, Color::Dark)).count()
    }

    #[cfg(test)]
    pub fn to_debug_str(&self) -> String {
        let w = self.w as i16;
        let mut res = String::with_capacity((w * (w + 1)) as usize);
        res.push('\n');
        for i in 0..w {
            for j in 0..w {
                let c = match self.get(i, j) {
                    DeModule::Unmarked(Color::Dark) => 'u',
                    DeModule::Unmarked(Color::Light | Color::Hue(..)) => 'U',
                    DeModule::Marked => '.',
                };
                res.push(c);
            }
            res.push('\n');
        }
        res
    }

    fn coord_to_index(r: i16, c: i16, w: usize) -> usize {
        let w = w as i16;
        debug_assert!(-w <= r && r < w, "row should be greater than or equal to w");
        debug_assert!(-w <= c && c < w, "column should be greater than or equal to w");

        let r = if r < 0 { r + w } else { r };
        let c = if c < 0 { c + w } else { c };
        (r * w + c) as _
    }

    pub fn get(&self, r: i16, c: i16) -> DeModule {
        self.grid[Self::coord_to_index(r, c, self.w)]
    }

    pub fn get_mut(&mut self, r: i16, c: i16) -> &mut DeModule {
        let idx = Self::coord_to_index(r, c, self.w);
        &mut self.grid[idx]
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
        let ver = Version::Normal(2);
        let sz = ver.width() as i16;
        let ecl = ECLevel::L;

        let qr = QRBuilder::new(data.as_bytes()).version(ver).ec_level(ecl).build().unwrap();
        let qr_str = qr.to_str(1);

        let deqr = DeQR::from_str(&qr_str, ver);

        for r in 0..sz {
            for c in 0..sz {
                assert_eq!(*qr.get(r, c), *deqr.get(r, c), "{r} {c}");
            }
        }
    }

    #[test]
    fn test_from_image() {
        let data = "Hello, world! 🌎";
        let ver = Version::Normal(2);
        let sz = ver.width() as i16;
        let ecl = ECLevel::L;

        let qr = QRBuilder::new(data.as_bytes()).version(ver).ec_level(ecl).build().unwrap();
        let qr_str = qr.render(1);

        let deqr = DeQR::from_image(&qr_str, ver);

        for r in 0..sz {
            for c in 0..sz {
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
        let (ecl, mask) = parse_format_info_qr(f);
        self.ecl = Some(ecl);
        self.mask = Some(mask);
        Ok((ecl, mask))
    }

    pub fn read_version_info(&mut self) -> QRResult<Version> {
        debug_assert!(
            !matches!(self.ver, Version::Micro(_) | Version::Normal(1..=6)),
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
        let mut num = 0;
        for (r, c) in coords {
            let m = self.get_mut(*r, *c);
            num = (num << 1) | u32::from(**m);
        }
        num
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
    use crate::metadata::{Color, ECLevel, Version};
    use crate::MaskPattern;

    #[test]
    fn test_read_format_info() {
        let data = "Hello, world! 🌎";
        let ver = Version::Normal(2);
        let ecl = ECLevel::L;
        let mask = MaskPattern::new(1);

        let qr =
            QRBuilder::new(data.as_bytes()).version(ver).ec_level(ecl).mask(mask).build().unwrap();
        let qr_str = qr.to_str(1);

        let mut deqr = DeQR::from_str(&qr_str, ver);

        let fmt_info = deqr.read_format_info().unwrap();
        assert_eq!(fmt_info, (ecl, mask));
    }

    #[test]
    fn test_read_format_info_one_corrupted() {
        let data = "Hello, world! 🌎";
        let ver = Version::Normal(2);
        let ecl = ECLevel::L;
        let mask = MaskPattern::new(1);

        let mut qr =
            QRBuilder::new(data.as_bytes()).version(ver).ec_level(ecl).mask(mask).build().unwrap();
        qr.set(8, 1, Module::Format(Color::Light));
        qr.set(8, 2, Module::Format(Color::Light));
        qr.set(8, 4, Module::Format(Color::Dark));
        let qr_str = qr.to_str(1);

        let mut deqr = DeQR::from_str(&qr_str, ver);

        let fmt_info = deqr.read_format_info().unwrap();
        assert_eq!(fmt_info, (ecl, mask));
    }

    #[test]
    fn test_read_format_info_one_fully_corrupted() {
        let data = "Hello, world! 🌎";
        let ver = Version::Normal(2);
        let ecl = ECLevel::L;
        let mask = MaskPattern::new(1);

        let mut qr =
            QRBuilder::new(data.as_bytes()).version(ver).ec_level(ecl).mask(mask).build().unwrap();
        qr.set(8, 1, Module::Format(Color::Light));
        qr.set(8, 2, Module::Format(Color::Light));
        qr.set(8, 3, Module::Format(Color::Dark));
        qr.set(8, 4, Module::Format(Color::Dark));
        let qr_str = qr.to_str(1);

        let mut deqr = DeQR::from_str(&qr_str, ver);

        let fmt_info = deqr.read_format_info().unwrap();
        assert_eq!(fmt_info, (ecl, mask));
    }

    #[test]
    #[should_panic]
    fn test_read_format_info_both_fully_corrupted() {
        let data = "Hello, world! 🌎";
        let ver = Version::Normal(2);
        let ecl = ECLevel::L;
        let mask = MaskPattern::new(1);

        let mut qr =
            QRBuilder::new(data.as_bytes()).version(ver).ec_level(ecl).mask(mask).build().unwrap();
        qr.set(8, 1, Module::Format(Color::Light));
        qr.set(8, 2, Module::Format(Color::Light));
        qr.set(8, 3, Module::Format(Color::Dark));
        qr.set(8, 4, Module::Format(Color::Dark));
        qr.set(-2, 8, Module::Format(Color::Light));
        qr.set(-3, 8, Module::Format(Color::Light));
        qr.set(-4, 8, Module::Format(Color::Dark));
        qr.set(-5, 8, Module::Format(Color::Dark));
        let qr_str = qr.to_str(1);

        let mut deqr = DeQR::from_str(&qr_str, ver);

        let fmt_info = deqr.read_format_info().unwrap();
        assert_eq!(fmt_info, (ecl, mask));
    }

    #[test]
    fn test_mark_format_info() {
        let data = "Hello, world! 🌎";
        let ver = Version::Normal(2);
        let ecl = ECLevel::L;

        let qr = QRBuilder::new(data.as_bytes()).version(ver).ec_level(ecl).build().unwrap();
        let qr_str = qr.to_str(1);

        let mut deqr = DeQR::from_str(&qr_str, ver);
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
        let ver = Version::Normal(7);
        let ecl = ECLevel::L;

        let qr = QRBuilder::new(data.as_bytes()).version(ver).ec_level(ecl).build().unwrap();
        let qr_str = qr.to_str(1);

        let mut deqr = DeQR::from_str(&qr_str, ver);

        let ver_info = deqr.read_version_info().unwrap();
        assert_eq!(ver_info, ver);
    }

    #[test]
    fn test_read_version_info_one_corrupted() {
        let data = "Hello, world! 🌎";
        let ver = Version::Normal(7);
        let ecl = ECLevel::L;

        let mut qr = QRBuilder::new(data.as_bytes()).version(ver).ec_level(ecl).build().unwrap();
        qr.set(-9, 5, Module::Format(Color::Dark));
        qr.set(-10, 5, Module::Format(Color::Dark));
        qr.set(-11, 5, Module::Format(Color::Dark));
        let qr_str = qr.to_str(1);

        let mut deqr = DeQR::from_str(&qr_str, ver);

        let ver_info = deqr.read_version_info().unwrap();
        assert_eq!(ver_info, ver);
    }

    #[test]
    fn test_read_version_info_one_fully_corrupted() {
        let data = "Hello, world! 🌎";
        let ver = Version::Normal(7);
        let ecl = ECLevel::L;

        let mut qr = QRBuilder::new(data.as_bytes()).version(ver).ec_level(ecl).build().unwrap();
        qr.set(-9, 5, Module::Format(Color::Dark));
        qr.set(-10, 5, Module::Format(Color::Dark));
        qr.set(-11, 5, Module::Format(Color::Dark));
        qr.set(-9, 4, Module::Format(Color::Light));
        let qr_str = qr.to_str(1);

        let mut deqr = DeQR::from_str(&qr_str, ver);

        let ver_info = deqr.read_version_info().unwrap();
        assert_eq!(ver_info, ver);
    }

    #[test]
    #[should_panic]
    fn test_read_version_info_both_fully_corrupted() {
        let data = "Hello, world! 🌎";
        let ver = Version::Normal(7);
        let ecl = ECLevel::L;

        let mut qr = QRBuilder::new(data.as_bytes()).version(ver).ec_level(ecl).build().unwrap();
        qr.set(-9, 5, Module::Format(Color::Dark));
        qr.set(-10, 5, Module::Format(Color::Dark));
        qr.set(-11, 5, Module::Format(Color::Dark));
        qr.set(-9, 4, Module::Format(Color::Light));
        qr.set(5, -9, Module::Format(Color::Dark));
        qr.set(5, -10, Module::Format(Color::Dark));
        qr.set(5, -11, Module::Format(Color::Dark));
        qr.set(4, -9, Module::Format(Color::Light));
        let qr_str = qr.to_str(1);

        let mut deqr = DeQR::from_str(&qr_str, ver);

        let _ = deqr.read_version_info().unwrap();
    }

    #[test]
    fn test_mark_version_info() {
        let data = "Hello, world! 🌎";
        let ver = Version::Normal(7);
        let ecl = ECLevel::L;

        let qr = QRBuilder::new(data.as_bytes()).version(ver).ec_level(ecl).build().unwrap();
        let qr_str = qr.to_str(1);

        let mut deqr = DeQR::from_str(&qr_str, ver);
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
        let ver = Version::Normal(7);
        let ecl = ECLevel::L;

        let qr = QRBuilder::new(data.as_bytes()).version(ver).ec_level(ecl).build().unwrap();
        let qr_str = qr.to_str(1);

        let mut deqr = DeQR::from_str(&qr_str, ver);
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
        match self.ver {
            Version::Micro(_) => {}
            Version::Normal(_) => {
                self.mark_finder_pattern_at(3, -4);
                self.mark_finder_pattern_at(-4, 3);
            }
        }
    }

    fn mark_finder_pattern_at(&mut self, r: i16, c: i16) {
        let (dr_l, dr_r) = if r > 0 { (-3, 4) } else { (-4, 3) };
        let (dc_t, dc_b) = if c > 0 { (-3, 4) } else { (-4, 3) };
        for i in dr_l..=dr_r {
            for j in dc_t..=dc_b {
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
        let ver = Version::Normal(2);
        let ecl = ECLevel::L;

        let qr = QRBuilder::new(data.as_bytes()).version(ver).ec_level(ecl).build().unwrap();
        let qr_str = qr.to_str(1);

        let mut deqr = DeQR::from_str(&qr_str, ver);
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
        let w = self.w as i16;
        let (off, last) = match self.ver {
            Version::Micro(_) => (0, w - 1),
            Version::Normal(_) => (6, w - 9),
        };
        self.mark_line(off, 8, off, last);
        self.mark_line(8, off, last, off);
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
        let ver = Version::Normal(2);
        let ecl = ECLevel::L;

        let qr = QRBuilder::new(data.as_bytes()).version(ver).ec_level(ecl).build().unwrap();
        let qr_str = qr.to_str(1);

        let mut deqr = DeQR::from_str(&qr_str, ver);
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
        let poses = self.ver.alignment_pattern();
        for &r in poses {
            for &c in poses {
                self.mark_alignment_pattern_at(r, c);
            }
        }
    }

    fn mark_alignment_pattern_at(&mut self, r: i16, c: i16) {
        let w = self.w as i16;
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
        let ver = Version::Normal(2);
        let ecl = ECLevel::L;

        let qr = QRBuilder::new(data.as_bytes()).version(ver).ec_level(ecl).build().unwrap();
        let qr_str = qr.to_str(1);

        let mut deqr = DeQR::from_str(&qr_str, ver);
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
        let mask_fn = pattern.mask_functions();
        let w = self.w as i16;
        for r in 0..w {
            for c in 0..w {
                if mask_fn(r, c) {
                    self.set(r, c, !self.get(r, c))
                }
            }
        }
    }
}

// Encoding region
//------------------------------------------------------------------------------

impl DeQR {
    // TODO: Write testcases
    pub fn extract_payload(&mut self, ver: Version) -> BitArray {
        let chan_bits = ver.channel_codewords() << 3;
        let (g_off, b_off) = (chan_bits, 2 * chan_bits);
        let mut pld = BitArray::new(chan_bits * 3);
        let mut rgn_iter = EncRegionIter::new(ver);

        for i in 0..chan_bits {
            for (r, c) in rgn_iter.by_ref() {
                if let DeModule::Unmarked(clr) = self.get(r, c) {
                    let (r, g, b) = match clr {
                        Color::Light => (false, false, false),
                        Color::Dark => (true, true, true),
                        Color::Hue(r, g, b) => (r != 255, g != 255, b != 255),
                    };
                    pld.put(i, r);
                    pld.put(i + g_off, g);
                    pld.put(i + b_off, b);
                    break;
                }
            }
        }
        pld
    }
}
