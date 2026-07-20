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

#[derive(Debug, Clone)]
pub struct BitMatrix {
    data: Vec<u64>,
    w: u32,
    h: u32,
}

impl BitMatrix {
    pub fn new(w: u32, h: u32) -> Self {
        let cap = ((w * h + 63) >> 6) as usize;
        Self { data: vec![0u64; cap], w, h }
    }

    pub fn width(&self) -> u32 {
        self.w
    }

    pub fn height(&self) -> u32 {
        self.h
    }

    pub fn data(&self) -> &[u64] {
        &self.data
    }
}

// Put bits for bit matrix
//------------------------------------------------------------------------------

impl BitMatrix {
    pub fn get(&self, x: u32, y: u32) -> bool {
        debug_assert!(x < self.w, "X coordinate is out of bounds: Width {}, X {}", self.w, x);
        debug_assert!(y < self.h, "Y coordinate is out of bounds: Height {}, Y {}", self.h, y);

        let flat_pos = y * self.w + x;
        let idx = (flat_pos >> 6) as usize;
        let off = flat_pos & 63;

        debug_assert!(
            idx < self.data.len(),
            "Out of bit matrix bounds: Len {}, Index {}",
            self.data.len(),
            idx
        );

        ((self.data[idx] >> off) & 1) == 1
    }

    pub fn get_bits(&self, x: u32, y: u32, size: u32) -> u64 {
        debug_assert!((1..=64).contains(&size), "size must be 1..=64: Size {size}");
        debug_assert!(x < self.w, "X coordinate is out of bounds: Width {}, X {}", self.w, x);
        debug_assert!(y < self.h, "Y coordinate is out of bounds: Height {}, Y {}", self.h, y);

        let flat_pos = y * self.w + x;
        let idx = (flat_pos >> 6) as usize;
        let off = flat_pos & 63;

        debug_assert!(
            idx < self.data.len(),
            "Out of bit matrix bounds: Len {}, Index {}",
            self.data.len(),
            idx
        );

        // Bits that come from word at `idx`; the rest comes from `idx + 1`.
        let in_first = (64 - off).min(size);
        let first_mask = if in_first == 64 { u64::MAX } else { (1u64 << in_first) - 1 };
        let mut bits = (self.data[idx] & (first_mask << off)) >> off;

        let remaining = size - in_first;
        if remaining > 0 {
            debug_assert!(
                idx + 1 < self.data.len(),
                "Bit matrix capacity overflow when fetching bits"
            );
            let carry_mask = (1u64 << remaining) - 1;
            bits |= (self.data[idx + 1] & carry_mask) << in_first;
        }

        bits
    }

    pub fn put(&mut self, x: u32, y: u32, bit: bool) {
        debug_assert!(x < self.w, "X coordinate is out of bounds: Width {}, X {}", self.w, x);
        debug_assert!(y < self.h, "Y coordinate is out of bounds: Height {}, Y {}", self.h, y);

        let flat_pos = y * self.w + x;
        let idx = (flat_pos >> 6) as usize;
        let off = flat_pos & 63;

        debug_assert!(
            idx < self.data.len(),
            "Out of bit matrix bounds: Len {}, Index {}",
            self.data.len(),
            idx
        );

        if bit {
            self.data[idx] |= 1u64 << off;
        } else {
            self.data[idx] &= !(1u64 << off);
        }
    }

    pub fn put_bits(&mut self, x: u32, y: u32, bits: u64, size: u32) {
        debug_assert!((1..=64).contains(&size), "size must be 1..=64: Size {size}");
        debug_assert!(x < self.w, "X coordinate is out of bounds: Width {}, X {}", self.w, x);
        debug_assert!(y < self.h, "Y coordinate is out of bounds: Height {}, Y {}", self.h, y);

        let flat_pos = y * self.w + x;
        let idx = (flat_pos >> 6) as usize;
        let off = flat_pos & 63;

        debug_assert!(
            idx < self.data.len(),
            "Out of bit matrix bounds: Len {}, Index {}",
            self.data.len(),
            idx
        );

        let payload = if size == 64 { bits } else { bits & ((1u64 << size) - 1) };

        // Bits that land in word `idx`; the rest spill into `idx + 1`.
        let in_first = (64 - off).min(size);
        let first_mask = if in_first == 64 { u64::MAX } else { (1u64 << in_first) - 1 };
        self.data[idx] = (self.data[idx] & !(first_mask << off)) | ((payload & first_mask) << off);

        let remaining = size - in_first;
        if remaining > 0 {
            debug_assert!(idx + 1 < self.data.len(), "Bit matrix capacity overflow");
            let carry_mask = (1u64 << remaining) - 1;
            self.data[idx + 1] =
                (self.data[idx + 1] & !carry_mask) | ((payload >> in_first) & carry_mask);
        }
    }
}

