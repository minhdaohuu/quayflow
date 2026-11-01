# Quayflow Mod

The Claude Code (2.1.316) side of Quayflow. It runs inside every Claude session and talks to the per-user engine `quayflow.exe` over its loopback API v1 (`docs/tech-spec.md` §6). It never reads or writes a credential.

What it does:

- **Request hook (`turn.step`).** After every main or subagent request it posts `/v1/step` with the session's own rate-limit readings. When the engine answers `verify: true`, it reads the account the request used (`$.session.authorize()` handle + the first-party profile endpoint, SHA-256 of `account.uuid`, first 12 hex characters) and posts `/v1/adoption`, every time the engine asks (after a switch, and on every request while the live canary records). A request that ended with no response (no stop reason, no text, no tool call, not cancelled) is reported to `/v1/failure` (field names and counts only, plus an error class the engine acts on: `model_not_found` or `auth`), then `/v1/recover` is asked once per request key; on `retry: true` the same request is retried exactly once. A cancelled request is never recovered. Every failure of the Mod's own calls leaves Claude's result standing.
- **Handshake and engine start.** On session start, off the startup path, it reads `%LOCALAPPDATA%\Quayflow\engine.json` and posts `/v1/hello`. With no engine it runs `quayflow.exe start --if-consented` once per session and respects `no_consent` and `stopped_by_user`. With an incompatible or absent engine, everything Quayflow does is off and Claude behaves as without it.
- **Status line.** `Quayflow · <mode> · <account> <binding%> <window> · <notice>`, with persistent warnings (managed-login exception on, automation paused, in-place switching off for this Claude version, unknown login, another switcher). Kept current by the engine's event long-poll (`/v1/events?wait=25`). Each suggestion and notice is announced once.
- **`/quay` and its pane.** Tabs Overview, Accounts, Groups, Activity and Settings (keys 1-5), the switch flow, trust views and first use. `/quay <subcommand>` does the same from the keyboard and in headless scripts (`/quay help`).
- **Approvals stay explicit.** When the engine refuses a change with HTTP 409 `needs_disclosure` (a group would mix identities: membership, registration, saving a sign-in) or `needs_approval` (Automatic for a group), the Mod shows the text and resends with `acceptDisclosure` / `approveAutomatic` only after the person accepts.
- **Headless (`claude -p`, the SDK).** Only the hooks run: no pane, question, toast, status or log line. A `/quay` command typed as the prompt of a headless run prints its own text output, and approvals that would need a question must be typed (`--approve`, `--accept-disclosure`).

## Commands

`/quay` alone opens the pane (interactive) or prints the status (headless). Subcommands:

