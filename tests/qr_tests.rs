#[cfg(test)]
mod qr_proptests {

    use prop::string::string_regex;
    use proptest::prelude::*;

    use qrism::*;
    use reader::{detect_hc_qr, detect_qr};

    pub fn ec_level_strategy() -> BoxedStrategy<ECLevel> {
        prop_oneof![Just(ECLevel::L), Just(ECLevel::M), Just(ECLevel::Q), Just(ECLevel::H)].boxed()
    }

    pub fn capacity_strategy() -> BoxedStrategy<bool> {
        prop_oneof![Just(false), Just(true)].boxed()
    }

    pub fn qr_strategy(regex: String) -> impl Strategy<Value = (ECLevel, bool, String)> {
        (ec_level_strategy(), capacity_strategy()).prop_flat_map(move |(ecl, hi_cap)| {
            let mut max_sz = match ecl {
                ECLevel::L => 2956,
                ECLevel::M => 2334,
                ECLevel::Q => 1656,
                ECLevel::H => 1276,
            };
            if hi_cap {
                max_sz *= 3;
            }
            let pattern = format!(r"{}{{1,{}}}", regex, max_sz);
            string_regex(&pattern).unwrap().prop_map(move |data| (ecl, hi_cap, data))
        })
    }

    proptest! {
        #[test]
        #[ignore]
        fn proptest_numeric(params in qr_strategy("[0-9]".to_string())) {
            let (ecl, hi_cap, data) = params;

            let qr = QRBuilder::new(data.as_bytes()).ec_level(ecl).high_capacity(hi_cap).build().unwrap();

            let img = image::DynamicImage::ImageRgb8(qr.to_image(3));
            let mut res = if hi_cap { detect_hc_qr(&img) } else {detect_qr(&img)};
            let (_meta, decoded) = res.symbols()[0].decode().expect("Failed to read QR");

            prop_assert_eq!(data, decoded);
        }

        #[test]
        #[ignore]
        fn proptest_alphanumeric(params in qr_strategy(r"[0-9A-Z $%*+\-./:]".to_string())) {
            let (ecl, hi_cap, data) = params;

            let qr = QRBuilder::new(data.as_bytes()).ec_level(ecl).high_capacity(hi_cap).build().unwrap();

            let img = image::DynamicImage::ImageRgb8(qr.to_image(3));
            let mut res = if hi_cap { detect_hc_qr(&img) } else {detect_qr(&img)};
            let (_meta, decoded) = res.symbols()[0].decode().expect("Failed to read QR");

            prop_assert_eq!(data, decoded);
        }
    }
}

#[cfg(test)]
mod qr_tests {
    use test_case::test_case;

    use qrism::{
        reader::{detect_hc_qr, detect_qr},
        ECLevel, QRBuilder, Version,
    };

