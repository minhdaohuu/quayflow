# Changelog

All notable changes to this project are documented here. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project uses [Semantic Versioning](https://semver.org/).

## [Unreleased]

## [0.3.0] - 2026-11-09

Clearer wording in the `/quay` pane, `/quay` commands, the status line and the CLI; the Uninstall screen can delete all data; `quayflow policy` and `quayflow group list` print readable values.

### Changed
- Clearer wording across the `/quay` pane, `/quay` commands, the status line, the CLI and engine messages: one name for each thing (switch, environment, "usage unknown"), mode names capitalised (Manual, Suggest, Automatic), and errors that say what to do next. Examples in the docs and screenshots use the accounts `work-01` and `work-02`.
- The Uninstall screen in `/quay` can choose to delete all Quayflow data (`--delete-data`), not only the vault and the logs.
- On a group page, **Switch home** opens the switch review for the group's home account. Before, the button was called "Return home" and only opened the Overview. The Overview's button to the group page is now called **Group** instead of "Home".

### Fixed
- `quayflow policy` printed raw JSON and Rust debug text for the managed-login rules; it now prints the sources, organizations and login method plainly.
- `quayflow group list` showed the home account's id instead of its name.

## [0.2.0] - 2026-11-08

Fixes from a full code review, including an attestation check of the installer that `quayflow update` runs. From this release the version number follows the changes (see CONTRIBUTING.md): Linux (0.1.5) and macOS (0.1.6) would each have been a minor version.

### Fixed
- `quayflow update` now checks the attestation of the release's installer before running it. Before, it checked the installer only against `SHA256SUMS` from the same release, which does not prove who built it.
- When every approved account was at its limit, recovery could switch to an account whose last usage reading was out of date and showed room, as if it were available now. An out-of-date reading now never counts as room there either.
- A `quayflow use` right after the login changed outside Quayflow could be refused with "already live". The engine now reads the login files before refusing.
- Two failures at the same moment could leave the second one without its retry, when the first one's switch landed between the second one's check and its own switch request. The second now shares the first one's switch.
- A session that the engine had not listed yet (a new `claude -p` job, for example in a lane) could have the default environment recovered instead of its own.
- Inconsistent login files (one account's tokens, another account's record) were forgotten when the engine restarted. They are now found again at start, and automation stays paused until a switch repairs them.
- macOS: a Keychain login that contains a non-ASCII character (in an MCP login, for example) could not be read, so switching stopped in that environment. `security` prints such a login as hex; Quayflow now reads it.
- macOS: when the Keychain did not answer, deleting a lane, or saving or cancelling a sign-in, could leave a login behind in an item nothing could find again. A lane is now not deleted, and a sign-in folder is kept and retired later.
- macOS: a Keychain write that did not read back as written is now rolled back, as a failed account-record write already was.
- `quayflow lanes check` no longer deletes a lane's own `settings.json` or `CLAUDE.md` when the shared one is gone; it moves the lane's copy aside (`.lane-copy`).
- Unknown options are refused. Before, a misspelled `--check` ran a full `quayflow update`, and a misspelled `--if-consented` started the engine without checking consent.
- `curl ... | sh -s -- --uninstall` asks its question on the terminal; before, the uninstall read its answer from the piped script.
- In `/quay add`, a "Save as new" choice no longer carries over to the next sign-in. Cancelling during a recovery retry is no longer reported as "the retry could not run". The headless hint of `/quay group remove` named `group add`.
- Linux and macOS: the uninstall question, the startup consent and the `/quay` privacy line no longer say Windows; `quayflow update` suggests `brew install gh` on macOS.
- Config files with a zero `backoffMaxS`, or a freshness limit shorter than the poll interval, are refused instead of re-polling a rate-limited endpoint on every tick or showing every reading as out of date.
- Logs and the journal replace longer account labels first, so no part of a label like "Work Org" is left when another account is called "Work".

### Changed
- On Linux and macOS, `quayflow shell install` now refuses with a message instead of writing a PowerShell prompt hook that cannot work there. Same-terminal Replace is Windows only; use Open New Session.

## [0.1.6] - 2026-11-08

macOS (Apple silicon and Intel; not fully tested yet: see Known limits); adding an account signs in first.

### Added
- macOS (Apple silicon and Intel). Switching, lanes and recovery from a simulated usage limit passed with two real accounts on an Apple silicon Mac; a real usage limit, Open New Session, Intel Macs and an install or update from a published release are not tested yet. Install with the same `install.sh` as on Linux; it installs `quayflow-macos-arm64` or `quayflow-macos-x86_64` to `~/.local/share/quayflow/bin`, linked as `~/.local/bin/quayflow`, and `--startup` adds a LaunchAgent. On macOS Claude Code keeps its login in the Keychain: a switch writes the environment's Keychain item through `/usr/bin/security` (the login never appears on a command line), keeps the item's other entries such as MCP logins, reads it back, and briefly leaves an owner-only copy in `.credentials.json` so running sessions use the new account from their next request; Quayflow removes that copy about 35 seconds later. Sign-in and Open New Session open a Terminal window. Saved logins are owner-only files, as on Linux. A session started with another spelling of `CLAUDE_CONFIG_DIR` (it then reads another Keychain item and cannot follow a switch) gets a notice naming the value to use, and does not turn in-place switching off for its Claude version. Not yet read on macOS: login policy set only through an MDM configuration profile (Claude Code still enforces it).
- Releases carry `quayflow-macos-arm64` and `quayflow-macos-x86_64`, built twice and compared like the other programs.

### Changed
- Adding an account (`/quay add`) now signs in first and names the account after: pick the group, choose **Sign in with Claude**, and save with a name suggested from the account's e-mail address. Before, it asked for a name first, and when nothing was registered yet it only flashed a short warning about the missing vault consent, so it looked stuck.
- The credential storage consent is asked once, the first time you save a login, instead of a box on every save. Problems on the register and add screens (a name in use, the box not ticked) now stay on the screen instead of a passing message. Registering suggests a name from the e-mail address too, and Enter on the name saves.
- `quayflow onboard` and the `/quay` welcome screen describe the vault on macOS too.
- A switch also waits for Claude Code's `.storage-write.lock`, which Claude holds while it saves its login, on every platform. On Linux and macOS the legacy lock is named after the resolved folder, as Claude names it.

## [0.1.5] - 2026-11-08

Linux x86_64 support (not fully tested yet: see Known limits); the repository is public and releases are attested; a quiet status line.

### Added
- Linux (x86_64; glibc 2.35 and OpenSSL 3 or newer, such as Ubuntu 22.04, Debian 12, Fedora 36 and later). Install with `curl -fsSL https://github.com/minhdaohuu/quayflow/releases/latest/download/install.sh | sh`; `install.sh` makes the same checks as `install.ps1` (hashes, attestation, the Mod's source commit) and takes the same choices as long options. The program goes to `~/.local/share/quayflow/bin`, linked as `~/.local/bin/quayflow`; `--startup` adds a systemd user unit; `quayflow update` hands off to `install.sh --update`. Saved logins are owner-only files (`0600`), not encrypted, the same protection Claude Code gives its own login on Linux. Same-terminal Replace is not available on Linux yet; Open New Session uses the desktop's terminal.
- Releases carry `quayflow-linux-x86_64` and `install.sh`, built twice and compared like the Windows programs.
- `quayflow shell refresh` rewrites an installed PowerShell prompt hook module from the running version.

### Changed
- The repository is public. Releases from v0.1.5 on carry build-provenance attestations, so the installer's default checks pass with a signed-in GitHub CLI; v0.1.0 to v0.1.4 stay unattested and need `-SkipAttestation`. The documentation no longer describes a private pilot.
- When the GitHub CLI is missing or signed out, the installer and `quayflow update` say how to install it and sign in (`winget install --id GitHub.cli -e`, `gh auth login`), and what `-SkipAttestation` gives up.
- The README, user guide and installer say to use only accounts that are yours to use.
- The Quayflow status line no longer looks like a warning all the time. In the terminal, the everyday line (usage, home or away, a pin, a switch under way) is drawn dim at the end of the hint line under the prompt; only a line that needs you (a suggestion to accept, a sign-in, a paused automation, a problem, the engine off) is pinned, where Claude Code draws it as a warning.
- In the terminal, a status change could cancel a read the `/quay` pane was waiting for, so a screen such as Register showed "not readable (engine unreachable (ui.render: superseded))". The status line no longer redraws the pane, and a cancelled read is asked again.
- Linux: adding an account or Open New Session opened no window when the engine had started without a desktop (over SSH, or from systemd). The Mod now tells the engine which desktop its session runs on, and the command shown when no window can open is written for bash, not PowerShell.
- Linux: the sign-in window's link could open in LibreOffice instead of the browser (the window lacked the desktop's `XDG_DATA_DIRS`, where a snap browser is listed). The desktop's own variables are now passed on. The window also stays open after `claude auth login` and says whether the sign-in finished.
- After `quayflow stop` from a terminal, the status line said "engine not running" instead of "engine stopped by you (/quay start)" in sessions that were already open.
- An update from a local build says the Mod was left as it is, instead of "Skipped the Claude Code plugin (-NoPlugin)".
- A per-model weekly limit is written "7d Fable" in reasons, `quayflow status` and `quayflow last`, instead of the internal key "7d:Fable".
- A switch is told once in the transcript: the extra "Session … now runs on … (verified on its request)" note is gone. The status line shows the account; a session that does not adopt a switch still gets a notice.
- `quayflow group tuning` refusing a threshold at or below the margin now names both values and says to pass `--margin` with a lower value.
- `quayflow lanes list` names the lane's live account by its label instead of its id.

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
- `/quay` pane follows the design's information order: Overview with [Home], [Live] and [Recommended] marks and entry points to Running sessions, Session actions and Limits and recovery; "Why this account?" with every candidate as Eligible, Excluded or Unknown, its reason and observation age, and the threshold, margin and cooldown; the home state (Home account, Home ready, Home ready, return deferred, Away from home) with Pin current account.
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

[Unreleased]: https://github.com/minhdaohuu/quayflow/compare/v0.3.0...HEAD
[0.3.0]: https://github.com/minhdaohuu/quayflow/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/minhdaohuu/quayflow/compare/v0.1.6...v0.2.0
[0.1.6]: https://github.com/minhdaohuu/quayflow/compare/v0.1.5...v0.1.6
[0.1.5]: https://github.com/minhdaohuu/quayflow/compare/v0.1.4...v0.1.5
[0.1.4]: https://github.com/minhdaohuu/quayflow/compare/v0.1.3...v0.1.4
[0.1.3]: https://github.com/minhdaohuu/quayflow/compare/v0.1.2...v0.1.3
[0.1.2]: https://github.com/minhdaohuu/quayflow/compare/v0.1.1...v0.1.2
[0.1.1]: https://github.com/minhdaohuu/quayflow/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/minhdaohuu/quayflow/releases/tag/v0.1.0
