use std::ops::Deref;

use image::{GrayImage, Luma};

use crate::{
    iter::EncRegionIter,
    mask::MaskingPattern,
    metadata::{
        Color, ECLevel, Palette, Version, FORMAT_INFOS_QR, FORMAT_INFO_BIT_LEN,
        FORMAT_INFO_COORDS_QR_MAIN, FORMAT_INFO_COORDS_QR_SIDE, PALETTE_INFOS,
        PALETTE_INFO_BIT_LEN, PALETTE_INFO_COORDS_BL, PALETTE_INFO_COORDS_TR, VERSION_INFOS,
        VERSION_INFO_BIT_LEN, VERSION_INFO_COORDS_BL, VERSION_INFO_COORDS_TR,
    },
};

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum Module {
    Empty,
    Func(Color),
    Version(Color),
    Format(Color),
    Palette(Color),
    Data(Color),
}

impl Deref for Module {
    type Target = Color;
    fn deref(&self) -> &Self::Target {
        match self {
            Module::Empty => &Color::Dark,
            Module::Func(c) => c,
            Module::Version(c) => c,
            Module::Format(c) => c,
            Module::Palette(c) => c,
            Module::Data(c) => c,
        }
    }
}

#[derive(Debug, Clone)]
pub struct QR {
    version: Version,
    width: usize,
    ec_level: ECLevel,
    palette: Palette,
    grid: Vec<Module>,
}

// QR type for builder
//------------------------------------------------------------------------------

impl QR {
    pub fn new(version: Version, ec_level: ECLevel, palette: Palette) -> Self {
        debug_assert!(
            matches!(version, Version::Micro(1..=4) | Version::Normal(1..=40)),
            "Invalid version"
        );
        debug_assert!(0 < *palette && *palette < 17, "Invalid palette");

        let width = version.width();
        Self { version, width, ec_level, palette, grid: vec![Module::Empty; width * width] }
    }

    pub fn version(&self) -> Version {
        self.version
    }

    pub fn width(&self) -> usize {
        self.width
    }

    pub fn ec_level(&self) -> ECLevel {
        self.ec_level
    }

    pub fn palette(&self) -> Palette {
        self.palette
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
                    Module::Empty => '.',
                    Module::Func(Color::Dark) => 'f',
                    Module::Func(Color::Light | Color::Hue(_)) => 'F',
                    Module::Version(Color::Dark) => 'v',
                    Module::Version(Color::Light | Color::Hue(_)) => 'V',
                    Module::Format(Color::Dark) => 'm',
                    Module::Format(Color::Light | Color::Hue(_)) => 'M',
                    Module::Palette(Color::Dark) => 'p',
                    Module::Palette(Color::Light | Color::Hue(_)) => 'P',
                    Module::Data(Color::Dark) => 'd',
                    Module::Data(Color::Light | Color::Hue(_)) => 'D',
                };
                res.push(c);
            }
            res.push('\n');
        }
        res
    }

    fn coord_to_index(&self, r: i16, c: i16) -> usize {
        let w = self.width as i16;
        debug_assert!(-w <= r && r < w, "row should be greater than or equal to width");
        debug_assert!(-w <= c && c < w, "column should be greater than or equal to width");

        let r = if r < 0 { r + w } else { r };
        let c = if c < 0 { c + w } else { c };
        (r * w + c) as _
    }

    pub fn get(&self, r: i16, c: i16) -> Module {
        self.grid[self.coord_to_index(r, c)]
    }

    pub fn get_mut(&mut self, r: i16, c: i16) -> &mut Module {
        let index = self.coord_to_index(r, c);
        &mut self.grid[index]
    }

    pub fn set(&mut self, r: i16, c: i16, module: Module) {
        *self.get_mut(r, c) = module;
    }
}

#[cfg(test)]
mod qr_util_tests {
    use crate::{
        metadata::{Color, ECLevel, Palette, Version},
        qr::{Module, QR},
    };

    #[test]
    fn test_index_wrap() {
        let mut qr = QR::new(Version::Normal(1), ECLevel::L, Palette::Monochrome);
        let w = qr.width as i16;
        qr.set(-1, -1, Module::Func(Color::Dark));
        assert_eq!(qr.get(w - 1, w - 1), Module::Func(Color::Dark));
        qr.set(0, 0, Module::Func(Color::Dark));
        assert_eq!(qr.get(-w, -w), Module::Func(Color::Dark));
    }

