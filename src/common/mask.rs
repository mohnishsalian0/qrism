use std::ops::Deref;

use super::metadata::{Color, Version};
use crate::builder::QR;

#[derive(Debug, PartialEq, Eq, Copy, Clone, PartialOrd, Ord)]
pub struct MaskPattern(u8);

impl MaskPattern {
    pub fn new(pattern: u8) -> Self {
        debug_assert!(pattern < 8, "Invalid masking pattern");
        Self(pattern)
    }
}

impl Deref for MaskPattern {
    type Target = u8;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

mod mask_functions {
    pub fn checkerboard(x: i32, y: i32) -> bool {
        (x + y) & 1 == 0
    }

    pub fn horizontal_lines(_: i32, y: i32) -> bool {
        y & 1 == 0
    }

    pub fn vertical_lines(x: i32, _: i32) -> bool {
        x % 3 == 0
    }

    pub fn diagonal_lines(x: i32, y: i32) -> bool {
        (x + y) % 3 == 0
    }

    pub fn large_checkerboard(x: i32, y: i32) -> bool {
        ((y >> 1) + (x / 3)) & 1 == 0
    }

    pub fn fields(x: i32, y: i32) -> bool {
        ((x * y) & 1) + ((x * y) % 3) == 0
    }

    pub fn diamonds(x: i32, y: i32) -> bool {
        (((x * y) & 1) + ((x * y) % 3)) & 1 == 0
    }

    pub fn meadow(x: i32, y: i32) -> bool {
        (((x + y) & 1) + ((x * y) % 3)) & 1 == 0
    }
}

impl MaskPattern {
    pub fn mask_functions(self) -> fn(i32, i32) -> bool {
        debug_assert!(*self < 8, "Invalid pattern");

        match *self {
            0b000 => mask_functions::checkerboard,
            0b001 => mask_functions::horizontal_lines,
            0b010 => mask_functions::vertical_lines,
            0b011 => mask_functions::diagonal_lines,
            0b100 => mask_functions::large_checkerboard,
            0b101 => mask_functions::fields,
            0b110 => mask_functions::diamonds,
            0b111 => mask_functions::meadow,
            _ => unreachable!(),
        }
    }
}

pub fn apply_best_mask(qr: &mut QR) -> MaskPattern {
    let best_mask = (0..8)
        .min_by_key(|m| {
            let mut qr = qr.clone();
            qr.apply_mask(MaskPattern(*m));
            compute_total_penalty(&qr)
        })
        .expect("Should return atleast 1 mask");
    let best_mask = MaskPattern(best_mask);
    qr.apply_mask(best_mask);
    best_mask
}

pub fn compute_total_penalty(qr: &QR) -> u32 {
    match qr.version() {
        Version::Micro(_) => todo!(),
        Version::Normal(_) => {
            let adj_pen = compute_adjacent_penalty(qr);
            let blk_pen = compute_block_penalty(qr);
            let fp_pen_h = compute_finder_pattern_penalty(qr, true);
            let fp_pen_v = compute_finder_pattern_penalty(qr, false);
            let bal_pen = compute_balance_penalty(qr);
            adj_pen + blk_pen + fp_pen_h + fp_pen_v + bal_pen
        }
    }
}

fn compute_adjacent_penalty(qr: &QR) -> u32 {
    let mut pen = 0;
    let w = qr.width();
    let mut cols = vec![(Color::Black, 0); w];
    for r in 0..w {
        let mut last = Color::Black;
        let mut consec_row_len = 0;
        for (c, col) in cols.iter_mut().enumerate() {
            let clr = *qr.get(r as i32, c as i32);
            if last != clr {
                last = clr;
                consec_row_len = 0;
            }
            consec_row_len += 1;
            if consec_row_len >= 5 {
                pen += consec_row_len as u32 - 2;
            }
            if col.0 != clr {
                col.0 = clr;
                col.1 = 0;
            }
            col.1 += 1;
            if col.1 >= 5 {
                pen += col.1 as u32 - 2;
            }
        }
    }
    pen
}

fn compute_block_penalty(qr: &QR) -> u32 {
    let mut pen = 0;
    let w = qr.width() as i32;
    for r in 0..w - 1 {
        for c in 0..w - 1 {
            let clr = *qr.get(r, c);
            if clr == *qr.get(r + 1, c) && clr == *qr.get(r, c + 1) && clr == *qr.get(r + 1, c + 1)
            {
                pen += 3;
            }
        }
    }
    pen
}

fn compute_finder_pattern_penalty(qr: &QR, is_hor: bool) -> u32 {
    let mut pen = 0;
    let w = qr.width() as i32;
    static PATTERN: [Color; 7] = [
        Color::Black,
        Color::White,
        Color::Black,
        Color::Black,
        Color::Black,
        Color::White,
        Color::Black,
    ];
    for i in 0..w {
        for j in 0..w - 6 {
            let get: Box<dyn Fn(i32) -> Color> =
                if is_hor { Box::new(|c| *qr.get(i, c)) } else { Box::new(|r| *qr.get(r, i)) };
            if !(j..j + 7).map(&*get).ne(PATTERN.iter().copied()) {
                let match_qz = |x| x >= 0 && x < w && get(x) == Color::Black;
                if (j - 4..j).any(&match_qz) || (j + 7..j + 11).any(&match_qz) {
                    pen += 40;
                }
            }
        }
    }
    pen
}

fn compute_balance_penalty(qr: &QR) -> u32 {
    let dark_cnt = qr.count_dark_modules();
    let w = qr.width();
    let tot = w * w;
    let ratio = dark_cnt * 200 / tot;
    if ratio < 100 {
        (100 - ratio) as _
    } else {
        (ratio - 100) as _
    }
}

// TODO: Write test cases
