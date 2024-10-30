use image::GrayImage;

pub struct QRReader();

impl QRReader {
    pub fn read(img: GrayImage) -> String {
        todo!()
    }

    // Chunks is the list of chunks' size and count
    fn deinterleave(data: &[u8], blocks: (usize, usize, usize, usize)) -> Vec<Vec<u8>> {
        let len = data.len();
        let (block1_size, block1_count, block2_size, block2_count) = blocks;

        let total_blocks = block1_count + block2_count;
        let partition = block1_size * total_blocks;
        let total_size = block1_size * block1_count + block2_size * block2_count;

        debug_assert!(len == total_size, "Data size doesn't match chunk total size: Data size {len}, Chunks total size {total_size}");

        let mut res = vec![Vec::with_capacity(block2_size); total_blocks];
        data[..partition]
            .chunks(total_blocks)
            .for_each(|ch| ch.iter().enumerate().for_each(|(i, v)| res[i].push(*v)));
        data[partition..]
            .chunks(block2_count)
            .for_each(|ch| ch.iter().enumerate().for_each(|(i, v)| res[block1_count + i].push(*v)));
        res
    }
}

#[cfg(test)]
mod reader_tests {
    use super::QRReader;

    #[test]
    fn test_deinterleave() {
        let data = vec![1, 4, 7, 2, 5, 8, 3, 6, 9, 0];
        let deinterleaved = QRReader::deinterleave(&data, (3, 2, 4, 1));
        let exp_deinterleaved = vec![vec![1, 2, 3], vec![4, 5, 6], vec![7, 8, 9, 0]];
        assert_eq!(deinterleaved, exp_deinterleaved);
    }
}