#[cfg(test)]
mod bit_matrix_tests {

    use super::BitMatrix;

    #[test]
    fn test_new_is_all_zero() {
        let (w, h) = (10, 7);
        let bm = BitMatrix::new(w, h);
        assert_eq!(bm.width(), w);
        assert_eq!(bm.height(), h);
        // ceil(70 / 64) = 2 words
        assert_eq!(bm.data().len(), 2);
        for y in 0..h {
            for x in 0..w {
                assert!(!bm.get(x, y), "fresh matrix should be all false at ({x}, {y})");
            }
        }
    }

    #[test]
    fn test_put_get_roundtrip() {
        let mut bm = BitMatrix::new(10, 7);
        bm.put(3, 2, true);
        assert!(bm.get(3, 2));
        // Clearing a set bit
        bm.put(3, 2, false);
        assert!(!bm.get(3, 2));
    }

    // A transposed get/put (x*w+y instead of y*w+x) passes on the diagonal but fails
    // off it. Use a non-square matrix and an asymmetric cell to catch that.
    #[test]
    fn test_not_transposed() {
        let mut bm = BitMatrix::new(10, 7);
        bm.put(6, 2, true);
        assert!(bm.get(6, 2), "the exact cell that was set must read back true");
        // The mirror cell is a different location and must stay false.
        assert!(!bm.get(2, 6), "mirror cell must be unaffected");
    }

    #[test]
    fn test_bits_are_independent() {
        let mut bm = BitMatrix::new(10, 7);
        bm.put(4, 3, true);
        // Every 4-neighbour stays false.
        assert!(!bm.get(3, 3));
        assert!(!bm.get(5, 3));
        assert!(!bm.get(4, 2));
        assert!(!bm.get(4, 4));
        // Clearing a neighbour doesn't disturb the set bit.
        bm.put(5, 3, false);
        assert!(bm.get(4, 3));
    }

    // Bits either side of a 64-bit word boundary must land in different words.
    #[test]
    fn test_word_boundary() {
        let mut bm = BitMatrix::new(100, 2);
        // Row 0: flat 63 -> word 0 bit 63, flat 64 -> word 1 bit 0.
        bm.put(63, 0, true);
        bm.put(64, 0, true);
        assert!(bm.get(63, 0));
        assert!(bm.get(64, 0));
        assert_eq!(bm.data()[0].count_ones(), 1);
        assert_eq!(bm.data()[1].count_ones(), 1);
        // They are genuinely distinct cells.
        bm.put(63, 0, false);
        assert!(!bm.get(63, 0));
        assert!(bm.get(64, 0));
    }

    // Fill a full non-square matrix with a checkerboard and read it all back.
    #[test]
    fn test_checkerboard_full_coverage() {
        let (w, h) = (13, 9);
        let mut bm = BitMatrix::new(w, h);
        for y in 0..h {
            for x in 0..w {
                if (x + y) % 2 == 0 {
                    bm.put(x, y, true);
                }
            }
        }
        for y in 0..h {
            for x in 0..w {
                assert_eq!(bm.get(x, y), (x + y) % 2 == 0, "mismatch at ({x}, {y})");
            }
        }
    }

    #[test]
    fn test_last_cell() {
        let (w, h) = (10, 7);
        let mut bm = BitMatrix::new(w, h);
        bm.put(w - 1, h - 1, true);
        assert!(bm.get(w - 1, h - 1));
        assert_eq!(bm.data().iter().map(|word| word.count_ones()).sum::<u32>(), 1);
    }

    #[test]
    #[should_panic]
    fn test_get_x_out_of_bounds() {
        let bm = BitMatrix::new(10, 7);
        bm.get(10, 0);
    }

    #[test]
    #[should_panic]
    fn test_get_y_out_of_bounds() {
        let bm = BitMatrix::new(10, 7);
        bm.get(0, 7);
    }

