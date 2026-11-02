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
| `crates/qf-vault` | DPAPI vault |
| `crates/qf-adapter` | Everything that knows Claude's on-disk layout: locks, login swap, sessions, policy, canary, lanes, history |
| `crates/qf-provider` | Usage and profile endpoints, poll schedule |
| `crates/qf-engine` | The daemon: state, transitions, adoption, recovery, journal, HTTP API |
| `crates/quayflow` | The `quayflow.exe` CLI and the `quayflow-bg.exe` launcher |
| `mod/quayflow` | The Claude Code Mod (TypeScript) |
| `shell/` | Opt-in PowerShell prompt hook |
| `installer/` | `install.ps1` |
| `tests/e2e` | Fake-environment smoke test and owner-run real-login scenarios |
| `docs/` | Requirements, technical spec, user guide, security notes, acceptance |
| `assets/brand/` | Logo, app icon and palette ([usage](assets/brand/README.md)) |

## Development setup

- Windows 10/11, Rust 1.98 (pinned in `rust-toolchain.toml`), Node 20+ for the Mod, Claude Code 2.1.316 or later.
- With Visual Studio Build Tools: `cargo build`. Without them, use the GNU toolchain locally: `$env:RUSTUP_TOOLCHAIN = "1.98.0-x86_64-pc-windows-gnu"` (releases are always built with MSVC in CI).

```powershell
cargo fmt --all
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
pwsh -File tests/e2e/smoke-fake-env.ps1        # real quayflow.exe against a fake environment
claude plugin validate mod/quayflow
claude plugin test mod/quayflow
```

## Pull requests

1. Open an issue first for anything beyond a small fix, so the approach can be agreed.
2. Keep changes focused: one concern per pull request.
3. Add or update tests. Policy changes need table tests in `qf-policy`; engine behavior needs a scenario in `crates/qf-engine/tests/engine.rs`.
4. Update `docs/tech-spec.md` when you change an API route, a file format, a default or a CLI command, and `CHANGELOG.md` under *Unreleased*.
5. Commit messages follow [Conventional Commits](https://www.conventionalcommits.org/) (`feat:`, `fix:`, `docs:`, `test:`, `refactor:`, `ci:`).
6. CI must be green.

## Licensing

By contributing you agree that your contribution is licensed under the MIT License of this repository. Do not copy code from other projects unless its license allows it and the notice is kept.
