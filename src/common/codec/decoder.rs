pub use decode::*;

// Reader for encoded data
//------------------------------------------------------------------------------

mod reader {
    use std::cmp::min;

    use crate::codec::Mode;
    use crate::metadata::Version;
    use crate::utils::BitStream;

    pub fn take_segment(inp: &mut BitStream, ver: Version) -> Option<(Vec<u8>, usize)> {
        let (mode, char_cnt) = take_header(inp, ver)?;
        let byte_data = match mode {
            Mode::Numeric => take_numeric_data(inp, char_cnt),
            Mode::Alphanumeric => take_alphanumeric_data(inp, char_cnt),
            Mode::Byte => take_byte_data(inp, char_cnt),
        };
        let encoded_len = mode.encoded_len(byte_data.len());
        let bit_len = ver.mode_bits() + ver.char_cnt_bits(mode) + encoded_len;
        Some((byte_data, bit_len))
    }

    fn take_header(inp: &mut BitStream, ver: Version) -> Option<(Mode, usize)> {
        let mode_bits = inp.take_bits(4)?;
        let mode = match mode_bits {
            0 => return None,
            1 => Mode::Numeric,
            2 => Mode::Alphanumeric,
            4 => Mode::Byte,
            _ => unreachable!("Unsupported Mode: {mode_bits}"),
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
        use super::{
            take_alphanumeric_data, take_byte_data, take_header, take_numeric_data, take_segment,
            BitStream, Mode,
        };
        use crate::common::codec::encoder::encode_with_version;
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
            let mut bs = encode_with_version(data, ver, ecl, pal).unwrap();
            take_header(&mut bs, ver).unwrap();
            let numeric_data = take_numeric_data(&mut bs, 3);
            assert_eq!(numeric_data, "123".as_bytes().to_vec());
            let numeric_data = take_numeric_data(&mut bs, 2);
            assert_eq!(numeric_data, "45".as_bytes().to_vec());
            let data = "6".as_bytes();
            let mut bs = encode_with_version(data, ver, ECLevel::L, pal).unwrap();
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
            let mut bs = encode_with_version(data, ver, ecl, pal).unwrap();
            take_header(&mut bs, ver).unwrap();
            let alphanumeric_data = take_alphanumeric_data(&mut bs, 2);
            assert_eq!(alphanumeric_data, "AC".as_bytes().to_vec());
            let alphanumeric_data = take_alphanumeric_data(&mut bs, 1);
            assert_eq!(alphanumeric_data, "-".as_bytes().to_vec());
            let data = "%".as_bytes();
            let mut bs = encode_with_version(data, ver, ECLevel::L, pal).unwrap();
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
            let mut bs = encode_with_version(data, ver, ecl, pal).unwrap();
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
            let mut bs = encode_with_version(data, ver, ecl, pal).unwrap();
            let (seg_data, _) = take_segment(&mut bs, ver).unwrap();
            assert_eq!(seg_data, "abc".as_bytes().to_vec());
            let (seg_data, _) = take_segment(&mut bs, ver).unwrap();
            assert_eq!(seg_data, "ABCDEF".as_bytes().to_vec());
            let (seg_data, _) = take_segment(&mut bs, ver).unwrap();
            assert_eq!(seg_data, "1234567890123".as_bytes().to_vec());
            let (seg_data, _) = take_segment(&mut bs, ver).unwrap();
            assert_eq!(seg_data, "ABCDEF".as_bytes().to_vec());
            let (seg_data, _) = take_segment(&mut bs, ver).unwrap();
            assert_eq!(seg_data, "abc".as_bytes().to_vec());
        }
    }
}

// Decoder
//------------------------------------------------------------------------------

pub mod decode {
    use super::reader::take_segment;
    use crate::utils::BitStream;
    use crate::{ECLevel, Palette, Version};

    pub fn decode(encoded: &mut BitStream, ver: Version, ecl: ECLevel, pal: Palette) -> Vec<u8> {
        let data_bit_cap = ver.data_bit_capacity(ecl, Palette::Mono);
        let mut res = Vec::with_capacity(encoded.len());
        let mut total_bit_len = 0;
        while let Some((decoded_seg, bit_len)) = take_segment(encoded, ver) {
            res.extend(decoded_seg);
            total_bit_len += bit_len;

            //FIXME: Remove
            println!("Total bit len: {total_bit_len}, Data cap: {data_bit_cap}, Palette {pal:?}");

            // Handles an edge case where the diff between capacity and data len is less than
            // 4 bits, in which case there isn't enough space for 4 terminator bits, in the
            // absence of which the decoder would proceed to the next channel
            if total_bit_len <= data_bit_cap
                && data_bit_cap - total_bit_len < 4
                && pal == Palette::Mono
            {
                break;
            }
        }
        res
    }

    #[cfg(test)]
    mod decode_tests {
        use super::decode;
        use crate::codec::encode_with_version;
        use crate::{ECLevel, Palette, Version};

        #[test]
        fn test_decode() {
            let data = "abcABCDEF1234567890123ABCDEFabc".as_bytes();
            let ver = Version::Normal(2);
            let ecl = ECLevel::L;
            let pal = Palette::Mono;
            let mut bs = encode_with_version(data, ver, ecl, pal).unwrap();
            let decoded_data = decode(&mut bs, ver, ecl, pal);
            assert_eq!(decoded_data, data);
        }
    }
}
