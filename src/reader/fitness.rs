use super::utils::{geometry::Point, homography::Homography};
use crate::utils::QRResult;

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
