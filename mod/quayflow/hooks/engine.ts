// The loopback link to the engine (quayflow): connection file, authenticated calls,
// handshake (FR-20) and on-demand start (FR-19). Every failure here fails
// open: the caller gets an error or `off`, Claude keeps working unchanged.
import type { EngineInterface } from 'claude-code'

import type { HelloResponse, QuayLink, QuayOffReason, StartState, StateView } from '../types'
import { MOD_VERSION, PROTOCOL, Q, errorText, pause, setLink } from './context'
import type { Conn } from './context'

export class EngineError extends Error {
  readonly code: string
  readonly status: number
  constructor(message: string, code: string, status = 0) {
    super(message)
    this.code = code
    this.status = status
  }
}

/** Whether `err` is the engine's refusal with machine code `code` (409 `needs_disclosure`, `needs_approval`, ...). */
export function hasCode(err: unknown, code: string): err is EngineError {
  return err instanceof EngineError && err.code === code
}

/** Where the engine lives on this computer: its data folder, path separator and program name. */
export type Layout = { root: string; sep: '\\' | '/'; exe: string }

/**
 * Windows: `%LOCALAPPDATA%\Quayflow`, `quayflow.exe`. Linux: `$XDG_DATA_HOME/quayflow`
 * (`~/.local/share/quayflow`), `quayflow`. As the engine and the installers choose it.
 */
export async function layout($: EngineInterface): Promise<Layout> {
  const trim = (p: string) => p.replace(/[\\/]+$/, '')
  const local = await $.env.get('LOCALAPPDATA').catch(() => undefined)
  if (local) return { root: `${trim(local)}\\Quayflow`, sep: '\\', exe: 'quayflow.exe' }
  const xdg = await $.env.get('XDG_DATA_HOME').catch(() => undefined)
  if (xdg) return { root: `${trim(xdg)}/quayflow`, sep: '/', exe: 'quayflow' }
  const home = await $.env.get('HOME').catch(() => undefined)
  if (home) return { root: `${trim(home)}/.local/share/quayflow`, sep: '/', exe: 'quayflow' }
  throw new EngineError('neither LOCALAPPDATA nor HOME is set', 'unreachable')
}

export async function dataDir($: EngineInterface): Promise<string> {
  return (await layout($)).root
}

/** A path inside the data folder, joined with this platform's separator. */
export async function dataPath($: EngineInterface, ...parts: string[]): Promise<string> {
  const l = await layout($)
  return [l.root, ...parts].join(l.sep)
}

/** The engine program (`bin/quayflow`, `bin\quayflow.exe`). */
export async function engineExe($: EngineInterface): Promise<string> {
  const l = await layout($)
  return [l.root, 'bin', l.exe].join(l.sep)
}

/** Races `p` against a session timer; the timer is cancelled once `p` settles. */
export function withTimeout<T>($: EngineInterface, p: Promise<T>, ms: number, what: string): Promise<T> {
  return new Promise<T>((resolve, reject) => {
    let done = false
    let timer: { cancel: () => void } | undefined
    try {
      timer = $.clock.after(ms, () => {
        if (done) return
        done = true
        reject(new EngineError(`${what} timed out after ${ms} ms`, 'timeout'))
      })
    } catch {}
    p.then(
      value => {
        if (done) return
        done = true
        timer?.cancel()
        resolve(value)
      },
      err => {
        if (done) return
        done = true
        timer?.cancel()
        reject(err)
      },
    )
  })
}

async function readConn($: EngineInterface): Promise<Conn> {
  const file = await dataPath($, 'engine.json')
  let parsed: Partial<Conn>
  try {
    parsed = JSON.parse(await $.fs.read(file)) as Partial<Conn>
  } catch (err) {
    throw new EngineError(`no engine connection file (${errorText(err)})`, 'unreachable')
  }
  if (typeof parsed.port !== 'number' || typeof parsed.token !== 'string') {
    throw new EngineError('engine connection file is incomplete', 'unreachable')
  }
  return parsed as Conn
}