    #[test_case("Hello, world!🌎".to_string(), Version::Normal(1), ECLevel::L, false; "test_qr_1")]
    #[test_case("TEST".to_string(), Version::Normal(1), ECLevel::M, true; "test_qr_2")]
    #[test_case("12345".to_string(), Version::Normal(1), ECLevel::Q, false; "test_qr_3")]
    #[test_case("OK".to_string(), Version::Normal(1), ECLevel::H, true; "test_qr_4")]
    #[test_case("B3@j🎮#Z%8v🍣K!🔑3zC^8📖&r💾F9*🔐b6🌼".repeat(3).to_string(), Version::Normal(7), ECLevel::L, false; "test_qr_5")]
    #[test_case("A11111111111111".repeat(11).to_string(), Version::Normal(7), ECLevel::M, true; "test_qr_6")]
    #[test_case("aAAAAAA1111111111111AAAAAAa".repeat(3).to_string(), Version::Normal(7), ECLevel::Q, false; "test_qr_7")]
    #[test_case("1234567890".repeat(15).to_string(), Version::Normal(7), ECLevel::H, true; "test_qr_8")]
    #[test_case( "B3@j🎮#Z%8v🍣K!🔑3zC^8📖&r💾F9*🔐b6🌼".repeat(4).to_string(), Version::Normal(10), ECLevel::L, false; "test_qr_9")]
    #[test_case("A11111111111111".repeat(20).to_string(), Version::Normal(10), ECLevel::M, true; "test_qr_10")]
    #[test_case("aAAAAAAAAA1111111111111111AAAAAAAAAAa".repeat(4).to_string(), Version::Normal(10), ECLevel::Q, false; "test_qr_11")]
    #[test_case("1234567890".repeat(28).to_string(), Version::Normal(10), ECLevel::H, true; "test_qr_12")]
    #[test_case("B3@j🎮#Z%8v🍣K!🔑3zC^8📖&r💾F9*🔐b6🌼".repeat(22).to_string(), Version::Normal(27), ECLevel::L, false; "test_qr_13")]
    #[test_case("A111111111111111".repeat(100).to_string(), Version::Normal(27), ECLevel::M, true; "test_qr_14")]
    #[test_case("aAAAAAAAAA111111111111111111AAAAAAAAAAa".repeat(20).to_string(), Version::Normal(27), ECLevel::Q, false; "test_qr_15")]
    #[test_case("1234567890".repeat(145).to_string(), Version::Normal(27), ECLevel::H, true; "test_qr_16")]
    #[test_case("B3@j🎮#Z%8v🍣K!🔑3zC^8📖&r💾F9*🔐b6🌼".repeat(57).to_string(), Version::Normal(40), ECLevel::L, false; "test_qr_17")]
    #[test_case("A111111111111111".repeat(97).to_string(), Version::Normal(40), ECLevel::M, true; "test_qr_18")]
    #[test_case( "aAAAAAAAAA111111111111111111AAAAAAAAAAa".repeat(42).to_string(), Version::Normal(40), ECLevel::Q, false; "test_qr_19")]
    #[test_case("1234567890".repeat(305).to_string(), Version::Normal(40), ECLevel::H, true; "test_qr_20")]
    #[test_case("aA00000298542515764186657331886415260738886433510273480049978764987230758543376676257538587037428591120694472658573041637".to_string(), Version::Normal(3), ECLevel::L, false; "test_qr_21")]
    #[test_case("aA000000000000000000000000000000000000000000000000000000000000000000000".to_string(), Version::Normal(1), ECLevel::L, true; "test_qr_22")]
    fn test_qr(data: String, ver: Version, ecl: ECLevel, hi_cap: bool) {
        let qr = QRBuilder::new(data.as_bytes())
            .version(ver)
            .ec_level(ecl)
            .high_capacity(hi_cap)
            .build()
            .unwrap();

        let img = image::DynamicImage::ImageRgb8(qr.to_image(3));
        let mut res = if hi_cap { detect_hc_qr(&img) } else { detect_qr(&img) };
        let (_meta, decoded) = res.symbols()[0].decode().expect("Failed to read QR");

        assert_eq!(data, decoded);
    }

    #[test]
    fn test_qr_0() {
        let data = "000003102240522040101032134589200040100032256802000001000230031030100051322320302010102287757583444005058202946794230192593114436932953370175316685191098675305648442486981451187345202833326821009949644832254029455434265792710428622979190276282956185887462621840559174608893562970842263910702908981904037304248915".to_string();
        let ecl = ECLevel::M;
        let hi_cap = false;

        let qr =
            QRBuilder::new(data.as_bytes()).ec_level(ecl).high_capacity(hi_cap).build().unwrap();

        let img = image::DynamicImage::ImageRgb8(qr.to_image(3));
        let mut res = if hi_cap { detect_hc_qr(&img) } else { detect_qr(&img) };
        let (_meta, decoded) = res.symbols()[0].decode().expect("Failed to read QR");

        assert_eq!(data, decoded);
    }

    #[test]
    fn test_qr_1() {
        let data = "130407959251346537095501374241020273428824333989563193855535638107600416445913821746745354008914879409621312794504288844011950994664079308679632820857446749249296922551100509857072228783442790500176471052053566270490922801161101332003314337857300511229738236935140276430283311100418702091322703565668310440281208001472027142138615268130844063977580966581911733846481301272053468332549112870826140051242250962248961400024759818400200613548627896632282389857416436911017502190391328417855690473946582177335355321332502687726749523678190316089539262695579440353377466238166737471863276158242725901013947043399124870408386765574199374381085772408323388544103850674918055686641107570070490918055847388763139939189764147643069160097201288940776299996547638723098727866719626951070511076399537294362279102063475533164652744796958426936041570276326121813882169664842422143150120911024315633904428045414321933910477934641596737303770194304834698231320952899497150572655722499133070125387907536118725556970052610786417346800408724300416027846485818576698646266066465318116479894692152617904231953987787749994668965563916076848594702477192860080535740345826072395544579172057523721218500150094712459445338182673048928761325581549113174600372521673706145373125049020083684078589927530313160511435162548996357179334777031491972039471964818522756277093342921714508302453842522218264287343199836868382272592134982404699584229941095927744568140246732208792373216597637417520801698911645320809550040063469116344961191946640009198737625278779470780099345346724028056805158744067758112281968811143991719285716307104026788184839290477319741983506875958143341212950635468327886200702567336277344502937088841965979424042452945706987677338791433591341658552528705965780184367137835853898976485805055066356827659810067597770508740362740988198253722276907856092616368445318718452454653869175298".to_string();
        let ecl = ECLevel::M;
        let hi_cap = false;

        let qr =
            QRBuilder::new(data.as_bytes()).ec_level(ecl).high_capacity(hi_cap).build().unwrap();

        let img = image::DynamicImage::ImageRgb8(qr.to_image(3));
        let mut res = if hi_cap { detect_hc_qr(&img) } else { detect_qr(&img) };
        let (_meta, decoded) = res.symbols()[0].decode().expect("Failed to read QR");

        assert_eq!(data, decoded);
    }

