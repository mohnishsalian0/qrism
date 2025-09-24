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
//     use crate::metadata::{ECLevel,  Version};
//
//     pub fn version_strategy() -> impl Strategy<Value = Version> {
//         (1usize..=40).prop_map(Version::Normal)
//     }
//
//     pub fn ec_level_strategy() -> BoxedStrategy<ECLevel> {
//         prop_oneof![Just(ECLevel::L), Just(ECLevel::M), Just(ECLevel::Q), Just(ECLevel::H)].boxed()
//     }
//     pub fn capacity_strategy() -> BoxedStrategy<bool> {
//         prop_oneof![Just(false), Just(true)].boxed()
//     }
//     pub fn codec_strategy() -> impl Strategy<Value = (Version, ECLevel, bool, Vec<u8>)> {
//         (version_strategy(), ec_level_strategy(), capacity_strategy()).prop_flat_map(
//             |(ver, ecl, hi_cap)| {
//                 prop::collection::vec(any::<u8>(), 1..(ver.data_capacity(ecl, hi_cap) - 3))
//                     .prop_map(move |data| (ver, ecl, hi_cap, data))
//             },
//         )
//     }
//
//     proptest! {
//         #[test]
//         fn proptest_codec(params in codec_strategy()) {
//             let (ver, ecl, hi_cap, data) = params;
//             let mut encoded = encode_with_version(&data, ver, ecl, hi_cap).unwrap();
//             let decoded = decode(&mut encoded, ver, ecl, hi_cap).unwrap();
//             prop_assert_eq!(data, decoded);
//         }
//     }
// }

#[cfg(test)]
mod codec_tests {
    use crate::ECLevel;

    use super::{decode, encode};

    #[test]
    fn test_codec_0() {
        let data = "585627067124432801036103098250466587082098029070289300241937719552437691575758733990547740427060786150911824925218003093789713212340411443393453215736157098813647870370509699666994642172773053091686025211301162247120566590041376085629647328730765988396689756357154425367242929221144994350535936221971939862436426382195503314392694830527873017681509101063367239953387016735772250128493546221677430709324753049832805787576927081397983420929208781413727502534110683825359131552802354813122716205595937448260030931653748316829394915867465670210517662573074777412441734453277711863202037758141390937485469501821225579494064439085907172027191340811777551424268728445718136306108402432567238996727775132716127240779175063531852394713818093707330300955495233249170660080756396523604545527328928195380162726259100822815594225239691049083204542665311151952515630445918551258222995302755008018593184200107470441720219199158988429795069275279042168214162459235148763271872680724594555599146074347024055268560549240864096855985858687549424188464728754234506826883995610401307977214772508022291788532304307729364126297258902907158750269554024753572118965890761497045194839854960544561556625280899085580947630085874600874209662749852830163793054862534236587544555353618781309788942121079535132463075630530068641084725536491919282641198144657547101028410410474490649931208989584229601468833705028509118635595372780151790414330642".to_string();
        let ecl = ECLevel::H;
        let hi_cap = true;

        let (mut enc, ver) = encode(data.as_bytes(), ecl, hi_cap).unwrap();

        let decoded = decode(&mut enc, ver, ecl, hi_cap).unwrap();

        assert_eq!(decoded, data);
    }
}