    #[test]
    #[should_panic]
    fn test_row_out_of_bound() {
        let qr = QR::new(Version::Normal(1), ECLevel::L, Palette::Monochrome);
        let w = qr.width as i16;
        qr.get(w, 0);
    }

    #[test]
    #[should_panic]
    fn test_col_out_of_bound() {
        let qr = QR::new(Version::Normal(1), ECLevel::L, Palette::Monochrome);
        let w = qr.width as i16;
        qr.get(0, w);
    }

    #[test]
    #[should_panic]
    fn test_row_index_overwrap() {
        let qr = QR::new(Version::Normal(1), ECLevel::L, Palette::Monochrome);
        let w = qr.width as i16;
        qr.get(-(w + 1), 0);
    }

    #[test]
    #[should_panic]
    fn test_col_index_overwrap() {
        let qr = QR::new(Version::Normal(1), ECLevel::L, Palette::Monochrome);
        let w = qr.width as i16;
        qr.get(0, -(w + 1));
    }
}

// Finder pattern
//------------------------------------------------------------------------------

impl QR {
    fn draw_finder_patterns(&mut self) {
        self.draw_finder_pattern_at(3, 3);
        match self.version {
            Version::Micro(_) => {}
            Version::Normal(_) => {
                self.draw_finder_pattern_at(3, -4);
                self.draw_finder_pattern_at(-4, 3);
            }
        }
    }

    fn draw_finder_pattern_at(&mut self, r: i16, c: i16) {
        let (dr_left, dr_right) = if r > 0 { (-3, 4) } else { (-4, 3) };
        let (dc_top, dc_bottom) = if c > 0 { (-3, 4) } else { (-4, 3) };
        for i in dr_left..=dr_right {
            for j in dc_top..=dc_bottom {
                self.set(
                    r + i,
                    c + j,
                    match (i, j) {
                        (4 | -4, _) | (_, 4 | -4) => Module::Func(Color::Light),
                        (3 | -3, _) | (_, 3 | -3) => Module::Func(Color::Dark),
                        (2 | -2, _) | (_, 2 | -2) => Module::Func(Color::Light),
                        _ => Module::Func(Color::Dark),
                    },
                );
            }
        }
    }
}

#[cfg(test)]
mod finder_pattern_tests {
    use crate::{
        metadata::{ECLevel, Palette, Version},
        qr::QR,
    };

    #[test]
    fn test_finder_pattern_qr() {
        let mut qr = QR::new(Version::Normal(1), ECLevel::L, Palette::Monochrome);
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
        let w = self.width as i16;
        let (offset, last) = match self.version {
            Version::Micro(_) => (0, w - 1),
            Version::Normal(_) => (6, w - 9),
        };
        self.draw_line(offset, 8, offset, last);
        self.draw_line(8, offset, last, offset);
    }

    fn draw_line(&mut self, r1: i16, c1: i16, r2: i16, c2: i16) {
        debug_assert!(r1 == r2 || c1 == c2, "Line is neither vertical nor horizontal");

        if r1 == r2 {
            for j in c1..=c2 {
                let m =
                    if j & 1 == 0 { Module::Func(Color::Dark) } else { Module::Func(Color::Light) };
                self.set(r1, j, m);
            }
        } else {
            for i in r1..=r2 {
                let m =
                    if i & 1 == 0 { Module::Func(Color::Dark) } else { Module::Func(Color::Light) };
                self.set(i, c1, m);
            }
        }
    }
}

#[cfg(test)]
mod timing_pattern_tests {
    use crate::{
        metadata::{ECLevel, Palette, Version},
        qr::QR,
    };

    #[test]
    fn test_timing_pattern_1() {
        let mut qr = QR::new(Version::Normal(1), ECLevel::L, Palette::Monochrome);
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
        let positions = self.version.alignment_pattern();
        for &r in positions {
            for &c in positions {
                self.draw_alignment_pattern_at(r, c)
            }
        }
    }

