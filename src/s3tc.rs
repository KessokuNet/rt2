//! Decodes S3TC/DXT blocks for GLES drivers that only expose the extension on desktop GL.

pub(crate) const DXT1_RGB: u32 = 0x83f0;
pub(crate) const DXT1_RGBA: u32 = 0x83f1;
pub(crate) const DXT3: u32 = 0x83f2;
pub(crate) const DXT5: u32 = 0x83f3;

/// Converts a full compressed level to tightly packed top-down RGBA8 pixels.
pub(crate) fn decode(format: u32, data: &[u8], width: u32, height: u32) -> Option<Vec<u8>> {
    let block_size = match format {
        DXT1_RGB | DXT1_RGBA => 8u32,
        DXT3 | DXT5 => 16u32,
        _ => return None,
    };
    let blocks_x = width.max(1).div_ceil(4);
    let blocks_y = height.max(1).div_ceil(4);
    let required =
        usize::try_from(blocks_x.checked_mul(blocks_y)?.checked_mul(block_size)?).ok()?;
    if data.len() < required {
        return None;
    }

    let mut decoded = vec![0; usize::try_from(width.checked_mul(height)?.checked_mul(4)?).ok()?];
    for block_y in 0..blocks_y {
        for block_x in 0..blocks_x {
            let offset = usize::try_from((block_y * blocks_x + block_x) * block_size).ok()?;
            let block = &data[offset..offset + usize::try_from(block_size).ok()?];
            let mut pixels = [0u32; 16];
            match format {
                DXT1_RGB => texture2ddecoder::decode_bc1_block(block, &mut pixels),
                DXT1_RGBA => texture2ddecoder::decode_bc1a_block(block, &mut pixels),
                DXT3 => texture2ddecoder::decode_bc2_block(block, &mut pixels),
                DXT5 => texture2ddecoder::decode_bc3_block(block, &mut pixels),
                _ => unreachable!(),
            }
            for y in 0..4 {
                for x in 0..4 {
                    let (px, py) = (block_x * 4 + x, block_y * 4 + y);
                    if px < width && py < height {
                        let [b, g, r, a] = pixels[(y * 4 + x) as usize].to_le_bytes();
                        let destination = usize::try_from((py * width + px) * 4).ok()?;
                        decoded[destination..destination + 4].copy_from_slice(&[r, g, b, a]);
                    }
                }
            }
        }
    }
    Some(decoded)
}

#[cfg(test)]
mod tests {
    use super::{DXT1_RGBA, DXT5, decode};

    #[test]
    fn dxt1_honors_one_bit_transparency_and_pads_tiny_mipmaps() {
        let mut block = [0u8; 8];
        block[..2].copy_from_slice(&0x001fu16.to_le_bytes());
        block[2..4].copy_from_slice(&0xf800u16.to_le_bytes());
        block[4..8].copy_from_slice(&0xffff_ffffu32.to_le_bytes());
        let rgba = decode(DXT1_RGBA, &block, 1, 1).unwrap();
        assert_eq!(&rgba, &[0, 0, 0, 0]);
    }

    #[test]
    fn dxt5_expands_explicit_alpha_endpoints() {
        let mut block = [0u8; 16];
        block[8..10].copy_from_slice(&0xffffu16.to_le_bytes());
        block[10..12].copy_from_slice(&0xffffu16.to_le_bytes());
        let rgba = decode(DXT5, &block, 1, 1).unwrap();
        assert_eq!(&rgba, &[255, 255, 255, 0]);
    }
}
