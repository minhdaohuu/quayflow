// The request hook (T41, FR-13, FR-15): report every request, verify adoption
// when the engine asks, and on a failed request ask the engine to recover and
// retry the same request at most once. Never prints in a headless run; every
// failure of the Mod's own calls leaves Claude's result standing.
import type { EngineInterface, StreamNext, TurnStepChunk, TurnStepInput, TurnStepResult } from 'claude-code'

import type { FailureShape, RecoverRequest, RecoverResponse, SessionRateLimitWire, StepReport, StepResponse } from '../types'
import { Q, errorText, uiLog } from './context'
import { call, dataDir, linkReady, markUnreachable } from './engine'
import { liveAccountHash } from './identity'

type StepEvent = Pick<TurnStepInput, 'turnId' | 'index' | 'model' | 'agentId'>

/** Request keys already given their one recovery attempt (per process). */
const recovered = new Set<string>()

export function requestKey(e: StepEvent): string {
  return `${e.turnId}:${e.index}:${e.agentId ?? 'main'}`
}

/** No response arrived: no stop reason, no text, no tool call, and not cancelled (FR-15). */
export function isFailedRequest(r: TurnStepResult | undefined, aborted: boolean): boolean {
  return !!r && !aborted && r.stopReason === null && r.answer === '' && r.toolUses.length === 0
}

/**
 * The error class the engine acts on, read from any string in the result:
 * `model_not_found` (a missing model) or `auth` (401, authentication_error,
 * an OAuth token or /login message); else undefined. Only the class leaves.
 */
export function errorKindOf(r: unknown): string | undefined {
  const strings: string[] = []
  const walk = (v: unknown, depth: number): void => {
    if (depth > 6 || strings.length > 200) return
    if (typeof v === 'string') strings.push(v)
    else if (Array.isArray(v)) v.forEach(x => walk(x, depth + 1))
    else if (v && typeof v === 'object') Object.values(v as Record<string, unknown>).forEach(x => walk(x, depth + 1))
  }
  walk(r, 0)
  const hit =
    strings.some(s => /model_not_found/i.test(s)) ||
    (strings.some(s => /not_found_error/i.test(s)) && strings.some(s => /model/i.test(s)))
  if (hit) return 'model_not_found'
  const auth = strings.some(s => /\b401\b|authentication_error|OAuth token|\/login\b/i.test(s))
  return auth ? 'auth' : undefined
}

/** The session's own rate-limit readings (free: no breakdown is computed). */
async function rateLimits($: EngineInterface): Promise<SessionRateLimitWire[]> {
  try {
    const u = await $.session.usage()
    return u.rateLimits.map(r => ({ kind: r.kind, percentUsed: r.percentUsed, resetsAt: r.resetsAt ?? null }))
  } catch {
    return []
  }
}

/**
 * Test hook: `%LOCALAPPDATA%\Quayflow\simulate-limit.json` `{"atStep": n}`
 * answers the first main-agent request with index >= n as failed, without
 * sending it, once; the file is then consumed.
 */
async function takeSimulatedLimit($: EngineInterface, e: StepEvent): Promise<boolean> {
  if (e.agentId) return false
  let file: string
  try {
    file = `${await dataDir($)}\\simulate-limit.json`
  } catch {
    return false
  }
  let sim: { atStep?: unknown }
  try {
    sim = JSON.parse(await $.fs.read(file)) as { atStep?: unknown }
  } catch {
    return false
  }
  if (typeof sim.atStep !== 'number' || e.index < sim.atStep) return false
  await $.fs.write(file, '{}').catch(() => {})
  return true
}

function emptyResult(e: StepEvent): TurnStepResult {
  return { turnId: e.turnId, index: e.index, answer: '', toolUses: [], stopReason: null, usage: null }
}

