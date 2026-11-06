# Changelog

All notable changes to this project are documented here. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project uses [Semantic Versioning](https://semver.org/).

## [Unreleased]

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

[Unreleased]: https://github.com/minhdaohuu/quayflow/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/minhdaohuu/quayflow/releases/tag/v0.1.0
