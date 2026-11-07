// Review screens of the /quay pane against a mocked engine: what each shows, and that
// nothing is sent to the engine before its explicit step.
import { describe, expect, test } from 'claude-code/testing'

import type { ExplainView, StateView } from '../types'
import { EXPLAIN, STATE } from './fixtures'
import { quay, start, world } from './harness'

const PANE = {
  plugin: 'quayflow',
  component: 'Pane',
  requestId: 'quay',
  props: { title: 'Quayflow', isFocused: true, bodyColumns: 120, placement: 'dock', scroll: { offset: 0, bodyRows: 60 }, view: {} },
} as const

const ENV = STATE.envs[0]!

/** STATE plus a lane "studio" whose login is personal. */
const WITH_LANE: StateView = {
  ...STATE,
  envs: [
    ENV,
    { ...ENV, id: 'studio', configDir: 'C:\\q\\lanes\\studio', live: { account: 'b2b2b2b2b2b2', known: true, label: 'personal' }, sessions: [], suggestion: null, governingGroups: ['Default'] },
  ],
  lanes: [{ name: 'studio', configDir: 'C:\\q\\lanes\\studio', live: 'b2b2b2b2b2b2', linksOk: true, linkDetail: [], canaryPassed: true, sessions: 0 }],
}

describe('overview and decisions', () => {
  test('Why this account? names each candidate, the threshold, margin and cooldown', async ($, on) => {
    const w = world(on)
    await start($, w, true)
    await quay($, '')
    const ui = await $.ui.mount({ ...PANE, surface: 'terminal' })
    await ui.press({ key: 'hints' })
    expect((await ui.find({ key: 'ov-acct:a1a1a1a1a1a1' }))?.text).toBeDefined()
    expect((await ui.find({ key: 'ov-acct:b2b2b2b2b2b2' }))?.text).toBeDefined()
    await ui.press({ key: 'ov-why' })
    expect(await ui.find({ type: 'Text', text: /^Eligible: 12% 7d/ })).toBeDefined()
    expect(await ui.find({ type: 'Text', text: /^Live: live here/ })).toBeDefined()
    expect(await ui.find({ type: 'Text', text: /Required margin\s+at least 10 points below the threshold \(75% or less\)/ })).toBeDefined()
    expect(await ui.find({ type: 'Text', text: /Proactive cooldown\s+5 min; elapsed/ })).toBeDefined()
    // Reading the explanation changes nothing.
    expect(w.routes().filter(r => r !== '/v1/explain' && r !== '/v1/state' && r !== '/v1/events' && r !== '/v1/hello')).toEqual([])
    await ui.unmount()
  })

  for (const [state, words] of [
    ['deferred_pin', 'Home ready, return deferred (pinned)'],
    ['away', 'Away from home'],
    ['ready', 'Home ready'],
  ] as const) {
    test(`home state "${state}" reads "${words}"; the policy is the engine's`, async ($, on) => {
      const explain: ExplainView = { ...EXPLAIN, live: 'b2b2b2b2b2b2', homeState: state, homeDetail: 'work has room again (31% 5h (1 min old)).' }
      const live: StateView = { ...STATE, envs: [{ ...ENV, live: { account: 'b2b2b2b2b2b2', known: true }, suggestion: null }] }
      const w = world(on, { state: live, routes: { '/v1/explain': explain } })
      await start($, w, true)
      await quay($, '')
      const ui = await $.ui.mount({ ...PANE, surface: 'terminal' })
      await ui.press({ key: 'hints' })
      expect(await ui.find({ type: 'Text', text: new RegExp(`^Home account: ${words.replace(/[()]/g, '\\$&')}$`) })).toBeDefined()
      await ui.press({ key: 'home-pin' })
      expect(w.bodies('/v1/pin')).toEqual([{ env: 'default' }])
      await ui.unmount()
    })
  }

  test('Resume is offered only for a pause it ends; other causes show their remedy', async ($, on) => {
    const unknown: StateView = { ...STATE, envs: [{ ...ENV, live: { account: 'c3c3c3c3c3c3', known: false }, automationPaused: true, pausedBy: 'unknown_login', pausedReason: 'the live login #c3c3 is not registered' }] }
    const w = world(on, { state: unknown })
    await start($, w, true)
    await quay($, '')
    const ui = await $.ui.mount({ ...PANE, surface: 'terminal' })
    await ui.press({ key: 'hints' })
    expect(await ui.find({ key: 'ov-resume' })).toBeUndefined()
    expect(await ui.find({ key: 'ov-unknown' })).toBeDefined()
    await ui.press({ key: 'tab:settings' })
    expect(await ui.find({ key: 'se-resume' })).toBeUndefined()
    expect(await ui.find({ key: 'se-remedy' })).toBeDefined()
    await ui.unmount()
  })

  test('a pause by the person can be resumed from the overview', async ($, on) => {
    const paused: StateView = { ...STATE, envs: [{ ...ENV, automationPaused: true, pausedBy: 'user', pausedReason: 'automation paused by you (/quay resume)' }] }
    const w = world(on, { state: paused })
    await start($, w, true)
    await quay($, '')
    const ui = await $.ui.mount({ ...PANE, surface: 'terminal' })
    await ui.press({ key: 'hints' })
    await ui.press({ key: 'ov-resume' })
    expect(w.bodies('/v1/engine')).toEqual([{ action: 'resume' }])
    // The engine still reports the pause (the mocked state does not change): no false "resumed".
    expect(await ui.find({ type: 'Text', text: /Still paused: automation paused by you/ })).toBeDefined()
    await ui.unmount()
  })

  test('limits: the all-exhausted table and the native wait shown as unknown', async ($, on) => {
    const now = 1_800_000_000_000
    const explain: ExplainView = {
      ...EXPLAIN,
      allExhausted: true,
      soonest: 'b2b2b2b2b2b2',
      candidates: [
        { ...EXPLAIN.candidates[0]!, exhausted: true, availableAt: now + 7_200_000 },
        { ...EXPLAIN.candidates[1]!, status: 'excluded', reason: 'at its limit (100% 5h)', exhausted: true, availableAt: now + 1_800_000 },
      ],
    }
    const w = world(on, { routes: { '/v1/explain': explain } })
    await start($, w, true)
    await quay($, '')
    const ui = await $.ui.mount({ ...PANE, surface: 'terminal' })
    await ui.press({ key: 'hints' })
    await ui.press({ key: 'ov-limits' })
    expect(await ui.find({ type: 'Text', text: /\(selected\)$/ })).toBeDefined()
    expect(await ui.find({ type: 'Text', text: /Native wait setting\s+Unknown: Quayflow does not read it/ })).toBeDefined()
    expect(await ui.find({ type: 'Text', text: /Headless jobs get a recommendation only/ })).toBeDefined()
    await ui.unmount()
  })
})

