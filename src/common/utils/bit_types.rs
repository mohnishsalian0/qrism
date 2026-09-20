use core::panic;
use std::{fmt::Display, mem};

use num_traits::PrimInt;

// Bit stream
//------------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct BitStream {
    data: Box<[u8; MAX_PAYLOAD_SIZE]>,
    // Bit length
    len: usize,
    // Max bit capacity
    capacity: usize,
    // Pointer to take bits
    cursor: usize,
}

impl BitStream {
    pub fn new(capacity: usize) -> Self {
        Self { data: Box::new([0; MAX_PAYLOAD_SIZE]), len: 0, capacity, cursor: 0 }
    }

    pub fn from(inp: &[u8]) -> Self {
        let len = inp.len();
        let bit_len = len << 3;
        let mut data = Box::new([0; MAX_PAYLOAD_SIZE]);
        data[..len].copy_from_slice(inp);
        Self { data, len: bit_len, capacity: bit_len, cursor: 0 }
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn capacity(&self) -> usize {
        self.capacity
    }

    pub fn bits_left(&self) -> usize {
        self.len - self.cursor
    }

    pub fn data(&self) -> &[u8] {
        &self.data[..(self.len + 7) >> 3]
    }

    pub fn truncate(&mut self, len: usize) {
        debug_assert!(
            len < self.len,
            "Truncate length must be less than current length: Current length {}, Truncate length {}",
            self.len,
            len
        );
        self.len = len;
    }
}

// Push bits for bit stream
//------------------------------------------------------------------------------

impl BitStream {
    pub fn push_byte(&mut self, byte: u8) {
        debug_assert!(
            self.len + 8 <= self.capacity,
            "Insufficient capacity: Capacity {}, Size {}",
            self.capacity,
            self.len + 8
        );

        let off = self.len & 7;
        let pos = self.len >> 3;

        if off == 0 {
            self.data[pos] = byte;
            self.len += 8;
        } else {
            self.push_bits(byte, 8);
        }
    }

    pub fn push_bits<T>(&mut self, bits: T, size: usize)
    where
        T: PrimInt + Display,
    {
        let max_bits = mem::size_of::<T>() * 8;
        debug_assert!(
            size >= max_bits - bits.leading_zeros() as usize,
            "Bit count shouldn't exceed bit length: Length {size}, Bits {bits}"
        );
        debug_assert!(
            self.len + size <= self.capacity,
            "Insufficient capacity: Capacity {}, Size {}",
            self.capacity,
            self.len + size
        );

        match size {
            0 => (),
            1..=8 => {
                let bits = bits.to_u8().unwrap();
                let off = self.len & 7;
                let pos = self.len >> 3;

                if off + size <= 8 {
                    self.data[pos] |= bits << (8 - size - off);
                } else {
                    self.data[pos] |= bits >> (size + off - 8);
                    self.data[pos + 1] = bits << (16 - size - off);
                }

                self.len += size;
            }
            9..=16 => {
                self.push_bits((bits >> 8).to_u8().unwrap(), size - 8);
                self.push_bits((bits & T::from(0xFF).unwrap()).to_u8().unwrap(), 8);
            }
            _ => panic!("Bits from only u8 and u16 can be pushed"),
        }
    }

    pub fn push(&mut self, bit: bool) {
        debug_assert!(
            self.len < self.capacity,
            "Insufficient capacity: Capacity {}, Size {}",
            self.capacity,
            self.len + 1
        );

        if bit {
            let off = self.len & 7;
            let pos = self.len >> 3;
            self.data[pos] |= 0b10000000 >> off;
        }

        self.len += 1;
    }

    pub fn extend(&mut self, arr: &[u8]) {
        debug_assert!(
            (self.len & 7) == 0,
            "Bit offset must be zero to extend from another array: Bit offset {}",
            self.len & 7
        );
        let pos = self.len >> 3;
        let arr_bits = arr.len() << 3;
        debug_assert!(
            self.len + arr_bits <= self.capacity,
            "Extension shouldn't overflow capacity: Capacity {}, Size {}",
            self.capacity,
            self.len + arr_bits
        );
        self.data[pos..pos + arr.len()].copy_from_slice(arr);
        self.len += arr_bits;
    }
}

#[cfg(test)]
mod bit_stream_push_tests {

    use super::BitStream;

