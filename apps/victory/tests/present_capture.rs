//! Opt-in real CAMetalDrawable GPU readback over the authenticated helper protocol.
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::time::Duration;

const MAGIC: u32 = 0x3147_4d56;

fn request(addr: SocketAddr, token: &[u8; 16], fields: [u32; 6], pixels: &[u8]) -> (u32, Vec<u8>, String) {
    let mut stream = TcpStream::connect_timeout(&addr, Duration::from_secs(30)).unwrap();
    stream.set_read_timeout(Some(Duration::from_secs(60))).unwrap();
    stream.set_write_timeout(Some(Duration::from_secs(30))).unwrap();
    for value in [MAGIC, 4].into_iter().chain(fields) {
        stream.write_all(&value.to_le_bytes()).unwrap();
    }
    stream.write_all(token).unwrap();
    if !pixels.is_empty() { stream.write_all(pixels).unwrap(); }
    let mut header = [0; 16];
    stream.read_exact(&mut header).unwrap();
    let word = |at| u32::from_le_bytes(header[at..at + 4].try_into().unwrap());
    assert_eq!(word(0), MAGIC);
    let status = word(4);
    let len = word(8) as usize;
    let msg_len = word(12) as usize;
    assert!(len <= 16 * 1024 * 1024 + 12 && msg_len <= 4096);
    let mut payload = vec![0; len];
    let mut message = vec![0; msg_len];
    stream.read_exact(&mut payload).unwrap();
    stream.read_exact(&mut message).unwrap();
    (status, payload, String::from_utf8_lossy(&message).into_owned())
}

#[test]
#[ignore = "requires an Aqua session and a running presentation-enabled Metal helper"]
fn drawable_capture_checks_channels_alpha_orientation_and_changed_source_size() {
    let addr: SocketAddr = std::env::var("VICTORY_WINE_METAL_ADDR").unwrap().parse().unwrap();
    assert!(addr.ip().is_loopback());
    let hex = std::env::var("VICTORY_WINE_METAL_TOKEN").unwrap();
    assert!(hex.len() == 32 && hex.is_ascii());
    let mut token = [0; 16];
    for (i, b) in token.iter_mut().enumerate() { *b = u8::from_str_radix(&hex[2*i..2*i+2], 16).unwrap(); }
    let mut wrong = token; wrong[0] ^= 1;
    let (status, _, _) = request(addr, &wrong, [64, 64, 28, 256, 16384, 1], &[]);
    assert_ne!(status, 0, "wrong token must be rejected");
    // Send headers only: a malformed source must fail before allocating or
    // waiting for the announced payload, even when its pixel count is small.
    for fields in [[0, 64, 28, 256, 16384, 1], [64, 64, 30, 256, 16384, 1],
        [64, 64, 28, 255, 16384, 1], [64, 64, 28, 256, 16384, 2],
        [3841, 1, 28, 15364, 15364, 0], [1, 2161, 28, 4, 8644, 0],
        [1, 1, 28, u32::MAX, 4, 0], [3840, 2160, 28, 16384, 35_388_416, 0],
        [3840, 2160, 28, 15360, 33_177_599, 0]] {
        let (status, _, message) = request(addr, &token, fields, &[]);
        assert_ne!(status, 0, "bad fields must be rejected: {fields:?}");
        assert!(message.contains("invalid present request"), "{fields:?}: {message}");
    }
    // BGRA expectations at top-left, top-right, bottom-left and bottom-right.
    let colors = [[19, 37, 223, 255], [201, 42, 13, 192], [31, 209, 53, 128], [177, 91, 7, 64]];
    for (width, height, format) in [(64usize, 64usize, 28u32), (64, 64, 29), (96, 80, 87), (96, 80, 91)] {
        let pitch = width * 4 + 16;
        let mut pixels = vec![0x5a; (height - 1) * pitch + width * 4];
        for y in 0..height { for x in 0..width {
            let bgra = colors[(y >= height/2) as usize * 2 + (x >= width/2) as usize];
            let pixel = if format == 28 || format == 29 { [bgra[2], bgra[1], bgra[0], bgra[3]] } else { bgra };
            let at = y * pitch + x * 4;
            pixels[at..at+4].copy_from_slice(&pixel);
        }}
        let (status, payload, message) = request(addr, &token, [width as u32, height as u32, format, pitch as u32, pixels.len() as u32, 1], &pixels);
        assert_eq!(status, 0, "{message}");
        assert!(message.is_empty() && payload.len() >= 12);
        let word = |at| u32::from_le_bytes(payload[at..at + 4].try_into().unwrap()) as usize;
        let (dw, dh, len) = (word(0), word(4), word(8));
        assert!(dw >= 4 && dh >= 4);
        assert_eq!(len, dw * dh * 4);
        assert_eq!(payload.len(), 12 + len);
        for (i, (x, y)) in [(dw/4, dh/4), (3*dw/4, dh/4), (dw/4, 3*dh/4), (3*dw/4, 3*dh/4)].into_iter().enumerate() {
            let at = 12 + (y * dw + x) * 4;
            assert_eq!(&payload[at..at+4], &colors[i], "drawable {dw}x{dh}, quadrant {i}, source {width}x{height}, format {format}");
        }
    }
}
