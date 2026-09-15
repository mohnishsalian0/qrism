use super::{
    alignment::Anchors,
    utils::{geometry::Point, homography::Homography},
};
use crate::{utils::QRResult, Version};

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

    #[cfg(test)]
    #[inline]
    pub(super) fn contains(&self, x: u32, y: u32) -> bool {
        (self.x0..self.x1).contains(&x) && (self.y0..self.y1).contains(&y)
    }

    #[inline]
    pub(super) fn map(&self, x: f64, y: f64) -> QRResult<Point> {
        self.h.map(x, y)
    }

    pub(super) fn exact_map(&self, x: f64, y: f64) -> QRResult<(f64, f64)> {
        self.h.exact_map(x, y)
    }

    // The tile's outline in image space, TL, TR, BR, BL
    #[cfg(test)]
    pub(super) fn corners(&self) -> QRResult<[Point; 4]> {
        let (x0, y0) = (self.x0 as f64, self.y0 as f64);
        let (x1, y1) = (self.x1 as f64 + 1.0, self.y1 as f64 + 1.0);
        Ok([self.h.map(x0, y0)?, self.h.map(x1, y0)?, self.h.map(x1, y1)?, self.h.map(x0, y1)?])
    }
}

// Symbol tiles & band
//------------------------------------------------------------------------------

// Module space centre of the alignment pattern
// Note: the three cells coinciding with finders carry the finder centre
fn anchor_coord(ver: Version, row: usize, col: usize) -> (f64, f64) {
    let aps = ver.alignment_pattern();
    let last_cell = aps.len().max(2) - 1;
    let w = ver.width() as f64;

    if [(0, 0), (0, last_cell), (last_cell, 0)].contains(&(row, col)) {
        let x = if col == 0 { 3.5 } else { w - 3.5 };
        let y = if row == 0 { 3.5 } else { w - 3.5 };
        (x, y)
    } else if aps.is_empty() {
        // Handles v1
        (w - 3.5, w - 3.5)
    } else {
        (aps[col] as f64 + 0.5, aps[row] as f64 + 0.5)
    }
}

// Image space centres in the order: TL, TR, BR, BL -- or `None` if any of
// the four went unlocated. A tile is only as good as its worst corner.
fn tile_centres(centres: &Anchors, row: usize, col: usize) -> Option<[Point; 4]> {
    Some([
        centres[row][col]?,
        centres[row][col + 1]?,
        centres[row + 1][col + 1]?,
        centres[row + 1][col]?,
    ])
}

// Splits the symbol into a grid of tiles, one per cell of the alignment grid, each carrying a
// homography fitted to the four located centres at its own corners.
//
// A tile spans from one alignment coordinate to the next, but the outermost tiles run out to the
// symbol edge rather than stopping at their anchor -- the top-left tile is anchored on a finder
// centre at module 3 yet owns every module from 0. A tile with an unlocated corner is left `None`.
//
// Versions carrying no alignment patterns yield no tiles.
pub(super) fn build_tiles(ver: Version, centres: &Anchors) -> [[Option<Tile>; 6]; 6] {
    let mut tiles: [[Option<Tile>; 6]; 6] = Default::default();

    let aps = ver.alignment_pattern().iter().map(|&ap| ap as u32).collect::<Vec<_>>();

    let w = ver.width() as u32;
    let last_tile = aps.len().max(2) - 2; // Last tile of v1 is 0

    for row in 0..=last_tile {
        for col in 0..=last_tile {
            let Some(quad) = tile_centres(centres, row, col) else { continue };

            // Half-open module bounds. The outer edges reach the symbol boundary; the inner ones
            // stop on the next alignment coordinate, which the neighbouring
            let x0 = if col == 0 { 0 } else { aps[col] };
            let x1 = if col == last_tile { w } else { aps[col + 1] };
            let y0 = if row == 0 { 0 } else { aps[row] };
            let y1 = if row == last_tile { w } else { aps[row + 1] };

            let anchors = [
                anchor_coord(ver, row, col),
                anchor_coord(ver, row, col + 1),
                anchor_coord(ver, row + 1, col + 1),
                anchor_coord(ver, row + 1, col),
            ];

            tiles[row][col] = Tile::new((x0, y0, x1, y1), anchors, quad).ok();
        }
    }

    tiles
}

