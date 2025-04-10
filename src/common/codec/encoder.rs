pub use encode::*;

// Encoder
//------------------------------------------------------------------------------

pub mod encode {
    use std::mem::swap;

    use crate::codec::{Mode, Segment, MODES};
    use crate::metadata::{ECLevel, Palette, Version};
    use crate::utils::{BitStream, QRError, QRResult};

    use super::writer::{pad_remaining_capacity, push_segment, push_terminator};

    // TODO: Write testcases
    pub fn encode(data: &[u8], ecl: ECLevel, pal: Palette) -> QRResult<(BitStream, Version)> {
        let (ver, segs) = find_optimal_version_and_segments(data, ecl, pal)?;
        let bcap = ver.data_bit_capacity(ecl, pal);
        let mut bs = BitStream::new(bcap);
        for seg in segs {
            push_segment(seg, &mut bs);
        }

        push_terminator(&mut bs);
        pad_remaining_capacity(&mut bs);
        Ok((bs, ver))
    }

    // TODO: Write testcases
    pub fn encode_with_version(
        data: &[u8],
        ver: Version,
        ecl: ECLevel,
        pal: Palette,
    ) -> QRResult<BitStream> {
        let bcap = ver.data_bit_capacity(ecl, pal);
        let segs = compute_optimal_segments(data, ver);
        let sz: usize = segs.iter().map(|s| s.bit_len()).sum();
        if sz > bcap {
            return Err(QRError::DataTooLong);
        }
        let bcap = ver.data_bit_capacity(ecl, pal);
        let mut bs = BitStream::new(bcap);
        for seg in segs {
            push_segment(seg, &mut bs);
        }
        push_terminator(&mut bs);
        pad_remaining_capacity(&mut bs);
        Ok(bs)
    }

    fn find_optimal_version_and_segments(
        data: &[u8],
        ecl: ECLevel,
        pal: Palette,
    ) -> QRResult<(Version, Vec<Segment>)> {
        let mut segs = vec![];
        let mut sz = 0;
        for v in 1..=40 {
            let ver = Version::Normal(v);
            let bcap = ver.data_bit_capacity(ecl, pal);
            if v == 1 || v == 10 || v == 27 {
                segs = compute_optimal_segments(data, ver);
                sz = segs.iter().map(|s| s.bit_len()).sum();
            }
            if sz <= bcap {
                return Ok((ver, segs));
            }
        }
        Err(QRError::DataTooLong)
    }

    // Dynamic programming to compute optimum mode segments
    fn compute_optimal_segments(data: &[u8], ver: Version) -> Vec<Segment> {
        debug_assert!(!data.is_empty(), "Empty data");

        let len = data.len();
        let mut prev_cost = [0usize; 3];
        MODES.iter().enumerate().for_each(|(i, &m)| prev_cost[i] = (4 + ver.char_cnt_bits(m)) * 6);
        let mut cur_cost = [usize::MAX; 3];
        let mut min_path = vec![[usize::MAX; 3]; len];
        for (i, b) in data.iter().enumerate() {
            for (j, to_mode) in MODES.iter().enumerate() {
                if !to_mode.contains(*b) {
                    continue;
                }
                let encoded_char_size = match to_mode {
                    Mode::Numeric => 20,
                    Mode::Alphanumeric => 33,
                    Mode::Byte => 48,
                };
                for (k, from_mode) in MODES.iter().enumerate() {
                    if prev_cost[k] == usize::MAX {
                        continue;
                    }
                    let mut cost = 0;
                    if to_mode != from_mode {
                        cost += (prev_cost[k] + 5) / 6 * 6;
                        cost += (4 + ver.char_cnt_bits(*to_mode)) * 6;
                    } else {
                        cost += prev_cost[k];
                    }
                    cost += encoded_char_size;
                    if cost < cur_cost[j] {
                        cur_cost[j] = cost;
                        min_path[i][j] = k;
                    }
                }
            }
            swap(&mut prev_cost, &mut cur_cost);
            cur_cost.fill(usize::MAX);
        }

        let char_modes = trace_optimal_modes(min_path, prev_cost);
        build_segments(ver, char_modes, data)
    }

    // Backtrack min_path and identify optimal char mode
    // TODO: Write testcases
    fn trace_optimal_modes(min_path: Vec<[usize; 3]>, prev_cost: [usize; 3]) -> Vec<Mode> {
        let len = min_path.len();
        let mut mode_index = 0;
        for i in 1..3 {
            if prev_cost[i] < prev_cost[mode_index] {
                mode_index = i;
            }
        }
        (0..len)
            .rev()
            .scan(mode_index, |mi, i| {
                let old_mi = *mi;
                *mi = min_path[i][*mi];
                Some(MODES[old_mi])
            })
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect()
    }

