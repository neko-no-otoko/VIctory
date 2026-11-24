//! Process-wide synchronous Win32 events for native guests.
//!
//! Closing a handle removes it immediately. An operation that already looked
//! it up retains the event until it completes; in particular, closing does not
//! wake an existing waiter. Windows leaves close-during-wait undefined, so this
//! is a deterministic safety policy rather than a Windows compatibility promise.
use std::collections::{HashMap, VecDeque};
use std::ffi::c_void;
use std::sync::{Arc, Condvar, LazyLock, Mutex};
use std::time::{Duration, Instant};

use victory_pe::ImportSymbol;

const ERROR_INVALID_HANDLE: u32 = 6;
const ERROR_NOT_SUPPORTED: u32 = 50;
const WAIT_OBJECT_0: u32 = 0;
const WAIT_TIMEOUT: u32 = 258;
const WAIT_FAILED: u32 = u32::MAX;
const INFINITE: u32 = u32::MAX;

static EVENTS: LazyLock<Mutex<HashMap<usize, Arc<Event>>>> = LazyLock::new(|| Mutex::new(HashMap::new()));

struct Waiter {
    id: u64,
    granted: bool,
    deadline: Option<Instant>,
}

struct State {
    signaled: bool,
    next_waiter: u64,
    waiters: VecDeque<Waiter>,
}

struct Event {
    manual: bool,
    state: Mutex<State>,
    changed: Condvar,
    #[cfg(test)]
    registered: Condvar,
}

impl Event {
    fn new(manual: bool, signaled: bool) -> Self {
        Self {
            manual,
            state: Mutex::new(State { signaled, next_waiter: 0, waiters: VecDeque::new() }),
            changed: Condvar::new(),
            #[cfg(test)]
            registered: Condvar::new(),
        }
    }

    // Grants belong to waiters, not the signaled bit: ResetEvent must not
    // revoke a release made before a blocked thread reacquires the mutex.
    fn set_locked(&self, state: &mut State) {
        let now = Instant::now();
        if self.manual {
            state.signaled = true;
            for waiter in &mut state.waiters {
                if waiter.deadline.is_none_or(|deadline| now < deadline) { waiter.granted = true; }
            }
            self.changed.notify_all();
        } else if !state.signaled {
            if let Some(waiter) = state.waiters.iter_mut().find(|waiter| {
                !waiter.granted && waiter.deadline.is_none_or(|deadline| now < deadline)
            }) {
                waiter.granted = true;
                // A shared condvar cannot target the selected waiter.
                self.changed.notify_all();
            } else {
                state.signaled = true;
            }
        }
    }

    fn set(&self) {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        self.set_locked(&mut state);
    }

    fn reset(&self) {
        self.state.lock().unwrap_or_else(|error| error.into_inner()).signaled = false;
    }

    fn wait(&self, milliseconds: u32) -> u32 {
        let deadline = if milliseconds == INFINITE { None } else {
            Some(Instant::now() + Duration::from_millis(u64::from(milliseconds)))
        };
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        if state.signaled {
            if !self.manual { state.signaled = false; }
            return WAIT_OBJECT_0;
        }
        if milliseconds == 0 { return WAIT_TIMEOUT; }
        let mut id = state.next_waiter;
        while state.waiters.iter().any(|waiter| waiter.id == id) { id = id.wrapping_add(1); }
        state.next_waiter = id.wrapping_add(1);
        state.waiters.push_back(Waiter { id, granted: false, deadline });
        #[cfg(test)]
        self.registered.notify_all();
        loop {
            let position = state.waiters.iter().position(|waiter| waiter.id == id).expect("registered waiter");
            if state.waiters[position].granted {
                state.waiters.remove(position);
                return WAIT_OBJECT_0;
            }
            if let Some(deadline) = deadline {
                let remaining = deadline.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    state.waiters.remove(position);
                    return WAIT_TIMEOUT;
                }
                state = self.changed.wait_timeout(state, remaining)
                    .unwrap_or_else(|error| error.into_inner()).0;
            } else {
                state = self.changed.wait(state).unwrap_or_else(|error| error.into_inner());
            }
        }
    }
}

fn lookup(handle: usize) -> Option<Arc<Event>> {
    EVENTS.lock().unwrap_or_else(|error| error.into_inner()).get(&handle).cloned()
}

pub(crate) fn close(handle: usize) -> bool {
    EVENTS.lock().unwrap_or_else(|error| error.into_inner()).remove(&handle).is_some()
}

pub(crate) fn resolve(dll: &str, symbol: &ImportSymbol) -> Option<usize> {
    if !dll.eq_ignore_ascii_case("kernel32.dll") { return None; }
    let ImportSymbol::Name { name, .. } = symbol else { return None };
    Some(match name.as_str() {
        "CreateEventW" => create_event_w as *const () as usize,
        "SetEvent" => set_event as *const () as usize,
        "ResetEvent" => reset_event as *const () as usize,
        "WaitForSingleObject" => wait_for_single_object as *const () as usize,
        _ => return None,
    })
}

