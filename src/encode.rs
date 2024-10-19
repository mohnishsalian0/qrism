use crate::types::{ECLevel, QRError, QRResult, Version};
use std::{
    cmp::{min, Ordering},
    io::Read,
};

// Mode
//------------------------------------------------------------------------------

#[derive(Debug, PartialEq, Eq, Copy, Clone)]
pub enum Mode {
    Numeric = 0b0001,
    Alphanumeric = 0b0010,
    Byte = 0b0100,
}

impl Mode {
    pub fn char_count_bit_len(&self, version: Version) -> usize {
        debug_assert!(
            matches!(version, Version::Micro(1..=4) | Version::Normal(1..=40)),
            "Invalid version"
        );

        match version {
            Version::Micro(v) => match *self {
                Self::Numeric => v + 2,
                Self::Alphanumeric => v + 1,
                Self::Byte => v + 1,
            },
            Version::Normal(1..=9) => match *self {
                Self::Numeric => 10,
                Self::Alphanumeric => 9,
                Self::Byte => 8,
            },
            Version::Normal(10..=26) => match *self {
                Self::Numeric => 12,
                Self::Alphanumeric => 11,
                Self::Byte => 16,
            },
            Version::Normal(_) => match *self {
                Self::Numeric => 14,
                Self::Alphanumeric => 13,
                Self::Byte => 16,
            },
        }
    }

    pub fn data_bits_len(&self, raw_data: &[u8]) -> usize {
        let len = raw_data.len();
        match *self {
            Self::Numeric => (len * 10 + 2) / 3,
            Self::Alphanumeric => (len * 11 + 1) / 2,
            Self::Byte => len * 8,
        }
    }
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
        (char - b'0') as u16
    }

    #[inline]
    fn alphanumeric_digit(char: u8) -> u16 {
        match char {
            b'0'..=b'9' => (char - b'0') as u16,
            b'A'..=b'Z' => (char - b'A') as u16,
            b' ' => 36,
            b'$' => 37,
            b'%' => 38,
            b'*' => 39,
            b'+' => 40,
            b'-' => 41,
            b'.' => 42,
            b'/' => 43,
            b':' => 44,
            _ => 0,
        }
    }

    pub fn from(&self, data: &[u8]) -> u16 {
        match self {
            Self::Numeric => {
                debug_assert!(data.len() <= 3, "Data is too long for numeric conversion");
                data.iter()
                    .fold(0_u16, |n, b| n * 10 + Self::numeric_digit(*b))
            }
            Self::Alphanumeric => {
                debug_assert!(
                    data.len() <= 2,
                    "Data is too long for alphanumeric conversion"
                );
                data.iter()
                    .fold(0_u16, |n, b| n * 10 + Self::alphanumeric_digit(*b))
            }
            Self::Byte => {
                debug_assert!(data.len() == 1, "Data is too long for byte conversion");
                data[0] as u16
            }
        }
    }

    pub fn contains(&self, byte: u8) -> bool {
        match self {
            Self::Numeric => match byte {
                b'0'..=b'9' => true,
                _ => false,
            },
            Self::Alphanumeric => match byte {
                b'0'..=b'9' => true,
                b'A'..=b'Z' => true,
                b' ' => true,
                b'$' => true,
                b'%' => true,
                b'*' => true,
                b'+' => true,
                b'-' => true,
                b'.' => true,
                b'/' => true,
                b':' => true,
                _ => false,
            },
            _ => true,
        }
    }
}

#[cfg(test)]
mod mode_tests {
    use super::Mode::*;
    use super::Version::*;

    #[test]
    #[should_panic]
    fn test_char_count_bit_len_invalid_version_low() {
        Numeric.char_count_bit_len(Normal(0));
    }

    #[test]
    #[should_panic]
    fn test_char_count_bit_len_invalid_version_high() {
        Alphanumeric.char_count_bit_len(Normal(41));
    }

    #[test]
    #[should_panic]
    fn test_char_count_bit_len_invalid_version_max() {
        Alphanumeric.char_count_bit_len(Normal(usize::MAX));
    }

    #[test]
    fn test_comparison() {
        assert!(Numeric == Numeric);
        assert!(Numeric < Alphanumeric);
        assert!(Numeric < Byte);
        assert!(Alphanumeric == Alphanumeric);
        assert!(Alphanumeric < Byte);
        assert!(Byte == Byte);
    }
}

