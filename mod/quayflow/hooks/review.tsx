// Review screens of the /quay pane: "Why this account?", home state, running clients,
// usage limits and recovery, full-history session actions, lane moves and creation,
// version and canary, update, uninstall, operations and the support report.
// Each draws what the engine reports; none invents a state the engine does not have.
import type { EngineInterface } from 'claude-code'

import type { CanaryStatus, EnvView, ExplainView, HandoffCheck, HandoffResponse, OperationView, StateView } from '../types'
import { MOD_VERSION, Q, flash, setReport } from './context'
import {
  actCanary,
  actCompatCheck,
  actConsent,
  actExplain,
  actHandoff,
  actHandoffCheck,
  actLaneCreate,
  actOperations,
  actPin,
  actReportPreview,
  actReportSave,
  actShellInstall,
  actSwitchTo,
} from './actions'
import { account, adoptionWords, age, clock, label, myEnv, myGroup, pct, sessionName, when, windowName } from './format'
import { candidateUsage, candidateWord, homeOf, homeWords, resetText, seconds } from './words'
import { Alert, Badge, Btn, Card, Field, Gauge, Hint, KV, hintsOn, Line, Pick, Row, Section, Steps, Table, Toggle, clearForm, go, run, setForm, useRead } from './ui'
import type { Ctx } from './ui'

/** The review screen for the current view, or undefined when the view is not one of them. */
export function Review($: EngineInterface, c: Ctx): JSX.Element | undefined {
  const { tab, sub } = c.view
  if (tab === 'overview' && sub === 'why') return Why($, c)
  if (tab === 'overview' && sub === 'sessions') return RunningClients($, c)
  if (tab === 'overview' && sub === 'limits') return Limits($, c)
  if (tab === 'accounts' && sub === 'observation' && c.view.arg) return Observation($, c, c.view.arg)
  if (tab === 'session') return SessionActions($, c)
  if (tab === 'switch' && sub === 'lanemove') return LaneMove($, c)
  if (tab === 'groups' && sub === 'newlane') return NewLane($, c)
  if (tab === 'settings' && sub === 'version') return Version($, c)
  if (tab === 'settings' && sub === 'update') return Update($, c)
  if (tab === 'settings' && sub === 'uninstall') return Uninstall($, c)
  if (tab === 'activity' && sub === 'op' && c.view.arg) return Operation($, c, c.view.arg)
  if (tab === 'activity' && sub === 'report') return Report($, c)
  if (tab === 'trust' && sub === 'policy') return PolicyChange($, c)
  return undefined
}

/** The engine's explanation for this environment, read again whenever the state moves on. */
export function explanation($: EngineInterface, c: Ctx): { x?: ExplainView; error?: string } {
  const s = c.s!
  const env = myEnv(s)
  const r = useRead<ExplainView>($, c, 'explain', `${env?.id ?? '?'}:${s.seq}`, () => actExplain($), true)
  const x = r.value && Array.isArray(r.value.candidates) ? r.value : undefined
  return { x, error: r.error }
}

function Pending(c: Ctx, what: string, error?: string): JSX.Element {
  return error ? Line(c, `${what} could not be read: ${error}`, { tone: 'warn' }) : Line(c, `Checking ${what}...`, { dim: true })
}

// ------------------------------------------------------------ S13 Why this account?

function Why($: EngineInterface, c: Ctx): JSX.Element {
  const { Box } = c.els
  const s = c.s!
  const env = myEnv(s)
  const { x, error } = explanation($, c)
  if (!env) return Line(c, 'No environment.')
  const sg = env.suggestion
  const firstEligible = x?.candidates.find(k => k.status === 'eligible')
  // A running session whose group refuses a candidate: the conflict to resolve.
  const conflict = x?.candidates
    .filter(k => k.status !== 'live')
    .map(k => ({ to: k.account, blocker: env.sessions.find(z => z.group && !s.groups.find(g => g.name === z.group)?.accounts.includes(k.account)) }))
    .find(k => k.blocker)
  return (
    <Box flexDirection="column">
      {Section(
        c,
        'Why?',
        x ? Line(c, x.decision) : null,
        x
          ? Table(
              c,
              'why',
              ['Account', 'Decision'],
              x.candidates.map(k => [`${sg?.to === k.account ? '\u2605 ' : ''}${label(s, k.account)}`, `${candidateWord(k.status)}: ${k.fresh === false ? candidateUsage(k) : k.reason}`]),
            )
          : Pending(c, 'the explanation', error),
        x ? Line(c, `Threshold ${x.threshold}% \u00b7 margin ${x.margin} \u00b7 cooldown ${x.cooldownLeftS > 0 ? `${x.cooldownLeftS} s left` : 'elapsed'}`, { dim: true }) : null,
        Hint(c, 'Eligibility first; binding windows, not averaged capacity.'),
      )}
      {x &&
        hintsOn(c) &&
        Section(
          c,
          'Decision inputs',
          KV(c, 'Threshold', `${x.threshold}% of the binding window`),
          KV(c, 'Required margin', `at least ${x.margin} points below the threshold (${x.threshold - x.margin}% or less)`),
          KV(c, 'Proactive cooldown', `${seconds(x.cooldownS)}${x.cooldownLeftS > 0 ? `; ${x.cooldownLeftS} s left after the last switch` : '; elapsed'}`),
          KV(c, 'Effective mode', `${x.mode} (group ${x.governingGroups.join(', ') || myGroup(s)})`),
          KV(c, 'Current decision', x.decision),
          Hint(c, 'Percentages across different plans are not equivalent token capacities. Local coordination does not reserve allowance; an idle account may be used on other machines.'),
        )}
      {Row(
        c,
        (sg || firstEligible) &&
          Btn(c, 'why-review', 'Switch', () => go($, sg ? { tab: 'switch', sub: 'accept' } : { tab: 'switch', sub: 'review', arg: firstEligible!.account }), true),
        conflict &&
          Btn(c, 'why-conflict', 'Resolve conflict', () => {
            setForm($, 'conflict', JSON.stringify(conflict.blocker))
            go($, { tab: 'switch', sub: 'conflict', arg: conflict.to })
          }),
        Btn(c, 'why-close', 'Close', () => go($, { tab: 'overview' })),
      )}
    </Box>
  )
}

// ------------------------------------------------------------ S15 Home

