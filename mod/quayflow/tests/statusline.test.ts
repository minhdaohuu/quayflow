// The status line (design C01: `Quayflow · <Account> · <Mode> · <one tail>`), the
// detection of a Claude Code status line that already shows the limits, and the
// one-time transcript announcements of engine events.
// Run: claude plugin test mod/quayflow
import { describe, expect, test } from 'claude-code/testing'

import { clock, statusLine, statusParts } from '../hooks/format'
import type { StatusOptions } from '../hooks/format'
import type { EnvView, QuayLink, StateView } from '../types'
import { STATE } from './fixtures'
import { start, world } from './harness'

const ON: QuayLink = { on: true, engineVersion: '0.1.0' }
const ENV = STATE.envs[0]!
const WORK = STATE.accounts[0]!
const PAUSED = { automationPaused: true }

function st(env: Partial<EnvView>, top: Partial<StateView> = {}): StateView {
  return { ...STATE, ...top, envs: [{ ...ENV, ...env }] }
}
function line(s: StateView, opts?: StatusOptions): string | undefined {
  return statusLine(ON, s, opts)
}
const PIN_UNTIL = 1_800_007_200_000

describe('status line: off states', () => {
  const cases: [QuayLink, string | undefined][] = [
    [{ on: false, reason: 'connecting' }, undefined],
    [{ on: false, reason: 'unreachable' }, 'Quayflow · off · engine not running (Claude works as usual)'],
    [{ on: false, reason: 'not_installed' }, 'Quayflow · off · engine not installed (Claude works as usual)'],
    [{ on: false, reason: 'no_consent' }, 'Quayflow · not set up · run /quay to review and start'],
    [{ on: false, reason: 'stopped_by_user' }, 'Quayflow · off · engine stopped by you (/quay start)'],
    [{ on: false, reason: 'start_failed', message: 'port 47123 in use' }, 'Quayflow · off · engine did not start (port 47123 in use)'],
    [{ on: false, reason: 'incompatible', engineVersion: '0.3.0' }, 'Quayflow · off · update needed: incompatible with engine 0.3.0 (quayflow update)'],
  ]
  for (const [link, want] of cases) {
    test(`${link.reason}`, () => {
      expect(statusLine(link, undefined)).toBe(want)
    })
  }

  test('setup not finished', () => {
    expect(line({ ...STATE, onboarded: false })).toBe('Quayflow · not set up · finish setup in /quay')
  })
})

