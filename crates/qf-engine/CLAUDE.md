# qf-engine: rules

- **New in-memory state: saved to disk, or a restart test.** Anything kept only in `Inner` (holds, markers, caches that change behavior) is lost when the engine restarts. Use `restart(&w, e)` in `tests/engine.rs` to show it is found again.
- **Tests run the engine as production does.** The engine notices a login change at its next `tick()`, not when a test calls `watch()`. New scenarios drive `e.tick()`; a check-then-act on state another thread updates needs a test with two callers (see `simultaneous_failures_share_one_switch_and_both_retry`).
- **Assume a second caller arrives between your check and your act.** Recovery, switches and the observe loop run concurrently: re-check after taking the lock, and treat a "noop" or "conflict" refusal as possibly someone else's success.
- **The invariants apply in every branch**, fallbacks included: stale is never zero, one switch and at most one retry, the strictest mode. `qf-policy` checks them in generated situations (`invariants.rs`); a new decision belongs there.
- **Lock order:** `cred_lock`, then the engine lock. No blocking I/O, network or binary scans while the engine lock is held.
- Logs and the journal name accounts by id; labels are scrubbed, longer labels first.
