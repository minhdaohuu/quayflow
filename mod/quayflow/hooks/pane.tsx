// The /quay pane (T43-T48): tabs Overview, Accounts, Groups, Activity and
// Settings drawn from host elements, plus the flows in flows.tsx and the review
// screens in review.tsx. Tabs are a row of buttons with hotkeys 1-5; Tab and the
// arrows walk the controls.
import type { Elements, EngineInterface, RenderSurface } from 'claude-code'

import type { AccountView, EnvView, IdentityView, Mode, QuayView, StateView, Window } from '../types'
import { MOD_VERSION, Q, flash, readPaneState, setCacheEntry, setDisclosure } from './context'
import {
  actAssign,
  actEngine,
  actGroupDelete,
  actIdentity,
  actLaneCheck,
  actLaneDelete,
  actMembers,
  actPin,
  actPoll,
  actPreferred,
  actRank,
  actRegister,
  actRelabel,
  actRemove,
  actSetDefault,
  actSetMode,
  actSetTuning,
  actSigninConfirm,
  actGroupCreate,
  actTuning,
  actUnassign,
  actUnpin,
} from './actions'
import { account, age, clock, healthText, isTerminal, label, myEnv, myGroup, orgName, pathCovers, pct, persistentWarnings, transitionWords, when, windowLine, windowName } from './format'
import { hasCode } from './engine'
import { Conflict, OffView, Onboarding, SwitchProgress, SwitchReview, Trust, afterConfirm, afterRegister } from './flows'
import { HomeBlock, Operations, Review } from './review'
import { GROUP, fleet } from './chart'
import { Alert, Badge, Btn, Card, Field, Gauge, Paint, canPaint, Hint, KV, Line, Pick, Row, Section, Table, Toggle, clearForm, gaugeTone, go, hintsOn, run, setForm, toneColor, useRead } from './ui'
import type { Ctx } from './ui'
import { DORG_ON_TEXT, groupOf, homeOf, identityName, minutes, pauseRemedy, seconds, switchHeading } from './words'

const TABS = [
  { id: 'overview', label: 'Overview' },
  { id: 'accounts', label: 'Accounts' },
  { id: 'groups', label: 'Groups' },
  { id: 'activity', label: 'Activity' },
  { id: 'settings', label: 'Settings' },
]

export async function renderPane(
  $: EngineInterface,
  e: { surface: RenderSurface; props: { bodyColumns?: number } },
  els: Elements['terminal'],
  claudeVersion: string,
): Promise<JSX.Element> {
  const c: Ctx = {
    els,
    surface: e.surface,
    canType: e.surface !== 'mobile',
    cols: e.props.bodyColumns ?? 80,
    now: Date.now(),
    ...(await readPaneState($)),
    claudeVersion,
    ids: { n: 0 },
  }
  const { Box, Text } = els
  return (
    <Box flexDirection="column">
      {Header($, c)}
      {c.flash && (
        <Text color={c.flash.tone === 'error' ? 'error' : c.flash.tone === 'warn' ? 'warning' : undefined}>
          {c.flash.tone === 'error' ? 'Error: ' : c.flash.tone === 'warn' ? 'Note: ' : ''}
          {c.flash.text}
        </Text>
      )}
      <Box flexDirection="column">{safeBody($, c)}</Box>
    </Box>
  )
}

function Header($: EngineInterface, c: Ctx): JSX.Element {
  const { Box, Button, Text } = c.els
  const s = c.s
  const env = s ? myEnv(s) : undefined
  const tab = c.view.tab
  const hints = hintsOn(c)
  return (
    <Box flexDirection="column">
      <Box flexDirection="row" flexWrap="wrap" columnGap={1}>
        {TABS.map((t, i) => (
          <Button key={`tab:${t.id}`} label={t.label} hotkey={String(i + 1)} variant={t.id === tab ? 'primary' : undefined} onPress={() => go($, { tab: t.id })} />
        ))}
        <Button key="hints" label={hints ? '? Hints on' : '? Hints'} hotkey="h" dimColor={!hints} onPress={() => setForm($, 'ui:hints', hints ? 'off' : 'on')} />
        <Button key="close" label="Close" role="dismiss" onPress={() => void $.ui.close({ id: 'quay' }).catch(() => {})} />
      </Box>
      {c.link.on && s && env ? StatusStrip(c, s, env) : null}
    </Box>
  )
}

/**
 * One line under the tabs: a beacon for the environment's health, then the
 * live account, the mode, the group and what this session last used.
 */
function StatusStrip(c: Ctx, s: StateView, env: EnvView): JSX.Element {
  const { Box, Text } = c.els
  const warned = persistentWarnings(s, env).length > 0
  const beacon = env.automationPaused || warned ? 'warning' : env.live.known ? 'success' : 'error'
  const live = env.live.signedOut ? 'signed out' : env.live.known ? label(s, env.live.account) : 'unknown login'
  const v = s.session?.verifiedAccount
  const extra = [env.id === 'default' ? '' : env.id, v && v !== env.live.account ? `this session: ${label(s, v)}` : ''].filter(Boolean)
  const mode = `${env.mode.charAt(0).toUpperCase()}${env.mode.slice(1)}${env.automationPaused ? ' (paused)' : ''}`
  return (
    <Box flexDirection="row" flexWrap="wrap" columnGap={1}>
      <Text color={beacon}>{'\u25cf'}</Text>
      <Text bold>{live}</Text>
      <Text dimColor>{`\u00b7 ${mode} \u00b7 ${myGroup(s)}${extra.map(x => ` \u00b7 ${x}`).join('')}`}</Text>
    </Box>
  )
}

/** A view that fails to draw says so, with a way back; the pane itself stays usable. */
function safeBody($: EngineInterface, c: Ctx): JSX.Element {
  try {
    return Body($, c)
  } catch (err) {
    const { Box } = c.els
    return (
      <Box flexDirection="column">
        {Line(c, `This view could not be drawn: ${err instanceof Error ? err.message : String(err)}`, { tone: 'error' })}
        {Row(c, Btn(c, 'draw-back', 'Back', () => go($, { tab: 'overview' }), true))}
      </Box>
    )
  }
}

function Body($: EngineInterface, c: Ctx): JSX.Element {
  if (!c.link.on) return OffView($, c)
  const s = c.s
  if (!s) return Line(c, 'Reading the engine state...')
  if (!s.onboarded || c.view.tab === 'onboarding') return Onboarding($, c)
  const review = Review($, c)
  if (review) return review
  switch (c.view.tab) {
    case 'accounts':
      return Accounts($, c)
    case 'groups':
      return Groups($, c)
    case 'activity':
      return Activity($, c)
    case 'settings':
      return Settings($, c)
    case 'switch':
      return c.view.sub === 'review' || c.view.sub === 'accept' ? SwitchReview($, c) : c.view.sub === 'conflict' ? Conflict($, c) : SwitchProgress($, c)
    case 'trust':
      return Trust($, c)
    default:
      return Overview($, c)
  }
}

// ------------------------------------------------------------ Overview (S10)