describe('registration', () => {
  test('register shows the live login\'s e-mail and organization before saving', async ($, on) => {
    const unknownLive: StateView = { ...STATE, envs: [{ ...ENV, live: { account: 'c3c3c3c3c3c3', known: false, kind: 'organization' } }] }
    const w = world(on, {
      state: unknownLive,
      routes: { '/v1/identity': { account: 'c3c3c3c3c3c3', kind: 'organization', display: 'avery@harbor.example', orgName: 'Harbor Studio' } },
    })
    await start($, w, true)
    await quay($, 'register')
    const ui = await $.ui.mount({ ...PANE, surface: 'terminal' })
    await ui.press({ key: 'hints' })
    expect(await ui.find({ type: 'Text', text: /Identity\s+avery@harbor\.example/ })).toBeDefined()
    expect(await ui.find({ type: 'Text', text: /Organization\s+Harbor Studio/ })).toBeDefined()
    expect(w.bodies('/v1/identity')).toEqual([{}])
    await ui.input({ key: 'reg:newgroup', text: 'Harbor' })
    expect(w.bodies('/v1/groups/create')).toEqual([{ name: 'Harbor' }])
    await ui.unmount()
  })

  test('a duplicate identity offers the existing registration; saving needs the storage box', async ($, on) => {
    const ready = { id: 'si-1', state: 'ready', configDir: 'C:\\t', command: 'claude', identity: { account: 'a1a1a1a1a1a1', kind: 'organization', duplicateOf: 'work' } }
    const w = world(on, { routes: { '/v1/accounts/signin/start': ready, '/v1/accounts/signin/status': ready } })
    await start($, w, true)
    await quay($, 'add')
    const ui = await $.ui.mount({ ...PANE, surface: 'terminal' })
    await ui.press({ key: 'hints' })
    // A name already in use is refused before any sign-in opens.
    await ui.input({ key: 'add:label', text: 'Personal', kind: 'change' } as never)
    expect(await ui.find({ type: 'Text', text: /"personal" is already a name here/ })).toBeDefined()
    await ui.press({ key: 'add-continue' })
    expect(w.routes()).not.toContain('/v1/accounts/signin/start')
    // Enter in the name field signs in.
    await ui.input({ key: 'add:label', text: 'work backup' })
    expect(await ui.find({ type: 'Text', text: /This account is already registered/ })).toBeDefined()
    expect(await ui.find({ key: 'signin-review-existing' })).toBeDefined()
    await ui.press({ key: 'signin-save' })
    expect(w.routes()).not.toContain('/v1/accounts/signin/confirm')
    await ui.press({ key: 'add:consent' })
    await ui.press({ key: 'signin-save' })
    expect(w.bodies('/v1/accounts/signin/confirm')).toEqual([{ id: 'si-1', label: 'work backup', group: 'Default', acceptDisclosure: false }])
    await ui.unmount()
  })

  test('a reauthentication that returned another identity can be registered as new, never over the old one', async ($, on) => {
    const ready = { id: 'si-2', state: 'ready', configDir: 'C:\\t', command: 'claude', identity: { account: 'd4d4d4d4d4d4', kind: 'personal', unexpected: true } }
    const w = world(on, { routes: { '/v1/accounts/signin/start': ready, '/v1/accounts/signin/status': ready } })
    await start($, w, true)
    await quay($, '')
    const ui = await $.ui.mount({ ...PANE, surface: 'terminal' })
    await ui.press({ key: 'hints' })
    await ui.press({ key: 'tab:accounts' })
    await ui.press({ key: 'acct:b2b2b2b2b2b2' })
    await ui.press({ key: 'ad-reauth' })
    await ui.press({ key: 'add-continue' })
    expect(await ui.find({ type: 'Text', text: /Reauthentication returned a different identity/ })).toBeDefined()
    expect(await ui.find({ key: 'signin-save' })).toBeUndefined()
    await ui.press({ key: 'signin-as-new' })
    await ui.input({ key: 'add:label', text: 'travel' })
    await ui.press({ key: 'add:consent' })
    await ui.press({ key: 'signin-save' })
    expect(w.bodies('/v1/accounts/signin/confirm')).toEqual([{ id: 'si-2', label: 'travel', group: 'Default', acceptDisclosure: false, asNew: true }])
    await ui.unmount()
  })

  test('Welcome says Quayflow is not affiliated with Anthropic; readiness is a table of checks', async ($, on) => {
    const fresh: StateView = { ...STATE, onboarded: false }
    const w = world(on, { state: fresh })
    await start($, w, true)
    await quay($, '')
    const ui = await $.ui.mount({ ...PANE, surface: 'terminal' })
    await ui.press({ key: 'hints' })
    expect(await ui.find({ type: 'Text', text: /not affiliated with or endorsed by Anthropic/ })).toBeDefined()
    await ui.press({ key: 'welcome-continue' })
    expect(await ui.find({ type: 'Text', text: /^Vault$/ })).toBeDefined()
    expect(await ui.find({ type: 'Text', text: /^Live adoption$/ })).toBeDefined()
    await ui.unmount()
  })
})