/** The home-account block of the Overview (S15): state in the design's words; policy is the engine's. */
export function HomeBlock($: EngineInterface, c: Ctx, env: EnvView): JSX.Element | null {
  const s = c.s!
  const home = homeOf(s)
  const { x } = explanation($, c)
  const state = x?.homeState ?? (home ? (env.live.account === home ? 'home' : 'away') : 'none')
  // At home, or no home account: the live card's HOME mark and the Home button say it all.
  if (state === 'none' || state === 'home') return null
  const homeId = x?.home ?? home
  const returnSuggested = !!homeId && env.suggestion?.to === homeId
  const pin = env.pin
  return Section(
    c,
    `Home account: ${homeWords(state)}`,
    Line(c, x?.homeDetail ?? `Home is ${label(s, homeId)}.`, { tone: state === 'away' ? undefined : 'ok' }),
    Hint(
      c,
      `Pin: ${pin ? `${label(s, pin.account)} until ${clock(pin.until)}` : 'none'}. Cooldown: ${x ? (x.cooldownLeftS > 0 ? `${x.cooldownLeftS} s left` : 'elapsed') : 'unknown'}. Automatic returns at a permitted moment; Suggest offers it instead. Pin, cooldown, plan and group checks apply first.`,
    ),
    Row(
      c,
      returnSuggested && Btn(c, 'home-return', 'Return home', () => go($, { tab: 'switch', sub: 'accept' }), true),
      !returnSuggested && Btn(c, 'home-why', 'Why?', () => go($, { tab: 'overview', sub: 'why' })),
      !pin && env.live.known && Btn(c, 'home-pin', 'Pin', () => void run($, 'pinning the current account', () => actPin($), p => `Pinned "${label(c.s ?? undefined, p.account)}" until ${clock(p.until)}.`)),
    ),
  )
}

// ------------------------------------------------------------ S16 Running clients

function requestIdentity(s: StateView, env: EnvView, sessionId: string, pid: number): string {
  if (sessionId === Q.sessionId && s.session?.verifiedAccount) return `${label(s, s.session.verifiedAccount)} verified`
  const a = [...env.adoption].reverse().find(z => z.sessionId === sessionId || z.pid === pid)
  if (!a) return 'Not observed yet'
  if (a.lastRequestAccount) return `${label(s, a.lastRequestAccount)} ${a.state === 'mismatch' ? '(mismatch)' : 'verified'}`
  return adoptionWords(a.state)
}

function RunningClients($: EngineInterface, c: Ctx): JSX.Element {
  const { Box } = c.els
  const s = c.s!
  const env = myEnv(s)
  if (!env) return Line(c, 'No environment.')
  const groups = [...new Set(env.sessions.map(x => x.group).filter(Boolean))]
  return (
    <Box flexDirection="column">
      {Section(
        c,
        'Sessions',
        env.sessions.length
          ? Table(
              c,
              'clients',
              ['Session', 'State', 'Group', 'Last request'],
              env.sessions.map(x => [
                `${x.sessionId === Q.sessionId ? '\u25b8 ' : ''}${sessionName(x)}`,
                `${x.status ?? '?'}${x.version ? ` \u00b7 ${x.version}` : ''}`,
                x.group ?? '?',
                requestIdentity(s, env, x.sessionId, x.pid),
              ]),
            )
          : Line(c, 'No running session is listed.', { dim: true }),
        Hint(c, `\u25b8 marks this session. Environment ${env.id} (shared login)${env.inPlace.governingVersion ? `; the oldest running Claude, ${env.inPlace.governingVersion}, must pass the version check` : ''}.`),
      )}
      {Alert(c, 'Other clients may exist', 'Editor extensions may not be listed; every client of this login adopts a change at its next request.')}
      {groups.length > 1 ? Line(c, `Sessions here work in different groups (${groups.join(', ')}): a shared switch must be permitted by each.`, { tone: 'warn' }) : null}
      {Hint(c, 'Projects are shown here locally; they are never copied into routine logs or reports.')}
      {Row(
        c,
        groups.length > 1 && Btn(c, 'rc-conflict', 'Conflict', () => go($, { tab: 'overview', sub: 'why' }), true),
        Btn(c, 'rc-lanes', 'Lanes', () => go($, { tab: 'groups', sub: 'lanes' })),
        Btn(c, 'rc-close', 'Close', () => go($, { tab: 'overview' })),
      )}
    </Box>
  )
}

// ------------------------------------------------------------ S36-S43 limits and recovery