    fn draw_alignment_pattern_at(&mut self, r: i16, c: i16) {
        let w = self.width as i16;
        if (r == 6 && (c == 6 || c - w == -7)) || (r - w == -7 && c == 6) {
            return;
        }
        for i in -2..=2 {
            for j in -2..=2 {
                self.set(
                    r + i,
                    c + j,
                    match (i, j) {
                        (-2 | 2, _) | (_, -2 | 2) | (0, 0) => Module::Func(Color::Dark),
                        _ => Module::Func(Color::Light),
                    },
                )
            }
        }
    }
}

#[cfg(test)]
mod alignment_pattern_tests {
    use crate::{
        metadata::{ECLevel, Palette, Version},
        qr::QR,
    };

    #[test]
    fn test_alignment_pattern_1() {
        let mut qr = QR::new(Version::Normal(1), ECLevel::L, Palette::Monochrome);
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
        let mut qr = QR::new(Version::Normal(3), ECLevel::L, Palette::Monochrome);
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
        let mut qr = QR::new(Version::Normal(7), ECLevel::L, Palette::Monochrome);
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
    use crate::{
        metadata::{ECLevel, Palette, Version},
        qr::QR,
    };

    #[test]
    fn test_all_function_patterns() {
        let mut qr = QR::new(Version::Normal(3), ECLevel::L, Palette::Monochrome);
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
        match self.version {
            Version::Micro(_) => todo!(),
            Version::Normal(_) => {
                self.draw_number(
                    format_info,
                    FORMAT_INFO_BIT_LEN,
                    Module::Format(Color::Light),
                    Module::Format(Color::Dark),
                    &FORMAT_INFO_COORDS_QR_MAIN,
                );
                self.draw_number(
                    format_info,
                    FORMAT_INFO_BIT_LEN,
                    Module::Format(Color::Light),
                    Module::Format(Color::Dark),
                    &FORMAT_INFO_COORDS_QR_SIDE,
                );
                self.set(-8, 8, Module::Format(Color::Dark));
            }
        }
    }

    fn draw_version_info(&mut self) {
        match self.version {
            Version::Micro(_) | Version::Normal(1..=6) => {}
            Version::Normal(7..=40) => {
                let version_info = version_info(self.version);
                self.draw_number(
                    version_info,
                    VERSION_INFO_BIT_LEN,
                    Module::Version(Color::Light),
                    Module::Version(Color::Dark),
                    &VERSION_INFO_COORDS_BL,
                );
                self.draw_number(
                    version_info,
                    VERSION_INFO_BIT_LEN,
                    Module::Version(Color::Light),
                    Module::Version(Color::Dark),
                    &VERSION_INFO_COORDS_TR,
                );
            }
            _ => unreachable!(),
        }
    }

    fn draw_palette_info(&mut self) {
        match self.version {
            Version::Micro(_) => {}
            Version::Normal(_) => match self.palette {
                Palette::Monochrome => {}
                Palette::Polychrome(2..=16) => {
                    let palette_info = palette_info(self.palette);
                    self.draw_number(
                        palette_info,
                        PALETTE_INFO_BIT_LEN,
                        Module::Palette(Color::Light),
                        Module::Palette(Color::Dark),
                        &PALETTE_INFO_COORDS_BL,
                    );
                    self.draw_number(
                        palette_info,
                        PALETTE_INFO_BIT_LEN,
                        Module::Palette(Color::Light),
                        Module::Palette(Color::Dark),
                        &PALETTE_INFO_COORDS_TR,
                    );
                    self.set(3, 3, Module::Palette(Color::Light));
                    self.set(3, -4, Module::Palette(Color::Light));
                    self.set(-4, 3, Module::Palette(Color::Light));
                }
                _ => unreachable!("Invalid palette"),
            },
        }
    }

    fn draw_number(
        &mut self,
        number: u32,
        bit_len: usize,
        off_color: Module,
        on_color: Module,
        coords: &[(i16, i16)],
    ) {
        let mut mask = 1 << (bit_len - 1);
        for (r, c) in coords {
            if number & mask == 0 {
                self.set(*r, *c, off_color);
            } else {
                self.set(*r, *c, on_color);
            }
            mask >>= 1;
        }
    }
}

#[cfg(test)]
mod qr_information_tests {
    use crate::{
        metadata::{ECLevel, Palette, Version},
        qr::QR,
    };

