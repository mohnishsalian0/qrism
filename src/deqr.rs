use std::ops::{Deref, Not};

use image::{DynamicImage, GenericImageView, GrayImage, Luma};

use crate::{
    ecc::rectify_info,
    error::{QRError, QRResult},
    iter::EncRegionIter,
    mask::MaskingPattern,
    metadata::{
        Color, ECLevel, Version, FORMAT_INFOS_QR, FORMAT_INFO_COORDS_QR_MAIN,
        FORMAT_INFO_COORDS_QR_SIDE, FORMAT_MASK, VERSION_INFOS, VERSION_INFO_COORDS_BL,
        VERSION_INFO_COORDS_TR,
    },
};

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
enum Module {
    Unmarked(Color),
    Marked,
}

impl Deref for Module {
    type Target = Color;
    fn deref(&self) -> &Self::Target {
        match self {
            Module::Unmarked(c) => c,
            Module::Marked => &Color::Dark,
        }
    }
}

impl Not for Module {
    type Output = Module;
    fn not(self) -> Self::Output {
        match self {
            Module::Unmarked(c) => Module::Unmarked(!c),
            Module::Marked => Module::Marked,
        }
    }
}

// QR type for reader
//------------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct DeQR {
    width: usize,
    grid: Vec<Module>,
    version: Version,
    ec_level: Option<ECLevel>,
}

impl DeQR {
    pub fn from_image(image: GrayImage, version: Version) -> Self {
        let qr_width = version.width();
        let (w, h) = image.dimensions();

        debug_assert!(w == h, "Image is not perfect square");
        debug_assert!(w as usize % qr_width == 0, "Image width is not a multiple of qr size");

        let mod_size = w as usize / qr_width;
        let half_area = mod_size * mod_size / 2;
        let mut black_count = vec![0; qr_width * qr_width];

        for (c, r, pixel) in image.enumerate_pixels() {
            let index = Self::coord_to_index(r as i16, c as i16, qr_width);
            let Luma([luma]) = *pixel;
            black_count[index] += if luma < 128 { 1 } else { 0 };
        }

        let grid = black_count
            .iter()
            .map(|&bc| Module::Unmarked(if bc > half_area { Color::Dark } else { Color::Light }))
            .collect();

        Self { width: qr_width, grid, version, ec_level: None }
    }

    pub fn count_dark_modules(&self) -> usize {
        self.grid.iter().filter(|&m| matches!(**m, Color::Dark)).count()
    }

