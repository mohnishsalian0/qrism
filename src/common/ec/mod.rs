mod decoder;
mod encoder;

mod block;
mod galois;

pub(crate) use block::*;
pub(crate) use decoder::*;

// FIXME: Remove
pub mod ec_bench;
pub mod version_db;

pub static MAX_BLOCK_SIZE: usize = 256;

pub static MAX_EC_SIZE: usize = 64;