    #[test]
    fn test_qr_2() {
        let data = "281484657979052403538715238673525721073084313746809826278238300097167415053049664415058765376542899803642924727764137068786912056814981365935168105423538225958356384667886269217413219336061345740628422634317190984762892319005916987945728143413468180351849398628140520366009336791583771362284370446274675391085928310320059744198843790808888699302232764138596536539970008561431870548756990191551282254270593057722030333227198211564784865925179060967108433382659253094240462542743668068863467896165767222906416296740791468943222863398724336706377018783941136745007710037278803537374114598057922770876326356224653120126178194659392226772305460771511283508829005441670240933097793391400080774106048976304331035126423450440935260910729325559176566024083489315334968064602629222509227148987079776744219045245925654049984739031222687026850217736165704095346872519670467451830385229734".to_string();
        let ecl = ECLevel::Q;
        let hi_cap = false;

        let qr =
            QRBuilder::new(data.as_bytes()).ec_level(ecl).high_capacity(hi_cap).build().unwrap();

        let img = image::DynamicImage::ImageRgb8(qr.to_image(3));
        let mut res = if hi_cap { detect_hc_qr(&img) } else { detect_qr(&img) };
        let (_meta, decoded) = res.symbols()[0].decode().expect("Failed to read QR");

        assert_eq!(data, decoded);
    }

    #[test]
    fn test_qr_3() {
        let data = "*0FK+/Y.3%.-Q*/+$.D:T++$.E*$%% $* 3GK *UMY+LY/7B* % P *4+%Q$$*B1MB *  *+*AU% 8Q$+Q8O*:S3J%+H++/6$/0*H+++ /*$O+::+$V/+%7M8/T1B $-+77$+2--:$ZP*.G33:$:*:*$ $*%++ *5+*$3%%*% *M%% %T *V %C%0UD%+.++3* SN$**QF*.+*QE50  :4O /T$/*%8%*  5  OS:$+*/1FAG.C %XS7VV+00ZR$*L*$$*:*TE *+ *W%4J$*.3*O$0+6$+3$-+%- *$ 0+T9-$.HET$%S S8LRV$%+LGQ*$.:ZU*+:4L$C  6K+ +/NB: *+ I/.%%%%7J%+*D$O.% C XF2 9/*UY:.*O/ $MAX:1 2Z2716W+Q++%.  D $9-%/+0V%.D M$..D$LB35K5 9YC%+C E +SIH$* K.*+%+S+6. $6  +$Z %J*%% + 4$*%J$+$..HA /S %$GE+A 8 4T+88$L*% O0B$A+%IO26Z+ U/*/Q*:4 :  5/$1+ H4$*V%HV%62*%+-6$+9C.$$  +*+7 :4%$:0J+C+.HU3$ 7*+$*/1GN QFF4*I*:%.N1*4Z%320$5%VBY :%+ 1J-N1 B8%D*X/Z**A.2Z8GB. %L+3U%2+%*9 *%TB+O$*:% :6V% %*RW *6*Y*J*AD2:R0-%-*C *$6% $:%KT%-*/L  %*$ 5$* %*FVXQSM8*:ZG*$%%$+:/%+C$ 2/L-TS%*6*X1$U+/%- LJP/:W4/$BB+8K%%%:+%O65/: V8+9$W OT .*QE I+/1C/+%+N+ZX2+6$.9$/%K$J91$+ +9$%/V+%+* P +%*: +*. $B +K84 . $A / :Y JRKZ DQM$5*S  1%0%%X%TJ6*Q P*%1:2:%%%*K* *:++%V$%*:K%$$*ER:/$2FY* *5 9X$% %%$%I$L/+$ *8Q  /%:+OB 7$00U$$$%$%$$  6:U4K%/ WK.+4 : *%$ +* .2++*%$6$A*  4$A: W  %:$D/+$A-7 6L:2 +*0 .Y 7%Q $*$C5 .0*%*-$.  B$ -  %%BS:+*4$+**+2J%+U  %9WV $W %*N1++* *$$O%/.*/W*-U $Q-%Y+9U7*+% XX$SJ 8N6:++C/.OHT*5.*$/*:-S*%8Q/-* 2 % +Y8+**%6% %J..A:U/$%ROPHM:P3P*$ $X*/ +ZAF * *J.7$$H32/$*D%P.$:0% $T25/$XACQ8$92ZA*T. NP$ 9Q$4/BO*%PZ*3$* O 1..+ $F*WFDIT-*%F   19908$%%*.*   7%45: $4$*%%4G8$N %4:*: *O*C $+.Y0$$% K Q0%%%: %0%P+% +Q$ +$/*$  A+28+*3:Y*T* *./UP7+++KO +  Y 2UX%75%%%$N/1 E*X **%%B%$4 J5: G$* +P T+/ :T  KI*/*03O.*.  +*C%/ .%*D- -+   EB.YB$TV8 %8S7E .+%%K%%+B$Q3 68E+2C/+-$%5%*$$F D*6X*$ P%5 S% /+H9 $.$$*$E+*I DN6*5W/5E**%$*S- +% X4.X$Q-8*.+O%9.++3 +3**5/T.$:++%3XF*U:8T*3M % $.%V**$A*$**%$5721+89 GZ $6*$X$5MSN*%+Z% H4A%$%$:* $M++Q++K1*J29LA7%L+..0:   +1GGI+%%  Y$6*$* 0 :% QRS+Z$ %$4-*S0:  6$ /I%FVFRIF:* EG /6%2%% *A$XA*3*/ /8*W%M W $BL $ T4G/%*P-X10/*+/+%$*.5.A 5 Q*. 13.W +A+F ** IM.M R3  $I+-VT 4 %$%-+  +:+ P%LA++2+$%$A8+L$** /91  EF*$%$+$%G T3 **N+9BDA+  .*H%M.$G*I$+  5*L: .*5/*6% $*DE F2U+ $2*3-U*H/F4MFBB-$0P$  9M9Y8PHA%8 Y8*Z:: +%+** %%.+  2H2D3%PJ*Y%: .$E**N$/X**7*$$/%/+7**$1L:L:W2 /2P PM$S*+6$. $+ ***K%S::W:: % F *M6+** 9$94$ +K %$*/J*+L+$K$$$P2E64TJO 9O%*Z2/  8 - : %+.R1$3 +N 7+5%8.*$+5%8+* /O1  /:7V/W$+T2  X%S-+  $IE+**.WO %%*3 G +:%%OKO6$ 26$+F.$%%.A$6G %%*$%K++$ E*6* W $9 Q$+F8% *Z +$$9/.*$8%+ADEFQYU%$I U$$%$I++N$4 0*$:*%+%  L3  W$U 9% XR*BB+$".to_string();
        let ecl = ECLevel::H;
        let hi_cap = true;

        let qr =
            QRBuilder::new(data.as_bytes()).ec_level(ecl).high_capacity(hi_cap).build().unwrap();

        let img = image::DynamicImage::ImageRgb8(qr.to_image(3));
        let mut res = if hi_cap { detect_hc_qr(&img) } else { detect_qr(&img) };
        let (_meta, decoded) = res.symbols()[0].decode().expect("Failed to read QR");

        assert_eq!(data, decoded);
    }