    #[cfg(test)]
    fn to_debug_str(&self) -> String {
        let w = self.width as i16;
        let mut res = String::with_capacity((w * (w + 1)) as usize);
        res.push('\n');
        for i in 0..w {
            for j in 0..w {
                let c = match self.get(i, j) {
                    Module::Unmarked(Color::Dark) => 'u',
                    Module::Unmarked(Color::Light | Color::Hue(_)) => 'U',
                    Module::Marked => '.',
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

    fn get(&self, r: i16, c: i16) -> Module {
        self.grid[Self::coord_to_index(r, c, self.width)]
    }

    fn get_mut(&mut self, r: i16, c: i16) -> &mut Module {
        let index = Self::coord_to_index(r, c, self.width);
        &mut self.grid[index]
    }

    fn set(&mut self, r: i16, c: i16, module: Module) {
        *self.get_mut(r, c) = module;
    }
}

// Format & version info
//------------------------------------------------------------------------------

impl DeQR {
    pub fn identify_format_info(&mut self) -> QRResult<u32> {
        let main = self.get_number(&FORMAT_INFO_COORDS_QR_MAIN);
        let f = rectify_info(main, &FORMAT_INFOS_QR, 3)
            .or_else(|_| {
                let side = self.get_number(&FORMAT_INFO_COORDS_QR_SIDE);
                rectify_info(side, &FORMAT_INFOS_QR, 3)
            })
            .or(Err(QRError::InvalidFormatInfo))?;
        self.mark_coords(&FORMAT_INFO_COORDS_QR_MAIN);
        self.mark_coords(&FORMAT_INFO_COORDS_QR_SIDE);
        Ok(f ^ FORMAT_MASK)
    }

    pub fn verify_version_info(&mut self) -> QRResult<Version> {
        let bl = self.get_number(&VERSION_INFO_COORDS_BL);
        let v = rectify_info(bl, &VERSION_INFOS, 3)
            .or_else(|_| {
                let tr = self.get_number(&VERSION_INFO_COORDS_TR);
                rectify_info(tr, &VERSION_INFOS, 3)
            })
            .or(Err(QRError::InvalidVersionInfo))?;
        self.mark_coords(&VERSION_INFO_COORDS_BL);
        self.mark_coords(&VERSION_INFO_COORDS_TR);
        Ok(Version::Normal(v as usize))
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
            self.set(*r, *c, Module::Marked);
        }
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

// Finder pattern
//------------------------------------------------------------------------------

impl DeQR {
    pub fn mark_finder_patterns(&mut self) {
        self.mark_finder_pattern_at(3, 3);
        match self.version.expect("Version not found") {
            Version::Micro(_) => {}
            Version::Normal(_) => {
                self.mark_finder_pattern_at(3, -4);
                self.mark_finder_pattern_at(-4, 3);
            }
        }
    }

    pub fn mark_finder_pattern_at(&mut self, r: i16, c: i16) {
        let (dr_left, dr_right) = if r > 0 { (-3, 4) } else { (-4, 3) };
        let (dc_top, dc_bottom) = if c > 0 { (-3, 4) } else { (-4, 3) };
        for i in dr_left..=dr_right {
            for j in dc_top..=dc_bottom {
                self.set(r + i, c + j, Module::Marked);
            }
        }
    }
}

// Timing pattern
//------------------------------------------------------------------------------

impl DeQR {
    pub fn mark_timing_patterns(&mut self) {
        let w = self.width as i16;
        let (offset, last) = match self.version.expect("Version not found") {
            Version::Micro(_) => (0, w - 1),
            Version::Normal(_) => (6, w - 9),
        };
        self.mark_line(offset, 8, offset, last);
        self.mark_line(8, offset, last, offset);
    }

    pub fn mark_line(&mut self, r1: i16, c1: i16, r2: i16, c2: i16) {
        debug_assert!(r1 == r2 || c1 == c2, "Line is neither vertical nor horizontal");

        if r1 == r2 {
            for j in c1..=c2 {
                self.set(r1, j, Module::Marked);
            }
        } else {
            for i in r1..=r2 {
                self.set(i, c1, Module::Marked);
            }
        }
    }
}

// Alignment pattern
//------------------------------------------------------------------------------

impl DeQR {
    pub fn mark_alignment_patterns(&mut self) {
        let positions = self.version.expect("Version not found").alignment_pattern();
        for &r in positions {
            for &c in positions {
                self.mark_alignment_pattern_at(r, c);
            }
        }
    }

    pub fn mark_alignment_pattern_at(&mut self, r: i16, c: i16) {
        let w = self.width as i16;
        if (r == 6 && (c == 6 || c - w == -7)) || (r - w == -7 && c == 6) {
            return;
        }
        for i in -2..=2 {
            for j in -2..=2 {
                self.set(r + i, c + j, Module::Marked);
            }
        }
    }
}

// Unmask
//------------------------------------------------------------------------------

impl DeQR {
    pub fn unmask(&mut self, pattern: MaskingPattern) {
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
        let mut codewords = Vec::with_capacity(self.width * self.width);
        let mut coords = EncRegionIter::new(version);
        while let Some((mut r, mut c)) = coords.next() {
            let mut codeword = 0;
            for _ in 0..8 {
                while !matches!(self.get(r, c), Module::Unmarked(_)) {
                    (r, c) = match coords.next() {
                        Some(next) => next,
                        None => return codewords,
                    };
                }
                codeword = (codeword << 1) | u8::from(*self.get(r, c));
            }
            codewords.push(codeword);
        }
        codewords
    }
}