function dropConnection(): void {
  Q.conn = undefined
}

export type CallOptions = { timeoutMs?: number }

/**
 * One API v1 call. Network failures and a refused token drop the connection,
 * so the next call reads engine.json again (a restarted engine has a new port
 * and token). Errors carry the engine's machine code.
 */
export async function call<T>(
  $: EngineInterface,
  method: 'GET' | 'POST',
  route: string,
  body?: unknown,
  options: CallOptions = {},
): Promise<T> {
  const conn = Q.conn ?? (Q.conn = await readConn($))
  let res
  try {
    res = await withTimeout(
      $,
      $.http.fetch(`http://127.0.0.1:${conn.port}${route}`, {
        method,
        headers: { 'content-type': 'application/json', 'x-quayflow-token': conn.token },
        body: body === undefined ? undefined : JSON.stringify(body),
      }),
      options.timeoutMs ?? 10_000,
      `${method} ${route.split('?')[0]}`,
    )
  } catch (err) {
    dropConnection()
    throw err instanceof EngineError ? err : new EngineError(`engine unreachable (${errorText(err)})`, 'unreachable')
  }
  let parsed: unknown = {}
  try {
    parsed = res.text ? JSON.parse(res.text) : {}
  } catch {
    parsed = {}
  }
  if (!res.ok) {
    const e = (parsed ?? {}) as { error?: string; code?: string }
    if (e.code === 'bad_token' || res.status === 401) dropConnection()
    throw new EngineError(e.error ?? `HTTP ${res.status}`, e.code ?? 'internal', res.status)
  }
  return parsed as T
}

/** Mark everything off after a lost connection; the watcher or the next step reconnects. */
export async function markUnreachable($: EngineInterface): Promise<void> {
  if (Q.link.on) await setLink($, { on: false, reason: 'unreachable' })
}

async function hello($: EngineInterface, timeoutMs: number): Promise<HelloResponse> {
  return call<HelloResponse>(
    $,
    'POST',
    '/v1/hello',
    { protocol: PROTOCOL, modVersion: MOD_VERSION, sessionId: Q.sessionId, headless: Q.headless, claudeVersion: Q.claudeVersion || undefined },
    { timeoutMs },
  )
}

/** `listening`: something answered or the wait ran out, so the engine may only be busy or starting. */
type HelloOutcome = { kind: 'on'; version: string } | { kind: 'incompatible'; message: string; version?: string } | { kind: 'unreachable'; message: string; listening?: boolean }

async function tryHello($: EngineInterface, timeoutMs = 5_000): Promise<HelloOutcome> {
  try {
    const h = await hello($, timeoutMs)
    if (!h.compatible) {
      return {
        kind: 'incompatible',
        version: h.engineVersion,
        message: h.message ?? `engine ${h.engineVersion} speaks protocol ${h.acceptedProtocols.join('/')}; this Mod speaks ${PROTOCOL}`,
      }
    }
    return { kind: 'on', version: h.engineVersion }
  } catch (err) {
    if (err instanceof EngineError && err.code === 'incompatible') return { kind: 'incompatible', message: err.message }
    // An answer (an error status) or a timeout: an engine may be there, busy or starting.
    // A refused connection or no engine.json: nothing listens, and waiting will not help.
    const listening = err instanceof EngineError && (err.code === 'timeout' || err.status > 0)
    return { kind: 'unreachable', message: errorText(err), listening }
  }
}