    #[test]
    fn test_len() {
        let bit_capacity = 152;
        let mut bs = BitStream::new(bit_capacity);
        assert_eq!(bs.len(), 0);
        bs.push_bits(0, 0);
        assert_eq!(bs.len(), 0);
        bs.push_bits(0b1000, 4);
        assert_eq!(bs.len(), 4);
        bs.push_bits(0b1000, 8);
        assert_eq!(bs.len(), 12);
        bs.push_bits(0b1000, 4);
        assert_eq!(bs.len(), 16);
        bs.push_bits(0b1111111, 7);
        assert_eq!(bs.len(), 23);
        bs.push_bits(0b111111111111, 12);
        assert_eq!(bs.len(), 35);
        bs.push_bits(0b111111111111, 16);
        assert_eq!(bs.len(), 51);
    }

    #[test]
    #[should_panic]
    fn test_invalid_len() {
        let bit_capacity = 152;
        let mut bs = BitStream::new(bit_capacity);
        bs.push_bits(256, 17);
    }

    #[test]
    fn test_push() {
        let mut bs = BitStream::new(2);
        bs.push(false);
        assert_eq!(bs.data[..1], vec![0b00000000]);
        bs.push(true);
        assert_eq!(bs.data[..1], vec![0b01000000]);
    }

    #[test]
    fn test_push_bits() {
        let bit_capacity = 152;
        let exp_vec = [210, 52, 141, 35, 72, 210, 183, 42, 7, 219, 91, 14, 253, 68, 120, 193];
        let mut inp = BitStream::from(&exp_vec);
        let mut out = BitStream::new(bit_capacity);
        for n in [0, 1, 2, 3, 4, 5, 6, 7, 8, 4, 8, 9, 11, 15, 16, 5, 16] {
            let bits = inp.take_bits(n).unwrap();
            out.push_bits(bits, n);
            let out_off = out.len() & 7;
            let len = out.len() >> 3;
            assert_eq!(out.data[..len], exp_vec[..len], "n {n}");
            if out_off > 0 {
                assert_eq!(out.data[len] >> (8 - out_off), exp_vec[len] >> (8 - out_off));
            }
        }
    }

    #[test]
    #[should_panic]
    fn test_push_bits_capacity_overflow() {
        let bit_capacity = 152;
        let capacity = (bit_capacity + 7) >> 3;
        let mut bs = BitStream::new(bit_capacity);
        for _ in 0..capacity {
            bs.push_bits(8, 0b1);
        }
        bs.push_bits(1, 0b1)
    }
}

// Take bits for bit stream
//------------------------------------------------------------------------------

impl BitStream {
    pub fn take_bits(&mut self, n: usize) -> Option<u16> {
        debug_assert!(n <= 16, "Cannot take more than 16 bits: N {n}");

        if self.cursor + n > self.len {
            return None;
        }

        let off = self.cursor & 7;
        let pos = self.cursor >> 3;

        let mut res = (self.data[pos] as u32) << 16;
        if off + n > 8 {
            res |= (self.data[pos + 1] as u32) << 8;
        }
        if off + n > 16 {
            res |= self.data[pos + 2] as u32;
        }
        res >>= 24 - off - n;
        res &= (1 << n) - 1;

        self.cursor += n;
        Some(res as u16)
    }

    pub fn take_bit(&mut self) -> Option<bool> {
        if self.cursor == self.len {
            return None;
        }

        let off = self.cursor & 7;
        let pos = self.cursor >> 3;
        let bit = (self.data[pos] << off) >> 7;

        self.cursor += 1;

        Some(bit != 0)
    }
}

#[cfg(test)]
mod bit_stream_take_tests {

    use super::BitStream;

    #[test]
    fn test_take_bits() {
        let data = [
            0b11010010, 0b00110100, 0b10001101, 0b00100011, 0b01001000, 0b11010010, 0b00110100,
            0b10001101, 0b00100011, 0b01001000, 0b11010010, 0b00110100, 0b10001100,
        ];
        let mut bs = BitStream::from(&data);
        let bits = bs.take_bits(0);
        assert_eq!(bits, Some(0));
        let bits = bs.take_bits(4);
        assert_eq!(bits, Some(0b1101));
        let bits = bs.take_bits(4);
        assert_eq!(bits, Some(0b0010));
        let bits = bs.take_bits(8);
        assert_eq!(bits, Some(0b00110100));
        let bits = bs.take_bits(9);
        assert_eq!(bits, Some(0b100011010));
        let bits = bs.take_bits(7);
        assert_eq!(bits, Some(0b0100011));
        let bits = bs.take_bits(16);
        assert_eq!(bits, Some(0b01001000_11010010));
        let bits = bs.take_bits(1);
        assert_eq!(bits, Some(0b0));
        let bits = bs.take_bits(11);
        assert_eq!(bits, Some(0b01101001000));
        let bits = bs.take_bits(14);
        assert_eq!(bits, Some(0b11010010001101));
        let bits = bs.take_bits(16);
        assert_eq!(bits, Some(0b0010001101001000));
        let bits = bs.take_bits(4);
        assert_eq!(bits, Some(0b1101));
        let bits = bs.take_bits(4);
        assert_eq!(bits, Some(0b0010));
    }

