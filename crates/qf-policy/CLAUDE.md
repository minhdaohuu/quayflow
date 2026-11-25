# qf-policy: rules

- Pure decisions, no I/O. Every decision function is covered by `invariants.rs`, which generates 20,000 situations and checks the rules of CLAUDE.md (stale is never zero, destination allowed by every governing group, strictest mode, soonest only from an exhausted window with a known reset).
- A new decision or branch: add its rule to `check()` there, not only example tests. A failure prints its seed; `QF_INVARIANT_SEED=<n> cargo test -p qf-policy invariants` replays it.
- A fallback never reuses a reading that the main path would refuse.
- Every target gate is below `hard_limit_pct`, whatever the group's threshold and margin allow (config accepts threshold 100, margin 0). The generator covers threshold 100 and a lowered hard limit; keep configuration corners in it, not only defaults.
