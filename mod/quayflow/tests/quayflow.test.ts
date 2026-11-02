// Hooks: handshake, engine start, the request hook (report, verify, recover,
// one retry) and the headless contract, against a mocked engine.
// Run: claude plugin test mod/quayflow
import { describe, expect, test } from 'claude-code/testing'

import { errorKindOf, isFailedRequest } from '../hooks/step'
import { failed, ok } from './fixtures'
import { ENGINE_EXE, PROFILE_UUID, STEP, quay, refuse, start, step, world } from './harness'

async function hash12(value: string): Promise<string> {
  const digest = await crypto.subtle.digest('SHA-256', new TextEncoder().encode(value))
  return Array.from(new Uint8Array(digest).slice(0, 6), b => b.toString(16).padStart(2, '0')).join('')
}

describe('handshake and engine start', () => {
  test('an incompatible engine turns everything off: no report, no recovery, no retry', async ($, on) => {
    const w = world(on, { hello: { compatible: false, protocol: 1, acceptedProtocols: [2, 3], engineVersion: '0.3.0', message: 'protocol 1 is too old' } })
    let requests = 0
    on('turn.step', async function* (_$, e) {
      requests += 1
      return failed(e)
    })
    await start($, w, false)
    const r = await step($)
    expect(r.stopReason).toBeNull()
    expect(requests).toBe(1)
    expect(w.routes()).toEqual(['/v1/hello'])
    expect(w.bodies('/v1/hello')[0]).toEqual({ protocol: 1, modVersion: '0.1.0', sessionId: 'S1', headless: true, claudeVersion: '2.1.316' })
  })

  test('no engine: start runs once with --if-consented; no_consent is respected', async ($, on) => {
    const w = world(on, { down: true, start: 'no_consent' })
    let requests = 0
    on('turn.step', async function* (_$, e) {
      requests += 1
      return failed(e)
    })
    await start($, w, false)
    await step($)
    await step($, { ...STEP, index: 1 })
    expect(requests).toBe(2)
    expect(w.runs).toEqual([[ENGINE_EXE, 'start', '--if-consented']])
    expect(w.sent).toEqual([])
  })
})

describe('request hook', () => {
  test('a failed request: failure shape, one recover, exactly one retry', async ($, on) => {
    const w = world(on)
    let requests = 0
    on('turn.step', async function* (_$, e) {
      requests += 1
      return failed(e)
    })
    await start($, w, false)
    const r = await step($)
    // The retry failed too: Claude's own result stands and nothing more is tried.
    expect(r.stopReason).toBeNull()
    expect(requests).toBe(2)
    expect(w.bodies('/v1/recover')).toHaveLength(1)

    const failure = w.bodies('/v1/failure')[0]!
    expect(failure).toMatchObject({ sessionId: 'S1', model: 'claude-opus-5-5', answerEmpty: true, toolUses: 0, hasUsage: false, simulated: false })
    expect(failure.fields).toEqual(['answer', 'index', 'stopReason', 'toolUses', 'turnId', 'usage'])
    expect(JSON.stringify(w.sent)).not.toContain('opaque-handle')

    expect(w.bodies('/v1/recover')[0]).toMatchObject({
      sessionId: 'S1',
      turnId: 'turn-1',
      index: 0,
      agentId: null,
      model: 'claude-opus-5-5',
      simulated: false,
      rateLimits: [{ kind: 'five_hour', percentUsed: 100, resetsAt: '2027-01-15T10:00:00Z' }],
    })

    // The same request failing again is not recovered a second time.
    await step($)
    expect(w.bodies('/v1/recover')).toHaveLength(1)
    expect(requests).toBe(3)
  })

  test('the engine declines recovery: no retry, the result stands', async ($, on) => {
    const w = world(on, { recover: { retry: false, switched: false, why: 'not a confirmed usage limit' } })
    let requests = 0
    on('turn.step', async function* (_$, e) {
      requests += 1
      return failed(e)
    })
    await start($, w, false)
    await step($)
    expect(requests).toBe(1)
    expect(w.bodies('/v1/recover')).toHaveLength(1)
  })

  test('a successful retry is the result Claude receives', async ($, on) => {
    const w = world(on)
    let requests = 0
    on('turn.step', async function* (_$, e) {
      requests += 1
      return requests === 1 ? failed(e) : ok(e)
    })
    await start($, w, false)
    const r = await step($)
    expect(r.answer).toBe('done')
    expect(requests).toBe(2)
    expect(w.bodies('/v1/step').map(b => b.ok)).toEqual([false, true])
  })

  test('an interrupted request is never recovered or retried', async ($, on) => {
    const w = world(on)
    let requests = 0
    on('turn.step', async function* (_$, e) {
      requests += 1
      yield { kind: 'text', index: 0, text: 'partial', ref: 1 } as never
      return failed(e)
    })
    await start($, w, false)
    const stream = $.turn.step(STEP)
    await stream.next()
    // The person interrupts while the response streams: the dispatch is abandoned.
    await stream.return(undefined as never).catch(() => undefined)
    await w.clock.settle()
    expect(requests).toBe(1)
    expect(w.routes()).not.toContain('/v1/recover')
    expect(w.routes()).not.toContain('/v1/failure')
  })

  test('verify: the identity hash of the account each request used goes to /v1/adoption', async ($, on) => {
    const w = world(on, { step: { verify: true, transition: 'tr-7' } })
    on('turn.step', async function* (_$, e) {
      return ok(e)
    })
    await start($, w, false)
    const t0 = Date.now()
    await step($)
    await step($, { ...STEP, index: 1 })
    const expected = await hash12(PROFILE_UUID)
    expect(expected).toHaveLength(12)
    // Every verify: true is answered (the live canary asks on every request).
    const any = expect.any(Number)
    expect(w.bodies('/v1/adoption')).toEqual([
      { sessionId: 'S1', account: expected, transition: 'tr-7', startedAt: any },
      { sessionId: 'S1', account: expected, transition: 'tr-7', startedAt: any },
    ])
    const steps = w.bodies('/v1/step')
    expect(steps[0]).toMatchObject({ sessionId: 'S1', turnId: 'turn-1', index: 0, ok: true, stopReason: 'end_turn', model: 'claude-opus-5-5' })
    // The start time is taken before the request, so the engine can tell requests that were
    // already in flight at a switch from the first request after it.
    const s0 = Number(steps[0]?.startedAt)
    const s1 = Number(steps[1]?.startedAt)
    expect(s0).toBeGreaterThanOrEqual(t0)
    expect(s1).toBeGreaterThanOrEqual(s0)
    expect(w.bodies('/v1/adoption')[0]?.startedAt).toBe(s0)
  })

  test('failure rules', () => {
    expect(isFailedRequest(failed(STEP), false)).toBe(true)
    expect(isFailedRequest(failed(STEP), true)).toBe(false)
    expect(isFailedRequest(ok(STEP), false)).toBe(false)
    expect(errorKindOf({ error: { type: 'not_found_error', message: 'model: claude-x' } })).toBe('model_not_found')
    expect(errorKindOf({ detail: 'model_not_found' })).toBe('model_not_found')
    expect(errorKindOf({ status: '401', error: 'authentication_error' })).toBe('auth')
    expect(errorKindOf({ message: 'OAuth token has expired. Please run /login' })).toBe('auth')
    expect(errorKindOf({ answer: '', stopReason: null })).toBeUndefined()
  })
})

