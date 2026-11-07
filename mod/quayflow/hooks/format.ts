// Pure text helpers shared by the status line, /quay text output and the pane.
// Copy rules: no success wording when only stored, "Pending next
// request", no fake progress, unavailable usage is never 0%.
import type {
  AccountView,
  AdoptionState,
  EnvView,
  GroupView,
  Health,
  QuayLink,
  SessionInfo,
  StateView,
  Transition,
  TransitionState,
  Window,
} from '../types'

export function myEnv(s: StateView): EnvView | undefined {
  const id = s.session?.env ?? 'default'
  return s.envs.find(e => e.id === id) ?? s.envs.find(e => e.id === 'default') ?? s.envs[0]
}

export function myGroup(s: StateView): string {
  return s.session?.group ?? s.defaultGroup
}

export function account(s: StateView, id: string | null | undefined): AccountView | undefined {
  if (!id) return undefined
  return s.accounts.find(a => a.id === id)
}

/** Resolves a typed account reference (label, id, or id prefix) to an account. */
export function findAccount(s: StateView, ref: string): AccountView | undefined {
  const r = ref.trim().toLowerCase()
  return (
    s.accounts.find(a => a.id.toLowerCase() === r) ??
    s.accounts.find(a => a.label.toLowerCase() === r) ??
    s.accounts.find(a => r.length >= 4 && a.id.toLowerCase().startsWith(r))
  )
}

/**
 * Whether `folder` is `base` or inside it: a whole-component prefix (C:\code\app covers
 * C:\code\app\src but not C:\code\app-v2), case-insensitive as Windows paths are, with
 * either separator.
 */
export function pathCovers(base: string, folder: string): boolean {
  const norm = (p: string): string => p.replace(/\//g, '\\').toLowerCase()
  const b = norm(base).replace(/\\+$/, '')
  const f = norm(folder)
  if (!b) return false
  return f === b || f.replace(/\\+$/, '') === b || f.startsWith(`${b}\\`)
}

export function findGroup(s: StateView, name: string): GroupView | undefined {
  const n = name.trim().toLowerCase()
  return s.groups.find(g => g.name.toLowerCase() === n)
}

export function label(s: StateView | undefined, id: string | null | undefined): string {
  if (!id) return 'unknown'
  const a = s ? account(s, id) : undefined
  return a ? a.label : `account #${id}`
}

export function windowName(key: string): string {
  if (key === '5h' || key === 'five_hour') return '5 h'
  if (key === '7d' || key === 'seven_day') return '7 d'
  if (key.startsWith('7d:')) return `7 d ${key.slice(3)}`
  return key
}

export function pct(n: number): string {
  return `${Math.round(n)}%`
}

export function age(seconds: number): string {
  if (!Number.isFinite(seconds) || seconds < 0) return 'unknown age'
  if (seconds < 60) return `${Math.round(seconds)} s`
  if (seconds < 3600) return `${Math.round(seconds / 60)} min`
  if (seconds < 172800) return `${Math.round(seconds / 3600)} h`
  return `${Math.round(seconds / 86400)} d`
}

export function clock(ms: number | null | undefined): string {
  if (!ms) return '?'
  const d = new Date(ms)
  return `${String(d.getHours()).padStart(2, '0')}:${String(d.getMinutes()).padStart(2, '0')}`
}

export function when(ms: number | null | undefined): string {
  if (!ms) return '?'
  const d = new Date(ms)
  return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, '0')}-${String(d.getDate()).padStart(2, '0')} ${clock(ms)}`
}

/** "62% 5 h" with the observation age when stale; never 0% for missing usage. */
export function usageText(a: AccountView, long = false): string {
  const u = a.usage
  if (!u || !u.binding) return a.usageNote ? `usage unavailable (${a.usageNote})` : 'usage not observed yet'
  // An old reading is never shown as a figure: only that it is not current.
  if (!u.fresh) return `usage unknown (last read ${age(u.ageS)} ago)`
  const b = u.binding
  const base = `${pct(b.pct)} ${windowName(b.key)}`
  const stale = u.fresh ? '' : `, stale`
  const ageText = long || !u.fresh ? ` (observed ${age(u.ageS)} ago${stale})` : ''
  return base + ageText
}

export function windowLine(w: Window): string {
  const reset = w.resetsAt ? `, resets ${when(w.resetsAt)}` : ''
  return `${windowName(w.key)}: ${pct(w.pct)}${reset}`
}

export function healthText(h: Health): string {
  switch (h) {
    case 'ok':
      return 'credential healthy'
    case 'stale_access':
      return 'access token expired; refreshed at the next reading (not an error)'
    case 'refresh_rejected':
      return 'refresh token rejected: set aside; sign in again (Accounts → Sign in again)'
    case 'vault_error':
      return 'protected storage failed: no plaintext fallback; check the vault'
  }
}

export function transitionWords(state: TransitionState): string {
  switch (state) {
    case 'requested':
      return 'requested'
    case 'preparing':
      return 'preparing'
    case 'idle_wait':
      return 'waiting for an idle moment (optional, bounded)'
    case 'applying':
      return "applying under Claude's credential locks"
    case 'verifying_adoption':
      return 'stored; each client adopts it at its next request'
    case 'completed':
      return 'completed'
    case 'cancelled':
      return 'cancelled before a write; login unchanged'
    case 'failed':
      return 'did not apply; login unchanged'
    case 'uncertain':
      return 'transition already started; checking state'
  }
}

/** Why a transition did not apply (or is uncertain after the write), in words. */
export function failureWords(t: Transition): string | undefined {
  switch (t.failure) {
    case 'lock_held':
      return "a Claude credential lock is still held; the switch did not apply and the login is unchanged"
    case 'vault':
      return 'protected storage failed; the switch did not apply and nothing fell back to plaintext'
    case 'pre_write':
      return 'stopped before the write; the login is unchanged'
    case 'post_write':
      return 'ended after the write without certainty; Quayflow is reconciling the state'
    case 'cancelled':
      return 'cancelled before a write; the login is unchanged'
    default:
      return undefined
  }
}

/**
 * The enforcing organization by name where the engine knows it (a registered
 * account belongs to it), else by its opaque hash.
 */
export function orgName(s: StateView, hash: string): string {
  const byAccount = s.accounts.find(a => a.org === hash && a.orgName)?.orgName
  const enforcing = s.envs.filter(e => (e.managed.forceOrg ?? []).includes(hash))
  const only = enforcing.find(e => (e.managed.forceOrg ?? []).length === 1 && (e.managedOrgNames ?? []).length === 1)
  const name = byAccount ?? only?.managedOrgNames?.[0]
  return name ? name : `organization #${hash}`
}