extern "win64" fn create_event_w(security: *const c_void, manual: i32, initial: i32, name: *const u16) -> usize {
    if !security.is_null() || !name.is_null() {
        crate::files::fail(ERROR_NOT_SUPPORTED);
        return 0;
    }
    let handle = match crate::files::allocate_handle() {
        Ok(handle) => handle,
        Err(code) => { crate::files::fail(code); return 0; }
    };
    let event = Arc::new(Event::new(manual != 0, initial != 0));
    EVENTS.lock().unwrap_or_else(|error| error.into_inner()).insert(handle, event);
    handle
}

extern "win64" fn set_event(handle: usize) -> i32 {
    match lookup(handle) {
        Some(event) => { event.set(); 1 }
        None => { crate::files::fail(ERROR_INVALID_HANDLE); 0 }
    }
}

extern "win64" fn reset_event(handle: usize) -> i32 {
    match lookup(handle) {
        Some(event) => { event.reset(); 1 }
        None => { crate::files::fail(ERROR_INVALID_HANDLE); 0 }
    }
}

extern "win64" fn wait_for_single_object(handle: usize, milliseconds: u32) -> u32 {
    match lookup(handle) {
        Some(event) => event.wait(milliseconds),
        None => {
            let error = if crate::files::is_scoped_handle(handle) {
                ERROR_NOT_SUPPORTED
            } else {
                ERROR_INVALID_HANDLE
            };
            crate::files::fail(error);
            WAIT_FAILED
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ptr::null;
    use std::sync::mpsc;
    use std::thread;

    fn symbol(name: &str) -> ImportSymbol {
        ImportSymbol::Name { hint: 0, name: name.into() }
    }

    fn close_handle(handle: usize) -> i32 {
        let close: extern "win64" fn(usize) -> i32 = unsafe {
            std::mem::transmute(crate::files::resolve("kernel32.dll", &symbol("CloseHandle")).unwrap())
        };
        close(handle)
    }

    fn get_error() -> u32 {
        let get: extern "win64" fn() -> u32 = unsafe {
            std::mem::transmute(crate::files::resolve("kernel32.dll", &symbol("GetLastError")).unwrap())
        };
        get()
    }

    fn event(manual: bool, initial: bool) -> usize {
        let handle = create_event_w(null(), i32::from(manual), i32::from(initial), null());
        assert_ne!(handle, 0);
        handle
    }

    // A bounded handshake proves the waiters are actually queued; no sleeps or
    // scheduling assumptions are needed for signal/reset and close races.
    fn registered(event: &Event, count: usize) {
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut state = event.state.lock().unwrap();
        while state.waiters.len() < count {
            let remaining = deadline.saturating_duration_since(Instant::now());
            assert!(!remaining.is_zero(), "waiters did not register");
            state = event.registered.wait_timeout(state, remaining).unwrap().0;
        }
    }

    #[test]
    fn resolver_errors_and_preserved_last_error() {
        for name in ["CreateEventW", "SetEvent", "ResetEvent", "WaitForSingleObject"] {
            assert!(resolve("KERNEL32.DLL", &symbol(name)).is_some());
            assert!(resolve("other.dll", &symbol(name)).is_none());
        }
        assert!(crate::files::resolve("kernel32.dll", &symbol("CloseHandle")).is_some());
        assert_eq!(create_event_w(std::ptr::dangling::<c_void>(), 0, 0, null()), 0);
        assert_eq!(get_error(), ERROR_NOT_SUPPORTED);
        assert_eq!(create_event_w(null(), 0, 0, std::ptr::dangling::<u16>()), 0);
        assert_eq!(get_error(), ERROR_NOT_SUPPORTED);
        crate::files::fail(1234);
        let handle = event(false, false);
        assert_eq!(wait_for_single_object(handle, 0), WAIT_TIMEOUT);
        assert_eq!(set_event(handle), 1);
        assert_eq!(wait_for_single_object(handle, 0), WAIT_OBJECT_0);
        assert_eq!(reset_event(handle), 1);
        assert_eq!(close_handle(handle), 1);
        assert_eq!(get_error(), 1234);
        assert_eq!(set_event(handle), 0);
        assert_eq!(get_error(), ERROR_INVALID_HANDLE);
        assert_eq!(reset_event(handle), 0);
        assert_eq!(wait_for_single_object(handle, 0), WAIT_FAILED);
        assert_eq!(close_handle(handle), 0);
        let next = event(false, false);
        assert_ne!(handle, next);
        assert_eq!(close_handle(next), 1);
    }

    #[test]
    fn initial_state_and_nonzero_bool_arguments() {
        let manual = create_event_w(null(), -1, -1, null());
        assert_ne!(manual, 0);
        assert_eq!(wait_for_single_object(manual, 0), WAIT_OBJECT_0);
        assert_eq!(wait_for_single_object(manual, 0), WAIT_OBJECT_0);
        assert_eq!(reset_event(manual), 1);
        assert_eq!(wait_for_single_object(manual, 0), WAIT_TIMEOUT);
        assert_eq!(close_handle(manual), 1);

        let auto = event(false, true);
        assert_eq!(wait_for_single_object(auto, 0), WAIT_OBJECT_0);
        assert_eq!(wait_for_single_object(auto, 0), WAIT_TIMEOUT);
        assert_eq!(close_handle(auto), 1);
    }

    #[test]
    fn manual_reset_releases_all_even_if_immediately_reset() {
        let handle = event(true, false);
        let object = lookup(handle).unwrap();
        let (tx, rx) = mpsc::channel();
        let threads: Vec<_> = (0..4).map(|_| {
            let tx = tx.clone();
            thread::spawn(move || tx.send(wait_for_single_object(handle, INFINITE)).unwrap())
        }).collect();
        registered(&object, threads.len());
        {
            let mut state = object.state.lock().unwrap();
            object.set_locked(&mut state);
            // Perform reset before any awakened waiter can reacquire the lock.
            state.signaled = false;
        }
        for _ in 0..threads.len() {
            assert_eq!(rx.recv_timeout(Duration::from_secs(5)).unwrap(), WAIT_OBJECT_0);
        }
        for thread in threads { thread.join().unwrap(); }
        assert_eq!(wait_for_single_object(handle, 0), WAIT_TIMEOUT);
        assert_eq!(set_event(handle), 1);
        assert_eq!(wait_for_single_object(handle, 0), WAIT_OBJECT_0);
        assert_eq!(wait_for_single_object(handle, 0), WAIT_OBJECT_0);
        assert_eq!(reset_event(handle), 1);
        assert_eq!(wait_for_single_object(handle, 0), WAIT_TIMEOUT);
        assert_eq!(close_handle(handle), 1);
    }

    #[test]
    fn auto_reset_grants_one_per_signal_and_survives_reset() {
        let handle = event(false, false);
        let object = lookup(handle).unwrap();
        let (tx, rx) = mpsc::channel();
        let threads: Vec<_> = (0..2).map(|_| {
            let tx = tx.clone();
            thread::spawn(move || tx.send(wait_for_single_object(handle, INFINITE)).unwrap())
        }).collect();
        registered(&object, 2);
        {
            let mut state = object.state.lock().unwrap();
            object.set_locked(&mut state);
            state.signaled = false;
            assert_eq!(state.waiters.iter().filter(|waiter| waiter.granted).count(), 1);
        }
        assert_eq!(rx.recv_timeout(Duration::from_secs(5)).unwrap(), WAIT_OBJECT_0);
        assert_eq!(set_event(handle), 1);
        assert_eq!(rx.recv_timeout(Duration::from_secs(5)).unwrap(), WAIT_OBJECT_0);
        for thread in threads { thread.join().unwrap(); }
        assert_eq!(wait_for_single_object(handle, 0), WAIT_TIMEOUT);
        assert_eq!(set_event(handle), 1);
        assert_eq!(set_event(handle), 1);
        assert_eq!(wait_for_single_object(handle, 0), WAIT_OBJECT_0);
        assert_eq!(wait_for_single_object(handle, 0), WAIT_TIMEOUT);
        assert_eq!(close_handle(handle), 1);
    }

    #[test]
    fn finite_timeout_removes_waiter_and_does_not_consume_later_signal() {
        let handle = event(false, false);
        let start = Instant::now();
        assert_eq!(wait_for_single_object(handle, 20), WAIT_TIMEOUT);
        assert!(start.elapsed() >= Duration::from_millis(20));
        assert_eq!(set_event(handle), 1);
        assert_eq!(wait_for_single_object(handle, 0), WAIT_OBJECT_0);
        assert_eq!(close_handle(handle), 1);
    }

    #[test]
    fn close_keeps_inflight_wait_and_signal_alive_without_handle_reuse() {
        let handle = event(false, false);
        let object = lookup(handle).unwrap();
        let (tx, rx) = mpsc::channel();
        let waiter = thread::spawn(move || tx.send(wait_for_single_object(handle, INFINITE)).unwrap());
        registered(&object, 1);
        // Represents SetEvent having already acquired its reference before close.
        let inflight_signal = lookup(handle).unwrap();
        assert_eq!(close_handle(handle), 1);
        assert_eq!(wait_for_single_object(handle, 0), WAIT_FAILED);
        assert_eq!(set_event(handle), 0);
        let next = event(false, false);
        assert_ne!(next, handle);
        inflight_signal.set();
        assert_eq!(rx.recv_timeout(Duration::from_secs(5)).unwrap(), WAIT_OBJECT_0);
        waiter.join().unwrap();
        assert_eq!(close_handle(next), 1);
    }
}
