//! Single-instance guard. Opening Murmur while it already runs must not
//! start a second tray icon, keyboard hook and engine set: the second launch
//! only asks the first one to show its window, then exits.
//!
//! Two per-user (`Local\`) kernel objects, both named from the Tauri
//! identifier:
//! * a mutex: whoever creates it is the primary instance;
//! * an auto-reset "show" event: a second launch sets it and the primary's
//!   listener thread shows Settings.
//!
//! Every launch opens the event *before* it claims the mutex, so the event
//! always exists by the time a second launch looks for it (no startup race).
//! A set event stays set until the primary waits on it, so a launch that
//! lands while the primary is still starting is not lost.
//!
//! The decision logic sits behind [`Os`] and is unit-tested without Windows;
//! the Win32 calls are in [`win`].

/// `identifier` in `tauri.conf.json` (a test keeps the two in step).
pub const APP_ID: &str = "dev.flowclone.windows";

/// The mutex that marks the primary instance (per user session).
pub fn mutex_name(id: &str) -> String {
    format!("Local\\Murmur-{id}")
}

/// The event a second launch sets to bring the primary's window up.
pub fn event_name(id: &str) -> String {
    format!("Local\\Murmur-{id}-show")
}

/// Result of claiming the instance mutex.
pub enum Claim<L> {
    /// This process created it: it is the primary instance.
    First(L),
    /// Another process already holds it.
    Taken,
}

/// The OS operations the guard needs.
pub trait Os {
    /// Held for the life of the primary (released by the OS on exit).
    type Lock;
    /// The show event, opened or created.
    type Signal;
    fn open_signal(&self, name: &str) -> Result<Self::Signal, String>;
    fn claim(&self, name: &str) -> Result<Claim<Self::Lock>, String>;
    /// Ask the primary to show its window.
    fn raise(&self, signal: &Self::Signal) -> Result<(), String>;
}

/// What this process should do.
pub enum Startup<L, S> {
    /// Run the app. `signal` (when present) is the event to listen on; both
    /// fields are `None` only when the OS calls failed, in which case the app
    /// runs unguarded rather than not at all.
    Primary { lock: Option<L>, signal: Option<S> },
    /// Another instance runs and has been asked to show itself: exit now.
    Secondary,
}

/// Decide whether this process is the primary instance; a secondary one has
/// already signalled the primary when this returns.
pub fn start<O: Os>(os: &O, id: &str) -> Startup<O::Lock, O::Signal> {
    let signal = match os.open_signal(&event_name(id)) {
        Ok(s) => Some(s),
        Err(e) => {
            eprintln!("[instance] show event unavailable: {e}");
            None
        }
    };
    match os.claim(&mutex_name(id)) {
        Ok(Claim::First(lock)) => Startup::Primary {
            lock: Some(lock),
            signal,
        },
        Ok(Claim::Taken) => {
            match &signal {
                Some(s) => {
                    if let Err(e) = os.raise(s) {
                        eprintln!("[instance] could not signal the running instance: {e}");
                    }
                }
                None => eprintln!("[instance] already running; cannot ask it to show"),
            }
            Startup::Secondary
        }
        Err(e) => {
            eprintln!("[instance] guard unavailable, running unguarded: {e}");
            Startup::Primary { lock: None, signal }
        }
    }
}

#[cfg(windows)]
pub mod win {
    use super::{Claim, Os, Startup};
    use windows::core::HSTRING;
    use windows::Win32::Foundation::{GetLastError, ERROR_ALREADY_EXISTS, HANDLE, WAIT_OBJECT_0};
    use windows::Win32::System::Threading::{
        CreateEventW, CreateMutexW, SetEvent, WaitForSingleObject, INFINITE,
    };
    use windows::Win32::UI::WindowsAndMessaging::{AllowSetForegroundWindow, ASFW_ANY};

    /// A kernel handle kept for the life of the process (never closed: the
    /// OS releases it on exit, which is what frees the mutex).
    pub struct Handle(HANDLE);
    // SAFETY: a kernel object handle is process-wide, usable from any thread.
    unsafe impl Send for Handle {}

    impl Handle {
        /// The raw handle (a method, so closures capture the whole `Send`
        /// wrapper rather than the raw pointer inside it).
        fn raw(&self) -> HANDLE {
            self.0
        }
    }

    pub struct Win32;

    impl Os for Win32 {
        type Lock = Handle;
        type Signal = Handle;

        fn open_signal(&self, name: &str) -> Result<Handle, String> {
            // Auto-reset, initially clear; opens the existing one if present.
            unsafe { CreateEventW(None, false, false, &HSTRING::from(name)) }
                .map(Handle)
                .map_err(|e| e.to_string())
        }

        fn claim(&self, name: &str) -> Result<Claim<Handle>, String> {
            let h = unsafe { CreateMutexW(None, false, &HSTRING::from(name)) }
                .map_err(|e| e.to_string())?;
            if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
                Ok(Claim::Taken)
            } else {
                Ok(Claim::First(Handle(h)))
            }
        }

