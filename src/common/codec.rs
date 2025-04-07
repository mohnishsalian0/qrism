use std::cmp::Ordering;

pub use decode::*;
pub use encode::*;

// Mode
//------------------------------------------------------------------------------

#[derive(Debug, PartialEq, Eq, Copy, Clone)]
pub enum Mode {
    Numeric = 0b0001,
    Alphanumeric = 0b0010,
    Byte = 0b0100,
}

impl PartialOrd for Mode {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Mode {
    fn cmp(&self, other: &Self) -> Ordering {
        match (*self, *other) {
            (a, b) if a == b => Ordering::Equal,
            (Self::Numeric, _) | (_, Self::Byte) => Ordering::Less,
            (_, Self::Numeric) | (Self::Byte, _) => Ordering::Greater,
            _ => unreachable!(),
        }
    }
}

impl Mode {
    #[inline]
    fn numeric_digit(char: u8) -> u16 {
        debug_assert!(Mode::Numeric.contains(char), "Invalid numeric data: {char}");
        (char - b'0') as u16
    }

    #[inline]
    fn alphanumeric_digit(char: u8) -> u16 {
        debug_assert!(Mode::Alphanumeric.contains(char), "Invalid alphanumeric data: {char}");
        match char {
            b'0'..=b'9' => (char - b'0') as u16,
            b'A'..=b'Z' => (char - b'A' + 10) as u16,
            b' ' => 36,
            b'$' => 37,
            b'%' => 38,
            b'*' => 39,
            b'+' => 40,
            b'-' => 41,
            b'.' => 42,
            b'/' => 43,
            b':' => 44,
            _ => unreachable!("Invalid alphanumeric {char}"),
        }
    }

    #[inline]
    fn byte(&self, mode_digit: u8) -> u8 {
        match self {
            Self::Numeric => match mode_digit {
                md @ 0..=9 => md + b'0',
                _ => unreachable!("Invalid numeric digit {mode_digit}"),
            },
            Self::Alphanumeric => match mode_digit {
                md @ 0..=9 => md + b'0',
                md @ 10..=35 => md - 10 + b'A',
                36 => b' ',
                37 => b'$',
                38 => b'%',
                39 => b'*',
                40 => b'+',
                41 => b'-',
                42 => b'.',
                43 => b'/',
                44 => b':',
                _ => unreachable!("Invalid alphanumeric digit {mode_digit}"),
            },
            Self::Byte => mode_digit,
        }
    }

    pub fn encode_chunk(&self, data: &[u8]) -> u16 {
        let len = data.len();
        match self {
            Self::Numeric => {
                debug_assert!(len <= 3, "Data is too long for numeric conver: {len}");
                data.iter().fold(0_u16, |n, b| n * 10 + Self::numeric_digit(*b))
            }
            Self::Alphanumeric => {
                debug_assert!(len <= 2, "Data is too long for alphanumeric conver: {len}");
                data.iter().fold(0_u16, |n, b| n * 45 + Self::alphanumeric_digit(*b))
            }
            Self::Byte => {
                debug_assert!(len == 1, "Data is too long for byte conver: {len}");
                data[0] as u16
            }
        }
    }

    fn decode_numeric_chunk(mut data: u16, bit_len: usize) -> Vec<u8> {
        debug_assert!(
            bit_len == 10 || bit_len == 7 || bit_len == 4,
            "Invalid numeric encoded length: {bit_len}"
        );

        let len = bit_len / 3;
        let mut res = vec![0; len];
        for i in 0..len {
            res[len - 1 - i] = Mode::Numeric.byte((data % 10) as u8);
            data /= 10;
        }
        res
    }

    fn decode_alphanumeric_chunk(mut data: u16, bit_len: usize) -> Vec<u8> {
        debug_assert!(
            bit_len == 11 || bit_len == 6,
            "Invalid alphanumeric encoded length: {bit_len}"
        );

        let len = bit_len / 5;
        let mut res = vec![0; len];
        for i in 0..len {
            res[len - 1 - i] = Mode::Alphanumeric.byte((data % 45) as u8);
            data /= 45;
        }
        res
    }