describe('status line: one tail by priority', () => {
  const unknown = st({
    ...PAUSED,
    live: { account: 'c3c3c3c3c3c3', known: false, kind: 'personal' },
    pausedReason: 'the live login #c3c3c3c3c3c3 is not registered: register it (/quay register) or switch manually',
    notices: [{ id: 'unknown_login:default', level: 'warn', kind: 'unknown_login', text: 'The live login (#c3c3c3c3c3c3) is not registered.', persistent: true }],
  })
  const cases: [string, StateView, string][] = [
    [
      'mixed login files',
      st({ ...PAUSED, pausedReason: 'the login files are inconsistent (credentials of one account, account record of another); switch to an account to repair them' }),
      'Quayflow · work · Suggest · mixed login files: switch to an account to repair',
    ],
    [
      'in-place switching off',
      st({
        ...PAUSED,
        inPlace: { allowed: false, reason: 'Claude 2.2.0 is not checked yet' },
        pausedReason: 'in-place switching disabled: Claude 2.2.0 is not checked yet',
        notices: [{ id: 'incompatible_version:default', level: 'warn', kind: 'incompatible_version', text: 'In-place switching is off.', persistent: true }],
      }),
      'Quayflow · work · Suggest · in-place switching off for this Claude version',
    ],
    [
      'an error notice beats the suggestion',
      st({ notices: [{ id: 'vault_error:default', level: 'error', kind: 'vault_error', text: 'personal: its protected copy cannot be read.', persistent: true }] }),
      'Quayflow · work · Suggest · personal: its protected copy cannot be read',
    ],
    ['unknown login', unknown, 'Quayflow · Unknown login · Suggest · not registered: /quay register'],
    [
      'signed out',
      st({ ...PAUSED, live: { known: false, signedOut: true }, pausedReason: 'no login in this environment', suggestion: null }),
      'Quayflow · Signed out · Suggest · sign in to Claude (/login)',
    ],
    [
      'another switcher',
      st({ ...PAUSED, pausedReason: 'the login changed repeatedly without Quayflow', notices: [{ id: 'other_switcher:default', level: 'warn', kind: 'other_switcher', text: 'x', persistent: true }] }),
      'Quayflow · work · Suggest · another switcher may be active',
    ],
    ['paused by you', st({ ...PAUSED, pausedReason: 'automation paused by you (/quay resume)' }), 'Quayflow · work · Suggest · automation paused (/quay resume)'],
    [
      'switching',
      st({ transition: { id: 't1', env: 'default', from: WORK.id, to: 'b2b2b2b2b2b2', how: 'manual', reason: 'by you', state: 'applying', requestedAt: 1 } }),
      'Quayflow · work · Suggest · switching to personal',
    ],
    [
      'waiting for an idle moment',
      st({ transition: { id: 't1', env: 'default', from: WORK.id, to: 'b2b2b2b2b2b2', how: 'manual', reason: 'by you', state: 'idle_wait', requestedAt: 1 } }),
      'Quayflow · work · Suggest · switching to personal: waiting for an idle moment',
    ],
    [
      'pending next request',
      st(
        {
          live: { account: 'b2b2b2b2b2b2', known: true, label: 'personal' },
          suggestion: null,
          transition: { id: 't1', env: 'default', from: WORK.id, to: 'b2b2b2b2b2b2', how: 'accepted', reason: 'by you', state: 'verifying_adoption', requestedAt: 1 },
        },
        { session: { ...STATE.session!, adoption: { sessionId: 'S1', pid: 100, transition: 't1', destination: 'b2b2b2b2b2b2', state: 'pending_next_request' } } },
      ),
      'Quayflow · personal · Suggest · pending next request',
    ],
    ['suggestion', STATE, 'Quayflow · work · Suggest · 5h 91% · 7d 40% | personal suggested'],
    ['pin', st({ suggestion: null, pin: { account: WORK.id, until: PIN_UNTIL, at: 1 } }), `Quayflow · work · Suggest · pinned until ${clock(PIN_UNTIL)}`],
    [
      'pin to another account',
      st({ suggestion: null, pin: { account: 'b2b2b2b2b2b2', until: PIN_UNTIL, at: 1 } }),
      `Quayflow · work · Suggest · pinned to personal until ${clock(PIN_UNTIL)}`,
    ],
    ['managed-login override ON', st({ suggestion: null }, { dOrg: { enabled: true, confirmedOrg: '0f0f0f0f0f0f' } }), 'Quayflow · work · Suggest · managed-login override ON'],
    [
      'a warning notice',
      st({ suggestion: null, notices: [{ id: 'refresh_rejected:default', level: 'warn', kind: 'refresh_rejected', text: 'personal needs a new sign-in; it is set aside.', persistent: true }] }),
      'Quayflow · work · Suggest · personal needs a new sign-in',
    ],
    [
      'stale usage',
      st({ suggestion: null }, { accounts: [{ ...WORK, usage: { ...WORK.usage!, fresh: false, ageS: 1500 } }, STATE.accounts[1]!] }),
      'Quayflow · work · Suggest · usage unknown (read 25 min ago)',
    ],
    ['home account', st({ suggestion: null }), 'Quayflow · work · Suggest · Home account · 5h 91% · 7d 40%'],
    [
      'an info notice never shows',
      st({ suggestion: null, notices: [{ id: 'managed_login:default', level: 'info', kind: 'managed_login', text: 'Your organization enforces its login here.', persistent: true }] }),
      'Quayflow · work · Suggest · Home account · 5h 91% · 7d 40%',
    ],
    ['away from home', st({ suggestion: null }, { groups: [{ ...STATE.groups[0]!, preferred: 'b2b2b2b2b2b2' }] }), 'Quayflow · work · Suggest · Away from home · 5h 91% · 7d 40%'],
    ['plain usage (no Preferred)', st({ suggestion: null }, { groups: [{ ...STATE.groups[0]!, preferred: null }] }), 'Quayflow · work · Suggest · 5h 91% · 7d 40%'],
    ['Automatic mode', st({ suggestion: null, mode: 'automatic' }), 'Quayflow · work · Automatic · Home account · 5h 91% · 7d 40%'],
    ['Manual mode', st({ suggestion: null, mode: 'manual' }), 'Quayflow · work · Manual · Home account · 5h 91% · 7d 40%'],
  ]
  for (const [name, s, want] of cases) {
    test(name, () => {
      expect(line(s)).toBe(want)
    })
  }

  test('only a line that needs the person is urgent (pinned); the everyday line is quiet', () => {
    const urgent = new Set([
      'mixed login files', 'in-place switching off', 'an error notice beats the suggestion', 'unknown login', 'signed out',
      'another switcher', 'paused by you', 'suggestion', 'managed-login override ON', 'a warning notice',
    ])
    for (const [name, s] of cases) expect([name, statusParts(ON, s).urgent]).toEqual([name, urgent.has(name)])
    expect(statusParts({ on: false, reason: 'unreachable' }, undefined).urgent).toBe(true)
    expect(statusParts({ on: false, reason: 'connecting', message: 'starting the engine' }, undefined).urgent).toBe(false)
    expect(statusParts(ON, { ...STATE, onboarded: false }).urgent).toBe(true)
  })

  test('no fact is said twice', () => {
    const text = line(unknown) ?? ''
    expect(text.toLowerCase().split('unknown login')).toHaveLength(2)
    expect(text).not.toContain('automation paused')
    expect(line(st({ ...PAUSED, live: { known: false, signedOut: true }, pausedReason: 'no login in this environment' }))?.toLowerCase().split('signed out')).toHaveLength(2)
  })

  test('without usage figures: the suggestion names the account only; no %, home or plain', () => {
    expect(line(STATE, { showUsage: false })).toBe('Quayflow · work · Suggest · personal suggested')
    expect(line(st({ suggestion: null }), { showUsage: false })).toBe('Quayflow · work · Suggest · Home account')
    expect(line(st({ suggestion: null }, { groups: [{ ...STATE.groups[0]!, preferred: null }] }), { showUsage: false })).toBe('Quayflow · work · Suggest')
  })

  const plain = (windows: { key: string; pct: number }[]): StateView =>
    st(
      { suggestion: null },
      {
        groups: [{ ...STATE.groups[0]!, preferred: null }],
        accounts: [{ ...WORK, usage: { ...WORK.usage!, windows, binding: windows[0] ?? null } }, STATE.accounts[1]!],
      },
    )
  const OPUS = [{ key: '5h', pct: 45 }, { key: '7d', pct: 30 }, { key: '7d:Opus', pct: 88 }]

  test('the per-model weekly window shows when it applies to the model and is the fullest', () => {
    expect(line(plain(OPUS), { model: 'claude-opus-5-5' })).toBe('Quayflow · work · Suggest · 5h 45% · 7d 30% · Opus 7d 88%')
    // Another model's window does not apply.
    expect(line(plain(OPUS), { model: 'claude-sonnet-4-6' })).toBe('Quayflow · work · Suggest · 5h 45% · 7d 30%')
    // Not the fullest applicable window: left out.
    expect(line(plain([{ key: '5h', pct: 95 }, { key: '7d', pct: 30 }, { key: '7d:Opus', pct: 60 }]), { model: 'claude-opus-5-5' })).toBe(
      'Quayflow · work · Suggest · 5h 95% · 7d 30%',
    )
    // No model known yet: every per-model window applies (the engine's rule).
    expect(line(plain(OPUS))).toBe('Quayflow · work · Suggest · 5h 45% · 7d 30% · Opus 7d 88%')
    // With a suggestion too.
    expect(line(st({}, { accounts: [{ ...WORK, usage: { ...WORK.usage!, windows: OPUS } }, STATE.accounts[1]!] }), { model: 'claude-opus-5-5' })).toBe(
      'Quayflow · work · Suggest · 5h 45% · 7d 30% · Opus 7d 88% | personal suggested',
    )
  })

  test('a missing window is left out', () => {
    expect(line(plain([{ key: '5h', pct: 45 }]))).toBe('Quayflow · work · Suggest · 5h 45%')
    expect(line(plain([{ key: 'seven_day', pct: 30 }]))).toBe('Quayflow · work · Suggest · 7d 30%')
    expect(line(plain([]))).toBe('Quayflow · work · Suggest')
  })

  test('the width budget drops usage first, then cuts the home text', () => {
    const home = st({ suggestion: null })
    expect(line(home, { width: 60 })).toBe('Quayflow · work · Suggest · Home account · 5h 91% · 7d 40%')
    expect(line(home, { width: 50 })).toBe('Quayflow · work · Suggest · Home account')
    const opus = st({ suggestion: null }, { accounts: [{ ...WORK, usage: { ...WORK.usage!, windows: OPUS } }, STATE.accounts[1]!] })
    // The per-model figure goes before the 5h / 7d figures.
    expect(line(opus, { width: 62, model: 'claude-opus-5-5' })).toBe('Quayflow · work · Suggest · Home account · 5h 45% · 7d 30%')
    expect(line(STATE, { width: 50 })).toBe('Quayflow · work · Suggest · personal suggested')
  })

  test('the length is budgeted: the tail is cut, never the account label', () => {
    const long = 'Harbor Studio engineering team account'
    const s = st({ live: { account: WORK.id, known: true, label: long } }, { accounts: [{ ...WORK, label: long }, STATE.accounts[1]!] })
    const text = line(s, { width: 72 }) ?? ''
    expect(text.length).toBeLessThanOrEqual(72)
    expect(text).toContain(long)
    expect(text.endsWith('…')).toBe(true)
    // Too narrow for any tail: the account and mode stay whole.
    expect(line(s, { width: 40 })).toBe(`Quayflow · ${long} · Suggest`)
    expect(line(STATE, { width: 200 })).toBe('Quayflow · work · Suggest · 5h 91% · 7d 40% | personal suggested')
  })
})

