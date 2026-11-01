// Interactive sessions only: keeps the status line and the pane's snapshot
// current through the engine's event long-poll (GET /v1/events?wait=25), and
// announces each suggestion or notice once. A headless run never starts it.
import type { EngineInterface } from 'claude-code'

import type { EventsResponse, StateView } from '../types'
import { Q, pause, seedAnnounced, setSnapshot, takeUnannounced, uiLog, uiStatus, uiToast } from './context'
import { call, establish, fetchState, markUnreachable } from './engine'
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
  } catch {
    await markUnreachable($)
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
    while (mine === generation) {
      if (!Q.link.on) {
        const reason = Q.link.reason
        // Stay off until the person acts (onboarding, /quay start, update).
        if (reason === 'no_consent' || reason === 'stopped_by_user' || reason === 'incompatible' || reason === 'not_installed') {
          await pause($, 30_000)
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
      if (!Q.state) await refresh($)
      const startedAt = Date.now()
      try {
        const ev = await call<EventsResponse>($, 'GET', `/v1/events?since=${Q.seq}&wait=25`, undefined, { timeoutMs: 35_000 })
        if (ev.events.length) {
          await refresh($)
          Q.seq = Math.max(Q.seq, ev.seq)
        } else {
          Q.seq = Math.max(Q.seq, ev.seq)
        }
      } catch {
        await markUnreachable($)
        await paintStatus($)
        continue
      }
      // An engine that answers at once with nothing (no long-poll support) is not hammered.
      if (Date.now() - startedAt < 2_000) await pause($, 5_000)
    }
  })()
}
