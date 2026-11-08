# Quayflow: working notes for Claude

Quayflow switches a Claude Code environment between the user's approved Claude accounts in place: running sessions continue on the new account at their next request. It watches usage, suggests or makes switches, recovers a request that hit a usage limit (one switch, one retry), and is built for unattended `claude -p` jobs. It ships for Windows x64 and Linux x86_64; macOS (Keychain) is built for the next release: its Keychain switch is tested with real logins, Quayflow end to end is not yet (never call it Windows-only).

Read `CHANGELOG.md` first for what changed recently; it is kept accurate.

## Layout

| Path | What |
|---|---|
| `crates/qf-model` | Shared types: config (`Tuning` defaults), API v1 request/response structs, time helpers |
| `crates/qf-policy` | Pure decisions: eligibility and its explanations, proactive switch, recovery target, effective mode |
| `crates/qf-provider` | Usage and profile API, poll `Schedule` (backoff, slow pace after a 429), idle-token refresh |
| `crates/qf-vault` | Vault of saved logins: DPAPI on Windows, owner-only files on Linux (`OwnerOnly`); `atomic_write` keeps file modes |
| `crates/qf-adapter` | Claude environment files: read/write login under Claude's own locks |
| `crates/qf-engine` | The engine: state, observe loop, transitions (switches), recovery, journal, HTTP API (`http.rs` routes) |
| `crates/quayflow` | CLI `quayflow.exe` (`main.rs` HELP lists every command) and `quayflow-bg.exe`; `last.rs` = `quayflow last` |
| `mod/quayflow/hooks` | The Mod (Claude Code plugin, TypeScript): `register.tsx` hooks, `step.ts` request hook and recovery, `engine.ts` link/handshake, `watcher.ts` event long-poll and status line, `commands.ts` `/quay`, `pane.tsx`/`flows.tsx`/`review.tsx` the pane, `ui.tsx` design kit, `chart.ts` Raster graphics |
| `shell/Quayflow.psm1` | PowerShell prompt hook for same-terminal Replace |
| `installer/install.ps1`, `install.sh` | Installers (hash + attestation checks, Mod install, `-Update`/`--update` for `quayflow update`); `test-install.ps1`, `test-install.sh` |
| `tests/e2e` | Fake-environment suites (CI) and owner-run real-login scripts |
| `docs/` | User guide, tech spec, requirements, acceptance, security, known limits, verify-release |

## Invariants (do not break)

- **Headless output is never changed.** In a headless run (`claude -p`, SDK) the Mod prints, asks and toasts nothing; stdout, stderr and the exit code are Claude's own. When the engine declines a retry, `onStep` returns Claude's result untouched.
- **Recovery:** only on a confirmed usage limit; one switch and at most one retry per failed request; concurrent failures share one switch; cancellation is never recovered; nothing is replayed. When every approved account is at its limit: switch to the soonest-available one, no retry.
- **A switch moves a whole environment** (all sessions sharing that config folder), each at its next request. The destination must be allowed by the group of every running session; the strictest mode among them applies.
- **Stale is never zero.** A usage reading past its freshness limit is shown as unknown and never counts as room.
- **The live login is Claude's to refresh.** The engine refreshes only an idle account's saved copy, under `cred_lock` (lock order: `cred_lock`, then the engine lock), and skips it when any environment's login files hold the same refresh token.
- **Secrets:** tokens never reach logs, the journal, reports, the Mod or state views. Logs and the journal name accounts by id only; labels are scrubbed. CLI output may show labels.
- **Groups are data approvals.** Automatic mode needs a one-time explicit approval per group; mixed identities need the disclosure.
- **Engine API is loopback-only with a per-run token**; the Mod speaks protocol 1 (`PROTOCOL`), engine accepts current and previous.

## Build and test

Rust (from PowerShell; on a machine without MSVC Build Tools set `$env:RUSTUP_TOOLCHAIN = '1.98.0-x86_64-pc-windows-gnu'`):

