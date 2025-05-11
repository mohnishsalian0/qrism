use core::panic;
use image::{GrayImage, Luma, Rgb, RgbImage};
use std::ops::Deref;

use crate::metadata::*;
use crate::utils::{BitStream, EncRegionIter};
use crate::MaskPattern;

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum Module {
    Empty,
    Func(Color),
    Version(Color),
    Format(Color),
    Data(Color),
}

impl Deref for Module {
    type Target = Color;
    fn deref(&self) -> &Self::Target {
        match self {
            Module::Empty => &Color::White,
            Module::Func(c) => c,
            Module::Version(c) => c,
            Module::Format(c) => c,
            Module::Data(c) => c,
        }
    }
}

#[derive(Debug, Clone)]
pub struct QR {
    grid: Box<[Module; MAX_QR_SIZE]>,
    w: usize,
    ver: Version,
    ecl: ECLevel,
    pal: Palette,
    mask: Option<MaskPattern>,
}

// QR type for builder
//------------------------------------------------------------------------------

impl QR {
    pub fn new(ver: Version, ecl: ECLevel, pal: Palette) -> Self {
        debug_assert!(
            matches!(ver, Version::Micro(1..=4) | Version::Normal(1..=40)),
            "Invalid version"
        );

        let w = ver.width();
        Self { grid: Box::new([Module::Empty; MAX_QR_SIZE]), w, ver, ecl, pal, mask: None }
    }

    pub fn grid(&self) -> &[Module] {
        &*self.grid
    }

    pub fn version(&self) -> Version {
        self.ver
    }

    pub fn width(&self) -> usize {
        self.w
    }

    pub fn ec_level(&self) -> ECLevel {
        self.ecl
    }

    pub fn palette(&self) -> Palette {
        self.pal
    }

    pub fn mask(&self) -> Option<MaskPattern> {
        self.mask
    }

    pub fn metadata(&self) -> Metadata {
        Metadata::new(Some(self.ver), Some(self.ecl), self.mask)
    }

    pub fn count_dark_modules(&self) -> usize {
        self.grid.iter().filter(|&m| matches!(**m, Color::Black)).count()
    }

    #[cfg(test)]
    pub fn to_debug_str(&self) -> String {
        let w = self.w as i32;
        let mut res = String::with_capacity((w * (w + 1)) as usize);
        res.push('\n');
        for i in 0..w {
            for j in 0..w {
                let c = match self.get(i, j) {
                    Module::Empty => '.',
                    Module::Func(Color::Black) => 'f',
                    Module::Func(_) => 'F',
                    Module::Version(Color::Black) => 'v',
                    Module::Version(_) => 'V',
                    Module::Format(Color::Black) => 'm',
                    Module::Format(_) => 'M',
                    Module::Data(Color::Black) => 'd',
                    Module::Data(_) => 'D',
                };
                res.push(c);
            }
            res.push('\n');
        }
        res
    }

    fn coord_to_index(&self, r: i32, c: i32) -> usize {
        let w = self.w as i32;
        debug_assert!(-w <= r && r < w, "row should be greater than or equal to w");
        debug_assert!(-w <= c && c < w, "column should be greater than or equal to w");

        let r = if r < 0 { r + w } else { r };
        let c = if c < 0 { c + w } else { c };
        (r * w + c) as _
    }

    pub fn get(&self, r: i32, c: i32) -> Module {
        self.grid[self.coord_to_index(r, c)]
    }

    pub fn get_mut(&mut self, r: i32, c: i32) -> &mut Module {
        let index = self.coord_to_index(r, c);
        &mut self.grid[index]
    }

    pub fn set(&mut self, r: i32, c: i32, module: Module) {
        *self.get_mut(r, c) = module;
    }
}

#[cfg(test)]
mod qr_util_tests {
    use crate::builder::{Module, QR};
    use crate::common::metadata::{Color, ECLevel, Palette, Version};

