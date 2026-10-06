use crate::nt::{NTSTATUS, STATUS_ACCESS_DENIED};
use std::cell::Cell;
use std::io::Write;
use std::panic::{catch_unwind, AssertUnwindSafe};

thread_local! {
    static INSIDE: Cell<bool> = const { Cell::new(false) };
}

pub fn is_inside() -> bool {
    INSIDE.with(|i| i.get())
}

pub fn set_inside(val: bool) {
    INSIDE.with(|i| i.set(val));
}

/// Runs `f` with hooks passing straight through on this thread (our own file work must not be
/// redirected). Returns `None` when already inside, so the caller calls the original.
pub fn guarded<T>(f: impl FnOnce() -> T) -> Option<T> {
    if INSIDE.with(|i| i.replace(true)) {
        return None;
    }
    let r = f();
    INSIDE.with(|i| i.set(false));
    Some(r)
}

pub fn log(msg: impl AsRef<str>) {
    if let Some(cfg) = crate::maybe_cfg() {
        if let Some(path) = &cfg.log {
            if let Ok(mut f) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
            {
                let _ = writeln!(f, "[{}] {}", std::process::id(), msg.as_ref());
            }
        }
    }
}

/// Wraps an NT hook implementation to catch panics and return STATUS_ACCESS_DENIED instead
/// of unwinding across FFI into the game.
pub fn catch_hook_panic<F>(name: &'static str, f: F) -> NTSTATUS
where
    F: FnOnce() -> NTSTATUS,
{
    match catch_unwind(AssertUnwindSafe(f)) {
        Ok(status) => status,
        Err(err) => {
            let msg = if let Some(s) = err.downcast_ref::<&str>() {
                *s
            } else if let Some(s) = err.downcast_ref::<String>() {
                s.as_str()
            } else {
                "unknown panic"
            };
            log(format!("PANIC caught in hook {name}: {msg}"));
            STATUS_ACCESS_DENIED
        }
    }
}