    #[test]
    #[should_panic]
    fn test_take_bits_over_capacity() {
        let data = vec![];
        let mut eb = BitStream::from(&data);
        eb.take_bits(5).unwrap();
    }
}

// Iterator for bit stream
//------------------------------------------------------------------------------

impl Iterator for BitStream {
    type Item = bool;
    fn next(&mut self) -> Option<Self::Item> {
        self.take_bit()
    }
}

// Bit array
//------------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct BitArray {
    data: [u8; MAX_PAYLOAD_SIZE],
    // Fixed bit length of array
    len: usize,
}

impl BitArray {
    pub fn new(len: usize) -> Self {
        Self { data: [0; MAX_PAYLOAD_SIZE], len }
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn data(&self) -> &[u8] {
        &self.data[..(self.len + 7) >> 3]
    }
}

// Put bits for bit array
//------------------------------------------------------------------------------

impl BitArray {
    pub fn put(&mut self, pos: usize, bit: bool) {
        debug_assert!(pos < self.len, "Out of bitarray bounds: Len {}, Pos {}", self.len, pos);

        let off = pos & 7;
        let index = pos >> 3;

        if bit {
            self.data[index] |= (0b10000000) >> off;
        } else {
            self.data[index] &= !(0b10000000 >> off);
        }
    }
}

// Bit matrix
//------------------------------------------------------------------------------

/// A packed grid of `w`×`h` elements, each `elem_bits` wide. `get`/`put` address an element by its
/// column/row and stride by `elem_bits` internally, so callers never multiply the column by hand.
/// `elem_bits` is required to be a factor of 64, so every element sits wholly within one 64-bit
/// word, no element ever straddles a word boundary. Elements are arranged right to left within a
/// word. Which means an element at x=0, y=0 will be placed at right end of first word in the
/// matrix.
#[derive(Debug, Clone)]
pub struct BitMatrix {
    data: Vec<u64>,
    len: usize,
    w: u32,
    h: u32,
    elem_bits: u32,
    mask: u64,
}

impl BitMatrix {
    pub fn new(w: u32, h: u32, elem_bits: u32) -> Self {
        debug_assert!((1..=64).contains(&elem_bits), "elem_bits must be 1..=64: {elem_bits}");
        debug_assert!(64 % elem_bits == 0, "elem_bits must be a factor of 64: {elem_bits}");
        let cap = ((w * h * elem_bits + 63) >> 6) as usize;
        let mask = if elem_bits == 64 { u64::MAX } else { (1u64 << elem_bits) - 1 };
        Self { data: vec![0u64; cap], len: 0, w, h, elem_bits, mask }
    }

    pub fn width(&self) -> u32 {
        self.w
    }

    pub fn height(&self) -> u32 {
        self.h
    }

    pub fn capacity(&self) -> usize {
        (self.w * self.h * self.elem_bits) as usize
    }

    pub fn elem_bits(&self) -> u32 {
        self.elem_bits
    }

    pub fn data(&self) -> &[u64] {
        &self.data
    }
}

// Get/put elements for bit matrix
//------------------------------------------------------------------------------

impl BitMatrix {
    pub fn get(&self, x: u32, y: u32) -> u64 {
        debug_assert!(x < self.w, "X coordinate is out of bounds: Width {}, X {}", self.w, x);
        debug_assert!(y < self.h, "Y coordinate is out of bounds: Height {}, Y {}", self.h, y);

        let (idx, off) = self.elem_pos(x, y);

        debug_assert!(
            idx < self.data.len(),
            "Out of bit matrix bounds: Len {}, Index {}",
            self.data.len(),
            idx
        );

        // `elem_bits` divides 64, so the element never crosses into the next word.
        (self.data[idx] >> off) & self.mask
    }

