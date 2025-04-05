use core::panic;
use std::{cmp::min, fmt::Display, mem};

use num_traits::PrimInt;

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

// EncodedBlob methods for encoding
//------------------------------------------------------------------------------

impl BitStream {
    pub fn new(capacity: usize) -> Self {
        Self { data: [0; MAX_PAYLOAD_SIZE], len: 0, capacity, cursor: 0 }
    }

    pub fn with(data: [u8; MAX_PAYLOAD_SIZE], len: usize) -> Self {
        Self { data, len, capacity: len, cursor: 0 }
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn capacity(&self) -> usize {
        self.capacity
    }

    pub fn data(&self) -> &[u8] {
        &self.data
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
            0 => return,
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
                self.push_bits(((bits << 8) >> 8).to_u8().unwrap(), 8);
            }
            _ => panic!("Bits from only u8 and u16 can be pushed"),
        }
    }

    pub fn push_bits_test(&mut self, bits: u8, size: usize) {
        debug_assert!(
            size >= (8 - bits.leading_zeros()) as usize,
            "Bit count shouldn't exceed bit length: Length {size}, Bits {bits}"
        );
        debug_assert!(
            self.len + size <= self.capacity,
            "Insufficient capacity: Capacity {}, Size {}",
            self.capacity,
            self.len + size
        );

        if size == 0 {
            return;
        }

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

    pub fn take_byte(&mut self) -> Option<u8> {
        if self.cursor == self.len {
            return None;
        }

        let size = min(8, self.len - self.cursor);
        let offset = self.cursor & 7;
        let pos = self.cursor >> 3;

        let mut res = (self.data[pos] as u16) << 8;
        if offset + size > 8 {
            res |= self.data[pos + 1] as u16;
        }
        res >>= 16 - offset - size;
        res &= (1 << size) - 1;

        self.cursor += size;
        Some(res as u8)
    }
}

#[cfg(test)]
mod bit_stream_tests {

    use super::BitStream;

    #[test]
    fn test_len() {
        let bit_capacity = 152;
        let mut eb = BitStream::new(bit_capacity);
        assert_eq!(eb.len(), 0);
        eb.push_bits(0, 0);
        assert_eq!(eb.len(), 0);
        eb.push_bits(0b1000, 4);
        assert_eq!(eb.len(), 4);
        eb.push_bits(0b1000, 8);
        assert_eq!(eb.len(), 12);
        eb.push_bits(0b1000, 4);
        assert_eq!(eb.len(), 16);
        eb.push_bits(0b1111111, 7);
        assert_eq!(eb.len(), 23);
    }

    #[test]
    fn test_push() {
        let mut eb = BitStream::new(2);
        eb.push(false);
        assert_eq!(eb.data[..1], vec![0b00000000]);
        eb.push(true);
        assert_eq!(eb.data[..1], vec![0b01000000]);
    }

    #[test]
    fn test_push_bits() {
        let bit_capacity = 152;
        let exp_vec = [0b11010010, 0b00110100, 0b10001101, 0b00100011, 0b01001000, 0b11010010];
        let mut cursor = 0;
        let mut eb = BitStream::new(bit_capacity);
        for n in [0, 1, 2, 3, 4, 5, 6, 7, 8, 4, 8] {
            let offset = cursor & 7;
            let pos = cursor >> 3;
            let bits = if n == 0 {
                0
            } else if n + offset <= 8 {
                (exp_vec[pos] << offset) >> (8 - n)
            } else {
                let bits = (exp_vec[pos] << offset) >> (8 - n);
                bits | (exp_vec[pos + 1] >> (16 - offset - n))
            };
            cursor += n;
            eb.push_bits(bits, n);
            let eb_offset = eb.len() & 7;
            let len = eb.len() >> 3;
            assert_eq!(eb.data[..len], exp_vec[..len], "n {n}");
            if eb_offset > 0 {
                assert_eq!(eb.data[len] >> (8 - eb_offset), exp_vec[len] >> (8 - eb_offset));
            }
        }
    }

