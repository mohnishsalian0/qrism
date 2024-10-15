use std::ops::Deref;

use crate::{
    mask::MaskingPattern,
    types::{get_format_info, Color, ECLevel, Palette, Version},
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

impl QR {
    // TODO: debug assert for params
    pub fn new(version: Version, ec_level: ECLevel, palette: Palette) -> Self {
        let width = version.get_width();
        Self {
            version,
            width,
            ec_level,
            palette,
            grid: vec![Module::Empty; width * width],
        }
    }

    pub fn get_version(&self) -> Version {
        self.version
    }

    pub fn get_width(&self) -> usize {
        self.width
    }

    pub fn get_ec_level(&self) -> ECLevel {
        self.ec_level
    }

    pub fn get_palette(&self) -> Palette {
        self.palette
    }

    pub fn count_dark_modules(&self) -> usize {
        self.grid
            .iter()
            .filter(|&m| matches!(**m, Color::Dark))
            .count()
    }

    fn coord_to_index(&self, r: i16, c: i16) -> usize {
        debug_assert!(
            r >= -(self.width as i16) && r < (self.width as i16),
            "row should be greater than or equal to width"
        );
        debug_assert!(
            c >= -(self.width as i16) && c < (self.width as i16),
            "column should be greater than or equal to width"
        );

        let r = if r < 0 { r + self.width as i16 } else { r } as usize;
        let c = if c < 0 { c + self.width as i16 } else { c } as usize;
        r * (self.width) + c
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
                        (2 | -2, _) | (_, 2 | -2) => Module::Func(Color::Dark),
                        _ => Module::Func(Color::Dark),
                    },
                );
            }
        }
    }

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

    fn draw_line(&mut self, r1: i16, c1: i16, r2: i16, c2: i16) {
        debug_assert!(
            r1 == r2 || c1 == c2,
            "Line is neither vertical nor horizontal"
        );

        if r1 == r2 {
            for j in c1..=c2 {
                self.set(
                    r1,
                    j,
                    if j & 1 == 0 {
                        Module::Func(Color::Dark)
                    } else {
                        Module::Func(Color::Light)
                    },
                );
            }
        } else {
            for i in r1..=r2 {
                self.set(
                    i,
                    c1,
                    if i & 1 == 0 {
                        Module::Func(Color::Dark)
                    } else {
                        Module::Func(Color::Light)
                    },
                );
            }
        }
    }

    fn draw_timing_pattern(&mut self) {
        let w = self.width as i16;
        let (offset, last) = match self.version {
            Version::Micro(_) => (0, w - 1),
            Version::Normal(_) => (6, w - 9),
        };
        self.draw_line(offset, 8, offset, last);
        self.draw_line(8, offset, last, offset);
    }

    fn draw_alignment_pattern_at(&mut self, r: i16, c: i16) {
        if self.get(r, c) != Module::Empty {
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

    fn draw_alignment_patterns(&mut self) {
        let positions = self.version.get_alignment_pattern();
        for &r in positions {
            for &c in positions {
                self.draw_alignment_pattern_at(r, c)
            }
        }
    }

    pub fn draw_all_function_patterns(&mut self) {
        self.draw_finder_patterns();
        self.draw_timing_pattern();
        self.draw_alignment_patterns();
    }

    fn draw_number(
        &mut self,
        number: u32,
        off_color: Module,
        on_color: Module,
        coords: &[(i16, i16)],
    ) {
        let bits = 32 - number.leading_zeros();
        debug_assert!(
            bits == coords.len() as u32,
            "Format info length doesn't match coordinates length"
        );

        let mut mask = 1 << (bits - 1);
        for (r, c) in coords {
            if number & mask == 0 {
                self.set(*r, *c, off_color);
            } else {
                self.set(*r, *c, on_color);
            }
            mask >>= 1;
        }
    }

    fn reserve_format_area(&mut self) {
        match self.version {
            Version::Micro(_) => todo!(),
            Version::Normal(_) => {
                let format_info = 1 << 14;
                self.draw_number(
                    format_info,
                    Module::Format(Color::Light),
                    Module::Format(Color::Dark),
                    &FORMAT_INFO_COORDS_QR_MAIN,
                );
                self.draw_number(
                    format_info,
                    Module::Format(Color::Light),
                    Module::Format(Color::Dark),
                    &FORMAT_INFO_COORDS_QR_SIDE,
                );
            }
        }
    }

    fn draw_format_info(&mut self, mask_pattern: MaskingPattern) {
        match self.version {
            Version::Micro(_) => todo!(),
            Version::Normal(_) => {
                let format_info = get_format_info(self.version, self.ec_level, mask_pattern);
                self.draw_number(
                    format_info,
                    Module::Format(Color::Light),
                    Module::Format(Color::Dark),
                    &FORMAT_INFO_COORDS_QR_MAIN,
                );
                self.draw_number(
                    format_info,
                    Module::Format(Color::Light),
                    Module::Format(Color::Dark),
                    &FORMAT_INFO_COORDS_QR_SIDE,
                );
            }
        }
    }

    fn draw_version_info(&mut self) {
        match self.version {
            Version::Micro(_) | Version::Normal(1..=6) => {}
            Version::Normal(7..=40) => {
                let version_info = self.version.get_version_info();
                self.draw_number(
                    version_info,
                    Module::Version(Color::Light),
                    Module::Version(Color::Dark),
                    &VERSION_INFO_COORDS_BL,
                );
                self.draw_number(
                    version_info,
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
                    let palette_info = self.palette.get_palette_info().unwrap();
                    self.draw_number(
                        palette_info,
                        Module::Palette(Color::Light),
                        Module::Palette(Color::Dark),
                        &PALETTE_INFO_COORDS_BL,
                    );
                    self.draw_number(
                        palette_info,
                        Module::Palette(Color::Light),
                        Module::Palette(Color::Dark),
                        &PALETTE_INFO_COORDS_TR,
                    );
                }
                _ => unreachable!("Invalid palette"),
            },
        }
    }

    fn draw_codeword(&mut self) {
        todo!();
    }

    fn draw_data(&mut self) {
        todo!();
    }

    pub fn draw_encoding_region(&mut self) {
        self.draw_version_info();
        self.draw_palette_info();
        self.draw_data();
        self.reserve_format_area();
    }

    pub fn draw_mask_pattern(&mut self, pattern: MaskingPattern) {
        let mask_function = pattern.get_mask_functions();
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
        self.draw_format_info(pattern);
    }
}

// Global constants
//------------------------------------------------------------------------------

static FORMAT_INFO_COORDS_QR_MAIN: [(i16, i16); 15] = [
    (0, 8),
    (1, 8),
    (2, 8),
    (3, 8),
    (4, 8),
    (5, 8),
    (7, 8),
    (8, 8),
    (8, 7),
    (8, 5),
    (8, 4),
    (8, 3),
    (8, 2),
    (8, 1),
    (8, 0),
];

static FORMAT_INFO_COORDS_QR_SIDE: [(i16, i16); 15] = [
    (8, -1),
    (8, -2),
    (8, -3),
    (8, -4),
    (8, -5),
    (8, -6),
    (8, -7),
    (-8, 8),
    (-7, 8),
    (-6, 8),
    (-5, 8),
    (-4, 8),
    (-3, 8),
    (-2, 8),
    (-1, 8),
];

static FORMAT_INFO_COORDS_MICRO_QR: [(i16, i16); 15] = [
    (1, 8),
    (2, 8),
    (3, 8),
    (4, 8),
    (5, 8),
    (6, 8),
    (7, 8),
    (8, 8),
    (8, 7),
    (8, 6),
    (8, 5),
    (8, 4),
    (8, 3),
    (8, 2),
    (8, 1),
];

static VERSION_INFO_COORDS_BL: [(i16, i16); 18] = [
    (5, -9),
    (5, -10),
    (5, -11),
    (4, -9),
    (4, -10),
    (4, -11),
    (3, -9),
    (3, -10),
    (3, -11),
    (2, -9),
    (2, -10),
    (2, -11),
    (1, -9),
    (1, -10),
    (1, -11),
    (0, -9),
    (0, -10),
    (0, -11),
];

static VERSION_INFO_COORDS_TR: [(i16, i16); 18] = [
    (-9, 5),
    (-10, 5),
    (-11, 5),
    (-9, 4),
    (-10, 4),
    (-11, 4),
    (-9, 3),
    (-10, 3),
    (-11, 3),
    (-9, 2),
    (-10, 2),
    (-11, 2),
    (-9, 1),
    (-10, 1),
    (-11, 1),
    (-9, 0),
    (-10, 0),
    (-11, 0),
];

static PALETTE_INFO_COORDS_BL: [(i16, i16); 12] = [
    (-1, 10),
    (-1, 9),
    (-2, 10),
    (-2, 9),
    (-3, 10),
    (-3, 9),
    (-4, 10),
    (-4, 9),
    (-5, 10),
    (-5, 9),
    (-6, 10),
    (-6, 9),
];

static PALETTE_INFO_COORDS_TR: [(i16, i16); 12] = [
    (10, -1),
    (9, -1),
    (10, -2),
    (9, -2),
    (10, -3),
    (9, -3),
    (10, -4),
    (9, -4),
    (10, -5),
    (9, -5),
    (10, -6),
    (9, -6),
];