    pub fn decode_chunk(&self, data: u16, bit_len: usize) -> Vec<u8> {
        match self {
            Self::Numeric => Self::decode_numeric_chunk(data, bit_len),
            Self::Alphanumeric => Self::decode_alphanumeric_chunk(data, bit_len),
            Self::Byte => {
                debug_assert!(bit_len == 8, "Invalid byte encoded length: {bit_len}");

                vec![data as u8]
            }
        }
    }

    pub fn contains(&self, byte: u8) -> bool {
        match self {
            Self::Numeric => byte.is_ascii_digit(),
            Self::Alphanumeric => {
                matches!(byte, b'0'..=b'9' | b'A'..=b'Z' | b' ' | b'$' | b'%' | b'*' | b'+' | b'-' | b'.' | b'/' | b':')
            }
            Self::Byte => true,
        }
    }

    pub fn encoded_len(&self, len: usize) -> usize {
        match *self {
            Self::Numeric => (len * 10 + 2) / 3,
            Self::Alphanumeric => (len * 11 + 1) / 2,
            Self::Byte => len * 8,
        }
    }
}

#[cfg(test)]
mod mode_tests {

    use super::Mode;
    use super::Mode::*;

    #[test]
    fn test_comparison() {
        assert!(Numeric == Numeric);
        assert!(Numeric < Alphanumeric);
        assert!(Numeric < Byte);
        assert!(Alphanumeric == Alphanumeric);
        assert!(Alphanumeric < Byte);
        assert!(Byte == Byte);
    }

    #[test]
    fn test_numeric_digit() {
        assert_eq!(Mode::numeric_digit(b'0'), 0);
        assert_eq!(Mode::numeric_digit(b'9'), 9);
    }

    #[test]
    #[should_panic]
    fn test_invalid_numeric_digit() {
        Mode::numeric_digit(b'A');
    }

    #[test]
    fn test_alphanumeric_digit() {
        assert_eq!(Mode::alphanumeric_digit(b'0'), 0);
        assert_eq!(Mode::alphanumeric_digit(b'9'), 9);
        assert_eq!(Mode::alphanumeric_digit(b'A'), 10);
        assert_eq!(Mode::alphanumeric_digit(b'Z'), 35);
        assert_eq!(Mode::alphanumeric_digit(b' '), 36);
        assert_eq!(Mode::alphanumeric_digit(b':'), 44);
    }

    #[test]
    #[should_panic]
    fn test_invalid_alphanumeric_digit() {
        Mode::alphanumeric_digit(b'a');
    }

    #[test]
    fn test_numeric_to_byte() {
        assert_eq!(Numeric.byte(0), b'0');
        assert_eq!(Numeric.byte(9), b'9');
    }

    #[test]
    #[should_panic]
    fn test_invalid_numeric_digit_to_byte() {
        Numeric.byte(b'A');
    }

    #[test]
    fn test_alphanumeric_to_byte() {
        assert_eq!(Alphanumeric.byte(0), b'0');
        assert_eq!(Alphanumeric.byte(9), b'9');
        assert_eq!(Alphanumeric.byte(10), b'A');
        assert_eq!(Alphanumeric.byte(35), b'Z');
        assert_eq!(Alphanumeric.byte(36), b' ');
        assert_eq!(Alphanumeric.byte(44), b':');
    }

    #[test]
    #[should_panic]
    fn test_invalid_alphanumeric_digit_to_byte() {
        Alphanumeric.byte(b'a');
    }

    #[test]
    fn test_numeric_encoding() {
        assert_eq!(Numeric.encode_chunk("012".as_bytes()), 0b0000001100);
        assert_eq!(Numeric.encode_chunk("345".as_bytes()), 0b0101011001);
        assert_eq!(Numeric.encode_chunk("901".as_bytes()), 0b1110000101);
        assert_eq!(Numeric.encode_chunk("67".as_bytes()), 0b1000011);
        assert_eq!(Numeric.encode_chunk("8".as_bytes()), 0b1000);
    }