```powershell
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cargo build --locked -p quayflow          # debug exe used by the e2e suites
```

Mod:

```powershell
npx -y -p typescript@5.6.3 tsc -p mod/quayflow --noEmit
claude plugin validate mod/quayflow
claude plugin test mod/quayflow           # tests in mod/quayflow/tests, harness in harness.ts
```

The first type check needs `mod/quayflow/.claude-plugin/types/`, which Claude Code writes beside a Mod it loads (gitignored). Without it, put Claude Code's declaration file at `mod/host-types/claude-code-2.1.316.d.ts` (never committed) and run `npx -y -p typescript@5.6.3 tsc -p mod/tsconfig.ci.json`, as CI does.

Fake-environment end-to-end (safe anywhere, run in CI): `pwsh -File tests/e2e/smoke-fake-env.ps1`, `cli-fake-env.ps1`, `sim-usage.ps1` (fake usage API `tests/sim/fake_api.py`, `QUAYFLOW_SIM_API`). Also `shell/test-Quayflow.ps1` and `installer/test-install.ps1`.

Linux (CI job `linux`, or a Linux machine): `cargo clippy`/`cargo test` as above, `sh installer/test-install.sh`, `sh tests/e2e/smoke-unix.sh target/debug/quayflow`, `shellcheck -S warning installer/*.sh tests/e2e/*.sh`, `claude plugin test mod/quayflow`. macOS (CI job `macos`): the same, plus the Keychain backend (`qf-adapter/src/keychain.rs`, `login.rs` `Store`). On a Mac with your real login, run tests with `QUAYFLOW_KEYCHAIN=off` unless you are working on the Keychain: an environment that resolves to the bare `Claude Code-credentials` item is your live login (only your real `~/.claude` may resolve to it). Platform code is behind `cfg(windows)` / `cfg(not(windows))`: Linux paths and facts are in `qf-adapter` (`process.rs`, `env.rs`, `lock.rs`, `lanes.rs`, `policy.rs`), `qf-engine/src/paths.rs`, `qf-vault` and `quayflow/src/local.rs`; the Mod picks the layout in `hooks/engine.ts` (`layout`).

Real-login scripts (`tests/e2e/real-*.ps1`) switch the user's live login. Never run them; give the user the command and read the results they paste.

## Mod notes

- Tests mount the pane with `$.ui.mount`, press keys with `ui.press({key})`, type with `ui.input({key, text, kind?})`; `kind` defaults to submit (Enter). The hooks module under test is a separate instance: test files cannot reach its `Q` state.
- The terminal draws `Raster` cells (no Svg): base64 of little-endian u32 triplets `[codePoint, fg 0x00RRGGBB, bg]`, `0x01000000` = terminal default. Other surfaces fall back to text gauges (`canPaint`).
- Button hotkeys are 0-9 or a-z. Explanations go through `Hint` (shown with Hints on, key `h`); rules the person must see before committing always show.
- Pane state lives in `$.state` atoms per session; module variables (`Q`) reset on reload. A resume or `/clear` changes the session id without `session.start` (handled in `session.end`).

## Conventions

- Plain, precise English in docs, UI text and commits; short sentences; no marketing, no emoji. Name things the way the user guide does (environment, lane, group, Suggest/Manual/Automatic, live account).
- Every user-visible change gets a `CHANGELOG.md` line under `[Unreleased]`, written for users.
- Version lives in: `Cargo.toml` (workspace), `Cargo.lock`, `mod/quayflow/.claude-plugin/plugin.json`, `mod/quayflow/hooks/context.ts` (`MOD_VERSION`), `mod/quayflow/README.md`, and the `modVersion` expectation in `mod/quayflow/tests/quayflow.test.ts`.
- Releases: CI must pass on `main`, then an annotated tag `vX.Y.Z` runs `.github/workflows/release.yml` (reproducible double build, assets `install.ps1`, `quayflow.exe`, `quayflow-bg.exe`, `Quayflow.psm1`, `SHA256SUMS`, `SOURCE_COMMIT`).