/** "team account, Pro" style description of a login's kind and plan, when known. */
export function kindText(kind: string | null | undefined, subscription: string | null | undefined): string {
  return [kind, subscription].filter(Boolean).join(', ')
}

export function isTerminal(t: Transition): boolean {
  return t.state === 'completed' || t.state === 'cancelled' || t.state === 'failed' || t.state === 'uncertain'
}

export function adoptionWords(state: AdoptionState): string {
  switch (state) {
    case 'pending_next_request':
      return 'Pending next request'
    case 'adopted':
      return 'Adopted'
    case 'mismatch':
      return 'Mismatch'
  }
}

export function sessionName(x: SessionInfo): string {
  if (x.name) return x.name
  const project = (x.cwd ?? '').split(/[\\/]/).filter(Boolean).pop() ?? '?'
  return `${project} (pid ${x.pid})`
}

export function linkOffText(link: QuayLink): string {
  switch (link.reason) {
    case 'connecting':
      return link.message ?? 'connecting to the engine'
    case 'unreachable':
      return 'engine not running; Claude works as usual'
    case 'not_installed':
      return 'engine not installed (%LOCALAPPDATA%\\Quayflow\\bin\\quayflow.exe)'
    case 'no_consent':
      return 'not set up: run /quay to review consent and start'
    case 'stopped_by_user':
      return 'engine stopped by you: /quay start (or quayflow start)'
    case 'start_failed':
      return `engine did not start${link.message ? ` (${link.message})` : ''}`
    case 'incompatible':
      return `incompatible with engine ${link.engineVersion ?? ''}: switching and recovery are off; update with: quayflow update`.replace('  ', ' ')
    default:
      return 'off'
  }
}

