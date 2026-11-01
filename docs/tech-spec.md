# Quayflow v0.1 technical specification

Companion to `docs/prd/Quayflow-PRD-v1.1.md` (requirements) and `docs/build-plan.md` (tasks). This file holds the routine details the PRD leaves to engineering: paths, schemas, constants, routes, commands, and the results of the early checks.

## 1. Components and files

| Component | Source | Installed as |
|---|---|---|
| Engine and CLI | `crates/quayflow` (bin `quayflow`) | `%LOCALAPPDATA%\Quayflow\bin\quayflow.exe` (console subsystem) |
| Background launcher | `crates/quayflow` (bin `quayflow-bg`) | `%LOCALAPPDATA%\Quayflow\bin\quayflow-bg.exe` (windows subsystem; used by the sign-in startup entry so no console appears) |
| Mod | `mod/quayflow` | plugin `quayflow` from marketplace `quayflow` (this repository, `.claude-plugin/marketplace.json` at the root) |
| PowerShell integration | `shell/Quayflow.psm1` (embedded in `quayflow.exe`) | written to `%LOCALAPPDATA%\Quayflow\shell\Quayflow.psm1` by `quayflow shell install` |
| Installer | `installer/install.ps1` | GitHub Release asset |

## 2. Data folder `%LOCALAPPDATA%\Quayflow`

| Path | Content |
|---|---|
| `config.json` | `qf_model::Config` (accounts metadata, groups, projects, tuning, consents, D-ORG setting). No credentials |
| `vault\<account-id>.dpapi` | Base64 of a DPAPI blob (CurrentUser, entropy `quayflow-vault-v1`, `CRYPTPROTECT_UI_FORBIDDEN`) of `{"oauth": <claudeAiOauth>, "account": <oauthAccount>}` |
| `backups\unknown-<hash>-<ts>.dpapi` | Protected backup of an unknown live login taken before a manual switch |
| `observations.json` | Last usage observation per account with timestamps, and poll schedule state |
| `journal.jsonl` | Durable journal (switches, recovery decisions, policy overrides, unresolved operations); fsync before a transition write |
| `logs\quayflow-YYYYMMDD.jsonl` | Sanitized structured log; 7 days, 20 MB per day cap |
| `engine.json` | `{port, token, pid, protocol, version, startedAt}` for the running engine |
| `engine.lock` | Single-instance lock: PID, created with `CREATE_NEW`; a dead holder is replaced |
| `engine.stopped` | Present after an intentional stop: on-demand start from the Mod is skipped until `quayflow start` is run by the user |
| `compat\<claude-version>.json` | `qf_model::CompatRecord` |
| `lanes\<name>\` | Lane credential environment (its own `.credentials.json`, `.claude.json`, `sessions\`, policy files; links to the shared setup) |
| `signin\<id>\` | Temporary sign-in environments; credential copies are retired after saving |
| `handoff\claims\<shell-pid>.json` | Pending same-terminal replacement for the PowerShell prompt hook |
| `handoff\backups\` | Destination history replaced by an advancing copy |
| `reports\` | Support reports saved by the user |

PoC data in `%USERPROFILE%\.quayflow-poc` is imported once with `quayflow migrate-poc` (consent required; plaintext entries are sealed during import and never written back as plaintext).

## 3. Claude environment facts (2.1.316, observed)

- Default environment: credentials `%USERPROFILE%\.claude\.credentials.json` (key `claudeAiOauth`), account record `%USERPROFILE%\.claude.json` (key `oauthAccount`).
- With `CLAUDE_CONFIG_DIR=X`: `X\.credentials.json` and `X\.claude.json`.
- Locks (proper-lockfile directory locks; mkdir is the mutex; holders touch about every 5 s): refresh `<configDir>\.oauth_refresh.lock` (stale 60 s), legacy `<configDir>.lock` (stale 60 s), config `<claude.json>.lock` (stale 10 s). Quayflow takes them in that order, waits up to `lockWaitS` (9 s) per lock, never removes a lock younger than its stale age, touches held locks every 2 s, and releases in reverse order.
- Session registry: `<configDir>\sessions\<pid>.json` with `pid, sessionId, cwd, version, kind, status (busy|idle|...), name`.
- Managed policy: registry `HKLM\SOFTWARE\Policies\ClaudeCode` and `HKCU\SOFTWARE\Policies\ClaudeCode` value `Settings` (JSON string); files `C:\Program Files\ClaudeCode\managed-settings.json` and `managed-settings.d\*.json`; server-managed settings stored as `<configDir>\remote-settings.json`. Keys `forceLoginOrgUUID` (string or array) and `forceLoginMethod` (`claudeai` or `console`).
- Usage: `GET https://api.anthropic.com/api/oauth/usage` with `Authorization: Bearer <access>` and `anthropic-beta: oauth-2025-04-20`; fields `five_hour`, `seven_day` (`utilization`, `resets_at`), `limits[]` (`scope.model.display_name`, `percent`, `resets_at`). Extra fields ignored.
- Profile: `GET https://api.anthropic.com/api/oauth/profile` (same headers) gives `account.uuid`, `organization.uuid`.
- Sign-in: `claude auth login` under `CLAUDE_CONFIG_DIR=<temp>`.