/** Runs `quayflow start [--if-consented]` and reads its one JSON line. */
export async function runStart($: EngineInterface, ifConsented: boolean): Promise<{ state: StartState | 'not_installed'; detail?: string }> {
  const exe = await engineExe($)
  if (!(await $.fs.exists(exe).catch(() => false))) return { state: 'not_installed' }
  try {
    const argv = ifConsented ? [exe, 'start', '--if-consented'] : [exe, 'start']
    const run = await $.process.run(argv, { timeoutMs: 30_000 })
    const lines = run.stdout.split(/\r?\n/).map(l => l.trim()).filter(Boolean)
    for (const line of lines.reverse()) {
      let out: { state?: StartState; error?: unknown; log?: unknown }
      try {
        out = JSON.parse(line) as typeof out
      } catch {
        continue
      }
      if (!out || typeof out !== 'object' || !out.state) continue
      // The CLI's own words on a failed start: `{"state":"failed","error":...,"log":...}`.
      const why = [typeof out.error === 'string' ? out.error.trim() : '', typeof out.log === 'string' ? `log: ${out.log.trim()}` : ''].filter(Boolean).join('; ')
      return why ? { state: out.state, detail: why.slice(0, 200) } : { state: out.state }
    }
    return { state: 'failed', detail: (run.stderr.trim() || `exit ${run.exitCode}`).slice(0, 200) }
  } catch (err) {
    return { state: 'failed', detail: errorText(err).slice(0, 200) }
  }
}

function offReason(state: StartState | 'not_installed'): QuayOffReason {
  switch (state) {
    case 'no_consent':
      return 'no_consent'
    case 'stopped_by_user':
      return 'stopped_by_user'
    case 'not_installed':
      return 'not_installed'
    default:
      return 'start_failed'
  }
}

/**
 * Brings the link up: handshake with a running engine, else (once per
 * session, or when the person asks) start it and handshake again.
 * `explicit` is the person's own request (onboarding, `/quay start`): it
 * starts without `--if-consented` and retries after an earlier attempt.
 */
export async function establish($: EngineInterface, options: { allowStart: boolean; explicit?: boolean } = { allowStart: true }): Promise<void> {
  Q.linkAttempts += 1
  try {
    await attempt($, options)
  } finally {
    Q.linkAttempts -= 1
  }
}

async function attempt($: EngineInterface, options: { allowStart: boolean; explicit?: boolean }): Promise<void> {
  const ticket = ++Q.linkTicket
  Q.lastLinkTry = Date.now()
  // The session's first handshake runs while Claude Code (and maybe the engine) is still
  // starting up: one slow or refused hello is not yet "the engine is not running".
  const first = options.allowStart && !options.explicit && !Q.startAttempted
  let outcome = first ? await firstHello($) : await tryHello($)
  if (outcome.kind === 'unreachable' && options.allowStart && (options.explicit || !Q.startAttempted)) {
    // Another attempt brought the link up meanwhile: nothing to start.
    if (cameOnSince(ticket)) return
    Q.startAttempted = true
    if (!Q.headless && !Q.link.on) await applyLink($, ticket, { on: false, reason: 'connecting', message: 'starting the engine' })
    const started = await runStart($, !options.explicit)
    // Whatever start reported, an engine that now runs (started meanwhile, or by the
    // CLI despite its answer) is the truth; with no engine.json this fails at once.
    Q.conn = undefined
    outcome = await tryHello($)
    if (outcome.kind === 'unreachable') {
      const ran = started.state === 'running' || started.state === 'started'
      await applyLink($, ticket, ran ? { on: false, reason: 'start_failed', message: outcome.message } : { on: false, reason: offReason(started.state), message: started.detail })
      return
    }
  }
  if (outcome.kind === 'on') {
    await applyLink($, ticket, { on: true, engineVersion: outcome.version })
  } else if (outcome.kind === 'incompatible') {
    await applyLink($, ticket, { on: false, reason: 'incompatible', message: outcome.message, engineVersion: outcome.version })
  } else {
    // Keep a known start outcome (no consent, stopped by the person, the start's own
    // failure words) over a bare "unreachable".
    const r = Q.link.reason
    const keep = r === 'no_consent' || r === 'stopped_by_user' || r === 'not_installed' || r === 'start_failed'
    if (!keep || Q.link.on) await applyLink($, ticket, { on: false, reason: 'unreachable', message: outcome.message })
  }
}

/** Per-attempt timeouts and the pause before each: at most ~8 s in all, well under 10 s. */
const FIRST_HELLO: { timeoutMs: number; pauseMs: number }[] = [
  { timeoutMs: 3_000, pauseMs: 0 },
  { timeoutMs: 2_000, pauseMs: 400 },
  { timeoutMs: 2_000, pauseMs: 800 },
]