/** Why automatic behavior and in-place switching may be unavailable here, as short warnings. */
export function persistentWarnings(s: StateView, env: EnvView | undefined): string[] {
  const out: string[] = []
  if (s.dOrg.enabled) out.push('managed-login exception ON')
  if (!env) return out
  const kinds = new Set(env.notices.filter(n => n.persistent).map(n => n.kind))
  if (kinds.has('incompatible_version') || !env.inPlace.allowed) out.push('in-place switching off for this Claude version')
  if (kinds.has('unknown_login') || (!env.live.known && !env.live.signedOut)) out.push('unknown login: /quay register or ignore')
  if (env.live.signedOut) out.push('signed out')
  if (kinds.has('other_switcher')) out.push('another switcher may be active')
  if (env.automationPaused || kinds.has('paused')) out.push('automation paused')
  for (const n of env.notices) {
    if (!n.persistent) continue
    if (['d_org_enabled', 'incompatible_version', 'unknown_login', 'other_switcher', 'paused', 'suggestion'].includes(n.kind)) continue
    out.push(n.text.length > 60 ? `${n.text.slice(0, 57)}...` : n.text)
  }
  return [...new Set(out)]
}

// ---------------------------------------------------------------- status line
//
// Design C01: `Quayflow · <Account> · <Mode> · <tail>`, one line, the tail the single
// most important fact. Priority: errors > unknown login / signed out > switching or
// pending > suggestion > pin > managed-login override > other warnings > stale usage >
// home / away > plain usage. Info-level notices never show here. The line is budgeted
// to the terminal width: the tail is cut, never the account label.

/** How the status line is drawn: the width it must fit, and whether usage figures show. */
export type StatusOptions = {
  /** Characters available (default 80). */
  width?: number
  /** False when Claude Code's own status line already shows the usage limits. */
  showUsage?: boolean
  /** The session's model id, when known: picks the per-model weekly window that applies. */
  model?: string
}

const DOT = ' · '

function modeWord(m: string): string {
  return m ? m[0]!.toUpperCase() + m.slice(1) : m
}

/** First clause of a notice, for a compact tail: "X needs a new sign-in; it is set aside." -> "X needs a new sign-in". */
function clause(text: string): string {
  const t = text.trim().split(/(?<=[^.]{3})[.;](\s|$)/)[0] ?? text
  return t.replace(/[.;:]+$/, '')
}

/**
 * Fits `head · tail` into `width`. `tails` runs from the fullest form to the shortest (usage
 * figures dropped first): the first that fits wins, else the shortest is cut with an
 * ellipsis. The head is never cut.
 */
function fit(head: string, tails: readonly string[], width: number): string {
  if (!tails.length) return head
  for (const t of tails) {
    if (head.length + DOT.length + t.length <= width) return head + DOT + t
  }
  const room = width - head.length - DOT.length
  if (room < 8) return head
  return `${head}${DOT}${tails[tails.length - 1]!.slice(0, room - 1).trimEnd()}…`
}

/** The line while Quayflow is off for this session: still one clean line. */
function offLine(link: QuayLink): string | undefined {
  switch (link.reason) {
    case 'connecting':
      return link.message ? `Quayflow · connecting · ${link.message}` : undefined
    case 'unreachable':
      return 'Quayflow · off · engine not running (Claude works as usual)'
    case 'not_installed':
      return 'Quayflow · off · engine not installed (Claude works as usual)'
    case 'no_consent':
      return 'Quayflow · not set up · run /quay to review and start'
    case 'stopped_by_user':
      return 'Quayflow · off · engine stopped by you (/quay start)'
    case 'start_failed':
      return `Quayflow · off · engine did not start${link.message ? ` (${link.message})` : ''}`
    case 'incompatible':
      return `Quayflow · off · update needed: incompatible with engine${link.engineVersion ? ` ${link.engineVersion}` : ''} (quayflow update)`
    default:
      return 'Quayflow · off'
  }
}

/** The account slot: the live account's label; never cut. */
function accountSlot(s: StateView, env: EnvView): string {
  const live = env.live
  if (live.signedOut) return 'Signed out'
  if (!live.known) return 'Unknown login'
  return account(s, live.account)?.label ?? live.label ?? label(s, live.account)
}

/**
 * A per-model weekly window ("7d:Opus", "7d:Sonnet 4.6") applies to a model id when its
 * family word appears in the id (the engine's rule); with no model known, every one applies.
 */
function windowApplies(key: string, model: string | undefined): boolean {
  if (!key.startsWith('7d:')) return true
  if (!model) return true
  const family = key.slice(3).trim().split(/\s+/)[0]?.toLowerCase() ?? ''
  return !!family && model.toLowerCase().includes(family)
}

/**
 * Usage figures of an account, fullest form first: `5h 45% · 7d 30% · Opus 7d 88%`, the
 * per-model weekly window only when it applies to the model and is the fullest applicable
 * window; then without it. Missing windows are left out; empty when none is observed.
 */
