# Changelog

All notable changes to this project are documented here. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project uses [Semantic Versioning](https://semver.org/).

## [Unreleased]

### Changed
- The repository is public. Releases from v0.1.5 on carry build-provenance attestations, so the installer's default checks pass with a signed-in GitHub CLI; v0.1.0 to v0.1.4 stay unattested and need `-SkipAttestation`. The documentation no longer describes a private pilot.

## [0.1.4] - 2026-11-07

`quayflow update` runs the release's own installer, with all its checks; wording fixes; documentation reviewed against the code.

### Fixed
- Messages about an account that needs a new sign-in pointed to "/quay accounts → Reauthenticate", which does not exist; they now say /quay → Accounts → Sign in again.
- A reading that is out of date is called "usage unknown" everywhere (`quayflow status`, `/quay status`, the pane); the Mod said "not current".
- A `/quay` command sent while the engine is still starting says "Quayflow is still connecting to the engine; try again in a moment" instead of "Quayflow is off".
- The build's integrity line no longer claims "provenance-attested": releases are not attested while the repository is private. It says to verify with the release's `SHA256SUMS`.
- An idle account whose token could not be refreshed yet is explained as such, not as "usage unknown until it is live".
- After an update, the PowerShell prompt hook module stayed at the old version. The installer and `quayflow update` now refresh it (`quayflow shell refresh`).
- `quayflow update` could leave the Mod uninstalled when adding the new release failed; it now puts the previous Mod back, as the installer does.

### Changed
- `quayflow update` makes the same checks as the installer: it downloads the release's `install.ps1`, checks it against `SHA256SUMS`, and runs it in a new update mode (`-Update`). Without a signed-in GitHub CLI it refuses, unless you add `--skip-attestation`. Your consents, PATH and startup setting are kept, and a Mod installed from a local folder is left as it is.
- `quayflow update --source <folder>` installs a local release folder (the engine only).
- Documentation reviewed against the code: README (a walkthrough with real outputs, interactive and headless, and a section on headless jobs), user guide, technical specification, requirements, acceptance results (real usage-limit and headless evidence of 2026-11-07), security notes, known limits, release verification, contributing guide, Mod and test READMEs.

## [0.1.3] - 2026-11-07

Headless jobs: `quayflow last` explains what Quayflow did for a `claude -p` run and when to run again. Tested with real logins and a real usage limit (42 headless checks).

### Added
- `quayflow last --session <id> [--json]`: what Quayflow did for one Claude session, for headless jobs: `recovered`, `all_exhausted` (switched to the account available soonest), `not_recovered` with the reason, or `no_limit`; the limit that was hit and its reset; and `availableAt`, the earliest time worth running again. Reads the journal, so it works with the engine stopped; exit code 3 when nothing is recorded. Recovery decisions now record the account that failed and, when every account is at its limit, when the soonest one is available.
- User guide: "Headless jobs (`claude -p`)": setup, what a job sees in each case, running again after a limit, and how interactive sessions are affected.
- `tests/e2e/real-headless.ps1`: case X5 (a run fails at the limit, `quayflow last` names a time to run again, a fresh `claude -p` succeeds on another account) and `quayflow last` checks in X1 and X3.

## [0.1.2] - 2026-11-07

Pane redesign with true-color graphics, readable idle accounts, connection and headless fixes. Tested end to end with real logins and a real usage limit, including headless runs. Upgrading is recommended.

### Added
- `tests/e2e/real-headless.ps1`: owner-run headless (`claude -p`) scenarios with a real exhausted account: Automatic recovery, parallel runs sharing one switch, Suggest leaving Claude's result, and the engine stopped; every run's stdout must be exactly Claude's JSON.
- README: screenshots of the `/quay` Overview and switch review, drawn from the Mod's own drawing code.
- `/quay` pane matches the v1.1 design atlas: Overview in the design's information order with [Home], [Live] and [Recommended] marks and entry points to Running sessions, Session actions and Limits and recovery; "Why this account?" with every candidate as Eligible, Excluded or Unknown, its reason and observation age, and the threshold, margin and cooldown; the home state (Home account, Home ready, Home ready, return deferred, Away from home) with Pin current account.
- Reviews before commitment: accepting a recommendation opens the compact Prepare view (checks, optional quiet moment, participants); switch progress shows the phases, a participant table during the idle wait and a per-client adoption table; adoption mismatch, return-home and `model_not_found` variants have their own wording and actions.
- Registration shows the live login's e-mail and organization for review (never stored or logged), can create a group inline, and asks for the storage box on every save. A duplicate identity offers the existing registration; a reauthentication that returned another identity can be registered as a new account, never over the old one. The mixed-identity disclosure names organizations and needs an explicit "I approve" box.
- Full-history Session actions (Open New Session preselected) with reviews that name destination, context, window and the one-executor rule; the shell is named before anything is committed, the PowerShell prompt hook is a separate opt-in, and the outcome shows the preparation steps or the preserved source on failure. Group conflicts show a participant table and a "move to lane" review; lane creation shows linked and private state and can store the lane's login.
- Limits and recovery panel (recovery checks, all-exhausted table with the selected soonest account, endpoint backoff and next check, Claude's native wait shown as unknown), observation detail per account, policy and plan changes, version and live-canary review, update steps, uninstall options, operation drill-down and a support report that is previewed, reviewed and only then saved.
- Settings: proactive cooldown and manual pin length can be changed (every group).
- Engine API: `GET /v1/explain`, `GET /v1/identity`, `GET /v1/handoff/check`, `GET /v1/operations`, `POST /v1/report/preview`, `POST /v1/report/save`, `POST /v1/pin`, `POST /v1/tuning`; `asNew` on `/v1/accounts/signin/confirm`; `pausedBy` on each environment.
- CLI: `quayflow uninstall --delete-vault` and `--delete-logs` (separate, off by default; `--delete-data` still deletes everything); `quayflow report --preview`. `/quay report` previews without saving (`--save` saves); `/quay open|replace <lane>` asks before preparing in an interactive session.

### Changed
- Account rows no longer say a bare "excluded": they say in a word or two why the account is not a switch target now ("no room to gain", "usage unknown", "at its limit", "not in this group", "group conflict", "blocked by your organization", "plan lacks this model"), and nothing for the live account or what a badge already says. The full reason stays in the account's detail and in "Why?".
- `/quay add` reworked: a three-step track (name and group, Claude's sign-in, review), the name field takes the keyboard when the screen opens and Enter in it starts the sign-in, a line under it confirms the name and its group as you type (or says the name is already in use, which is refused), and "+ New group" opens the group field only when wanted. Register and "register as new" name fields also take the keyboard and confirm Enter.
- Idle accounts stay readable: when an idle account's access token expires, the engine refreshes that account's saved copy (Claude's own token service and public client, the login's own scopes) and reads its usage with the new token, so it remains a candidate for suggestions, automatic switching and recovery. The live login is never refreshed by Quayflow. The refresh runs under the credential lock every switch holds, and only when no environment's login files hold the same refresh token; the new login is stored before it is used, and a switch reads the saved login again under that lock. A refused refresh token marks the account for a new sign-in.
- A stale usage reading is never shown as a current figure: `quayflow status` prints `usage unknown (last read … ago)`.
- `/quay` draws true-color graphics in the terminal: a fleet chart with every account's 5-hour and 7-day columns (colored by level, the threshold as a dashed line, ▼ over the live account and ★ over the recommended one, the names under it as buttons), gradient usage gauges with a threshold tick, and a step track for a switch. Other surfaces keep the text gauges. Every figure is also written out beside its graphic.
- `/quay` pane redesigned for scanning. The Overview shows a card for the live account with 5-hour and 7-day usage gauges (colored by the group threshold, with the threshold marked on each gauge, and a countdown to each reset), other accounts as rows with small gauges and LIVE / HOME badges, and the recommendation as a card with one primary action. A one-line status strip under the tabs shows the live account, mode, group and what this session last used. Explanations are hidden behind "? Hints" (hotkey `h`); rules you must see before committing, such as the one-executor rule and mixed-identity approval, always show. Steps, notices and labels use a consistent set of marks.
- Mod status line follows the design: `Quayflow · <account> · <mode> · <status>`, with the mode capitalized and one status chosen by priority (problem, unknown login or sign-out, switch in progress or pending next request, suggestion such as `5h 91% · 7d 40% | Personal suggested`, pin end, managed-login override, warning, stale usage, `Home account` / `Away from home`, plain usage). No fact is repeated, informational notices are left out, and the line fits the terminal width by shortening the status, never the account name. Off states are one line, such as `Quayflow · off · engine not running (Claude works as usual)`.
- Usage in the status line shows both windows, `5h 45% · 7d 30%`, plus the per-model weekly window (`· Opus 7d 88%`) when it applies to the session's model and is the fullest; on the suggestion, plain-usage and home/away statuses (`Home account · 5h 45% · 7d 30%`). Usage is the first thing dropped when the line is too long.
- When your own Claude Code status line already shows the 5-hour / 7-day limits, the Quayflow line leaves out its usage figures. Detected by reading the `statusLine` setting and the script it names (never running it); override with the plugin option `statusLineUsage` (`auto`, `show`, `hide`).

### Fixed
- `/quay` ignored an option it does not know, so a mistyped one (`--aprove`) silently had no effect; it is now refused with the option named and the nearest known one suggested.
- An account at its limit was explained as "100% 7d; needs 75% or less to be worth a switch"; it now reads "at its limit (100% 7d)" in /quay, `quayflow status` and "Why this account?".
- `/quay` could show "Quayflow is off: connecting to the engine" while the engine was running: after `/reload-plugins` the link was painted "connecting" over a working one, and a reload that did not run the session start left nothing to connect it. A reload now keeps the last link until the new handshake answers; the pane and every `/quay` command connect when no handshake is under way (never starting the engine) and start the event watcher if it is missing; the short wait reads "Connecting to the engine…", not "off".
- After resuming a conversation (or `/clear`) inside a running Claude, `/quay` stayed on "connecting to the engine": Claude goes on under a new session id without a session start, so the pane's values started over while the Mod was still linked. The Mod now carries the link and the last state over to the new session, says hello to the engine under the new id, and repaints a pane that shows "off" while it is linked.
- Opening Claude with no engine running (after a restart of the computer, say) waited about 8 s before starting the engine: the first handshake was tried three times, and Windows takes about 2 s to refuse each connection to a port nothing listens on. The handshake is now retried only when something answered or the wait ran out; a refused connection or a missing `engine.json` starts the engine at once.
- The live account's usage was read every minute even after the usage endpoint refused a read (its budget is per login and shared with Claude Code), so about every third read was refused and logged as a warning "rate-limited (retry after 0 s)". After a refusal the live account is now read every two minutes for an hour, a refresh from `/quay` waits for that pace and for any backoff instead of forcing a read, a single refusal is logged as information (only refusals in a row are warnings), and "retry after 0 s" is no longer shown.
- The managed-login exception was described as letting Quayflow switch to accounts the policy refuses; this build only records the setting and still enforces the policy, and the pane, flows and `/quay status` now say so and show the engine's own reply.
- "Resume automation" was offered for every pause cause and reported "Automation resumed." while automation stayed paused; it is now offered only for a pause by you or a hold, other causes show their remedy, and `/quay resume` says when automation is still paused.
- Engine messages pointed to a "Replace Session" action that does not exist under that name; they now point to the full-history fallback in /quay (Session actions).
- Engine notices, switches and adoptions (login changed outside Quayflow, signed out, switch back after `model_not_found`, the outcome of an interrupted switch after a restart) now reach the transcript: each once per session, in interactive sessions only, without replaying events from before the session started. Events are followed with the watcher's own cursor, so a `/quay` command's state read no longer skips them, and an engine restart is noticed.

## [0.1.1] - 2026-11-06

Fixes from a full project review. Upgrading is recommended.

### Fixed
- Switching: a failed `.claude.json` write rolls the credentials back. Mixed login files (one account's credentials under another's account record) are detected from the credentials themselves, held with automation paused, never stored under the wrong account, and repaired by the next switch. A failed capture of the outgoing login stops the switch. Switching to an account already live keeps Claude's fresher tokens instead of writing the older vault copy.
- One login can no longer be switched into two environments at once (in-flight switches count). Cancel either wins or is refused before the write.
- Engine: exclusive single-instance lock held for the engine's lifetime; torn journal lines repaired; reconcile really pauses automation when the login matches neither side; constant-time API token check; random-source failures stop start-up; duplicate-request cache keyed by method and path, with concurrent duplicates refused; session state pruned.
- Recovery treats the account whose limit was just confirmed as limited, and ignores other models' windows when choosing the soonest account.
- Managed policy: registry and `managed-settings.d` changes are noticed; a malformed `forceLoginOrgUUID` fails closed.
- Sessions whose PID was reused are no longer counted as live; stale lock directories are not stranded; a failed lane creation is cleaned up; BOM-prefixed `.claude.json` handled for lanes; lane delete keeps the login if a session started meanwhile; `remove_account` keeps memory and disk in step.
- CLI: `mode` and `migrate-poc` use the configured default group. Unsafe Claude version strings never name a file.
- Mod status line and pane refresh when new usage figures arrive (they kept showing the figures from the last switch or notice, marked stale).
- Mod: Welcome keeps consents given earlier; event batches are not lost when a state refresh fails; project assignment matching respects path boundaries; the simulated-limit hook only runs with `QUAYFLOW_SIM_LIMIT=1`.
- Shell hook: same-terminal Replace works with non-ASCII paths.
- Installer: attestation is required (signed-in GitHub CLI, release workflow, tag and commit) unless `-SkipAttestation` is passed; the Mod is checked against the release's `SOURCE_COMMIT`; a failed Mod upgrade restores the previous Mod; native command output parsed from stdout only; a hung old engine is stopped and reported truthfully.
- CI and release: actions pinned to commit SHAs, no persisted credentials, tag name passed through the environment, tool versions pinned.

## [0.1.0] - 2026-11-06

First release: a private pilot for Windows 10/11 x64 (Quayflow targets every platform Claude Code runs on; Windows is the first release).

### Added
- Rust engine `quayflow.exe` (one per Windows user, no window): loopback API v1 with per-run token and versioned handshake, durable journal, sanitized rotating logs, crash reconciliation.
- In-place account switching under Claude Code's own credential locks; adoption verified on each client's next request; per-version static check and gating.
- Groups as data approval with a one-time mixed-identity disclosure, Default group, project assignments, and conflict detection that names the blocking session.
- Suggest (default), Manual and Automatic modes; 85% threshold with margin and cooldown; manual pin; return home to the Preferred account.
- Hard-limit recovery: confirmed limits only, one switch and one retry per request, concurrent failures share one switch, soonest-available account when all are exhausted, plan-compatibility fallback.
- DPAPI vault with refresh capture, unknown-login protection and PoC import.
- Lanes with linked shared setup; Open New Session and same-terminal Replace (PowerShell 7 and 5.1) with full history.
- Claude Code Mod with `/quay` pane, status line and request hooks; installer; updater with hash and attestation checks; uninstall.
- Brand assets in `assets/brand/`: logo for light and dark backgrounds, harbor mark, application icon (embedded in both executables with product and version information), palette and social preview.
- Simulation mode for tests (`QUAYFLOW_SIM_API`, loopback only) with a fake usage API, plus fake-environment smoke, CLI and simulation suites in CI.

### Known limitations
- Real-login acceptance passed on a local build (`docs/acceptance.md`); a real usage limit end to end and Claude's own usage-limit wait adopting a switched login are still open release conditions.
- Release assets are not attested while the repository is private; install with `-SkipAttestation`.
- An idle account's access token expires, so it is not observed until it is used again (see `docs/known-limits.md`).

[Unreleased]: https://github.com/minhdaohuu/quayflow/compare/v0.1.4...HEAD
[0.1.4]: https://github.com/minhdaohuu/quayflow/compare/v0.1.3...v0.1.4
[0.1.3]: https://github.com/minhdaohuu/quayflow/compare/v0.1.2...v0.1.3
[0.1.2]: https://github.com/minhdaohuu/quayflow/compare/v0.1.1...v0.1.2
[0.1.1]: https://github.com/minhdaohuu/quayflow/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/minhdaohuu/quayflow/releases/tag/v0.1.0
