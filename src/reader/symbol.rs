use super::{
    deqr_temp::DeQR,
    utils::geometry::{Homography, Point},
};
use crate::Version;

// Finder line
//------------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct Symbol<'a> {
    deqr: &'a DeQR,
    homography: Homography,
    bounds: [Point; 4],
    ver: Version,
}