    pub fn get_bit(&self, x: u32, y: u32) -> bool {
        debug_assert!(self.elem_bits == 1);
        debug_assert!(x < self.w, "X coordinate is out of bounds: Width {}, X {}", self.w, x);
        debug_assert!(y < self.h, "Y coordinate is out of bounds: Height {}, Y {}", self.h, y);

        let (idx, off) = self.elem_pos(x, y);

        debug_assert!(
            idx < self.data.len(),
            "Out of bit matrix bounds: Len {}, Index {}",
            self.data.len(),
            idx
        );

        ((self.data[idx] >> off) & 1) != 0
    }

    /// Returns the element at `(x, y)` together with the length of the maximal run of that same
    /// element starting there and ending at row `y`'s right edge. Scanning a row run-by-run
    /// instead of pixel-by-pixel. XORing a word against the element broadcast into all its lanes
    /// zeroes the lanes that match, so `trailing_zeros` of the result names the first differing lane
    /// directly.
    pub fn run(&self, x: u32, y: u32) -> (bool, u32) {
        debug_assert!(self.elem_bits == 1, "Bit size should be 1, but is {}", self.elem_bits);
        debug_assert!(x < self.w, "X coordinate is out of bounds: Width {}, X {}", self.w, x);
        debug_assert!(y < self.h, "Y coordinate is out of bounds: Height {}, Y {}", self.h, y);

        let remaining = self.w - x;

        let (mut idx, mut off) = self.elem_pos(x, y);

        let elem = (self.data[idx] >> off) & 1;

        // Broadcast the element into every lane.
        let pattern = elem.wrapping_neg();

        // Lanes past the row's end are still compared, so the count is clamped at the end rather
        // than masked off per word. Lanes past the matrix's end read as zero padding, which can only
        // extend a run of zeroes, also caught by the clamp.
        let mut run = 0;
        while run < remaining {
            let diff = (self.data[idx] ^ pattern) >> off;
            if diff == 0 {
                run += 64 - off;
                idx += 1;
                off = 0;
            } else {
                run += diff.trailing_zeros();
                break;
            }
        }

        (elem != 0, run.min(remaining))
    }

    pub fn put(&mut self, x: u32, y: u32, bits: u64) {
        debug_assert!(x < self.w, "X coordinate is out of bounds: Width {}, X {}", self.w, x);
        debug_assert!(y < self.h, "Y coordinate is out of bounds: Height {}, Y {}", self.h, y);
        debug_assert!(
            self.elem_bits == 64 || bits >> self.elem_bits == 0,
            "bits {bits} do not fit in elem_bits {}",
            self.elem_bits
        );

        let (idx, off) = self.elem_pos(x, y);

        debug_assert!(
            idx < self.data.len(),
            "Out of bit matrix bounds: Len {}, Index {}",
            self.data.len(),
            idx
        );

        // `elem_bits` divides 64, so the element sits wholly within word `idx`.
        self.data[idx] = (self.data[idx] & !(self.mask << off)) | (bits << off);
    }

    // Push bits of length n to the end
    pub fn push_bits(&mut self, bits: u64, n: usize) {
        debug_assert!(self.len + n <= self.capacity(), "Bit matrix capacity overflow");
        debug_assert!(n <= 64, "Bit length is over 64");
        debug_assert!(n == 64 || bits >> n == 0);

        if n == 0 {
            return;
        }

        let idx = self.len >> 6;
        let off = self.len & 63;

        self.data[idx] |= bits << off;

        if off + n > 64 {
            self.data[idx + 1] |= bits >> (64 - off);
        }

        self.len += n;
    }

    fn elem_pos(&self, x: u32, y: u32) -> (usize, u32) {
        let flat_pos = (y * self.w + x) * self.elem_bits;
        let idx = (flat_pos >> 6) as usize;
        let off = flat_pos & 63;

        (idx, off)
    }
}

#[cfg(test)]
mod bit_matrix_tests {

    use super::BitMatrix;

    #[test]
    fn test_new_is_all_zero() {
        let (w, h) = (10, 7);
        let bm = BitMatrix::new(w, h, 1);
        assert_eq!(bm.width(), w);
        assert_eq!(bm.height(), h);
        assert_eq!(bm.elem_bits(), 1);
        // ceil(70 / 64) = 2 words
        assert_eq!(bm.data().len(), 2);
        for y in 0..h {
            for x in 0..w {
                assert_eq!(bm.get(x, y), 0, "fresh matrix should be all zero at ({x}, {y})");
            }
        }
    }

    #[test]
    fn test_new_multibit_capacity() {
        let bm = BitMatrix::new(10, 7, 4);
        assert_eq!(bm.elem_bits(), 4);
        // 10 * 7 * 4 = 280 bits -> ceil(280 / 64) = 5 words
        assert_eq!(bm.data().len(), 5);
    }