function Limits($: EngineInterface, c: Ctx): JSX.Element {
  const { Box } = c.els
  const s = c.s!
  const env = myEnv(s)
  const { x, error } = explanation($, c)
  if (!env) return Line(c, 'No environment.')
  const t = s.tuning
  const live = x?.candidates.find(k => k.status === 'live')
  const sooner = live?.exhausted ? x?.candidates.find(k => k.status === 'eligible') : undefined
  const exhausted = x?.candidates.filter(k => k.exhausted) ?? []
  const last = env.lastSwitch
  const recoveryLast = last && (last.how === 'recovery' || last.how === 'model_fallback') ? last : undefined
  const mode = x?.mode ?? env.mode
  return (
    <Box flexDirection="column">
      {Section(
        c,
        'Limits',
        mode !== 'automatic'
          ? Alert(
              c,
              'Headless jobs get a recommendation only',
              'In Suggest or Manual, an unattended claude -p job gets no prompt, hold, account change or transcript notice; its usage limit is handled as by Claude without Quayflow. The recommendation is recorded locally. Approve Automatic for the group before unattended switching is expected.',
            )
          : Line(c, 'A confirmed limit switches once and retries once.'),
        recoveryLast ? KV(c, 'Last recovery', `${recoveryLast.how === 'model_fallback' ? 'model not available on the plan: switched back to' : 'switched to'} "${label(s, recoveryLast.to)}" ${when(recoveryLast.requestedAt)} (${recoveryLast.reason})`) : null,
      )}
      {hintsOn(c) &&
        Section(
        c,
        'Confirm the failure before recovering',
        Table(c, 'confirm', ['Observed condition', 'Recovery check'], [
          ['No stop reason, text or tool call', 'Required failure shape'],
          ['Caller cancelled', 'No recovery'],
          ['Session rate-limit observation', `Binding at or above ${t.hardLimitPct}%`],
          ['Engine observation', 'Only if fresh'],
          ['Usage endpoint at failure time', 'Not called'],
        ]),
        KV(c, 'Per-failure budget', 'One switch; at most one retry of the same request'),
        Hint(c, 'An unknown or generic error changes no account: Claude\'s own result stays. A transport error is never read as an exhausted quota.'),
      )}
      {!x && Pending(c, 'the accounts', error)}
      {x && !x.allExhausted && Line(c, '\u2714 Not every account is at its limit', { dim: true })}
      {x?.allExhausted &&
        Section(
        c,
        'Every account at its limit',
        Table(
                c,
                'soonest',
                ['Account', 'Exhausted-window resets', 'Earliest usable'],
                exhausted.map(k => {
                  const windows = (account(s, k.account)?.usage?.windows ?? []).filter(w => w.pct >= x.hardLimitPct)
                  return [
                    label(s, k.account),
                    windows.map(w => `${windowName(w.key)} ${resetText(w.resetsAt, c.now)}`).join('; ') || 'unknown',
                    `${resetText(k.availableAt, c.now)}${x.soonest === k.account ? ' (selected)' : ''}`,
                  ]
                }),
              ),
        Alert(c, 'Claude handles the wait', `${x.soonest ? `Switch to "${label(s, x.soonest)}"` : 'No account has a known reset time'}; then Claude decides whether to wait or finish.`, 'note'),
        Hint(c, 'An account is usable only after every exhausted window resets. Unknown reset data is shown as unknown, never as a made-up time.'),
      )}
      {hintsOn(c) &&
        Section(
        c,
        'Claude owns the wait and the result',
        KV(c, 'Quayflow availability hold', 'None'),
        KV(c, 'Native wait state', 'Unknown: Claude does not report it to Quayflow'),
        KV(c, 'Native wait setting', 'Unknown: Quayflow does not read it'),
        KV(c, 'Caller deadline', 'Not reported to Quayflow'),
        Hint(c, 'Claude waits and resumes only when its own settings allow it; otherwise its usage-limit result stands. Quayflow does not keep the job alive, restart it, or append another result. There is no "stop waiting" command: cancel the job the usual way.'),
      )}
      {sooner &&
        Section(
          c,
          'Opens sooner',
          Line(c, `${label(s, sooner.account)} is eligible now (${candidateUsage(sooner)}).`),
          Hint(c, `${mode === 'automatic' ? 'Automatic mode can switch to it; a waiting client adopts it at its next recheck or request.' : 'Accept it with /quay accept when it is recommended.'} Quayflow sends no prompt to wake a waiting job and does not cancel or rewrite Claude's timer.`),
        )}
      {hintsOn(c) &&
        Section(
        c,
        'Unsafe to continue, cancelled, or exited',
        Hint(c, 'When an earlier external operation has no confirmed result, the caller cancelled, or the process exited, Quayflow does not replay the action, restart the job, or append a second result: the native result stands. There is no "continue anyway".'),
        Hint(c, 'Account state and task state are separate: the account may be healthy while a task needs your review.'),
      )}
      {Row(
        c,
        Btn(c, 'lim-activity', 'Activity', () => go($, { tab: 'activity' })),
        Btn(c, 'lim-mode', 'Group mode', () => go($, { tab: 'groups', sub: 'group', arg: myGroup(s) })),
        Btn(c, 'lim-close', 'Close', () => go($, { tab: 'overview' })),
      )}
    </Box>
  )
}

// ------------------------------------------------------------ S41 observation detail

function Observation($: EngineInterface, c: Ctx, id: string): JSX.Element {
  const { Box } = c.els
  const s = c.s!
  const a = account(s, id)
  const { x, error } = explanation($, c)
  const k = x?.candidates.find(z => z.account === id)
  const t = s.tuning
  const u = a?.usage
  return (
    <Box flexDirection="column">
      {Section(
        c,
        `Usage observation: ${a?.label ?? `account #${id}`}`,
        KV(c, 'Last good observation', u?.binding ? `${pct(u.binding.pct)} ${windowName(u.binding.key)} / ${age(u.ageS)} ago${u.fresh ? '' : ' (stale: excluded from automatic decisions)'}` : 'none'),
        !x ? Pending(c, 'the endpoint state', error) : null,
        k ? KV(c, 'Endpoint state', k.endpointNote ?? 'answering normally') : null,
        k?.backoffS ? KV(c, 'Backoff', `${k.backoffS} s (Retry-After respected)`) : null,
        k ? KV(c, 'Next check', k.nextCheckAt ? `scheduled ${clock(k.nextCheckAt)}; no burst refresh` : 'due now') : null,
      )}
      {Section(
        c,
        'Observation policy',
        Table(c, 'obs', ['Observation policy', 'Setting'], [
          ['Active / idle poll', `${seconds(t.livePollS)} / ${seconds(t.idlePollS)}`],
          ['Active / idle freshness', `${seconds(t.liveFreshS)} / ${seconds(t.idleFreshS)}`],
          ['Missing Retry-After', `doubling backoff, up to ${seconds(t.backoffMaxS)}`],
        ]),
      )}
      {Alert(c, 'The account may be used elsewhere', 'Allowance and endpoint limits are shared across machines. Local observations are timestamped snapshots, not reserved or globally current capacity.', 'warn')}
      {Row(c, Btn(c, 'obs-back', 'Back', () => go($, { tab: 'accounts', sub: 'detail', arg: id }), true))}
    </Box>
  )
}

// ------------------------------------------------------------ S44-S50 session actions

type HandoffResult = { how: 'open' | 'replace' | 'prepare'; dest: string; ok: boolean; fileCount?: number; command?: string; launched?: boolean; message?: string }

function destinations(s: StateView, env: EnvView | undefined): { value: string; label: string }[] {
  return s.envs
    .filter(e => e.id !== env?.id)
    .map(e => ({ value: e.id, label: `${e.id === 'default' ? 'default environment' : `lane ${e.id}`} / ${e.live.known ? label(s, e.live.account) : 'no registered login'}` }))
}

function destText(s: StateView, dest: string): string {
  const e = s.envs.find(x => x.id === dest)
  return `${dest === 'default' ? 'default environment' : `lane ${dest}`} / ${e?.live.known ? label(s, e.live.account) : 'no registered login'}`
}

