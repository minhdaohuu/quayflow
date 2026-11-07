// Text helpers of the review screens (pure; no engine call). Shared by the pane
// and /quay text output.
import type { AccountView, CandidateView, EnvView, GroupView, QuayView, StateView, Transition } from '../types'
import { age, clock, label, myGroup, pct, when, windowName } from './format'

/** The home (Preferred) account of the project's group, if it names one. */
export function homeOf(s: StateView): string | undefined {
  return s.groups.find(g => g.name === myGroup(s))?.preferred ?? undefined
}

export function groupOf(s: StateView, name: string): GroupView | undefined {
  return s.groups.find(g => g.name === name)
}

/** Minutes as "2 h", "90 min". */
export function minutes(m: number): string {
  return m >= 60 && m % 60 === 0 ? `${m / 60} h` : `${m} min`
}

/** Seconds as "5 min", "45 s". */
export function seconds(sec: number): string {
  return sec >= 60 && sec % 60 === 0 ? `${sec / 60} min` : `${sec} s`
}

/** The home state in the design's words. */
export function homeWords(state: string): string {
  switch (state) {
    case 'home':
      return 'Home account'
    case 'ready':
      return 'Home ready'
    case 'deferred_pin':
      return 'Home ready, return deferred (pinned)'
    case 'deferred_cooldown':
      return 'Home ready, return deferred (cooldown)'
    case 'away':
      return 'Away from home'
    default:
      return 'No home account'
  }
}

export function candidateWord(status: string): string {
  switch (status) {
    case 'eligible':
      return 'Eligible'
    case 'excluded':
      return 'Excluded'
    case 'live':
      return 'Live'
    default:
      return 'Unknown'
  }
}

/** "47% 7 d · 4 min ago" for a candidate, or "usage unknown". */
export function candidateUsage(c: CandidateView): string {
  if (c.pct === undefined || c.pct === null) return 'usage unknown (never read as 0%)'
  if (!c.fresh) return `not current${c.ageS !== undefined && c.ageS !== null ? ` (read ${age(c.ageS)} ago)` : ''}`
  return `${pct(c.pct)} ${windowName(c.window ?? '?')}${c.ageS !== undefined && c.ageS !== null ? ` · ${age(c.ageS)} ago` : ''}${c.fresh ? '' : ' · stale'}`
}

/**
 * Why automation is paused and what ends it. Only a pause by the person, or a hold
 * left by an unresolved switch, ends on Resume; every other cause has its own remedy.
 */
export function pauseRemedy(env: EnvView): { resume: boolean; text: string; action?: { label: string; view: QuayView } } | undefined {
  if (!env.automationPaused) return undefined
  const reason = env.pausedReason ?? 'automation is paused'
  const by = env.pausedBy ?? (/paused by you/i.test(reason) ? 'user' : undefined)
  switch (by) {
    case 'user':
      return { resume: true, text: 'Automation paused by you. Manual switches still work.' }
    case 'hold':
      return { resume: true, text: `Automation held: ${reason}. Resume lifts the hold once you have checked the login.` }
    case 'repair':
      return { resume: false, text: `Automation held: ${reason}.`, action: { label: 'Choose an account', view: { tab: 'accounts' } } }
    case 'signed_out':
      return { resume: false, text: 'Automation paused: no login in this environment. Sign in with Claude (/login); a signed-out environment cannot be registered.' }
    case 'unknown_login':
      return { resume: false, text: `Automation paused: ${reason}.`, action: { label: 'Review unknown login', view: { tab: 'trust', sub: 'unknown' } } }
    case 'other_switcher':
      return { resume: false, text: `Automation paused: ${reason}.`, action: { label: 'Review live login', view: { tab: 'trust', sub: 'switcher' } } }
    case 'incompatible':
      return { resume: false, text: `Automation paused: ${reason}.`, action: { label: 'Version detail', view: { tab: 'settings', sub: 'version' } } }
    default:
      return { resume: false, text: `Automation paused: ${reason}.` }
  }
}

/** The phases of a switch as a timeline (C15): prepared, quiet moment, apply, verify. */
type Phase = { mark: 'done' | 'current' | 'todo' | 'failed'; title: string; desc: string }

export function phases(t: Transition, failed: boolean): Phase[] {
  const order = ['requested', 'preparing', 'idle_wait', 'applying', 'verifying_adoption', 'completed']
  const at = order.indexOf(t.state)
  const items = [
    { title: 'Prepared', desc: 'group, plan, managed-login and version checks' },
    { title: 'Quiet moment (optional)', desc: 'prefers no listed session busy; a timing preference, not a safety barrier' },
    { title: "Apply under Claude's credential locks", desc: 'refresh, legacy credentials, global configuration; unrelated keys unchanged' },
    { title: 'Verify on each next request', desc: 'per client; a storage write is not adoption' },
  ]
  // The step that stopped: verification after a write, else the apply.
  const stopped = t.writtenAt ? 3 : 2
  return items.map((i, k): Phase => {
    if (failed) return { ...i, mark: k < stopped ? 'done' : k === stopped ? 'failed' : 'todo' }
    if (at >= k + 2) return { ...i, mark: 'done' }
    if (at === k + 1 || (k === 0 && at <= 0)) return { ...i, mark: 'current' }
    return { ...i, mark: 'todo' }
  })
}

/** Heading of a switch by its cause (S34 variants are told apart). */
export function switchHeading(s: StateView, t: Transition): string {
  switch (t.how) {
    case 'return_home':
      return `Return to "${label(s, t.to)}" (home account)`
    case 'model_fallback':
      return `Plan rejection (model_not_found): back to "${label(s, t.to)}"`
    case 'recovery':
      return `Usage-limit recovery: switch to "${label(s, t.to)}"`
    case 'automatic':
      return `Automatic switch to "${label(s, t.to)}"`
    case 'accepted':
      return `Accepted suggestion: switch to "${label(s, t.to)}"`
    default:
      return `Switch to "${label(s, t.to)}"`
  }
}

/** Organization (or kind) of an account, for naming identity boundaries. */
export function identityName(a: AccountView | undefined, fallback: string): string {
  if (!a) return fallback
  if (a.kind === 'organization') return `${a.label} (${a.orgName ?? 'organization'})`
  return `${a.label} (personal)`
}

/** "17:00" when known, else "unknown" (never a made-up estimate). */
export function resetText(ms: number | null | undefined, now: number): string {
  if (!ms) return 'unknown'
  return ms - now < 86_400_000 ? clock(ms) : when(ms)
}

/** Wording of the managed-login exception, true to this build: the setting is recorded; enforcement stays. */
export const DORG_ON_TEXT =
  "Managed-login exception setting is ON (recorded only): this build still enforces your organization's login policy and never switches to an account it would refuse."
