<div align="center">

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="assets/brand/quayflow-logo-dark.png">
  <img src="assets/brand/quayflow-logo-light.png" alt="Quayflow" width="360">
</picture>

**Change your port. Keep your flow.**

Switch between your approved Claude accounts without leaving your Claude Code session.

[![CI](https://github.com/minhdaohuu/quayflow/actions/workflows/ci.yml/badge.svg)](https://github.com/minhdaohuu/quayflow/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/minhdaohuu/quayflow?include_prereleases&sort=semver)](https://github.com/minhdaohuu/quayflow/releases)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
![Platform: Windows](https://img.shields.io/badge/platform-Windows%2010%2F11%20x64-0078D4)

[Features](#features) · [How it works](#how-it-works) · [Install](#install) · [Quick start](#quick-start) · [Documentation](#documentation) · [Roadmap](#roadmap)

</div>

> [!IMPORTANT]
> **You are responsible for complying with your provider's terms and your organization's rules**, including rules about which accounts may process your organization's work. Quayflow is an independent open-source project and is **not affiliated with or endorsed by Anthropic**.

**Status:** v0.1 private pilot. Quayflow targets every platform Claude Code runs on; **Windows is the first release (MVP)**, with macOS and Linux to follow. Official builds stay in the pilot until v0.1 passes its [acceptance scenarios](docs/acceptance.md).

## What and why

If you use more than one Claude subscription, for example a work seat and a personal plan, each has its own usage windows. When one runs low, the usual fix is to sign out, sign in again, and restart your session, which breaks your flow.

Quayflow does that switch for you, in place. The running session continues on the next account at its next request, with the same conversation and process, and nothing is replayed. It also makes the switch safe to automate:

- **You decide which accounts may serve which projects.** Groups are explicit data approvals; mixing organization and personal accounts needs a one-time disclosure.
- **It switches only when it should.** It acts on fresh usage readings; stale data never counts as zero. Suggest mode is the default; Automatic is opt-in per group.
- **It never touches your work.** No killed tools, no lost input, no replayed actions, and at most one retry of a request that hit a confirmed usage limit.
- **Your normal commands stay.** You keep typing `claude` and `claude -p`. There is no wrapper or launcher, and no change to how commands resolve.

## Features

| Feature | What it does |
|---|---|
| **In-place switching** | The running session moves to the next account at its next request: same conversation, same process, about 300 ms under Claude's own credential locks. |
| **Usage-aware** | Watches each account's five-hour, seven-day and per-model windows. Suggests (or, in Automatic mode, makes) a switch near the threshold, and returns to your Preferred account when it has room. |
| **Groups as approvals** | You decide which accounts may serve which projects. Organization-managed login rules are detected and respected. |
| **Hard-limit recovery** | On a confirmed usage limit: one switch and at most one retry of that request. Concurrent failures share one switch. |
| **Verified, not assumed** | Every switch is checked against the account that each client's next request actually used. A mismatch turns in-place switching off for that Claude version. |
| **Lanes** | Keep one session on a different account in a separate environment that still links your shared Claude setup. |
| **Headless-safe** | `claude -p` output is unchanged and never prompts. Without Quayflow running, Claude behaves exactly as before. |
| **Local only** | Logins are sealed with the operating system's per-user protection. No telemetry; logs are scrubbed. |

## How it works

```mermaid
flowchart LR
    subgraph CC["Claude Code (unmodified)"]
        direction TB
        S1["claude<br/>interactive session"]
        S2["claude -p<br/>headless job"]
        MOD["Quayflow Mod<br/>/quay pane · status line<br/>limit detection · one retry"]
        S1 --- MOD
        S2 --- MOD
    end

    ENG["quayflow engine<br/>groups · usage · policy<br/>journal · reconciliation"]
    VAULT[("Encrypted vault<br/>saved logins")]
    USAGE[["Claude usage API"]]
    ENV[("Claude environment<br/>~/.claude or a lane")]

    MOD <-->|"loopback API<br/>per-run token"| ENG
    ENG <--> VAULT
    ENG -->|"reads usage windows"| USAGE
    ENG ==>|"atomic login swap<br/>under Claude's locks"| ENV
    S1 -.->|"login read at<br/>next request"| ENV
    S2 -.-> ENV
```

**A switch, step by step**

```mermaid
sequenceDiagram
    autonumber
    actor You
    participant C as Claude session
    participant M as Quayflow Mod
    participant E as quayflow engine
    participant F as Login files

    E->>E: Account A reaches 85% of its binding window
    alt Suggest mode (default)
        E-->>M: Candidate: account B (same group)
        M-->>You: Status line shows the suggestion
        You->>M: /quay accept
        M->>E: Switch to B
    else Automatic mode (approved per group)
        E->>E: Pick B by eligibility, then remaining allowance
    end
    Note over C: A request in flight finishes on A
    E->>F: Swap A → B under Claude's credential locks
    C->>F: Next request reads login B
    M->>E: Report the identity the request used
    E-->>M: Adoption verified ✓
```

**When a request hits a usage limit**

```mermaid
flowchart LR
    R["Request fails"] --> Q{"Confirmed<br/>usage limit?"}
    Q -- "no" --> P["Pass the error through<br/>unchanged"]
    Q -- "yes" --> S["One switch to an eligible account<br/>(shared by concurrent failures)"]
    S --> T["Retry the same request once,<br/>in the same process"]
    T --> D{"Succeeded?"}
    D -- "yes" --> OK["Session continues"]
    D -- "no" --> W["Claude's own usage-limit<br/>behavior takes over"]
```

1. You register your logins and put them in **groups**.
2. The engine watches each account's usage windows (five-hour, seven-day, per model).
3. Near the threshold (85% by default) it **suggests** an account from the same group, or switches by itself in **Automatic** mode.
4. The switch replaces only the login entries of the Claude environment, under Claude's own locks, in about 300 ms. Requests in flight finish on the old account.
5. The Mod confirms which account each session's next request actually used.
6. Sessions that need a different account from the rest can run in a **lane**, a separate environment that links your shared Claude setup.

## Install

Windows 10/11 x64 with Claude Code installed. No administrator rights. In PowerShell:

```powershell
irm https://github.com/minhdaohuu/quayflow/releases/latest/download/install.ps1 | iex
```

The installer shows what it will do and asks for consent. It verifies `quayflow.exe` against the release's `SHA256SUMS`, and its GitHub build-provenance attestation when the GitHub CLI is available, and refuses on a mismatch. It then installs to `%LOCALAPPDATA%\Quayflow\bin` and adds the Mod to Claude Code from the same release. Running it again upgrades.

<details>
<summary><b>Safer: download, verify, read, then run</b></summary>

```powershell
$v = 'v0.1.0'   # the release you want
$base = "https://github.com/minhdaohuu/quayflow/releases/download/$v"
Invoke-WebRequest -UseBasicParsing "$base/install.ps1" -OutFile install.ps1
Invoke-WebRequest -UseBasicParsing "$base/SHA256SUMS" -OutFile SHA256SUMS
(Get-FileHash install.ps1 -Algorithm SHA256).Hash.ToLower()   # compare with the install.ps1 line in SHA256SUMS
gh attestation verify install.ps1 --repo minhdaohuu/quayflow    # optional, needs the GitHub CLI
notepad install.ps1                                              # read it
powershell -ExecutionPolicy Bypass -File .\install.ps1 -Version $v
```

See [Verifying a release](docs/verify-release.md).
</details>

<details>
<summary><b>Installer options</b></summary>

| Option | Effect |
|---|---|
| `-Version v0.1.0` | A specific release instead of the latest |
| `-Startup` | Also start the engine at Windows sign-in |
| `-NoPath` | Do not add the install folder to your user PATH |
| `-NoPlugin` | Engine only; do not touch Claude Code plugins |
| `-Uninstall [-DeleteData]` | Remove Quayflow, optionally with its vault and logs |

With `irm | iex`, pass options like this: `& ([scriptblock]::Create((irm <url>))) -Startup`.

The installer adds the Mod for you, pinned to the engine's release. To add only the Mod by hand (it does nothing without `quayflow.exe`): `/plugin install quayflow --marketplace minhdaohuu/quayflow`.
</details>

## Quick start

1. Start Claude as usual: `claude`.
2. Run `/quay`, give the consents, and **register** the login you are using. It joins the `Default` group.
3. **Add** another account. A window opens with Claude's own sign-in; Quayflow never asks for a token.
4. Watch the status line: `Quayflow · suggest · <account> <usage>`. When it suggests a switch, run `/quay accept`.
5. Optional:
   - `quayflow mode automatic --group Default --approve` lets Quayflow switch by itself. Unattended `claude -p` jobs need this.
   - `quayflow shell install` lets a session be replaced in the same PowerShell terminal.

Everything is also available from the command line: `quayflow status`, `quayflow use <account>`, `quayflow group ...`. See the [user guide](docs/user-guide.md).

## Safety and disclaimer

- **Terms and organization rules are your responsibility.** Quayflow is not affiliated with or endorsed by Anthropic.
- **A switch moves every session of an environment** (by default `~/.claude`), each at its next request. Use lanes to keep a session on a different account.
- **Saved logins** are encrypted with Windows DPAPI for your user. This does not protect against malware running as you. Quayflow never refreshes tokens; Claude does, and Quayflow saves the result. Logins never go into logs, reports or the Mod, and there is no telemetry.
- **Organization-managed login enforcement is respected.** The managed-login override setting is recorded but has no bypass path in this build. See [Security notes](docs/security.md#managed-login-enforcement-and-the-override-setting).
- **Releases are not Authenticode-signed.** They are reproducible, hashed and provenance-attested. Smart App Control and signed-only company policies may block them. See [Verifying a release](docs/verify-release.md).
- **Idle accounts go stale.** Quayflow never refreshes tokens, so an account left idle for some hours can no longer be observed. It stops being a candidate for automatic switching until it is used again; manual switching still works.
- **Prompt cache is per account.** The first request after a switch reprocesses the conversation.

Read [Known limits](docs/known-limits.md) before relying on Quayflow for unattended work.

## Documentation

| For users | |
|---|---|
| [User guide](docs/user-guide.md) | Install, onboarding, groups, modes, lanes, recovery, PowerShell integration, update, uninstall, troubleshooting |
| [Security notes](docs/security.md) | What Quayflow touches, the vault and its limits, logs, loopback API, managed-login setting |
| [Verifying a release](docs/verify-release.md) | Hashes, attestations, reproducible builds, SmartScreen and company policies |
| [Known limits](docs/known-limits.md) | Scope, undocumented dependencies, open release conditions, recovery procedures |

| For contributors | |
|---|---|
| [Requirements](docs/requirements.md) | Functional requirements (FR-xx) and release conditions |
| [Technical specification](docs/tech-spec.md) | Paths, schemas, API v1, CLI, Claude environment facts |
| [Acceptance results](docs/acceptance.md) | Scenario results against the product build |
| [Changelog](CHANGELOG.md) | Release history |

## Building from source

```powershell
cargo build --release --locked -p quayflow   # toolchain pinned by rust-toolchain.toml (1.98.0, MSVC)
cargo test --workspace --locked
pwsh -File tests/e2e/smoke-fake-env.ps1 -Exe target\release\quayflow.exe   # fake environment, no real logins
```

Without Visual Studio Build Tools, set `$env:RUSTUP_TOOLCHAIN = '1.98.0-x86_64-pc-windows-gnu'` for local development. Releases are built by CI with MSVC.

<details>
<summary><b>Repository layout</b></summary>

| Path | Content |
|---|---|
| `crates/` | Rust workspace: engine, vault, adapter, policy, provider, CLI |
| `mod/quayflow/` | The Mod (TypeScript Claude Code plugin) |
| `shell/` | PowerShell integration (`Quayflow.psm1`) |
| `installer/` | `install.ps1` |
| `tests/e2e/` | Fake-environment smoke test; owner-run real-login scenarios |
| `.github/workflows/` | CI and the reproducible release workflow |

</details>

## Roadmap

- **v0.1 (now, MVP):** Windows 10/11 x64 private pilot. Release once every [acceptance scenario](docs/acceptance.md) passes against real logins.
- **Next:** macOS and Linux, each with the platform's own credential protection (Keychain, Secret Service) and shell integration; public release with build attestations.

Have a use case that is not covered? [Open an issue](https://github.com/minhdaohuu/quayflow/issues/new/choose).

## Contributing

Contributions are welcome. Please read [CONTRIBUTING.md](CONTRIBUTING.md) and the [Code of Conduct](CODE_OF_CONDUCT.md) first. Quayflow handles login files, so changes are held to a high bar for safety and honest evidence. Report security problems privately as described in [SECURITY.md](SECURITY.md), not in public issues.

## License

[MIT](LICENSE).

"Claude" and "Claude Code" are trademarks of Anthropic, PBC, used here only to describe compatibility.