    #[test]
    fn test_index_wrap() {
        let mut qr = QR::new(Version::Normal(1), ECLevel::L, Palette::Mono);
        let w = qr.w as i32;
        qr.set(-1, -1, Module::Func(Color::Black));
        assert_eq!(qr.get(w - 1, w - 1), Module::Func(Color::Black));
        qr.set(0, 0, Module::Func(Color::Black));
        assert_eq!(qr.get(-w, -w), Module::Func(Color::Black));
    }

    #[test]
    #[should_panic]
    fn test_row_out_of_bound() {
        let qr = QR::new(Version::Normal(1), ECLevel::L, Palette::Mono);
        let w = qr.w as i32;
        qr.get(w, 0);
    }

    #[test]
    #[should_panic]
    fn test_col_out_of_bound() {
        let qr = QR::new(Version::Normal(1), ECLevel::L, Palette::Mono);
        let w = qr.w as i32;
        qr.get(0, w);
    }

    #[test]
    #[should_panic]
    fn test_row_index_overwrap() {
        let qr = QR::new(Version::Normal(1), ECLevel::L, Palette::Mono);
        let w = qr.w as i32;
        qr.get(-(w + 1), 0);
    }

    #[test]
    #[should_panic]
    fn test_col_index_overwrap() {
        let qr = QR::new(Version::Normal(1), ECLevel::L, Palette::Mono);
        let w = qr.w as i32;
        qr.get(0, -(w + 1));
    }
}

// Finder pattern
//------------------------------------------------------------------------------

impl QR {
    fn draw_finder_patterns(&mut self) {
        self.draw_finder_pattern_at(3, 3);
        match self.ver {
            Version::Micro(_) => {}
            Version::Normal(_) => {
                self.draw_finder_pattern_at(3, -4);
                self.draw_finder_pattern_at(-4, 3);
            }
        }
    }

    fn draw_finder_pattern_at(&mut self, r: i32, c: i32) {
        let (dr_left, dr_right) = if r > 0 { (-3, 4) } else { (-4, 3) };
        let (dc_top, dc_bottom) = if c > 0 { (-3, 4) } else { (-4, 3) };
        for i in dr_left..=dr_right {
            for j in dc_top..=dc_bottom {
                self.set(
                    r + i,
                    c + j,
                    match (i, j) {
                        (4 | -4, _) | (_, 4 | -4) => Module::Func(Color::White),
                        (3 | -3, _) | (_, 3 | -3) => Module::Func(Color::Black),
                        (2 | -2, _) | (_, 2 | -2) => Module::Func(Color::White),
                        _ => Module::Func(Color::Black),
                    },
                );
            }
        }
    }
}

#[cfg(test)]
mod finder_pattern_tests {
    use crate::builder::QR;
    use crate::common::metadata::{ECLevel, Palette, Version};

    #[test]
    fn test_finder_pattern_qr() {
        let mut qr = QR::new(Version::Normal(1), ECLevel::L, Palette::Mono);
        qr.draw_finder_patterns();
        assert_eq!(
            qr.to_debug_str(),
            "\n\
             fffffffF.....Ffffffff\n\
             fFFFFFfF.....FfFFFFFf\n\
             fFfffFfF.....FfFfffFf\n\
             fFfffFfF.....FfFfffFf\n\
             fFfffFfF.....FfFfffFf\n\
             fFFFFFfF.....FfFFFFFf\n\
             fffffffF.....Ffffffff\n\
             FFFFFFFF.....FFFFFFFF\n\
             .....................\n\
             .....................\n\
             .....................\n\
             .....................\n\
             .....................\n\
             FFFFFFFF.............\n\
             fffffffF.............\n\
             fFFFFFfF.............\n\
             fFfffFfF.............\n\
             fFfffFfF.............\n\
             fFfffFfF.............\n\
             fFFFFFfF.............\n\
             fffffffF.............\n"
        );
    }
}