| Command | Engine route |
|---|---|
| `status` | `GET /v1/state` |
| `accept` | `POST /v1/accept` |
| `use <account> [--idle-wait]` | `POST /v1/switch` (pins the choice) |
| `unpin` | `POST /v1/unpin` |
| `mode <manual\|suggest\|automatic> [group] [--approve]` | `POST /v1/groups/mode` |
| `group list\|create\|delete\|add\|remove\|preferred\|rank\|default\|tuning\|assign\|unassign ...` | `/v1/groups/*`, `/v1/projects/*` |
| `pause`, `resume` | `POST /v1/engine` |
| `poll` | `POST /v1/poll` |
| `register [label] [group] [--accept-disclosure]` | `POST /v1/accounts/register` |
| `add` | `/v1/accounts/signin/*` (pane) |
| `lanes [create\|delete\|check <name>]` | `/v1/lanes/*` |
| `replace <lane>`, `open <lane>` | `POST /v1/handoff/replace`, `/open` |
| `report` | `POST /v1/report` |
| `whoami` | profile lookup through Claude's own credential handle |
| `canary` | `GET /v1/canary` |
| `migrate-poc [group] [--accept-disclosure]` | `POST /v1/migrate-poc` |
| `ignore` | `POST /v1/unknown` (automation stays paused for that login) |
| `start` | `quayflow.exe start` (the person's explicit request) |
| `help` | |

## Layout

| Path | Content |
|---|---|
| `.claude-plugin/plugin.json` | Manifest (`quayflow` 0.1.0), names the type contract |
| `hooks/hooks.json` | Names the hooks module `./register.tsx` |
| `hooks/register.tsx` | Entry: `session.start`, `turn.step`, `command.run` (`/quay`), `ui.render` (pane `quay`) |
| `hooks/context.ts` | Session context, `$.state` atoms (all reads and writes), headless UI gates |
| `hooks/engine.ts` | Connection file, authenticated calls with timeouts, handshake, on-demand start, reconnect |
| `hooks/step.ts` | Request hook: step report, adoption verification, failure shape, recovery, single retry, simulated limit |
| `hooks/identity.ts` | Identity hash through the session's own authentication path |
| `hooks/watcher.ts` | Event long-poll, snapshot refresh, status line, one-time announcements (interactive only) |
| `hooks/format.ts` | Text rules: status line, usage, transition and adoption wording |
| `hooks/actions.ts` | Engine operations shared by `/quay` and the pane (API v1 routes) |
| `hooks/commands.ts` | `/quay` subcommands |
| `hooks/pane.tsx`, `hooks/flows.tsx`, `hooks/ui.tsx` | The pane: tabs, flows (first use, switch, conflict, trust), shared controls |
| `types/index.d.ts` | API v1 schema mirrored from `crates/qf-model` (camelCase) and the `$.state` contract |
| `tests/*.test.ts(x)` | Tests against a mocked engine (`tests/harness.ts`, `tests/fixtures.ts`) |

## Early checks

### C1: the hooks module can import sibling modules — supported

The 2.1.316 declaration file says: "A file of the plugin is imported with an `import` declaration. A module holding `import()` does not load." `claude plugin validate mod/quayflow` reads `register.tsx` and every module it imports (it reports calls "via" functions in `engine.ts`, `step.ts`, `pane.tsx` and so on), and `claude plugin test` loads them. **Chosen path:** the Mod is split into the files above with static imports only.

Two limits found while validating, which shape the code:

- `$` may only be passed by name to a named function (`call($, ...)`); passing it through an object member (`api.x($)`) or keeping it in an object is refused. Operations are therefore plain exported functions, and pane helpers take `$` as their first parameter.
- `$.state` atoms must be read and written in the file that defines them. All atoms live in `hooks/context.ts`, which exports the read and write functions.

### C2: a pane with tab-like navigation, Input and Select, keyboard-usable in the terminal — supported

The terminal's element table (`Elements['terminal']`) has `Box`, `Text`, `Button`, `Input`, `Select` (no native tab widget). A pane opened with `focus` takes the keys: Tab and the arrows walk its controls, a Button's `hotkey` presses it, Esc returns the keys to the prompt. **Chosen path:** tabs are a row of Buttons with hotkeys 1-5 (the active one marked `>` in its label and drawn primary, so state is not color alone); forms use `Input` and `Select`; checkboxes are Buttons whose label carries `[x]`/`[ ]`. On a surface without `Input`/`Select` (mobile) the pane falls back to buttons and a note, and `/quay` subcommands cover every action.

Validated with the test kit (`tests/pane.test.tsx`): the pane is mounted on the `terminal` and `desktop` surfaces, the trees validate against each surface's element rules, and the tests drive it by key: tab buttons, account detail, the switch flow, a `Select` for the group mode (Automatic needs its approval view), an `Input` that creates a group, the mixed-identity disclosure (group membership and registration), the group-conflict view, first-use consents and the typed managed-login acknowledgment. A look at the pane in a live terminal session (`claude --plugin-dir mod/quayflow`) remains for the owner: loading the Mod against a real engine touches the live login.

## Check, type-check and test

All three run without a Claude login and without an engine.

```text
claude plugin validate mod/quayflow
claude plugin test mod/quayflow
```

Type check (no package.json needed):

```text
npx -y -p typescript@5.6.3 tsc -p mod/tsconfig.ci.json
```

`mod/tsconfig.ci.json` sits outside the Mod folder and reads Claude Code's API declaration file from `mod/host-types/claude-code-2.1.316.d.ts`. That file is Claude Code's own and is **not committed** (`mod/host-types/` is gitignored); CI skips the type check with a warning when it is absent. To get it on a development machine, copy it from either place Claude Code writes it, as `mod/host-types/claude-code-2.1.316.d.ts`:

- a loaded Mod's `.claude-plugin/types/claude-code/index.d.ts` (Claude Code lays it beside a Mod it loads from a hot-reloaded folder, for example `claude --plugin-dir mod/quayflow` in an interactive session; also gitignored), or
- the plugin-authoring skill's folder, `%TEMP%\claude\bundled-skills\<claude-version>\<id>\plugin-authoring\types\claude-code.d.ts`, written when that skill loads.

Once `.claude-plugin/types/` exists, `tsc -p mod/quayflow` also works with the `tsconfig.json` in this folder. After a Claude Code update, refresh the copy from the new build.

`claude plugin test mod/quayflow` needs no signed-in Claude: everything beneath the plugin is mocked (checked with an empty `CLAUDE_CONFIG_DIR`).

## Not implemented in v0.1

- **Pre-request hold.** FR-11 makes a bounded Mod hold at the request boundary optional. The engine's credential swap is atomic between requests, so v0.1 has no hold: the request hook never delays a request before it is sent.
- **Live-terminal review of the pane.** The tests validate the pane on the terminal and desktop element rules and drive it by key; a look at it in a live terminal session remains for the owner.

## Testing a usage limit by hand

Write `%LOCALAPPDATA%\Quayflow\simulate-limit.json` as `{"atStep": 0}`. The next main-agent request with index >= `atStep` is answered as failed without being sent, once, and the file is emptied; the failure is reported with `simulated: true` and the engine decides as for a real one.

## Privacy

The Mod sends the engine opaque ids (session, turn, agent, identity hashes), labels and group names you typed, the session's rate-limit percentages, the project folder path, and field names of a failed result. It never sends prompt, response or tool content, tokens, or e-mail addresses. The profile lookup goes to the first-party host only, through Claude's own credential handle.

## Install

The Quayflow installer (`installer/install.ps1`) adds this repository's marketplace and installs the plugin at user scope; `quayflow update` updates the engine and the Mod as a matched pair.