// Segment
//------------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct Segment<'a> {
    mode: Mode,
    data: &'a [u8], // Reference to raw data
}

impl<'a> Segment<'a> {
    pub fn new(mode: Mode, data: &'a [u8]) -> Self {
        Self { mode, data }
    }

    pub fn len(&self, version: Version) -> usize {
        let mode_len = version.get_mode_len();
        let char_count_len = self.mode.char_count_bit_len(version);
        let data_len = self.mode.data_bits_len(self.data);
        mode_len + char_count_len + data_len
    }
}

// Encoded data
//------------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct EncodedData {
    data: Vec<u8>,
    bit_offset: usize,
    version: Version,
}

impl EncodedData {
    fn new(capacity: usize, version: Version) -> Self {
        Self {
            data: Vec::with_capacity(capacity),
            bit_offset: 0,
            version,
        }
    }

    fn len(&self) -> usize {
        match self.bit_offset {
            0 => self.data.len() * 8,
            o => (self.data.len() - 1) * 8 + o,
        }
    }

    fn push_bits(&mut self, bit_len: usize, bits: u16) {
        debug_assert!(
            (4..=11).contains(&bit_len) && bit_len != 5 && bit_len != 9,
            "Invalid bits"
        );
        let shifted_len = bit_len + self.bit_offset;
        if self.bit_offset == 0 {
            if shifted_len <= 8 {
                self.data.push((bits << (8 - shifted_len)) as u8);
            } else {
                self.data.push((bits >> (shifted_len - 8)) as u8);
                self.data.push((bits << (16 - shifted_len)) as u8);
            }
        } else {
            let last = self.data.len() - 1;
            if shifted_len <= 8 {
                self.data[last] |= (bits << (8 - shifted_len)) as u8;
            } else if shifted_len <= 16 {
                self.data[last] |= (bits >> (shifted_len - 8)) as u8;
                self.data.push((bits << (16 - shifted_len)) as u8);
            } else {
                self.data[last] |= (bits >> (shifted_len - 8)) as u8;
                self.data.push((bits >> (shifted_len - 16)) as u8);
                self.data.push((bits << (24 - shifted_len)) as u8);
            }
        }
        self.bit_offset = shifted_len & 7;
    }

    fn push_header(&mut self, mode: Mode, char_count: usize) {
        self.push_bits(4, mode as u16);
        let char_count_bit_len = mode.char_count_bit_len(self.version);
        debug_assert!(
            char_count < (1 << char_count_bit_len),
            "Char count exceeds bit length"
        );
        self.push_bits(char_count_bit_len, char_count as u16);
    }

    fn push_numeric_data(&mut self, data: &[u8]) {
        self.push_header(Mode::Numeric, data.len());
        for chunk in data.chunks(3) {
            let len = (chunk.len() * 10 + 2) / 3;
            let data = Mode::Numeric.from(chunk);
            self.push_bits(len, data);
        }
    }

    fn push_alphanumeric_data(&mut self, data: &[u8]) {
        self.push_header(Mode::Alphanumeric, data.len());
        for chunk in data.chunks(2) {
            let len = (chunk.len() * 11 + 1) / 2;
            let data = Mode::Alphanumeric.from(chunk);
            self.push_bits(len, data);
        }
    }

    fn push_byte_data(&mut self, data: &[u8]) {
        self.push_header(Mode::Byte, data.len());
        for chunk in data.chunks(1) {
            let data = Mode::Byte.from(chunk);
            self.push_bits(8, data);
        }
    }

    fn push_segment(&mut self, seg: Segment) {
        match seg.mode {
            Mode::Numeric => self.push_numeric_data(seg.data),
            Mode::Alphanumeric => self.push_alphanumeric_data(seg.data),
            Mode::Byte => self.push_byte_data(seg.data),
        }
    }

    fn push_terminator(&mut self, ec_level: ECLevel) {
        let bit_len = self.len();
        let bit_capacity = self.version.get_bit_capacity(ec_level);

        debug_assert!(bit_len > bit_capacity, "Data too long");

        if bit_len < bit_capacity {
            let term_len = min(4, bit_capacity - bit_len);
            self.push_bits(term_len, 0);
        }
    }