function SessionActions($: EngineInterface, c: Ctx): JSX.Element {
  const sub = c.view.sub
  if (sub === 'open') return OpenReview($, c)
  if (sub === 'replace') return ReplaceReview($, c)
  if (sub === 'result') return HandoffOutcome($, c)
  const { Box } = c.els
  const s = c.s!
  const env = myEnv(s)
  const dests = destinations(s, env)
  const how = c.form['sa:how'] === 'replace' ? 'replace' : 'open'
  const dest = c.form['sa:dest'] ?? c.view.arg ?? dests[0]?.value
  return (
    <Box flexDirection="column">
      {Section(
        c,
        'Session actions',
        Toggle(c, 'sa:open', 'Open New Session (new window)', how === 'open', () => setForm($, 'sa:how', 'open')),
        Toggle(c, 'sa:replace', 'Replace Current Session (same terminal)', how === 'replace', () => setForm($, 'sa:how', 'replace')),
        Hint(c, 'Both carry the full retained history (messages, tool results, summaries, checkpoints). Replace needs the PowerShell prompt hook. For lanes and version fallback, not routine account switching: an ordinary switch moves the environment in place.'),
      )}
      {dests.length ? Pick(c, 'sa:dest', 'Destination', dests, dest, v => setForm($, 'sa:dest', v)) : Line(c, 'No other environment yet: create a lane first.', { tone: 'warn' })}
      {Alert(c, 'One active executor', 'Only one of the two sessions may continue the task.', 'note')}
      {Row(
        c,
        dest && Btn(c, 'sa-review-open', 'Review', () => go($, { tab: 'session', sub: 'open', arg: dest }), how === 'open'),
        dest && Btn(c, 'sa-review-replace', 'Review', () => go($, { tab: 'session', sub: 'replace', arg: dest }), how === 'replace'),
        !dests.length && Btn(c, 'sa-newlane', 'Create lane', () => go($, { tab: 'groups', sub: 'newlane' }), true),
        Btn(c, 'sa-inplace', 'Switch instead', () => go($, { tab: 'accounts' })),
      )}
    </Box>
  )
}

function checkFor($: EngineInterface, c: Ctx, dest: string): { check?: HandoffCheck; error?: string } {
  const r = useRead<HandoffCheck>($, c, 'handoff', `${dest}:${c.s?.seq ?? 0}`, () => actHandoffCheck($, dest), true)
  return { check: r.value && typeof r.value.shellSupported === 'boolean' ? r.value : undefined, error: r.error }
}

function OpenReview($: EngineInterface, c: Ctx): JSX.Element {
  const { Box } = c.els
  const s = c.s!
  const dest = c.view.arg ?? ''
  const { check, error } = checkFor($, c, dest)
  const own = c.form['sa:own'] === '1'
  return (
    <Box flexDirection="column">
      {Section(
        c,
        'Open new session',
        KV(c, 'Source', 'This conversation / preserved'),
        KV(c, 'Destination', destText(s, dest)),
        KV(c, 'Window', 'New terminal window'),
        KV(c, 'Context', 'Full retained history'),
        check?.blocked ? Line(c, `Not possible now: ${check.blocked}.`, { tone: 'warn' }) : !check ? Pending(c, 'the destination', error) : null,
      )}
      {Toggle(c, 'sa:own', 'Allow the destination to continue this task after source activity is resolved', own, v => setForm($, 'sa:own', v ? '1' : ''))}
      {Line(c, 'Quayflow copies the retained transcript and side files with per-file integrity checks before launching; the working directory and uncommitted changes stay as they are. The new window continues only after this session ends (/exit).')}
      {Alert(c, 'No duplicate continuation', 'Resolve pending work in this session before the destination continues. Keeping this session for reference is not permission for two active agents.', 'note')}
      {Row(
        c,
        Btn(c, 'sa-open-go', 'Open', () => {
          if (!own) return void flash($, 'warn', 'Tick the continuation box first: only one session may continue the task.')
          void doHandoff($, 'open', dest)
        }, true),
        Btn(c, 'sa-back', 'Back', () => go($, { tab: 'session', arg: dest })),
      )}
    </Box>
  )
}

function ReplaceReview($: EngineInterface, c: Ctx): JSX.Element {
  const { Box } = c.els
  const s = c.s!
  const dest = c.view.arg ?? ''
  const { check, error } = checkFor($, c, dest)
  if (!check) return Pending(c, 'this terminal', error)
  const own = c.form['sa:own'] === '1'
  if (!check.running || !check.shellSupported) {
    // S47: name the limitation before anything is committed.
    return (
      <Box flexDirection="column">
        {Section(
          c,
          'Same-terminal replacement is unavailable here',
          KV(c, 'Shell', check.shell ?? 'unknown'),
          Alert(
            c,
            !check.running ? 'This session is not listed as running' : `${check.shell ?? 'This shell'} is not PowerShell 7 or Windows PowerShell 5.1`,
            'Use Open New Session with full history. This does not change your shell or require you to replace claude.',
            'warn',
          ),
          Hint(c, 'Full-history preparation and the one-executor rule still apply. A prepared launch command is generated by the engine only after the history is copied.'),
        )}
        {Row(
          c,
          Btn(c, 'sa-alt-open', 'Open new', () => go($, { tab: 'session', sub: 'open', arg: dest }), true),
          Btn(c, 'sa-alt-prepare', 'Show command', () => void doHandoff($, 'prepare', dest)),
          Btn(c, 'sa-back', 'Back', () => go($, { tab: 'session', arg: dest })),
        )}
      </Box>
    )
  }
  const enableHook = c.form['sa:hook'] === '1'
  return (
    <Box flexDirection="column">
      {Section(
        c,
        'Replace in this terminal',
        KV(c, 'Shell', check.shell ?? 'PowerShell'),
        KV(c, 'Prompt hook', check.promptHook ? 'Enabled' : 'Not enabled'),
        KV(c, 'Destination', destText(s, dest)),
        KV(c, 'Context', 'Full retained history'),
        check.blocked ? Line(c, `Not possible now: ${check.blocked}.`, { tone: 'warn' }) : null,
      )}
      {!check.promptHook && Toggle(c, 'sa:hook', 'Enable Quayflow same-terminal resume integration for PowerShell (adds the prompt hook to your PowerShell profiles)', enableHook, v => setForm($, 'sa:hook', v ? '1' : ''))}
      {!check.promptHook && Hint(c, 'A separate opt-in from engine start and install. The hook loads in PowerShell windows opened afterwards: in this terminal, run the prepared command after /exit.')}
      {Line(c, 'This session exits only after the destination and history are prepared; the hook then resumes the retained session under the destination. It does not wrap or alias claude.')}
      {Toggle(c, 'sa:own', 'Allow the destination to continue this task after this session ends', own, v => setForm($, 'sa:own', v ? '1' : ''))}
      {Alert(c, 'Supported shells', 'PowerShell 7 and Windows PowerShell 5.1.')}
      {Row(
        c,
        Btn(c, 'sa-replace-go', !check.promptHook && enableHook ? 'Enable and replace' : 'Replace with full history', () => {
          if (!own) return void flash($, 'warn', 'Tick the continuation box first: only one session may continue the task.')
          void replaceWithHook($, dest, !check.promptHook && enableHook)
        }, true),
        Btn(c, 'sa-alt-open', 'Open new', () => go($, { tab: 'session', sub: 'open', arg: dest })),
        Btn(c, 'sa-back', 'Back', () => go($, { tab: 'session', arg: dest })),
      )}
    </Box>
  )
}

