//! Opt-in loopback server for the restricted Wine Metal compute protocol.

#[cfg(all(target_os = "macos", target_arch = "x86_64"))]
mod host {
    use std::io::{self, Read, Write};
    use std::net::{SocketAddr, TcpListener, TcpStream};

    use std::time::Duration;
    const MAGIC: u32 = 0x3147_4d56;
    const MAX_SHADER: usize = 4 * 1024 * 1024;
    const MAX_BUFFER: usize = 16 * 1024 * 1024;
    const MAX_MESSAGE: usize = 4096;

    fn token() -> Result<[u8; 16], String> {
        let value = std::env::var("VICTORY_WINE_METAL_TOKEN")
            .map_err(|_| "VICTORY_WINE_METAL_TOKEN must be 32 hexadecimal digits")?;
        let bytes = value.as_bytes();
        if bytes.len() != 32 || !bytes.iter().all(u8::is_ascii_hexdigit) {
            return Err("VICTORY_WINE_METAL_TOKEN must be 32 hexadecimal digits".into());
        }
        let mut token = [0; 16];
        for (i, byte) in token.iter_mut().enumerate() {
            let digit = |c: u8| match c {
                b'0'..=b'9' => c - b'0',
                b'a'..=b'f' => c - b'a' + 10,
                _ => c - b'A' + 10,
            };
            *byte = (digit(bytes[2 * i]) << 4) | digit(bytes[2 * i + 1]);
        }
        Ok(token)
    }

    fn word(bytes: &[u8]) -> u32 {
        u32::from_le_bytes(bytes.try_into().expect("four-byte word"))
    }

    fn request(
        stream: &mut TcpStream,
        expected_token: &[u8; 16],
        cache: &mut Option<(Vec<u8>, victory_gpu::WineMetalCompute)>,
    ) -> Result<Vec<u8>, String> {
        let mut header = [0; 32];
        stream.read_exact(&mut header).map_err(|e| e.to_string())?;
        let fields: [u32; 8] = std::array::from_fn(|i| word(&header[i * 4..i * 4 + 4]));
        if fields[0] != MAGIC || fields[1] != 1 {
            return Err("invalid protocol header".into());
        }
        let shader_len = fields[2] as usize;
        let buffer_len = fields[3] as usize;
        let count = fields[4] as usize;
        let groups = [fields[5], fields[6], fields[7]];
        if shader_len == 0 || shader_len > MAX_SHADER || buffer_len == 0
            || buffer_len > MAX_BUFFER || count > 64 || groups.contains(&0)
        {
            return Err("invalid request lengths or dispatch dimensions".into());
        }
        let mut supplied_token = [0; 16];
        stream.read_exact(&mut supplied_token).map_err(|e| e.to_string())?;
        if supplied_token.iter().zip(expected_token).fold(0u8, |diff, (a, b)| diff | (a ^ b)) != 0 {
            return Err("authentication failed".into());
        }
        let mut shader = vec![0; shader_len];
        let mut buffer = vec![0; buffer_len];
        let mut constant_bytes = vec![0; count * 4];
        stream.read_exact(&mut shader).map_err(|e| e.to_string())?;
        stream.read_exact(&mut buffer).map_err(|e| e.to_string())?;
        stream.read_exact(&mut constant_bytes).map_err(|e| e.to_string())?;
        let constants: Vec<u32> = constant_bytes.chunks_exact(4).map(word).collect();
        if cache.as_ref().is_none_or(|(cached, _)| *cached != shader) {
            // Drop the old pipeline before constructing a replacement.
            *cache = None;
            let pipeline = victory_gpu::WineMetalCompute::new(&shader)?;
            *cache = Some((shader, pipeline));
        }
        cache.as_mut().expect("pipeline was prepared").1.dispatch(&mut buffer, &constants, groups)?;
        Ok(buffer)
    }

    fn respond(stream: &mut TcpStream, result: Result<Vec<u8>, String>) -> io::Result<()> {
        let (status, data, message) = match result {
            Ok(data) => (0u32, data, String::new()),
            Err(mut message) => {
                if message.len() > MAX_MESSAGE {
                    let mut end = MAX_MESSAGE;
                    while !message.is_char_boundary(end) { end -= 1; }
                    message.truncate(end);
                }
                (1, Vec::new(), message)
            }
        };
        for value in [MAGIC, status, data.len() as u32, message.len() as u32] {
            stream.write_all(&value.to_le_bytes())?;
        }
        stream.write_all(&data)?;
        stream.write_all(message.as_bytes())
    }

    pub fn run() -> Result<(), String> {
        let mut args = std::env::args();
        let program = args.next().unwrap_or_default();
        let address: SocketAddr = args.next().ok_or_else(|| format!("usage: {program} LOOPBACK_ADDR:PORT"))?
            .parse().map_err(|e| format!("invalid listen address: {e}"))?;
        if args.next().is_some() || !address.ip().is_loopback() {
            return Err("expected exactly one loopback listen address".into());
        }
        let token = token()?;
        let listener = TcpListener::bind(address).map_err(|e| e.to_string())?;
        let mut cache = None;
        for connection in listener.incoming() {
            match connection {
                Ok(mut stream) => {
                    if !stream.peer_addr().map(|a| a.ip().is_loopback()).unwrap_or(false) {
                        continue;
                    }
                    let deadline = Some(Duration::from_secs(30));
                    if let Err(error) = stream.set_read_timeout(deadline)
                        .and_then(|()| stream.set_write_timeout(deadline))
                    {
                        eprintln!("setting connection timeouts failed: {error}");
                        continue;
                    }
                    let result = request(&mut stream, &token, &mut cache);
                    if let Err(error) = respond(&mut stream, result) {
                        eprintln!("response failed: {error}");
                    }
                }
                Err(error) => eprintln!("accept failed: {error}"),
            }
        }
        Ok(())
    }
}

#[cfg(all(target_os = "macos", target_arch = "x86_64"))]
fn main() {
    if let Err(error) = host::run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}

#[cfg(not(all(target_os = "macos", target_arch = "x86_64")))]
fn main() {
    eprintln!("wine_metal_compute_host requires macOS x86_64");
    std::process::exit(1);
}