    #[test]
    #[should_panic]
    fn test_invalid_numeric_encoding() {
        Numeric.encode_chunk("1234".as_bytes());
    }

    #[test]
    fn test_numeric_decoding() {
        let data = "012".as_bytes();
        let encoded_data = Numeric.encode_chunk(data);
        assert_eq!(Numeric.decode_chunk(encoded_data, 10), data);
        let data = "345".as_bytes();
        let encoded_data = Numeric.encode_chunk(data);
        assert_eq!(Numeric.decode_chunk(encoded_data, 10), data);
        let data = "901".as_bytes();
        let encoded_data = Numeric.encode_chunk(data);
        assert_eq!(Numeric.decode_chunk(encoded_data, 10), data);
        let data = "67".as_bytes();
        let encoded_data = Numeric.encode_chunk(data);
        assert_eq!(Numeric.decode_chunk(encoded_data, 7), data);
        let data = "8".as_bytes();
        let encoded_data = Numeric.encode_chunk(data);
        assert_eq!(Numeric.decode_chunk(encoded_data, 4), data);
    }

    #[test]
    fn test_alphanumeric_encoding() {
        assert_eq!(Alphanumeric.encode_chunk("AC".as_bytes()), 0b00111001110);
        assert_eq!(Alphanumeric.encode_chunk("-4".as_bytes()), 0b11100111001);
        assert_eq!(Alphanumeric.encode_chunk("2".as_bytes()), 0b000010);
    }

    #[test]
    #[should_panic]
    fn test_invalid_alphanumeric_encoding() {
        Alphanumeric.encode_chunk("1234".as_bytes());
    }

    #[test]
    fn test_alphanumeric_decoding() {
        let data = "AC".as_bytes();
        let encoded_data = Alphanumeric.encode_chunk(data);
        assert_eq!(Alphanumeric.decode_chunk(encoded_data, 11), data);
        let data = "-4".as_bytes();
        let encoded_data = Alphanumeric.encode_chunk(data);
        assert_eq!(Alphanumeric.decode_chunk(encoded_data, 11), data);
        let data = "2".as_bytes();
        let encoded_data = Alphanumeric.encode_chunk(data);
        assert_eq!(Alphanumeric.decode_chunk(encoded_data, 6), data);
    }

    #[test]
    fn test_is_numeric() {
        assert!(Numeric.contains(b'0'));
        assert!(Numeric.contains(b'9'));
        assert!(!Numeric.contains(b'A'));
        assert!(!Numeric.contains(b'Z'));
        assert!(!Numeric.contains(b' '));
        assert!(!Numeric.contains(b':'));
    }

    #[test]
    fn test_is_alphanumeric() {
        assert!(Alphanumeric.contains(b'0'));
        assert!(Alphanumeric.contains(b'9'));
        assert!(Alphanumeric.contains(b'A'));
        assert!(Alphanumeric.contains(b'Z'));
        assert!(Alphanumeric.contains(b' '));
        assert!(Alphanumeric.contains(b':'));
        assert!(!Alphanumeric.contains(b'@'));
        assert!(!Alphanumeric.contains(b'('));
    }

    #[test]
    fn test_encoded_len() {
        assert_eq!(Numeric.encoded_len(3), 10);
        assert_eq!(Numeric.encoded_len(2), 7);
        assert_eq!(Numeric.encoded_len(1), 4);
        assert_eq!(Alphanumeric.encoded_len(2), 11);
        assert_eq!(Alphanumeric.encoded_len(1), 6);
        assert_eq!(Byte.encoded_len(1), 8);
    }
}

// Segment
//------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
struct Segment<'a> {
    mode: Mode,
    mode_bits: usize,
    len_bits: usize, // Bit len of char count
    data: &'a [u8],  // Reference to raw data
}

impl<'a> Segment<'a> {
    pub fn new(mode: Mode, mode_bits: usize, len_bits: usize, data: &'a [u8]) -> Self {
        Self { mode, mode_bits, len_bits, data }
    }

