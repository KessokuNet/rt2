//! The game's four-bit paletted texture, stored as an RGB565 palette followed by indices.

/// The game passes this format with a four-bit payload rather than eight-bit indices.
const PALETTED_RGB565: u32 = 0x8b94;
/// Sixteen RGB565 palette entries.
const PALETTE_SIZE: usize = 32;

fn rgb565(value: &[u8]) -> [u8; 4] {
    let value = u16::from_le_bytes(value.try_into().expect("palette entry is two bytes"));
    [
        ((value >> 11) * 255 / 31) as u8,
        (((value >> 5) & 63) * 255 / 63) as u8,
        ((value & 31) * 255 / 31) as u8,
        255,
    ]
}

pub(crate) fn decode(format: u32, data: &[u8], width: u32, height: u32) -> Option<Vec<u8>> {
    if format != PALETTED_RGB565 {
        return None;
    }
    let count = usize::try_from(width.checked_mul(height)?).ok()?;
    if data.len() != count.checked_div(2)?.checked_add(PALETTE_SIZE)? {
        return None;
    }
    let palette: Vec<_> = data[..PALETTE_SIZE]
        .as_chunks::<2>()
        .0
        .iter()
        .map(|value| rgb565(value))
        .collect();
    let pixels = data[PALETTE_SIZE..]
        .iter()
        .flat_map(|&byte| [byte & 15, byte >> 4])
        .take(count)
        .flat_map(|index| palette[index as usize]);
    Some(pixels.collect())
}

#[cfg(test)]
mod tests {
    use super::{PALETTED_RGB565, decode};

    #[test]
    fn decodes_low_nibble_before_high_nibble() {
        let mut data = vec![0u8; 32];
        data[22..24].copy_from_slice(&0x07e0u16.to_le_bytes());
        data.push(0x0b);
        let pixels = decode(PALETTED_RGB565, &data, 2, 1).unwrap();
        assert_eq!(&pixels, &[0, 255, 0, 255, 0, 0, 0, 255]);
    }
}