describe('status line: usage figures and the Claude Code status line', () => {
  const last = (w: ReturnType<typeof world>) => w.ui.filter(u => u.kind === 'ui.status').pop()?.text ?? ''
  const SCRIPT = 'C:\\Users\\t\\.claude\\statusline.ps1'

  test('a configured status line that shows the limits: no usage % in ours', async ($, on) => {
    const w = world(on)
    on('settings.read', () => ({ value: { statusLine: { type: 'command', command: 'node -e "console.log(JSON.parse(s).rate_limits.five_hour)"' } } }))
    await start($, w, true)
    expect(last(w)).toBe('Quayflow · work · Suggest · personal suggested')
  })

  test('a script the command runs is read (never run) for the limits', async ($, on) => {
    const w = world(on, {
      env: { USERPROFILE: 'C:\\Users\\t' },
      files: { [SCRIPT]: '$j = $input | ConvertFrom-Json; "$($j.rate_limits.seven_day.used_percentage)%"' },
    })
    on('settings.read', () => ({ value: { statusLine: { type: 'command', command: 'pwsh -NoProfile -File ~/.claude/statusline.ps1' } } }))
    await start($, w, true)
    expect(last(w)).toBe('Quayflow · work · Suggest · personal suggested')
    expect(w.runs.some(r => r.join(' ').includes('statusline'))).toBe(false)
  })

  test('a configured status line without the limits: usage % stays', async ($, on) => {
    const w = world(on)
    on('settings.read', () => ({ value: { statusLine: { type: 'command', command: 'echo "$(git branch --show-current)"' } } }))
    await start($, w, true)
    expect(last(w)).toBe('Quayflow · work · Suggest · 5h 91% · 7d 40% | personal suggested')
  })

  test('statusLineUsage = show keeps the % even when the other line shows the limits', { options: { statusLineUsage: 'show' } }, async ($, on) => {
    const w = world(on)
    on('settings.read', () => ({ value: { statusLine: { type: 'command', command: 'ccline --five_hour --seven_day' } } }))
    await start($, w, true)
    expect(last(w)).toBe('Quayflow · work · Suggest · 5h 91% · 7d 40% | personal suggested')
  })

  test('statusLineUsage = hide drops the % with no status line configured', { options: { statusLineUsage: 'hide' } }, async ($, on) => {
    const w = world(on)
    await start($, w, true)
    expect(last(w)).toBe('Quayflow · work · Suggest · personal suggested')
  })
})