    pub fn bit_len(&self) -> usize {
        let encoded_bits = self.mode.encoded_len(self.data.len());
        self.mode_bits + self.len_bits + encoded_bits
    }
}

#[cfg(test)]
mod segment_tests {
    use super::{Mode, Segment};
    use crate::Version;

    #[test]
    fn test_bit_len_numeric_mode_1() {
        let ver = Version::Normal(1);
        let mode = Mode::Numeric;
        let mode_bits = ver.mode_bits();
        let len_bits = ver.char_cnt_bits(mode);
        let seg = Segment::new(mode, mode_bits, len_bits, "123".as_bytes());
        assert_eq!(seg.bit_len(), 24);
        let seg = Segment::new(mode, mode_bits, len_bits, "45".as_bytes());
        assert_eq!(seg.bit_len(), 21);
        let seg = Segment::new(mode, mode_bits, len_bits, "6".as_bytes());
        assert_eq!(seg.bit_len(), 18);
    }

    #[test]
    fn test_bit_len_numeric_mode_10() {
        let ver = Version::Normal(10);
        let mode = Mode::Numeric;
        let mode_bits = ver.mode_bits();
        let len_bits = ver.char_cnt_bits(mode);
        let seg = Segment::new(mode, mode_bits, len_bits, "123".as_bytes());
        assert_eq!(seg.bit_len(), 26);
        let seg = Segment::new(mode, mode_bits, len_bits, "45".as_bytes());
        assert_eq!(seg.bit_len(), 23);
        let seg = Segment::new(mode, mode_bits, len_bits, "6".as_bytes());
        assert_eq!(seg.bit_len(), 20);
    }

    #[test]
    fn test_bit_len_numeric_mode_27() {
        let ver = Version::Normal(27);
        let mode = Mode::Numeric;
        let mode_bits = ver.mode_bits();
        let len_bits = ver.char_cnt_bits(mode);
        let seg = Segment::new(mode, mode_bits, len_bits, "123".as_bytes());
        assert_eq!(seg.bit_len(), 28);
        let seg = Segment::new(mode, mode_bits, len_bits, "45".as_bytes());
        assert_eq!(seg.bit_len(), 25);
        let seg = Segment::new(mode, mode_bits, len_bits, "6".as_bytes());
        assert_eq!(seg.bit_len(), 22);
    }

    #[test]
    fn test_bit_len_alphanumeric_mode_1() {
        let ver = Version::Normal(1);
        let mode = Mode::Alphanumeric;
        let mode_bits = ver.mode_bits();
        let len_bits = ver.char_cnt_bits(mode);
        let seg = Segment::new(mode, mode_bits, len_bits, "AZ".as_bytes());
        assert_eq!(seg.bit_len(), 24);
        let seg = Segment::new(mode, mode_bits, len_bits, "-".as_bytes());
        assert_eq!(seg.bit_len(), 19);
    }

    #[test]
    fn test_bit_len_alphanumeric_mode_10() {
        let ver = Version::Normal(10);
        let mode = Mode::Alphanumeric;
        let mode_bits = ver.mode_bits();
        let len_bits = ver.char_cnt_bits(mode);
        let seg = Segment::new(mode, mode_bits, len_bits, "AZ".as_bytes());
        assert_eq!(seg.bit_len(), 26);
        let seg = Segment::new(mode, mode_bits, len_bits, "-".as_bytes());
        assert_eq!(seg.bit_len(), 21);
    }

    #[test]
    fn test_bit_len_alphanumeric_mode_27() {
        let ver = Version::Normal(27);
        let mode = Mode::Alphanumeric;
        let mode_bits = ver.mode_bits();
        let len_bits = ver.char_cnt_bits(mode);
        let seg = Segment::new(mode, mode_bits, len_bits, "AZ".as_bytes());
        assert_eq!(seg.bit_len(), 28);
        let seg = Segment::new(mode, mode_bits, len_bits, "-".as_bytes());
        assert_eq!(seg.bit_len(), 23);
    }

