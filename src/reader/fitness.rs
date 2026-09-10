use super::{
    binarize::BinaryImage,
    finder::FinderGroup,
    utils::{geometry::Point, homography::Homography},
};
use crate::{metadata::Color, utils::QRResult, Version};

// One rectangular patch of the symbol, bounded by four located centres -- alignment patterns,
// finder centres, or a mix of the two at the symbol's edges. Each tile carries a homography
// fitted to its own four corners, so perspective and print warp are absorbed locally instead of
// being averaged across the whole symbol.
#[derive(Debug, Clone)]
pub(super) struct Tile {
    // Maps absolute module coordinates onto image pixels. Absolute, not tile-relative, so a
    // module can be looked up without first rebasing it onto the tile's origin.
    h: Homography,

    // Half-open module rectangle this tile owns: x in [x0, x1), y in [y0, y1).
    x0: u32,
    y0: u32,
    x1: u32,
    y1: u32,
}

impl Tile {
    // `anchors` are the module-space pattern centres the mapping is fitted to, TL, TR, BR, BL,
    // already carrying their half-module offset -- a finder centre is 3.5, an alignment centre at
    // coord `a` is `a + 0.5`. `centres` are where those patterns were measured in the image, in
    // the same order.
    pub(super) fn new(
        bounds: (u32, u32, u32, u32),
        anchors: [(f64, f64); 4],
        centres: [Point; 4],
    ) -> QRResult<Self> {
        let (x0, y0, x1, y1) = bounds;
        let dst = centres.map(|p| (p.x as f64, p.y as f64));

        Ok(Self { h: Homography::compute(anchors, dst)?, x0, y0, x1, y1 })
    }

    #[inline]
    pub(super) fn contains(&self, x: u32, y: u32) -> bool {
        (self.x0 as u32..self.x1 as u32).contains(&x)
            && (self.y0 as u32..self.y1 as u32).contains(&y)
    }

    #[inline]
    pub(super) fn map(&self, x: f64, y: f64) -> QRResult<Point> {
        self.h.map(x, y)
    }

    pub(super) fn exact_map(&self, x: f64, y: f64) -> QRResult<(f64, f64)> {
        self.h.exact_map(x, y)
    }

    // The tile's outline in image space, TL, TR, BR, BL
    pub(super) fn corners(&self) -> QRResult<[Point; 4]> {
        let (x0, y0) = (self.x0 as f64, self.y0 as f64);
        let (x1, y1) = (self.x1 as f64 + 1.0, self.y1 as f64 + 1.0);
        Ok([self.h.map(x0, y0)?, self.h.map(x1, y0)?, self.h.map(x1, y1)?, self.h.map(x0, y1)?])
    }
}

pub(super) fn setup_homography(
    img: &BinaryImage,
    group: &FinderGroup,
    align_centre: Point,
    ver: Version,
) -> Option<Homography> {
    let size = ver.width() as f64;
    let br_off = if *ver == 1 { 3.5 } else { 6.5 };
    let src = [(3.5, 3.5), (size - 3.5, 3.5), (size - br_off, size - br_off), (3.5, size - 3.5)];

    let c0 = (group.finders[0].x as f64, group.finders[0].y as f64);
    let c1 = (group.finders[1].x as f64, group.finders[1].y as f64);
    let c2 = (group.finders[2].x as f64, group.finders[2].y as f64);
    let ca = (align_centre.x as f64, align_centre.y as f64);
    let dst = [c1, c2, ca, c0];

    let initial_h = Homography::compute(src, dst).ok()?;

    jiggle_homography(img, initial_h, ver)
}

// Adjust the homography slightly to refine projection of qr
fn jiggle_homography(img: &BinaryImage, mut h: Homography, ver: Version) -> Option<Homography> {
    let mut best = symbol_fitness(img, &h, ver);

    // Create an adjustment matrix by scaling the homography
    let mut adjustments = h.0.map(|x| x * 0.04);

    for _pass in 0..6 {
        for i in 0..8 {
            let old = h[i];
            for j in 0..2 {
                let step = adjustments[i];
                h[i] = if j & 1 == 0 { old - step } else { old + step };

                let test = symbol_fitness(img, &h, ver);
                if test > best {
                    best = test
                } else {
                    h[i] = old
                }
            }
        }

        // Halve all adjustment steps
        adjustments = adjustments.map(|x| x * 0.5);
    }
    let max_score = max_fitness_score(ver);

    // 60% tolerance
    if best >= max_score * 4 / 10 {
        Some(h)
    } else {
        None
    }
}

fn symbol_fitness(img: &BinaryImage, h: &Homography, ver: Version) -> i32 {
    let mut score = 0;
    let grid_size = ver.width() as i32;

    // Score timing patterns
    for i in 7..grid_size - 7 {
        let flip = if i & 1 == 0 { -1 } else { 1 };
        score += cell_fitness(img, h, i, 6) * flip;
        score += cell_fitness(img, h, 6, i) * flip;
    }

    // Score finders
    score += finder_fitness(img, h, 0, 0);
    score += finder_fitness(img, h, grid_size - 7, 0);
    score += finder_fitness(img, h, 0, grid_size - 7);

    // Score alignment patterns
    for (x, y) in alignment_centres(ver) {
        score += alignment_fitness(img, h, x, y);
    }

    score
}