async function replaceWithHook($: EngineInterface, dest: string, install: boolean): Promise<void> {
  if (install) {
    const r = await run($, 'installing the PowerShell prompt hook', () => actShellInstall($))
    if (!r) return
    if (!r.ok) return void flash($, 'warn', `The prompt hook was not installed: ${r.text || 'no reason given'}. Nothing was moved.`)
  }
  await doHandoff($, 'replace', dest)
}

async function doHandoff($: EngineInterface, how: 'open' | 'replace' | 'prepare', dest: string): Promise<void> {
  let result: HandoffResult
  try {
    await flash($, 'info', 'Working: preparing the full history')
    const r: HandoffResponse = await actHandoff($, how, dest)
    result = { how, dest, ok: r.ok, fileCount: r.fileCount, command: r.command, launched: r.launched, message: r.message ?? undefined }
  } catch (err) {
    result = { how, dest, ok: false, message: (err as Error).message }
  }
  setForm($, 'sa:result', JSON.stringify(result))
  await clearForm($, 'sa:own', 'sa:hook')
  go($, { tab: 'session', sub: 'result' })
}

function HandoffOutcome($: EngineInterface, c: Ctx): JSX.Element {
  const { Box, Code } = c.els
  const s = c.s!
  let r: HandoffResult | undefined
  try {
    r = JSON.parse(c.form['sa:result'] ?? '') as HandoffResult
  } catch {}
  if (!r) return Line(c, 'No full-history move was prepared.')
  if (!r.ok) {
    return (
      <Box flexDirection="column">
        {Alert(c, 'Full-history transition stopped', r.message ?? 'The engine refused.', 'error')}
        {KV(c, 'Source history', 'Preserved')}
        {KV(c, 'Source task', 'Not restarted or replayed')}
        {KV(c, 'Temporary recovery data', 'Retained only as needed')}
        {Line(c, 'Review the cause and retry the preparation explicitly. A blank session or a summary is never substituted.')}
        {Row(c, Btn(c, 'ho-source', 'Back to source', () => go($, { tab: 'session', arg: r!.dest }), true), Btn(c, 'ho-activity', 'Details', () => go($, { tab: 'activity' })))}
      </Box>
    )
  }
  const files = `${r.fileCount ?? 0} file${r.fileCount === 1 ? '' : 's'}; source intact`
  const last =
    r.how === 'open'
      ? r.launched
        ? { mark: 'done' as const, title: 'Launch', desc: 'a new window opened; it continues once this session ends' }
        : { mark: 'current' as const, title: 'Launch', desc: 'no window could be opened: run the command below after /exit' }
      : r.how === 'replace'
        ? { mark: 'current' as const, title: 'Exit and resume', desc: 'end this session (/exit); the prompt hook resumes it in this terminal' }
        : { mark: 'current' as const, title: 'Launch', desc: 'run the command below after /exit' }
  return (
    <Box flexDirection="column">
      {Section(
        c,
        'Prepare history',
        Steps(c, [
          { mark: 'done', title: 'Validate destination', desc: 'account, environment and policy checks' },
          { mark: 'done', title: 'Copy retained transcript and side files', desc: files },
          { mark: 'done', title: 'Check per-file integrity', desc: 'a stale copy cannot overwrite newer history' },
          { mark: 'current', title: 'Resolve source task ownership', desc: 'end this session (/exit): no two executors of one task' },
          last,
        ]),
      )}
      {Alert(c, 'Full history prepared', `Destination: ${destText(s, r.dest)}. The original history remains preserved.`, 'ok')}
      {KV(c, 'Working directory', 'Unchanged')}
      {KV(c, 'Completed tool results', 'Retained; not replayed')}
      {KV(c, 'Source', 'Continues until you end it; reference only afterwards')}
      {r.message ? Line(c, r.message) : null}
      {r.command && (!r.launched || r.how !== 'open') ? Line(c, r.how === 'replace' ? 'Without the hook in this terminal, run after /exit:' : 'Run after /exit:') : null}
      {r.command && (!r.launched || r.how !== 'open') ? <Code key="ho-cmd" source={r.command} /> : null}
      {Hint(c, 'Future switches in a lane affect that lane\'s login only; linked shared settings and memory remain shared.')}
      {Row(c, Btn(c, 'ho-done', 'Overview', () => go($, { tab: 'overview' }), true), Btn(c, 'ho-lanes', 'Lane details', () => go($, { tab: 'groups', sub: 'lanes' })))}
    </Box>
  )
}

// ------------------------------------------------------------ S23 move a restricted session