## 4. Identity and classification

- Account id: first 12 hex characters of SHA-256 of `oauthAccount.accountUuid`. The Mod computes the same hash from the profile endpoint's `account.uuid`.
- Organization hash: same function over `organizationUuid`.
- Kind: `organization` when `claudeAiOauth.subscriptionType` is `team` or `enterprise`, else `personal`.
- Mixed identities: an organization account together with a personal one, or organization accounts of different organizations.

## 5. Policy rules (qf-policy)

- Governing groups of an environment: the groups that the running sessions' projects resolve to (FR-05); the default group when no session runs. Candidates must be members of every governing group. Effective mode: the most restrictive (manual < suggest < automatic). Threshold: the lowest; margin: the largest; home: only when all governing groups name the same Preferred account.
- Binding window: the fullest of 5 h, 7 d and the per-model weekly window matching the session's model (all per-model windows when the model is unknown).
- Freshness: live account 5 min, idle accounts 60 min (B2: an idle observation is never treated as guaranteed capacity; its age is shown).
- Proactive trigger: live binding >= threshold; candidate binding <= threshold - margin; cooldown 5 min between proactive switches.
- Ranking: lowest binding first; among candidates within 10 points of the best, rank order Preferred < Normal < Backup decides; ties by binding.
- Pin: a manual switch pins until unpin, binding reset of the pinned account, or 2 h; released at 98% of a window.
- Return home: live != home, home fresh and <= threshold - margin, not pinned, cooldown passed.
- Hard limit: confirmed when the session's own rate limit (or a fresh engine observation) is >= 99%. Generic errors never switch.
- Recovery target: eligible member with the most remaining allowance below 99%; otherwise the soonest-available account (availability = latest reset among its exhausted windows; unknown reset times are not ranked). Concurrent failures within 30 s retry on the account already switched to, if it is still live.
- `model_not_found` (or a model-unavailable error) on the first request after a switch marks the model unavailable for that account and switches back; that is the incident's single recovery. An authentication failure (`auth`) right after a switch sets the account aside (refresh rejected) and switches back the same way.
- One login is never live in two environments at once (both clients would refresh the same token); such a switch is refused.

**Known gap (owner decision needed):** Quayflow never refreshes tokens, so an idle account's access token expires after some hours and its usage can no longer be observed. Its last observation then ages past the 60-minute idle freshness limit, and the account stops being a candidate for automatic switching and recovery until it is used again. Options: accept it (current behavior; manual switching still works), allow recovery to use an account whose last observation is old but whose windows have all reset since (contradicts B2 for multi-machine use), or make idle accounts briefly live to let Claude refresh them (changes the shared login, not acceptable). Recorded in `docs/known-limits.md`.

## 6. API v1

Loopback `127.0.0.1:<port>`, header `x-quayflow-token: <token>` from `engine.json`. JSON bodies, camelCase, schemas in `crates/qf-model/src/api.rs`. Errors: HTTP 4xx/5xx with `{"error": "...", "code": "bad_token|invalid|not_found|conflict|refused|incompatible|internal"}`. Mutating requests may carry `requestId`; a repeat within the last 256 ids returns the first response.