    #[test]
    #[should_panic]
    fn test_put_x_out_of_bounds() {
        let mut bm = BitMatrix::new(10, 7);
        bm.put(10, 0, true);
    }

    // ---- put_bits ----

    // Word-aligned write (off == 0) — the case that used to make `bits >> 64` overflow.
    #[test]
    fn test_put_bits_word_aligned_full_word() {
        let mut bm = BitMatrix::new(200, 1);
        let pattern = 0xF0F0_F0F0_0F0F_0F0Fu64;
        bm.put_bits(0, 0, pattern, 64);
        assert_eq!(bm.get_bits(0, 0, 64), pattern);
        assert!(!bm.get(64, 0), "nothing should spill past the run");
    }

    // Partial write fully inside one word.
    #[test]
    fn test_put_bits_within_word() {
        let mut bm = BitMatrix::new(200, 1);
        bm.put_bits(4, 0, 0b1011, 4);
        assert_eq!(bm.get_bits(4, 0, 4), 0b1011);
        assert!(!bm.get(3, 0));
        assert!(!bm.get(8, 0));
    }

    // Bits above `size` in the payload must be ignored, not written.
    #[test]
    fn test_put_bits_ignores_bits_above_size() {
        let mut bm = BitMatrix::new(200, 1);
        bm.put_bits(10, 0, 0b1111_1101, 3); // only low 3 bits (0b101) count
        assert_eq!(bm.get_bits(10, 0, 3), 0b101);
        assert!(!bm.get(13, 0), "the 4th payload bit must not be written");
    }

    // Run straddling the 64-bit word boundary: flat 40..104 spans word 0 and word 1.
    #[test]
    fn test_put_bits_crosses_word_boundary() {
        let mut bm = BitMatrix::new(200, 1);
        let pattern = 0xABCD_1234_5678_9EF1u64;
        bm.put_bits(40, 0, pattern, 64);
        assert_eq!(bm.get_bits(40, 0, 64), pattern);
        assert!(!bm.get(39, 0));
        assert!(!bm.get(104, 0));
    }

    // Writing preserves the surrounding cells on both sides of the run.
    #[test]
    fn test_put_bits_preserves_neighbors() {
        let mut bm = BitMatrix::new(200, 1);
        bm.put(1, 0, true); // below the run
        bm.put(70, 0, true); // above the run
        bm.put_bits(2, 0, 0x0000_0000_FFFF_FFFF, 64); // covers flat 2..66
        assert!(bm.get(1, 0), "cell below the run must be preserved");
        assert!(bm.get(70, 0), "cell above the run must be preserved");
        assert_eq!(bm.get_bits(2, 0, 64), 0x0000_0000_FFFF_FFFF);
    }

    // A zero payload clears the run (and only the run) rather than being a no-op.
    #[test]
    fn test_put_bits_zero_clears_run() {
        let mut bm = BitMatrix::new(200, 1);
        bm.put(3, 0, true); // guard bit below
        bm.put_bits(4, 0, u64::MAX, 60); // set flat 4..64
        bm.put_bits(4, 0, 0, 60); // clear the same run
        assert_eq!(bm.get_bits(4, 0, 60), 0);
        assert!(bm.get(3, 0), "cell outside the run must remain set");
    }

    // Overwriting a run replaces it wholesale — no stale set bits remain.
    #[test]
    fn test_put_bits_overwrite_replaces() {
        let mut bm = BitMatrix::new(200, 1);
        bm.put_bits(5, 0, u64::MAX, 40);
        bm.put_bits(5, 0, 0x0000_00AA, 40);
        assert_eq!(bm.get_bits(5, 0, 40), 0x0000_00AA);
    }

    // size == 1 must behave exactly like `put`.
    #[test]
    fn test_put_bits_size_one_matches_put() {
        let mut a = BitMatrix::new(200, 3);
        let mut b = BitMatrix::new(200, 3);
        for &(x, y, bit) in &[(0u32, 0u32, 1u64), (63, 1, 1), (64, 2, 1), (199, 2, 0)] {
            a.put_bits(x, y, bit, 1);
            b.put(x, y, bit == 1);
        }
        assert_eq!(a.data(), b.data());
    }

