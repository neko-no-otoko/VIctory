//! The title's own id (from its manifest), which runtime areas exist, and
//! game invites and events. Offline no invite arrives and events go
//! nowhere.
#![allow(clippy::not_unsafe_ptr_arg_deref)]

use std::ffi::{CStr, c_char, c_void};
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::time::Duration;

use super::xgameruntime::{
    Guid, HRESULT, Interface, Methods, Object, S_OK, add_ref, missing, put, text, token,
};
use crate::runtime;

/// Title id and launching another title.
pub const TITLE_PROVIDER: Guid = Guid::new(
    0x973a_344e,
    0x24bf,
    0x4d0f,
    [0x84, 0x57, 0x56, 0xc5, 0x34, 0x89, 0x2b, 0x29],
);
/// As Minecraft (2022) asks for it.
pub const TITLE_IID: Guid = Guid::new(
    0x2549_f142,
    0x6419,
    0x4a06,
    [0x97, 0xb5, 0x93, 0x1a, 0xab, 0x7c, 0x2f, 0x34],
);
/// Feature checks; the interface id is the provider's, as for the next two.
pub const FEATURES: Guid = Guid::new(
    0x8836_fe87,
    0xedb9,
    0x4fe3,
    [0x8d, 0xad, 0x05, 0xf0, 0xd2, 0xcd, 0x5b, 0x40],
);
pub const INVITES: Guid = Guid::new(
    0x0651_aae2,
    0x4012,
    0x4077,
    [0xbf, 0x84, 0x8b, 0x90, 0x97, 0x09, 0x0e, 0x2c],
);
pub const EVENTS: Guid = Guid::new(
    0xbbfb_dcc7,
    0xbfe7,
    0x409b,
    [0xa5, 0xca, 0xed, 0xf0, 0x54, 0x96, 0x0b, 0x4d],
);

pub static TITLE_OBJECT: Object = Object::new("title", &TITLE_VTABLE);
pub static FEATURES_OBJECT: Object = Object::new("features", &FEATURES_VTABLE);
pub static INVITES_OBJECT: Object = Object::new("invites", &INVITES_VTABLE);
pub static EVENTS_OBJECT: Object = Object::new("events", &EVENTS_VTABLE);

pub const INTERFACES: &[Interface] = &[
    (TITLE_PROVIDER, TITLE_IID, &TITLE_OBJECT),
    (FEATURES, FEATURES, &FEATURES_OBJECT),
    (INVITES, INVITES, &INVITES_OBJECT),
    (EVENTS, EVENTS, &EVENTS_OBJECT),
];

type This = *const Object;

extern "system" fn title_id(_: This, out: *mut u32) -> HRESULT {
    put(out, runtime::title().id)
}

/// Ends the title and launches another game. Native launches restart the
/// custom loader; Wine launches ask the supervising launcher instead.
extern "system" fn launch_new_game(
    _: This,
    exe: *const c_char,
    args: *const c_char,
    _user: *const c_void,
) {
    #[cfg(windows)]
    {
        if runtime::title_storage().1 != runtime::StorageMode::Virtual
            || std::env::var_os("VICTORY_MAPPED_RESULT_ADDR").is_some()
        {
            // Copy the title's input before suspension can invalidate it.
            let request = wine_launch_text(exe).and_then(|exe| {
                wine_launch_text(args).map(|args| (exe, args))
            });
            super::lifecycle::end_title(
                "XLaunchNewGame",
                Box::new(move || {
                    let result = request.and_then(|(exe, args)| wine_relaunch(&exe, &args));
                    if let Err(error) = result {
                        eprintln!("XLaunchNewGame: Wine relaunch failed: {error}");
                        std::process::exit(1);
                    }
                }),
            );
        } else {
            let (exe, args) = (text(exe).unwrap_or_default(), text(args).unwrap_or_default());
            tracing::info!(exe, args, "XLaunchNewGame");
            let current = runtime::main_image()
                .and_then(runtime::image_path)
                .unwrap_or_default();
            let ours: Vec<_> = std::env::args_os().skip(1).collect();
            let command = relaunch_args(ours, current.as_os_str(), &exe, &args);
            super::lifecycle::end_title(
                "XLaunchNewGame",
                Box::new(move || super::lifecycle::relaunch(command)),
            );
        }
        loop {
            std::thread::park();
        }
    }
    #[cfg(not(windows))]
    let _ = (exe, args);
}

const WINE_LAUNCH_LIMIT: usize = 32_767;
const WINE_LAUNCH_TIMEOUT: Duration = Duration::from_secs(5);