function Overview($: EngineInterface, c: Ctx): JSX.Element {
  const { Box, Text } = c.els
  const s = c.s!
  const env = myEnv(s)
  if (!env) return Line(c, 'No credential environment is known yet.')
  const warnings = persistentWarnings(s, env)
  const unknown = !env.live.known && !env.live.signedOut
  const switcher = env.notices.some(n => n.kind === 'other_switcher')
  const managed = (env.managed.forceOrg?.length ?? 0) > 0 || !!env.managed.forceMethod
  const t = env.transition
  const sg = env.suggestion
  const paused = pauseRemedy(env)
  const last = env.lastSwitch
  const lastNotable = !t && last && ['return_home', 'model_fallback', 'recovery', 'automatic'].includes(last.how) && c.now - last.requestedAt < 6 * 3_600_000 ? last : undefined
  const n = env.sessions.length
  const liveAcct = env.live.known ? account(s, env.live.account) : undefined
  const others = s.accounts.filter(a => a.id !== liveAcct?.id)
  const reading = ensureFreshUsage($, c)
  const row = (key: string, ...children: unknown[]): JSX.Element => (
    <Box key={key} flexDirection="row" columnGap={1} flexWrap="wrap">
      {children as JSX.Element[]}
    </Box>
  )
  return (
    <Box flexDirection="column">
      {warnings.length > 0 &&
        Card(
          c,
          '⚠ Attention',
          'warn',
          ...warnings.map(w => Line(c, w)),
          paused ? Line(c, paused.text, { dim: true }) : null,
          Row(
            c,
            unknown && Btn(c, 'ov-unknown', 'Review login', () => go($, { tab: 'trust', sub: 'unknown' }), true),
            switcher && Btn(c, 'ov-switcher', 'Switcher', () => go($, { tab: 'trust', sub: 'switcher' })),
            managed && Btn(c, 'ov-managed', 'Managed login', () => go($, { tab: 'trust', sub: 'managed' })),
            s.dOrg.enabled && Btn(c, 'ov-dorg', 'Exception', () => go($, { tab: 'trust', sub: 'dorg' })),
            paused?.resume && Btn(c, 'ov-resume', 'Resume', () => void resume($), true),
            paused?.action && !unknown && !switcher && Btn(c, 'ov-remedy', paused.action.label, () => go($, paused.action!.view)),
          ),
        )}
      {t &&
        !isTerminal(t) &&
        row('ov-t', <Text color="claude">{'⇄'}</Text>, <Text>{`${switchHeading(s, t)}: ${transitionWords(t.state)}`}</Text>, Btn(c, 'ov-progress', 'View', () => go($, { tab: 'switch', sub: 'progress' }), true))}
      {liveAcct ? LiveCard($, c, liveAcct, reading) : null}
      {s.accounts.length === 0 &&
        Card(
          c,
          'No accounts yet',
          undefined,
          Line(c, 'Your Claude login is unchanged.', { dim: true }),
          !env.live.known && !env.live.signedOut ? Row(c, Btn(c, 'ov-register', 'Register login', () => go($, { tab: 'onboarding', sub: 'register' }), true)) : null,
        )}
      {others.length > 0 &&
        (canPaint(c) && s.accounts.length * GROUP <= c.cols ? FleetChart($, c, reading) : <Box flexDirection="column">{others.map(a => FleetRow($, c, a, 'ov-acct', reading))}</Box>)}
      {sg
        ? row(
            'ov-sg',
            <Text color="claude">{'★'}</Text>,
            <Text bold>{`Switch to ${label(s, sg.to)}?`}</Text>,
            Btn(c, 'ov-accept', 'Switch', () => go($, { tab: 'switch', sub: 'accept' }), true),
            Btn(c, 'ov-why', 'Why?', () => go($, { tab: 'overview', sub: 'why' })),
          )
        : row('ov-stay', <Text color="success">{'✔'}</Text>, <Text dimColor>{liveAcct ? `Staying on ${liveAcct.label}` : 'No change'}</Text>, Btn(c, 'ov-why', 'Why?', () => go($, { tab: 'overview', sub: 'why' })))}
      {Hint(c, sg ? sg.reason : env.decision || 'Stay on the current account.')}
      {env.pin &&
        row(
          'ov-pin-row',
          Badge(c, 'PINNED', 'accent'),
          <Text>{`${label(s, env.pin.account)} until ${clock(env.pin.until)}`}</Text>,
          Btn(c, 'ov-unpin', 'Unpin', () => void run($, 'unpinning', () => actUnpin($), r => (r.ok ? 'Pin cleared.' : (r.message ?? 'Nothing pinned.')))),
        )}
      {env.pin && Hint(c, `No suggestion, automatic switch or return home away from it until then, its window resets, or it reaches ${s.tuning.pinReleasePct}%.`)}
      {lastNotable && row('ov-last-row', <Text dimColor>{`↳ ${switchHeading(s, lastNotable)} ${clock(lastNotable.requestedAt)}`}</Text>, Btn(c, 'ov-last', 'View', () => go($, { tab: 'switch', sub: 'progress' })))}
      {HomeBlock($, c, env)}
      <Box flexDirection="row" columnGap={1} marginTop={1} flexWrap="wrap">
        <Text dimColor>{`${n} session${n === 1 ? '' : 's'}`}</Text>
        {Btn(c, 'ov-sessions', 'Sessions', () => go($, { tab: 'overview', sub: 'sessions' }))}
        {Btn(c, 'ov-actions', 'Actions', () => go($, { tab: 'session' }))}
        {Btn(c, 'ov-limits', 'Limits', () => go($, { tab: 'overview', sub: 'limits' }))}
        {Btn(c, 'ov-home', 'Home', () => go($, { tab: 'groups', sub: 'group', arg: myGroup(s) }))}
      </Box>
      {Hint(c, 'Other clients of this login (for example editor extensions) may not be listed. A switch moves the environment, not just this terminal; LIVE is the stored login, not proof of what each session used.')}
    </Box>
  )
}

/**
 * Usage is shown only while it is current. When any account's reading is not, the pane
 * asks the engine to read every account again (at most every 2 min); rows say
 * "reading..." meanwhile. Answers true while that read is in flight.
 */
let pollInFlight = false
function ensureFreshUsage($: EngineInterface, c: Ctx): boolean {
  const s = c.s
  if (!s) return false
  const e = c.cache['usage-poll']
  if (e?.key === 'reading' || pollInFlight) return true
  const stale = s.accounts.some(a => a.health !== 'refresh_rejected' && !a.usage?.fresh)
  if (!stale || (e && c.now - e.at < 120_000)) return false
  pollInFlight = true
  void setCacheEntry($, 'usage-poll', { key: 'reading', at: c.now })
  void actPoll($)
    .catch(() => undefined)
    .finally(() => {
      pollInFlight = false
      void setCacheEntry($, 'usage-poll', { key: 'done', at: Date.now() })
    })
  return true
}

/** A window's percentage, only when the account's reading is current. */
function currentPct(a: AccountView, w: Window | undefined): number | undefined {
  return a.usage?.fresh && w ? w.pct : undefined
}

