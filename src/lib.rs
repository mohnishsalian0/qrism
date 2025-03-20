// TODO: Remember to remove unused_variables & dead_code
#![allow(clippy::items_after_test_module, unused_variables, dead_code, mixed_script_confusables)]

pub mod builder;
mod common;
pub mod reader;

pub use builder::QRBuilder;
pub use common::error::*;
pub use common::metadata::{ECLevel, Palette, Version};
pub use reader::QRReader;