        fn raise(&self, signal: &Handle) -> Result<(), String> {
            // This launch owns the foreground (the user just started it);
            // let the primary take it so its window really comes to front.
            let _ = unsafe { AllowSetForegroundWindow(ASFW_ANY) };
            unsafe { SetEvent(signal.raw()) }.map_err(|e| e.to_string())
        }
    }

    /// The running app's side of the guard.
    pub struct Primary {
        signal: Option<Handle>,
    }

    /// Claim the guard; a second instance signals the first and exits here,
    /// before anything (hook, tray, engines) starts.
    pub fn claim_or_exit() -> Primary {
        match super::start(&Win32, super::APP_ID) {
            // `Handle` never closes itself, so dropping the lock keeps the
            // mutex held until the process exits.
            Startup::Primary { lock: _, signal } => Primary { signal },
            Startup::Secondary => std::process::exit(0),
        }
    }

    impl Primary {
        /// Call `show` (on a background thread) each time another launch
        /// asks this instance to come forward.
        pub fn listen(self, show: impl Fn() + Send + 'static) {
            let Some(signal) = self.signal else { return };
            let spawned = std::thread::Builder::new()
                .name("instance-listen".into())
                .spawn(move || loop {
                    if unsafe { WaitForSingleObject(signal.raw(), INFINITE) } != WAIT_OBJECT_0 {
                        eprintln!("[instance] show-event wait failed; listener stopped");
                        return;
                    }
                    show();
                });
            if let Err(e) = spawned {
                eprintln!("[instance] could not start the listener: {e}");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::{Cell, RefCell};
    use std::collections::HashSet;

    /// One fake session: named objects that exist, and how often each event
    /// was set.
    #[derive(Default)]
    struct FakeOs {
        mutexes: RefCell<HashSet<String>>,
        raised: RefCell<Vec<String>>,
        fail_signal: Cell<bool>,
        fail_claim: Cell<bool>,
        fail_raise: Cell<bool>,
    }

    impl Os for FakeOs {
        type Lock = String;
        type Signal = String;
        fn open_signal(&self, name: &str) -> Result<String, String> {
            if self.fail_signal.get() {
                return Err("denied".into());
            }
            Ok(name.to_string())
        }
        fn claim(&self, name: &str) -> Result<Claim<String>, String> {
            if self.fail_claim.get() {
                return Err("denied".into());
            }
            if self.mutexes.borrow_mut().insert(name.to_string()) {
                Ok(Claim::First(name.to_string()))
            } else {
                Ok(Claim::Taken)
            }
        }
        fn raise(&self, signal: &String) -> Result<(), String> {
            self.raised.borrow_mut().push(signal.clone());
            if self.fail_raise.get() {
                return Err("gone".into());
            }
            Ok(())
        }
    }

    fn is_primary<L, S>(s: &Startup<L, S>) -> bool {
        matches!(s, Startup::Primary { .. })
    }

    #[test]
    fn names_are_per_user_and_distinct() {
        assert_eq!(
            mutex_name("dev.flowclone.windows"),
            "Local\\Murmur-dev.flowclone.windows"
        );
        assert_eq!(
            event_name("dev.flowclone.windows"),
            "Local\\Murmur-dev.flowclone.windows-show"
        );
        assert_ne!(mutex_name(APP_ID), event_name(APP_ID));
    }

    #[test]
    fn app_id_matches_tauri_identifier() {
        let conf = include_str!("../tauri.conf.json");
        assert!(
            conf.contains(&format!("\"identifier\": \"{APP_ID}\"")),
            "APP_ID must equal tauri.conf.json's identifier"
        );
    }

    #[test]
    fn first_launch_is_primary_and_listens_without_raising() {
        let os = FakeOs::default();
        match start(&os, APP_ID) {
            Startup::Primary { lock, signal } => {
                assert_eq!(lock.as_deref(), Some(mutex_name(APP_ID).as_str()));
                assert_eq!(signal.as_deref(), Some(event_name(APP_ID).as_str()));
            }
            Startup::Secondary => panic!("first launch must be primary"),
        }
        assert!(os.raised.borrow().is_empty());
    }

    #[test]
    fn second_launch_raises_the_first_and_exits() {
        let os = FakeOs::default();
        assert!(is_primary(&start(&os, APP_ID)));
        assert!(matches!(start(&os, APP_ID), Startup::Secondary));
        assert_eq!(*os.raised.borrow(), vec![event_name(APP_ID)]);
        // A third launch raises again; still only one primary.
        assert!(matches!(start(&os, APP_ID), Startup::Secondary));
        assert_eq!(os.raised.borrow().len(), 2);
    }

    #[test]
    fn other_identifiers_do_not_collide() {
        let os = FakeOs::default();
        assert!(is_primary(&start(&os, APP_ID)));
        assert!(is_primary(&start(&os, "dev.flowclone.other")));
        assert!(os.raised.borrow().is_empty());
    }

    #[test]
    fn second_launch_exits_even_if_it_cannot_signal() {
        let os = FakeOs::default();
        assert!(is_primary(&start(&os, APP_ID)));
        os.fail_raise.set(true);
        assert!(matches!(start(&os, APP_ID), Startup::Secondary));
        os.fail_raise.set(false);
        os.fail_signal.set(true);
        assert!(matches!(start(&os, APP_ID), Startup::Secondary));
        // No event handle: nothing to raise.
        assert_eq!(os.raised.borrow().len(), 1);
    }

    #[test]
    fn guard_failure_runs_unguarded_rather_than_not_at_all() {
        let os = FakeOs::default();
        os.fail_claim.set(true);
        match start(&os, APP_ID) {
            Startup::Primary { lock, signal } => {
                assert!(lock.is_none());
                assert!(signal.is_some());
            }
            Startup::Secondary => panic!("must still run"),
        }
    }

    #[test]
    fn primary_without_show_event_still_holds_the_lock() {
        let os = FakeOs::default();
        os.fail_signal.set(true);
        match start(&os, APP_ID) {
            Startup::Primary { lock, signal } => {
                assert!(lock.is_some());
                assert!(signal.is_none());
            }
            Startup::Secondary => panic!("first launch must be primary"),
        }
    }
}
