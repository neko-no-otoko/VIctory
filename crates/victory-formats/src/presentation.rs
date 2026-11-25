//! Linear 8-bit color source layout accepted by the Metal presentation path.

/// Maximum packed BGRA8 drawable readback, independent of the source dimensions.
pub const MAX_CAPTURE_BYTES: usize = 16 * 1024 * 1024;

/// Accessible source bytes through the last pixel (no trailing row padding).
/// Bound the stride independently: a single row does not otherwise constrain it.
pub fn source_len(width: u32, height: u32, pitch: u32) -> Option<usize> {
    // Apple Silicon's 2D Metal textures have at most 16,384 texels per side.
    const MAX_SIDE: u32 = 16_384;
    if !(1..=MAX_SIDE).contains(&width) || !(1..=MAX_SIDE).contains(&height) {
        return None;
    }
    let row = width.checked_mul(4)?;
    if !(row..=MAX_SIDE * 4).contains(&pitch) {
        return None;
    }
    // For a single row the stride does not contribute to the byte count.
    // Keep the usual 16 KiB ceiling unless an actual wider row needs it.
    if height == 1 && pitch > (16 * 1024).max(row.next_multiple_of(256)) {
        return None;
    }
    let bytes = (height - 1).checked_mul(pitch)?.checked_add(row)?;
    if bytes <= 16 * 1024 * 1024
        || (width <= 3840 && height <= 2160 && pitch <= 16 * 1024 && bytes <= 32 * 1024 * 1024)
    {
        Some(bytes as usize)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounds_packed_and_padded_rows_independently_of_capture() {
        assert_eq!(source_len(3840, 2160, 15360), Some(33_177_600));
        assert_eq!(source_len(3840, 2160, 15376), Some(33_212_144));
        assert_eq!(source_len(3840, 2160, 16384), None);
        assert_eq!(source_len(1080, 2560, 4320), Some(11_059_200));
        assert_eq!(source_len(4096, 1024, 16384), Some(16_777_216));
        assert_eq!(source_len(8192, 512, 32768), Some(16_777_216));
        assert_eq!(source_len(16_384, 1, 65_536), Some(65_536));
        for (width, height, pitch) in [(0, 1, 4), (1, 0, 4), (16_385, 1, 65_540),
            (1, 16_385, 4), (1, 1, 16_385), (1, 2, 65_537), (1, 1, u32::MAX),
            (4096, 1025, 16384), (2048, 2161, 8192), (3840, 2160, 15359), (u32::MAX, u32::MAX, 0)] {
            assert_eq!(source_len(width, height, pitch), None);
        }
    }
}
