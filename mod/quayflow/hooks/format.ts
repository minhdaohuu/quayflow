// Pure text helpers shared by the status line, /quay text output and the pane.
// Copy rules (design §7): no success wording when only stored, "Pending next
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
      return 'access token stale (Claude refreshes it at next use; not an error)'
    case 'refresh_rejected':
      return 'refresh token rejected: set aside; sign in again (Reauthenticate)'
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

/** `Quayflow · <mode> · <account label> <binding%> <window> · <notice>` (FR-07, S10/S11/S56). */
export function statusLine(link: QuayLink, s: StateView | undefined): string | undefined {
  if (!link.on) {
    if (link.reason === 'connecting' && !link.message) return undefined
    const lead = link.reason === 'no_consent' ? 'not set up' : 'off'
    return `Quayflow · ${lead} · ${linkOffText(link)}`
  }
  if (!s) return undefined
  if (!s.onboarded) return 'Quayflow · setup needed · /quay'
  const env = myEnv(s)
  if (!env) return 'Quayflow · no environment'
  const mode = env.mode
  const parts: string[] = [`Quayflow · ${mode}`]
  const live = env.live
  if (live.signedOut) parts.push('signed out')
  else if (!live.known) parts.push('unknown login')
  else {
    const a = account(s, live.account)
    parts.push(a ? `${a.label} ${a.usage?.binding ? usageText(a) : 'usage n/a'}` : (live.label ?? 'unknown login'))
  }
  const notes: string[] = persistentWarnings(s, env)
  const t = env.transition
  if (t && !isTerminal(t)) notes.push(`switching to ${label(s, t.to)}: ${transitionWords(t.state)}`)
  else if (env.suggestion) notes.push(`suggests ${label(s, env.suggestion.to)} · /quay accept`)
  else if (env.pin) notes.push(`pinned ${label(s, env.pin.account)} until ${clock(env.pin.until)}`)
  if (notes.length) parts.push(notes.join(' · '))
  return parts.join(' · ')
}

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
  const t = env.transition ?? env.lastSwitch
  if (t && (t.state === 'verifying_adoption' || t.state === 'completed')) {
    out.push({
      id: `stored:${t.id}`,
      text: `Quayflow stored "${label(s, t.to)}" for environment ${t.env} (${t.how.replace('_', ' ')}: ${t.reason}). Each client adopts it at its next request.`,
    })
  }
  for (const n of env.notices) {
    if (n.persistent || n.kind === 'suggestion' || n.level === 'info') continue
    out.push({ id: `notice:${n.id}`, text: `Quayflow: ${n.text}` })
  }
  return out
}

/** Ids that were already true when the session started: not news to it. */
export function seedAnnouncements(s: StateView): string[] {
  const env = myEnv(s)
  if (!env) return []
  const ids: string[] = []
  if (env.lastSwitch) ids.push(`stored:${env.lastSwitch.id}`)
  if (env.transition) ids.push(`stored:${env.transition.id}`)
  for (const n of env.notices) ids.push(`notice:${n.id}`)
  return ids
}
