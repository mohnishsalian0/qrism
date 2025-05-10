pub mod accumulate;
pub mod geometry;

#[cfg(test)]
use image::RgbImage;

#[cfg(test)]
pub trait Highlight {
    fn highlight(&self, img: &mut RgbImage);
}
