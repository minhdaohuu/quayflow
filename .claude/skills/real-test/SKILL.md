---
name: real-test
description: Run Quayflow's real-login scenarios through the user (real-switching, real-headless, real-lanes, real-macos-keychain), then record the results in the acceptance docs. Use when a change needs real-login evidence, or the user wants to test with their accounts.
---

# Real-login tests

The real-login scripts switch the user's live Claude login. Claude never runs them (`.claude/hooks/guard.py` blocks it); the user runs them with `! <command>` and pastes the output.

1. **Install what is being tested.** A local build: `tests/e2e/README.md`, "One-time setup", for this OS. A published release: `quayflow update` (this also tests an update from a published release).
2. **Preconditions.** `quayflow status`: at least two accounts in the group, the expected one live; `quayflow consent live-canary on` for adoption checks. For `real-headless.ps1`, one account must be at a real usage limit (`-Exhausted <label>`).
3. **Give the exact commands**, one case at a time when debugging:
   - `pwsh -File tests/e2e/real-switching.ps1 [-Case H,S,R1,R2]`
   - `pwsh -File tests/e2e/real-headless.ps1 -Exhausted <label> [-Case X1,X5]`
   - `pwsh -File tests/e2e/real-lanes.ps1 [-Account <label>]`
   - macOS, without Quayflow: `python3 tests/e2e/real-macos-keychain.py --auto`
4. **Read the pasted table.** On a FAIL, find the cause in the code and the logs (`quayflow report`, the data folder's `logs/`) before changing anything; never adjust a script to pass.
5. **After the run**, ask the user to confirm the original account is live and no pin is left (`quayflow status`).
6. **Record** in `docs/acceptance.md`: the platform section (date, OS, Claude Code version, build: a local commit or a release), each script's result and the timings it printed, and the AC rows the run covers. Update `docs/known-limits.md` ("not yet run"), the README platform table and the user guide's untested list. Run `python3 tests/docs/check-docs.py`.