// Centres of every alignment pattern in the symbol, in module coordinates. The alignment
// coordinates form a grid, minus the three corners occupied by the finders. Version 1 has
// no alignment coordinates, so this yields nothing.
fn alignment_centres(ver: Version) -> impl Iterator<Item = (i32, i32)> {
    let aps = ver.alignment_pattern();
    let len = aps.len();
    let last = len.saturating_sub(1);

    (0..len)
        .flat_map(move |i| (0..len).map(move |j| (i, j)))
        .filter(move |ij| ![(0, 0), (0, last), (last, 0)].contains(ij))
        .map(move |(i, j)| (aps[i] as i32, aps[j] as i32))
}

fn max_fitness_score(ver: Version) -> i32 {
    let mut total_mods = 0;

    // Finder modules
    total_mods += 49 * 3;

    // Timing modules
    let grid_size = ver.width() as i32;
    total_mods += (grid_size - 14) * 2;

    // Alignment modules
    total_mods += 25 * alignment_centres(ver).count() as i32;

    total_mods * 9 // Each module has a maximum score of 9
}

fn finder_fitness(img: &BinaryImage, h: &Homography, x: i32, y: i32) -> i32 {
    let (x, y) = (x + 3, y + 3);
    cell_fitness(img, h, x, y) + ring_fitness(img, h, x, y, 1) - ring_fitness(img, h, x, y, 2)
        + ring_fitness(img, h, x, y, 3)
}

fn alignment_fitness(img: &BinaryImage, h: &Homography, x: i32, y: i32) -> i32 {
    cell_fitness(img, h, x, y) - ring_fitness(img, h, x, y, 1) + ring_fitness(img, h, x, y, 2)
}

fn ring_fitness(img: &BinaryImage, h: &Homography, cx: i32, cy: i32, r: i32) -> i32 {
    let mut score = 0;

    for i in 0..r * 2 {
        score += cell_fitness(img, h, cx - r + i, cy - r);
        score += cell_fitness(img, h, cx - r, cy + r - i);
        score += cell_fitness(img, h, cx + r, cy - r + i);
        score += cell_fitness(img, h, cx + r - i, cy + r);
    }

    score
}

fn cell_fitness(img: &BinaryImage, hm: &Homography, x: i32, y: i32) -> i32 {
    const OFFSETS: [f64; 3] = [0.3, 0.5, 0.7];
    let white = Color::White;
    let mut score = 0;

    for dy in OFFSETS.iter() {
        for dx in OFFSETS.iter() {
            let pt = match hm.map(x as f64 + dx, y as f64 + dy) {
                Ok(v) => v,
                Err(_) => return 0,
            };
            if let Some(color) = img.get_at_point(&pt) {
                if color == white {
                    score -= 1;
                } else {
                    score += 1;
                }
            }
        }
    }
    score
}

#[cfg(test)]
mod fitness_tests {
    use super::{alignment_centres, max_fitness_score};
    use crate::metadata::Version;
    use std::collections::HashSet;

    // Alignment pattern counts per version from ISO/IEC 18004 Annex E: the alignment
    // coordinates form an n x n grid, minus the three cells taken by the finders.
    fn spec_alignment_count(v: usize) -> usize {
        let n = match v {
            1 => return 0,
            2..=6 => 2,
            7..=13 => 3,
            14..=20 => 4,
            21..=27 => 5,
            28..=34 => 6,
            35..=40 => 7,
            _ => unreachable!(),
        };
        n * n - 3
    }

    #[test]
    fn alignment_centres_matches_spec_count() {
        for v in 1..=40 {
            assert_eq!(
                alignment_centres(Version::Normal(v)).count(),
                spec_alignment_count(v),
                "version {v}: wrong number of alignment patterns"
            );
        }
    }

    #[test]
    fn alignment_centres_skips_finder_corners_and_has_no_duplicates() {
        for v in 2..=40 {
            let ver = Version::Normal(v);
            let aps = ver.alignment_pattern();
            let (first, last) = (aps[0] as i32, aps[aps.len() - 1] as i32);
            let centres: Vec<_> = alignment_centres(ver).collect();
            let uniq: HashSet<_> = centres.iter().copied().collect();

            assert_eq!(uniq.len(), centres.len(), "version {v}: duplicate alignment centres");
            for corner in [(first, first), (first, last), (last, first)] {
                assert!(
                    !uniq.contains(&corner),
                    "version {v}: {corner:?} collides with a finder but was emitted"
                );
            }
        }
    }

    // Version 1 has no alignment patterns; the iterator must stay empty rather than panic.
    #[test]
    fn version_1_has_no_alignment_centres() {
        assert_eq!(alignment_centres(Version::Normal(1)).count(), 0);
        assert_eq!(max_fitness_score(Version::Normal(1)), (49 * 3 + (21 - 14) * 2) * 9);
    }

    #[test]
    fn max_fitness_score_accounts_for_every_scored_module() {
        for v in 1..=40 {
            let ver = Version::Normal(v);
            let grid_size = ver.width() as i32;
            let expected_mods = 49 * 3                                  // 3 finders
                + (grid_size - 14) * 2                                  // 2 timing patterns
                + 25 * spec_alignment_count(v) as i32; // alignment patterns
            assert_eq!(
                max_fitness_score(ver),
                expected_mods * 9,
                "version {v}: max_fitness_score disagrees with what symbol_fitness scores"
            );
        }
    }
}
