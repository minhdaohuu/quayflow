// The loopback link to quayflow.exe: connection file, authenticated calls,
// handshake (FR-20) and on-demand start (FR-19). Every failure here fails
// open: the caller gets an error or `off`, Claude keeps working unchanged.
import type { EngineInterface } from 'claude-code'

import type { HelloResponse, QuayOffReason, StartState, StateView } from '../types'
import { MOD_VERSION, PROTOCOL, Q, errorText, setLink } from './context'
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

export async function dataDir($: EngineInterface): Promise<string> {
  const local = await $.env.get('LOCALAPPDATA')
  if (!local) throw new EngineError('LOCALAPPDATA is not set', 'unreachable')
  return `${local.replace(/[\\/]+$/, '')}\\Quayflow`
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
  const file = `${await dataDir($)}\\engine.json`
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

async function hello($: EngineInterface): Promise<HelloResponse> {
  return call<HelloResponse>(
    $,
    'POST',
    '/v1/hello',
    { protocol: PROTOCOL, modVersion: MOD_VERSION, sessionId: Q.sessionId, headless: Q.headless, claudeVersion: Q.claudeVersion || undefined },
    { timeoutMs: 5_000 },
  )
}

type HelloOutcome = { kind: 'on'; version: string } | { kind: 'incompatible'; message: string; version?: string } | { kind: 'unreachable'; message: string }

async function tryHello($: EngineInterface): Promise<HelloOutcome> {
  try {
    const h = await hello($)
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
    return { kind: 'unreachable', message: errorText(err) }
  }
}

/** Runs `quayflow.exe start [--if-consented]` and reads its one JSON line. */
export async function runStart($: EngineInterface, ifConsented: boolean): Promise<{ state: StartState | 'not_installed'; detail?: string }> {
  const exe = `${await dataDir($)}\\bin\\quayflow.exe`
  if (!(await $.fs.exists(exe).catch(() => false))) return { state: 'not_installed' }
  try {
    const argv = ifConsented ? [exe, 'start', '--if-consented'] : [exe, 'start']
    const run = await $.process.run(argv, { timeoutMs: 30_000 })
    const lines = run.stdout.split(/\r?\n/).map(l => l.trim()).filter(Boolean)
    for (const line of lines.reverse()) {
      try {
        const out = JSON.parse(line) as { state?: StartState }
        if (out.state) return { state: out.state }
      } catch {}
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
  Q.lastLinkTry = Date.now()
  let outcome = await tryHello($)
  if (outcome.kind === 'unreachable' && options.allowStart && (options.explicit || !Q.startAttempted)) {
    Q.startAttempted = true
    if (!Q.headless && !Q.link.on) await setLink($, { on: false, reason: 'connecting', message: 'starting the engine' })
    const started = await runStart($, !options.explicit)
    if (started.state === 'running' || started.state === 'started') {
      Q.conn = undefined
      outcome = await tryHello($)
      if (outcome.kind === 'unreachable') {
        await setLink($, { on: false, reason: 'start_failed', message: outcome.message })
        return
      }
    } else {
      await setLink($, { on: false, reason: offReason(started.state), message: started.detail })
      return
    }
  }
  if (outcome.kind === 'on') {
    await setLink($, { on: true, engineVersion: outcome.version })
  } else if (outcome.kind === 'incompatible') {
    await setLink($, { on: false, reason: 'incompatible', message: outcome.message, engineVersion: outcome.version })
  } else {
    // Keep a known start outcome (no consent, stopped by the person) over a bare "unreachable".
    const keep = Q.link.reason === 'no_consent' || Q.link.reason === 'stopped_by_user' || Q.link.reason === 'not_installed'
    if (!keep || Q.link.on) await setLink($, { on: false, reason: 'unreachable', message: outcome.message })
  }
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
  const retryable = Q.link.reason === 'unreachable' || Q.link.reason === 'start_failed'
  if (retryable && Date.now() - Q.lastLinkTry > 15_000) {
    await withTimeout($, establish($, { allowStart: false }), Math.max(waitMs, 3_000), 'reconnect').catch(() => {})
  }
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