    #[test]
    fn test_bit_len_byte_mode_1() {
        let ver = Version::Normal(1);
        let mode = Mode::Byte;
        let mode_bits = ver.mode_bits();
        let len_bits = ver.char_cnt_bits(mode);
        let seg = Segment::new(mode, mode_bits, len_bits, "a".as_bytes());
        assert_eq!(seg.bit_len(), 20);
    }

    #[test]
    fn test_bit_len_byte_mode_10() {
        let ver = Version::Normal(10);
        let mode = Mode::Byte;
        let mode_bits = ver.mode_bits();
        let len_bits = ver.char_cnt_bits(mode);
        let seg = Segment::new(mode, mode_bits, len_bits, "ab".as_bytes());
        assert_eq!(seg.bit_len(), 36);
    }

    #[test]
    fn test_bit_len_byte_mode_27() {
        let ver = Version::Normal(27);
        let mode = Mode::Byte;
        let mode_bits = ver.mode_bits();
        let len_bits = ver.char_cnt_bits(mode);
        let seg = Segment::new(mode, mode_bits, len_bits, "abc".as_bytes());
        assert_eq!(seg.bit_len(), 44);
    }
}

// Writer for encoded data
//------------------------------------------------------------------------------

mod writer {
    use crate::common::{codec::PADDING_CODEWORDS, BitStream};

    use super::{Mode, Segment};

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
        use super::{Mode, PADDING_CODEWORDS};
        use crate::common::{
            codec::{
                writer::{
                    push_alphanumeric_data, push_byte_data, push_header, push_numeric_data,
                    push_padding_bits, push_padding_codewords, push_terminator,
                },
                Segment,
            },
            BitStream,
        };
        use crate::{ECLevel, Palette, Version};

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

// Encoder
//------------------------------------------------------------------------------

mod encode {
    use std::mem::swap;

    use crate::{
        common::{codec::MODES, BitStream, Mode},
        ECLevel, Palette, QRError, QRResult, Version,
    };

    use super::{
        writer::{pad_remaining_capacity, push_segment, push_terminator},
        Segment,
    };

    // TODO: Write testcases
    pub fn encode(data: &[u8], ecl: ECLevel, pal: Palette) -> QRResult<(BitStream, Version)> {
        let (ver, segments) = find_optimal_ver_and_segments(data, ecl, pal)?;
        let bit_capacity = ver.data_bit_capacity(ecl, pal);
        let mut bs = BitStream::new(bit_capacity);
        for seg in segments {
            push_segment(seg, &mut bs);
        }
        let encoded_len = (bs.len() + 7) >> 3;

        push_terminator(&mut bs);
        pad_remaining_capacity(&mut bs);
        Ok((bs, ver))
    }

    // TODO: Write testcases
    pub fn encode_with_version(
        data: &[u8],
        ecl: ECLevel,
        ver: Version,
        pal: Palette,
    ) -> QRResult<BitStream> {
        let capacity = ver.data_bit_capacity(ecl, pal);
        let segments = compute_optimal_segments(data, ver);
        let size: usize = segments.iter().map(|s| s.bit_len()).sum();
        if size > capacity {
            return Err(QRError::DataTooLong);
        }
        let bit_capacity = ver.data_bit_capacity(ecl, pal);
        let mut bs = BitStream::new(bit_capacity);
        for seg in segments {
            push_segment(seg, &mut bs);
        }
        let encoded_len = (bs.len() + 7) >> 3;
        push_terminator(&mut bs);
        pad_remaining_capacity(&mut bs);
        Ok(bs)
    }

    fn find_optimal_ver_and_segments(
        data: &[u8],
        ecl: ECLevel,
        pal: Palette,
    ) -> QRResult<(Version, Vec<Segment>)> {
        let mut segments = vec![];
        let mut size = 0;
        for v in 1..=40 {
            let ver = Version::Normal(v);
            let capacity = ver.data_bit_capacity(ecl, pal);
            if v == 1 || v == 10 || v == 27 {
                segments = compute_optimal_segments(data, ver);
                size = segments.iter().map(|s| s.bit_len()).sum();
            }
            if size <= capacity {
                return Ok((ver, segments));
            }
        }
        Err(QRError::DataTooLong)
    }

