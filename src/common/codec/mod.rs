pub mod decoder;
pub mod encoder;
pub mod types;

pub use decoder::*;
pub use encoder::*;
pub use types::*;

// Codec proptesting
//------------------------------------------------------------------------------

// TODO: Update the testcase to accept String data as input
// #[cfg(test)]
// mod codec_proptests {
//
//     use proptest::prelude::*;
//
//     use super::{decode, encode_with_version};
//     use crate::metadata::{ECLevel, Palette, Version};
//
//     pub fn version_strategy() -> impl Strategy<Value = Version> {
//         (1usize..=40).prop_map(Version::Normal)
//     }
//
//     pub fn ec_level_strategy() -> BoxedStrategy<ECLevel> {
//         prop_oneof![Just(ECLevel::L), Just(ECLevel::M), Just(ECLevel::Q), Just(ECLevel::H)].boxed()
//     }
//     pub fn palette_strategy() -> BoxedStrategy<Palette> {
//         prop_oneof![Just(Palette::Mono), Just(Palette::Poly)].boxed()
//     }
//     pub fn codec_strategy() -> impl Strategy<Value = (Version, ECLevel, Palette, Vec<u8>)> {
//         (version_strategy(), ec_level_strategy(), palette_strategy()).prop_flat_map(
//             |(ver, ecl, pal)| {
//                 prop::collection::vec(any::<u8>(), 1..(ver.data_capacity(ecl, pal) - 3))
//                     .prop_map(move |data| (ver, ecl, pal, data))
//             },
//         )
//     }
//
//     proptest! {
//         #[test]
//         fn proptest_codec(params in codec_strategy()) {
//             let (ver, ecl, pal, data) = params;
//             let mut encoded = encode_with_version(&data, ver, ecl, pal).unwrap();
//             let decoded = decode(&mut encoded, ver, ecl, pal).unwrap();
//             prop_assert_eq!(data, decoded);
//         }
//     }
// }