// Timing pattern
//------------------------------------------------------------------------------

impl QR {
    fn draw_timing_pattern(&mut self) {
        let w = self.w as i32;
        let (off, last) = match self.ver {
            Version::Micro(_) => (0, w - 1),
            Version::Normal(_) => (6, w - 9),
        };
        self.draw_line(off, 8, off, last);
        self.draw_line(8, off, last, off);
    }

    fn draw_line(&mut self, r1: i32, c1: i32, r2: i32, c2: i32) {
        debug_assert!(r1 == r2 || c1 == c2, "Line is neither vertical nor horizontal");

        if r1 == r2 {
            for j in c1..=c2 {
                let m = if j & 1 == 0 { Color::Black } else { Color::White };
                self.set(r1, j, Module::Func(m));
            }
        } else {
            for i in r1..=r2 {
                let m = if i & 1 == 0 { Color::Black } else { Color::White };
                self.set(i, c1, Module::Func(m));
            }
        }
    }
}

#[cfg(test)]
mod timing_pattern_tests {
    use crate::builder::QR;
    use crate::common::metadata::{ECLevel, Palette, Version};

    #[test]
    fn test_timing_pattern_1() {
        let mut qr = QR::new(Version::Normal(1), ECLevel::L, Palette::Mono);
        qr.draw_timing_pattern();
        assert_eq!(
            qr.to_debug_str(),
            "\n\
             .....................\n\
             .....................\n\
             .....................\n\
             .....................\n\
             .....................\n\
             .....................\n\
             ........fFfFf........\n\
             .....................\n\
             ......f..............\n\
             ......F..............\n\
             ......f..............\n\
             ......F..............\n\
             ......f..............\n\
             .....................\n\
             .....................\n\
             .....................\n\
             .....................\n\
             .....................\n\
             .....................\n\
             .....................\n\
             .....................\n"
        );
    }
}

// Alignment pattern
//------------------------------------------------------------------------------

impl QR {
    fn draw_alignment_patterns(&mut self) {
        let poses = self.ver.alignment_pattern();
        for &r in poses {
            for &c in poses {
                self.draw_alignment_pattern_at(r, c)
            }
        }
    }

    fn draw_alignment_pattern_at(&mut self, r: i32, c: i32) {
        let w = self.w as i32;
        if (r == 6 && (c == 6 || c - w == -7)) || (r - w == -7 && c == 6) {
            return;
        }
        for i in -2..=2 {
            for j in -2..=2 {
                self.set(
                    r + i,
                    c + j,
                    match (i, j) {
                        (-2 | 2, _) | (_, -2 | 2) | (0, 0) => Module::Func(Color::Black),
                        _ => Module::Func(Color::White),
                    },
                )
            }
        }
    }
}

#[cfg(test)]
mod alignment_pattern_tests {
    use crate::builder::QR;
    use crate::common::metadata::{ECLevel, Palette, Version};

    #[test]
    fn test_alignment_pattern_1() {
        let mut qr = QR::new(Version::Normal(1), ECLevel::L, Palette::Mono);
        qr.draw_finder_patterns();
        qr.draw_alignment_patterns();
        assert_eq!(
            qr.to_debug_str(),
            "\n\
             fffffffF.....Ffffffff\n\
             fFFFFFfF.....FfFFFFFf\n\
             fFfffFfF.....FfFfffFf\n\
             fFfffFfF.....FfFfffFf\n\
             fFfffFfF.....FfFfffFf\n\
             fFFFFFfF.....FfFFFFFf\n\
             fffffffF.....Ffffffff\n\
             FFFFFFFF.....FFFFFFFF\n\
             .....................\n\
             .....................\n\
             .....................\n\
             .....................\n\
             .....................\n\
             FFFFFFFF.............\n\
             fffffffF.............\n\
             fFFFFFfF.............\n\
             fFfffFfF.............\n\
             fFfffFfF.............\n\
             fFfffFfF.............\n\
             fFFFFFfF.............\n\
             fffffffF.............\n"
        );
    }

