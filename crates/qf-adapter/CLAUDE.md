# qf-adapter: rules

This crate writes Claude Code's live login. A bug here destroys or leaks a user's login.

- **Every external call handles every outcome.** `security`, `gh`, a lock, a file read or write: success, not found, non-zero exit, timeout, garbage output. An error is never treated as "not there" (`if let Ok(..)` that falls through to the file store, `.ok().flatten()`, `unwrap_or_default()` on a read). Return it.
- **No `let _ =` on a credential path.** If a cleanup can fail, keep what is needed to retry it (the folder, the item name) instead of losing the login where nothing can find it.
- **A write is verified, and rolled back when it does not read back.** Report `PreWrite` only when the state is known to be unchanged; otherwise `PostWrite`.
- **Test the outside world's real output.** `security -w` prints hex when a byte is not printable ASCII; fixtures include non-ASCII. New Keychain behavior gets a case in `login.rs` `keychain_faults` (fake `security`, `tests-support/fake-security.py`; add a fault there when needed).
- **Never the user's Keychain item.** Only the real `~/.claude` maps to the bare `Claude Code-credentials`; tests use throwaway hashed items or the fake. Run tests with `QUAYFLOW_KEYCHAIN=off` unless the work is on the Keychain.
- Secrets go on stdin, never argv; errors never echo a token or the hex payload.
