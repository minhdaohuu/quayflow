---
name: review
description: Full code review of Quayflow against its invariants and error paths, with six read-only reviewer agents and a verifier. Use before every release, after a large change, or when asked to review the project or an area of it.
---

# Quayflow review

A review finds what tests and earlier reviews missed. It follows the same procedure every time, so no area depends on how the request was phrased.

## 1. Scope

- Whole project (before a release): all six areas below.
- An area or a diff: only the matching agents; for a diff, give them `git diff <base>` and the files it touches.

## 2. Run the reviewers in parallel (one message, read-only)

| Agent | Area |
|---|---|
| `adapter-reviewer` | `crates/qf-adapter`: Claude's login files, Keychain, locks, lanes, processes |
| `engine-reviewer` | `crates/qf-engine`: switches, recovery, observe loop, HTTP API, journal |
| `core-reviewer` | `crates/qf-vault`, `qf-provider`, `qf-policy`, `qf-model` |
| `cli-installer-reviewer` | `crates/quayflow`, `installer/`, `shell/Quayflow.psm1` |
| `mod-reviewer` | `mod/quayflow/hooks`, `types/index.d.ts` against `qf-model/src/api.rs` |
| `ci-reviewer` | `.github/workflows`, `tests/e2e`, `tests/sim`, installer tests |

Each returns at most 12 findings, ranked, with file:line, a concrete scenario and a fix.

## 3. Verify before reporting

For every high or medium finding, read the code path yourself, or send the finding to the `verifier` agent with the file:line (not your conclusion). Check external behavior for real when it is safe (for example `security` output with a throwaway item, never the user's). Drop what does not hold; mark what stays uncertain.

## 4. Fix or hand over

- Fix the confirmed ones that are contained, each with a test that fails without the fix (revert briefly to see it fail). Follow the folder's `CLAUDE.md`.
- Hand the rest to the user in a table: finding, why it is held (needs a design choice, touches the real login, cannot be tested here).
- Every user-visible fix gets a CHANGELOG line. Run all checks; push only when asked.

## 5. Improve the harness

For each confirmed bug, ask why the tests missed it, and add the missing kind of test (a rule in `qf-policy/src/invariants.rs`, a fault in `qf-adapter/tests-support/fake-security.py`, an engine test with `tick()`, `restart()` or two callers, a negative installer case), or a rule to the folder's `CLAUDE.md`.
