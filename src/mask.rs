use std::ops::Deref;

use crate::{
    render::QR,
    types::{Color, Version},
};

#[derive(Debug, PartialEq, Eq, Copy, Clone, PartialOrd, Ord)]
pub struct MaskingPattern(u8);

impl Deref for MaskingPattern {
    type Target = u8;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

mod mask_functions {
    pub fn checkerboard(r: i16, c: i16) -> bool {
        (r + c) & 1 == 0
    }

    pub fn horizontal_lines(r: i16, _: i16) -> bool {
        r & 1 == 0
    }

    pub fn vertical_lines(_: i16, c: i16) -> bool {
        c % 3 == 0
    }

    pub fn diagonal_lines(r: i16, c: i16) -> bool {
        (r + c) % 3 == 0
    }

    pub fn large_checkerboard(r: i16, c: i16) -> bool {
        ((r >> 1) + (c / 3)) & 1 == 0
    }

    pub fn fields(r: i16, c: i16) -> bool {
        ((r * c) & 1) + ((r * c) % 3) == 0
    }

    pub fn diamonds(r: i16, c: i16) -> bool {
        (((r * c) & 1) + ((r * c) % 3)) & 1 == 0
    }

    pub fn meadow(r: i16, c: i16) -> bool {
        (((r + c) & 1) + ((r * c) % 3)) & 1 == 0
    }
}

impl MaskingPattern {
    pub fn get_mask_functions(self) -> fn(i16, i16) -> bool {
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

fn compute_adjacent_penalty(qr: &QR) -> u16 {
    let mut penalty = 0;
    let w = qr.get_width();
    let mut cols = vec![(Color::Dark, 0); w];
    for r in 0..w {
        let mut last_row_color = Color::Dark;
        let mut consecutive_row_len = 0;
        for (c, col) in cols.iter_mut().enumerate() {
            let color = *qr.get(r as i16, c as i16);
            if last_row_color != color {
                last_row_color = color;
                consecutive_row_len = 0;
            }
            consecutive_row_len += 1;
            if consecutive_row_len >= 5 {
                penalty += consecutive_row_len as u16 - 2;
            }
            if col.0 != color {
                col.0 = color;
                col.1 = 0;
            }
            col.1 += 1;
            if col.1 >= 5 {
                penalty += col.1 as u16 - 2;
            }
        }
    }
    penalty
}

fn compute_block_penalty(qr: &QR) -> u16 {
    let mut penalty = 0;
    let w = qr.get_width() as i16;
    for r in 0..w - 1 {
        for c in 0..w - 1 {
            let color = *qr.get(r, c);
            if color == *qr.get(r + 1, c)
                && color == *qr.get(r, c + 1)
                && color == *qr.get(r + 1, c + 1)
            {
                penalty += 3;
            }
        }
    }
    penalty
}

fn compute_finder_pattern_penalty(qr: &QR, is_horizontal: bool) -> u16 {
    let mut penalty = 0;
    let w = qr.get_width() as i16;
    static PATTERN: [Color; 7] = [
        Color::Dark,
        Color::Light,
        Color::Dark,
        Color::Dark,
        Color::Dark,
        Color::Light,
        Color::Dark,
    ];
    for i in 0..w {
        for j in 0..w - 6 {
            let get: Box<dyn Fn(i16) -> Color> = if is_horizontal {
                Box::new(|c| *qr.get(i, c))
            } else {
                Box::new(|r| *qr.get(r, i))
            };
            if !(j..j + 7).map(&*get).ne(PATTERN.iter().copied()) {
                let match_quietzone = |x| x >= 0 && x < w && get(x) == Color::Dark;
                if (j - 4..j).any(&match_quietzone) || (j + 7..j + 11).any(&match_quietzone) {
                    penalty += 40;
                }
            }
        }
    }
    penalty
}

fn compute_balance_penalty(qr: &QR) -> u16 {
    let dark_count = qr.count_dark_modules();
    let w = qr.get_width();
    let total_count = w * w;
    let ratio = dark_count * 200 / total_count;
    if ratio < 100 {
        (100 - ratio) as _
    } else {
        (ratio - 100) as _
    }
}

pub fn compute_total_penalty(qr: &QR) -> u16 {
    match qr.get_version() {
        Version::Micro(_) => todo!(),
        Version::Normal(_) => {
            let adjacent_penalty = compute_adjacent_penalty(qr);
            let block_penalty = compute_block_penalty(qr);
            let finder_penalty_hor = compute_finder_pattern_penalty(qr, true);
            let finder_penalty_ver = compute_finder_pattern_penalty(qr, false);
            let balance_penalty = compute_balance_penalty(qr);
            adjacent_penalty
                + block_penalty
                + finder_penalty_hor
                + finder_penalty_ver
                + balance_penalty
        }
    }
}

pub fn apply_best_mask(qr: &mut QR) {
    let best_mask = (0..8)
        .min_by_key(|m| {
            let mut qr = qr.clone();
            qr.draw_mask_pattern(MaskingPattern(*m));
            compute_total_penalty(&qr)
        })
        .expect("Should return atleast 1 mask");
    qr.draw_mask_pattern(MaskingPattern(best_mask));
}