    #[test]
    fn test_alignment_pattern_3() {
        let mut qr = QR::new(Version::Normal(3), ECLevel::L, Palette::Mono);
        qr.draw_finder_patterns();
        qr.draw_alignment_patterns();
        assert_eq!(
            qr.to_debug_str(),
            "\n\
             fffffffF.............Ffffffff\n\
             fFFFFFfF.............FfFFFFFf\n\
             fFfffFfF.............FfFfffFf\n\
             fFfffFfF.............FfFfffFf\n\
             fFfffFfF.............FfFfffFf\n\
             fFFFFFfF.............FfFFFFFf\n\
             fffffffF.............Ffffffff\n\
             FFFFFFFF.............FFFFFFFF\n\
             .............................\n\
             .............................\n\
             .............................\n\
             .............................\n\
             .............................\n\
             .............................\n\
             .............................\n\
             .............................\n\
             .............................\n\
             .............................\n\
             .............................\n\
             .............................\n\
             ....................fffff....\n\
             FFFFFFFF............fFFFf....\n\
             fffffffF............fFfFf....\n\
             fFFFFFfF............fFFFf....\n\
             fFfffFfF............fffff....\n\
             fFfffFfF.....................\n\
             fFfffFfF.....................\n\
             fFFFFFfF.....................\n\
             fffffffF.....................\n"
        );
    }

    #[test]
    fn test_alignment_pattern_7() {
        let mut qr = QR::new(Version::Normal(7), ECLevel::L, Palette::Mono);
        qr.draw_finder_patterns();
        qr.draw_alignment_patterns();
        assert_eq!(
            qr.to_debug_str(),
            "\n\
             fffffffF.............................Ffffffff\n\
             fFFFFFfF.............................FfFFFFFf\n\
             fFfffFfF.............................FfFfffFf\n\
             fFfffFfF.............................FfFfffFf\n\
             fFfffFfF............fffff............FfFfffFf\n\
             fFFFFFfF............fFFFf............FfFFFFFf\n\
             fffffffF............fFfFf............Ffffffff\n\
             FFFFFFFF............fFFFf............FFFFFFFF\n\
             ....................fffff....................\n\
             .............................................\n\
             .............................................\n\
             .............................................\n\
             .............................................\n\
             .............................................\n\
             .............................................\n\
             .............................................\n\
             .............................................\n\
             .............................................\n\
             .............................................\n\
             .............................................\n\
             ....fffff...........fffff...........fffff....\n\
             ....fFFFf...........fFFFf...........fFFFf....\n\
             ....fFfFf...........fFfFf...........fFfFf....\n\
             ....fFFFf...........fFFFf...........fFFFf....\n\
             ....fffff...........fffff...........fffff....\n\
             .............................................\n\
             .............................................\n\
             .............................................\n\
             .............................................\n\
             .............................................\n\
             .............................................\n\
             .............................................\n\
             .............................................\n\
             .............................................\n\
             .............................................\n\
             .............................................\n\
             ....................fffff...........fffff....\n\
             FFFFFFFF............fFFFf...........fFFFf....\n\
             fffffffF............fFfFf...........fFfFf....\n\
             fFFFFFfF............fFFFf...........fFFFf....\n\
             fFfffFfF............fffff...........fffff....\n\
             fFfffFfF.....................................\n\
             fFfffFfF.....................................\n\
             fFFFFFfF.....................................\n\
             fffffffF.....................................\n"
        );
    }
}

// ALl function patterns
//------------------------------------------------------------------------------

impl QR {
    pub fn draw_all_function_patterns(&mut self) {
        self.draw_finder_patterns();
        self.draw_timing_pattern();
        self.draw_alignment_patterns();
    }
}

#[cfg(test)]
mod all_function_patterns_test {
    use crate::builder::QR;
    use crate::common::metadata::{ECLevel, Palette, Version};

