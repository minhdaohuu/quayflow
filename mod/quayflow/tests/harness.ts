// Test harness: the world beneath the plugin. Claude Code's nouns (session,
// env, clock, fs, process, UI) and the Quayflow engine's loopback API, all
// answered from memory, with every engine call and every UI call recorded.
import { expect, mock } from 'claude-code/testing'
import type { On, TurnStepInput, TurnStepResult } from 'claude-code'

import type { StateView } from '../types'
import { STATE } from './fixtures'

export type Sent = { method: string; route: string; body: Record<string, unknown> | undefined }

/** An engine answer with a status other than 200. */
export type Reply = { status: number; body: unknown }

/** An answer for one route: a body, a `Reply`, or a function of the request body. */
export type RouteAnswer = unknown | ((body: Record<string, unknown>) => unknown)

export type Answers = {
  hello?: Record<string, unknown>
  step?: Record<string, unknown>
  recover?: Record<string, unknown>
  state?: StateView
  /** The engine is not running: no engine.json, connections refused. */
  down?: boolean
  /** What `quayflow.exe start` prints; `started` / `running` bring a down engine up. */
  start?: 'running' | 'started' | 'failed' | 'no_consent' | 'stopped_by_user'
  /** Answers for any other route, by path. Unlisted routes answer `{ ok: true }`. */
  routes?: Record<string, RouteAnswer>
}

/** The engine's refusal: HTTP 409 with its machine code (`needs_disclosure`, `needs_approval`). */
export function refuse(code: string, error: string): Reply {
  return { status: 409, body: { error, code } }
}

const LOCALAPPDATA = 'C:\\Users\\t\\AppData\\Local'
export const ENGINE_EXE = `${LOCALAPPDATA}\\Quayflow\\bin\\quayflow.exe`
/** The account uuid the profile endpoint reports for the session's credential. */
export const PROFILE_UUID = '00000000-0000-0000-0000-000000000001'

function isReply(v: unknown): v is Reply {
  return !!v && typeof v === 'object' && 'status' in v && 'body' in v
}

