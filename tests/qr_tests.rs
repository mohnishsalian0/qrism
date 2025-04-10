#[cfg(test)]
mod qr_proptests {

    use prop::string::string_regex;
    use proptest::prelude::*;

    use qr_pro_max::*;

    pub fn version_strategy() -> impl Strategy<Value = Version> {
        (1usize..=40).prop_map(Version::Normal)
    }

    pub fn ec_level_strategy() -> BoxedStrategy<ECLevel> {
        prop_oneof![Just(ECLevel::L), Just(ECLevel::M), Just(ECLevel::Q), Just(ECLevel::H)].boxed()
    }
    pub fn palette_strategy() -> BoxedStrategy<Palette> {
        prop_oneof![Just(Palette::Mono), Just(Palette::Poly)].boxed()
    }
    pub fn qr_strategy() -> impl Strategy<Value = (Version, ECLevel, Palette, String)> {
        (version_strategy(), ec_level_strategy(), palette_strategy()).prop_flat_map(
            |(ver, ecl, pal)| {
                // let pattern = format!(r"[^\p{{Cc}}]{{1,{}}}", ver.data_capacity(ecl, pal));
                let pattern = format!(r"[ -~]{{1,{}}}", ver.data_capacity(ecl, pal));
                string_regex(&pattern).unwrap().prop_map(move |data| (ver, ecl, pal, data))
            },
        )
    }

    proptest! {
        #[test]
        fn proptest_qr(params in qr_strategy()) {
            let (ver, ecl, pal, data) = params;
            let qr = QRBuilder::new(data.as_bytes()).version(ver).ec_level(ecl).palette(pal).build().unwrap();
            let qr_img = qr.render_color(10);
            let decoded = QRReader::read(&qr_img, ver).unwrap();
            prop_assert_eq!(data, decoded);
        }
    }
}
