// Cell graphics: what each draws, read back as text (color only repeats the figures).
import { describe, expect, test } from 'claude-code/testing'

import { ACCENT, RED, base64, fleet, gauge, heat, track } from '../hooks/chart'
import { rankPicks } from '../hooks/groupedit'
import { agoText, signInText } from '../hooks/format'

describe('chart cells', () => {
  test('base64 matches the standard encoding', () => {
    const bytes = new Uint8Array([0x4d, 0x61, 0x6e, 0x4d, 0x61])
    expect(base64(bytes)).toBe('TWFuTWE=')
    expect(base64(new Uint8Array([0x4d]))).toBe('TQ==')
  })

  test('a gauge fills to its figure and marks the threshold ahead of it', () => {
    const g = gauge(50, 10, 85, 10).lines()[0]!
    expect(g.slice(0, 5)).toBe('█████')
    expect(g[8]).toBe('┃')
    expect(gauge(undefined, 6).lines()[0]).toBe('╌╌╌╌╌╌')
    expect(gauge(100, 4).lines()[0]).toBe('████')
  })

  test('heat is red from the threshold and never red below the margin', () => {
    expect(heat(90, 85, 10)).toBe(RED)
    expect(heat(20, 85, 10)).not.toBe(RED)
  })

  test('the fleet chart: live and recommended marks, figures, threshold line', () => {
    const cv = fleet([
      { five: 88, seven: 31, live: true, recommended: false },
      { five: undefined, seven: undefined, live: false, recommended: true },
    ])
    const lines = cv.lines()
    expect(lines[0]).toContain('▼')
    expect(lines[0]).toContain('★')
    expect(lines[1]).toContain('88')
    expect(lines[1]).toContain('31')
    expect(lines[1]).toContain('--')
    expect(lines.some(l => l.includes('╌'))).toBe(true)
    expect(lines[lines.length - 1]).toContain('5h')
    const words = cv.words
    expect(Array.from(words).includes(ACCENT)).toBe(true)
  })

  test('the step track fills behind the current step', () => {
    expect(track(['done', 'current', 'todo'], 3).lines()[0]).toBe('●━━◉──○')
    expect(track(['done', 'failed'], 2).lines()[0]).toBe('●━✖')
  })
})

import { notTarget } from '../hooks/pane'

describe('account rows: why not a switch target', () => {
  test('a short reason, nothing for what a badge already says', () => {
    expect(notTarget('already live')).toBeUndefined()
    expect(notTarget('sign-in expired (refresh rejected); reauthenticate')).toBeUndefined()
    expect(notTarget('81% 5h; needs 75% or less to be worth a switch')).toEqual({ text: 'no room to gain' })
    expect(notTarget('at its limit (100% 5h)')).toEqual({ text: 'at its limit', warn: true })
    expect(notTarget('usage unknown: last read 2 h ago, too old to count as room')).toEqual({ text: 'usage unknown' })
    expect(notTarget('usage observation 2 h old (stale is never zero)')).toEqual({ text: 'usage unknown' }) // an engine before 0.3
    expect(notTarget('refused by this environment\'s managed login enforcement (x)')?.warn).toBe(true)
  })
})

describe('group editor what-if ranking (as qf_policy::rank_by_score)', () => {
  const p = (id: string, pct: number, score: number | undefined, rank: 'preferred' | 'normal' | 'backup' = 'normal') => ({ id, label: id, pct, score, rank })
  test('scores within 5%/day are a tie that goes to more five-hour room', () => {
    expect(rankPicks([p('a', 70, 30), p('b', 10, 27)]).map(x => x.id)).toEqual(['b', 'a'])
  })
  test('a clearly higher score wins over five-hour room; unscored come last', () => {
    expect(rankPicks([p('a', 70, 30), p('b', 10, 20), p('u', 0, undefined)]).map(x => x.id)).toEqual(['a', 'b', 'u'])
  })
  test('within 10 points of usage, rank decides', () => {
    expect(rankPicks([p('a', 10, 30, 'backup'), p('b', 15, 29)]).map(x => x.id)).toEqual(['b', 'a'])
  })
})

describe('sign-in times', () => {
  const NOW = 1_000_000_000_000
  const H = 3_600_000
  test('the last renewal and the sign-in age; no countdown', () => {
    expect(signInText({ health: 'ok', registeredAt: NOW - 40 * 24 * H, signedInAt: NOW - 12 * 24 * H, renewedAt: NOW - 3 * H }, NOW)).toBe('renewed 3 h ago \u00b7 signed in 12 d ago')
    expect(signInText({ health: 'stale_access', registeredAt: NOW - 40 * 24 * H }, NOW)).toBe('renewal not seen yet \u00b7 added 40 d ago')
    expect(signInText({ health: 'refresh_rejected', registeredAt: NOW - 2 * H, renewedAt: NOW - 30 * 60000 }, NOW)).toBe('refused, last renewed 30 min ago \u00b7 added 2 h ago')
    expect(signInText({ health: 'ok' }, NOW)).toBe('renewal not seen yet')
    expect(agoText(20_000)).toBe('just now')
    expect(agoText(-5)).toBe('just now')
  })
})