    #[test]
    fn test_all_function_patterns() {
        let mut qr = QR::new(Version::Normal(3), ECLevel::L, Palette::Mono);
        qr.draw_all_function_patterns();
        assert_eq!(
            qr.to_debug_str(),
            "\n\
             fffffffF.............Ffffffff\n\
             fFFFFFfF.............FfFFFFFf\n\
             fFfffFfF.............FfFfffFf\n\
             fFfffFfF.............FfFfffFf\n\
             fFfffFfF.............FfFfffFf\n\
             fFFFFFfF.............FfFFFFFf\n\
             fffffffFfFfFfFfFfFfFfFfffffff\n\
             FFFFFFFF.............FFFFFFFF\n\
             ......f......................\n\
             ......F......................\n\
             ......f......................\n\
             ......F......................\n\
             ......f......................\n\
             ......F......................\n\
             ......f......................\n\
             ......F......................\n\
             ......f......................\n\
             ......F......................\n\
             ......f......................\n\
             ......F......................\n\
             ......f.............fffff....\n\
             FFFFFFFF............fFFFf....\n\
             fffffffF............fFfFf....\n\
             fFFFFFfF............fFFFf....\n\
             fFfffFfF............fffff....\n\
             fFfffFfF.....................\n\
             fFfffFfF.....................\n\
             fFFFFFfF.....................\n\
             fffffffF.....................\n"
        );
    }
}

// Format & version info
//------------------------------------------------------------------------------

impl QR {
    fn reserve_format_area(&mut self) {
        self.draw_format_info((1 << FORMAT_INFO_BIT_LEN) - 1);
    }

    fn draw_format_info(&mut self, format_info: u32) {
        match self.ver {
            Version::Micro(_) => todo!(),
            Version::Normal(_) => {
                self.draw_number(
                    format_info,
                    FORMAT_INFO_BIT_LEN,
                    Module::Format(Color::White),
                    Module::Format(Color::Black),
                    &FORMAT_INFO_COORDS_QR_MAIN,
                );
                self.draw_number(
                    format_info,
                    FORMAT_INFO_BIT_LEN,
                    Module::Format(Color::White),
                    Module::Format(Color::Black),
                    &FORMAT_INFO_COORDS_QR_SIDE,
                );
                let unused_mod = if self.palette() == Palette::Mono {
                    Module::Format(Color::Black)
                } else {
                    Module::Format(Color::White)
                };
                self.set(-8, 8, unused_mod);
            }
        }
    }

    fn draw_version_info(&mut self) {
        match self.ver {
            Version::Micro(_) | Version::Normal(1..=6) => {}
            Version::Normal(7..=40) => {
                let ver_info = self.ver.info();
                self.draw_number(
                    ver_info,
                    VERSION_INFO_BIT_LEN,
                    Module::Version(Color::White),
                    Module::Version(Color::Black),
                    &VERSION_INFO_COORDS_BL,
                );
                self.draw_number(
                    ver_info,
                    VERSION_INFO_BIT_LEN,
                    Module::Version(Color::White),
                    Module::Version(Color::Black),
                    &VERSION_INFO_COORDS_TR,
                );
            }
            _ => unreachable!(),
        }
    }

    fn draw_number(
        &mut self,
        number: u32,
        bit_len: usize,
        off_clr: Module,
        on_clr: Module,
        coords: &[(i32, i32)],
    ) {
        let mut mask = 1 << (bit_len - 1);
        for (r, c) in coords {
            if number & mask == 0 {
                self.set(*r, *c, off_clr);
            } else {
                self.set(*r, *c, on_clr);
            }
            mask >>= 1;
        }
    }
}

#[cfg(test)]
mod qr_information_tests {
    use crate::builder::QR;
    use crate::common::metadata::{ECLevel, Palette, Version};

