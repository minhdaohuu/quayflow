# Quayflow Mod

The Claude Code (2.1.316) side of Quayflow, version 0.1.6. It runs inside every Claude session and talks to the per-user engine `quayflow.exe` over its loopback API v1 (`docs/tech-spec.md` §6). It never reads or writes a credential.

What it does:

- **Request hook (`turn.step`).** After every main or subagent request it posts `/v1/step` with the session's own rate-limit readings. When the engine answers `verify: true`, it reads the account the request used (`$.session.authorize()` handle + the first-party profile endpoint, SHA-256 of `account.uuid`, first 12 hex characters) and posts `/v1/adoption`, every time the engine asks (after a switch, and on every request while the live canary records). A request that ended with no response (no stop reason, no text, no tool call, not cancelled) is reported to `/v1/failure` (field names and counts only, plus an error class the engine acts on: `model_not_found` or `auth`), then `/v1/recover` is asked once per request key; on `retry: true` the same request is retried exactly once. A cancelled request is never recovered. Every failure of the Mod's own calls leaves Claude's result standing.
- **Handshake and engine start.** On session start, off the startup path, it reads `engine.json` in the engine's data folder (`%LOCALAPPDATA%\Quayflow`, on Linux and macOS `~/.local/share/quayflow`) and posts `/v1/hello`, with the session's `CLAUDE_CONFIG_DIR` and `CLAUDE_SECURESTORAGE_CONFIG_DIR` exactly as set (on macOS the engine uses them to check that the session reads the Keychain item it writes). The first handshake is retried only when something answered or the wait ran out; a refused connection or a missing `engine.json` means nothing listens, so it runs `quayflow.exe start --if-consented` at once (once per session) and respects `no_consent` and `stopped_by_user`. With an incompatible or absent engine, everything Quayflow does is off and Claude behaves as without it.
- **Reconnect.** After `/reload-plugins` the last link is kept until the new handshake answers. A resume or `/clear` goes on under a new session id without a session start (`session.end` with reason `resume` or `clear`): the Mod carries the link and the last state over, says hello under the new id, and repaints a pane that shows "off" while it is linked. The pane and every `/quay` command connect when no handshake is under way (never starting the engine) and start the event watcher if it is missing. The short wait reads "Connecting to the engine…", not "off".
- **Status line.** `Quayflow · <account> · <mode> · <status>`, with the mode capitalized and one status chosen by priority (problem, unknown login or sign-out, switch in progress or pending next request, suggestion, pin end, managed-login override, warning, stale usage, home or away, plain usage). Usage shows both windows (`5h 45% · 7d 30%`) plus the per-model weekly window when it applies to the session's model and is the fullest. The line fits the terminal width by dropping usage first, then shortening the status, never the account name. Off states are one line, such as `Quayflow · off · engine not running (Claude works as usual)`. When Claude Code's own status line already shows the 5-hour / 7-day limits, the usage figures are left out (`hooks/hostline.ts` reads the `statusLine` setting and the script it names, never runs it); the plugin option `statusLineUsage` (`auto`, `show`, `hide`) overrides this. Kept current by the engine's event long-poll (`/v1/events?wait=25`). In the terminal the everyday line is the dim `tail` of the prompt hint (`ui.render` for `PromptHint`); a line that needs the person (`statusParts(...).urgent`: errors, sign-in, paused, suggestion, warnings, engine off) is the pinned `$.ui.status`, which Claude Code draws as a warning. Other surfaces use `$.ui.status` for every line.
- **Transcript notes.** Suggestions (also as a toast), switches, adoptions and engine notices (login changed outside Quayflow, signed out, switch back after `model_not_found`, the outcome of an interrupted switch after a restart) are written into the transcript once per session, in interactive sessions only, without replaying events from before the session started.
- **`/quay` and its pane.** Tabs Overview, Accounts, Groups, Activity and Settings (keys 1-5), a status strip under the tabs, a "? Hints" toggle (key `h`) that shows or hides explanations, the switch flow, review screens, trust views and first use. On the terminal, usage is drawn as true-color graphics (the `Raster` element): a fleet chart of every account's 5-hour and 7-day columns with the threshold as a dashed line, ▼ over the live account and ★ over the recommended one; gradient gauges with a threshold tick; a step track for a switch. Other surfaces draw text gauges. Every figure is also written out beside its graphic. `/quay <subcommand>` does the same from the keyboard and in headless scripts (`/quay help`).
- **Approvals stay explicit.** When the engine refuses a change with HTTP 409 `needs_disclosure` (a group would mix identities: membership, registration, saving a sign-in) or `needs_approval` (Automatic for a group), the Mod shows the text and resends with `acceptDisclosure` / `approveAutomatic` only after the person accepts. An option `/quay` does not know is refused with the nearest known one suggested (`--aprove` → `did you mean --approve?`), so a typo never reads as no approval.
- **Headless (`claude -p`, the SDK).** Only the hooks run: no pane, question, toast, status, transcript note or log line. A `/quay` command typed as the prompt of a headless run prints its own text output, and approvals that would need a question must be typed (`--approve`, `--accept-disclosure`).