fn wine_launch_text(ptr: *const c_char) -> Result<String, String> {
    if ptr.is_null() {
        return Ok(String::new());
    }
    // SAFETY: the title supplies a readable NUL-terminated string. Bound the
    // scan so an overlong input is rejected without copying it.
    let len = (0..=WINE_LAUNCH_LIMIT)
        .find(|&i| unsafe { *ptr.add(i) == 0 })
        .ok_or("launch input exceeds 32767 bytes")?;
    // SAFETY: the scan above established the readable byte range.
    let bytes = unsafe { std::slice::from_raw_parts(ptr.cast::<u8>(), len) };
    std::str::from_utf8(bytes)
        .map(str::to_owned)
        .map_err(|_| "launch input is not UTF-8".into())
}

fn wine_relaunch(exe: &str, args: &str) -> Result<(), String> {
    if [exe, args].iter().any(|s| s.len() > WINE_LAUNCH_LIMIT || s.as_bytes().contains(&0)) {
        return Err("invalid launch input".into());
    }
    let token = std::env::var("VICTORY_WINE_METAL_TOKEN")
        .map_err(|_| "missing or invalid Wine launcher token")?;
    if token.len() != 32 || !token.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)) {
        return Err("invalid Wine launcher token".into());
    }
    let address = std::env::var("VICTORY_WINE_RELAUNCH_ADDR")
        .map_err(|_| "missing or invalid Wine launcher address")?;
    if address.len() > 64 {
        return Err("invalid Wine launcher address".into());
    }
    let address: SocketAddr = address.parse().map_err(|_| "invalid Wine launcher address")?;
    if !address.ip().is_loopback() || address.port() == 0 {
        return Err("Wine launcher address is not a loopback endpoint".into());
    }
    let mut stream = TcpStream::connect_timeout(&address, WINE_LAUNCH_TIMEOUT)
        .map_err(|e| format!("connecting to Wine launcher: {e}"))?;
    stream.set_read_timeout(Some(WINE_LAUNCH_TIMEOUT))
        .and_then(|_| stream.set_write_timeout(Some(WINE_LAUNCH_TIMEOUT)))
        .map_err(|e| format!("setting Wine launcher timeouts: {e}"))?;
    let mut send = || -> std::io::Result<()> {
        stream.write_all(b"VILNCH01")?;
        stream.write_all(token.as_bytes())?;
        stream.write_all(&(exe.len() as u32).to_le_bytes())?;
        stream.write_all(&(args.len() as u32).to_le_bytes())?;
        stream.write_all(exe.as_bytes())?;
        stream.write_all(args.as_bytes())?;
        let mut ack = [0];
        stream.read_exact(&mut ack)?;
        if ack != [1] {
            return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "invalid acknowledgement"));
        }
        Ok(())
    };
    send().map_err(|e| format!("communicating with Wine launcher: {e}"))
}

/// Our own arguments for the relaunch: the title's executable replaced by
/// `exe`, earlier title arguments dropped, `args` after `--`.
fn relaunch_args(
    ours: Vec<std::ffi::OsString>,
    current: &std::ffi::OsStr,
    exe: &str,
    args: &str,
) -> Vec<std::ffi::OsString> {
    let mut out: Vec<_> = ours.into_iter().take_while(|a| a != "--").collect();
    // `runtime` makes the image path absolute when it maps the title, so a
    // relative launch argument is not textually equal to it. Normalise the
    // same way before matching, or a relaunch would run the old executable.
    if !exe.is_empty()
        && let Some(slot) = out.iter_mut().find(|a| {
            a.as_os_str() == current
                || std::path::absolute(a).is_ok_and(|p| p.as_os_str() == current)
        })
    {
        *slot = exe.into();
    }
    if !args.trim().is_empty() {
        out.push("--".into());
        out.extend(victory_core::split_title_args(args).into_iter().map(Into::into));
    }
    out
}


/// Runtime areas by number, in the public feature list's order, that we
/// serve: accessibility (0), app capture (1), async calls and providers
/// (2, 3), display (4), game (5), invites (6), game saves (7), game UI (8),
/// launcher (9), networking (10), packages (11), persistent local storage
/// (12), speech synthesis (13), store (14), system (15), task queues (16),
/// users (18), error reports (19), game events (20). Not threads (17) or
/// game streaming (21).
const AVAILABLE: [u32; 20] = [
    0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 18, 19, 20,
];

extern "system" fn is_feature_available(_: This, feature: u32) -> bool {
    AVAILABLE.contains(&feature)
}