function LaneMove($: EngineInterface, c: Ctx): JSX.Element {
  const { Box } = c.els
  const s = c.s!
  let blocked: { pid?: number; name?: string; cwd?: string; group?: string; sessionId?: string } = {}
  try {
    blocked = JSON.parse(c.form['conflict'] ?? '{}')
  } catch {}
  const name = blocked.name ?? `${(blocked.cwd ?? '?').split(/[\\/]/).filter(Boolean).pop()} (pid ${blocked.pid ?? '?'})`
  const lanes = s.lanes.map(l => ({ value: l.name, label: `${l.name} / ${l.live ? label(s, l.live) : 'no login yet'}` }))
  const dest = c.form['sa:dest'] ?? lanes[0]?.value
  const lane = s.lanes.find(l => l.name === dest)
  const mine = blocked.sessionId === Q.sessionId
  return (
    <Box flexDirection="column">
      {Section(
        c,
        'Move to lane',
        KV(c, 'Source', `${name}${mine ? ' (this session)' : ''} · group ${blocked.group ?? '?'}`),
        lanes.length ? Pick(c, 'sa:dest', 'Destination lane (required)', lanes, dest, v => setForm($, 'sa:dest', v)) : Line(c, 'No lane exists yet: create one first.', { tone: 'warn' }),
        lane ? KV(c, 'Destination login', lane.live ? label(s, lane.live) : 'none yet: switch an account into the lane first') : null,
        KV(c, 'Context', 'Full retained history'),
      )}
      {Alert(c, 'This is a session transition', 'It is not a silent change to a running process. Same-terminal replacement requires the supported PowerShell prompt hook.', 'note')}
      {Hint(c, 'The source stays protected until the destination and the history integrity checks succeed. Quayflow never moves a session into a lane automatically.')}
      {!mine && Line(c, 'The move is made from that session: open /quay there (Session actions). Quayflow does not close another terminal, and a headless job is not restarted.', { tone: 'warn' })}
      {Row(
        c,
        mine && dest && Btn(c, 'lm-review', 'Review', () => go($, { tab: 'session', sub: 'replace', arg: dest }), true),
        Btn(c, 'lm-create', 'Create lane', () => go($, { tab: 'groups', sub: 'newlane' }), !lanes.length),
        Btn(c, 'lm-cancel', 'Cancel', () => go($, { tab: 'switch', sub: 'conflict', arg: c.view.arg })),
      )}
    </Box>
  )
}

// ------------------------------------------------------------ S25 create a lane

function NewLane($: EngineInterface, c: Ctx): JSX.Element {
  const { Box } = c.els
  const s = c.s!
  const free = s.accounts.filter(a => !a.liveIn.length && (a.health === 'ok' || a.health === 'stale_access'))
  const opts = [{ value: '', label: '(none yet)' }, ...free.map(a => ({ value: a.id, label: `${a.label} / ${a.groups.join(', ') || 'no group'}` }))]
  return (
    <Box flexDirection="column">
      {Section(c, 'New lane', Hint(c, 'Shared and private state, before anything is created.'))}
      {Field($, c, 'newlane:name', 'Lane name (required)', { placeholder: 'studio', submitLabel: 'set', onSubmit: () => {} })}
      {Pick(c, 'newlane:acct', 'Account for this lane', opts, c.form['newlane:acct'] ?? '', v => setForm($, 'newlane:acct', v))}
      {Table(c, 'lane-links', ['Linked to your normal Claude setup', 'Kept separate'], [
        ['settings.json, CLAUDE.md', 'Login, .claude.json'],
        ['plugins/, skills/, agents/, commands/', 'Session folder and history'],
        ['Memory folders', 'Organization policy files'],
      ])}
      {Alert(c, 'Changes to linked setup are shared', 'Editing linked settings or memory affects your normal setup too. Organization policy is fetched per login and is never linked.', 'warn')}
      {KV(c, 'Administrator rights', 'Not required')}
      {Hint(c, 'A lane is not a separate allowance: two environments on one account share its usage. In-place switching in the lane waits for its check to pass.')}
      {Row(
        c,
        Btn(c, 'nl-create', 'Create lane', () => void createLane($, c), true),
        Btn(c, 'nl-cancel', 'Cancel', () => go($, { tab: 'groups', sub: 'lanes' })),
      )}
    </Box>
  )
}

async function createLane($: EngineInterface, c: Ctx): Promise<void> {
  const name = (c.form['newlane:name'] ?? '').trim()
  if (!name) return void flash($, 'warn', 'Type a lane name first (Enter in the field).')
  const acct = c.form['newlane:acct'] ?? ''
  const lane = await run($, 'creating the lane and linking your setup', () => actLaneCreate($, name))
  if (!lane) return
  let note = `Lane "${lane.name}" created; links ${lane.linksOk ? 'ok' : `need repair (${lane.linkDetail.join('; ')})`}; in-place switching there ${lane.canaryPassed ? 'enabled (check passed)' : 'waits for its check'}.`
  if (acct) {
    const r = await actSwitchTo($, acct, false, lane.name).catch(err => ({ ok: false, message: (err as Error).message }))
    note += r.ok ? ` "${label(c.s ?? undefined, acct)}" is stored as its login.` : ` Its login was not set: ${r.message ?? 'refused'}.`
  }
  await clearForm($, 'newlane:name', 'newlane:acct')
  go($, { tab: 'groups', sub: 'lanes' })
  await flash($, 'info', note)
}

// ------------------------------------------------------------ S59 version and canary

