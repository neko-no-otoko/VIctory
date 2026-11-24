//! Authenticated, synchronous presentation over the Wine Metal helper protocol.
use std::{io::{Read, Write}, net::{SocketAddr, TcpStream}, time::Duration};

const MAGIC: u32 = 0x31474d56;
const MAX_IMAGE: usize = 16 * 1024 * 1024;

/// The drawable dimensions and, when requested, tightly packed BGRA8 pixels.
pub struct Capture {
    pub width: u32,
    pub height: u32,
    pub bytes: Vec<u8>,
}

/// Read the authenticated loopback helper endpoint from the Wine environment.
pub fn config() -> Result<(SocketAddr, [u8; 16]), String> {
    let addr: SocketAddr = std::env::var("VICTORY_WINE_METAL_ADDR").map_err(|e| e.to_string())?
        .parse::<SocketAddr>().map_err(|e| e.to_string())?;
    if !addr.ip().is_loopback() || addr.port() == 0 { return Err("proxy address must be a loopback socket".into()); }
    let s = std::env::var("VICTORY_WINE_METAL_TOKEN").map_err(|e| e.to_string())?;
    if s.len() != 32 || !s.is_ascii() { return Err("token must be 32 hex digits".into()); }
    let mut token = [0; 16];
    for (i, b) in token.iter_mut().enumerate() {
        *b = u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).map_err(|_| "invalid token")?;
    }
    Ok((addr, token))
}

/// Present one RGBA8 or BGRA8 image. Returns only after GPU presentation completes.
pub fn send(addr: SocketAddr, token: [u8; 16], width: u32, height: u32, format: u32, pitch: u32, bytes: &[u8], capture: bool) -> Result<Capture, String> {
    if !addr.ip().is_loopback() || addr.port() == 0 || width == 0 || height == 0 || !matches!(format, 28 | 87) {
        return Err("invalid presentation endpoint or image".into());
    }
    let row = (width as usize).checked_mul(4).ok_or("image row overflow")?;
    let len = (height as usize - 1).checked_mul(pitch as usize).and_then(|n| n.checked_add(row)).ok_or("image size overflow")?;
    if (pitch as usize) < row || len > MAX_IMAGE || bytes.len() != len { return Err("invalid presentation image length or pitch".into()); }
    let mut s = TcpStream::connect_timeout(&addr, Duration::from_secs(30)).map_err(|e| e.to_string())?;
    s.set_read_timeout(Some(Duration::from_secs(60))).map_err(|e| e.to_string())?;
    s.set_write_timeout(Some(Duration::from_secs(60))).map_err(|e| e.to_string())?;
    for v in [MAGIC, 4, width, height, format, pitch, len as u32, u32::from(capture)] {
        s.write_all(&v.to_le_bytes()).map_err(|e| e.to_string())?;
    }
    s.write_all(&token).and_then(|_| s.write_all(bytes)).map_err(|e| e.to_string())?;
    let mut header = [0; 16];
    s.read_exact(&mut header).map_err(|e| e.to_string())?;
    let word = |n| u32::from_le_bytes(header[n..n + 4].try_into().unwrap());
    let (magic, status, len, msg) = (word(0), word(4), word(8) as usize, word(12) as usize);
    if magic != MAGIC || len > MAX_IMAGE + 12 || msg > 4096 || (status == 0 && (msg != 0 || len < 12)) || (status != 0 && len != 0) {
        return Err("invalid presentation response".into());
    }
    let mut payload = vec![0; len];
    let mut message = vec![0; msg];
    s.read_exact(&mut payload).and_then(|_| s.read_exact(&mut message)).map_err(|e| e.to_string())?;
    if status != 0 { return Err(format!("proxy status {status}: {}", String::from_utf8_lossy(&message))); }
    let word = |n| u32::from_le_bytes(payload[n..n + 4].try_into().unwrap());
    let (width, height, captured) = (word(0), word(4), word(8) as usize);
    let expected = (width as usize).checked_mul(height as usize).and_then(|n| n.checked_mul(4)).ok_or("drawable size overflow")?;
    if width == 0 || height == 0 || captured != len - 12 || (capture && captured != expected) || (!capture && captured != 0) {
        return Err("invalid drawable capture".into());
    }
    Ok(Capture { width, height, bytes: payload.split_off(12) })
}
