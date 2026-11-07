// Cell graphics: what each draws, read back as text (color only repeats the figures).
import { describe, expect, test } from 'claude-code/testing'

import { ACCENT, RED, base64, fleet, gauge, heat, track } from '../hooks/chart'

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
    expect(notTarget('usage observation 2 h old (stale is never zero)')).toEqual({ text: 'usage unknown' })
    expect(notTarget('refused by this environment\'s managed login enforcement (x)')?.warn).toBe(true)
  })
})
