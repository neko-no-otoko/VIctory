//! Identity of the single offline local user.

pub const XUID: u64 = 0x0009_0000_0000_0001;
pub const LOCAL_ID: u64 = 1;
pub const GAMERTAG: &str = "Player";
/// The documented "no such user" error.
pub const USER_NOT_FOUND: i32 = 0x8924_5104_u32 as i32;

pub fn find_id(id: u64) -> bool {
    id == XUID
}

pub fn find_local_id(id: u64) -> bool {
    id == LOCAL_ID
}

/// Copies the requested gamertag component and its NUL terminator.
///
/// # Safety
/// If non-null and large enough, `buffer` must be writable for `size` bytes.
/// If non-null, `used` must be writable for a `usize`.
pub unsafe fn write_gamertag(component: u32, size: usize, buffer: *mut u8, used: *mut usize) -> i32 {
    let text = if component == 2 { "" } else { GAMERTAG };
    let len = text.len() + 1;
    if buffer.is_null() || size < len {
        return 0x8007_007a_u32 as i32;
    }
    // SAFETY: the caller guarantees valid output pointers.
    unsafe {
        buffer.copy_from_nonoverlapping(text.as_ptr(), text.len());
        buffer.add(text.len()).write(0);
        if !used.is_null() {
            used.write(len);
        }
    }
    0
}
