pub use decode::*;

// Reader for encoded data
//------------------------------------------------------------------------------

mod reader {
    use std::cmp::min;

    use encoding_rs::SHIFT_JIS;

    use crate::codec::Mode;
    use crate::metadata::Version;
    use crate::utils::{BitStream, QRError, QRResult};

    pub fn write_segment(inp: &mut BitStream, ver: Version, out: &mut String) -> QRResult<usize> {
        let old_len = out.len();
        let (mode, char_cnt) = take_header(inp, ver)?;

        let decoded_len = match mode {
            Mode::Numeric => write_numeric(inp, char_cnt, out)?,
            Mode::Alphanumeric => write_alphanumeric(inp, char_cnt, out)?,
            Mode::Byte => write_byte(inp, char_cnt, out)?,
            Mode::Kanji => write_kanji(inp, char_cnt, out)?,
            Mode::Eci => write_eci(inp, char_cnt, out)?,
            Mode::Terminator => return Ok(0),
        };

        let encoded_len = mode.encoded_len(decoded_len);
        let bit_len = ver.mode_bits() + ver.char_cnt_bits(mode) + encoded_len;

        Ok(bit_len)
    }

    fn take_header(inp: &mut BitStream, ver: Version) -> QRResult<(Mode, usize)> {
        let mode_bits = inp.take_bits(4).unwrap_or(0);

        let mode = match mode_bits {
            0 => Mode::Terminator,
            1 => Mode::Numeric,
            2 => Mode::Alphanumeric,
            4 => Mode::Byte,
            7 => Mode::Eci,
            8 => Mode::Kanji,
            _ => return Err(QRError::InvalidMode(mode_bits as u8)),
        };

        let len_bits = ver.char_cnt_bits(mode);
        let char_cnt = inp.take_bits(len_bits).ok_or(QRError::CorruptDataSegment)?;

        Ok((mode, char_cnt.into()))
    }

    fn write_numeric(
        inp: &mut BitStream,
        mut char_cnt: usize,
        out: &mut String,
    ) -> QRResult<usize> {
        let mut total_bit_len = 0;

        while char_cnt > 0 {
            let bit_len = if char_cnt > 2 { 10 } else { (char_cnt % 3) * 3 + 1 };
            let chunk = inp.take_bits(bit_len).ok_or(QRError::CorruptDataSegment)?;
            let decoded = Mode::Numeric.decode_chunk(chunk, bit_len);
            total_bit_len += decoded.len();
            let decoded_str =
                String::from_utf8(decoded).map_err(|_| QRError::InvalidUTF8Encoding)?;
            out.push_str(&decoded_str);
            char_cnt -= min(3, char_cnt);
        }

        Ok(total_bit_len)
    }

    fn write_alphanumeric(
        inp: &mut BitStream,
        mut char_cnt: usize,
        out: &mut String,
    ) -> QRResult<usize> {
        let mut total_bit_len = 0;

        while char_cnt > 0 {
            let bit_len = if char_cnt > 1 { 11 } else { 6 };
            let chunk = inp.take_bits(bit_len).ok_or(QRError::CorruptDataSegment)?;
            let decoded = Mode::Alphanumeric.decode_chunk(chunk, bit_len);
            total_bit_len += decoded.len();
            let decoded_str =
                String::from_utf8(decoded).map_err(|_| QRError::InvalidUTF8Encoding)?;
            out.push_str(&decoded_str);
            char_cnt -= min(2, char_cnt);
        }

        Ok(total_bit_len)
    }

    fn write_byte(inp: &mut BitStream, mut char_cnt: usize, out: &mut String) -> QRResult<usize> {
        let mut total_bit_len = 0;
        let mut bytes = Vec::with_capacity(char_cnt);

        while char_cnt > 0 {
            let chunk = inp.take_bits(8).ok_or(QRError::CorruptDataSegment)?;
            let decoded = Mode::Byte.decode_chunk(chunk, 8);
            total_bit_len += 1;
            bytes.extend(decoded);
            char_cnt -= 1;
        }

        match String::from_utf8(bytes.clone()) {
            Ok(utf8) => out.push_str(&utf8),
            Err(_) => {
                let (kanji, _, has_err) = SHIFT_JIS.decode(&bytes);

                if has_err {
                    return Err(QRError::InvalidCharacterEncoding);
                }

                out.push_str(&kanji);
            }
        }

        Ok(total_bit_len)
    }

    fn write_kanji(inp: &mut BitStream, mut char_cnt: usize, out: &mut String) -> QRResult<usize> {
        let mut total_bit_len = 0;

        while char_cnt > 0 {
            let chunk = inp.take_bits(13).ok_or(QRError::CorruptDataSegment)?;
            let decoded = Mode::Kanji.decode_chunk(chunk, 13);
            total_bit_len += decoded.len();
            let (decoded_str, _, has_err) = SHIFT_JIS.decode(&decoded);

            if has_err {
                return Err(QRError::CorruptDataSegment);
            }

            out.push_str(&decoded_str);
            char_cnt -= 1;
        }

        Ok(total_bit_len)
    }

    fn write_eci(inp: &mut BitStream, _char_cnt: usize, _out: &mut str) -> QRResult<usize> {
        let mut total_bit_len = 8;
        let mut _eci = inp.take_bits(8).ok_or(QRError::CorruptDataSegment)? as u32;

        if _eci & 0b1100_0000 == 0b1000_0000 {
            _eci = (_eci << 8) | inp.take_bits(8).ok_or(QRError::CorruptDataSegment)? as u32;
            total_bit_len += 8;
        } else if _eci & 0b1110_0000 == 0b1100_0000 {
            _eci = (_eci << 16) | inp.take_bits(16).ok_or(QRError::CorruptDataSegment)? as u32;
            total_bit_len += 16;
        }

        Ok(total_bit_len)
    }