    #[test]
    fn test_version_info_1() {
        let mut qr = QR::new(Version::Normal(1), ECLevel::L, Palette::Monochrome);
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
        let mut qr = QR::new(Version::Normal(7), ECLevel::L, Palette::Monochrome);
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
        let mut qr = QR::new(Version::Normal(1), ECLevel::L, Palette::Monochrome);
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
    fn test_palette_info() {
        let mut qr = QR::new(Version::Normal(1), ECLevel::L, Palette::Polychrome(2));
        qr.draw_palette_info();
        assert_eq!(
            qr.to_debug_str(),
            "\n\
             .....................\n\
             .....................\n\
             .....................\n\
             ...P.............P...\n\
             .....................\n\
             .....................\n\
             .....................\n\
             .....................\n\
             .....................\n\
             ...............pppppp\n\
             ...............pppppp\n\
             .....................\n\
             .....................\n\
             .....................\n\
             .....................\n\
             .........pp..........\n\
             .........pp..........\n\
             ...P.....pp..........\n\
             .........pp..........\n\
             .........pp..........\n\
             .........pp..........\n"
        );
    }

    #[test]
    fn test_all_function_patterns_and_qr_info() {
        let mut qr = QR::new(Version::Normal(7), ECLevel::L, Palette::Polychrome(2));
        qr.draw_all_function_patterns();
        qr.draw_version_info();
        qr.reserve_format_area();
        qr.draw_palette_info();
        assert_eq!(
            qr.to_debug_str(),
            "\n\
             fffffffFm.........................VVvFfffffff\n\
             fFFFFFfFm.........................VvVFfFFFFFf\n\
             fFfffFfFm.........................VvVFfFfffFf\n\
             fFfPfFfFm.........................VvvFfFfPfFf\n\
             fFfffFfFm...........fffff.........vvvFfFfffFf\n\
             fFFFFFfFm...........fFFFf.........VVVFfFFFFFf\n\
             fffffffFfFfFfFfFfFfFfFfFfFfFfFfFfFfFfFfffffff\n\
             FFFFFFFFm...........fFFFf............FFFFFFFF\n\
             mmmmmmfmm...........fffff............mmmmmmmm\n\
             ......F................................pppppp\n\
             ......f................................pppppp\n\
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
             fFFFFFfFmpp.........fFFFf...........fFFFf....\n\
             fFfffFfFmpp.........fffff...........fffff....\n\
             fFfPfFfFmpp..................................\n\
             fFfffFfFmpp..................................\n\
             fFFFFFfFmpp..................................\n\
             fffffffFmpp..................................\n"
        );
    }
}

// Encoding region
//------------------------------------------------------------------------------

impl QR {
    pub fn draw_encoding_region(&mut self, payload: &[u8]) {
        self.reserve_format_area();
        self.draw_version_info();
        self.draw_palette_info();
        self.draw_payload(payload);

        debug_assert!(!self.grid.contains(&Module::Empty), "Empty module found in debug");
    }

    fn draw_payload(&mut self, payload: &[u8]) {
        let mut coords = EncRegionIter::new(self.version);
        self.draw_codewords(payload, &mut coords);
        self.fill_remainder_bits(&mut coords);
    }

    fn draw_codewords(&mut self, codewords: &[u8], coords: &mut EncRegionIter) {
        for &codeword in codewords.iter() {
            for i in (0..8).rev() {
                let bit = (codeword >> i) & 1;
                let module = Module::Data(if bit & 1 == 0 { Color::Light } else { Color::Dark });
                for (r, c) in coords.by_ref() {
                    if matches!(self.get(r, c), Module::Empty) {
                        self.set(r, c, module);
                        break;
                    }
                }
            }
        }
    }

