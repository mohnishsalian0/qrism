use std::ops::Deref;

use image::DynamicImage;

use crate::{
    ecc::rectify_info,
    error::{QRError, QRResult},
    metadata::{
        Color, Version, FORMAT_INFOS_QR, FORMAT_INFO_COORDS_QR_MAIN, FORMAT_INFO_COORDS_QR_SIDE,
        FORMAT_MASK, VERSION_INFOS, VERSION_INFO_COORDS_BL, VERSION_INFO_COORDS_TR,
    },
};

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
enum Module {
    Unknown(Color),
    Visited,
}

impl Deref for Module {
    type Target = Color;
    fn deref(&self) -> &Self::Target {
        match self {
            Module::Unknown(c) => c,
            Module::Visited => &Color::Dark,
        }
    }
}

// QR type for reader
//------------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct DeQR {
    width: usize,
    grid: Vec<Module>,
}

impl DeQR {
    pub fn from_image(image: DynamicImage) -> Self {
        todo!()
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
                    Module::Unknown(Color::Dark) => 'u',
                    Module::Unknown(Color::Light | Color::Hue(_)) => 'U',
                    Module::Visited => '.',
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

    fn get(&self, r: i16, c: i16) -> Module {
        self.grid[self.coord_to_index(r, c)]
    }

    fn get_mut(&mut self, r: i16, c: i16) -> &mut Module {
        let index = self.coord_to_index(r, c);
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
        Ok(f ^ FORMAT_MASK)
    }

    pub fn identify_version_info(&mut self) -> QRResult<Version> {
        let bl = self.get_number(&VERSION_INFO_COORDS_BL);
        let v = rectify_info(bl, &VERSION_INFOS, 3).or_else(|_| {
            let tr = self.get_number(&VERSION_INFO_COORDS_TR);
            rectify_info(tr, &VERSION_INFOS, 3).or(Err(QRError::InvalidVersionInfo))
        })?;
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
}

// Finder patterns
//------------------------------------------------------------------------------

impl DeQR {
    pub fn identify_all_function_patterns(&mut self) -> QRResult<()> {
        todo!()
    }
}

// Alignment pattern
//------------------------------------------------------------------------------

impl DeQR {
    pub fn identify_alignment_pattern(&mut self) -> QRResult<()> {
        todo!()
    }

    pub fn identify_alignment_pattern_at(&mut self) -> QRResult<()> {
        todo!()
    }
}

// Timing pattern
//------------------------------------------------------------------------------

impl DeQR {
    pub fn identify_timing_pattern(&mut self) -> QRResult<()> {
        todo!()
    }

    pub fn identify_timing_pattern_at(&mut self) -> QRResult<()> {
        todo!()
    }
}

// Finder pattern
//------------------------------------------------------------------------------

impl DeQR {
    pub fn identify_finder_pattern(&mut self) -> QRResult<()> {
        todo!()
    }

    pub fn identify_finder_pattern_at(&mut self) -> QRResult<()> {
        todo!()
    }
}

// Encoding region
//------------------------------------------------------------------------------

impl DeQR {
    pub fn identify_encoding_region(&mut self) -> Vec<u8> {
        todo!()
    }
}