    #[cfg(test)]
    mod reader_tests {
        use super::{
            take_header, write_alphanumeric, write_byte, write_numeric, write_segment, BitStream,
            Mode,
        };
        use crate::common::codec::encoder::encode_with_version;
        use crate::{ECLevel, Version};

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
        fn test_take_numeric() {
            let data = "12345".as_bytes();
            let ver = Version::Normal(1);
            let ecl = ECLevel::L;
            let hi_cap = false;
            let mut bs = encode_with_version(data, ver, ecl, hi_cap).unwrap();
            let mut out = String::with_capacity(100);

            take_header(&mut bs, ver).unwrap();

            write_numeric(&mut bs, 3, &mut out).unwrap();
            assert_eq!(out, "123");
            out.clear();

            write_numeric(&mut bs, 2, &mut out).unwrap();
            assert_eq!(out, "45");
            out.clear();

            let data = "6".as_bytes();
            let mut bs = encode_with_version(data, ver, ECLevel::L, hi_cap).unwrap();
            take_header(&mut bs, ver).unwrap();
            write_numeric(&mut bs, 1, &mut out).unwrap();
            assert_eq!(out, "6");
        }

        #[test]
        fn test_write_alphanumeric() {
            let data = "AC-".as_bytes();
            let ver = Version::Normal(1);
            let ecl = ECLevel::L;
            let hi_cap = false;
            let mut bs = encode_with_version(data, ver, ecl, hi_cap).unwrap();
            let mut out = String::with_capacity(100);

            take_header(&mut bs, ver).unwrap();

            write_alphanumeric(&mut bs, 2, &mut out).unwrap();
            assert_eq!(out, "AC");
            out.clear();

            write_alphanumeric(&mut bs, 1, &mut out).unwrap();
            assert_eq!(out, "-");
            out.clear();

            let data = "%".as_bytes();
            let mut bs = encode_with_version(data, ver, ECLevel::L, hi_cap).unwrap();
            take_header(&mut bs, ver).unwrap();
            write_alphanumeric(&mut bs, 1, &mut out).unwrap();
            assert_eq!(out, "%");
        }

        #[test]
        fn test_write_byte() {
            let data = "abc".as_bytes();
            let ver = Version::Normal(1);
            let ecl = ECLevel::L;
            let hi_cap = false;
            let mut bs = encode_with_version(data, ver, ecl, hi_cap).unwrap();
            let mut out = String::with_capacity(100);

            take_header(&mut bs, ver).unwrap();

            write_byte(&mut bs, 2, &mut out).unwrap();
            assert_eq!(out, "ab");
            out.clear();

            write_byte(&mut bs, 1, &mut out).unwrap();
            assert_eq!(out, "c");
        }

        #[test]
        fn test_write_segment() {
            let data = "abcABCDEF1234567890123ABCDEFabc".as_bytes();
            let ver = Version::Normal(2);
            let ecl = ECLevel::L;
            let hi_cap = false;
            let mut bs = encode_with_version(data, ver, ecl, hi_cap).unwrap();
            let mut out = String::with_capacity(100);

            write_segment(&mut bs, ver, &mut out).unwrap();
            assert_eq!(out, "abc");
            out.clear();

            write_segment(&mut bs, ver, &mut out).unwrap();
            assert_eq!(out, "ABCDEF");
            out.clear();

            write_segment(&mut bs, ver, &mut out).unwrap();
            assert_eq!(out, "1234567890123");
            out.clear();

            write_segment(&mut bs, ver, &mut out).unwrap();
            assert_eq!(out, "ABCDEF");
            out.clear();

            write_segment(&mut bs, ver, &mut out).unwrap();
            assert_eq!(out, "abc");
        }
    }
}

// Decoder
//------------------------------------------------------------------------------

pub mod decode {
    use super::reader::write_segment;
    use crate::utils::{BitStream, QRResult};
    use crate::{ECLevel, Version};

    pub fn decode(
        encoded: &mut BitStream,
        ver: Version,
        ecl: ECLevel,
        hi_cap: bool,
    ) -> QRResult<String> {
        let bcap = ver.data_bit_capacity(ecl, false);
        let mut res = String::with_capacity(encoded.len());
        let mut bit_len = 0;
        loop {
            let seg_bit_len = write_segment(encoded, ver, &mut res)?;
            if seg_bit_len == 0 {
                break;
            }

            bit_len += seg_bit_len;

            // Handles an edge case where the diff between capacity and data len is less than
            // 4 bits, in which case there isn't enough space for 4 terminator bits, in the
            // absence of which the decoder would proceed to the next channel
            if bit_len <= bcap && bcap - bit_len < 4 && !hi_cap {
                break;
            }
        }
        Ok(res)
    }

    #[cfg(test)]
    mod decode_tests {
        use super::decode;
        use crate::codec::encode_with_version;
        use crate::{ECLevel, Version};

        #[test]
        fn test_decode() {
            let data = "abcABCDEF1234567890123ABCDEFabc";
            let ver = Version::Normal(2);
            let ecl = ECLevel::L;
            let hi_cap = false;
            let mut bs = encode_with_version(data.as_bytes(), ver, ecl, hi_cap).unwrap();
            let decoded_data = decode(&mut bs, ver, ecl, hi_cap).unwrap();
            assert_eq!(decoded_data, data);
        }
    }
}
