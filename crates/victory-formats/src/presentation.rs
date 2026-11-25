//! Linear 8-bit color source layout accepted by the Metal presentation path.

/// Maximum packed BGRA8 drawable readback, independent of the source dimensions.
pub const MAX_CAPTURE_BYTES: usize = 16 * 1024 * 1024;

/// Accessible source bytes through the last pixel (no trailing row padding).
/// Bound the stride independently: a single row does not otherwise constrain it.
pub fn source_len(width: u32, height: u32, pitch: u32) -> Option<usize> {
    if !(1..=3840).contains(&width) || !(1..=2160).contains(&height) {
        return None;
    }
    let row = width * 4;
    if !(row..=16 * 1024).contains(&pitch) {
        return None;
    }
    let bytes = (height - 1) * pitch + row; // bounded above by 2160 * 16 KiB
    (bytes <= 32 * 1024 * 1024).then_some(bytes as usize)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounds_packed_and_padded_rows_independently_of_capture() {
        assert_eq!(source_len(3840, 2160, 15360), Some(33_177_600));
        assert_eq!(source_len(3840, 2160, 15376), Some(33_212_144));
        assert_eq!(source_len(3840, 2160, 16384), None);
        for (width, height, pitch) in [(0, 1, 4), (1, 0, 4), (3841, 1, 15364),
            (1, 2161, 4), (1, 1, u32::MAX), (3840, 2160, 15359), (u32::MAX, u32::MAX, 0)] {
            assert_eq!(source_len(width, height, pitch), None);
        }
    }
}