    // Build segments encode char modes
    fn build_segments(ver: Version, char_modes: Vec<Mode>, data: &[u8]) -> Vec<Segment> {
        let len = data.len();
        let mut segs: Vec<Segment> = vec![];
        let mut seg_start = 0;
        let mut seg_mode = char_modes[0];
        for (i, &m) in char_modes.iter().enumerate().skip(1) {
            if seg_mode != m {
                let mode_bits = ver.mode_bits();
                let len_bits = ver.char_cnt_bits(seg_mode);
                segs.push(Segment::new(seg_mode, mode_bits, len_bits, &data[seg_start..i]));
                seg_mode = m;
                seg_start = i;
            }
        }
        let mode_bits = ver.mode_bits();
        let len_bits = ver.char_cnt_bits(seg_mode);
        segs.push(Segment::new(seg_mode, mode_bits, len_bits, &data[seg_start..len]));

        segs
    }

    #[cfg(test)]
    mod encode_tests {
        use test_case::test_case;

        use super::{
            build_segments, compute_optimal_segments, encode_with_version,
            find_optimal_version_and_segments, ECLevel, Mode, Palette, Segment, Version,
        };

        #[test]
        fn test_build_segments() {
            let data = "aaaaa11111AAA";
            let ver = Version::Normal(1);
            let mode_bits = ver.mode_bits();
            let mut char_modes = vec![Mode::Alphanumeric; 5];
            char_modes.extend([Mode::Numeric; 5]);
            char_modes.extend([Mode::Byte; 3]);
            let segs = build_segments(ver, char_modes, data.as_bytes());
            let seg_1 = Segment::new(
                Mode::Alphanumeric,
                mode_bits,
                ver.char_cnt_bits(Mode::Alphanumeric),
                data[0..5].as_bytes(),
            );
            let seg_2 = Segment::new(
                Mode::Numeric,
                mode_bits,
                ver.char_cnt_bits(Mode::Numeric),
                data[5..10].as_bytes(),
            );
            let seg_3 = Segment::new(
                Mode::Byte,
                mode_bits,
                ver.char_cnt_bits(Mode::Byte),
                data[10..].as_bytes(),
            );
            assert_eq!(segs.len(), 3);
            assert_eq!(segs[0], seg_1);
            assert_eq!(segs[1], seg_2);
            assert_eq!(segs[2], seg_3);
        }

        #[test_case("1111111".to_string(), Version::Normal(1), vec![(Mode::Numeric, 0, None)])]
        #[test_case("AAAAA".to_string(), Version::Normal(1), vec![(Mode::Alphanumeric, 0, None)])]
        #[test_case("aaaaa".to_string(), Version::Normal(1), vec![(Mode::Byte, 0, None)])]
        #[test_case("1111111AAAA".to_string(), Version::Normal(1), vec![(Mode::Numeric, 0, Some(7)), (Mode::Alphanumeric, 7, None)])]
        #[test_case("111111AAAA".to_string(), Version::Normal(1), vec![(Mode::Alphanumeric, 0,None)])]
        #[test_case("aaa11111a".to_string(), Version::Normal(1), vec![(Mode::Byte, 0, None)])]
        #[test_case("aaa111111a".to_string(), Version::Normal(1), vec![(Mode::Byte, 0, Some(3)), (Mode::Numeric, 3, Some(9)), (Mode::Byte, 9, None)])]
        #[test_case("aaa1111A".to_string(), Version::Normal(1), vec![(Mode::Byte, 0, None)])]
        #[test_case("aaa1111AA".to_string(), Version::Normal(1), vec![(Mode::Byte, 0, Some(3)), (Mode::Alphanumeric, 3, None)])]
        #[test_case("aaa1111111AA".to_string(), Version::Normal(1), vec![(Mode::Byte, 0, Some(3)), (Mode::Numeric, 3, Some(10)), (Mode::Alphanumeric, 10, None)])]
        #[test_case(("A11111111111111".repeat(23) + "A").to_string(), Version::Normal(10), vec![(Mode::Alphanumeric, 0, None)])]
        #[test_case("Golden ratio φ = 1.6180339887498948482045868343656381177203091798057628621354486227052604628189024497072072041893911374......".to_string(), Version::Normal(9), vec![(Mode::Byte, 0, Some(20)), (Mode::Numeric, 20, Some(120)), (Mode::Alphanumeric, 120, Some(126))])]
        fn test_compute_optimal_segments(
            data: String,
            ver: Version,
            chunks: Vec<(Mode, usize, Option<usize>)>,
        ) {
            let mode_bits = ver.mode_bits();
            let segs = compute_optimal_segments(data.as_bytes(), ver);
            assert_eq!(segs.len(), chunks.len());
            for (seg, &(mode, start, end)) in segs.iter().zip(chunks.iter()) {
                let len_bits = ver.char_cnt_bits(mode);
                let exp_seg = match end {
                    Some(e) => Segment::new(mode, mode_bits, len_bits, data[start..e].as_bytes()),
                    None => Segment::new(mode, mode_bits, len_bits, data[start..].as_bytes()),
                };
                assert_eq!(*seg, exp_seg);
            }
        }