    #[test]
    fn test_run_matches_naive_scan() {
        for &(w, h) in &[(1u32, 1u32), (7, 5), (63, 3), (64, 3), (65, 3), (130, 4)] {
            let mut bm = BitMatrix::new(w, h, 1);
            let max = 1;

            // A deterministic mix of long runs and rapid flips.
            let mut seed = 0x9e3779b9u64;
            for y in 0..h {
                for x in 0..w {
                    seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
                    let v = if (seed >> 33) % 3 == 0 {
                        (seed >> 17) & max
                    } else {
                        (x as u64 / 5) & max
                    };
                    bm.put(x, y, v);
                }
            }

            for y in 0..h {
                for x in 0..w {
                    let (elem, len) = bm.run(x, y);
                    assert_eq!(elem, bm.get_bit(x, y), "elem at ({x}, {y}) w={w}");

                    let mut naive = 0;
                    while x + naive < w && bm.get_bit(x + naive, y) == elem {
                        naive += 1;
                    }
                    assert_eq!(len, naive, "run length at ({x}, {y}) w={w} h={h}");
                }
            }
        }
    }

    #[test]
    fn test_run_spans_whole_uniform_row() {
        let (w, h) = (200, 3);
        let mut bm = BitMatrix::new(w, h, 1);
        for y in 0..h {
            for x in 0..w {
                bm.put(x, y, 1);
            }
        }
        for y in 0..h {
            assert_eq!(bm.run(0, y), (true, w), "y={y}");
            assert_eq!(bm.run(w - 1, y), (true, 1), "last element");
        }
    }

    #[test]
    fn test_put_get_roundtrip() {
        let mut bm = BitMatrix::new(10, 7, 1);
        bm.put(3, 2, 1);
        assert_eq!(bm.get(3, 2), 1);
        // Clearing a set element
        bm.put(3, 2, 0);
        assert_eq!(bm.get(3, 2), 0);
    }

    #[test]
    fn test_put_get_roundtrip_multibit() {
        let mut bm = BitMatrix::new(10, 7, 4);
        bm.put(3, 2, 0b1011);
        assert_eq!(bm.get(3, 2), 0b1011);
        bm.put(3, 2, 0b0110);
        assert_eq!(bm.get(3, 2), 0b0110, "overwrite must replace the element wholesale");
    }

    #[test]
    #[should_panic]
    fn test_put_oversized_element_panics() {
        let mut bm = BitMatrix::new(10, 1, 4);
        bm.put(0, 0, 0b1_0000); // needs 5 bits, only 4 allowed
    }

    #[test]
    fn test_not_transposed() {
        let mut bm = BitMatrix::new(10, 7, 1);
        bm.put(6, 2, 1);
        assert_eq!(bm.get(6, 2), 1, "the exact cell that was set must read back");
        // The mirror cell is a different location and must stay zero.
        assert_eq!(bm.get(2, 6), 0, "mirror cell must be unaffected");
    }

    #[test]
    fn test_elements_are_independent() {
        let mut bm = BitMatrix::new(10, 7, 4);
        bm.put(4, 3, 0b1010);
        // Every 4-neighbour stays zero.
        assert_eq!(bm.get(3, 3), 0);
        assert_eq!(bm.get(5, 3), 0);
        assert_eq!(bm.get(4, 2), 0);
        assert_eq!(bm.get(4, 4), 0);
        // Writing a neighbour doesn't disturb the set element.
        bm.put(5, 3, 0b0101);
        assert_eq!(bm.get(4, 3), 0b1010);
    }

    #[test]
    fn test_word_boundary() {
        let mut bm = BitMatrix::new(100, 2, 4);
        bm.put(15, 0, 0b1111); // last element of word 0 (bits 60..64)
        bm.put(16, 0, 0b1111); // first element of word 1 (bits 64..68)
        assert_eq!(bm.get(15, 0), 0b1111);
        assert_eq!(bm.get(16, 0), 0b1111);
        assert_eq!(bm.data()[0].count_ones(), 4);
        assert_eq!(bm.data()[1].count_ones(), 4);
        // They are genuinely distinct cells.
        bm.put(15, 0, 0);
        assert_eq!(bm.get(15, 0), 0);
        assert_eq!(bm.get(16, 0), 0b1111);
    }