    pub fn fill_remaining_capacity(&mut self, ec_level: ECLevel) {
        // Padding bits
        if self.bit_offset > 0 {
            let padding_bits_len = 8 - self.bit_offset;
            self.push_bits(padding_bits_len, 0);
        }

        debug_assert!(self.bit_offset == 0, "Offset is not 0 before padding");

        // Padding codewords
        let byte_capacity = self.version.get_bit_capacity(ec_level) / 8;
        let byte_len = self.data.len();
        PADDING_CODEWORDS
            .iter()
            .copied()
            .cycle()
            .take(byte_capacity - byte_len)
            .for_each(|pc| self.push_bits(8, pc as u16));
    }
}

// Encoder
//------------------------------------------------------------------------------

fn compute_optimal_segments(data: &[u8], version: Version) -> Vec<Segment> {
    debug_assert!(!data.is_empty(), "Empty data");

    // Dynamic programming to calculate optimum mode for each char
    const MODES: [Mode; 3] = [Mode::Byte, Mode::Alphanumeric, Mode::Numeric];
    let len = data.len();
    let mut prev_cost: [usize; 3] = [0; 3];
    MODES
        .iter()
        .enumerate()
        .for_each(|(i, &m)| prev_cost[i] = (4 + m.char_count_bit_len(version)) * 6);
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
                let mut cost = encoded_char_size + prev_cost[k];
                if to_mode != from_mode {
                    cost += (4 + to_mode.char_count_bit_len(version)) * 6;
                }
                if cost < cur_cost[j] {
                    cur_cost[j] = cost;
                    min_path[i][j] = k;
                }
            }
        }
        prev_cost.clone_from_slice(&cur_cost);
        cur_cost.fill(usize::MAX);
    }

    // Construct char mode vector from min_path
    let mut mode_index = 0;
    for i in 1..3 {
        if prev_cost[i] < prev_cost[mode_index] {
            mode_index = i;
        }
    }
    let char_mode: Vec<Mode> = (0..len)
        .rev()
        .scan(mode_index, |mi, i| {
            *mi = min_path[i][*mi];
            Some(MODES[*mi])
        })
        .collect();

    // Convert modes to segments
    let mut segs: Vec<Segment> = vec![];
    let mut seg_start = 0;
    let mut seg_mode = char_mode[0];
    for (i, &m) in char_mode.iter().enumerate().skip(1) {
        if seg_mode != m {
            segs.push(Segment::new(seg_mode, &data[seg_start..i]));
            seg_mode = m;
            seg_start = i;
        }
    }
    segs.push(Segment::new(seg_mode, &data[seg_start..len]));
    segs
}

fn find_min_version(data: &[u8], ec_level: ECLevel) -> QRResult<(Vec<Segment>, Version)> {
    let mut segments = vec![];
    let mut size = 0;
    for v in 1..40 {
        let version = Version::Normal(v);
        let capacity = version.get_bit_capacity(ec_level);
        if v == 1 || v == 10 || v == 27 {
            segments = compute_optimal_segments(data, version);
            size = segments.iter().fold(0, |a, s| a + s.len(version));
        }
        if size <= capacity {
            return Ok((segments, version));
        }
    }
    Err(QRError::DataTooLong)
}

pub fn encode(data: &[u8], ec_level: ECLevel) -> QRResult<EncodedData> {
    let (segments, version) = find_min_version(data, ec_level)?;
    let capacity = (version.get_bit_capacity(ec_level) + 7) / 8;
    let mut encoded_data = EncodedData::new(capacity, version);
    for seg in segments {
        encoded_data.push_segment(seg);
    }
    Ok(encoded_data)
}

pub fn encode_with_version(
    data: &[u8],
    ec_level: ECLevel,
    version: Version,
) -> QRResult<EncodedData> {
    let capacity = version.get_bit_capacity(ec_level);
    let segments = compute_optimal_segments(data, version);
    let size = segments.iter().fold(0, |a, s| a + s.len(version));
    if size > capacity {
        return Err(QRError::DataTooLong);
    }
    let mut encoded_data = EncodedData::new(capacity, version);
    for seg in segments {
        encoded_data.push_segment(seg);
    }
    Ok(encoded_data)
}

// Global constants
//------------------------------------------------------------------------------

static PADDING_CODEWORDS: [u8; 2] = [0b1110_1100, 0b0001_0001];