function Version($: EngineInterface, c: Ctx): JSX.Element {
  const { Box, Code } = c.els
  const s = c.s!
  const env = myEnv(s)
  const v = env?.inPlace.governingVersion ?? c.claudeVersion
  const rec = s.compat.find(r => r.version === v) ?? s.compat.find(r => r.version === c.claudeVersion)
  const adopted = env?.adoption.filter(a => a.state === 'adopted').length ?? 0
  const pendingN = env?.adoption.filter(a => a.state === 'pending_next_request').length ?? 0
  const canary = useRead<CanaryStatus | null>($, c, 'canary', String(s.seq), () => actCanary($), true)
  const consent = !!s.consents.liveCanary
  return (
    <Box flexDirection="column">
      {Section(
        c,
        'Version',
        Table(c, 'version', ['Check', 'Result'], [
          ['Binary lock and entry markers', rec ? (rec.staticPass ? 'Passed' : `Failed${rec.staticDetail?.length ? `: ${rec.staticDetail.join('; ')}` : ''}`) : 'Not checked yet'],
          ['Oldest running version', v || 'unknown'],
          ['Live adoption evidence', rec?.livePass === true ? 'Passed' : rec?.livePass === false ? `Failed${rec.liveDetail ? `: ${rec.liveDetail}` : ''}` : 'Checked at the next real switch'],
          ['Latest adoption evidence', env?.adoption.length ? `${adopted} observed client(s) adopted; ${pendingN} pending request(s)` : 'none yet'],
          ['In-place switching here', env ? (env.inPlace.allowed ? 'Allowed' : `Off: ${env.inPlace.reason}`) : '?'],
        ]),
        Line(c, `Mod ${MOD_VERSION} · engine ${s.engine.version} (protocol ${s.engine.protocol}) · Claude ${c.claudeVersion || '?'}`, { dim: true }),
      )}
      {rec && (!rec.staticPass || rec.livePass === false)
        ? Alert(c, 'In-place switching is disabled for this version', 'Use the reviewed full-history fallback (Session actions). Quayflow never restarts a headless job.', 'warn')
        : null}
      {Section(
        c,
        'Live canary (manual)',
        Line(c, 'A synthetic headless job (model haiku, a short Bash sleep, no task content) runs on a registered account; after its first request the default environment switches to your other registered account, and each request reports the identity it used. It uses a little allowance and takes about a minute.'),
        KV(c, 'Consent', consent ? `given ${when(s.consents.liveCanary)}` : 'not given'),
        KV(c, 'Status', canary.value ? `recording since ${when(canary.value.since)}: ${canary.value.steps.length} request(s) verified` : 'not recording'),
        Line(c, 'Run it yourself in a terminal (it uses real logins):'),
        <Code key="canary-cmd" source="quayflow canary --live" />,
        Hint(c, 'Static checks never perform a hidden live switch.'),
      )}
      {Row(
        c,
        Btn(c, 've-static', 'Static check', () => void run($, 'checking this Claude version', () => actCompatCheck($), () => 'Static check done; see the table above.'), true),
        Btn(c, 've-consent', consent ? 'Withdraw live-canary consent' : 'Give live-canary consent', () => void run($, 'recording the consent', () => actConsent($, 'liveCanary', !consent), r => (r.ok ? (consent ? 'Live-canary consent withdrawn.' : 'Live-canary consent given; run quayflow canary --live in a terminal.') : (r.message ?? 'Refused.')))),
        Btn(c, 've-fallback', 'Full history', () => go($, { tab: 'session' })),
        Btn(c, 've-update', 'Update', () => go($, { tab: 'settings', sub: 'update' })),
        Btn(c, 've-back', 'Back', () => go($, { tab: 'settings' })),
      )}
    </Box>
  )
}

// ------------------------------------------------------------ S61 update

function Update($: EngineInterface, c: Ctx): JSX.Element {
  const { Box, Code } = c.els
  const s = c.s!
  const busy = s.envs.some(e => e.transition && !['completed', 'cancelled', 'failed', 'uncertain'].includes(e.transition.state))
  return (
    <Box flexDirection="column">
      {Section(
        c,
        'Update',
        Line(c, 'Run in a terminal:'),
        <Code key="update-cmd" source="quayflow update" />,
        Steps(c, [
          { mark: 'todo', title: 'Locate the matched release pair', desc: 'engine release and Mod together' },
          { mark: 'todo', title: 'Verify the published engine hash', desc: 'on a mismatch nothing is installed and the previous pair stays' },
          { mark: 'todo', title: 'Update both components', desc: 'never during a transition' },
          { mark: 'todo', title: 'Reconnect with the versioned handshake', desc: 'compatibility checked before switching resumes' },
        ]),
        Hint(c, 'quayflow update --check only looks for a newer release.'),
        busy ? Line(c, 'A switch is in progress: the update waits until it ends.', { tone: 'warn' }) : null,
      )}
      {Alert(c, 'Signing status', `This engine is ${s.engine.integrity}. SHA-256 ${s.engine.exeSha256}.`, 'note')}
      {Row(c, Btn(c, 'up-back', 'Back', () => go($, { tab: 'settings' }), true))}
    </Box>
  )
}

// ------------------------------------------------------------ S62 uninstall

function Uninstall($: EngineInterface, c: Ctx): JSX.Element {
  const { Box, Code } = c.els
  const s = c.s!
  const vault = c.form['un:vault'] === '1'
  const logs = c.form['un:logs'] === '1'
  const all = c.form['un:data'] === '1'
  const busy = s.envs.find(e => e.transition && !['completed', 'cancelled', 'failed', 'uncertain'].includes(e.transition.state))
  // --delete-data covers the vault and the logs, so it replaces those two flags.
  const cmd = all ? 'quayflow uninstall --delete-data' : `quayflow uninstall${vault ? ' --delete-vault' : ''}${logs ? ' --delete-logs' : ''}`
  return (
    <Box flexDirection="column">
      {Section(
        c,
        'Uninstall?',
        KV(c, 'Removed', 'Mod, startup entry, prompt hook, engine'),
        KV(c, 'Live Claude login', 'Left exactly as active at uninstall'),
        KV(c, 'Conversation history', 'Not deleted'),
      )}
      {Toggle(c, 'un:data', 'Delete all Quayflow data: vault, logs, settings and lanes (with their logins and history)', all, v => setForm($, 'un:data', v ? '1' : ''))}
      {!all && Toggle(c, 'un:vault', 'Also delete the saved logins (vault)', vault, v => setForm($, 'un:vault', v ? '1' : ''))}
      {!all && Toggle(c, 'un:logs', 'Also delete logs and saved support reports', logs, v => setForm($, 'un:logs', v ? '1' : ''))}
      {Alert(c, 'Let running switches finish first', busy ? `A switch in ${busy.id} is still in progress: let it end first. Unresolved recovery records are kept and reported.` : 'Unresolved recovery records are never deleted with the logs; they are reported and kept.', 'warn')}
      {Line(c, 'Uninstall runs outside Claude (it removes this Mod). Run in a terminal:')}
      <Code key="uninstall-cmd" source={cmd} />
      {Hint(c, 'Quayflow never signs you out or revokes a provider authorization.')}
      {Row(c, Btn(c, 'un-keep', 'Keep Quayflow', () => go($, { tab: 'settings' }), true))}
    </Box>
  )
}

// ------------------------------------------------------------ S63 operation detail

