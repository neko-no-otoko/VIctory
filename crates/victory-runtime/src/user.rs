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
