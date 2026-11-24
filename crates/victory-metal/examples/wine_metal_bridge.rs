//! Experimental private-pipe transport for a trusted Wine guest, not an Xbox API.
//! Usage: wine_metal_bridge WINE GUEST [GUEST_ARG ...]

#[cfg(target_os = "macos")]
pub(crate) mod host {
    use std::ffi::OsString;
    use std::io::{self, Read, Write};
    use std::process::{Command, Stdio};

    const MAGIC: u32 = 0x3142_4d56;
    const MAX_PAYLOAD: usize = 1024 * 1024;

    fn invalid(message: &'static str) -> io::Error {
        io::Error::new(io::ErrorKind::InvalidData, message)
    }

    fn response(output: &mut impl Write, status: u32, pixels: &[u8]) -> io::Result<()> {
        output.write_all(&MAGIC.to_le_bytes())?;
        output.write_all(&status.to_le_bytes())?;
        output.write_all(&(pixels.len() as u32).to_le_bytes())?;
        output.write_all(pixels)?;
        output.flush()
    }

    fn serve(input: &mut impl Read, output: &mut impl Write) -> io::Result<()> {
        loop {
            let mut header = [0u8; 28];
            // EOF is only normal between complete frames.
            match input.read(&mut header[..1])? {
                0 => return Ok(()),
                _ => {}
            }
            if let Err(error) = input.read_exact(&mut header[1..]) {
                let _ = response(output, 1, &[]);
                return Err(error);
            }
            let word = |i: usize| u32::from_le_bytes(header[i..i + 4].try_into().unwrap());
            let (magic, op, width, height, pitch, len, color) =
                (word(0), word(4), word(8), word(12), word(16), word(20), word(24));
            let pitch = pitch as usize;
            let len = len as usize;
            let valid = magic == MAGIC && op == 1 && width != 0 && height != 0
                && len <= MAX_PAYLOAD
                && (width as usize).checked_mul(4).is_some_and(|row| {
                    pitch >= row && pitch.checked_mul(height as usize - 1)
                        .and_then(|n| n.checked_add(row)).is_some_and(|required| required <= len)
                });
            if !valid {
                let _ = response(output, 1, &[]);
                return Err(invalid("invalid Wine Metal request"));
            }
            let mut pixels = vec![0; len];
            if let Err(error) = input.read_exact(&mut pixels) {
                let _ = response(output, 1, &[]);
                return Err(error);
            }
            if let Err(error) = victory_metal::clear_rgba8(
                width, height, pitch, &mut pixels, color.to_le_bytes(), &[],
            ) {
                let _ = response(output, 1, &[]);
                return Err(io::Error::other(error));
            }
            response(output, 0, &pixels)?;
        }
    }

    pub fn run(args: impl IntoIterator<Item = OsString>) -> Result<(), Box<dyn std::error::Error>> {
        let mut args = args.into_iter();
        let wine = args.next().ok_or("usage: wine_metal_bridge WINE GUEST [GUEST_ARG ...]")?;
        let guest = args.next().ok_or("usage: wine_metal_bridge WINE GUEST [GUEST_ARG ...]")?;
        let mut child = Command::new(wine).arg(guest).args(args)
            .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::inherit()).spawn()?;
        let mut input = child.stdout.take().expect("piped stdout");
        let mut output = child.stdin.take().expect("piped stdin");
        let result = serve(&mut input, &mut output);
        drop(input);
        drop(output);
        if let Err(error) = result {
            let _ = child.kill();
            let _ = child.wait();
            return Err(error.into());
        }
        let status = child.wait()?;
        if !status.success() {
            return Err(format!("Wine guest exited with {status}").into());
        }
        Ok(())
    }
    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn rejects_invalid_or_truncated_frames() {
            for words in [
                [0, 1, 2, 2, 8, 16, 0],
                [MAGIC, 2, 2, 2, 8, 16, 0],
                [MAGIC, 1, 2, 2, 8, 1024 * 1024 + 1, 0],
                [MAGIC, 1, 2, 2, 7, 16, 0],
            ] {
                let bytes: Vec<u8> = words.into_iter().flat_map(u32::to_le_bytes).collect();
                let mut reply = Vec::new();
                assert!(serve(&mut bytes.as_slice(), &mut reply).is_err());
                assert_eq!(reply, [MAGIC.to_le_bytes(), 1u32.to_le_bytes(), 0u32.to_le_bytes()].concat());
            }
            let mut reply = Vec::new();
            assert!(serve(&mut &[1u8, 2][..], &mut reply).is_err());
            assert_eq!(reply, [MAGIC.to_le_bytes(), 1u32.to_le_bytes(), 0u32.to_le_bytes()].concat());
        }
    }
}

#[cfg(target_os = "macos")]
fn main() {
    if let Err(error) = host::run(std::env::args_os().skip(1)) {
        eprintln!("wine_metal_bridge: {error}");
        std::process::exit(1);
    }
}

#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("wine_metal_bridge requires macOS Metal");
    std::process::exit(1);
}
