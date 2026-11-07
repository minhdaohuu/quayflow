# Contributing to Quayflow

Thanks for your interest. Quayflow touches Claude Code's login files, so changes are held to a high bar for safety and honesty. Please read this before opening a pull request.

## Ground rules

- **No bypass of restrictions.** Quayflow never works around an organization's or the client's controls. Pull requests that weaken managed-login enforcement, the credential-selector restriction, consent gates or group permissions are declined.
- **No secrets in logs, journals, reports or tests.** Use opaque ids (12-hex identity hashes). Never paste real tokens, emails or transcripts into issues.
- **Never test against real logins in CI.** Unit and fake-environment tests only. Real-login scripts live in `tests/e2e/` and are run by maintainers on their own machines.
- **Mark evidence honestly.** A mock never proves client adoption. Do not mark an acceptance scenario as passed unless it ran against the product build (`docs/acceptance.md`).

## Repository layout

| Path | What |
|---|---|
| `crates/qf-model` | Shared types and the loopback API v1 schema |
| `crates/qf-policy` | Pure decision logic (eligibility, ranking, pin, return home, recovery target) |
| `crates/qf-vault` | Vault of saved logins (DPAPI on Windows, owner-only files on Linux) |
| `crates/qf-adapter` | Everything that knows Claude's on-disk layout: locks, login swap, sessions, policy, canary, lanes, history |
| `crates/qf-provider` | Usage and profile endpoints, idle-account token refresh, poll schedule and backoff |
| `crates/qf-engine` | The daemon: state, transitions, adoption, recovery, journal, HTTP API |
| `crates/quayflow` | The `quayflow.exe` CLI (including `quayflow last`) and the `quayflow-bg.exe` launcher |
| `mod/quayflow` | The Claude Code Mod (TypeScript) |
| `shell/` | Opt-in PowerShell prompt hook and its self-test |
| `installer/` | `install.ps1`, `install.sh` and their sandbox tests |
| `tests/e2e` | Fake-environment smoke, CLI and simulation tests; owner-run real-login scenarios (`real-switching.ps1`, `real-lanes.ps1`, `real-headless.ps1`) |
| `tests/sim` | Loopback fake of the usage, profile and token API used by the simulation test |
| `docs/` | Requirements, technical spec, user guide, security notes, known limits, release verification, acceptance |
| `assets/brand/` | Logo, app icon and palette ([usage](assets/brand/README.md)) |

## Development setup

Quayflow targets all platforms Claude Code runs on; it ships for Windows x64 and Linux x86_64, macOS is next. Keep platform-specific code (vault, paths and locks, shell integration, installer) behind its module boundary so the policy, engine and Mod stay shared. See [Platform scope](docs/requirements.md#platform-scope).

- Windows 10/11 or Linux (Ubuntu 22.04 or later), Rust 1.98 (pinned in `rust-toolchain.toml`), Node 20+ for the Mod (CI uses Node 24), Claude Code 2.1.316 or later, PowerShell 7 and Windows PowerShell 5.1 for the shell and installer tests.
- With Visual Studio Build Tools: `cargo build`. Without them, use the GNU toolchain locally: `$env:RUSTUP_TOOLCHAIN = "1.98.0-x86_64-pc-windows-gnu"` (releases are always built with MSVC in CI).

```powershell
cargo fmt --all
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
pwsh -File tests/e2e/smoke-fake-env.ps1        # real quayflow.exe against a fake environment
pwsh -File tests/e2e/cli-fake-env.ps1          # CLI behavior and error paths, fake environment
pwsh -File tests/e2e/sim-usage.ps1            # scripted usage from a fake usage API (needs Python 3)
pwsh -File shell/test-Quayflow.ps1            # prompt hook, PowerShell 7 and 5.1
# On Linux:
sh installer/test-install.sh                   # install.sh in a sandbox
sh tests/e2e/smoke-linux.sh                    # real program, fake environment: install, engine, update, uninstall
pwsh -File installer/test-install.ps1         # installer in a sandbox, PowerShell 7 and 5.1
claude plugin validate --strict .
claude plugin validate --strict mod/quayflow
claude plugin test mod/quayflow
```

The fake-environment scripts take `-Exe <path>` to test a particular build (CI passes `target\debug\quayflow.exe`). The `tests/e2e/real-*.ps1` scripts switch your real Claude login; never run them in CI.

## Pull requests

1. Open an issue first for anything beyond a small fix, so the approach can be agreed.
2. Keep changes focused: one concern per pull request.
3. Add or update tests. Policy changes need table tests in `qf-policy`; engine behavior needs a scenario in `crates/qf-engine/tests/engine.rs`.
4. Update `docs/tech-spec.md` when you change an API route, a file format, a default or a CLI command, `docs/known-limits.md` when a limit appears or goes away, and `CHANGELOG.md` under *Unreleased*.
5. Commit messages follow [Conventional Commits](https://www.conventionalcommits.org/) (`feat:`, `fix:`, `docs:`, `test:`, `refactor:`, `ci:`).
6. CI must be green.

## Licensing

By contributing you agree that your contribution is licensed under the MIT License of this repository. Do not copy code from other projects unless its license allows it and the notice is kept.
