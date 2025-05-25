#[cfg(test)]
mod qr_proptests {

    use prop::string::string_regex;
    use proptest::prelude::*;

    use qrism::*;

    pub fn ec_level_strategy() -> BoxedStrategy<ECLevel> {
        prop_oneof![Just(ECLevel::L), Just(ECLevel::M), Just(ECLevel::Q), Just(ECLevel::H)].boxed()
    }

    pub fn palette_strategy() -> BoxedStrategy<Palette> {
        prop_oneof![Just(Palette::Mono), Just(Palette::Poly)].boxed()
    }

    pub fn qr_strategy(regex: String) -> impl Strategy<Value = (ECLevel, Palette, String)> {
        (ec_level_strategy(), palette_strategy()).prop_flat_map(move |(ecl, pal)| {
            let mut max_sz = match ecl {
                ECLevel::L => 2956,
                ECLevel::M => 2334,
                ECLevel::Q => 1656,
                ECLevel::H => 1276,
            };
            if matches!(pal, Palette::Poly) {
                max_sz *= 3;
            }
            let pattern = format!(r"{}{{1,{}}}", regex, max_sz);
            string_regex(&pattern).unwrap().prop_map(move |data| (ecl, pal, data))
        })
    }

    proptest! {
        #[test]
        #[ignore]
        fn proptest_numeric(params in qr_strategy("[0-9]".to_string())) {
            let (ecl, pal, data) = params;

            let qr = QRBuilder::new(data.as_bytes()).ec_level(ecl).palette(pal).build().unwrap().to_image(3);

            let decoded = QRReader::read(&qr).unwrap();

            prop_assert_eq!(data, decoded);
        }

        #[test]
        #[ignore]
        fn proptest_alphanumeric(params in qr_strategy(r"[0-9A-Z $%*+\-./:]".to_string())) {
            let (ecl, pal, data) = params;

            let qr = QRBuilder::new(data.as_bytes()).ec_level(ecl).palette(pal).build().unwrap().to_image(3);

            let decoded = QRReader::read(&qr).unwrap();

            prop_assert_eq!(data, decoded);
        }
    }
}

#[cfg(test)]
mod qr_tests {
    use test_case::test_case;

    use qrism::{ECLevel, Palette, QRBuilder, QRReader, Version};

    #[test_case("Hello, world!🌎".to_string(), Version::Normal(1), ECLevel::L, Palette::Mono)]
    #[test_case("TEST".to_string(), Version::Normal(1), ECLevel::M, Palette::Poly)]
    #[test_case("12345".to_string(), Version::Normal(1), ECLevel::Q, Palette::Mono)]
    #[test_case("OK".to_string(), Version::Normal(1), ECLevel::H, Palette::Poly)]
    #[test_case("B3@j🎮#Z%8v🍣K!🔑3zC^8📖&r💾F9*🔐b6🌼".repeat(3).to_string(), Version::Normal(7), ECLevel::L, Palette::Mono)]
    #[test_case("A11111111111111".repeat(11).to_string(), Version::Normal(7), ECLevel::M, Palette::Poly)]
    #[test_case("aAAAAAA1111111111111AAAAAAa".repeat(3).to_string(), Version::Normal(7), ECLevel::Q, Palette::Mono)]
    #[test_case("1234567890".repeat(15).to_string(), Version::Normal(7), ECLevel::H, Palette::Poly)]
    #[test_case( "B3@j🎮#Z%8v🍣K!🔑3zC^8📖&r💾F9*🔐b6🌼".repeat(4).to_string(), Version::Normal(10), ECLevel::L, Palette::Mono)]
    #[test_case("A11111111111111".repeat(20).to_string(), Version::Normal(10), ECLevel::M, Palette::Poly)]
    #[test_case("aAAAAAAAAA1111111111111111AAAAAAAAAAa".repeat(4).to_string(), Version::Normal(10), ECLevel::Q, Palette::Mono)]
    #[test_case("1234567890".repeat(28).to_string(), Version::Normal(10), ECLevel::H, Palette::Poly)]
    #[test_case("B3@j🎮#Z%8v🍣K!🔑3zC^8📖&r💾F9*🔐b6🌼".repeat(22).to_string(), Version::Normal(27), ECLevel::L, Palette::Mono)]
    #[test_case("A111111111111111".repeat(100).to_string(), Version::Normal(27), ECLevel::M, Palette::Poly)]
    #[test_case("aAAAAAAAAA111111111111111111AAAAAAAAAAa".repeat(20).to_string(), Version::Normal(27), ECLevel::Q, Palette::Mono)]
    #[test_case("1234567890".repeat(145).to_string(), Version::Normal(27), ECLevel::H, Palette::Poly)]
    #[test_case("B3@j🎮#Z%8v🍣K!🔑3zC^8📖&r💾F9*🔐b6🌼".repeat(57).to_string(), Version::Normal(40), ECLevel::L, Palette::Mono)]
    #[test_case("A111111111111111".repeat(97).to_string(), Version::Normal(40), ECLevel::M, Palette::Poly)]
    #[test_case( "aAAAAAAAAA111111111111111111AAAAAAAAAAa".repeat(42).to_string(), Version::Normal(40), ECLevel::Q, Palette::Mono)]
    #[test_case("1234567890".repeat(305).to_string(), Version::Normal(40), ECLevel::H, Palette::Poly)]
    #[test_case("aA00000298542515764186657331886415260738886433510273480049978764987230758543376676257538587037428591120694472658573041637".to_string(), Version::Normal(3), ECLevel::L, Palette::Mono)]
    #[test_case("aA000000000000000000000000000000000000000000000000000000000000000000000".to_string(), Version::Normal(1), ECLevel::L, Palette::Poly)]
    fn test_qr(data: String, ver: Version, ecl: ECLevel, pal: Palette) {
        let qr = QRBuilder::new(data.as_bytes())
            .version(ver)
            .ec_level(ecl)
            .palette(pal)
            .build()
            .unwrap()
            .to_image(3);

        let decoded_data = QRReader::read(&qr).unwrap();

        assert_eq!(decoded_data, data);
    }

