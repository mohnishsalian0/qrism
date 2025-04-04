#[derive(Debug, Clone)]
struct BitStream {
    data: [u8; MAX_PAYLOAD_SIZE],
    // Bit length
    len: usize,
    // Max bit capacity
    capacity: usize,
}

// EncodedBlob methods for encoding
//------------------------------------------------------------------------------

impl BitStream {
    pub fn new(capacity: usize) -> Self {
        Self { data: [0; MAX_PAYLOAD_SIZE], len: 0, capacity }
    }

    pub fn len(&self) -> usize {
        self.len
    }

    fn push_bits(&mut self, bits: u8, size: usize) {
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
}

// Global constants
//------------------------------------------------------------------------------

static MAX_PAYLOAD_SIZE: usize = 16384;