    // Dynamic programming to compute optimum mode segments
    fn compute_optimal_segments(data: &[u8], ver: Version) -> Vec<Segment> {
        debug_assert!(!data.is_empty(), "Empty data");

        let len = data.len();
        let mut prev_cost: [usize; 3] = [0; 3];
        MODES.iter().enumerate().for_each(|(i, &m)| prev_cost[i] = (4 + ver.char_cnt_bits(m)) * 6);
        let mut cur_cost: [usize; 3] = [usize::MAX; 3];
        let mut min_path: Vec<Vec<usize>> = vec![vec![usize::MAX; 3]; len];
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
    fn trace_optimal_modes(min_path: Vec<Vec<usize>>, prev_cost: [usize; 3]) -> Vec<Mode> {
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
            build_segments, compute_optimal_segments, find_optimal_ver_and_segments, ECLevel, Mode,
            Palette, Segment, Version,
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
            let (ver, _) = find_optimal_ver_and_segments(data.as_bytes(), ecl, pal).unwrap();
            assert_eq!(ver, exp_ver);
        }

        #[test]
        #[should_panic]
        fn test_find_optimal_ver_and_segments_panic() {
            let data = "a".repeat(2954);
            let ecl = ECLevel::L;
            let pal = Palette::Mono;
            find_optimal_ver_and_segments(data.as_bytes(), ecl, pal).unwrap();
        }
    }
}

// Reader for encoded data
//------------------------------------------------------------------------------

mod reader {
    use std::cmp::min;

    use crate::{common::BitStream, Version};

    use super::Mode;

    pub fn take_segment(inp: &mut BitStream, ver: Version) -> Option<Vec<u8>> {
        let (mode, char_cnt) = take_header(inp, ver)?;
        let byte_data = match mode {
            Mode::Numeric => take_numeric_data(inp, char_cnt),
            Mode::Alphanumeric => take_alphanumeric_data(inp, char_cnt),
            Mode::Byte => take_byte_data(inp, char_cnt),
        };
        Some(byte_data)
    }

    fn take_header(inp: &mut BitStream, ver: Version) -> Option<(Mode, usize)> {
        let mode_bits = inp.take_bits(4)?;
        let mode = match mode_bits {
            0 => return None,
            1 => Mode::Numeric,
            2 => Mode::Alphanumeric,
            4 => Mode::Byte,
            _ => unreachable!("Invalid Mode: {mode_bits}"),
        };
        let len_bits = ver.char_cnt_bits(mode);
        let char_cnt = inp.take_bits(len_bits)?;
        Some((mode, char_cnt.into()))
    }

    fn take_numeric_data(inp: &mut BitStream, mut char_cnt: usize) -> Vec<u8> {
        let mut res = Vec::with_capacity(char_cnt);
        while char_cnt > 0 {
            let bit_len = if char_cnt > 2 { 10 } else { (char_cnt % 3) * 3 + 1 };
            let chunk = inp.take_bits(bit_len).unwrap();
            let bytes = Mode::Numeric.decode_chunk(chunk, bit_len);
            res.extend(bytes);
            char_cnt -= min(3, char_cnt);
        }
        res
    }

    fn take_alphanumeric_data(inp: &mut BitStream, mut char_cnt: usize) -> Vec<u8> {
        let mut res = Vec::with_capacity(char_cnt);
        while char_cnt > 0 {
            let bit_len = if char_cnt > 1 { 11 } else { 6 };
            let chunk = inp.take_bits(bit_len).unwrap();
            let bytes = Mode::Alphanumeric.decode_chunk(chunk, bit_len);
            res.extend(bytes);
            char_cnt -= min(2, char_cnt);
        }
        res
    }