/** Mocks the world beneath the plugin; returns what it recorded. */
export function world(on: On, a: Answers = {}) {
  const sent: Sent[] = []
  const ui: { kind: string; text?: string }[] = []
  const runs: string[][] = []
  let down = !!a.down

  mock.env(on, { LOCALAPPDATA })
  const clock = mock.clock(on, { now: 1_800_000_000_000 })
  on('session.start', (_$, e) => ({ cwd: e.cwd }))
  on('session.id', () => ({ value: 'S1' }))
  on('session.version', () => ({ value: { version: '2.1.316', base: '2.1.316', builtAt: '' } as never }))
  on('session.usage', () => ({
    value: { startedAt: 0, rateLimits: [{ kind: 'five_hour', percentUsed: 100, resetsAt: '2027-01-15T10:00:00Z' }] } as never,
  }))
  on('session.authorize', () => ({ value: { handle: 'opaque-handle', kind: 'bearer' } }))
  on('command.register', () => ({ value: { command: 'quay' } }))

  on('fs.read', (_$, e) => {
    if (!down && e.path === `${LOCALAPPDATA}\\Quayflow\\engine.json`) {
      return { value: JSON.stringify({ port: 47123, token: 'tok', pid: 9, protocol: 1 }) }
    }
    return { deny: 'ENOENT: no such file' }
  })
  on('fs.exists', () => ({ value: true }))
  on('fs.write', () => ({ value: undefined }))
  on('process.run', (_$, e) => {
    runs.push([...e.argv])
    if (a.start === 'started' || a.start === 'running') down = false
    return { value: { exitCode: 0, stdout: `{"state":"${a.start ?? 'no_consent'}"}\n`, stderr: '' } as never }
  })

  on('ui.status', (_$, e) => {
    ui.push({ kind: 'ui.status', text: e.text })
    return { value: undefined }
  })
  on('ui.log', (_$, e) => {
    ui.push({ kind: 'ui.log', text: e.text })
    return { value: undefined }
  })
  on('ui.toast', (_$, e) => {
    ui.push({ kind: 'ui.toast', text: e.text })
    return { value: undefined }
  })
  on('ui.open', () => {
    ui.push({ kind: 'ui.open' })
    return { value: { isPlaced: true } }
  })
  // `$.ui.ask` is an AskUserQuestion call; nobody answers it in a test.
  on('tool.call', { tool: 'AskUserQuestion' }, () => {
    ui.push({ kind: 'ui.ask' })
    return { deny: 'no one to ask' }
  })

  on('http.fetch', (_$, e) => {
    const url = new URL(e.url)
    const reply = (status: number, body: unknown) => ({ value: { status, ok: status < 300, headers: {}, text: JSON.stringify(body) } })
    if (url.hostname === 'api.anthropic.com') {
      // The profile lookup spends Claude's own credential handle; the Mod never holds a token.
      expect(e.init?.auth).toBe('opaque-handle')
      return reply(200, { account: { uuid: PROFILE_UUID } })
    }
    if (down) return { deny: 'ECONNREFUSED' }
    expect(e.init?.headers?.['x-quayflow-token']).toBe('tok')
    const body = e.init?.body ? (JSON.parse(e.init.body) as Record<string, unknown>) : undefined
    sent.push({ method: e.init?.method ?? 'GET', route: url.pathname, body })
    const state = a.state ?? STATE
    const fixed: Record<string, unknown> = {
      '/v1/hello': a.hello ?? { compatible: true, protocol: 1, acceptedProtocols: [1], engineVersion: '0.1.0' },
      '/v1/step': a.step ?? { verify: false },
      '/v1/recover': a.recover ?? { retry: true, switched: true, to: 'b2b2b2b2b2b2', toLabel: 'personal', why: 'five_hour at 100%' },
      '/v1/state': state,
      '/v1/events': { seq: state.seq, events: [] },
    }
    const route = a.routes?.[url.pathname]
    const answer = typeof route === 'function' ? (route as (b: Record<string, unknown>) => unknown)(body ?? {}) : (route ?? fixed[url.pathname] ?? { ok: true })
    return isReply(answer) ? reply(answer.status, answer.body) : reply(200, answer)
  })

  return {
    sent,
    ui,
    runs,
    clock,
    /** The engine comes up by other means (installed, set up or started from the command line). */
    up: () => {
      down = false
    },
    routes: () => sent.map(s => s.route),
    bodies: (route: string) => sent.filter(s => s.route === route).map(s => s.body ?? {}),
  }
}

export const STEP: TurnStepInput = { turnId: 'turn-1', index: 0, model: 'claude-opus-5-5', messageCount: 3 }

type Stepper = { turn: { step: (e: TurnStepInput) => AsyncGenerator<unknown, TurnStepResult> } }

/** Drives one model request through the plugin and returns its result. */
export async function step($: Stepper, e: TurnStepInput = STEP): Promise<TurnStepResult> {
  const stream = $.turn.step(e)
  for (;;) {
    const r = await stream.next()
    if (r.done) return r.value
  }
}

/** Runs `/quay <args>` as the person typed it. */
export async function quay($: { command: { run: (e: never) => Promise<{ text?: string }> } }, args: string): Promise<string> {
  const out = await $.command.run({ command: 'quay', args, origin: { kind: 'composer' }, presentation: {} } as never)
  return out.text ?? ''
}

/** Starts a session the way Claude Code does, then lets the Mod's start-up work settle. */
export async function start(
  $: { session: { start: (e: { cwd: string; surface: 'terminal' | null; isInteractive: boolean }) => Promise<unknown> } },
  w: { clock: { settle: () => Promise<void> } },
  interactive: boolean,
): Promise<void> {
  await $.session.start({ cwd: 'C:\\work\\app', surface: interactive ? 'terminal' : null, isInteractive: interactive })
  await w.clock.settle()
}