describe('announcements of engine events', () => {
  const NOW = 1_800_000_000_000
  const batch = [
    { seq: 10, ts: NOW - 3_600_000, kind: 'notice', env: 'default', text: 'Old news from before this session.' },
    { seq: 43, ts: NOW + 5_000, kind: 'notice', env: 'default', text: 'The login changed outside Quayflow to personal.' },
    { seq: 44, ts: NOW + 6_000, kind: 'usage', env: 'default', text: null },
    { seq: 45, ts: NOW + 7_000, kind: 'switched', env: 'default', text: 'Switched to personal (accepted).' },
    { seq: 46, ts: NOW + 8_000, kind: 'adoption', env: 'default', text: 'Session S2 now runs on personal (verified on its request).' },
    { seq: 47, ts: NOW + 9_000, kind: 'notice', env: 'lane-a', text: 'Another environment.' },
    { seq: 48, ts: NOW + 9_500, kind: 'state', env: null, text: 'Pin cleared.' },
  ]

  test('each notice and switch once; no adoption, no history, no usage, no other environment', async ($, on) => {
    // The engine answers the same batch to every poll: the Mod still tells each event once.
    const w = world(on, { routes: { '/v1/events': { seq: 48, events: batch } } })
    await start($, w, true)
    await w.clock.advance(60_000)
    await w.clock.settle()
    const logs = w.ui.filter(u => u.kind === 'ui.log').map(u => u.text ?? '')
    const of = (t: string) => logs.filter(l => l.includes(t))
    expect(of('The login changed outside Quayflow to personal.')).toEqual(['Quayflow: The login changed outside Quayflow to personal.'])
    expect(of('Switched to personal')).toHaveLength(1)
    expect(of('Session S2 now runs on personal')).toHaveLength(0)
    expect(of('Old news')).toHaveLength(0)
    expect(of('Another environment')).toHaveLength(0)
    expect(of('Pin cleared')).toHaveLength(0)
    expect(w.routes().filter(r => r === '/v1/events').length).toBeGreaterThanOrEqual(3)
    // The suggestion is still told once, with its one toast.
    expect(of('Quayflow suggests "personal"')).toHaveLength(1)
    expect(w.ui.filter(u => u.kind === 'ui.toast')).toHaveLength(1)
  })

  test('headless: nothing is announced and events are not polled', async ($, on) => {
    const w = world(on, { routes: { '/v1/events': { seq: 48, events: batch } } })
    await start($, w, false)
    await w.clock.advance(60_000)
    expect(w.ui).toEqual([])
    expect(w.routes()).not.toContain('/v1/events')
  })

  test('a restarted engine counts from zero: its events are still told', async ($, on) => {
    let restarted = false
    const w = world(on, {
      routes: {
        '/v1/state': () => (restarted ? { ...STATE, seq: 2, engine: { ...STATE.engine, pid: 77, startedAt: NOW + 20_000 } } : STATE),
        '/v1/events': (_b: Record<string, unknown>, q: URLSearchParams) => {
          if (!restarted) return { seq: 42, events: [] }
          const since = Number(q.get('since'))
          const ev = [{ seq: 2, ts: NOW + 21_000, kind: 'notice', env: 'default', text: 'After a restart: the switch completed.' }]
          return { seq: 2, events: ev.filter(e => e.seq > since) }
        },
      },
    })
    await start($, w, true)
    restarted = true
    await w.clock.advance(60_000)
    await w.clock.settle()
    expect(w.ui.filter(u => u.kind === 'ui.log' && (u.text ?? '').includes('After a restart'))).toHaveLength(1)
  })
})