    #[test]
    fn test_version_info_1() {
        let mut qr = QR::new(Version::Normal(1), ECLevel::L, Palette::Mono);
        qr.draw_version_info();
        assert_eq!(
            qr.to_debug_str(),
            "\n\
             .....................\n\
             .....................\n\
             .....................\n\
             .....................\n\
             .....................\n\
             .....................\n\
             .....................\n\
             .....................\n\
             .....................\n\
             .....................\n\
             .....................\n\
             .....................\n\
             .....................\n\
             .....................\n\
             .....................\n\
             .....................\n\
             .....................\n\
             .....................\n\
             .....................\n\
             .....................\n\
             .....................\n"
        );
    }

    #[test]
    fn test_version_info_7() {
        let mut qr = QR::new(Version::Normal(7), ECLevel::L, Palette::Mono);
        qr.draw_version_info();
        assert_eq!(
            qr.to_debug_str(),
            "\n\
             ..................................VVv........\n\
             ..................................VvV........\n\
             ..................................VvV........\n\
             ..................................Vvv........\n\
             ..................................vvv........\n\
             ..................................VVV........\n\
             .............................................\n\
             .............................................\n\
             .............................................\n\
             .............................................\n\
             .............................................\n\
             .............................................\n\
             .............................................\n\
             .............................................\n\
             .............................................\n\
             .............................................\n\
             .............................................\n\
             .............................................\n\
             .............................................\n\
             .............................................\n\
             .............................................\n\
             .............................................\n\
             .............................................\n\
             .............................................\n\
             .............................................\n\
             .............................................\n\
             .............................................\n\
             .............................................\n\
             .............................................\n\
             .............................................\n\
             .............................................\n\
             .............................................\n\
             .............................................\n\
             .............................................\n\
             VVVVvV.......................................\n\
             VvvvvV.......................................\n\
             vVVvvV.......................................\n\
             .............................................\n\
             .............................................\n\
             .............................................\n\
             .............................................\n\
             .............................................\n\
             .............................................\n\
             .............................................\n\
             .............................................\n"
        );
    }

    #[test]
    fn test_reserve_format_info_qr() {
        let mut qr = QR::new(Version::Normal(1), ECLevel::L, Palette::Mono);
        qr.reserve_format_area();
        assert_eq!(
            qr.to_debug_str(),
            "\n\
             ........m............\n\
             ........m............\n\
             ........m............\n\
             ........m............\n\
             ........m............\n\
             ........m............\n\
             .....................\n\
             ........m............\n\
             mmmmmm.mm....mmmmmmmm\n\
             .....................\n\
             .....................\n\
             .....................\n\
             .....................\n\
             ........m............\n\
             ........m............\n\
             ........m............\n\
             ........m............\n\
             ........m............\n\
             ........m............\n\
             ........m............\n\
             ........m............\n"
        );
    }