    #[test]
    fn test_qr_0() {
        let data = "000003102240522040101032134589200040100032256802000001000230031030100051322320302010102287757583444005058202946794230192593114436932953370175316685191098675305648442486981451187345202833326821009949644832254029455434265792710428622979190276282956185887462621840559174608893562970842263910702908981904037304248915".to_string();
        let ecl = ECLevel::M;
        let pal = Palette::Mono;

        let qr =
            QRBuilder::new(data.as_bytes()).ec_level(ecl).palette(pal).build().unwrap().to_image(3);

        let decoded_data = QRReader::read(&qr).unwrap();

        assert_eq!(decoded_data, data);
    }

    #[test]
    fn test_qr_1() {
        let data = "130407959251346537095501374241020273428824333989563193855535638107600416445913821746745354008914879409621312794504288844011950994664079308679632820857446749249296922551100509857072228783442790500176471052053566270490922801161101332003314337857300511229738236935140276430283311100418702091322703565668310440281208001472027142138615268130844063977580966581911733846481301272053468332549112870826140051242250962248961400024759818400200613548627896632282389857416436911017502190391328417855690473946582177335355321332502687726749523678190316089539262695579440353377466238166737471863276158242725901013947043399124870408386765574199374381085772408323388544103850674918055686641107570070490918055847388763139939189764147643069160097201288940776299996547638723098727866719626951070511076399537294362279102063475533164652744796958426936041570276326121813882169664842422143150120911024315633904428045414321933910477934641596737303770194304834698231320952899497150572655722499133070125387907536118725556970052610786417346800408724300416027846485818576698646266066465318116479894692152617904231953987787749994668965563916076848594702477192860080535740345826072395544579172057523721218500150094712459445338182673048928761325581549113174600372521673706145373125049020083684078589927530313160511435162548996357179334777031491972039471964818522756277093342921714508302453842522218264287343199836868382272592134982404699584229941095927744568140246732208792373216597637417520801698911645320809550040063469116344961191946640009198737625278779470780099345346724028056805158744067758112281968811143991719285716307104026788184839290477319741983506875958143341212950635468327886200702567336277344502937088841965979424042452945706987677338791433591341658552528705965780184367137835853898976485805055066356827659810067597770508740362740988198253722276907856092616368445318718452454653869175298".to_string();
        let ecl = ECLevel::M;
        let pal = Palette::Mono;

        let qr =
            QRBuilder::new(data.as_bytes()).ec_level(ecl).palette(pal).build().unwrap().to_image(3);

        let decoded_data = QRReader::read(&qr).unwrap();

        assert_eq!(decoded_data, data);
    }

