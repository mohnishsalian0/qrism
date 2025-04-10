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

// Error correction proptesting
//------------------------------------------------------------------------------

#[cfg(test)]
mod error_correction_proptests {

    use proptest::prelude::*;

    use super::Block;

    pub fn block_strategy() -> impl Strategy<Value = (Vec<u8>, usize)> {
        (1usize..128).prop_flat_map(|dlen| {
            let data = prop::collection::vec(any::<u8>(), dlen);
            let ec_len = (1usize..32).prop_map(|el| el * 2);
            (data, ec_len)
        })
    }

    proptest! {
        #[test]
        fn proptest_ec((data, ec_len) in block_strategy()) {
            let len = data.len() + ec_len;
            let mut blk = Block::new(&data, len);

            use rand::{seq::IteratorRandom, rng};
            let t = ec_len / 2;
            let mut rng = rng();
            let corrupt_indices = (0..len).choose_multiple(&mut rng, t);

            for i in corrupt_indices {
                blk.full_mut()[i] ^= 0xFF;
            }

            let rectified = blk.rectify();
            prop_assert!(rectified.is_ok());
            prop_assert_eq!(rectified.unwrap(), data);
        }
    }
}
