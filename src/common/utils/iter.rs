use crate::metadata::Version;

// Iterator for placing data in encoding region of QR
//------------------------------------------------------------------------------

#[derive(Clone)]
pub struct EncRegionIter {
    x: i32,
    y: i32,
    w: i32,
    ap: &'static [i32], // Alignment pattern locations
    ver: Version,
    vert_timing_col: i32,
}

impl EncRegionIter {
    pub fn new(ver: Version) -> Self {
        let w = ver.width() as i32;
        let ap = ver.alignment_pattern();
        let vert_timing_col = match ver {
            Version::Micro(_) => 0,
            Version::Normal(_) => 6,
        };
        Self { x: w - 1, y: w - 1, w, ap, ver, vert_timing_col }
    }

    // Checks if the module is reserved for functional pattern or metadata info
    fn is_reserved(&self, x: i32, y: i32) -> bool {
        let w = self.w;

        // Top left finder & info check
        if x < 9 && y < 9 {
            return true;
        }

        // Top right finder & format info check
        if x >= w - 8 && y < 9 {
            return true;
        }

        // Bottom left finder & format info check
        if x < 9 && y >= w - 8 {
            return true;
        }

        // Timing pattern check
        if x == 6 || y == 6 {
            return true;
        }

        // Version info check
        if matches!(self.ver, Version::Normal(7..=40)) {
            // Top right
            if (w - 11..=w - 9).contains(&x) && (0..=5).contains(&y) {
                return true;
            }

            // Bottom left
            if (0..=5).contains(&x) && (w - 11..=w - 9).contains(&y) {
                return true;
            }
        }

        // Alignment pattern check
        for &ax in self.ap {
            for &ay in self.ap {
                if (ax == 6 && (ay == 6 || ay == w - 7)) || (ax == w - 7 && ay == 6) {
                    continue;
                }
                if ax - 2 <= x && x <= ax + 2 && ay - 2 <= y && y <= ay + 2 {
                    return true;
                }
            }
        }

        false
    }
}

impl Iterator for EncRegionIter {
    type Item = (i32, i32);

    fn next(&mut self) -> Option<Self::Item> {
        if self.x < 0 {
            return None;
        }

        let res = (self.x, self.y);

        loop {
            let adjusted_x = if self.x <= self.vert_timing_col { self.x + 1 } else { self.x };
            let col_type = (self.w - adjusted_x) % 4;

            match col_type {
                2 if self.y > 0 => {
                    self.y -= 1;
                    self.x += 1;
                }
                0 if self.y < self.w - 1 => {
                    self.y += 1;
                    self.x += 1;
                }
                0 | 2 if self.x == self.vert_timing_col + 1 => {
                    self.x -= 2;
                }
                _ => {
                    self.x -= 1;
                }
            }

            if !self.is_reserved(self.x, self.y) || self.x < 0 {
                break;
            }
        }

        Some(res)
    }
}

#[cfg(test)]
mod iter_tests {
    use super::EncRegionIter;
    use crate::common::metadata::Version;

    #[test]
    fn test_enc_region_iter() {
        for v in 1..40 {
            let ver = Version::Normal(v);
            let coords = EncRegionIter::new(ver);
            let total_codewords = coords.into_iter().count() / 8;
            let exp_codewords = ver.channel_codewords();
            assert_eq!(total_codewords, exp_codewords);
        }
    }
}