    #[test]
    fn test_qr_2() {
        let data = "281484657979052403538715238673525721073084313746809826278238300097167415053049664415058765376542899803642924727764137068786912056814981365935168105423538225958356384667886269217413219336061345740628422634317190984762892319005916987945728143413468180351849398628140520366009336791583771362284370446274675391085928310320059744198843790808888699302232764138596536539970008561431870548756990191551282254270593057722030333227198211564784865925179060967108433382659253094240462542743668068863467896165767222906416296740791468943222863398724336706377018783941136745007710037278803537374114598057922770876326356224653120126178194659392226772305460771511283508829005441670240933097793391400080774106048976304331035126423450440935260910729325559176566024083489315334968064602629222509227148987079776744219045245925654049984739031222687026850217736165704095346872519670467451830385229734".to_string();
        let ecl = ECLevel::Q;
        let pal = Palette::Mono;

        let qr =
            QRBuilder::new(data.as_bytes()).ec_level(ecl).palette(pal).build().unwrap().to_image(3);

        let path = std::path::Path::new("assets/built.png");
        qr.save(path).unwrap();

        let decoded_data = QRReader::read(&qr).unwrap();

        assert_eq!(decoded_data, data);
    }

    #[test]
    #[ignore]
    fn test_qr_detection() {
        use std::io::Write;

        let file_counts = [20, 36, 42, 48, 19, 15]; // Count of qrs in 6 folders
        let total = file_counts.iter().sum::<u32>();
        let mut passed = 0;
        let mut out_file = std::fs::File::create("tests/images/result.txt").unwrap();

        for (i, file_count) in file_counts.iter().enumerate() {
            let folder_id = i + 1;

            for qr_id in 1..=*file_count {
                let img_path_str = format!("tests/images/qrcode-{folder_id}/{qr_id}.png");
                let img_path = std::path::Path::new(&img_path_str);
                let img = image::open(img_path).unwrap().to_luma8(); // WARN: Grayscale image

                // let err_msg = format!("Failed to read QR from: {}", &img_path_str);
                // let msg = QRReader::read(img).expect(&err_msg);
                // let msg = match QRReader::read(img) {
                //     Ok(msg) => msg,
                //     Err(_) => continue,
                // };

                let msg = match std::panic::catch_unwind(|| QRReader::read(&img)) {
                    Ok(Ok(msg)) => {
                        let msg_path_str = format!("tests/images/qrcode-{folder_id}/{qr_id}.txt");
                        let msg_path = std::path::Path::new(&msg_path_str);
                        let exp_msg = std::fs::read_to_string(msg_path).unwrap();
                        let exp_msg = exp_msg.replace("\r\n", "\n");
                        let _ = if msg == exp_msg {
                            writeln!(out_file, "[{}-{}] PASSED", folder_id, qr_id)
                        } else {
                            writeln!(out_file, "[{}-{}] DECODED", folder_id, qr_id)
                        };
                        msg
                    }
                    Ok(Err(e)) => {
                        let _ = writeln!(out_file, "[{}-{}] {}", folder_id, qr_id, e);
                        continue;
                    }
                    Err(e) => {
                        let panic_msg = if let Some(s) = e.downcast_ref::<&str>() {
                            *s
                        } else if let Some(s) = e.downcast_ref::<String>() {
                            s.as_str()
                        } else {
                            "Unknown panic"
                        };
                        let _ = writeln!(out_file, "[{}-{}] {}", folder_id, qr_id, panic_msg);
                        continue;
                    }
                };

                let msg_path_str = format!("tests/images/qrcode-{folder_id}/{qr_id}.txt");
                let msg_path = std::path::Path::new(&msg_path_str);
                let exp_msg = std::fs::read_to_string(msg_path).unwrap();

                if msg == exp_msg {
                    passed += 1;
                }

                // assert_eq!(
                //     exp_msg, msg,
                //     "Failed to read QR from file {qr_id} in folder {folder_id}"
                // );
            }
            let _ = writeln!(out_file);
        }
        let percentage = passed * 100 / total;
        assert_eq!(passed, total, "Passed: {passed} out of {total} ({percentage}%)");
    }
}
