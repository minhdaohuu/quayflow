// Session context shared by the hooks module's files, and the `$.state` atoms
// the pane draws from. Module variables start over on a reload (register and
// session.start run again); the atoms survive it.
import { atom, read, update } from 'claude-code'
import type { EngineInterface } from 'claude-code'

import type { QuayCacheEntry, QuayDisclosure, QuayFlash, QuayForm, QuayLink, QuayReport, QuaySignin, QuayView, SigninStatus, StateView } from '../types'

export const MOD_VERSION = '0.1.3'
export const PROTOCOL = 1
export const PANE_ID = 'quay'

export type Conn = { port: number; token: string; pid?: number; protocol?: number; version?: string }

/** Per-session facts and the engine link, owned by this module instance. */
export const Q = {
  sessionId: '',
  cwd: '',
  /** The running Claude Code version (`$.session.version()`), sent in the handshake. */
  claudeVersion: '',
  /** `claude -p` / SDK: no pane, prompt, toast, status or log line from Quayflow. */
  headless: true,
  started: false,
  conn: undefined as Conn | undefined,
  link: { on: false, reason: 'connecting' } as QuayLink,
  /** Settles once the first handshake (and engine start, if any) has an outcome. */
  ready: Promise.resolve() as Promise<void>,
  /** `quayflow start --if-consented` runs at most once per session (FR-19). */
  startAttempted: false,
  lastLinkTry: 0,
  /** Ticket of the newest `establish` attempt; each attempt takes the next one. */
  linkTicket: 0,
  /** The newest ticket handed out when the link last turned on. */
  linkOnTicket: 0,
  /** `establish` attempts still running: a pane showing "connecting" with none starts one. */
  linkAttempts: 0,
  state: undefined as StateView | undefined,
  seq: 0,
  requestCounter: 0,
  /** When this session (or this module instance, after a reload) started: older engine events are history. */
  startedAt: 0,
  /** The terminal's width as the last `/quay` saw it; the status line is budgeted to it. */
  columns: undefined as number | undefined,
  /** The `statusLineUsage` option: whether the status line shows usage figures. */
  usageDisplay: 'auto' as UsageDisplay,
  /** The model of this session's last main-thread request, for the per-model weekly window. */
  model: undefined as string | undefined,
}

/** `auto`: hide the usage % when Claude Code's own status line already shows the limits. */
export type UsageDisplay = 'auto' | 'show' | 'hide'

const linkAtom = atom({ plugin: 'quayflow', key: 'link' } as const, { on: false, reason: 'connecting' })
const snapshotAtom = atom({ plugin: 'quayflow', key: 'snapshot' } as const, null, { shape: 'state-v1' })
const viewAtom = atom({ plugin: 'quayflow', key: 'view' } as const, { tab: 'overview' })
const formAtom = atom({ plugin: 'quayflow', key: 'form' } as const, {})
const flashAtom = atom({ plugin: 'quayflow', key: 'flash' } as const, null)
const signinAtom = atom({ plugin: 'quayflow', key: 'signin' } as const, null)
const disclosureAtom = atom({ plugin: 'quayflow', key: 'disclosure' } as const, null)
const reportAtom = atom({ plugin: 'quayflow', key: 'report' } as const, null)
const announcedAtom = atom({ plugin: 'quayflow', key: 'announced' } as const, [])
const cacheAtom = atom({ plugin: 'quayflow', key: 'cache' } as const, {})

/** A request id for a mutating call: a repeat returns the first response (tech-spec §6). */
export function requestId(): string {
  Q.requestCounter += 1
  return `${Q.sessionId || 'mod'}-${Date.now().toString(36)}-${Q.requestCounter}`
}

// ---- UI gates: every UI call goes through these, and none reaches a headless run.

export function uiStatus($: EngineInterface, text: string | undefined): void {
  if (Q.headless) return
  try {
    $.ui.status(text)
  } catch {}
}

export function uiLog($: EngineInterface, text: string): void {
  if (Q.headless) return
  try {
    $.ui.log(text)
  } catch {}
}

export function uiToast($: EngineInterface, text: string): void {
  if (Q.headless) return
  try {
    $.ui.toast(text)
  } catch {}
}

/** The link the pane last drew (it survives a reload of this module; `Q.link` does not). */
export async function shownLink($: EngineInterface): Promise<QuayLink | undefined> {
  if (Q.headless) return undefined
  return read($, linkAtom).catch(() => undefined)
}