describe('headless runs', () => {
  test('no pane, question, toast, status or log line, even while recovering', async ($, on) => {
    const w = world(on)
    on('turn.step', async function* (_$, e) {
      return failed(e)
    })
    await start($, w, false)
    await step($)
    await step($, { ...STEP, turnId: 'turn-2' })
    await w.clock.advance(120_000)
    expect(w.routes()).toContain('/v1/recover')
    expect(w.ui).toEqual([])
    expect(w.routes()).not.toContain('/v1/events')
  })

  test('/quay prints text and never asks: approvals must be typed', async ($, on) => {
    const w = world(on, {
      routes: {
        '/v1/groups/mode': (b: Record<string, unknown>) => (b.approveAutomatic ? { ok: true } : refuse('needs_approval', 'Approve it explicitly (approveAutomatic).')),
      },
    })
    await start($, w, false)
    const status = await quay($, 'status')
    expect(status).toContain('Running sessions of default (other clients, such as editor extensions, may not be listed)')
    expect(status).toContain('Recommendation: switch to "personal"')
    expect(await quay($, 'mode automatic Default')).toContain('/quay mode automatic Default --approve')
    expect(w.routes()).not.toContain('/v1/groups/mode')
    expect(await quay($, 'mode automatic Default --approve')).toContain('now in automatic mode')
    expect(w.bodies('/v1/groups/mode')).toEqual([{ group: 'Default', mode: 'automatic', approveAutomatic: true }])
    expect(w.ui).toEqual([])
  })

  test('/quay register: a mixed group needs --accept-disclosure', async ($, on) => {
    const w = world(on, {
      routes: {
        '/v1/accounts/register': (b: Record<string, unknown>) =>
          b.acceptDisclosure ? { id: 'c3c3c3c3c3c3', label: 'client', kind: 'organization' } : refuse('needs_disclosure', 'Group "Default" would mix two organizations.'),
      },
    })
    await start($, w, false)
    const refused = await quay($, 'register client Default')
    expect(refused).toContain('would mix two organizations')
    expect(refused).toContain('--accept-disclosure')
    const saved = await quay($, 'register client Default --accept-disclosure')
    expect(saved).toContain('Saved "client"')
    expect(saved).toContain('Nothing was switched')
    expect(w.bodies('/v1/accounts/register').map(b => b.acceptDisclosure)).toEqual([false, true])
  })
})

describe('interactive sessions', () => {
  test('status line, and a suggestion announced once across polls', async ($, on) => {
    const w = world(on)
    await start($, w, true)
    const status = w.ui.filter(u => u.kind === 'ui.status').pop()?.text ?? ''
    expect(status).toMatch(/^Quayflow · suggest · work 91% 5 h/)
    expect(status).toContain('suggests personal · /quay accept')
    await w.clock.advance(30_000)
    await w.clock.advance(30_000)
    expect(w.ui.filter(u => u.kind === 'ui.log' && /suggests "personal"/.test(u.text ?? ''))).toHaveLength(1)
    expect(w.ui.filter(u => u.kind === 'ui.toast')).toHaveLength(1)
    expect(w.routes()).toContain('/v1/events')
  })

  test('an incompatible engine says how to update', async ($, on) => {
    const w = world(on, { hello: { compatible: false, protocol: 1, acceptedProtocols: [2], engineVersion: '0.3.0' } })
    await start($, w, true)
    const status = w.ui.filter(u => u.kind === 'ui.status').pop()?.text ?? ''
    expect(status).toContain('incompatible with engine 0.3.0')
    expect(status).toContain('quayflow update')
  })
})
