//! Compatibility adapter: everything that knows how Claude Code keeps its login.
//! The client behavior this relies on is observed, not documented (docs/tech-spec.md §3); the static
//! canary in [`canary`] checks the markers per Claude version.

pub mod canary;
pub mod env;
pub mod history;
pub mod lanes;
pub mod lock;
pub mod login;
pub mod policy;
pub mod process;
pub mod sessions;

pub use env::Env;
pub use lock::{HeldLocks, LockError};
pub use login::{read_identity, read_login, swap, swap_with, SwapError, SwapOutcome};
