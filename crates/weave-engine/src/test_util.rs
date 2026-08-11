//! Shared test helpers (process-wide env serialization).

use std::sync::{Mutex, MutexGuard};

static WEAVE_HOME_LOCK: Mutex<()> = Mutex::new(());

/// Serialize tests that mutate `WEAVE_HOME`.
pub fn lock_weave_home() -> MutexGuard<'static, ()> {
    WEAVE_HOME_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}