| Method and route | Body / query | Response |
|---|---|---|
| `POST /v1/hello` | `HelloRequest` | `HelloResponse` (protocol 1; accepted `[1]`) |
| `GET /v1/state` | `?sessionId=&cwd=` | `StateView` |
| `GET /v1/events` | `?since=<seq>&wait=<s, max 25>` | `EventsResponse` (long-poll) |
| `POST /v1/onboarding` | `{"consents": {"engineStart": true, "credentialStorage": true, "terms": true, "signinStartup": false}, "complete": true}` | `OkResponse` |
| `POST /v1/accounts/register` | `{"label", "group", "env"?}` (current live login) | `AccountView` or error |
| `POST /v1/accounts/signin/start` | `{"reauth"?: accountId}` | `SigninStatus` (opens a new console window running `claude auth login`) |
| `GET /v1/accounts/signin/status` | `?id=` | `SigninStatus` |
| `POST /v1/accounts/signin/confirm` | `{"id", "label", "group"}` | `AccountView` |
| `POST /v1/accounts/signin/cancel` | `{"id"}` | `OkResponse` |
| `POST /v1/accounts/remove` | `{"account"}` | `OkResponse` (refused while live) |
| `POST /v1/accounts/label` | `{"account", "label"}` | `OkResponse` |
| `POST /v1/groups/create` / `delete` | `{"name"}` | `OkResponse` |
| `POST /v1/groups/members` | `{"group", "add": [...], "remove": [...], "acceptDisclosure": bool}` | `GroupChangeResponse` (`needsDisclosure` when the result would mix identities without a covering acceptance) |
| `POST /v1/groups/preferred` | `{"group", "account": id or null}` | `OkResponse` |
| `POST /v1/groups/rank` | `{"group", "account", "rank": "preferred|normal|backup"}` | `OkResponse` |
| `POST /v1/groups/mode` | `{"group", "mode", "approveAutomatic": bool}` | `OkResponse` |
| `POST /v1/groups/tuning` | `{"group", "threshold"?, "margin"?}` | `OkResponse` |
| `POST /v1/groups/default` | `{"group"}` | `OkResponse` |
| `POST /v1/projects/assign` / `unassign` | `{"path", "group"}` / `{"path"}` | `OkResponse` |
| `POST /v1/switch` | `SwitchRequest` (manual; pins) | `SwitchResponse` |
| `POST /v1/accept` | `{"sessionId"?, "env"?}` | `SwitchResponse` |
| `POST /v1/transition/cancel` | `{"id"}` | `OkResponse` (`message` "already started" after the write) |
| `POST /v1/unpin` | `{"env"?}` | `OkResponse` |
| `POST /v1/step` | `StepReport` | `StepResponse` |
| `POST /v1/adoption` | `AdoptionReport` | `OkResponse` |
| `POST /v1/recover` | `RecoverRequest` | `RecoverResponse` |
| `POST /v1/failure` | `FailureShape` | `OkResponse` |
| `POST /v1/unknown` | `{"env"?, "action": "ignore"}` | `OkResponse` |
| `GET /v1/lanes` | | `[LaneView]` |
| `POST /v1/lanes/create` / `delete` / `check` | `{"name"}` | `LaneView` / `OkResponse` |
| `POST /v1/handoff/prepare` | `{"sessionId", "toEnv"}` | `HandoffResponse` (history copied, command only) |
| `POST /v1/handoff/open` | `{"sessionId", "toEnv"}` | `HandoffResponse` (new window launched) |
| `POST /v1/handoff/replace` | `{"sessionId", "toEnv"}` | `HandoffResponse` (claim written for the PowerShell hook; the user then exits the session) |
| `POST /v1/engine` | `{"action": "pause|resume|stop"}` | `OkResponse` |
| `GET /v1/policy` | | `[{"env", "managed": ManagedLogin}]` plus `dOrg` |
| `POST /v1/policy/dorg` | `{"enable": bool, "confirmOrg": "<org hash>", "acknowledge": "<typed text>"}` | `OkResponse` (setting recorded only; no bypass path in this build) |
| `GET /v1/compat` / `POST /v1/compat/check` | | `[CompatRecord]` |
| `POST /v1/report` | `{}` | `ReportResponse` (saved locally for review) |
| `POST /v1/poll` | `{}` | `OkResponse` (observe every account now, respecting backoff) |
| `POST /v1/migrate-poc` | `{"group", "acceptDisclosure"}` | `OkResponse` (imports the PoC vault) |
| `POST /v1/canary/start` / `GET /v1/canary` / `POST /v1/canary/stop` | | Manual live canary recording (consent `liveCanary`); while active, `/v1/step` asks for identity on every request |
| `POST /v1/canary/switch` | `{"to"}` | `SwitchResponse` (switch marked `canary`, no pin) |
| `POST /v1/compat/reset` | `{"version", "livePass", "detail"}` | `CompatRecord` (record a manual canary result) |

