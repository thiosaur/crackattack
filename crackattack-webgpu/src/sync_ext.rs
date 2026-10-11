//! Single locking policy of the crate.
//!
//! The project's mutexes guard scalar registers or append-only streams. A panic
//! in a writer cannot leave a broken invariant there, so poisoning is ignored
//! instead of propagated.

use std::sync::{Mutex, MutexGuard};

/// Locks the mutex and ignores poisoning.
pub fn lock_ignore_poison<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}