export function usageFigures(a: AccountView | undefined, model?: string): string[] {
  // Figures only from a current reading; an old one is reported as not current instead.
  const ws = a?.usage?.fresh ? a.usage.windows : []
  const find = (...keys: string[]) => ws.find(w => keys.includes(w.key) && Number.isFinite(w.pct))
  const base: string[] = []
  const h5 = find('5h', 'five_hour')
  const d7 = find('7d', 'seven_day')
  if (h5) base.push(`5h ${pct(h5.pct)}`)
  if (d7) base.push(`7d ${pct(d7.pct)}`)
  const applicable = ws.filter(w => Number.isFinite(w.pct) && windowApplies(w.key, model))
  const top = Math.max(-Infinity, ...applicable.map(w => w.pct))
  const own = applicable.filter(w => w.key.startsWith('7d:')).sort((x, y) => y.pct - x.pct)[0]
  const out: string[] = []
  if (own && own.pct >= top) out.push([...base, `${own.key.slice(3)} 7d ${pct(own.pct)}`].join(DOT))
  if (base.length) out.push(base.join(DOT))
  return out
}

/** The one tail worth showing, by priority; undefined when nothing needs saying. */
export function statusTail(s: StateView, env: EnvView, showUsage = true, model?: string): string | undefined {
  return statusTails(s, env, showUsage, model)[0]
}

/** The tail in its forms, fullest first (see `fit`); empty when nothing needs saying. */
function statusTails(s: StateView, env: EnvView, showUsage: boolean, model: string | undefined, flag: { urgent: boolean } = { urgent: false }): string[] {
  const one = (t: string | undefined): string[] => (t ? [t] : [])
  // Something the person must act on: drawn as a pinned status, not as the quiet footer.
  const act = (t: string | undefined): string[] => ((flag.urgent = true), one(t))
  const kinds = new Set(env.notices.filter(n => n.persistent && n.level !== 'info').map(n => n.kind))
  const paused = env.pausedReason ?? ''
  const live = env.live
  const liveAccount = live.known ? account(s, live.account) : undefined

  // 1. Errors: Quayflow cannot act safely here until something is fixed.
  if (/inconsistent/i.test(paused)) return act('mixed login files: switch to an account to repair')
  if (!env.inPlace.allowed || kinds.has('incompatible_version')) return act('in-place switching off for this Claude version')
  if (kinds.has('adoption_mismatch')) return act('a session did not adopt the switch: /quay')
  if (/interrupted switch/i.test(paused)) return act('interrupted switch: check the login, then /quay resume')
  for (const n of env.notices) {
    if (n.persistent && n.level === 'error') return act(clause(n.text))
  }

  // 2. The live login itself: not one Quayflow knows, or none at all.
  if (live.signedOut) return act('sign in to Claude (/login)')
  if (!live.known) return act('not registered: /quay register')
  if (kinds.has('other_switcher')) return act('another switcher may be active')
  if (env.automationPaused) return act(/by you/i.test(paused) || kinds.has('paused') ? 'automation paused (/quay resume)' : 'automation paused')

  // 3. A switch in flight, or one this session has not adopted yet.
  const t = env.transition
  const mine = s.session?.adoption
  if (t && !isTerminal(t) && t.state !== 'verifying_adoption') {
    return one(t.state === 'idle_wait' ? `switching to ${label(s, t.to)}: waiting for an idle moment` : `switching to ${label(s, t.to)}`)
  }
  if (mine?.state === 'pending_next_request' || (t?.state === 'verifying_adoption' && mine?.state !== 'adopted')) return one('pending next request')

  const figures = showUsage ? usageFigures(liveAccount, model) : []

  // 4. A suggestion: the usage behind it, and where to.
  const sg = env.suggestion
  if (sg) {
    const to = `${label(s, sg.to)} suggested`
    flag.urgent = true
    return [...figures.map(f => `${f} | ${to}`), to]
  }

  // 5. A manual pin, with its end.
  if (env.pin) {
    const until = `until ${clock(env.pin.until)}`
    return one(env.pin.account === live.account ? `pinned ${until}` : `pinned to ${label(s, env.pin.account)} ${until}`)
  }

  // 6. The managed-login override: persistent, never a passing toast (S56).
  if (s.dOrg.enabled || kinds.has('d_org_enabled')) return act('managed-login override ON')

  // 7. Other warnings (a set-aside account, a vault problem).
  for (const n of env.notices) {
    if (!n.persistent || n.level === 'info') continue
    if (['d_org_enabled', 'incompatible_version', 'unknown_login', 'other_switcher', 'paused', 'suggestion', 'adoption_mismatch'].includes(n.kind)) continue
    return act(clause(n.text))
  }

  // 8. Stale usage: the figures are not current enough to act on.
  const u = liveAccount?.usage
  if (u && u.binding && !u.fresh) return one(`usage stale (${age(u.ageS)})`)

  // 9. Home or away, when the group has a Preferred account.
  const group = s.groups.find(g => g.name === myGroup(s))
  if (group?.preferred) {
    const where = group.preferred === live.account ? 'Home account' : 'Away from home'
    return [...figures.map(f => where + DOT + f), where]
  }

  // 10. Plain usage.
  return figures
}