Refusals that need an explicit user decision use HTTP 409 with `code` `needs_disclosure` (mixed-identity group; resend with `acceptDisclosure: true`) or `needs_approval` (Automatic mode; resend with `approveAutomatic: true`).

The Mod's request hook calls `/v1/step` after every main or subagent request; when the response says `verify`, it reads the identity its request used (`$.session.authorize()` handle + profile endpoint, hashed) and posts `/v1/adoption`.

## 7. CLI

`quayflow <command>`; commands that need the engine talk to it over the API.

```text
serve                          run the engine (foreground; used by start)
start [--if-consented]         start the engine without a window; prints {"state":"running|started|failed|no_consent|stopped_by_user"}
stop | pause | resume          intentional stop; pause or resume automation
status [--json]                summary of environments, accounts and usage
onboard                        interactive first-use consents (terminal)
register --label L --group G   register the current login (asks for consent)
add --label L --group G        add another account through native sign-in in a new window
reauth <account>               sign in again for a set-aside account
remove <account> | label <account> <new>
use <account> [--idle-wait] | accept | unpin
mode <manual|suggest|automatic> [--group G] [--approve]
group list | create <G> | delete <G> | add <G> <account..> [--accept-disclosure] | remove <G> <account..>
      | preferred <G> <account|none> | rank <G> <account> <rank> | default <G> | tuning <G> [--threshold N] [--margin N]
project assign <path> <G> | project unassign <path> | project list
canary [--live]                static check now; --live runs the synthetic adoption job (consent)
lanes list | create <name> | delete <name> | check <name>
handoff open|replace|prepare --session <id> --to <env>
shell install | uninstall      PowerShell prompt hook (PowerShell 7 and 5.1 profiles)
shell claim                    used by the prompt hook: print and consume this terminal's pending replacement
policy [--json]                managed login enforcement per environment
report                         build a local support report for review
migrate-poc                    import the PoC vault (~/.quayflow-poc)
update [--check]               engine and Mod as a matched pair (hash-verified)
uninstall [--delete-data] [--yes]
consent <engine-start|signin-startup|credential-storage|terms|live-canary> <on|off>
version
```

## 8. Early checks

| Check | Result | Chosen path |
|---|---|---|
| C1 Mod imports sibling modules | See `mod/quayflow/README.md` (recorded by the Mod build) | |
| C2 Pane with tabs, Input and Select | See `mod/quayflow/README.md` | |
| C3 Claude through lane links | Junctions (`junction` crate, no admin) for folders. Files (`settings.json`, `CLAUDE.md`) are hard links; if Claude replaces a file by rename, the link breaks: `lanes check` detects a broken link and re-links to the newer content. Release condition: confirm with a real lane session (owner-run, `tests/e2e/lanes.ps1`) | Junctions + hard links with repair |
| C4 PoC DPAPI blobs decrypt in Rust | The PoC stored `base64(ProtectedData.Protect(utf8 JSON, entropy, CurrentUser))`; Rust `CryptUnprotectData` with the same entropy reads it (unit test round-trips the format; owner runs `quayflow migrate-poc`) | Migration supported |
| C5 Native sign-in in a temporary environment | `claude auth login` exists in 2.1.316. The engine opens it in a new console window (`CREATE_NEW_CONSOLE`) with `CLAUDE_CONFIG_DIR` set and detects completion by polling the temporary environment for `claudeAiOauth` + `oauthAccount`; the exact command is shown as a fallback | New window + detection |

## 9. Build notes

- Local development on a machine without Visual Studio Build Tools: `RUSTUP_TOOLCHAIN=1.98.0-x86_64-pc-windows-gnu` (no C compiler needed: HTTPS uses Windows SChannel through `native-tls`). Releases are built by CI with the pinned MSVC toolchain.
- Reproducible release: pinned toolchain (`rust-toolchain.toml`), `Cargo.lock`, `--locked`, `strip`, no debug info, `-C link-arg=/Brepro`, `SOURCE_DATE_EPOCH` from the tag's commit, `--remap-path-prefix`.