    #[test]
    fn test_full_coverage() {
        let (w, h, elem_bits) = (13u32, 9u32, 4u32);
        let mut bm = BitMatrix::new(w, h, elem_bits);
        let val = |x: u32, y: u32| ((x + y) & 0b1111) as u64;
        for y in 0..h {
            for x in 0..w {
                bm.put(x, y, val(x, y));
            }
        }
        for y in 0..h {
            for x in 0..w {
                assert_eq!(bm.get(x, y), val(x, y), "mismatch at ({x}, {y})");
            }
        }
    }

    #[test]
    fn test_last_cell() {
        let (w, h) = (10, 7);
        let mut bm = BitMatrix::new(w, h, 1);
        bm.put(w - 1, h - 1, 1);
        assert_eq!(bm.get(w - 1, h - 1), 1);
        assert_eq!(bm.data().iter().map(|word| word.count_ones()).sum::<u32>(), 1);
    }

    #[test]
    fn test_full_word_element() {
        let mut bm = BitMatrix::new(3, 1, 64);
        let pattern = 0xF0F0_F0F0_0F0F_0F0Fu64;
        bm.put(1, 0, pattern);
        assert_eq!(bm.get(1, 0), pattern);
        assert_eq!(bm.get(0, 0), 0, "neighbouring word must stay zero");
        assert_eq!(bm.get(2, 0), 0);
    }

    #[test]
    #[should_panic]
    fn test_get_x_out_of_bounds() {
        let bm = BitMatrix::new(10, 7, 1);
        bm.get(10, 0);
    }

    #[test]
    #[should_panic]
    fn test_get_y_out_of_bounds() {
        let bm = BitMatrix::new(10, 7, 1);
        bm.get(0, 7);
    }

    #[test]
    #[should_panic]
    fn test_put_x_out_of_bounds() {
        let mut bm = BitMatrix::new(10, 7, 1);
        bm.put(10, 0, 1);
    }

    #[test]
    #[should_panic]
    fn test_new_non_factor_elem_bits_panics() {
        BitMatrix::new(10, 7, 3);
    }

    #[test]
    #[should_panic]
    fn test_new_zero_elem_bits_panics() {
        BitMatrix::new(10, 7, 0);
    }

    #[test]
    fn test_put_get_round_trip_sweep() {
        // Deterministic pseudo-random payloads via a small LCG.
        let mut seed = 0x9E37_79B9_7F4A_7C15u64;
        let mut next = || {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            seed
        };

        for elem_bits in [1u32, 2, 4, 8, 16, 32, 64] {
            let mask = if elem_bits == 64 { u64::MAX } else { (1u64 << elem_bits) - 1 };
            // Wide enough to span several words for the smaller element widths.
            let w = 200u32;
            let mut bm = BitMatrix::new(w, 1, elem_bits);

            let mut expected = vec![0u64; w as usize];
            for x in 0..w {
                let v = next() & mask;
                expected[x as usize] = v;
                bm.put(x, 0, v);
            }
            for x in 0..w {
                assert_eq!(
                    bm.get(x, 0),
                    expected[x as usize],
                    "round trip failed at x={x}, elem_bits={elem_bits}"
                );
            }
        }
    }

    #[test]
    fn test_get_bit_matches_get() {
        for &(w, h) in &[(1u32, 1u32), (7, 5), (63, 3), (64, 3), (65, 3), (130, 4)] {
            let mut bm = BitMatrix::new(w, h, 1);

            let mut seed = 0x9e3779b9u64;
            for y in 0..h {
                for x in 0..w {
                    seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
                    bm.put(x, y, (seed >> 33) & 1);
                }
            }

            for y in 0..h {
                for x in 0..w {
                    assert_eq!(
                        bm.get_bit(x, y),
                        bm.get(x, y) != 0,
                        "get_bit disagrees with get at ({x}, {y}) w={w} h={h}"
                    );
                }
            }
        }
    }

    #[test]
    fn test_get_bit_reads_exact_cell() {
        let mut bm = BitMatrix::new(10, 7, 1);
        bm.put(6, 2, 1);
        assert!(bm.get_bit(6, 2), "the exact cell that was set must read back");
        assert!(!bm.get_bit(2, 6), "mirror cell must be unaffected");
        assert!(!bm.get_bit(5, 2), "row neighbours must be unaffected");
        assert!(!bm.get_bit(7, 2));
    }