    #[test]
    fn test_all_function_patterns_and_qr_info() {
        let mut qr = QR::new(Version::Normal(7), ECLevel::L, Palette::Mono);
        qr.draw_all_function_patterns();
        qr.draw_version_info();
        qr.reserve_format_area();
        assert_eq!(
            qr.to_debug_str(),
            "\n\
             fffffffFm.........................VVvFfffffff\n\
             fFFFFFfFm.........................VvVFfFFFFFf\n\
             fFfffFfFm.........................VvVFfFfffFf\n\
             fFfffFfFm.........................VvvFfFfffFf\n\
             fFfffFfFm...........fffff.........vvvFfFfffFf\n\
             fFFFFFfFm...........fFFFf.........VVVFfFFFFFf\n\
             fffffffFfFfFfFfFfFfFfFfFfFfFfFfFfFfFfFfffffff\n\
             FFFFFFFFm...........fFFFf............FFFFFFFF\n\
             mmmmmmfmm...........fffff............mmmmmmmm\n\
             ......F......................................\n\
             ......f......................................\n\
             ......F......................................\n\
             ......f......................................\n\
             ......F......................................\n\
             ......f......................................\n\
             ......F......................................\n\
             ......f......................................\n\
             ......F......................................\n\
             ......f......................................\n\
             ......F......................................\n\
             ....fffff...........fffff...........fffff....\n\
             ....fFFFf...........fFFFf...........fFFFf....\n\
             ....fFfFf...........fFfFf...........fFfFf....\n\
             ....fFFFf...........fFFFf...........fFFFf....\n\
             ....fffff...........fffff...........fffff....\n\
             ......F......................................\n\
             ......f......................................\n\
             ......F......................................\n\
             ......f......................................\n\
             ......F......................................\n\
             ......f......................................\n\
             ......F......................................\n\
             ......f......................................\n\
             ......F......................................\n\
             VVVVvVf......................................\n\
             VvvvvVF......................................\n\
             vVVvvVf.............fffff...........fffff....\n\
             FFFFFFFFm...........fFFFf...........fFFFf....\n\
             fffffffFm...........fFfFf...........fFfFf....\n\
             fFFFFFfFm...........fFFFf...........fFFFf....\n\
             fFfffFfFm...........fffff...........fffff....\n\
             fFfffFfFm....................................\n\
             fFfffFfFm....................................\n\
             fFFFFFfFm....................................\n\
             fffffffFm....................................\n"
        );
    }
}

// Encoding region
//------------------------------------------------------------------------------

impl QR {
    pub fn draw_encoding_region(&mut self, payload: BitStream) {
        self.reserve_format_area();
        self.draw_version_info();
        match self.pal {
            Palette::Mono => self.draw_payload(payload),
            Palette::Poly => self.draw_color_payload(payload),
        }

        let w = self.ver.width();
        let ver_sz = w * w;
        debug_assert!(!self.grid[..ver_sz].contains(&Module::Empty), "Empty module found in debug");
    }

    fn draw_payload(&mut self, payload: BitStream) {
        let mut coords = EncRegionIter::new(self.ver);
        for bit in payload {
            let module = Module::Data(if bit { Color::Black } else { Color::White });
            for (r, c) in coords.by_ref() {
                if matches!(self.get(r, c), Module::Empty) {
                    self.set(r, c, module);
                    break;
                }
            }
        }
        self.fill_remainder_bits(&mut coords);
    }

    fn draw_color_payload(&mut self, mut payload: BitStream) {
        let chan_cap = self.ver.channel_codewords();
        let chan_bit_cap = chan_cap << 3;
        debug_assert_eq!(
            chan_cap * 3,
            payload.len() >> 3,
            "Channel capacity {chan_cap} is not equal to 1/3rd of codewords sz {}",
            payload.len() >> 3
        );
        let mut coords = EncRegionIter::new(self.ver).cycle();
        for chan in (0..=2).rev() {
            for bit in Iterator::take(&mut payload, chan_bit_cap) {
                for (r, c) in coords.by_ref() {
                    match self.get_mut(r, c) {
                        Module::Empty => {
                            let clr = if bit { Color::Black } else { Color::Red };
                            let module = Module::Data(clr);
                            self.set(r, c, module);
                            break;
                        }
                        Module::Data(clr) => {
                            let mut byte = *clr as u8;
                            if !bit {
                                byte |= 1 << chan;
                            }
                            let clr = Color::try_from(byte).unwrap();
                            self.set(r, c, Module::Data(clr));
                            break;
                        }
                        _ => (),
                    }
                }
            }
            self.fill_remainder_bits(&mut coords);
        }
    }

    fn fill_remainder_bits(&mut self, coords: impl Iterator<Item = (i32, i32)>) {
        let n = self.ver.remainder_bits();
        for (r, c) in coords.take(n).by_ref() {
            if matches!(self.get(r, c), Module::Empty) {
                self.set(r, c, Module::Data(Color::White));
            }
        }
    }