    fn take_byte_data(inp: &mut BitStream, mut char_cnt: usize) -> Vec<u8> {
        let mut res = Vec::with_capacity(char_cnt);
        while char_cnt > 0 {
            let chunk = inp.take_bits(8).unwrap();
            let bytes = Mode::Byte.decode_chunk(chunk, 8);
            res.extend(bytes);
            char_cnt -= 1;
        }
        res
    }

    #[cfg(test)]
    mod reader_tests {
        use super::super::encode::encode_with_version;
        use super::{
            take_alphanumeric_data, take_byte_data, take_header, take_numeric_data, take_segment,
            BitStream, Mode,
        };
        use crate::{ECLevel, Palette, Version};

        #[test]
        fn test_take_header_v1() {
            let data = vec![0b00011111, 0b11111100, 0b10111111, 0b11101001, 0b11111110];
            let ver = Version::Normal(1);
            let mut bs = BitStream::from(&data);
            let (mode, char_cnt) = take_header(&mut bs, ver).unwrap();
            assert_eq!(mode, Mode::Numeric);
            assert_eq!(char_cnt, 0b11_1111_1111);
            let (mode, char_cnt) = take_header(&mut bs, ver).unwrap();
            assert_eq!(mode, Mode::Alphanumeric);
            assert_eq!(char_cnt, 0b1_1111_1111);
            let (mode, char_cnt) = take_header(&mut bs, ver).unwrap();
            assert_eq!(mode, Mode::Byte);
            assert_eq!(char_cnt, 0b11111111);
        }

        #[test]
        fn test_take_header_v10() {
            let data = vec![
                0b00011111, 0b11111111, 0b00101111, 0b11111110, 0b10011111, 0b11111111, 0b11100000,
            ];
            let ver = Version::Normal(10);
            let mut bs = BitStream::from(&data);
            let (mode, char_cnt) = take_header(&mut bs, ver).unwrap();
            assert_eq!(mode, Mode::Numeric);
            assert_eq!(char_cnt, 0b1111_1111_1111);
            let (mode, char_cnt) = take_header(&mut bs, ver).unwrap();
            assert_eq!(mode, Mode::Alphanumeric);
            assert_eq!(char_cnt, 0b111_1111_1111);
            let (mode, char_cnt) = take_header(&mut bs, ver).unwrap();
            assert_eq!(mode, Mode::Byte);
            assert_eq!(char_cnt, 0b11111111_11111111);
        }

        #[test]
        fn test_take_header_v27() {
            let data = vec![
                0b00011111, 0b11111111, 0b11001011, 0b11111111, 0b11101001, 0b11111111, 0b11111110,
            ];
            let ver = Version::Normal(27);
            let mut bs = BitStream::from(&data);
            let (mode, char_cnt) = take_header(&mut bs, ver).unwrap();
            assert_eq!(mode, Mode::Numeric);
            assert_eq!(char_cnt, 0b11_1111_1111_1111);
            let (mode, char_cnt) = take_header(&mut bs, ver).unwrap();
            assert_eq!(mode, Mode::Alphanumeric);
            assert_eq!(char_cnt, 0b1_1111_1111_1111);
            let (mode, char_cnt) = take_header(&mut bs, ver).unwrap();
            assert_eq!(mode, Mode::Byte);
            assert_eq!(char_cnt, 0b11111111_11111111);
        }

        #[test]
        fn test_take_numeric_data() {
            let data = "12345".as_bytes();
            let ver = Version::Normal(1);
            let ecl = ECLevel::L;
            let pal = Palette::Mono;
            let mut bs = encode_with_version(data, ecl, ver, pal).unwrap();
            take_header(&mut bs, ver).unwrap();
            let numeric_data = take_numeric_data(&mut bs, 3);
            assert_eq!(numeric_data, "123".as_bytes().to_vec());
            let numeric_data = take_numeric_data(&mut bs, 2);
            assert_eq!(numeric_data, "45".as_bytes().to_vec());
            let data = "6".as_bytes();
            let mut bs = encode_with_version(data, ECLevel::L, ver, pal).unwrap();
            take_header(&mut bs, ver).unwrap();
            let numeric_data = take_numeric_data(&mut bs, 1);
            assert_eq!(numeric_data, "6".as_bytes().to_vec());
        }