    #[test]
    fn test_get_bit_across_word_boundary() {
        let mut bm = BitMatrix::new(200, 2, 1);
        bm.put(63, 0, 1);
        bm.put(64, 0, 1);
        assert!(bm.get_bit(63, 0), "last bit of word 0");
        assert!(bm.get_bit(64, 0), "first bit of word 1");
        assert!(!bm.get_bit(62, 0));
        assert!(!bm.get_bit(65, 0));
        // They are genuinely distinct cells in distinct words.
        bm.put(63, 0, 0);
        assert!(!bm.get_bit(63, 0));
        assert!(bm.get_bit(64, 0));
    }

    #[test]
    fn test_get_bit_last_cell() {
        let (w, h) = (10, 7);
        let mut bm = BitMatrix::new(w, h, 1);
        assert!(!bm.get_bit(w - 1, h - 1), "a fresh matrix reads zero");
        bm.put(w - 1, h - 1, 1);
        assert!(bm.get_bit(w - 1, h - 1));
    }

    #[test]
    #[cfg(debug_assertions)]
    #[should_panic]
    fn test_get_bit_x_out_of_bounds() {
        let bm = BitMatrix::new(10, 7, 1);
        bm.get_bit(10, 0);
    }

    #[test]
    #[cfg(debug_assertions)]
    #[should_panic]
    fn test_get_bit_y_out_of_bounds() {
        let bm = BitMatrix::new(10, 7, 1);
        bm.get_bit(0, 7);
    }