    pub fn apply_mask(&mut self, pattern: MaskPattern) {
        self.mask = Some(pattern);
        let mask_fn = pattern.mask_functions();
        let w = self.w as i32;
        for r in 0..w {
            for c in 0..w {
                if mask_fn(r, c) {
                    if let Module::Data(clr) = self.get(r, c) {
                        self.set(r, c, Module::Data(!clr))
                    }
                }
            }
        }
        let format_info = generate_format_info_qr(self.ecl, pattern);
        self.draw_format_info(format_info);
    }
}

// Render
//------------------------------------------------------------------------------

// TODO: Write testcases
impl QR {
    pub fn to_gray_image(&self, module_sz: u32) -> GrayImage {
        let qz_sz = if let Version::Normal(_) = self.ver { 4 } else { 2 } * module_sz;
        let qr_sz = self.w as u32 * module_sz;
        let total_sz = qz_sz + qr_sz + qz_sz;

        let mut canvas = GrayImage::new(total_sz, total_sz);
        for i in 0..total_sz {
            for j in 0..total_sz {
                if i < qz_sz || i >= qz_sz + qr_sz || j < qz_sz || j >= qz_sz + qr_sz {
                    canvas.put_pixel(j, i, Luma([255]));
                    continue;
                }
                let r = (i - qz_sz) / module_sz;
                let c = (j - qz_sz) / module_sz;

                let clr = match self.get(r as i32, c as i32) {
                    Module::Func(c) | Module::Format(c) | Module::Version(c) | Module::Data(c) => c,
                    Module::Empty => panic!("Empty module found at: {r} {c}"),
                };

                let pixel =
                    if clr != Color::White { Luma([(clr as u8) * 35]) } else { Luma([255]) };

                canvas.put_pixel(j, i, pixel);
            }
        }

        canvas
    }

    pub fn to_image(&self, module_sz: u32) -> RgbImage {
        let qz_sz = if let Version::Normal(_) = self.ver { 4 } else { 2 } * module_sz;
        let qr_sz = self.w as u32 * module_sz;
        let total_sz = qz_sz + qr_sz + qz_sz;

        let mut canvas = RgbImage::new(total_sz, total_sz);
        for i in 0..total_sz {
            for j in 0..total_sz {
                if i < qz_sz || i >= qz_sz + qr_sz || j < qz_sz || j >= qz_sz + qr_sz {
                    canvas.put_pixel(j, i, Rgb([255, 255, 255]));
                    continue;
                }
                let r = (i - qz_sz) / module_sz;
                let c = (j - qz_sz) / module_sz;

                let clr = match self.get(r as i32, c as i32) {
                    Module::Func(c) | Module::Format(c) | Module::Version(c) | Module::Data(c) => c,
                    Module::Empty => panic!("Empty module found at: {r} {c}"),
                };

                canvas.put_pixel(j, i, clr.into());
            }
        }

        canvas
    }

    pub fn to_str(&self, module_sz: usize) -> String {
        let qz_sz = if let Version::Normal(_) = self.ver { 4 } else { 2 } * module_sz;
        let qr_sz = self.w * module_sz;
        let total_sz = qz_sz + qr_sz + qz_sz;

        let mut canvas = String::new();
        for i in 0..total_sz {
            for j in 0..total_sz {
                if i < qz_sz || i >= qz_sz + qr_sz || j < qz_sz || j >= qz_sz + qr_sz {
                    canvas.push('█');
                    continue;
                }
                let r = ((i - qz_sz) / module_sz) as i32;
                let c = ((j - qz_sz) / module_sz) as i32;

                let clr = match self.get(r, c) {
                    Module::Func(c) | Module::Format(c) | Module::Version(c) | Module::Data(c) => c,
                    Module::Empty => panic!("Empty module found at: {r} {c}"),
                };
                canvas.push(clr.select('█', ' '));
            }
            canvas.push('\n');
        }

        canvas
    }
}

// Global constants
//------------------------------------------------------------------------------