    fn fill_remainder_bits(&mut self, coords: &mut EncRegionIter) {
        let empty_modules =
            coords.filter(|(r, c)| self.get(*r, *c) == Module::Empty).collect::<Vec<_>>();
        debug_assert!(
            matches!(
                (self.version, empty_modules.len()),
                (Version::Micro(_), 0)
                    | (Version::Normal(1), 0)
                    | (Version::Normal(2..=6), 7)
                    | (Version::Normal(7..=13), 0)
                    | (Version::Normal(14..=20), 3)
                    | (Version::Normal(21..=27), 4)
                    | (Version::Normal(28..=34), 3)
                    | (Version::Normal(35..=40), 0)
            ),
            "Incorrect number of empty modules for remainder bits: Version {:?}, Empty bits {}",
            self.version,
            empty_modules.len()
        );
        empty_modules.iter().for_each(|(r, c)| self.set(*r, *c, Module::Data(Color::Light)));
    }

    pub fn mask(&mut self, pattern: MaskingPattern) {
        let mask_function = pattern.mask_functions();
        let w = self.width as i16;
        for r in 0..w {
            for c in 0..w {
                if mask_function(r, c) {
                    if let Module::Data(clr) = self.get(r, c) {
                        self.set(r, c, Module::Data(!clr))
                    }
                }
            }
        }
        let format_info = format_info_qr(self.ec_level, pattern);
        self.draw_format_info(format_info);
    }
}

// Render
//------------------------------------------------------------------------------

// TODO: Write testcases
impl QR {
    pub fn render(&self, module_size: u32) -> GrayImage {
        let qz_size = if let Version::Normal(_) = self.version { 4 } else { 2 } * module_size;
        let qr_size = self.width as u32 * module_size;
        let total_size = qz_size + qr_size + qz_size;

        let mut canvas = GrayImage::new(total_size, total_size);
        for i in 0..total_size {
            for j in 0..total_size {
                if i < qz_size || i >= qz_size + qr_size || j < qz_size || j >= qz_size + qr_size {
                    canvas.put_pixel(j, i, Luma([255]));
                    continue;
                }
                let r = (i - qz_size) / module_size;
                let c = (j - qz_size) / module_size;

                let color = match self.get(r as i16, c as i16) {
                    Module::Func(c)
                    | Module::Format(c)
                    | Module::Version(c)
                    | Module::Palette(c)
                    | Module::Data(c) => c,
                    Module::Empty => panic!("Empty module found at: {r} {c}"),
                };

                let pixel = match color {
                    Color::Dark => Luma([0]),
                    Color::Light => Luma([255]),
                    Color::Hue(_) => todo!(),
                };

                canvas.put_pixel(j, i, pixel);
            }
        }

        canvas
    }

    pub fn to_str(&self, module_size: usize) -> String {
        let qz_size = if let Version::Normal(_) = self.version { 4 } else { 2 } * module_size;
        let qr_size = self.width * module_size;
        let total_size = qz_size + qr_size + qz_size;

        let mut canvas = String::new();
        for i in 0..total_size {
            for j in 0..total_size {
                if i < qz_size || i >= qz_size + qr_size || j < qz_size || j >= qz_size + qr_size {
                    canvas.push('█');
                    continue;
                }
                let r = ((i - qz_size) / module_size) as i16;
                let c = ((j - qz_size) / module_size) as i16;

                let color = match self.get(r, c) {
                    Module::Func(c)
                    | Module::Format(c)
                    | Module::Version(c)
                    | Module::Palette(c)
                    | Module::Data(c) => c,
                    Module::Empty => panic!("Empty module found at: {r} {c}"),
                };
                canvas.push(color.select('█', ' '));
            }
            canvas.push('\n');
        }

        canvas
    }
}

// Format information
//------------------------------------------------------------------------------

pub fn format_info_qr(ec_level: ECLevel, mask_pattern: MaskingPattern) -> u32 {
    let format_data = ((ec_level as usize) ^ 1) << 3 | (*mask_pattern as usize);
    FORMAT_INFOS_QR[format_data]
}

pub fn version_info(version: Version) -> u32 {
    debug_assert!(matches!(version, Version::Normal(7..=40)), "Invalid version");
    match version {
        Version::Normal(v) => VERSION_INFOS[v - 7],
        _ => unreachable!(),
    }
}
pub fn palette_info(palette: Palette) -> u32 {
    debug_assert!(0 < *palette && *palette < 17, "Invalid palette");

    match palette {
        Palette::Monochrome => 1,
        Palette::Polychrome(p) => PALETTE_INFOS[p as usize],
    }
}

// Global constants
//------------------------------------------------------------------------------