/// Nothing raises these events offline, so the callback never runs.
extern "system" fn register(
    _: This,
    _queue: *const c_void,
    _context: *mut c_void,
    _callback: *const c_void,
    out: *mut u64,
) -> HRESULT {
    put(out, token())
}
extern "system" fn unregister(_: This, _token: u64, _wait: bool) -> bool {
    true
}

/// Accepted and dropped; the name goes to the debug log.
extern "system" fn write_event(
    _: This,
    _user: *const c_void,
    _service_config: *const c_char,
    _session: *const c_char,
    name: *const c_char,
    _dimensions: *const c_char,
    _measurements: *const c_char,
) -> HRESULT {
    if !name.is_null() {
        // SAFETY: a non-null name is a NUL-terminated string.
        let name = unsafe { CStr::from_ptr(name) }.to_string_lossy();
        tracing::debug!(%name, "game event dropped");
    }
    S_OK
}

static TITLE_VTABLE: Methods<5> = Methods([
    missing::<0> as *const (),
    add_ref as *const (),
    add_ref as *const (),
    title_id as *const (),        // 0x18
    launch_new_game as *const (), // 0x20
]);

static FEATURES_VTABLE: Methods<4> = Methods([
    missing::<0> as *const (),
    add_ref as *const (),
    add_ref as *const (),
    is_feature_available as *const (), // 0x18
]);

static INVITES_VTABLE: Methods<5> = Methods([
    missing::<0> as *const (),
    add_ref as *const (),
    add_ref as *const (),
    register as *const (),   // 0x18
    unregister as *const (), // 0x20
]);

static EVENTS_VTABLE: Methods<4> = Methods([
    missing::<0> as *const (),
    add_ref as *const (),
    add_ref as *const (),
    write_event as *const (), // 0x18
]);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modules::xgameruntime::method;

    #[test]
    fn a_relaunch_keeps_our_options() {
        let ours: Vec<std::ffi::OsString> =
            ["run", "G:/game.exe", "--execute", "--console", "series-s", "--", "-old"]
                .map(Into::into)
                .into();
        let current = std::ffi::OsStr::new("G:/game.exe");
        let again = relaunch_args(ours.clone(), current, "G:/game.exe", " -config=x  -y ");
        let expected = ["run", "G:/game.exe", "--execute", "--console", "series-s", "--"];
        assert_eq!(again[..6], expected.map(std::ffi::OsString::from));
        assert_eq!(again[6..], ["-config=x", "-y"].map(std::ffi::OsString::from));
        let other = relaunch_args(ours, current, "G:/other.exe", "");
        assert_eq!(other.len(), 5);
        assert_eq!(other[1], "G:/other.exe");
    }

    #[test]
    fn a_relaunch_replaces_a_relative_startup_executable() {
        // `runtime::set_image_path` makes this absolute before the launch.
        let current = std::path::absolute("game.exe").unwrap();
        let ours: Vec<std::ffi::OsString> =
            ["run", "game.exe", "--execute", "--console", "series-s", "--", "-old"]
                .map(Into::into)
                .into();
        let again = relaunch_args(
            ours,
            current.as_os_str(),
            "other.exe",
            r#"--config="C:\Program Files\game" "" -name=O'Brien -y"#,
        );
        let expected: Vec<std::ffi::OsString> = [
            "run",
            "other.exe",
            "--execute",
            "--console",
            "series-s",
            "--",
            r"--config=C:\Program Files\game",
            "",
            "-name=O'Brien",
            "-y",
        ]
        .map(Into::into)
        .into();
        assert_eq!(again, expected, "the requested exe and quoted args survive");
    }

    #[test]
    fn served_areas_are_available_others_not() {
        type Check = extern "system" fn(This, u32) -> bool;
        let check = method::<Check>(&FEATURES_OBJECT, 0x18);
        assert!(check(&FEATURES_OBJECT, 18), "users");
        assert!(!check(&FEATURES_OBJECT, 21), "game streaming");

        type Register = extern "system" fn(
            This,
            *const c_void,
            *mut c_void,
            *const c_void,
            *mut u64,
        ) -> HRESULT;
        type Unregister = extern "system" fn(This, u64, bool) -> bool;
        let null = std::ptr::null();
        let mut token = 0;
        let register = method::<Register>(&INVITES_OBJECT, 0x18);
        assert_eq!(
            register(&INVITES_OBJECT, null, null as _, null, &mut token),
            S_OK
        );
        assert_ne!(token, 0);
        assert!(method::<Unregister>(&INVITES_OBJECT, 0x20)(
            &INVITES_OBJECT,
            token,
            true
        ));
    }
}