export async function setLink($: EngineInterface, link: QuayLink): Promise<void> {
  Q.link = link
  if (Q.headless) return
  await update($, linkAtom, () => link).catch(() => {})
}

export async function setSnapshot($: EngineInterface, s: StateView | null): Promise<void> {
  if (Q.headless) return
  await update($, snapshotAtom, () => s).catch(() => {})
}

export async function setView($: EngineInterface, view: QuayView): Promise<void> {
  await update($, viewAtom, () => view).catch(() => {})
}

export async function flash($: EngineInterface, tone: QuayFlash['tone'], text: string): Promise<void> {
  const at = Date.now()
  await update($, flashAtom, () => ({ tone, text, at })).catch(() => {})
}

// The atoms are only touched here: a module's state reads and writes name
// their atoms in the file that defines them.

export async function clearFlash($: EngineInterface): Promise<void> {
  await update($, flashAtom, () => null).catch(() => {})
}

export async function setFormValue($: EngineInterface, key: string, value: string): Promise<void> {
  await update($, formAtom, f => ({ ...f, [key]: value })).catch(() => {})
}

export async function clearFormKeys($: EngineInterface, keys: string[]): Promise<void> {
  await update($, formAtom, f => {
    const next = { ...f }
    for (const k of keys) delete next[k]
    return next
  }).catch(() => {})
}

export async function setSignin($: EngineInterface, value: QuaySignin | null): Promise<void> {
  await update($, signinAtom, () => value).catch(() => {})
}

export async function patchSigninStatus($: EngineInterface, status: SigninStatus): Promise<void> {
  await update($, signinAtom, cur => (cur && cur.status.id === status.id ? { ...cur, status } : cur)).catch(() => {})
}

export async function setDisclosure($: EngineInterface, value: QuayDisclosure | null): Promise<void> {
  await update($, disclosureAtom, () => value).catch(() => {})
}

export async function setReport($: EngineInterface, value: QuayReport | null): Promise<void> {
  await update($, reportAtom, () => value).catch(() => {})
}

/** Stores an engine read for a review screen in its slot (one entry per slot). */
export async function setCacheEntry($: EngineInterface, slot: string, entry: QuayCacheEntry | null): Promise<void> {
  await update($, cacheAtom, c => {
    const next = { ...c }
    if (entry) next[slot] = entry
    else delete next[slot]
    return next
  }).catch(() => {})
}

/** Marks ids as already announced (no transcript line for them). */
export async function seedAnnounced($: EngineInterface, ids: string[]): Promise<void> {
  await update($, announcedAtom, list => [...new Set([...list, ...ids])].slice(-1000)).catch(() => {})
}

/** Of `ids`, those not announced yet (suggestions, notices, engine events by sequence); they are marked announced as they are returned. */
export async function takeUnannounced($: EngineInterface, ids: string[]): Promise<string[]> {
  const seen = new Set(await read($, announcedAtom).catch(() => [] as string[]))
  const fresh = ids.filter(id => !seen.has(id))
  if (fresh.length) await update($, announcedAtom, list => [...list, ...fresh].slice(-1000)).catch(() => {})
  return fresh
}

export type PaneState = {
  link: QuayLink
  s: StateView | null
  view: QuayView
  form: QuayForm
  flash: QuayFlash | null
  signin: QuaySignin | null
  disclosure: QuayDisclosure | null
  report: QuayReport | null
  cache: Record<string, QuayCacheEntry>
}

/** Every value the pane draws from; read while drawing, so a write redraws it. */
export async function readPaneState($: EngineInterface): Promise<PaneState> {
  return {
    link: await read($, linkAtom),
    s: await read($, snapshotAtom),
    view: await read($, viewAtom),
    form: await read($, formAtom),
    flash: await read($, flashAtom),
    signin: await read($, signinAtom),
    disclosure: await read($, disclosureAtom),
    report: await read($, reportAtom),
    cache: await read($, cacheAtom),
  }
}

/** Resolves after `ms` on the session clock (a `$` timer: free of the hook budget). */
export function pause($: EngineInterface, ms: number): Promise<void> {
  return new Promise(resolve => {
    try {
      $.clock.after(ms, () => resolve())
    } catch {
      resolve()
    }
  })
}

export function errorText(err: unknown): string {
  return err instanceof Error ? err.message : String(err)
}
