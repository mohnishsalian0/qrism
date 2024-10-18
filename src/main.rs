// TODO: Remember to remove unused_variables & dead_code
#![allow(clippy::items_after_test_module, unused_variables, dead_code)]

use std::error::Error;

pub mod encode;
pub mod mask;
pub mod render;
pub mod types;

// TODO: Remove rqrr and clean up main function after testing
fn main() -> Result<(), Box<dyn Error>> {
    let img = image::open("assets/test_qr_5.png")?.to_luma8();
    // Prepare for detection
    let mut img = rqrr::PreparedImage::prepare(img);
    // Search for grids, without decoding
    let grids = img.detect_grids();
    assert_eq!(grids.len(), 1);
    // Decode the grid
    let (meta, content) = grids[0].decode()?;
    println!("Meta: {:?}", meta);
    println!("Content: {}", content);
    Ok(())
}