/** The operations list of the Activity tab (S63): each opens its sanitized trail. */
export function Operations($: EngineInterface, c: Ctx): JSX.Element {
  const s = c.s!
  const r = useRead<OperationView[]>($, c, 'operations', String(s.seq), () => actOperations($, 10), true)
  const ops = Array.isArray(r.value) ? r.value : undefined
  const { Button, Text } = c.els
  return Section(
    c,
    'Operations',
    ...(ops
      ? ops.map(o => (
          <Button key={`op:${o.id}`} plain onPress={() => go($, { tab: 'activity', sub: 'op', arg: o.id })}>
            {clock(o.ts)} {o.how ?? 'switch'} to {o.to ? label(s, o.to) : '?'} <Text dimColor>· {o.stage} · {o.id}</Text>
          </Button>
        ))
      : [Pending(c, 'the operations', r.error)]),
    ops && !ops.length ? Line(c, 'No switch recorded yet.', { dim: true }) : null,
  )
}

function Operation($: EngineInterface, c: Ctx, id: string): JSX.Element {
  const { Box } = c.els
  const s = c.s!
  const r = useRead<OperationView[]>($, c, 'operations', String(s.seq), () => actOperations($, 10), true)
  const op = Array.isArray(r.value) ? r.value.find(o => o.id === id) : undefined
  if (!op) return Pending(c, 'the operation', r.error)
  const detail = (rec: Record<string, unknown>): string =>
    Object.entries(rec)
      .filter(([k]) => k !== 'ts' && k !== 'event')
      .map(([k, v]) => `${k}=${typeof v === 'string' ? v : JSON.stringify(v)}`)
      .join(' ')
  return (
    <Box flexDirection="column">
      {Section(
        c,
        `Operation ${op.id}`,
        KV(c, 'Environment', op.env ?? '?'),
        KV(c, 'Cause', op.how ?? '?'),
        KV(c, 'From / to', `${op.from ? label(s, op.from) : '?'} -> ${op.to ? label(s, op.to) : '?'}`),
        KV(c, 'Last stage', op.stage),
      )}
      {Section(
        c,
        'Trail',
        Table(c, 'trail', ['Time', 'Event', 'Detail'], op.records.map(rec => [clock(Number(rec.ts)), String(rec.event ?? '?'), detail(rec)])),
      )}
      {Row(c, Btn(c, 'opd-back', 'Back', () => go($, { tab: 'activity' }), true))}
    </Box>
  )
}

// ------------------------------------------------------------ S64 report preview

function Report($: EngineInterface, c: Ctx): JSX.Element {
  const { Box, Code } = c.els
  const rep = c.report
  const reviewed = c.form['rp:reviewed'] === '1'
  return (
    <Box flexDirection="column">
      {Section(
        c,
        'Support report preview',
        KV(c, 'Included', 'Versions, opaque ids, stages, timings, sanitized reasons, settings'),
        KV(c, 'Excluded', 'Credentials, conversations, projects, account labels, e-mail addresses'),
        Hint(c, 'Nothing is saved until you review it; nothing is ever uploaded, and no issue or e-mail is created.'),
      )}
      {rep ? <Code key="report-text" source={rep.text} /> : Line(c, 'Build the preview to see exactly what the report contains.', { dim: true })}
      {rep && !rep.path && Toggle(c, 'rp:reviewed', 'I reviewed this report before saving it for sharing', reviewed, v => setForm($, 'rp:reviewed', v ? '1' : ''))}
      {rep?.path ? Line(c, `Saved locally: ${rep.path}. Share it yourself only if you choose to.`, { tone: 'ok' }) : null}
      {Row(
        c,
        !rep && Btn(c, 'rp-build', 'Build preview', () => void previewReport($), true),
        rep && !rep.path && Btn(c, 'rp-save', 'Save locally', () => void saveReport($, c), true),
        rep && Btn(c, 'rp-rebuild', 'Rebuild', () => void previewReport($)),
        Btn(c, 'rp-back', 'Back', () => go($, { tab: 'activity' })),
      )}
    </Box>
  )
}

async function previewReport($: EngineInterface): Promise<void> {
  const r = await run($, 'building the report preview', () => actReportPreview($), () => 'Preview built; nothing was saved.')
  if (r) {
    await setReport($, { id: r.id, text: r.text })
    await clearForm($, 'rp:reviewed')
  }
}

async function saveReport($: EngineInterface, c: Ctx): Promise<void> {
  const rep = c.report
  if (!rep?.id) return
  if (c.form['rp:reviewed'] !== '1') return void flash($, 'warn', 'Tick the review box first: a report is saved only after you reviewed it.')
  const r = await run($, 'saving the reviewed report', () => actReportSave($, rep.id!), r => `Saved locally: ${r.path}. Nothing was uploaded.`)
  if (r) {
    await setReport($, { text: rep.text, path: r.path })
    await clearForm($, 'rp:reviewed')
  }
}

// ------------------------------------------------------------ S57 policy and plan changes

function PolicyChange($: EngineInterface, c: Ctx): JSX.Element {
  const { Box } = c.els
  const s = c.s!
  const env = myEnv(s)
  const learned = s.accounts.filter(a => (a.unavailableModels?.length ?? 0) > 0)
  return (
    <Box flexDirection="column">
      {Section(
        c,
        'Policy changes',
        KV(c, 'Live login', env?.live.known ? label(s, env.live.account) : 'not registered'),
        KV(c, 'Next request', 'Claude re-fetches its organization policy'),
        Alert(c, 'Restrictions may change', 'The destination\'s policy applies. Quayflow does not copy permissions or force the previous environment\'s policy onto the new login; it rechecks eligibility after each policy change.', 'note'),
        Table(c, 'policy', ['New condition', 'Result'], [
          ['Enforced login no longer matches', 'The account becomes ineligible; the administrator\'s rule is named'],
          ['Current model or context unsupported', 'Excluded; a "model not found" answer switches back once'],
          ['Actual authentication path changed', 'Incompatible switching pauses; billing is not changed'],
        ]),
      )}
      {learned.length
        ? Section(c, 'Learned plan exclusions', ...learned.map(a => Line(c, `${a.label}: cannot serve ${a.unavailableModels!.join(', ')}`)))
        : null}
      {Row(
        c,
        Btn(c, 'pc-policy', 'Policy', () => go($, { tab: 'trust', sub: 'managed' }), true),
        Btn(c, 'pc-plan', 'Plan recovery', () => go($, { tab: 'overview', sub: 'limits' })),
        Btn(c, 'pc-groups', 'Groups', () => go($, { tab: 'groups' })),
      )}
    </Box>
  )
}