        #[test]
        fn test_compute_optimal_segments_1() {
            let data = "A11111111111111".repeat(23);
            let ver = Version::Normal(9);
            let mode_bits = ver.mode_bits();
            let segs = compute_optimal_segments(data.as_bytes(), ver);
            assert_eq!(segs.len(), 46);
            for (i, c) in data.as_bytes().chunks(15).enumerate() {
                let seg_1 = Segment::new(
                    Mode::Alphanumeric,
                    mode_bits,
                    ver.char_cnt_bits(Mode::Alphanumeric),
                    &c[..1],
                );
                assert_eq!(segs[i * 2], seg_1);
                let seg_2 = Segment::new(
                    Mode::Numeric,
                    mode_bits,
                    ver.char_cnt_bits(Mode::Numeric),
                    &c[1..],
                );
                assert_eq!(segs[i * 2 + 1], seg_2);
            }
        }

        #[test_case("aaaaa11111AAA".to_string(), Version::Normal(1), ECLevel::L, Palette::Mono)]
        #[test_case("A11111111111111".repeat(2).to_string(), Version::Normal(2), ECLevel::L, Palette::Mono)]
        #[test_case("A11111111111111".repeat(4).to_string(), Version::Normal(3), ECLevel::L, Palette::Mono)]
        #[test_case("aAAAAAAAAAAA".repeat(5).to_string(), Version::Normal(4), ECLevel::L, Palette::Mono)]
        #[test_case("aAAAAAAAAAAA".repeat(21).to_string(), Version::Normal(10), ECLevel::L, Palette::Mono)]
        #[test_case("a".repeat(2953).to_string(), Version::Normal(40), ECLevel::L, Palette::Mono)]
        fn test_find_optimal_ver_and_segments(
            data: String,
            exp_ver: Version,
            ecl: ECLevel,
            pal: Palette,
        ) {
            let (ver, _) = find_optimal_version_and_segments(data.as_bytes(), ecl, pal).unwrap();
            assert_eq!(ver, exp_ver);
        }

        #[test]
        #[should_panic]
        fn test_find_optimal_ver_and_segments_panic() {
            let data = "a".repeat(2954);
            let ecl = ECLevel::L;
            let pal = Palette::Mono;
            find_optimal_version_and_segments(data.as_bytes(), ecl, pal).unwrap();
        }

        #[test]
        fn test_encode_with_version() {
            let data = "!".repeat(256);
            let ver = Version::Normal(9);
            let ecl = ECLevel::L;
            let pal = Palette::Poly;
            let encoded = encode_with_version(data.as_bytes(), ver, ecl, pal).unwrap();
        }
    }
}

// Writer for encoded data
//------------------------------------------------------------------------------

pub(super) mod writer {
    use crate::codec::{Mode, Segment, PADDING_CODEWORDS};
    use crate::utils::BitStream;

    pub fn push_segment(seg: Segment, out: &mut BitStream) {
        push_header(&seg, out);
        match seg.mode {
            Mode::Numeric => push_numeric_data(seg.data, out),
            Mode::Alphanumeric => push_alphanumeric_data(seg.data, out),
            Mode::Byte => push_byte_data(seg.data, out),
        }
    }

    fn push_header(seg: &Segment, out: &mut BitStream) {
        out.push_bits(seg.mode as u8, seg.mode_bits);
        let char_cnt = seg.data.len();
        debug_assert!(
            char_cnt < (1 << seg.len_bits),
            "Char count exceeds bit length: Char count {char_cnt}, Char count bits {}",
            seg.len_bits
        );
        out.push_bits(char_cnt as u16, seg.len_bits);
    }