/** The first handshake of a session: a few quick tries before the engine counts as not running. */
async function firstHello($: EngineInterface): Promise<HelloOutcome> {
  let outcome: HelloOutcome = { kind: 'unreachable', message: 'no handshake' }
  for (const attempt of FIRST_HELLO) {
    if (attempt.pauseMs) await pause($, attempt.pauseMs)
    outcome = await tryHello($, attempt.timeoutMs)
    // Nothing listening (after a restart of the computer, say): start the engine now rather
    // than retry for seconds; `start` finds an engine that is starting meanwhile.
    if (outcome.kind !== 'unreachable' || !outcome.listening) return outcome
  }
  return outcome
}

/** Whether the link turned on after attempt `ticket` began (so its failure is stale). */
function cameOnSince(ticket: number): boolean {
  return Q.link.on && Q.linkOnTicket >= ticket
}

/**
 * Applies attempt `ticket`'s result. A completed handshake is the newest
 * observation and always applies. An off result applies only while no newer
 * attempt has started, and never over a link that came on after this attempt
 * began: concurrent attempts (start-up, watcher, `/quay start`, a request's
 * reconnect) cannot undo each other's success.
 */
async function applyLink($: EngineInterface, ticket: number, link: QuayLink): Promise<void> {
  if (!link.on && (ticket !== Q.linkTicket || cameOnSince(ticket))) return
  if (link.on) Q.linkOnTicket = Q.linkTicket
  await setLink($, link)
}

/**
 * Whether Quayflow may act for this request: waits for the first handshake
 * up to `waitMs`, and retries a lost link at most every 15 s. Incompatible,
 * unconsented and intentionally stopped engines stay off for the session.
 */
export async function linkReady($: EngineInterface, waitMs: number): Promise<boolean> {
  if (!Q.link.on && Q.link.reason === 'connecting' && waitMs > 0) {
    await withTimeout($, Q.ready, waitMs, 'handshake').catch(() => {})
  }
  if (Q.link.on) return true
  if (stuckConnecting()) {
    await withTimeout($, establish($, { allowStart: false }), Math.max(waitMs, 3_000), 'reconnect').catch(() => {})
    return Q.link.on
  }
  const retryable = Q.link.reason === 'unreachable' || Q.link.reason === 'start_failed'
  if (retryable && Date.now() - Q.lastLinkTry > 15_000) {
    await withTimeout($, establish($, { allowStart: false }), Math.max(waitMs, 3_000), 'reconnect').catch(() => {})
  }
  return Q.link.on
}

/**
 * "Connecting" with no attempt running: this module instance was loaded without its
 * session.start (a reload of some modules only), so nothing would ever connect it.
 */
export function stuckConnecting(): boolean {
  return !Q.link.on && Q.link.reason === 'connecting' && Q.linkAttempts === 0
}

/**
 * Waits for a handshake that is under way, or starts one when none is (never starts
 * the engine): what a command or the pane does before it reports Quayflow as off.
 */
export async function awaitLink($: EngineInterface, waitMs = 8_000): Promise<boolean> {
  if (Q.link.on) return true
  if (Q.link.reason !== 'connecting') return false
  if (stuckConnecting()) await withTimeout($, establish($, { allowStart: false }), waitMs, 'connect').catch(() => {})
  else await withTimeout($, Q.ready, waitMs, 'handshake').catch(() => {})
  return Q.link.on
}

/** GET /v1/state for this session; on success the snapshot and sequence advance. */
export async function fetchState($: EngineInterface): Promise<StateView> {
  const q = `sessionId=${encodeURIComponent(Q.sessionId)}&cwd=${encodeURIComponent(Q.cwd)}`
  const s = await call<StateView>($, 'GET', `/v1/state?${q}`)
  Q.state = s
  Q.seq = Math.max(Q.seq, s.seq ?? 0)
  return s
}