/** Why an account shows no figures: reading now, not read yet, or why it cannot be read. */
function usageGap(a: AccountView, reading: boolean): string | undefined {
  if (a.usage?.fresh) return undefined
  if (reading) return 'reading…'
  const note = a.usageNote?.split(/[;(]/)[0]?.trim()
  if (note) return `unreadable: ${note}`
  return a.usage ? 'not current' : 'not read yet'
}

/** The group threshold and margin that color the gauges. */
function limits(s: StateView): { threshold: number; margin: number } {
  const g = groupOf(s, myGroup(s))
  return { threshold: g?.threshold ?? 85, margin: g?.margin ?? 10 }
}

/** The 5-hour, 7-day and per-model weekly windows of an account, as known. */
function windowsOf(a: AccountView): { five?: Window; seven?: Window; model: Window[] } {
  const ws = a.usage?.windows ?? []
  return {
    five: ws.find(w => w.key === '5h' || w.key === 'five_hour'),
    seven: ws.find(w => w.key === '7d' || w.key === 'seven_day'),
    model: ws.filter(w => w.key.startsWith('7d:')),
  }
}

/** "2 h 10 min" until a reset; nothing when unknown. */
function untilText(now: number, ms: number | null | undefined): string {
  if (!ms) return ''
  const m = Math.max(0, Math.round((ms - now) / 60_000))
  if (m < 60) return `${m} min`
  if (m < 48 * 60) return `${Math.floor(m / 60)} h${m % 60 ? ` ${m % 60} min` : ''}`
  return `${Math.round(m / 1440)} d`
}

/** The live account: name and marks, then one gauge per window (current readings only). */
function LiveCard($: EngineInterface, c: Ctx, a: AccountView, reading: boolean): JSX.Element {
  const { Box, Button, Text } = c.els
  const s = c.s!
  const env = myEnv(s)
  const { threshold, margin } = limits(s)
  const { five, seven, model } = windowsOf(a)
  const width = Math.max(10, Math.min(40, c.cols - 30))
  const gap = usageGap(a, reading)
  const home = homeOf(s)
  const canPin = !!env && !env.pin && (!home || home === a.id)
  const gauge = (name: string, w: Window | undefined): JSX.Element => {
    const p = currentPct(a, w)
    return (
      <Box key={`live:${name}`} flexDirection="row" columnGap={1}>
        <Box width={4} flexShrink={0}>
          <Text dimColor>{name}</Text>
        </Box>
        {Gauge(c, p, width, { threshold, margin })}
        <Box width={4} flexShrink={0} justifyContent="flex-end">
          <Text bold color={p != null ? toneColor(gaugeTone(p, threshold, margin)) : undefined} dimColor={p == null}>
            {p != null ? pct(p) : '–'}
          </Text>
        </Box>
        <Text dimColor>{p != null && w?.resetsAt ? `↻ ${untilText(c.now, w.resetsAt)}` : ''}</Text>
      </Box>
    )
  }
  return Card(
    c,
    null,
    undefined,
    <Box key="live:head" flexDirection="row" columnGap={1} flexWrap="wrap">
      <Button key={`ov-acct:${a.id}`} plain onPress={() => go($, { tab: 'accounts', sub: 'detail', arg: a.id })}>
        <Text bold>{a.label}</Text>
      </Button>
      {Badge(c, 'LIVE', 'ok')}
      {home === a.id ? Badge(c, 'HOME', 'accent') : null}
      {a.health !== 'ok' && a.health !== 'stale_access' ? Badge(c, a.health.replace('_', ' ').toUpperCase(), 'warn') : null}
      <Text dimColor>{gap ?? `${age(a.usage!.ageS)} ago`}</Text>
      {canPin && Btn(c, 'home-pin', 'Pin', () => void pinNow($, s))}
    </Box>,
    gauge('5h', five),
    gauge('7d', seven),
    ...model.map(w => gauge(w.key.slice(3), w)),
    Hint(c, `The fullest window binds. The line on each gauge marks the ${threshold}% threshold.`),
  )
}

async function pinNow($: EngineInterface, s: StateView): Promise<void> {
  await run(
    $,
    'pinning the current account',
    () => actPin($),
    p => `Pinned "${label(Q.state ?? s, p.account)}" until ${clock(p.until)}.`,
  )
}

const MINI = 8

/**
 * Why an account is not a switch target now, in a word or two for its row (the account's
 * detail and "Why?" give the engine's full reason). Nothing for what a badge already says
 * (LIVE, SIGN IN, VAULT); `warn` only for what needs the person.
 */
export function notTarget(reason: string): { text: string; warn?: boolean } | undefined {
  const r = reason.toLowerCase()
  if (r.includes('already live') || r.includes('refresh rejected') || r.includes('cannot be read')) return undefined
  if (r.includes('managed login')) return { text: 'blocked by your organization', warn: true }
  if (r.startsWith('not in group')) return { text: 'not in this group' }
  if (r.includes('works in group')) return { text: 'group conflict', warn: true }
  if (r.startsWith('plan cannot')) return { text: 'plan lacks this model' }
  if (r.startsWith('at its limit')) return { text: 'at its limit', warn: true }
  if (r.includes('worth a switch')) return { text: 'no room to gain' }
  if (r.includes('not observed') || r.includes('stale') || r.includes('expired while idle') || r === 'unknown') return { text: 'usage unknown' }
  return { text: 'not a target now' }
}

/** One account: a star when recommended, mini gauges (current readings only), its marks. */
function FleetRow($: EngineInterface, c: Ctx, a: AccountView, keyPrefix: string, reading: boolean): JSX.Element {
  const { Box, Button, Text } = c.els
  const s = c.s!
  const env = myEnv(s)
  const { threshold, margin } = limits(s)
  const { five, seven } = windowsOf(a)
  const recommended = env?.suggestion?.to === a.id
  const why = a.eligibility && !a.eligibility.eligible ? notTarget(a.eligibility.reason) : undefined
  const wide = c.cols >= 70
  const gap = usageGap(a, reading)
  const mini = (name: string, w: Window | undefined): JSX.Element => {
    const p = currentPct(a, w)
    return (
      <Box key={`fleet:${a.id}:${name}`} width={MINI + 9} flexDirection="row" columnGap={1}>
        <Text dimColor>{name}</Text>
        {Gauge(c, p, MINI, { threshold, margin })}
        <Text dimColor={p == null}>{p != null ? pct(p) : '–'}</Text>
      </Box>
    )
  }
  return (
    <Box key={`fleet:${a.id}`} flexDirection="row" columnGap={1}>
      <Box width={2}>
        <Text color="claude">{recommended ? '★' : ' '}</Text>
      </Box>
      <Box width={12} flexShrink={0}>
        <Button key={`${keyPrefix}:${a.id}`} plain onPress={() => go($, { tab: 'accounts', sub: 'detail', arg: a.id })}>
          <Text bold={recommended}>{a.label}</Text>
        </Button>
      </Box>
      {gap ? (
        <Text dimColor>{gap}</Text>
      ) : (
        <Box flexDirection="row" columnGap={1}>
          {mini('5h', five)}
          {wide && mini('7d', seven)}
        </Box>
      )}
      <Box flexDirection="row" columnGap={1} flexShrink={1}>
        {env && a.liveIn.includes(env.id) ? Badge(c, 'LIVE', 'ok') : null}
        {homeOf(s) === a.id ? Badge(c, 'HOME', 'accent') : null}
        {a.health === 'refresh_rejected' || a.health === 'vault_error' ? Badge(c, a.health === 'vault_error' ? 'VAULT' : 'SIGN IN', 'warn') : null}
        {why ? <Text color={why.warn ? 'warning' : undefined} dimColor={!why.warn}>{why.text}</Text> : null}
      </Box>
    </Box>
  )
}

/** One window of an account's detail: name, gauge, figure, reset. Current readings only. */
function UsageGauge(c: Ctx, w: Window, binding: boolean): JSX.Element {
  const { Box, Text } = c.els
  const { threshold, margin } = limits(c.s!)
  return (
    <Box key={`usage:${w.key}`} flexDirection="row" columnGap={1}>
      <Box width={9} flexShrink={0}>
        <Text dimColor>{windowName(w.key).replace(' ', '')}</Text>
      </Box>
      {Gauge(c, w.pct, Math.max(10, Math.min(30, c.cols - 40)), { threshold, margin })}
      <Text bold color={toneColor(gaugeTone(w.pct, threshold, margin))}>
        {pct(w.pct)}
      </Text>
      <Text dimColor>{`${w.resetsAt ? `↻ ${untilText(c.now, w.resetsAt)}` : ''}${binding ? ' · binding' : ''}`}</Text>
    </Box>
  )
}

/**
 * Every account side by side: a 5-hour and a 7-day column each, the threshold across,
 * ▼ over the live account and ★ over the recommended one; the names under it are the
 * buttons to each account. Accounts without a current reading say why, in words.
 */
function FleetChart($: EngineInterface, c: Ctx, reading: boolean): JSX.Element {
  const { Box, Button, Text } = c.els
  const s = c.s!
  const env = myEnv(s)
  const { threshold, margin } = limits(s)
  const live = (a: AccountView): boolean => !!env && a.liveIn.includes(env.id)
  const cols = s.accounts.map(a => {
    const { five, seven } = windowsOf(a)
    return { five: currentPct(a, five), seven: currentPct(a, seven), live: live(a), recommended: env?.suggestion?.to === a.id }
  })
  const gaps = s.accounts.flatMap(a => {
    const g = usageGap(a, reading)
    return g ? [`${a.label}: ${g}`] : []
  })
  return (
    <Box key="fleet" flexDirection="column" marginTop={1}>
      {Paint(c, 'fleet', fleet(cols, threshold, margin))}
      <Box flexDirection="row">
        {s.accounts.map((a, i) => (
          <Box key={`fleet-name:${a.id}`} width={GROUP} justifyContent="center">
            <Button key={`${live(a) ? 'ov-col' : 'ov-acct'}:${a.id}`} plain onPress={() => go($, { tab: 'accounts', sub: 'detail', arg: a.id })}>
              <Text bold={cols[i]!.live || cols[i]!.recommended} color={cols[i]!.recommended ? 'claude' : undefined}>
                {a.label.length > GROUP - 2 ? `${a.label.slice(0, GROUP - 3)}…` : a.label}
              </Text>
            </Button>
          </Box>
        ))}
      </Box>
      {gaps.length > 0 && <Text dimColor>{gaps.join(' · ')}</Text>}
      {Hint(c, `Columns: 5-hour and 7-day usage. Dashed line: the ${threshold}% threshold. ▼ live account, ★ recommended.`)}
    </Box>
  )
}

/** Resume ends only a pause by the person or a hold; the engine's state after it is reported as it is. */
async function resume($: EngineInterface): Promise<void> {
  await run(
    $,
    'resuming automation',
    () => actEngine($, 'resume'),
    r => {
      if (!r.ok) return r.message ?? 'Refused.'
      const env = Q.state ? myEnv(Q.state) : undefined
      return env?.automationPaused ? `Still paused: ${env.pausedReason ?? 'another cause'}.` : 'Automation resumed.'
    },
  )
}

// ------------------------------------------------------------ Accounts

function Accounts($: EngineInterface, c: Ctx): JSX.Element {
  const { Box } = c.els
  const s = c.s!
  const env = myEnv(s)
  const sub = c.view.sub
  if (sub === 'detail' && c.view.arg) return AccountDetail($, c, c.view.arg)
  if (sub === 'remove' && c.view.arg) return RemoveAccount($, c, c.view.arg)
  const reading = ensureFreshUsage($, c)
  return (
    <Box flexDirection="column">
      {s.accounts.map(a => FleetRow($, c, a, 'acct', reading))}
      {s.accounts.length === 0 && Line(c, 'No accounts yet. Your Claude login is unchanged.', { dim: true })}
      <Box marginTop={1}>
        {Row(
          c,
          env && !env.live.known && !env.live.signedOut && Btn(c, 'ac-register', 'Register login', () => go($, { tab: 'onboarding', sub: 'register' }), true),
          Btn(c, 'ac-add', 'Add account', () => go($, { tab: 'onboarding', sub: 'add' })),
        )}
      </Box>
    </Box>
  )
}

function AccountDetail($: EngineInterface, c: Ctx, id: string): JSX.Element {
  const { Box } = c.els
  const s = c.s!
  const a = account(s, id)
  if (!a) return Line(c, 'That account is no longer registered.')
  const env = myEnv(s)
  const isLive = !!env && a.liveIn.includes(env.id)
  const u = a.usage
  const idr = useRead<IdentityView>($, c, 'account-identity', a.id, () => actIdentity($, { account: a.id }))
  const who = idr.value && idr.value.account ? idr.value : undefined
  const home = homeOf(s) === a.id
  const fresh = !!u?.fresh
  const reading = ensureFreshUsage($, c)
  return (
    <Box flexDirection="column">
      {Section(
        c,
        a.label,
        KV(c, 'Identity', `${who?.display ?? (idr.pending ? 'checking\u2026' : 'no e-mail stored')}${who?.orgName ? ` \u00b7 ${who.orgName}` : a.orgName ? ` \u00b7 ${a.orgName}` : ''}`),
        KV(c, 'Plan', `${a.kind}${a.subscription ? ` \u00b7 ${a.subscription}` : ''}`),
        KV(c, 'Groups', `${a.groups.join(', ') || 'none'}${home ? ' \u00b7 home' : a.rank && a.rank !== 'normal' ? ` \u00b7 ${a.rank}` : ''}`),
        KV(c, 'Live in', isLive ? `${env!.id} (here)` : a.liveIn.length ? a.liveIn.join(', ') : 'none'),
        Hint(c, `Account #${a.id}. The full identity is shown here locally; it is never written to logs or reports.`),
      )}
      {Section(
        c,
        'Usage',
        ...(u && fresh ? u.windows.map(w => UsageGauge(c, w, u.binding?.key === w.key)) : [Line(c, usageGap(a, reading) ?? 'not read yet', { dim: true })]),
        u && fresh ? Line(c, `read ${age(u.ageS)} ago`, { dim: true }) : null,
        a.eligibility && !a.eligibility.eligible ? Line(c, `Not eligible: ${a.eligibility.reason}`, { tone: 'warn' }) : null,
        a.eligibility?.eligible ? Hint(c, `Eligible: ${a.eligibility.reason}`) : null,
        (a.unavailableModels?.length ?? 0) > 0 ? Line(c, `Unavailable models: ${a.unavailableModels!.join(', ')}`, { dim: true }) : null,
      )}
      {a.health !== 'ok' && Line(c, healthText(a.health), { tone: a.health === 'stale_access' ? undefined : 'warn' })}
      {Field($, c, `rename:${a.id}`, 'Rename', { placeholder: a.label, submitLabel: 'rename', onSubmit: v => void renameAccount($, c, a.id, v) })}
      {Row(
        c,
        !isLive && Btn(c, 'ad-switch', 'Switch', () => go($, { tab: 'switch', sub: 'review', arg: a.id }), true),
        Btn(c, 'ad-reauth', 'Sign in again', () => go($, { tab: 'onboarding', sub: 'add', arg: a.id }), a.health === 'refresh_rejected'),
        Btn(c, 'ad-observation', 'Details', () => go($, { tab: 'accounts', sub: 'observation', arg: a.id })),
        Btn(c, 'ad-remove', 'Remove', () => go($, { tab: 'accounts', sub: 'remove', arg: a.id })),
        Btn(c, 'ad-back', 'Back', () => go($, { tab: 'accounts' })),
      )}
    </Box>
  )
}

async function renameAccount($: EngineInterface, c: Ctx, id: string, value: string): Promise<void> {
  const v = value.trim()
  if (!v) return
  await run($, 'renaming', () => actRelabel($, id, v), r => (r.ok ? `Renamed to "${v}".` : (r.message ?? 'Refused.')))
  await clearForm($, `rename:${id}`)
}

function RemoveAccount($: EngineInterface, c: Ctx, id: string): JSX.Element {
  const { Box } = c.els
  const s = c.s!
  const a = account(s, id)
  if (!a) return Line(c, 'That account is no longer registered.')
  if (a.liveIn.length) {
    return (
      <Box flexDirection="column">
        {Alert(c, 'Removal blocked: account in use', `"${a.label}" is the stored login of ${a.liveIn.join(', ')}. Switch that environment to another account before removing its protected copy.`, 'warn')}
        {KV(c, 'Retained on removal', 'Claude history and live provider authorization')}
        {Row(c, Btn(c, 'rm-overview', 'Environments', () => go($, { tab: 'overview', sub: 'sessions' }), true), Btn(c, 'rm-cancel', 'Cancel', () => go($, { tab: 'accounts', sub: 'detail', arg: id })))}
      </Box>
    )
  }
  return (
    <Box flexDirection="column">
      {Section(
        c,
        `Remove "${a.label}" from Quayflow?`,
        Line(c, 'Its registration and protected copy are deleted, and it is withdrawn from its groups for future switches. Running work is not moved.'),
        Hint(c, 'It does not sign out, revoke the account at the provider, or delete conversation history.'),
      )}
      {Row(
        c,
        Btn(c, 'rm-cancel', 'Keep account', () => go($, { tab: 'accounts', sub: 'detail', arg: id }), true),
        Btn(c, 'rm-confirm', 'Remove', () => void run($, 'removing', () => actRemove($, id), r => (r.ok ? `Removed "${a.label}" from Quayflow.` : (r.message ?? 'Refused.'))).then(r => r?.ok && go($, { tab: 'accounts' }))),
      )}
    </Box>
  )
}

// ------------------------------------------------------------ Groups

function Groups($: EngineInterface, c: Ctx): JSX.Element {
  const sub = c.view.sub
  if (sub === 'group' && c.view.arg) return GroupDetail($, c, c.view.arg)
  if (sub === 'automatic' && c.view.arg) return ApproveAutomatic($, c, c.view.arg)
  if (sub === 'disclosure') return Disclosure($, c)
  if (sub === 'lanes') return Lanes($, c)
  const { Box, Button, Text } = c.els
  const s = c.s!
  // The engine resolves the assignment; the path shown is the longest one that covers this folder.
  const assigned =
    s.session?.groupSource === 'assigned'
      ? [...s.projects].sort((a, b) => b.path.length - a.path.length).find(p => pathCovers(p.path, Q.cwd))
      : undefined
  const groupOpts = s.groups.map(g => ({ value: g.name, label: g.name }))
  const chosen = c.form['assign-group'] ?? myGroup(s)
  return (
    <Box flexDirection="column">
      {Hint(c, 'A group is a data approval: work in projects using it may be processed under any of its accounts.')}
      {s.groups.map(g => (
        <Box key={`grp-row:${g.name}`} flexDirection="row" columnGap={1}>
          <Button key={`grp:${g.name}`} plain onPress={() => go($, { tab: 'groups', sub: 'group', arg: g.name })}>
            <Text bold>{g.name}</Text>
          </Button>
          {g.isDefault ? Badge(c, 'DEFAULT', 'muted') : null}
          {g.mixed ? Badge(c, 'MIXED', 'warn') : null}
          <Text dimColor>{`${g.mode}${g.mode === 'automatic' && !g.automaticApprovedAt ? ' (not approved)' : ''} \u00b7 ${g.accounts.map(id => label(s, id)).join(', ') || 'no accounts'}${g.preferred ? ` \u00b7 home ${label(s, g.preferred)}` : ''}`}</Text>
        </Box>
      ))}
      {Hint(c, 'DEFAULT: used by projects without an explicit assignment. It is not all accounts: accounts of other groups are not implicitly added to it. MIXED: more than one identity boundary.')}
      {Section(
        c,
        'This project',
        KV(c, 'Path', Q.cwd || '?'),
        KV(c, 'Group', `${myGroup(s)} (${assigned ? `assigned: ${assigned.path}` : 'default group'})`),
        Pick(c, 'assign-group', 'Assign to', groupOpts, chosen, v => setForm($, 'assign-group', v)),
        Hint(c, 'Running sessions are not silently broadened: review them before applying a wider set of destinations. The assignment is stored locally; repository content can suggest settings but never grants account access.'),
        Row(
          c,
          Btn(c, 'gr-assign', 'Save', () => void run($, 'assigning', () => actAssign($, Q.cwd, chosen), r => (r.ok ? 'Assignment saved. Running jobs keep the destinations they started with.' : (r.message ?? 'Refused.')))),
          Btn(c, 'gr-sessions', 'Sessions', () => go($, { tab: 'overview', sub: 'sessions' })),
          assigned && Btn(c, 'gr-unassign', 'Use default', () => void run($, 'removing the assignment', () => actUnassign($, assigned.path), r => (r.ok ? `This project now uses the default group "${s.defaultGroup}".` : (r.message ?? 'Refused.')))),
        ),
      )}
      {Field($, c, 'new-group', 'New group', { placeholder: 'name', submitLabel: 'create', onSubmit: v => void createGroup($, c, v) })}
      {Row(c, Btn(c, 'gr-lanes', 'Lanes', () => go($, { tab: 'groups', sub: 'lanes' })))}
    </Box>
  )
}

async function createGroup($: EngineInterface, c: Ctx, name: string): Promise<void> {
  const n = name.trim()
  if (!n) return
  const r = await run($, 'creating the group', () => actGroupCreate($, n), r => (r.ok ? `Group "${n}" created (Suggest mode, no accounts).` : (r.message ?? 'Refused.')))
  if (r?.ok) {
    await clearForm($, 'new-group')
    go($, { tab: 'groups', sub: 'group', arg: n })
  }
}

const MODE_WORDS: Record<Mode, string> = {
  manual: 'Manual — only explicit account actions',
  suggest: 'Suggest — interactive /quay accept; headless: recommendation only',
  automatic: 'Automatic — proactive switching, return home and bounded limit recovery',
}

function GroupDetail($: EngineInterface, c: Ctx, name: string): JSX.Element {
  const { Box } = c.els
  const s = c.s!
  const g = s.groups.find(x => x.name === name)
  if (!g) return Line(c, `No group "${name}".`)
  const nonMembers = s.accounts.filter(a => !g.accounts.includes(a.id))
  const modeOpts = (['manual', 'suggest', 'automatic'] as const).map(m => ({ value: m, label: m === 'automatic' && !g.automaticApprovedAt ? `${MODE_WORDS[m]} (needs approval)` : MODE_WORDS[m] }))
  const rankOpts = ['preferred', 'normal', 'backup'].map(r => ({ value: r, label: r === 'preferred' ? 'preferred (home)' : r }))
  const t = s.tuning
  return (
    <Box flexDirection="column">
      {Section(
        c,
        `Group ${g.name}${g.isDefault ? ' (default)' : ''}`,
        g.mixed ? Line(c, `Mixed identities${g.mixedCovered ? ': disclosure accepted for this account set' : ': needs the one-time disclosure'}.`, { tone: g.mixedCovered ? undefined : 'warn' }) : null,
      )}
      {Pick(c, `mode:${g.name}`, 'Mode', modeOpts, g.mode, v => void changeMode($, c, g.name, v as Mode))}
      {KV(c, 'Proactive threshold', `${g.threshold}% of the binding window`)}
      {KV(c, 'Margin / cooldown', `${g.margin} points / ${seconds(t.cooldownS)}`)}
      {Field($, c, `thr:${g.name}`, `Threshold % (now ${g.threshold})`, { placeholder: String(g.threshold), submitLabel: 'save', onSubmit: v => void saveTuning($, c, g.name, v, undefined) })}
      {Field($, c, `mar:${g.name}`, `Margin points (now ${g.margin})`, { placeholder: String(g.margin), submitLabel: 'save', onSubmit: v => void saveTuning($, c, g.name, undefined, v) })}
      {Field($, c, 'cooldown', `Cooldown seconds, every group (now ${t.cooldownS})`, { placeholder: String(t.cooldownS), submitLabel: 'save', onSubmit: v => void saveGlobal($, 'cooldown', v) })}
      {g.mode !== 'automatic' && Alert(c, 'Before enabling Automatic', 'This group approves its accounts for assigned project context. Shared-session conflicts, plan compatibility, version checks and managed login still apply.', 'note')}
      {Hint(c, 'All exhausted: Quayflow selects the soonest-available eligible account, then Claude\'s own usage-limit behavior applies. There is no Quayflow availability hold.')}
      {Section(
        c,
        'Members',
        ...g.accounts.map(id =>
          Row(
            c,
            Line(c, `${label(s, id)}${g.preferred === id ? ' [Home]' : ''}`),
            Pick(c, `rank:${g.name}:${id}`, 'rank', rankOpts, g.preferred === id ? 'preferred' : (g.ranks?.[id] ?? 'normal'), v => void changeRank($, c, g.name, id, v)),
            Btn(c, `rm:${g.name}:${id}`, 'Remove from group', () => void changeMembers($, c, g.name, [], [id])),
          ),
        ),
        g.accounts.length === 0 ? Line(c, 'No accounts yet.', { dim: true }) : null,
        Hint(c, 'A nearly exhausted home account does not defeat a clearly suitable alternative; return home needs fresh usage below the threshold minus the margin. Accounts do not share the prompt cache: the first request after a switch processes the full context again.'),
      )}
      {nonMembers.length > 0 &&
        Pick(c, `add:${g.name}`, 'Add account', [{ value: '', label: '(choose)' }, ...nonMembers.map(a => ({ value: a.id, label: a.label }))], '', v => v && void changeMembers($, c, g.name, [v], []))}
      {Row(
        c,
        !g.isDefault && Btn(c, 'gd-default', 'Make default', () => void run($, 'setting the default', () => actSetDefault($, g.name), r => (r.ok ? `"${g.name}" is the default group now.` : (r.message ?? 'Refused.')))),
        g.preferred && Btn(c, 'gd-nohome', 'Clear home', () => void run($, 'clearing home', () => actPreferred($, g.name, null), r => (r.ok ? 'No home account: Quayflow stays on the current account until a trigger.' : (r.message ?? 'Refused.')))),
        g.preferred && Btn(c, 'gd-home', 'Return home', () => go($, { tab: 'overview' })),
        !g.isDefault && Btn(c, 'gd-delete', 'Delete group', () => void run($, 'deleting the group', () => actGroupDelete($, g.name), r => (r.ok ? `Group "${g.name}" deleted.` : (r.message ?? 'Refused.'))).then(r => r?.ok && go($, { tab: 'groups' }))),
        Btn(c, 'gd-back', 'Back', () => go($, { tab: 'groups' })),
      )}
    </Box>
  )
}

async function changeMode($: EngineInterface, c: Ctx, group: string, mode: Mode): Promise<void> {
  const approved = !!c.s?.groups.find(g => g.name === group)?.automaticApprovedAt
  if (mode === 'automatic' && !approved) return go($, { tab: 'groups', sub: 'automatic', arg: group })
  await run(
    $,
    'saving the mode',
    () => actSetMode($, group, mode, false),
    r => (r.ok ? `Group "${group}" is in ${mode} mode.` : (r.message ?? 'Refused.')),
    async err => {
      if (!hasCode(err, 'needs_approval')) return false
      go($, { tab: 'groups', sub: 'automatic', arg: group })
      return true
    },
  )
}

async function saveTuning($: EngineInterface, c: Ctx, group: string, threshold: string | undefined, margin: string | undefined): Promise<void> {
  const t = threshold !== undefined ? Number(threshold) : undefined
  const m = margin !== undefined ? Number(margin) : undefined
  if ((t !== undefined && !(t > 0 && t <= 100)) || (m !== undefined && !(m >= 0 && m < 100))) {
    return void flash($, 'warn', 'Threshold is a percentage (1-100); margin is points below it (0-99).')
  }
  await run($, 'saving tuning', () => actTuning($, group, t, m), r => (r.ok ? 'Tuning saved.' : (r.message ?? 'Refused.')))
  await clearForm($, `thr:${group}`, `mar:${group}`)
}

/** Cooldown seconds or pin minutes: one setting for every group. */
async function saveGlobal($: EngineInterface, what: 'cooldown' | 'pin', value: string): Promise<void> {
  const n = Number(value.trim())
  const ok = Number.isInteger(n) && (what === 'cooldown' ? n >= 0 && n <= 3600 : n >= 5 && n <= 1440)
  if (!ok) return void flash($, 'warn', what === 'cooldown' ? 'Cooldown is 0-3600 seconds.' : 'A pin lasts 5-1440 minutes.')
  await run($, 'saving the setting', () => actSetTuning($, what === 'cooldown' ? { cooldownS: n } : { pinMinutes: n }), r => r.message ?? (r.ok ? 'Saved.' : 'Refused.'))
  await clearForm($, what === 'cooldown' ? 'cooldown' : 'pin-minutes')
}

async function changeRank($: EngineInterface, c: Ctx, group: string, id: string, rank: string): Promise<void> {
  if (rank === 'preferred') {
    await run($, 'setting home', () => actPreferred($, group, id), r => (r.ok ? `"${label(c.s ?? undefined, id)}" is the home of ${group}; it replaces any earlier home.` : (r.message ?? 'Refused.')))
    return
  }
  await run($, 'saving the rank', () => actRank($, group, id, rank), r => (r.ok ? 'Rank saved.' : (r.message ?? 'Refused.')))
}

async function changeMembers($: EngineInterface, c: Ctx, group: string, add: string[], remove: string[]): Promise<void> {
  const r = await run($, 'saving membership', () => actMembers($, group, add, remove, false))
  if (!r) return
  if (r.needsDisclosure) {
    await setDisclosure($, { then: 'members', group, add, text: r.disclosure ?? '' })
    go($, { tab: 'groups', sub: 'disclosure' })
    return
  }
  await flash($, r.ok ? 'info' : 'warn', r.ok ? (remove.length ? 'Removed from the group: its approval for future switches is withdrawn. Running work is not moved.' : 'Added to the group.') : (r.message ?? 'Refused.'))
}

// ------------------------------------------------------------ S07 mixed-identity disclosure

function Disclosure($: EngineInterface, c: Ctx): JSX.Element {
  const { Box } = c.els
  const d = c.disclosure
  const s = c.s!
  if (!d) return Line(c, 'No pending disclosure.')
  const g = s.groups.find(x => x.name === d.group)
  const members = g?.accounts ?? []
  const joiningIds =
    d.then === 'members' ? (d.add ?? []) : d.then === 'register' ? [myEnv(s)?.live.account ?? ''] : [c.signin?.status.identity?.account ?? '']
  const live = useRead<IdentityView>($, c, 'identity', `env:${myEnv(s)?.id ?? '?'}:${myEnv(s)?.live.account ?? ''}`, () => actIdentity($, { env: myEnv(s)?.id }))
  const joiningName = (id: string): string => {
    const a = account(s, id)
    if (a) return identityName(a, `#${id}`)
    const who = d.then === 'register' ? live.value : undefined
    const kind = d.then === 'confirm' ? c.signin?.status.identity?.kind : who?.kind
    const name = d.label ?? `#${id}`
    return `${name} (${who?.orgName ?? (kind === 'personal' ? 'personal' : kind === 'organization' ? 'organization' : 'new identity')})`
  }
  const names = [...members.map(id => identityName(account(s, id), `#${id}`)), ...joiningIds.filter(id => id && !members.includes(id)).map(joiningName)]
  const approved = c.form['dc:approve'] === '1'
  const back: QuayView = d.then === 'members' ? { tab: 'groups' } : { tab: 'onboarding', sub: d.then === 'register' ? 'register' : 'add' }
  return (
    <Box flexDirection="column">
      {Section(
        c,
        'Mixed identities',
        Line(c, `Approve ${names.join(', ')} in ${d.group}?`, { tone: 'warn' }),
        Line(c, `Projects using ${d.group} may send their retained conversation context to any account in this group, including organization and personal accounts.`),
        g?.isDefault ? Line(c, `${d.group} is the default group: it includes unfamiliar and future projects without an explicit assignment. Your organization's rules still apply.`) : Line(c, "Your organization's rules on where its work may be processed still apply."),
        d.text ? Line(c, d.text, { dim: true }) : null,
      )}
      {Toggle(c, 'dc:approve', 'I approve these identities for work assigned to this group', approved, v => setForm($, 'dc:approve', v ? '1' : ''))}
      {Hint(c, 'Acceptance is recorded with this group: later switches within it do not repeat this prompt. A new identity boundary needs a new review, and jobs already running keep their destinations.')}
      {Row(
        c,
        Btn(c, 'dc-approve', 'Approve and save', () => {
          if (!approved) return void flash($, 'warn', 'Tick the approval box first; nothing was saved.')
          void approveDisclosure($, c)
        }, true),
        Btn(c, 'dc-other', 'Other group', () => void dropDisclosure($, back)),
        Btn(c, 'dc-cancel', 'Cancel', () => void dropDisclosure($, d.then === 'members' ? { tab: 'groups', sub: 'group', arg: d.group } : { tab: 'accounts' })),
      )}
    </Box>
  )
}

/** Resends the change that needed the disclosure, now with `acceptDisclosure: true`. */
async function approveDisclosure($: EngineInterface, c: Ctx): Promise<void> {
  const d = c.disclosure
  if (!d) return
  const doing = 'saving with the accepted disclosure'
  await clearForm($, 'dc:approve')
  if (d.then === 'register') {
    const a = await run($, doing, () => actRegister($, d.label ?? '', d.group, true))
    if (!a) return
    await setDisclosure($, null)
    await afterRegister($, c, a.label, d.group, a.note)
    return
  }
  if (d.then === 'confirm') {
    const r = await run($, doing, () => actSigninConfirm($, d.signinId ?? '', d.label ?? '', d.group, true, c.form['add:asnew'] === '1'))
    if (!r) return
    await setDisclosure($, null)
    await afterConfirm($, c, d.label ?? '', d.group, r.message ?? undefined)
    return
  }
  const r = await run($, doing, () => actMembers($, d.group, d.add ?? [], [], true), r =>
    r.ok ? `Group "${d.group}" saved; the disclosure acceptance is recorded with it.` : (r.message ?? 'Refused.'),
  )
  if (r?.ok) await dropDisclosure($, { tab: 'groups', sub: 'group', arg: d.group })
}

async function dropDisclosure($: EngineInterface, view: QuayView): Promise<void> {
  await setDisclosure($, null)
  await clearForm($, 'dc:approve')
  go($, view)
}

function ApproveAutomatic($: EngineInterface, c: Ctx, group: string): JSX.Element {
  const { Box } = c.els
  return (
    <Box flexDirection="column">
      {Section(
        c,
        `Approve Automatic for group ${group}?`,
        Line(c, 'Quayflow will then, without asking each time: switch proactively at the threshold, return to the home account after a reset, and recover from confirmed usage limits by switching and retrying the failed request once.'),
        Line(c, 'This applies to interactive sessions and to unattended headless jobs (claude -p) of projects using this group.'),
        Hint(c, 'Still enforced: group permissions, managed login, pins, cooldown, plan compatibility, version checks and adoption verification. A session is never moved into a lane automatically.'),
      )}
      {Row(
        c,
        Btn(c, 'au-approve', 'Approve', () => void run($, 'saving Automatic', () => actSetMode($, group, 'automatic', true), r => (r.ok ? `Group "${group}" is in Automatic mode.` : (r.message ?? 'Refused.'))).then(() => go($, { tab: 'groups', sub: 'group', arg: group }))),
        Btn(c, 'au-cancel', 'Keep mode', () => go($, { tab: 'groups', sub: 'group', arg: group }), true),
      )}
    </Box>
  )
}

// ------------------------------------------------------------ S24 lanes

function Lanes($: EngineInterface, c: Ctx): JSX.Element {
  const { Box } = c.els
  const s = c.s!
  const env = myEnv(s)
  const rows = s.envs.map(e => {
    const lane = s.lanes.find(l => l.name === e.id)
    return [
      `${e.id === 'default' ? 'Default shared' : `Lane ${e.id}`}${e.id === env?.id ? ' (this session)' : ''}`,
      `${e.live.known ? label(s, e.live.account) : e.live.signedOut ? 'no login yet' : 'not registered'} / ${e.governingGroups.join(', ') || '?'}`,
      e.id === 'default' ? (e.inPlace.allowed ? 'In-place allowed' : `Off: ${e.inPlace.reason}`) : lane?.canaryPassed ? 'Check passed' : 'Not checked',
    ]
  })
  return (
    <Box flexDirection="column">
      {Section(
        c,
        'Lane inventory',
        Table(c, 'lanes', ['Environment', 'Login / group', 'In-place status'], rows),
        Line(c, 'A lane has its own login, identity, history and organization policy; its linked setup (settings, CLAUDE.md, plugins, skills, agents, commands, memory) stays in sync with your normal Claude environment.'),
      )}
      {Alert(c, 'Not a separate allowance', 'Two environments using one account still share that account\'s allowance. A lane is not a security sandbox.')}
      {s.lanes.map(l =>
        Section(
          c,
          `Lane ${l.name}`,
          Line(c, `${l.sessions} session(s) listed · ${l.configDir}`, { dim: true }),
          Line(c, `Links ${l.linksOk ? 'ok' : `need repair: ${l.linkDetail.join('; ')}`} · in-place switching ${l.canaryPassed ? 'enabled (check passed)' : 'off until its check passes'}`, { tone: l.linksOk ? undefined : 'warn' }),
          Row(
            c,
            l.name !== env?.id && Btn(c, `ln-move:${l.name}`, 'Open full-history session', () => go($, { tab: 'session', arg: l.name })),
            Btn(c, `ln-check:${l.name}`, 'Check links', () => void run($, 'checking the lane', () => actLaneCheck($, l.name), r => r.message ?? (r.ok ? 'Lane checked.' : 'Check failed.'))),
            Btn(c, `ln-delete:${l.name}`, 'Delete lane', () => void run($, 'deleting the lane', () => actLaneDelete($, l.name), r => (r.ok ? `Lane "${l.name}" deleted.` : (r.message ?? 'Refused.')))),
          ),
        ),
      )}
      {Hint(c, 'Same-terminal replacement needs the PowerShell prompt hook; other shells get a new window or the exact prepared command.')}
      {Row(
        c,
        Btn(c, 'ln-create', 'Create lane', () => go($, { tab: 'groups', sub: 'newlane' }), true),
        Btn(c, 'ln-actions', 'Session actions', () => go($, { tab: 'session' })),
        Btn(c, 'ln-back', 'Back', () => go($, { tab: 'groups' })),
      )}
    </Box>
  )
}

// ------------------------------------------------------------ Activity (S63)

function Activity($: EngineInterface, c: Ctx): JSX.Element {
  const { Box } = c.els
  const s = c.s!
  const items = [...s.activity].sort((a, b) => b.ts - a.ts).slice(0, 40)
  return (
    <Box flexDirection="column">
      {Section(
        c,
        'Activity',
        items.length ? Table(c, 'activity', ['Time', 'Event', 'Result'], items.map(i => [when(i.ts), i.kind, i.text])) : Line(c, 'No activity recorded yet.', { dim: true }),
      )}
      {Operations($, c)}
      {Alert(c, 'Local only', 'No automatic telemetry. Reports exclude tokens, task content, source code, e-mail addresses, account labels, raw arguments and raw provider responses.')}
      {Hint(c, `Review a report before sharing it yourself; an opaque id is not a promise of anonymity. Routine logs are kept ${s.tuning.logRetentionDays} days (${s.tuning.logMaxMb} MB per day); unresolved journal records are kept until resolved.`)}
      {Row(c, Btn(c, 'act-report', 'Report', () => go($, { tab: 'activity', sub: 'report' }), true), Btn(c, 'act-close', 'Close', () => go($, { tab: 'overview' })))}
    </Box>
  )
}

// ------------------------------------------------------------ Settings

function Settings($: EngineInterface, c: Ctx): JSX.Element {
  const { Box } = c.els
  const s = c.s!
  const env = myEnv(s)
  const sub = c.view.sub
  const e = s.engine
  if (sub === 'stop') {
    return (
      <Box flexDirection="column">
        {Section(
          c,
          'Stop the engine?',
          Line(c, 'Claude keeps running unchanged. Quayflow stays off, and is not restarted on demand, until you start it again (/quay start or quayflow start).'),
          env?.transition && !isTerminal(env.transition) ? Line(c, 'A switch is in progress: the engine reconciles its real state when it starts again; it is not reported as unchanged.', { tone: 'warn' }) : null,
        )}
        {Row(
          c,
          Btn(c, 'st-cancel', 'Keep running', () => go($, { tab: 'settings' }), true),
          Btn(c, 'st-stop', 'Stop engine', () => void run($, 'stopping the engine', () => actEngine($, 'stop'), r => (r.ok ? 'Engine stopped.' : (r.message ?? 'Refused.')))),
        )}
      </Box>
    )
  }
  const consents = s.consents
  const managed = s.envs.filter(x => (x.managed.forceOrg?.length ?? 0) > 0 || x.managed.forceMethod)
  const paused = env ? pauseRemedy(env) : undefined
  const rec = s.compat.find(r => r.version === (env?.inPlace.governingVersion ?? c.claudeVersion))
  const tune = c.form['ui:tune'] === 'on'
  const consentList: [string, number | null | undefined][] = [
    ['engine start', consents.engineStart],
    ['sign-in start', consents.signinStartup],
    ['vault', consents.credentialStorage],
    ['terms', consents.terms],
    ['prompt hook', consents.promptHook],
    ['live canary', consents.liveCanary],
  ]
  const on = consentList.filter(([, at]) => at).map(([n]) => n)
  const off = consentList.filter(([, at]) => !at).map(([n]) => n)
  const versionWords = rec ? `${rec.staticPass ? 'checked' : 'check FAILED'}${rec.livePass === false ? ', live FAILED' : ''}` : 'not checked yet'
  return (
    <Box flexDirection="column">
      {Section(
        c,
        'Engine',
        KV(c, 'Engine', `${e.version} \u00b7 running \u00b7 ${consents.signinStartup ? 'starts at sign-in' : 'on demand'}`),
        KV(c, 'Claude', `${c.claudeVersion || '?'} \u00b7 ${versionWords}${env && !env.inPlace.allowed ? ' \u00b7 switching off' : ''}`, {
          tone: (rec && (!rec.staticPass || rec.livePass === false)) || (env && !env.inPlace.allowed) ? 'warn' : undefined,
        }),
        paused ? Line(c, paused.text, { tone: 'warn' }) : null,
        Hint(c, `Integrity: ${e.integrity} \u00b7 version ${e.version} \u00b7 SHA-256 ${e.exeSha256}`),
        Hint(c, `pid ${e.pid}, started ${when(e.startedAt)} \u00b7 protocol ${e.protocol} \u00b7 Mod ${MOD_VERSION} \u00b7 data ${e.dataDir}${env ? ` \u00b7 in-place switching ${env.inPlace.allowed ? 'allowed' : 'off'}: ${env.inPlace.reason}` : ''}`),
        Row(
          c,
          paused?.resume && Btn(c, 'se-resume', 'Resume', () => void resume($)),
          paused?.action && Btn(c, 'se-remedy', paused.action.label, () => go($, paused.action!.view)),
          !env?.automationPaused && Btn(c, 'se-pause', 'Pause', () => void run($, 'pausing', () => actEngine($, 'pause'), r => (r.ok ? 'Automation paused; manual switches still work.' : (r.message ?? 'Refused.')))),
          Btn(c, 'se-stop', 'Stop', () => go($, { tab: 'settings', sub: 'stop' })),
          Btn(c, 'se-version', 'Version', () => go($, { tab: 'settings', sub: 'version' })),
          Btn(c, 'se-update', 'Update', () => go($, { tab: 'settings', sub: 'update' })),
          Btn(c, 'se-uninstall', 'Uninstall', () => go($, { tab: 'settings', sub: 'uninstall' })),
        ),
      )}
      {Section(
        c,
        'Tuning',
        Line(c, `Cooldown ${seconds(s.tuning.cooldownS)} \u00b7 Pin ${minutes(s.tuning.pinMinutes)} \u00b7 Idle wait ${seconds(s.tuning.idleWaitS)}`),
        tune ? Field($, c, 'cooldown', 'Cooldown seconds', { placeholder: String(s.tuning.cooldownS), submitLabel: 'save', onSubmit: v => void saveGlobal($, 'cooldown', v) }) : null,
        tune ? Field($, c, 'pin-minutes', 'Pin minutes', { placeholder: String(s.tuning.pinMinutes), submitLabel: 'save', onSubmit: v => void saveGlobal($, 'pin', v) }) : null,
        Hint(c, `A pin is released early at ${s.tuning.pinReleasePct}% of a relevant window. Thresholds and margins are set per group (Groups).`),
        Row(c, Btn(c, 'se-tune', tune ? 'Done' : 'Edit', () => setForm($, 'ui:tune', tune ? 'off' : 'on'))),
      )}
      {Section(
        c,
        'Managed login',
        ...(managed.length
          ? managed.map(x => Line(c, `${x.id}: ${(x.managed.forceOrg ?? []).map(o => orgName(s, o)).join(', ') || 'no organization'}${x.managed.forceMethod ? ` \u00b7 method ${x.managed.forceMethod}` : ''}`))
          : [Line(c, 'None detected', { dim: true })]),
        s.dOrg.enabled ? Line(c, DORG_ON_TEXT, { tone: 'warn' }) : null,
        managed.length ? Hint(c, `Sources: ${managed.flatMap(x => x.managed.sources ?? []).join(', ') || 'unknown'}`) : null,
        Row(
          c,
          Btn(c, 'se-dorg', 'Exception', () => go($, { tab: 'trust', sub: 'dorg' })),
          Btn(c, 'se-policy', 'Policy changes', () => go($, { tab: 'trust', sub: 'policy' })),
        ),
      )}
      {Section(
        c,
        'Consents',
        on.length ? Line(c, `On: ${on.join(', ')}`) : null,
        off.length ? Line(c, `Off: ${off.join(', ')}`, { dim: true }) : null,
        Hint(c, `${consentList.filter(([, at]) => at).map(([n, at]) => `${n} since ${when(at)}`).join('; ')}. Change one in a terminal: quayflow consent <engine-start|signin-startup|credential-storage|terms|live-canary> <on|off>`),
      )}
      {Section(
        c,
        'Privacy',
        Line(c, `Logins: DPAPI, this Windows user \u00b7 logs ${s.tuning.logRetentionDays} days`),
        Hint(c, `No separate vault password. Logs at most ${s.tuning.logMaxMb} MB per day; unresolved journal records kept until reconciled; history-transfer copies only while a handoff needs them.`),
        Hint(c, 'No cloud vault, export or sync; protected-storage failure never falls back to plaintext. Before a manual switch away from an unknown login, a protected backup of it is stored.'),
        Hint(c, 'This does not protect against malware running as the same Windows user.'),
        Hint(c, 'Update the engine and this Mod together (hash-verified; never during a transition): quayflow update'),
      )}
    </Box>
  )
}
