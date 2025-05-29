#![allow(
    clippy::items_after_test_module,
    // unused_variables,
    // dead_code,
    mixed_script_confusables,
    clippy::suspicious_arithmetic_impl,
    clippy::suspicious_op_assign_impl
)]

pub mod builder;
pub(crate) mod common;
pub mod reader;

pub use builder::QRBuilder;
pub use common::mask::MaskPattern;
pub use common::metadata::{ECLevel, Palette, Version};
pub(crate) use common::*;
pub use reader::QRReader;

#[cfg(test)]
pub(crate) use builder::Module;
