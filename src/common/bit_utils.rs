use core::panic;
use std::{fmt::Display, mem};

use num_traits::PrimInt;

// Bit stream
//------------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct BitStream {
    data: [u8; MAX_PAYLOAD_SIZE],
    // Bit length
    len: usize,
    // Max bit capacity
    capacity: usize,
    // Pointer to take bits
    cursor: usize,
}

impl BitStream {
    pub fn new(capacity: usize) -> Self {
        Self { data: [0; MAX_PAYLOAD_SIZE], len: 0, capacity, cursor: 0 }
    }

    pub fn from(inp: &[u8]) -> Self {
        let len = inp.len();
        let bit_len = len << 3;
        let mut data = [0; MAX_PAYLOAD_SIZE];
        data[..len].copy_from_slice(inp);
        Self { data, len: bit_len, capacity: bit_len, cursor: 0 }
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn capacity(&self) -> usize {
        self.capacity
    }

    pub fn data(&self) -> &[u8] {
        &self.data[..(self.len + 7) >> 3]
    }
}

// Push bits for bit stream
//------------------------------------------------------------------------------

impl BitStream {
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
                let offset = self.len & 7;
                let pos = self.len >> 3;

                if offset + size <= 8 {
                    self.data[pos] |= bits << (8 - size - offset);
                } else {
                    self.data[pos] |= bits >> (size + offset - 8);
                    self.data[pos + 1] = bits << (16 - size - offset);
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
            let offset = self.len & 7;
            let pos = self.len >> 3;
            self.data[pos] |= 0b10000000 >> offset;
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

        if self.cursor + n >= self.len {
            return None;
        }

        let offset = self.cursor & 7;
        let pos = self.cursor >> 3;

        let mut res = (self.data[pos] as u32) << 16;
        if offset + n > 8 {
            res |= (self.data[pos + 1] as u32) << 8;
        }
        if offset + n > 16 {
            res |= self.data[pos + 2] as u32;
        }
        res >>= 24 - offset - n;
        res &= (1 << n) - 1;

        self.cursor += n;
        Some(res as u16)
    }

    pub fn take(&mut self) -> Option<bool> {
        if self.cursor == self.len {
            return None;
        }

        let offset = self.cursor & 7;
        let pos = self.cursor >> 3;
        let bit = (self.data[pos] << offset) >> 7;

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
        self.take()
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

        let offset = pos & 7;
        let index = pos >> 3;

        self.data[index] &= !(0b10000000 >> offset);
        if bit {
            self.data[index] |= (0b10000000) >> offset;
        }
    }
}

// Global constants
//------------------------------------------------------------------------------

pub static MAX_PAYLOAD_SIZE: usize = 16384;