// Maps each module coordinate to the index of the tile owning it along that axis. The symbol is
// square and both axes use the same alignment coordinates, so one table serves x and y alike.
pub(super) fn band_table(ver: Version) -> [u8; MAX_WIDTH] {
    let w = ver.width();
    let aps = ver.alignment_pattern();
    let n = aps.len().max(2);
    let interior = aps.get(1..n - 1).unwrap_or(&[]);

    let mut table = [u8::MAX; MAX_WIDTH];
    let mut band = 0usize;

    for (m, slot) in table.iter_mut().enumerate().take(w) {
        if interior.get(band).is_some_and(|&e| m as i32 >= e) {
            band += 1;
        }
        *slot = band as u8;
    }

    table
}

#[cfg(test)]
mod tile_tests {
    use super::{build_tiles, Anchors};
    use crate::metadata::Version;
    use crate::reader::alignment::MAX_ALIGN_CELLS;
    use crate::reader::utils::geometry::Point;

    const KX: f64 = 12.0; // Pixels per module, x
    const KY: f64 = 16.0; // Pixels per module, y
    const Q: f64 = 4.0; // Quiet zone, modules

    // Ground-truth placement of the symbol in an image, the same way this file's other tests do it.
    //
    // The two axes scale differently on purpose: at equal scales a row/column transposition inside
    // `build_tiles` maps onto itself and becomes invisible. Both scales are even, so a module
    // *centre* -- carrying its half-module offset -- still lands on a whole pixel. That keeps the
    // fitted homography exact and lets the reprojection check below run at zero tolerance.
    fn project(x: f64, y: f64) -> (f64, f64) {
        ((x + Q) * KX, (y + Q) * KY)
    }

    // Module-space centre of the pattern at grid cell (row, col), restated from the spec rather
    // than taken from `anchor_coord` -- a fixture that calls the code under test cannot catch it
    // being wrong. Finder cells sit on module 3 or w - 4; every other cell on its alignment
    // coordinate.
    fn cell_centre(ver: Version, row: usize, col: usize) -> (f64, f64) {
        let aps = ver.alignment_pattern();
        let last = aps.len().max(2) - 1;
        let w = ver.width() as f64;

        if [(0, 0), (0, last), (last, 0)].contains(&(row, col)) {
            let x = if col == 0 { 3.5 } else { w - 3.5 };
            let y = if row == 0 { 3.5 } else { w - 3.5 };
            (x, y)
        } else if aps.is_empty() {
            (w - 3.5, w - 3.5)
        } else {
            (aps[col] as f64 + 0.5, aps[row] as f64 + 0.5)
        }
    }

    // A fully located grid: every alignment centre present, placed by `project`.
    fn synthetic_centres(ver: Version) -> Anchors {
        let n = ver.alignment_pattern().len().max(2);
        let mut centres: Anchors = [[None; MAX_ALIGN_CELLS]; MAX_ALIGN_CELLS];

        for (row, centres_row) in centres.iter_mut().enumerate().take(n) {
            for (col, centre) in centres_row.iter_mut().enumerate().take(n) {
                let (mx, my) = cell_centre(ver, row, col);
                let (px, py) = project(mx, my);
                *centre = Some(Point { x: px as i32, y: py as i32 });
            }
        }
        centres
    }