        #[test]
        fn test_take_alphanumeric_data() {
            let data = "AC-".as_bytes();
            let ver = Version::Normal(1);
            let ecl = ECLevel::L;
            let pal = Palette::Mono;
            let mut bs = encode_with_version(data, ecl, ver, pal).unwrap();
            take_header(&mut bs, ver).unwrap();
            let alphanumeric_data = take_alphanumeric_data(&mut bs, 2);
            assert_eq!(alphanumeric_data, "AC".as_bytes().to_vec());
            let alphanumeric_data = take_alphanumeric_data(&mut bs, 1);
            assert_eq!(alphanumeric_data, "-".as_bytes().to_vec());
            let data = "%".as_bytes();
            let mut bs = encode_with_version(data, ECLevel::L, ver, pal).unwrap();
            take_header(&mut bs, ver).unwrap();
            let alphanumeric_data = take_alphanumeric_data(&mut bs, 1);
            assert_eq!(alphanumeric_data, "%".as_bytes().to_vec());
        }

        #[test]
        fn test_take_byte_data() {
            let data = "abc".as_bytes();
            let ver = Version::Normal(1);
            let ecl = ECLevel::L;
            let pal = Palette::Mono;
            let mut bs = encode_with_version(data, ecl, ver, pal).unwrap();
            take_header(&mut bs, ver).unwrap();
            let byte_data = take_byte_data(&mut bs, 2);
            assert_eq!(byte_data, "ab".as_bytes().to_vec());
            let byte_data = take_byte_data(&mut bs, 1);
            assert_eq!(byte_data, "c".as_bytes().to_vec());
        }

        #[test]
        fn test_take_segment() {
            let data = "abcABCDEF1234567890123ABCDEFabc".as_bytes();
            let ver = Version::Normal(2);
            let ecl = ECLevel::L;
            let pal = Palette::Mono;
            let mut bs = encode_with_version(data, ecl, ver, pal).unwrap();
            let seg_data = take_segment(&mut bs, ver).unwrap();
            assert_eq!(seg_data, "abc".as_bytes().to_vec());
            let seg_data = take_segment(&mut bs, ver).unwrap();
            assert_eq!(seg_data, "ABCDEF".as_bytes().to_vec());
            let seg_data = take_segment(&mut bs, ver).unwrap();
            assert_eq!(seg_data, "1234567890123".as_bytes().to_vec());
            let seg_data = take_segment(&mut bs, ver).unwrap();
            assert_eq!(seg_data, "ABCDEF".as_bytes().to_vec());
            let seg_data = take_segment(&mut bs, ver).unwrap();
            assert_eq!(seg_data, "abc".as_bytes().to_vec());
        }
    }
}

// Decoder
//------------------------------------------------------------------------------

mod decode {
    use super::reader::take_segment;
    use crate::{common::BitStream, Version};

    pub fn decode(encoded: &mut BitStream, ver: Version) -> Vec<u8> {
        let mut res = Vec::with_capacity(encoded.len());
        while let Some(decoded_seg) = take_segment(encoded, ver) {
            res.extend(decoded_seg);
        }
        res
    }

    #[cfg(test)]
    mod decode_tests {
        use super::super::encode::encode_with_version;
        use super::decode;
        use crate::{ECLevel, Palette, Version};

        #[test]
        fn test_decode() {
            let data = "abcABCDEF1234567890123ABCDEFabc".as_bytes();
            let ver = Version::Normal(2);
            let ecl = ECLevel::L;
            let pal = Palette::Mono;
            let mut bs = encode_with_version(data, ecl, ver, pal).unwrap();
            let decoded_data = decode(&mut bs, ver);
            assert_eq!(decoded_data, data);
        }
    }
}

// Global constants
//------------------------------------------------------------------------------

static PADDING_CODEWORDS: [u8; 2] = [0b1110_1100, 0b0001_0001];

static MODES: [Mode; 3] = [Mode::Numeric, Mode::Alphanumeric, Mode::Byte];