    fn push_numeric_data(data: &[u8], out: &mut BitStream) {
        for chunk in data.chunks(3) {
            let len = (chunk.len() * 10 + 2) / 3;
            let data = Mode::Numeric.encode_chunk(chunk);
            out.push_bits(data, len);
        }
    }

    fn push_alphanumeric_data(data: &[u8], out: &mut BitStream) {
        for chunk in data.chunks(2) {
            let len = (chunk.len() * 11 + 1) / 2;
            let data = Mode::Alphanumeric.encode_chunk(chunk);
            out.push_bits(data, len);
        }
    }

    fn push_byte_data(data: &[u8], out: &mut BitStream) {
        for chunk in data.chunks(1) {
            let data = Mode::Byte.encode_chunk(chunk);
            out.push_bits(data, 8);
        }
    }

    pub fn push_terminator(out: &mut BitStream) {
        let bit_len = out.len();
        let bit_capacity = out.capacity();
        if bit_len < bit_capacity {
            let term_len = std::cmp::min(4, bit_capacity - bit_len);
            out.push_bits(0, term_len);
        }
    }

    pub fn pad_remaining_capacity(out: &mut BitStream) {
        push_padding_bits(out);
        push_padding_codewords(out);
    }

    fn push_padding_bits(out: &mut BitStream) {
        let offset = out.len() & 7;
        if offset > 0 {
            let padding_bits_len = 8 - offset;
            out.push_bits(0, padding_bits_len);
        }
    }

    fn push_padding_codewords(out: &mut BitStream) {
        let offset = out.len() & 7;
        debug_assert!(
            offset == 0,
            "Bit offset should be zero before padding codewords: {}",
            offset
        );

        let remain_byte_capacity = (out.capacity() - out.len()) >> 3;
        PADDING_CODEWORDS.iter().copied().cycle().take(remain_byte_capacity).for_each(|pc| {
            out.push_bits(pc, 8);
        });
    }

    #[cfg(test)]
    mod writer_tests {
        use super::{Mode, Segment, PADDING_CODEWORDS};
        use crate::codec::writer::{
            push_alphanumeric_data, push_byte_data, push_header, push_numeric_data,
            push_padding_bits, push_padding_codewords, push_terminator,
        };
        use crate::metadata::{ECLevel, Palette, Version};
        use crate::utils::BitStream;

        #[test]
        fn test_push_header_v1() {
            let ver = Version::Normal(1);
            let ecl = ECLevel::L;
            let pal = Palette::Mono;
            let bit_capacity = ver.data_bit_capacity(ecl, pal);
            let mode_bits = ver.mode_bits();
            let exp_vecs: Vec<Vec<u8>> = vec![
                vec![0b00011111, 0b11111100],
                vec![0b00101111, 0b11111000],
                vec![0b01001111, 0b11110000],
            ];
            let dummy_vec = vec![0; 1023];
            let modes = [Mode::Numeric, Mode::Alphanumeric, Mode::Byte];
            let dummy_idx = [1023, 511, 255];
            for ((mode, di), exp_vec) in modes.iter().zip(dummy_idx.iter()).zip(exp_vecs.iter()) {
                let mut bs = BitStream::new(bit_capacity);
                let len_bits = ver.char_cnt_bits(*mode);
                let seg = Segment::new(*mode, mode_bits, len_bits, &dummy_vec[..*di]);
                push_header(&seg, &mut bs);
                assert_eq!(bs.data(), exp_vec);
            }
        }

        #[test]
        fn test_push_header_v10() {
            let ver = Version::Normal(10);
            let ecl = ECLevel::L;
            let pal = Palette::Mono;
            let bit_capacity = ver.data_bit_capacity(ecl, pal);
            let mode_bits = ver.mode_bits();
            let exp_vecs: Vec<Vec<u8>> = vec![
                vec![0b00011111, 0b11111111],
                vec![0b00101111, 0b11111110],
                vec![0b01001111, 0b11111111, 0b11110000],
            ];
            let dummy_vec = vec![0; 65535];
            let modes = [Mode::Numeric, Mode::Alphanumeric, Mode::Byte];
            let dummy_idx = [4095, 2047, 65535];
            for ((mode, di), exp_vec) in modes.iter().zip(dummy_idx.iter()).zip(exp_vecs.iter()) {
                let mut bs = BitStream::new(bit_capacity);
                let len_bits = ver.char_cnt_bits(*mode);
                let seg = Segment::new(*mode, mode_bits, len_bits, &dummy_vec[..*di]);
                push_header(&seg, &mut bs);
                assert_eq!(bs.data(), exp_vec);
            }
        }