describe('full-history session actions', () => {
  test('Open New Session is reviewed and needs the one-executor box before anything is copied', async ($, on) => {
    const w = world(on, {
      state: WITH_LANE,
      routes: { '/v1/handoff/open': { ok: true, fileCount: 3, command: 'claude --resume S1', launched: true, message: 'A new window opened.' } },
    })
    await start($, w, true)
    await quay($, '')
    const ui = await $.ui.mount({ ...PANE, surface: 'terminal' })
    await ui.press({ key: 'hints' })
    await ui.press({ key: 'ov-actions' })
    expect((await ui.find({ key: 'sa:open' }))?.text).toMatch(/^\[x\] Open New Session/)
    await ui.press({ key: 'sa-review-open' })
    expect(await ui.find({ type: 'Text', text: /Window\s+New terminal window/ })).toBeDefined()
    await ui.press({ key: 'sa-open-go' })
    expect(w.routes()).not.toContain('/v1/handoff/open')
    await ui.press({ key: 'sa:own' })
    await ui.press({ key: 'sa-open-go' })
    expect(w.bodies('/v1/handoff/open')).toEqual([{ sessionId: 'S1', toEnv: 'studio' }])
    expect(await ui.find({ type: 'Text', text: /Full history prepared/ })).toBeDefined()
    expect(await ui.find({ type: 'Text', text: /3 files; source intact/ })).toBeDefined()
    await ui.unmount()
  })

  test('an unsupported shell is named before anything is committed', async ($, on) => {
    const w = world(on, {
      state: WITH_LANE,
      routes: { '/v1/handoff/check': { toEnv: 'studio', toLogin: 'b2b2b2b2b2b2', running: true, shell: 'cmd.exe', shellSupported: false, promptHook: false } },
    })
    await start($, w, true)
    await quay($, '')
    const ui = await $.ui.mount({ ...PANE, surface: 'terminal' })
    await ui.press({ key: 'hints' })
    await ui.press({ key: 'ov-actions' })
    await ui.press({ key: 'sa:replace' })
    await ui.press({ key: 'sa-review-replace' })
    expect(await ui.find({ type: 'Text', text: /cmd\.exe is not PowerShell 7 or Windows PowerShell 5\.1/ })).toBeDefined()
    expect(await ui.find({ key: 'sa-replace-go' })).toBeUndefined()
    expect(w.routes()).not.toContain('/v1/handoff/replace')
    await ui.unmount()
  })

  test('PowerShell without the hook: the hook is a separate opt-in', async ($, on) => {
    const w = world(on, {
      state: WITH_LANE,
      routes: { '/v1/handoff/check': { toEnv: 'studio', toLogin: 'b2b2b2b2b2b2', running: true, shell: 'pwsh.exe', shellSupported: true, promptHook: false } },
    })
    await start($, w, true)
    await quay($, '')
    const ui = await $.ui.mount({ ...PANE, surface: 'terminal' })
    await ui.press({ key: 'hints' })
    await ui.press({ key: 'ov-actions' })
    await ui.press({ key: 'sa-review-replace' })
    expect(await ui.find({ type: 'Text', text: /Prompt hook\s+Not enabled/ })).toBeDefined()
    expect((await ui.find({ key: 'sa:hook' }))?.text).toMatch(/^\[ \] Enable Quayflow same-terminal resume integration/)
    await ui.unmount()
  })
})