    #[test]
    #[should_panic]
    fn test_push_bits_capacity_overflow() {
        let bit_capacity = 152;
        let capacity = (bit_capacity + 7) >> 3;
        let mut eb = BitStream::new(bit_capacity);
        for _ in 0..capacity {
            eb.push_bits(8, 0b1);
        }
        eb.push_bits(1, 0b1)
    }

    // #[test]
    // fn test_len() {
    //     let version = Version::Normal(1);
    //     let ec_level = ECLevel::L;
    //     let palette = Palette::Mono;
    //     let bit_capacity = version.data_bit_capacity(ec_level, palette);
    //     let mut eb = EncodedBlob::new(version, bit_capacity);
    //     assert_eq!(eb.bit_len(), 0);
    //     eb.push_bits(0, 0);
    //     assert_eq!(eb.bit_len(), 0);
    //     eb.push_bits(4, 0b1000);
    //     assert_eq!(eb.bit_len(), 4);
    //     eb.push_bits(8, 0b1000);
    //     assert_eq!(eb.bit_len(), 12);
    //     eb.push_bits(4, 0b1000);
    //     assert_eq!(eb.bit_len(), 16);
    //     eb.push_bits(7, 0b1111111);
    //     assert_eq!(eb.bit_len(), 23);
    // }
    //
    // #[test]
    // fn test_push_bits() {
    //     let version = Version::Normal(1);
    //     let ec_level = ECLevel::L;
    //     let palette = Palette::Mono;
    //     let bit_capacity = version.data_bit_capacity(ec_level, palette);
    //     let mut eb = EncodedBlob::new(version, bit_capacity);
    //     eb.push_bits(0, 0);
    //     assert_eq!(eb.data, vec![]);
    //     eb.push_bits(4, 0b1101);
    //     assert_eq!(eb.data, vec![0b11010000]);
    //     eb.push_bits(4, 0b0010);
    //     assert_eq!(eb.data, vec![0b11010010]);
    //     eb.push_bits(8, 0b00110100);
    //     assert_eq!(eb.data, vec![0b11010010, 0b00110100]);
    //     eb.push_bits(9, 0b100011010);
    //     assert_eq!(eb.data, vec![0b11010010, 0b00110100, 0b10001101, 0b00000000]);
    //     eb.push_bits(7, 0b0100011);
    //     assert_eq!(eb.data, vec![0b11010010, 0b00110100, 0b10001101, 0b00100011]);
    //     eb.push_bits(16, 0b01001000_11010010);
    //     assert_eq!(
    //         eb.data,
    //         vec![0b11010010, 0b00110100, 0b10001101, 0b00100011, 0b01001000, 0b11010010]
    //     );
    //     eb.push_bits(1, 0b0);
    //     assert_eq!(
    //         eb.data,
    //         vec![
    //             0b11010010, 0b00110100, 0b10001101, 0b00100011, 0b01001000, 0b11010010, 0b00000000
    //         ]
    //     );
    //     eb.push_bits(11, 0b01101001000);
    //     assert_eq!(
    //         eb.data,
    //         vec![
    //             0b11010010, 0b00110100, 0b10001101, 0b00100011, 0b01001000, 0b11010010, 0b00110100,
    //             0b10000000
    //         ]
    //     );
    //     eb.push_bits(14, 0b11010010001101);
    //     assert_eq!(
    //         eb.data,
    //         vec![
    //             0b11010010, 0b00110100, 0b10001101, 0b00100011, 0b01001000, 0b11010010, 0b00110100,
    //             0b10001101, 0b00100011, 0b01000000
    //         ]
    //     );
    //     eb.push_bits(16, 0b0010001101001000);
    //     assert_eq!(
    //         eb.data,
    //         vec![
    //             0b11010010, 0b00110100, 0b10001101, 0b00100011, 0b01001000, 0b11010010, 0b00110100,
    //             0b10001101, 0b00100011, 0b01001000, 0b11010010, 0b00000000
    //         ]
    //     );
    // }
}

// Global constants
//------------------------------------------------------------------------------

static MAX_PAYLOAD_SIZE: usize = 16384;