/** POST /v1/step, then /v1/adoption when the engine asks for verification. */
async function report($: EngineInterface, e: StepEvent, result: TurnStepResult, ok: boolean, limits: SessionRateLimitWire[]): Promise<void> {
  const body: StepReport = {
    sessionId: Q.sessionId,
    turnId: e.turnId,
    index: e.index,
    agentId: e.agentId ?? null,
    ok,
    stopReason: result.stopReason,
    model: e.model,
    rateLimits: limits,
  }
  let res: StepResponse
  try {
    res = await call<StepResponse>($, 'POST', '/v1/step', body, { timeoutMs: 3_000 })
  } catch (err) {
    if ((err as { code?: string }).code === 'unreachable') await markUnreachable($)
    return
  }
  if (!res.verify) return
  let account: string | undefined
  try {
    account = await liveAccountHash($)
  } catch {
    account = undefined
  }
  await call($, 'POST', '/v1/adoption', { sessionId: Q.sessionId, account: account ?? null, transition: res.transition ?? null }, { timeoutMs: 3_000 }).catch(() => {})
}

async function postFailure($: EngineInterface, e: StepEvent, result: TurnStepResult, limits: SessionRateLimitWire[], simulated: boolean, errorKind: string | undefined): Promise<void> {
  // Field names and counts only: no text, tool input or response content.
  const shape: FailureShape = {
    sessionId: Q.sessionId,
    model: e.model,
    stopReason: result.stopReason,
    answerEmpty: result.answer === '',
    toolUses: result.toolUses.length,
    hasUsage: result.usage !== null,
    errorKind,
    fields: Object.keys(result).sort(),
    rateLimits: limits,
    simulated,
  }
  await call($, 'POST', '/v1/failure', shape, { timeoutMs: 3_000 }).catch(() => {})
}

async function askRecover($: EngineInterface, e: StepEvent, limits: SessionRateLimitWire[], simulated: boolean, errorKind: string | undefined): Promise<RecoverResponse | undefined> {
  const body: RecoverRequest = {
    sessionId: Q.sessionId,
    turnId: e.turnId,
    index: e.index,
    agentId: e.agentId ?? null,
    model: e.model,
    simulated,
    rateLimits: limits,
    errorKind,
    requestId: `${Q.sessionId}:${requestKey(e)}`,
  }
  try {
    // The engine may switch under Claude's locks (bounded per lock); still bounded here.
    return await call<RecoverResponse>($, 'POST', '/v1/recover', body, { timeoutMs: 60_000 })
  } catch {
    return undefined
  }
}

/** The `turn.step` hook body. */
export async function* onStep($: EngineInterface, e: TurnStepInput, next: StreamNext<'turn.step'>): AsyncGenerator<TurnStepChunk, TurnStepResult> {
  const simulated = Q.link.on ? await takeSimulatedLimit($, e).catch(() => false) : false
  let result: TurnStepResult = simulated ? emptyResult(e) : yield* next(e)
  // A cancelled request is never recovered or retried, and is not a failure to report.
  if (next.signal.aborted) return result
  const failed = isFailedRequest(result, false)
  try {
    if (!(await linkReady($, failed ? 15_000 : 2_000))) return result
  } catch {
    return result
  }
  const limits = await rateLimits($)
  await report($, e, result, !failed, limits).catch(() => {})
  if (!failed || !Q.link.on) return result

  const errorKind = errorKindOf(result)
  await postFailure($, e, result, limits, simulated, errorKind)
  const key = requestKey(e)
  if (recovered.has(key)) return result
  recovered.add(key)

  const rec = await askRecover($, e, limits, simulated, errorKind)
  if (!rec) return result
  if (!rec.retry) {
    uiLog($, `Quayflow: the request ended without a response; not switching (${rec.why}). Claude's own result stands.`)
    return result
  }
  if (next.signal.aborted) return result
  const to = rec.toLabel ?? rec.to ?? 'the account already switched to'
  uiLog($, `Quayflow: ${rec.switched ? `switched to "${to}"` : `using "${to}"`} (${rec.why}); retrying the same request once.`)
  try {
    result = yield* next(e)
  } catch (err) {
    uiLog($, `Quayflow: the retry could not run (${errorText(err)}).`)
    return result
  }
  if (next.signal.aborted) return result
  const retryFailed = isFailedRequest(result, false)
  await report($, e, result, !retryFailed, await rateLimits($)).catch(() => {})
  if (retryFailed) await postFailure($, e, result, await rateLimits($), false, errorKindOf(result))
  return result
}