## Commands

`/quay` alone opens the pane (interactive) or prints the status (headless). Known options: `--approve`, `--accept-disclosure`, `--idle-wait`, `--save`; any other option is refused. Subcommands:

| Command | Engine route |
|---|---|
| `status` | `GET /v1/state` |
| `accept` | `POST /v1/accept` |
| `use <account> [--idle-wait]` (alias `switch`) | `POST /v1/switch` (pins the choice) |
| `unpin` | `POST /v1/unpin` |
| `mode <manual\|suggest\|automatic> [group] [--approve]` | `POST /v1/groups/mode` |
| `group list\|create\|delete\|add\|remove\|preferred\|rank\|default\|tuning\|assign\|unassign ...` | `/v1/groups/*`, `/v1/projects/*` |
| `pause`, `resume` | `POST /v1/engine` (`resume` says when automation is still paused for another cause) |
| `poll` | `POST /v1/poll` (reads at once, except while the engine has slowed or backed off after a refused read) |
| `register [label] [group] [--accept-disclosure]` | `POST /v1/accounts/register` (no label, interactive: opens the pane) |
| `add` | `/v1/accounts/signin/*` (pane) |
| `lanes [create\|delete\|check <name>]` | `/v1/lanes/*` |
| `replace <lane>`, `open <lane>` | `POST /v1/handoff/replace`, `/open` (asks first in an interactive session) |
| `report [--save]` | `POST /v1/report/preview` (nothing saved); with `--save`, `POST /v1/report` |
| `whoami` | profile lookup through Claude's own credential handle |
| `canary` | `GET /v1/canary` |
| `migrate-poc [group] [--accept-disclosure]` | `POST /v1/migrate-poc` |
| `ignore` | `POST /v1/unknown` (automation stays paused for that login) |
| `start` | `quayflow.exe start` (the person's explicit request) |
| `help` | |

The pane also uses `GET /v1/explain`, `/v1/identity`, `/v1/handoff/check`, `/v1/operations`, `POST /v1/report/preview`, `/v1/report/save`, `/v1/pin`, `/v1/tuning`, `/v1/accounts/label`, `/v1/accounts/remove`, `/v1/onboarding`, `/v1/policy/dorg`, `/v1/compat/check` and `/v1/transition/cancel`.

## Layout

| Path | Content |
|---|---|
| `.claude-plugin/plugin.json` | Manifest (`quayflow` 0.1.6), names the type contract and the `statusLineUsage` option |
| `hooks/hooks.json` | Names the hooks module `./register.tsx` |
| `hooks/register.tsx` | Entry: `session.start`, `session.end` (resume and `/clear`), `turn.step`, `command.run` (`/quay`), `ui.render` (pane `quay`) |
| `hooks/context.ts` | Session context, `$.state` atoms (all reads and writes), headless UI gates |
| `hooks/engine.ts` | Connection file, authenticated calls with timeouts, handshake, on-demand start, reconnect |
| `hooks/step.ts` | Request hook: step report, adoption verification, failure shape, recovery, single retry, simulated limit |
| `hooks/identity.ts` | Identity hash through the session's own authentication path |
| `hooks/watcher.ts` | Event long-poll, snapshot refresh, status line, one-time announcements (interactive only) |
| `hooks/hostline.ts` | Whether Claude Code's own status line already shows the usage limits |
| `hooks/format.ts` | Text rules: status line, usage, transition and adoption wording |
| `hooks/words.ts` | Text helpers of the review screens (home state, candidate status, switch phases) |
| `hooks/actions.ts` | Engine operations shared by `/quay` and the pane (API v1 routes) |
| `hooks/commands.ts` | `/quay` subcommands and option check |
| `hooks/pane.tsx` | The pane: tabs, status strip, Overview (live-account card, fleet chart or rows, recommendation), Accounts, Groups, Activity, Settings |
| `hooks/flows.tsx` | Engine off and incompatible states, first use, register and add account, switch review and progress, group conflict, trust views |
| `hooks/review.tsx` | Review screens: "Why this account?", home state, running clients, limits and recovery, session actions, lanes, version, update, uninstall, operations, support report |
| `hooks/chart.ts` | True-color cell graphics for the terminal: gauges, fleet chart, step track |
| `hooks/ui.tsx` | Shared pane pieces: render context, navigation, form fields, action runner, cards, badges, hints |
| `types/index.d.ts` | API v1 schema mirrored from `crates/qf-model` (camelCase) and the `$.state` contract |
| `tests/*.test.ts(x)` | Tests against a mocked engine (`tests/harness.ts`, `tests/fixtures.ts`): hooks and connection (`quayflow.test.ts`), status line (`statusline.test.ts`), pane (`pane.test.tsx`), review screens (`review.test.tsx`), graphics and account-row reasons (`chart.test.ts`) |

## Early checks

### C1: the hooks module can import sibling modules — supported

The 2.1.316 declaration file says: "A file of the plugin is imported with an `import` declaration. A module holding `import()` does not load." `claude plugin validate mod/quayflow` reads `register.tsx` and every module it imports (it reports calls "via" functions in `engine.ts`, `step.ts`, `pane.tsx` and so on), and `claude plugin test` loads them. **Chosen path:** the Mod is split into the files above with static imports only.

Two limits found while validating, which shape the code:

- `$` may only be passed by name to a named function (`call($, ...)`); passing it through an object member (`api.x($)`) or keeping it in an object is refused. Operations are therefore plain exported functions, and pane helpers take `$` as their first parameter.
- `$.state` atoms must be read and written in the file that defines them. All atoms live in `hooks/context.ts`, which exports the read and write functions.

### C2: a pane with tab-like navigation, Input and Select, keyboard-usable in the terminal — supported

The terminal's element table (`Elements['terminal']`) has `Box`, `Text`, `Button`, `Input`, `Select` and `Raster` (no native tab widget). A pane opened with `focus` takes the keys: Tab and the arrows walk its controls, a Button's `hotkey` presses it, Esc returns the keys to the prompt. **Chosen path:** tabs are a row of Buttons with hotkeys 1-5 (the active one drawn primary); "? Hints" is a Button with hotkey `h`; forms use `Input` (a field that opens a screen, such as the name in Add account, takes the keyboard, and Enter submits it) and `Select`; checkboxes are Buttons whose label carries `[x]`/`[ ]`. Graphics use `Raster` only on the terminal; every graphic has its figures written beside it. On a surface without `Input`/`Select` (mobile) the pane falls back to buttons and a note, and `/quay` subcommands cover every action.

Validated with the test kit (`tests/pane.test.tsx`, `tests/review.test.tsx`): the pane is mounted on the `terminal` and `desktop` surfaces, the trees validate against each surface's element rules, and the tests drive it by key: tab buttons, account detail, the switch flow, a `Select` for the group mode (Automatic needs its approval view), an `Input` that creates a group, the mixed-identity disclosure (group membership and registration), the group-conflict view, first-use consents, registration review, session actions, the support report and the typed managed-login acknowledgment.

## Check, type-check and test

All of these run without a Claude login and without an engine.

```text
claude plugin validate --strict .
claude plugin validate --strict mod/quayflow
claude plugin test mod/quayflow
```

Type check (no package.json needed):

```text
npx -y -p typescript@5.6.3 tsc -p mod/tsconfig.ci.json
```

The type check needs Claude Code's own API declaration file for 2.1.316, which is Claude Code's, not part of this repository. Copy it to `mod/host-types/claude-code-2.1.316.d.ts` (gitignored; never commit it). CI skips the type check with a warning when it is absent. Claude Code writes the file in two places you can copy it from:

- a loaded Mod's `.claude-plugin/types/claude-code/index.d.ts` (Claude Code lays it beside a Mod it loads from a hot-reloaded folder, for example `claude --plugin-dir mod/quayflow` in an interactive session; gitignored), or
- the plugin-authoring skill's folder, `%TEMP%\claude\bundled-skills\<claude-version>\<id>\plugin-authoring\types\claude-code.d.ts` (on Linux and macOS under `$TMPDIR` or `/tmp`), written when that skill loads.

Once `.claude-plugin/types/` exists, `tsc -p mod/quayflow` also works with the `tsconfig.json` in this folder. After a Claude Code update, refresh the copy from the new build.

`claude plugin test mod/quayflow` needs no signed-in Claude: everything beneath the plugin is mocked (checked with an empty `CLAUDE_CONFIG_DIR`).

## Not implemented in v0.1

- **Pre-request hold.** A bounded hold at the request boundary is optional. The engine's credential swap is atomic between requests, so v0.1 has no hold: the request hook never delays a request before it is sent.

## Testing a usage limit by hand

Start Claude with `QUAYFLOW_SIM_LIMIT=1` in its environment (without it the hook is off), then write `%LOCALAPPDATA%\Quayflow\simulate-limit.json` (Linux and macOS: `~/.local/share/quayflow/simulate-limit.json`) as `{"atStep": 0}`. The next main-agent request with index >= `atStep` is answered as failed without being sent, once: the session that fires takes the file by renaming it, so only one session fires even when several run. The failure is reported with `simulated: true` and the engine decides as for a real one.

## Privacy

The Mod sends the engine opaque ids (session, turn, agent, identity hashes), labels and group names you typed, the session's rate-limit percentages, the project folder path, and field names of a failed result. It never sends prompt, response or tool content, tokens, or e-mail addresses. The profile lookup goes to the first-party host only, through Claude's own credential handle. The live login's e-mail and organization shown for review at registration come from the engine and are never stored or logged.

## Install

The Quayflow installer (`installer/install.ps1`; `installer/install.sh` on Linux and macOS) adds this repository's marketplace pinned to the release tag and installs the plugin at user scope; `quayflow update` runs the release's installer in update mode, which updates the engine and the Mod as a matched pair (a Mod installed from a local folder is left as it is). Running Claude sessions load a new Mod after a restart.
