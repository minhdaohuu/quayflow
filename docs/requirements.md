# Quayflow requirements

The functional requirements Quayflow is built and tested against. Code comments, the technical specification and the acceptance table refer to these IDs (FR-xx; acceptance scenarios AC-xx are listed in [acceptance.md](acceptance.md)).

## Platform scope

Quayflow targets every platform Claude Code runs on: Windows, macOS and Linux. It ships for Windows 10/11 x64, Linux x86_64 and, from v0.1.6, macOS; the requirements below name Windows mechanisms (DPAPI, PowerShell, `%LOCALAPPDATA%`). Linux meets them with owner-only `0600` files for the vault (the same protection Claude Code gives its own login file), `~/.local/share/quayflow`, a systemd user unit and `install.sh`; same-terminal replacement is not available on Linux yet. macOS (Apple silicon and Intel, from v0.1.6) uses the Linux layout, owner-only vault files and `install.sh`, a LaunchAgent for sign-in startup, and Claude Code's own Keychain item for the live login ([tech spec §1b](tech-spec.md#1b-macos)); its vault is not in the Keychain yet, and same-terminal replacement is not available there either. Linux and macOS are not fully tested; see [Known limits](known-limits.md#platform-and-scope). Later platforms meet the same requirements with their own equivalents: OS credential protection for the vault, the platform's shell for same-terminal replacement and the platform's per-user data folder. New code should keep platform-specific parts behind the existing module boundaries (vault, adapter paths and locks, shell integration, installer) so that the policy, engine and Mod stay shared.

## Onboarding and accounts

| ID | Requirement |
|---|---|
| FR-01 | One-command install without administrator rights; the user keeps typing `claude`. Before anything is stored, explain roles, local credential handling, the shared-login scope and the modes, show the terms and organization-rules notice, and obtain explicit consent. A `Default` group exists from the start. |
| FR-02 | Register the current login without reauthentication: show the verified identity, a label and a mandatory group; registration never activates another account or enables Automatic mode. |
| FR-03 | Add another account through Claude's native sign-in in a separate temporary environment; group selection is mandatory; a cancelled sign-in leaves the live login unchanged; the temporary copy is retired without signing out. |
| FR-04 | Detect duplicates; reauthentication verifies identity; removal is blocked while an account is live; a rejected refresh token sets the account aside; an unknown live login pauses automation and is backed up in the vault's protected form before a manual switch; no plaintext fallback. |

## Groups and policy

| ID | Requirement |
|---|---|
| FR-05 | A trusted project assignment selects the group; otherwise the default group. Repository content cannot grant account permissions. |
| FR-06 | Group membership is the data approval. Switching inside a group needs no further confirmation; a group that mixes organization and personal accounts (or several organizations) needs a one-time disclosure. A switch must be permitted by every running session's group; otherwise it is refused and the blocking session is named, with a lane offered. |
| FR-06a | Managed login enforcement (`forceLoginOrgUUID`, `forceLoginMethod`) is detected from every managed-settings source and respected: non-matching accounts are not candidates. |

## Modes and selection

| ID | Requirement |
|---|---|
| FR-07 | Modes: Manual, Suggest (default) and Automatic (explicitly approved per group). Unattended headless jobs switch only in Automatic mode. A manual switch pins the account until unpinned, its window resets, or a period passes; near exhaustion normal behavior resumes. |
| FR-08 | Observe every account's 5-hour, 7-day and per-model windows on a per-account schedule with backoff, paced so that Quayflow does not use up the usage endpoint's per-login budget that Claude Code shares; keep observations with their age; stale or missing data never counts as zero. Default threshold 85% of the binding window. |
| FR-09 | Eligibility first (group, managed login, sessions, sign-in health, plan compatibility, freshness), then remaining allowance with optional ranks. Return to the Preferred (home) account when it has room again. Cooldown and margin apply. Explain choices with timestamps. |

## Switching

| ID | Requirement |
|---|---|
| FR-10 | A switch targets a credential environment and moves every session in it at its next request; list the sessions it affects. Lanes are optional separate environments that link the shared Claude setup and hold their own login. |
| FR-11 | Switch between requests: in-flight requests finish on the previous account; never suspend processes, kill tools, discard input or resend completed work. |
| FR-12 | One transition per environment with explicit states; before the write, cancellation or a held lock leaves the login unchanged; after the write, uncertainty is reconciled, never assumed. |
| FR-13 | Each running client's first request after a switch is verified against the destination identity; a mismatch is shown, recorded and turns in-place switching off for that Claude version. |

## Headless work and limits

| ID | Requirement |
|---|---|
| FR-14 | Quayflow never changes the stdout, stderr or exit code of `claude -p`; no prompts during jobs; if the Mod or engine is unavailable or mismatched, Claude behaves as without Quayflow. A job can ask afterwards what Quayflow did for its session and when to run again (`quayflow last`), also with the engine stopped. |
| FR-15 | Hard-limit recovery: only on a confirmed usage limit, one switch and at most one retry of the same request in the same process; concurrent failures share one switch; no replay of completed actions. |
| FR-16 | When every account is exhausted, switch to the account available soonest without a retry, and leave waiting to Claude's own usage-limit behavior; Claude's own result for that request stands. Quayflow does not hold requests. |

## Continuity

| ID | Requirement |
|---|---|
| FR-17 | Open New Session and Replace Current Session move a conversation between environments with its full history (per-file integrity checks, newer history never overwritten, source preserved); same-terminal replacement in PowerShell 7 and 5.1. |
| FR-18 | Never two executors of one task; a failed transfer preserves the source. |

## Engine, data and compatibility

| ID | Requirement |
|---|---|
| FR-19 | One engine per user, started on demand after consent, no windows except those the user asks for (sign-in, Open New Session), intentional stop respected, crash reconciliation without replay; matched engine and Mod updates; uninstall leaves the current login working. |
| FR-20 | Loopback API with a per-run token and a versioned handshake (current and previous protocol); an incompatible pair disables switching and recovery. Detect another tool switching the same login. |
| FR-21 | Credentials sealed with Windows DPAPI (current user plus application entropy); on Linux and macOS, owner-only files (on Linux the same protection as Claude Code's own login file; on macOS weaker than Claude Code's Keychain item); captures Claude's refreshes of live logins; refreshes only the saved copy of an idle account (held by the vault alone); no plaintext fallback. |
| FR-22 | Sanitized local logs (7 days, size cap) and a durable journal; no tokens, prompts, responses, emails or labels in records. |
| FR-23 | Support reports are local and shared only by the user; no telemetry. |
| FR-24 | Per-version compatibility: a static check of each Claude version, every real switch verified, the oldest running version governs; failures turn in-place switching off for that version. |
| FR-25 | Original code under MIT; engine in Rust as a single executable, Mod in TypeScript; reproducible builds with published hashes. |

## Release conditions for 1.0

- All acceptance scenarios pass against the product build ([acceptance.md](acceptance.md)).
- A real usage-limit failure is observed end to end, and Claude's own retry does not compete with Quayflow's.
- Claude's own usage-limit wait adopts a login switched during the wait.
- Claude reads and writes correctly through lane links.
- Crash reconciliation, version gating and dead-refresh-token handling pass on the product build.
- Every release artifact has a SHA-256 hash, a build attestation and a reproducible build; the installer and updater refuse a mismatch. Attestations exist from v0.1.5; v0.1.0 to v0.1.4 are not attested.
- The terms disclaimer is published.