    #[test]
    fn test_qr_4() {
        let data = "585627067124432801036103098250466587082098029070289300241937719552437691575758733990547740427060786150911824925218003093789713212340411443393453215736157098813647870370509699666994642172773053091686025211301162247120566590041376085629647328730765988396689756357154425367242929221144994350535936221971939862436426382195503314392694830527873017681509101063367239953387016735772250128493546221677430709324753049832805787576927081397983420929208781413727502534110683825359131552802354813122716205595937448260030931653748316829394915867465670210517662573074777412441734453277711863202037758141390937485469501821225579494064439085907172027191340811777551424268728445718136306108402432567238996727775132716127240779175063531852394713818093707330300955495233249170660080756396523604545527328928195380162726259100822815594225239691049083204542665311151952515630445918551258222995302755008018593184200107470441720219199158988429795069275279042168214162459235148763271872680724594555599146074347024055268560549240864096855985858687549424188464728754234506826883995610401307977214772508022291788532304307729364126297258902907158750269554024753572118965890761497045194839854960544561556625280899085580947630085874600874209662749852830163793054862534236587544555353618781309788942121079535132463075630530068641084725536491919282641198144657547101028410410474490649931208989584229601468833705028509118635595372780151790414330642".to_string();
        let ecl = ECLevel::H;
        let hi_cap = true;

        let qr =
            QRBuilder::new(data.as_bytes()).ec_level(ecl).high_capacity(hi_cap).build().unwrap();

        let img = image::DynamicImage::ImageRgb8(qr.to_image(3));
        let mut res = if hi_cap { detect_hc_qr(&img) } else { detect_qr(&img) };
        let (_meta, decoded) = res.symbols()[0].decode().expect("Failed to read QR");

        assert_eq!(data, decoded);
    }
}