describe('activity, report and lifecycle', () => {
  test('the report is previewed, reviewed, then saved; never saved before', async ($, on) => {
    const w = world(on, { routes: { '/v1/report/preview': { id: 'r1', text: '{\n  "engine": {}\n}' }, '/v1/report/save': { path: 'C:\\q\\reports\\r.json', preview: '' } } })
    await start($, w, true)
    await quay($, '')
    const ui = await $.ui.mount({ ...PANE, surface: 'terminal' })
    await ui.press({ key: 'hints' })
    await ui.press({ key: 'tab:activity' })
    await ui.press({ key: 'act-report' })
    await ui.press({ key: 'rp-build' })
    expect(w.routes()).toContain('/v1/report/preview')
    await ui.press({ key: 'rp-save' })
    expect(w.routes()).not.toContain('/v1/report/save')
    await ui.press({ key: 'rp:reviewed' })
    await ui.press({ key: 'rp-save' })
    expect(w.bodies('/v1/report/save')).toEqual([{ id: 'r1' }])
    expect(w.routes()).not.toContain('/v1/report')
    expect(await ui.find({ type: 'Text', text: /Saved locally: C:\\q\\reports\\r\.json/ })).toBeDefined()
    await ui.unmount()
  })

  test('an operation opens its sanitized trail', async ($, on) => {
    const ops = [
      {
        id: 'op-14',
        ts: 1_799_999_000_000,
        env: 'default',
        how: 'manual',
        from: 'a1a1a1a1a1a1',
        to: 'b2b2b2b2b2b2',
        stage: 'written',
        records: [
          { ts: 1_799_999_000_000, event: 'transition.requested', id: 'op-14' },
          { ts: 1_799_999_000_300, event: 'transition.written', id: 'op-14', lockedMs: 300 },
        ],
      },
    ]
    const w = world(on, { routes: { '/v1/operations': ops } })
    await start($, w, true)
    await quay($, '')
    const ui = await $.ui.mount({ ...PANE, surface: 'terminal' })
    await ui.press({ key: 'hints' })
    await ui.press({ key: 'tab:activity' })
    await ui.press({ key: 'op:op-14' })
    expect(await ui.find({ type: 'Text', text: /^transition\.written$/ })).toBeDefined()
    expect(await ui.find({ type: 'Text', text: /lockedMs=300/ })).toBeDefined()
    expect(await ui.find({ type: 'Text', text: /From \/ to\s+work -> personal/ })).toBeDefined()
    await ui.unmount()
  })

  test('uninstall names separate vault and log deletions, both off by default', async ($, on) => {
    const w = world(on)
    await start($, w, true)
    await quay($, '')
    const ui = await $.ui.mount({ ...PANE, surface: 'terminal' })
    await ui.press({ key: 'hints' })
    await ui.press({ key: 'tab:settings' })
    await ui.press({ key: 'se-uninstall' })
    expect((await ui.find({ key: 'un:vault' }))?.text).toMatch(/^\[ \] /)
    expect((await ui.find({ key: 'un:logs' }))?.text).toMatch(/^\[ \] /)
    await ui.press({ key: 'un:logs' })
    expect(await ui.find({ type: 'Code' })).toMatchObject({ props: { source: 'quayflow uninstall --delete-logs' } })
    await ui.unmount()
  })

  test('the managed-login exception, when on, never claims a switch to a refused account', async ($, on) => {
    const state: StateView = {
      ...STATE,
      dOrg: { enabled: true, confirmedOrg: '0f0f0f0f0f0f', confirmedAt: 1_799_999_000_000 },
      envs: [{ ...ENV, managed: { forceOrg: ['0f0f0f0f0f0f'], sources: ['registry:HKLM'] } }],
    }
    const w = world(on, { state })
    await start($, w, true)
    await quay($, '')
    const ui = await $.ui.mount({ ...PANE, surface: 'terminal' })
    await ui.press({ key: 'hints' })
    await ui.press({ key: 'tab:settings' })
    expect(await ui.find({ type: 'Text', text: /this build still enforces your organization's login policy/ })).toBeDefined()
    expect(await ui.find({ type: 'Text', text: /may switch to accounts/ })).toBeUndefined()
    await ui.press({ key: 'se-dorg' })
    expect(await ui.find({ type: 'Text', text: /^Managed-login enforcement\s+Still enforced in this build$/ })).toBeDefined()
    expect(await quay($, 'status')).toContain('this build still enforces')
    await ui.unmount()
  })

  test('creating a lane with an account stores that login in the new lane', async ($, on) => {
    const w = world(on, {
      routes: {
        '/v1/lanes/create': { name: 'studio', configDir: 'C:\\q\\lanes\\studio', live: null, linksOk: true, linkDetail: [], canaryPassed: true, sessions: 0 },
        '/v1/switch': { ok: true, transition: { id: 't', env: 'studio', to: 'b2b2b2b2b2b2', how: 'manual', reason: 'manual', state: 'verifying_adoption', requestedAt: 1 } },
      },
    })
    await start($, w, true)
    await quay($, '')
    const ui = await $.ui.mount({ ...PANE, surface: 'terminal' })
    await ui.press({ key: 'hints' })
    await ui.press({ key: 'tab:groups' })
    await ui.press({ key: 'gr-lanes' })
    await ui.press({ key: 'ln-create' })
    expect(await ui.find({ type: 'Text', text: /^Organization policy files$/ })).toBeDefined()
    await ui.input({ key: 'newlane:name', text: 'studio' })
    await ui.select({ key: 'newlane:acct', value: 'b2b2b2b2b2b2' })
    await ui.press({ key: 'nl-create' })
    expect(w.bodies('/v1/lanes/create')).toEqual([{ name: 'studio' }])
    expect(w.bodies('/v1/switch')[0]).toMatchObject({ to: 'b2b2b2b2b2b2', env: 'studio' })
    await ui.unmount()
  })
})

describe('/quay text', () => {
  test('a mistyped option is refused and named, never ignored', async ($, on) => {
    const w = world(on)
    await start($, w, false)
    expect(await quay($, 'mode automatic --aprove')).toBe('Quayflow: unknown option --aprove (did you mean --approve?).')
    expect(w.routes().filter(r => r.includes('mode'))).toEqual([])
  })

  test('/quay report previews without saving; --save saves', async ($, on) => {
    const w = world(on, { routes: { '/v1/report/preview': { id: 'r1', text: 'REPORT-TEXT' }, '/v1/report': { path: 'C:\\r.json', preview: 'P' } } })
    await start($, w, false)
    const out = await quay($, 'report')
    expect(out).toContain('not saved')
    expect(out).toContain('REPORT-TEXT')
    expect(w.routes()).not.toContain('/v1/report')
    expect(await quay($, 'report --save')).toContain('C:\\r.json')
  })

  test('/quay resume says when automation stays paused for another cause', async ($, on) => {
    const paused: StateView = { ...STATE, envs: [{ ...ENV, automationPaused: true, pausedBy: 'other_switcher', pausedReason: 'the login changed repeatedly without Quayflow' }] }
    const w = world(on, { state: paused })
    await start($, w, false)
    expect(await quay($, 'resume')).toContain('still paused: the login changed repeatedly')
  })
})