    #[test]
    #[cfg(debug_assertions)]
    #[should_panic]
    fn test_get_bit_rejects_multibit_matrix() {
        let bm = BitMatrix::new(10, 7, 4);
        bm.get_bit(0, 0);
    }

    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "Bit size should be 1")]
    fn test_run_rejects_multibit_matrix() {
        let bm = BitMatrix::new(10, 7, 4);
        bm.run(0, 0);
    }

    #[test]
    fn test_run_stops_at_flip_and_row_edge() {
        let (w, h) = (8u32, 2u32);
        let mut bm = BitMatrix::new(w, h, 1);
        // Row 0: 0 0 1 1 1 0 1 1. Row 1 is all ones, so a run overrunning row 0 would keep going.
        for (x, bit) in [0u64, 0, 1, 1, 1, 0, 1, 1].into_iter().enumerate() {
            bm.put(x as u32, 0, bit);
        }
        for x in 0..w {
            bm.put(x, 1, 1);
        }

        assert_eq!(bm.run(0, 0), (false, 2), "two zeroes, then a flip");
        assert_eq!(bm.run(1, 0), (false, 1), "one zero left of the flip");
        assert_eq!(bm.run(2, 0), (true, 3), "three ones, then a flip");
        assert_eq!(bm.run(4, 0), (true, 1));
        assert_eq!(bm.run(5, 0), (false, 1));
        assert_eq!(bm.run(6, 0), (true, 2), "clamped at the row edge, not run into row 1");
        assert_eq!(bm.run(w - 1, 0), (true, 1), "last element of the row");
    }

    // `push_bits` fills the matrix as one sequential bitstream, LSB first: the nth bit pushed
    // lands at flat element n, which for `elem_bits == 1` is the element at (n % w, n / w).
    // Callers must push bits whose value fits in `n`, into a matrix nothing has `put` into.

    #[test]
    fn test_push_bits_one_at_a_time_matches_put() {
        let (w, h) = (13u32, 5u32);
        let pattern = |x: u32, y: u32| ((x * 7 + y * 3) % 5 == 0) as u64;

        let mut pushed = BitMatrix::new(w, h, 1);
        for y in 0..h {
            for x in 0..w {
                pushed.push_bits(pattern(x, y), 1);
            }
        }

        let mut put = BitMatrix::new(w, h, 1);
        for y in 0..h {
            for x in 0..w {
                put.put(x, y, pattern(x, y));
            }
        }

        assert_eq!(pushed.data(), put.data(), "raster-order pushes must equal the same puts");
    }

    #[test]
    fn test_push_bits_word_aligned_chunks() {
        let mut bm = BitMatrix::new(128, 1, 1);
        bm.push_bits(0xDEAD_BEEF_0123_4567, 64);
        bm.push_bits(0x0FED_CBA9_8765_4321, 64);
        assert_eq!(bm.data(), [0xDEAD_BEEF_0123_4567, 0x0FED_CBA9_8765_4321].as_slice());
    }

    #[test]
    fn test_push_bits_straddles_word_boundary() {
        let lo = 0x0000_00AB_CDEF_1234 & ((1u64 << 40) - 1);
        let hi = 0x0000_0056_789A_BCDE & ((1u64 << 40) - 1);

        let mut bm = BitMatrix::new(80, 1, 1);
        bm.push_bits(lo, 40);
        bm.push_bits(hi, 40);

        // Word 0 takes `lo` plus the low 24 bits of `hi`; word 1 takes `hi`'s remaining 16.
        assert_eq!(bm.data()[0], lo | (hi << 40), "low word");
        assert_eq!(bm.data()[1], hi >> 24, "carry into the next word");
    }

    #[test]
    fn test_push_bits_matches_naive_stream() {
        // Mixed chunk lengths, including ones that straddle words, checked bit for bit.
        for &(w, h) in &[(1u32, 1u32), (7, 3), (63, 2), (64, 2), (65, 2), (100, 4), (128, 3)] {
            let total = (w * h) as usize;
            let mut seed = 0x9e37_79b9_7f4a_7c15u64;
            let mut rng = || {
                seed ^= seed << 13;
                seed ^= seed >> 7;
                seed ^= seed << 17;
                seed
            };

            let mut bm = BitMatrix::new(w, h, 1);
            let mut expect: Vec<bool> = Vec::with_capacity(total);

            while expect.len() < total {
                let n = ((rng() % 64) as usize + 1).min(total - expect.len());
                let mask = if n == 64 { u64::MAX } else { (1u64 << n) - 1 };
                let bits = rng() & mask;
                bm.push_bits(bits, n);
                for i in 0..n {
                    expect.push((bits >> i) & 1 == 1);
                }
            }

            for (i, &want) in expect.iter().enumerate() {
                let (x, y) = (i as u32 % w, i as u32 / w);
                assert_eq!(bm.get_bit(x, y), want, "bit {i} at ({x}, {y}) in {w}x{h}");
            }
        }
    }

    #[test]
    fn test_push_bits_row_major_fill_matches_put() {
        // The shape `BinaryImage::prepare` uses: per row, whole 64-bit chunks then a short tail.
        // Widths either side of a word boundary put the tail in every alignment.
        for &w in &[1u32, 63, 64, 65, 100, 127, 128, 200] {
            let h = 5u32;
            let pattern = |x: u32, y: u32| ((x ^ y).count_ones() % 2) as u64;

            let mut pushed = BitMatrix::new(w, h, 1);
            for y in 0..h {
                let mut x = 0;
                while x < w {
                    let n = std::cmp::min(64, w - x);
                    let mut word = 0u64;
                    for i in 0..n {
                        word |= pattern(x + i, y) << i;
                    }
                    pushed.push_bits(word, n as usize);
                    x += n;
                }
            }

            let mut put = BitMatrix::new(w, h, 1);
            for y in 0..h {
                for x in 0..w {
                    put.put(x, y, pattern(x, y));
                }
            }

            assert_eq!(pushed.data(), put.data(), "w={w}");
        }
    }

    #[test]
    fn test_push_bits_fills_exact_capacity() {
        // 70 bits: the last word is partial, and the padding above it must stay zero.
        let (w, h) = (10u32, 7u32);
        let mut bm = BitMatrix::new(w, h, 1);
        for _ in 0..(w * h) {
            bm.push_bits(1, 1);
        }

        for y in 0..h {
            for x in 0..w {
                assert!(bm.get_bit(x, y), "({x}, {y}) should be set");
            }
        }
        assert_eq!(bm.data().len(), 2);
        assert_eq!(bm.data()[0], u64::MAX);
        assert_eq!(bm.data()[1], (1u64 << 6) - 1, "only the 6 live bits of the last word");
    }

    #[test]
    fn test_push_bits_zero_length_is_a_noop() {
        let mut bm = BitMatrix::new(64, 2, 1);
        bm.push_bits(0b1011, 4);
        let before = bm.data().to_vec();

        bm.push_bits(0, 0);
        assert_eq!(bm.data(), before.as_slice(), "a zero-length push must change nothing");

        bm.push_bits(1, 1);
        assert!(bm.get_bit(4, 0), "the next push must still land at bit 4");
    }

    #[test]
    // The guard is a `debug_assert!`, so release builds compile it out and never panic.
    #[cfg(debug_assertions)]
    #[should_panic(expected = "Bit matrix capacity overflow")]
    fn test_push_bits_past_capacity_panics() {
        let mut bm = BitMatrix::new(8, 1, 1);
        bm.push_bits(0xFF, 8);
        bm.push_bits(1, 1);
    }
}

// Global constants
//------------------------------------------------------------------------------

pub const MAX_PAYLOAD_SIZE: usize = 16384;
