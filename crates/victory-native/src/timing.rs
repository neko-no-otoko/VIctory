//! Process-wide monotonic timing services for the native guest.
use std::sync::LazyLock;

use victory_pe::ImportSymbol;

const ERROR_INVALID_PARAMETER: u32 = 87;
const ERROR_GEN_FAILURE: u32 = 31;
const FREQUENCY: i64 = 1_000_000_000;
const INFINITE: u32 = u32::MAX;

#[repr(C)]
struct MachTimebaseInfo {
    numer: u32,
    denom: u32,
}

unsafe extern "C" {
    fn mach_continuous_time() -> u64;
    fn mach_timebase_info(info: *mut MachTimebaseInfo) -> i32;
}

// Mach ticks have a rational period. Convert to nanoseconds with a wide
// intermediate, giving QPC an integral, stable frequency on every host.
fn timebase() -> Option<(u32, u32)> {
    static TIMEBASE: LazyLock<Option<(u32, u32)>> = LazyLock::new(|| {
        let mut info = MachTimebaseInfo { numer: 0, denom: 0 };
        let status = unsafe { mach_timebase_info(&mut info) };
        (status == 0 && info.numer != 0 && info.denom != 0)
            .then_some((info.numer, info.denom))
    });
    *TIMEBASE
}

fn nanoseconds() -> Option<u128> {
    let (numer, denom) = timebase()?;
    let ticks = unsafe { mach_continuous_time() };
    Some(u128::from(ticks) * u128::from(numer) / u128::from(denom))
}

pub(crate) fn resolve(dll: &str, symbol: &ImportSymbol) -> Option<usize> {
    if !dll.eq_ignore_ascii_case("kernel32.dll") { return None; }
    let ImportSymbol::Name { name, .. } = symbol else { return None };
    Some(match name.as_str() {
        "QueryPerformanceCounter" => query_performance_counter as *const () as usize,
        "QueryPerformanceFrequency" => query_performance_frequency as *const () as usize,
        "GetTickCount64" => get_tick_count64 as *const () as usize,
        "Sleep" => sleep as *const () as usize,
        _ => return None,
    })
}

extern "win64" fn query_performance_counter(output: *mut i64) -> i32 {
    if output.is_null() {
        crate::files::fail(ERROR_INVALID_PARAMETER);
        return 0;
    }
    let Some(ns) = nanoseconds() else {
        crate::files::fail(ERROR_GEN_FAILURE);
        return 0;
    };
    // Saturation preserves monotonicity even after the signed QPC range ends.
    unsafe { output.write(ns.min(i64::MAX as u128) as i64) };
    1
}

extern "win64" fn query_performance_frequency(output: *mut i64) -> i32 {
    if output.is_null() {
        crate::files::fail(ERROR_INVALID_PARAMETER);
        return 0;
    }
    if timebase().is_none() {
        crate::files::fail(ERROR_GEN_FAILURE);
        return 0;
    }
    unsafe { output.write(FREQUENCY) };
    1
}

extern "win64" fn get_tick_count64() -> u64 {
    // mach_timebase_info is guaranteed to succeed for a valid pointer on
    // Darwin. There is no meaningful tick count if the host violates that.
    let Some(ns) = nanoseconds() else { std::process::abort() };
    (ns / 1_000_000).min(u64::MAX as u128) as u64
}

extern "win64" fn sleep(milliseconds: u32) {
    if milliseconds == 0 {
        unsafe { libc::sched_yield() };
    } else if milliseconds == INFINITE {
        loop { unsafe { libc::pause() }; }
    } else {
        let mut remaining = libc::timespec {
            tv_sec: (milliseconds / 1000) as libc::time_t,
            tv_nsec: ((milliseconds % 1000) * 1_000_000) as libc::c_long,
        };
        loop {
            let mut next = libc::timespec { tv_sec: 0, tv_nsec: 0 };
            if unsafe { libc::nanosleep(&remaining, &mut next) } == 0 { break; }
            if std::io::Error::last_os_error().raw_os_error() != Some(libc::EINTR) { break; }
            remaining = next;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    fn symbol(name: &str) -> ImportSymbol {
        ImportSymbol::Name { hint: 0, name: name.into() }
    }

    #[test]
    fn resolver_and_last_error() {
        for name in ["QueryPerformanceCounter", "QueryPerformanceFrequency", "GetTickCount64", "Sleep"] {
            assert!(resolve("KERNEL32.DLL", &symbol(name)).is_some());
            assert!(resolve("other.dll", &symbol(name)).is_none());
        }
        assert!(resolve("kernel32.dll", &symbol("unknown")).is_none());
        let get: extern "win64" fn() -> u32 = unsafe {
            std::mem::transmute(crate::files::resolve("kernel32.dll", &symbol("GetLastError")).unwrap())
        };
        crate::files::fail(1234);
        let mut value = 0;
        assert_eq!(query_performance_frequency(&mut value), 1);
        assert_eq!(value, FREQUENCY);
        assert_eq!(query_performance_counter(&mut value), 1);
        get_tick_count64();
        sleep(0);
        assert_eq!(get(), 1234);
        assert_eq!(query_performance_counter(std::ptr::null_mut()), 0);
        assert_eq!(get(), ERROR_INVALID_PARAMETER);
        crate::files::fail(1234);
        assert_eq!(query_performance_frequency(std::ptr::null_mut()), 0);
        assert_eq!(get(), ERROR_INVALID_PARAMETER);
    }

    #[test]
    fn clocks_share_source_and_are_monotonic_across_threads() {
        let threads: Vec<_> = (0..4).map(|_| std::thread::spawn(|| {
            let mut previous = 0;
            for _ in 0..1000 {
                let mut before = 0;
                let mut after = 0;
                assert_eq!(query_performance_counter(&mut before), 1);
                let tick = get_tick_count64();
                assert_eq!(query_performance_counter(&mut after), 1);
                assert!(before >= previous && after >= before);
                assert!(tick >= before as u64 / 1_000_000);
                assert!(tick <= after as u64 / 1_000_000);
                previous = after;
            }
        })).collect();
        for thread in threads { thread.join().unwrap(); }
    }

    #[test]
    fn finite_sleep_waits() {
        let start = Instant::now();
        sleep(20);
        assert!(start.elapsed() >= Duration::from_millis(20));
    }
}
