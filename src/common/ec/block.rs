use super::MAX_BLOCK_SIZE;

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub(crate) struct Block {
    pub data: [u8; MAX_BLOCK_SIZE],
    // Block length
    pub len: usize,
    // Data length
    pub dlen: usize,
}

impl Block {
    pub fn new(raw: &[u8], len: usize) -> Self {
        let dlen = raw.len();
        let mut data = [0u8; MAX_BLOCK_SIZE];
        data[..dlen].copy_from_slice(raw);
        let mut block = Self { data, len, dlen };
        block.compute_ecc();
        block
    }

    pub fn with_encoded(encoded: &[u8], dlen: usize) -> Self {
        let len = encoded.len();
        let mut data = [0u8; MAX_BLOCK_SIZE];
        data[..len].copy_from_slice(encoded);
        Self { data, len, dlen }
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn ec_len(&self) -> usize {
        self.len - self.dlen
    }

    pub fn data_len(&self) -> usize {
        self.dlen
    }

    pub fn full(&self) -> &[u8] {
        &self.data[..self.len]
    }

    #[cfg(test)]
    pub fn full_mut(&mut self) -> &mut [u8] {
        &mut self.data[..self.len]
    }

    pub fn data(&self) -> &[u8] {
        &self.data[..self.dlen]
    }

    pub fn ecc(&self) -> &[u8] {
        &self.data[self.dlen..self.len]
    }
}