    // A run that ends exactly on the word boundary must not spill into the next word.
    #[test]
    fn test_put_bits_ends_on_word_boundary() {
        let mut bm = BitMatrix::new(200, 1);
        bm.put(64, 0, true); // first cell of word 1
        bm.put_bits(40, 0, u64::MAX, 24); // fills flat 40..64 exactly (word 0 only)
        assert_eq!(bm.get_bits(40, 0, 24), 0x00FF_FFFF);
        assert!(bm.get(64, 0), "the word-1 cell must be untouched");
    }

    #[test]
    #[should_panic]
    fn test_put_bits_size_zero_panics() {
        let mut bm = BitMatrix::new(200, 1);
        bm.put_bits(0, 0, 0, 0);
    }

    // ---- get_bits ----

    // get_bits reads back exactly what put_bits wrote (both aligned and crossing).
    #[test]
    fn test_get_bits_reads_run() {
        let mut bm = BitMatrix::new(200, 1);
        bm.put_bits(0, 0, 0xDEAD_BEEF_CAFE_1234, 64); // aligned
        bm.put_bits(70, 0, 0b1101_0110, 8); // within word 1
        assert_eq!(bm.get_bits(0, 0, 64), 0xDEAD_BEEF_CAFE_1234);
        assert_eq!(bm.get_bits(70, 0, 8), 0b1101_0110);
    }

    // The result is zero-extended above `size`: cells past the run don't leak in.
    #[test]
    fn test_get_bits_zero_extended_above_size() {
        let mut bm = BitMatrix::new(200, 1);
        bm.put(3, 0, true); // set a cell just past a 3-bit run at (0,0)
        bm.put(0, 0, true);
        assert_eq!(bm.get_bits(0, 0, 3), 0b001, "cell 3 must not appear in a 3-bit read");
    }

    // Reading across the word boundary reassembles low+high halves correctly.
    #[test]
    fn test_get_bits_crosses_word_boundary() {
        let mut bm = BitMatrix::new(200, 1);
        let pattern = 0x0123_4567_89AB_CDEFu64;
        bm.put_bits(50, 0, pattern, 64); // flat 50..114 spans words 0 and 1
        assert_eq!(bm.get_bits(50, 0, 64), pattern);
    }

    // get_bits of a single bit equals get.
    #[test]
    fn test_get_bits_size_one_matches_get() {
        let mut bm = BitMatrix::new(200, 2);
        bm.put(63, 0, true);
        bm.put(64, 1, true);
        assert_eq!(bm.get_bits(63, 0, 1), bm.get(63, 0) as u64);
        assert_eq!(bm.get_bits(64, 1, 1), bm.get(64, 1) as u64);
        assert_eq!(bm.get_bits(0, 0, 1), 0);
    }

    #[test]
    #[should_panic]
    fn test_get_bits_size_zero_panics() {
        let bm = BitMatrix::new(200, 1);
        bm.get_bits(0, 0, 0);
    }

    // ---- round trip ----

    // put_bits then get_bits must be identity for every offset and size, including
    // runs that straddle the word boundary. Neighbouring cells stay untouched.
    #[test]
    fn test_put_get_bits_round_trip_sweep() {
        // Deterministic pseudo-random payloads via a small LCG.
        let mut seed = 0x9E37_79B9_7F4A_7C15u64;
        let mut next = || {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            seed
        };

        // Wide matrix so any (start, size<=64) run fits within a single row.
        let w = 300u32;
        for start in 0..130u32 {
            for size in 1..=64u32 {
                let mut bm = BitMatrix::new(w, 1);
                // Guard bits immediately on either side of the run.
                if start > 0 {
                    bm.put(start - 1, 0, true);
                }
                bm.put(start + size, 0, true);

                let expected = if size == 64 { next() } else { next() & ((1u64 << size) - 1) };
                bm.put_bits(start, 0, expected, size);

                assert_eq!(
                    bm.get_bits(start, 0, size),
                    expected,
                    "round trip failed at start={start}, size={size}"
                );
                if start > 0 {
                    assert!(
                        bm.get(start - 1, 0),
                        "low guard clobbered: start={start}, size={size}"
                    );
                }
                assert!(
                    bm.get(start + size, 0),
                    "high guard clobbered: start={start}, size={size}"
                );
            }
        }
    }
}

// Global constants
//------------------------------------------------------------------------------

pub const MAX_PAYLOAD_SIZE: usize = 16384;
