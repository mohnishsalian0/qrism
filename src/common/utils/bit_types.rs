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
/// word — no element ever straddles a word boundary.
#[derive(Debug, Clone)]
pub struct BitMatrix {
    data: Vec<u64>,
    w: u32,
    h: u32,
    elem_bits: u32,
}

impl BitMatrix {
    pub fn new(w: u32, h: u32, elem_bits: u32) -> Self {
        debug_assert!((1..=64).contains(&elem_bits), "elem_bits must be 1..=64: {elem_bits}");
        debug_assert!(64 % elem_bits == 0, "elem_bits must be a factor of 64: {elem_bits}");
        let cap = ((w * h * elem_bits + 63) >> 6) as usize;
        Self { data: vec![0u64; cap], w, h, elem_bits }
    }

    pub fn width(&self) -> u32 {
        self.w
    }

    pub fn height(&self) -> u32 {
        self.h
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

        let flat_pos = (y * self.w + x) * self.elem_bits;
        let idx = (flat_pos >> 6) as usize;
        let off = flat_pos & 63;

        debug_assert!(
            idx < self.data.len(),
            "Out of bit matrix bounds: Len {}, Index {}",
            self.data.len(),
            idx
        );

        // `elem_bits` divides 64, so the element never crosses into the next word.
        let mask = if self.elem_bits == 64 { u64::MAX } else { (1u64 << self.elem_bits) - 1 };
        (self.data[idx] >> off) & mask
    }

    pub fn put(&mut self, x: u32, y: u32, bits: u64) {
        debug_assert!(x < self.w, "X coordinate is out of bounds: Width {}, X {}", self.w, x);
        debug_assert!(y < self.h, "Y coordinate is out of bounds: Height {}, Y {}", self.h, y);
        debug_assert!(
            self.elem_bits == 64 || bits >> self.elem_bits == 0,
            "bits {bits} do not fit in elem_bits {}",
            self.elem_bits
        );

        let flat_pos = (y * self.w + x) * self.elem_bits;
        let idx = (flat_pos >> 6) as usize;
        let off = flat_pos & 63;

        debug_assert!(
            idx < self.data.len(),
            "Out of bit matrix bounds: Len {}, Index {}",
            self.data.len(),
            idx
        );

        // `elem_bits` divides 64, so the element sits wholly within word `idx`.
        let mask = if self.elem_bits == 64 { u64::MAX } else { (1u64 << self.elem_bits) - 1 };
        self.data[idx] = (self.data[idx] & !(mask << off)) | (bits << off);
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

    // A 4-bit-element matrix packs 16 elements per word and sizes its buffer accordingly.
    #[test]
    fn test_new_multibit_capacity() {
        let bm = BitMatrix::new(10, 7, 4);
        assert_eq!(bm.elem_bits(), 4);
        // 10 * 7 * 4 = 280 bits -> ceil(280 / 64) = 5 words
        assert_eq!(bm.data().len(), 5);
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

    // Multi-bit elements round-trip their full value.
    #[test]
    fn test_put_get_roundtrip_multibit() {
        let mut bm = BitMatrix::new(10, 7, 4);
        bm.put(3, 2, 0b1011);
        assert_eq!(bm.get(3, 2), 0b1011);
        bm.put(3, 2, 0b0110);
        assert_eq!(bm.get(3, 2), 0b0110, "overwrite must replace the element wholesale");
    }

    // Writing an element wider than `elem_bits` violates the contract and is rejected.
    #[test]
    #[should_panic]
    fn test_put_oversized_element_panics() {
        let mut bm = BitMatrix::new(10, 1, 4);
        bm.put(0, 0, 0b1_0000); // needs 5 bits, only 4 allowed
    }

    // A transposed get/put (x*w+y instead of y*w+x) passes on the diagonal but fails
    // off it. Use a non-square matrix and an asymmetric cell to catch that.
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

    // Elements either side of a 64-bit word boundary must land in different words.
    // With elem_bits = 4, element 16 begins exactly at bit 64 (word 1, bit 0).
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

    // Fill a full non-square multi-bit matrix and read it all back.
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

    // elem_bits == 64: one element per word, mask is the whole word.
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

    // elem_bits must divide 64 evenly.
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

    // Exhaustive round trip for every factor-of-64 element width: each element preserves its value
    // and leaves its immediate neighbours untouched, including across the word boundary.
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
}

// Global constants
//------------------------------------------------------------------------------

pub const MAX_PAYLOAD_SIZE: usize = 16384;
