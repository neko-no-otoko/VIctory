//! Opt-in Aqua-session probe: real drawable readback, close, then rejection.
#[cfg(all(target_os = "macos", target_arch = "x86_64"))]
fn main() {
    use victory_metal::present::{close, present, resize, run_app};
    let (tx, rx) = std::sync::mpsc::channel();
    run_app(move || {
        let outcome = (|| -> Result<(), String> {
            let mut pixels = vec![0u8; 64 * 64 * 4];
            for pixel in pixels.chunks_exact_mut(4) { pixel.copy_from_slice(&[223, 37, 19, 255]); }
            let frame = present(64, 64, 28, 256, &pixels, true).map_err(|e| e.to_string())?;
            if frame.width == 0 || frame.height == 0 || frame.capture.len() != frame.width as usize * frame.height as usize * 4 {
                return Err("invalid drawable dimensions or capture length".into());
            }
            let index = ((frame.height as usize / 2) * frame.width as usize + frame.width as usize / 2) * 4;
            if frame.capture[index..index+4] != [19, 37, 223, 255] { return Err("drawable center did not match the submitted color".into()); }
            resize(128, 96).map_err(|e| e.to_string())?;
            let resized = present(64, 64, 28, 256, &pixels, true).map_err(|e| e.to_string())?;
            if resized.width <= frame.width || resized.height <= frame.height {
                return Err("drawable did not follow the resized AppKit view".into());
            }
            let index = ((resized.height as usize / 2) * resized.width as usize + resized.width as usize / 2) * 4;
            if resized.capture[index..index+4] != [19, 37, 223, 255] {
                return Err("resized drawable center did not match the submitted color".into());
            }
            close().map_err(|e| e.to_string())?;
            if present(64, 64, 28, 256, &pixels, false).is_ok() { return Err("present succeeded after close".into()); }
            Ok(())
        })();
        tx.send(outcome).unwrap();
    }).expect("AppKit presentation loop failed");
    rx.recv().expect("presentation worker did not return").expect("presentation probe failed");
    println!("drawable readback, resize and close rejection passed");
}

#[cfg(not(all(target_os = "macos", target_arch = "x86_64")))]
fn main() {
    panic!("present_probe requires macOS x86_64 and an Aqua session");
}
