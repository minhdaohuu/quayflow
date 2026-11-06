// Interactive sessions only: keeps the status line and the pane's snapshot
// current through the engine's event long-poll (GET /v1/events?wait=25), and
// announces each suggestion or notice once. A headless run never starts it.
import type { EngineInterface } from 'claude-code'

import type { EventsResponse, StateView } from '../types'
import { Q, errorText, pause, seedAnnounced, setSnapshot, takeUnannounced, uiLog, uiStatus, uiToast } from './context'
import { EngineError, call, establish, fetchState, hasCode, markUnreachable } from './engine'
import { announcements, seedAnnouncements, statusLine } from './format'

let generation = 0
let seeded = false

/** Draws the status line from the link and the last snapshot. */
export async function paintStatus($: EngineInterface): Promise<void> {
  uiStatus($, statusLine(Q.link, Q.link.on ? Q.state : undefined))
}

/** GET /v1/state, then the snapshot, status line and one-time announcements. */
export async function refresh($: EngineInterface): Promise<StateView | undefined> {
  if (!Q.link.on) {
    await paintStatus($)
    return undefined
  }
  let s: StateView
  try {
    s = await fetchState($)
  } catch (err) {
    // Only a lost connection means the engine is down; a slow tick or an error answer
    // from an engine that just shook hands keeps the link, and the next poll retries.
    if (hasCode(err, 'unreachable')) await markUnreachable($)
    await paintStatus($)
    return undefined
  }
  await setSnapshot($, s)
  await paintStatus($)
  await announce($, s)
  return s
}

async function announce($: EngineInterface, s: StateView): Promise<void> {
  if (Q.headless) return
  if (!seeded) {
    seeded = true
    await seedAnnounced($, seedAnnouncements(s))
  }
  const all = announcements(s)
  const ids = new Set(await takeUnannounced($, all.map(a => a.id)))
  const fresh = all.filter(a => ids.has(a.id))
  for (const a of fresh) {
    uiLog($, a.text)
    if (a.toast) uiToast($, a.toast)
  }
}

/**
 * The long-poll loop. One per module instance; a reload starts a new one and
 * the old one stops at its next turn. Waits between empty answers and backs
 * off while the engine is away, so there is no polling spam.
 */
export function startWatcher($: EngineInterface): void {
  if (Q.headless) return
  const mine = ++generation
  void (async () => {
    let backoff = 5_000
    let failBackoff = 5_000
    let logged = false
    // Whether the snapshot and status line were drawn while the link was on; false after
    // any time off, so a link that comes back (here or elsewhere) is repainted at once.
    let drawnOn = Q.link.on && !!Q.state
    while (mine === generation) {
      try {
        if (!Q.link.on) {
          drawnOn = false
          const reason = Q.link.reason
          // Never start the engine for these reasons, but notice when the person has acted
          // elsewhere since: installed, set up, started or updated it from the command line.
          // Only a handshake with an engine that is already running; nothing is started here.
          if (reason === 'no_consent' || reason === 'stopped_by_user' || reason === 'incompatible' || reason === 'not_installed') {
            await pause($, 30_000)
            if (mine !== generation) break
            await establish($, { allowStart: false }).catch(() => {})
            if (!Q.link.on) await paintStatus($)
            continue
          }
          await establish($, { allowStart: false }).catch(() => {})
          if (!Q.link.on) {
            await paintStatus($)
            await pause($, backoff)
            backoff = Math.min(backoff * 2, 60_000)
            continue
          }
        }
        backoff = 5_000
        if (!drawnOn || !Q.state) {
          await refresh($)
          if (!Q.link.on) {
            // Shook hands, then lost the connection at once: wait before reconnecting.
            await pause($, failBackoff)
            failBackoff = Math.min(failBackoff * 2, 60_000)
            continue
          }
          drawnOn = !!Q.state
        }
        const startedAt = Date.now()
        let ev: EventsResponse
        try {
          ev = await call<EventsResponse>($, 'GET', `/v1/events?since=${Q.seq}&wait=25`, undefined, { timeoutMs: 35_000 })
          if (!isEvents(ev)) throw new EngineError('the engine answered /v1/events without events', 'bad_response')
        } catch (err) {
          // A lost connection turns the link off (the branch above reconnects); any other
          // failure (an error status, a missing route after a version skew, a malformed
          // answer) keeps it. Either way, wait before the next try: no tight loop.
          if (hasCode(err, 'unreachable')) await markUnreachable($)
          else drawnOn = false
          await paintStatus($)
          await pause($, failBackoff)
          failBackoff = Math.min(failBackoff * 2, 60_000)
          continue
        }
        if (ev.events.length && !(await refresh($))) {
          // The state behind this batch was not read: keep the sequence where it was, so
          // the batch is not lost, and refresh again (after a pause) before the next poll.
          drawnOn = false
          await pause($, failBackoff)
          failBackoff = Math.min(failBackoff * 2, 60_000)
          continue
        }
        failBackoff = 5_000
        Q.seq = Math.max(Q.seq, ev.seq)
        // An engine that answers at once with nothing (no long-poll support) is not hammered.
        if (Date.now() - startedAt < 2_000) await pause($, 5_000)
      } catch (err) {
        // Nothing here may end the loop; say so once in the transcript (never headless).
        if (!logged) {
          logged = true
          uiLog($, `Quayflow: status updates hit an error and will retry (${errorText(err)}).`)
        }
        await pause($, failBackoff)
        failBackoff = Math.min(failBackoff * 2, 60_000)
      }
    }
  })().catch(() => {})
}

function isEvents(v: unknown): v is EventsResponse {
  if (!v || typeof v !== 'object') return false
  const e = v as { seq?: unknown; events?: unknown }
  return Array.isArray(e.events) && typeof e.seq === 'number'
}
