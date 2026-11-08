# Contributing to Quayflow

Thanks for your interest. Quayflow touches Claude Code's login files, so changes are held to a high bar for safety and honesty. Please read this before opening a pull request.

## Ground rules

- **No bypass of restrictions.** Quayflow never works around an organization's or the client's controls. Pull requests that weaken managed-login enforcement, the credential-selector restriction, consent gates or group permissions are declined.
- **No secrets in logs, journals, reports or tests.** Use opaque ids (12-hex identity hashes). Never paste real tokens, emails or transcripts into issues.
- **Never test against real logins in CI.** Unit and fake-environment tests only. Real-login scripts live in `tests/e2e/` and are run by maintainers on their own machines.
- **Mark evidence honestly.** A mock never proves client adoption. Do not mark an acceptance scenario as passed without recorded real evidence in `docs/acceptance.md`: date, script, case and build (a local build or a release).

## Repository layout

| Path | What |
|---|---|
| `crates/qf-model` | Shared types and the loopback API v1 schema |
| `crates/qf-policy` | Pure decision logic (eligibility, ranking, pin, return home, recovery target) |
| `crates/qf-vault` | Vault of saved logins (DPAPI on Windows, owner-only files on Linux and macOS) |
| `crates/qf-adapter` | Everything that knows Claude's on-disk layout: locks, login swap, sessions, policy, canary, lanes, history |
| `crates/qf-provider` | Usage and profile endpoints, idle-account token refresh, poll schedule and backoff |
| `crates/qf-engine` | The daemon: state, transitions, adoption, recovery, journal, HTTP API |
| `crates/quayflow` | The `quayflow` CLI (`quayflow.exe` on Windows; including `quayflow last`) and the Windows `quayflow-bg.exe` launcher |
| `mod/quayflow` | The Claude Code Mod (TypeScript) |
| `shell/` | Opt-in PowerShell prompt hook and its self-test |
| `installer/` | `install.ps1`, `install.sh` and their sandbox tests |
| `tests/e2e` | Fake-environment smoke, CLI and simulation tests; owner-run real-login scenarios (`real-switching.ps1`, `real-lanes.ps1`, `real-headless.ps1`) |
| `tests/sim` | Loopback fake of the usage, profile and token API used by the simulation test |
| `docs/` | Requirements, technical spec, user guide, security notes, known limits, release verification, acceptance |
| `assets/brand/` | Logo, app icon and palette ([usage](assets/brand/README.md)) |

## Development setup

Quayflow targets all platforms Claude Code runs on; it ships for Windows x64, Linux x86_64 and, from v0.1.6, macOS (Keychain, `qf-adapter/src/keychain.rs`). Keep platform-specific code (vault, paths and locks, shell integration, installer) behind its module boundary so the policy, engine and Mod stay shared. See [Platform scope](docs/requirements.md#platform-scope).

Every platform needs Rust 1.98 (pinned in `rust-toolchain.toml`), Node 20 or later for the Mod (CI uses Node 24), Claude Code 2.1.316 or later (for `claude plugin test`; no sign-in needed) and Python 3.

- **Windows 10/11:** PowerShell 7 and Windows PowerShell 5.1 (shell and installer tests). With Visual Studio Build Tools, `cargo build` works as is. Without them, use the GNU toolchain locally: `$env:RUSTUP_TOOLCHAIN = "1.98.0-x86_64-pc-windows-gnu"` (releases are always built with MSVC in CI).
- **Linux (Ubuntu 22.04 or later):** a C toolchain and OpenSSL headers (`build-essential`, `pkg-config`, `libssl-dev`), and ShellCheck. PowerShell 7 only if you run the real-login scripts.
- **macOS:** the Xcode Command Line Tools (`xcode-select --install`) and ShellCheck (`brew install shellcheck`). PowerShell 7 (`brew install powershell`) only if you run the real-login scripts.

On Windows:

```powershell
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cargo build --locked -p quayflow                # the e2e scripts use target\debug\quayflow.exe
pwsh -File tests/e2e/smoke-fake-env.ps1         # real quayflow.exe against a fake environment
pwsh -File tests/e2e/cli-fake-env.ps1           # CLI behavior and error paths, fake environment
pwsh -File tests/e2e/sim-usage.ps1              # scripted usage from a fake usage API
pwsh -File shell/test-Quayflow.ps1              # prompt hook, PowerShell 7 and 5.1
pwsh -File installer/test-install.ps1           # install.ps1 in a sandbox, PowerShell 7 and 5.1
```

On Linux and macOS:

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cargo build --locked -p quayflow
QF_SH=dash sh installer/test-install.sh         # install.sh in a sandbox, under dash (if installed)
QF_SH=bash bash installer/test-install.sh       # ... and under bash (macOS /bin/sh)
sh tests/e2e/smoke-unix.sh target/debug/quayflow    # real program, fake environment: install, engine, update, uninstall
shellcheck -S warning installer/*.sh tests/e2e/*.sh
```

On every platform, the Mod:

```sh
claude plugin validate --strict .
claude plugin validate --strict mod/quayflow
claude plugin test mod/quayflow
```

**Type check (optional).** It needs Claude Code's own declaration file, which belongs to Claude Code and is not in this repository. Copy it to `mod/host-types/claude-code-2.1.316.d.ts` (where to find it: [Mod README](mod/quayflow/README.md#check-type-check-and-test)), then run `npx -y -p typescript@5.6.3 tsc -p mod/tsconfig.ci.json`. The folder is gitignored; never commit the file. CI skips the type check with a warning when the file is absent.

**On a Mac with your own Claude login,** run the Rust tests with `QUAYFLOW_KEYCHAIN=off` unless you are working on the Keychain backend. The tests never use the item of your real login (only your real `~/.claude` maps to it) and create only throwaway items, but the switch keeps them out of the Keychain entirely. `smoke-unix.sh` sets it itself.

The fake-environment scripts take `-Exe <path>` to test a particular build. The `tests/e2e/real-*.ps1` scripts switch your real Claude login; never run them in CI.

## Pull requests

1. Open an issue first for anything beyond a small fix, so the approach can be agreed.
2. Keep changes focused: one concern per pull request.
3. Add or update tests. Policy changes need table tests in `qf-policy`; engine behavior needs a scenario in `crates/qf-engine/tests/engine.rs`.
4. Update `docs/tech-spec.md` when you change an API route, a file format, a default or a CLI command, `docs/known-limits.md` when a limit appears or goes away, and `CHANGELOG.md` under *Unreleased*.
5. Commit messages follow [Conventional Commits](https://www.conventionalcommits.org/) (`feat:`, `fix:`, `docs:`, `test:`, `refactor:`, `ci:`).
6. CI must be green.

## Licensing

By contributing you agree that your contribution is licensed under the MIT License of this repository. Do not copy code from other projects unless its license allows it and the notice is kept.