    // Every cell of the alignment grid must yield a tile when all four of its corners are
    // located, and no cell outside that block may.
    #[test]
    fn build_tiles_fills_the_alignment_grid() {
        for v in 1..=40usize {
            let ver = Version::Normal(v);
            let n = ver.alignment_pattern().len().max(2);
            let expected = (n - 1) * (n - 1);

            let tiles = build_tiles(ver, &synthetic_centres(ver));

            let built = tiles.iter().flatten().filter(|t| t.is_some()).count();
            assert_eq!(built, expected, "version {v}: wrong number of tiles built");

            // The right count is not enough -- they have to be the right cells.
            for (row, tile_row) in tiles.iter().enumerate() {
                for (col, tile) in tile_row.iter().enumerate() {
                    let want = row < n - 1 && col < n - 1;
                    assert_eq!(
                        tile.is_some(),
                        want,
                        "version {v}: tile ({row}, {col}) is {}, expected {}",
                        if tile.is_some() { "built" } else { "absent" },
                        if want { "built" } else { "absent" },
                    );
                }
            }
        }
    }

    // Every module of the symbol must be owned by exactly one tile, and by the specific tile whose
    // band it falls in
    #[test]
    fn tile_bounds_partition_the_symbol() {
        for v in 1..=40usize {
            let ver = Version::Normal(v);
            let aps = ver.alignment_pattern();
            let n = aps.len().max(2);
            let w = ver.width() as u32;

            let tiles = build_tiles(ver, &synthetic_centres(ver));

            // Tile boundaries along either axis: the symbol edge, the interior alignment
            // coordinates, then the far edge. n boundaries enclose the n - 1 tiles on that axis.
            let mut edges = Vec::with_capacity(n);
            edges.push(0);
            edges.extend(aps.get(1..n - 1).unwrap_or(&[]).iter().map(|&ap| ap as u32));
            edges.push(w);

            // Band index of each module along one axis, precomputed once and shared by both.
            let bands: Vec<usize> = (0..w)
                .map(|m| {
                    edges.windows(2).position(|e| e[0] <= m && m < e[1]).unwrap_or_else(|| {
                        panic!("version {v}: module {m} falls outside every band")
                    })
                })
                .collect();

            for y in 0..w {
                let row = bands[y as usize];
                for x in 0..w {
                    let col = bands[x as usize];
                    let mut owners = 0;

                    for (r, tile_row) in tiles.iter().enumerate() {
                        for (c, tile) in tile_row.iter().enumerate() {
                            if tile.as_ref().is_some_and(|t| t.contains(x, y)) {
                                owners += 1;
                                assert_eq!(
                                    (r, c),
                                    (row, col),
                                    "version {v}: module ({x}, {y}) claimed by tile ({r}, {c})"
                                );
                            }
                        }
                    }

                    assert_eq!(
                        owners, 1,
                        "version {v}: module ({x}, {y}) owned by {owners} tiles, expected 1"
                    );
                }
            }
        }
    }

    // Every tile must reproduce the projection of every module it owns -- not merely at those corners.
    #[test]
    fn tile_homographies_reproduce_the_projection() {
        for v in 1..=40usize {
            let ver = Version::Normal(v);
            let w = ver.width() as u32;
            let tiles = build_tiles(ver, &synthetic_centres(ver));

            for y in 0..w {
                for x in 0..w {
                    let Some(tile) = tiles.iter().flatten().flatten().find(|t| t.contains(x, y))
                    else {
                        panic!("version {v}: module ({x}, {y}) is owned by no tile");
                    };

                    // Module centres -- the coordinates an actual lookup passes in.
                    let (mx, my) = (x as f64 + 0.5, y as f64 + 0.5);
                    let (ex, ey) = project(mx, my);
                    let got = tile.map(mx, my).expect("tile projection failed");

                    assert_eq!(
                        (got.x, got.y),
                        (ex as i32, ey as i32),
                        "version {v}: module ({x}, {y}) projects to ({}, {}) but should sit at \
                         ({ex}, {ey})",
                        got.x,
                        got.y
                    );
                }
            }
        }
    }
}

// Global constants
//------------------------------------------------------------------------------

pub(super) const MAX_WIDTH: usize = 177;