        #[test]
        fn test_push_header_v27() {
            let ver = Version::Normal(27);
            let ecl = ECLevel::L;
            let pal = Palette::Mono;
            let bit_capacity = ver.data_bit_capacity(ecl, pal);
            let mode_bits = ver.mode_bits();
            let exp_vecs: Vec<Vec<u8>> = vec![
                vec![0b00011111, 0b11111111, 0b11000000],
                vec![0b00101111, 0b11111111, 0b10000000],
                vec![0b01001111, 0b11111111, 0b11110000],
            ];
            let dummy_vec = vec![0; 65535];
            let modes = [Mode::Numeric, Mode::Alphanumeric, Mode::Byte];
            let dummy_idx = [16383, 8191, 65535];
            for ((mode, di), exp_vec) in modes.iter().zip(dummy_idx.iter()).zip(exp_vecs.iter()) {
                let mut bs = BitStream::new(bit_capacity);
                let len_bits = ver.char_cnt_bits(*mode);
                let seg = Segment::new(*mode, mode_bits, len_bits, &dummy_vec[..*di]);
                push_header(&seg, &mut bs);
                assert_eq!(bs.data(), exp_vec);
            }
        }

        #[test]
        fn test_push_numeric_data() {
            let ver = Version::Normal(1);
            let ecl = ECLevel::L;
            let pal = Palette::Mono;
            let bit_capacity = ver.data_bit_capacity(ecl, pal);
            let mut bs = BitStream::new(bit_capacity);
            push_numeric_data("01234567".as_bytes(), &mut bs);
            assert_eq!(bs.data(), vec![0b00000011, 0b00010101, 0b10011000, 0b01100000]);
            let mut bs = BitStream::new(bit_capacity);
            push_numeric_data("8".as_bytes(), &mut bs);
            assert_eq!(bs.data(), vec![0b10000000]);
        }

        #[test]
        fn test_push_alphanumeric_data() {
            let ver = Version::Normal(1);
            let ecl = ECLevel::L;
            let pal = Palette::Mono;
            let bit_capacity = ver.data_bit_capacity(ecl, pal);
            let mut bs = BitStream::new(bit_capacity);
            push_alphanumeric_data("AC-42".as_bytes(), &mut bs);
            assert_eq!(bs.data(), vec![0b00111001, 0b11011100, 0b11100100, 0b00100000])
        }

        #[test]
        fn test_push_byte_data() {
            let ver = Version::Normal(1);
            let ecl = ECLevel::L;
            let pal = Palette::Mono;
            let bit_capacity = ver.data_bit_capacity(ecl, pal);
            let mut bs = BitStream::new(bit_capacity);
            push_byte_data("a".as_bytes(), &mut bs);
            assert_eq!(bs.data(), vec![0b01100001])
        }

        #[test]
        fn test_push_terminator() {
            let ver = Version::Normal(1);
            let ecl = ECLevel::L;
            let pal = Palette::Mono;
            let bit_capacity = ver.data_bit_capacity(ecl, pal);
            let capacity = (bit_capacity + 7) >> 3;
            let mut bs = BitStream::new(bit_capacity);
            bs.push_bits(0b1, 1);
            push_terminator(&mut bs);
            assert_eq!(bs.data(), vec![0b10000000]);
            assert_eq!(bs.len() & 7, 5);
            for _ in 0..capacity - 1 {
                bs.push_bits(0b11111111, 8);
            }
            push_terminator(&mut bs);
            assert_eq!(bs.len() & 7, 0);
        }

        #[test]
        fn test_push_padding_bits() {
            let ver = Version::Normal(1);
            let ecl = ECLevel::L;
            let pal = Palette::Mono;
            let bit_capacity = ver.data_bit_capacity(ecl, pal);
            let mut bs = BitStream::new(bit_capacity);
            bs.push_bits(1, 0b1);
            push_padding_bits(&mut bs);
            assert_eq!(bs.data(), vec![0b10000000]);
            assert_eq!(bs.len() & 7, 0);
        }

        #[test]
        fn test_push_padding_codewords() {
            let ver = Version::Normal(1);
            let ecl = ECLevel::L;
            let pal = Palette::Mono;
            let bit_capacity = ver.data_bit_capacity(ecl, pal);
            let mut bs = BitStream::new(bit_capacity);
            bs.push_bits(1, 0b1);
            push_padding_bits(&mut bs);
            push_padding_codewords(&mut bs);
            let mut output = vec![0b10000000];
            output.extend(PADDING_CODEWORDS.iter().cycle().take(18));
            assert_eq!(bs.data(), output);
        }
    }
}