/** `Quayflow · <Account> · <Mode> · <tail>` (FR-07, design C01); undefined draws nothing. */
export function statusLine(link: QuayLink, s: StateView | undefined, opts: StatusOptions = {}): string | undefined {
  return statusParts(link, s, opts).text
}

/**
 * The status line and whether it needs the person (an error, a sign-in, a suggestion
 * to accept, the engine off): those are pinned; the everyday line is drawn quietly.
 */
export function statusParts(link: QuayLink, s: StateView | undefined, opts: StatusOptions = {}): { text: string | undefined; urgent: boolean } {
  const width = Math.max(40, Math.floor(opts.width ?? 80))
  const short = (line: string | undefined): string | undefined => {
    if (!line || line.length <= width) return line
    const [a, b, ...rest] = line.split(DOT)
    return fit(`${a}${DOT}${b}`, [rest.join(DOT)], width)
  }
  if (!link.on) return { text: short(offLine(link)), urgent: link.reason !== 'connecting' }
  if (!s) return { text: undefined, urgent: false }
  if (!s.onboarded) return { text: short('Quayflow · not set up · finish setup in /quay'), urgent: true }
  const env = myEnv(s)
  if (!env) return { text: 'Quayflow · no environment', urgent: true }
  const head = `Quayflow${DOT}${accountSlot(s, env)}${DOT}${modeWord(env.mode)}`
  const flag = { urgent: false }
  const text = fit(head, statusTails(s, env, opts.showUsage ?? true, opts.model, flag), width)
  return { text, urgent: flag.urgent }
}

// ---------------------------------------------------------------- transcript

/** Lines worth one transcript note each (interactive only), keyed so none repeats. */
export function announcements(s: StateView): { id: string; text: string; toast?: string }[] {
  const env = myEnv(s)
  if (!env) return []
  const out: { id: string; text: string; toast?: string }[] = []
  const sg = env.suggestion
  if (sg) {
    out.push({
      id: `suggestion:${sg.id}`,
      text: `Quayflow suggests "${label(s, sg.to)}": ${sg.reason}. /quay accept moves every session of this environment at its next request; nothing in progress is interrupted.`,
      toast: `Quayflow: switch to ${label(s, sg.to)}? /quay accept`,
    })
  }
  for (const n of env.notices) {
    if (n.persistent || n.kind === 'suggestion' || n.level === 'info') continue
    out.push({ id: `notice:${n.id}`, text: `Quayflow: ${n.text}` })
  }
  return out
}

/** Engine event kinds whose text is news for the person: told once in the transcript. */
const ANNOUNCED_EVENTS = new Set(['notice', 'switched', 'adoption'])

/**
 * Engine events to tell once (notices, switches, adoptions), for this session's
 * environment, from after `since` (the session's start): never older history, never a
 * text-less event (`usage`). Keyed by the engine run and the event's sequence.
 */
export function eventAnnouncements(
  s: StateView,
  events: readonly { seq: number; ts: number; kind: string; env?: string | null; text?: string | null }[],
  since: number,
): { id: string; text: string; toast?: string }[] {
  const env = myEnv(s)
  const run = `${s.engine.pid}:${s.engine.startedAt}`
  const out: { id: string; text: string; toast?: string }[] = []
  for (const e of events) {
    const text = e.text?.trim()
    if (!text || !ANNOUNCED_EVENTS.has(e.kind)) continue
    if (e.env && env && e.env !== env.id) continue
    if (e.ts < since) continue
    out.push({ id: `event:${run}:${e.seq}`, text: `Quayflow: ${text}` })
  }
  return out
}

/** Ids that were already true when the session started: not news to it. */
export function seedAnnouncements(s: StateView): string[] {
  const env = myEnv(s)
  if (!env) return []
  return env.notices.map(n => `notice:${n.id}`)
}
