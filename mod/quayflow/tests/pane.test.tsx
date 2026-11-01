// The /quay pane (early check C2): tab buttons, Input and Select, driven by
// key on the terminal and desktop surfaces, against a mocked engine.
import { describe, expect, test } from 'claude-code/testing'

import type { StateView } from '../types'
import { STATE } from './fixtures'
import { ENGINE_EXE, quay, refuse, start, world } from './harness'

const PANE = {
  plugin: 'quayflow',
  component: 'Pane',
  requestId: 'quay',
  props: {
    title: 'Quayflow',
    isFocused: true,
    bodyColumns: 100,
    placement: 'dock',
    scroll: { offset: 0, bodyRows: 40 },
    view: {},
  },
} as const

const MIXED: StateView = {
  ...STATE,
  accounts: [...STATE.accounts, { ...STATE.accounts[1]!, id: 'c3c3c3c3c3c3', label: 'client', kind: 'organization', groups: [], liveIn: [] }],
}

describe('navigation and switching', () => {
  test('tabs, account detail and the switch flow, on terminal and desktop', async ($, on) => {
    const w = world(on, {
      routes: {
        '/v1/switch': {
          ok: true,
          transition: { id: 'tr-1', env: 'default', from: 'a1a1a1a1a1a1', to: 'b2b2b2b2b2b2', how: 'manual', reason: 'manual', state: 'verifying_adoption', requestedAt: 1 },
        },
      },
    })
    await start($, w, true)
    for (const surface of ['terminal', 'desktop'] as const) {
      await quay($, '')
      const ui = await $.ui.mount({ ...PANE, surface })
      expect(await ui.find({ type: 'Text', text: /Environment default stores: "work"/ })).toBeDefined()
      expect(await ui.find({ type: 'Text', text: /may not be listed/ })).toBeDefined()

      await ui.press({ key: 'tab:accounts' })
      await ui.press({ key: 'acct:b2b2b2b2b2b2' })
      expect(await ui.find({ type: 'Text', text: /access token stale/ })).toBeDefined()
      expect(await ui.find({ type: 'Input', key: 'rename:b2b2b2b2b2b2' })).toBeDefined()

      await ui.press({ key: 'ad-switch' })
      expect(await ui.find({ type: 'Text', text: /Every session of default moves at its next request/ })).toBeDefined()
      await ui.press({ key: 'sw-go' })
      expect(w.bodies('/v1/switch').pop()).toMatchObject({ to: 'b2b2b2b2b2b2', env: 'default', sessionId: 'S1', idleWait: false })
      await ui.unmount()
    }
  })

  test('a group conflict names the blocking session and offers a lane', async ($, on) => {
    const w = world(on, {
      routes: {
        '/v1/accept': { ok: false, message: 'group conflict', blockedBy: { pid: 101, sessionId: 'S2', env: 'default', name: 'docs', cwd: 'C:\\work\\docs', group: 'Client' } },
      },
    })
    await start($, w, true)
    await quay($, '')
    const ui = await $.ui.mount({ ...PANE, surface: 'terminal' })
    await ui.press({ key: 'ov-accept' })
    expect(await ui.find({ type: 'Text', text: /Session "docs" \(project C:\\work\\docs, group Client\) does not permit "personal"/ })).toBeDefined()
    expect(await ui.find({ key: 'cf-lanes' })).toBeDefined()
    await ui.unmount()
  })
})

describe('switch progress', () => {
  const transition = { id: 'tr-9', env: 'default', from: 'a1a1a1a1a1a1', to: 'b2b2b2b2b2b2', how: 'manual' as const, reason: 'manual', requestedAt: 1 }

  test('a held lock is reported from the failure code; the login is unchanged', async ($, on) => {
    const failedSwitch = { ...transition, state: 'failed' as const, failure: 'lock_held' as const, detail: 'refresh lock held 9 s' }
    const state: StateView = { ...STATE, envs: [{ ...STATE.envs[0]!, lastSwitch: failedSwitch }] }
    const w = world(on, { state, routes: { '/v1/switch': { ok: true, transition: { ...transition, state: 'preparing' } } } })
    await start($, w, true)
    await quay($, '')
    const ui = await $.ui.mount({ ...PANE, surface: 'terminal' })
    await ui.press({ key: 'tab:accounts' })
    await ui.press({ key: 'acct:b2b2b2b2b2b2' })
    await ui.press({ key: 'ad-switch' })
    await ui.press({ key: 'sw-go' })
    // The engine's state now says the switch failed on a held lock.
    expect(await ui.find({ type: 'Text', text: /a Claude credential lock is still held; the switch did not apply and the login is unchanged/ })).toBeDefined()
    expect(await ui.find({ key: 'sp-cancel' })).toBeUndefined()
    await ui.unmount()
  })

  test('cancel after the write says "Transition already started; checking state"', async ($, on) => {
    const state: StateView = { ...STATE, envs: [{ ...STATE.envs[0]!, transition: { ...transition, state: 'preparing' } }] }
    const w = world(on, { state, routes: { '/v1/transition/cancel': { ok: true, message: 'Transition already started; checking state', started: true } } })
    await start($, w, true)
    await quay($, '')
    const ui = await $.ui.mount({ ...PANE, surface: 'terminal' })
    await ui.press({ key: 'ov-progress' })
    expect(await ui.find({ type: 'Text', text: /Switch to "personal": preparing/ })).toBeDefined()
    await ui.press({ key: 'sp-cancel' })
    expect(w.bodies('/v1/transition/cancel')).toEqual([{ id: 'tr-9' }])
    expect(await ui.find({ type: 'Text', text: /Transition already started; checking state\./ })).toBeDefined()
    await ui.unmount()
  })
})

describe('groups', () => {
  test('Automatic needs its explicit approval; an Input creates a group', async ($, on) => {
    const w = world(on)
    await start($, w, true)
    await quay($, '')
    const ui = await $.ui.mount({ ...PANE, surface: 'terminal' })
    await ui.press({ key: 'tab:groups' })
    await ui.press({ key: 'grp:Default' })
    await ui.select({ key: 'mode:Default', value: 'automatic' })
    expect(w.routes()).not.toContain('/v1/groups/mode')
    expect(await ui.find({ type: 'Text', text: /Approve Automatic for group Default\?/ })).toBeDefined()
    await ui.press({ key: 'au-approve' })
    expect(w.bodies('/v1/groups/mode')).toEqual([{ group: 'Default', mode: 'automatic', approveAutomatic: true }])

    await ui.press({ key: 'tab:groups' })
    await ui.input({ key: 'new-group', text: 'Team' })
    expect(w.bodies('/v1/groups/create')).toEqual([{ name: 'Team' }])
    await ui.unmount()
  })

  test('mixed identities: membership is resent with acceptDisclosure only after approval', async ($, on) => {
    const w = world(on, {
      state: MIXED,
      routes: {
        '/v1/groups/members': (b: Record<string, unknown>) =>
          b.acceptDisclosure ? { ok: true } : { ok: false, needsDisclosure: true, disclosure: 'Group "Default" would mix two organizations and a personal account.' },
      },
    })
    await start($, w, true)
    await quay($, '')
    const ui = await $.ui.mount({ ...PANE, surface: 'terminal' })
    await ui.press({ key: 'tab:groups' })
    await ui.press({ key: 'grp:Default' })
    await ui.select({ key: 'add:Default', value: 'c3c3c3c3c3c3' })
    expect(await ui.find({ type: 'Text', text: /would mix two organizations/ })).toBeDefined()
    expect(w.bodies('/v1/groups/members').map(b => b.acceptDisclosure)).toEqual([false])
    await ui.press({ key: 'dc-approve' })
    expect(w.bodies('/v1/groups/members').map(b => b.acceptDisclosure)).toEqual([false, true])
    await ui.unmount()
  })
})

describe('first use', () => {
  const FRESH: StateView = { ...STATE, onboarded: false, consents: {} }

  test('without the required consents, Continue does nothing and the engine stays off', async ($, on) => {
    const w = world(on, { down: true, start: 'no_consent', state: FRESH })
    await start($, w, true)
    expect(w.runs).toEqual([[ENGINE_EXE, 'start', '--if-consented']])
    expect(w.ui.filter(u => u.kind === 'ui.status').pop()?.text).toContain('not set up')
    await quay($, '')
    const ui = await $.ui.mount({ ...PANE, surface: 'terminal' })
    expect(await ui.find({ type: 'Text', text: /Unattended headless jobs \(claude -p\) only switch or recover in Automatic mode/ })).toBeDefined()
    await ui.press({ key: 'welcome-continue' })
    expect(w.runs).toHaveLength(1)
    expect(w.routes()).not.toContain('/v1/onboarding')
    await ui.unmount()
  })

  test('Continue records each consent separately', async ($, on) => {
    const w = world(on, { down: true, start: 'started', state: FRESH })
    await start($, w, true)
    await quay($, '')
    const ui = await $.ui.mount({ ...PANE, surface: 'terminal' })
    await ui.press({ key: 'c:engineStart' })
    await ui.press({ key: 'c:terms' })
    await ui.press({ key: 'welcome-continue' })
    expect(w.bodies('/v1/onboarding')).toEqual([{ consents: { engineStart: true, signinStartup: false, credentialStorage: false, terms: true }, complete: true }])
    await ui.unmount()
  })

  test('registering into a mixed group shows the disclosure, then resends with acceptDisclosure', async ($, on) => {
    const unknownLive: StateView = { ...STATE, envs: [{ ...STATE.envs[0]!, live: { account: 'c3c3c3c3c3c3', known: false } }] }
    const w = world(on, {
      state: unknownLive,
      routes: {
        '/v1/accounts/register': (b: Record<string, unknown>) =>
          b.acceptDisclosure ? { id: 'c3c3c3c3c3c3', label: 'client', kind: 'organization' } : refuse('needs_disclosure', 'Group "Default" would mix two organizations.'),
      },
    })
    await start($, w, true)
    await quay($, 'register')
    const ui = await $.ui.mount({ ...PANE, surface: 'terminal' })
    await ui.input({ key: 'reg:label', text: 'client' })
    await ui.press({ key: 'reg-save' })
    expect(await ui.find({ type: 'Text', text: /would mix two organizations/ })).toBeDefined()
    expect(w.bodies('/v1/accounts/register').map(b => b.acceptDisclosure)).toEqual([false])
    await ui.press({ key: 'dc-approve' })
    expect(w.bodies('/v1/accounts/register')[1]).toMatchObject({ label: 'client', group: 'Default', acceptDisclosure: true })
    await ui.unmount()
  })
})

describe('trust and settings', () => {
  test('the managed-login exception names the organization and needs the typed acknowledgment', async ($, on) => {
    const state: StateView = {
      ...STATE,
      envs: [{ ...STATE.envs[0]!, managed: { forceOrg: ['0f0f0f0f0f0f'], sources: ['registry:HKLM'] }, managedOrgNames: ['Acme Corp'] }],
    }
    const w = world(on, { state })
    await start($, w, true)
    await quay($, '')
    const ui = await $.ui.mount({ ...PANE, surface: 'terminal' })
    await ui.press({ key: 'tab:settings' })
    await ui.press({ key: 'se-dorg' })
    expect(await ui.find({ type: 'Text', text: /Acme Corp \(#0f0f0f0f0f0f\) enforces/ })).toBeDefined()
    expect(await ui.find({ type: 'Text', text: /can violate your employment or organization policy/ })).toBeDefined()
    await ui.input({ key: 'dorg:ack', text: 'yes' })
    await ui.press({ key: 'dorg-enable' })
    expect(w.routes()).not.toContain('/v1/policy/dorg')
    await ui.input({ key: 'dorg:ack', text: 'I accept the risk for Acme Corp' })
    await ui.press({ key: 'dorg-enable' })
    // The engine records the organization by its hash; the name is for the person.
    expect(w.bodies('/v1/policy/dorg')).toEqual([{ enable: true, confirmOrg: '0f0f0f0f0f0f', acknowledge: 'I accept the risk for Acme Corp' }])
    await ui.unmount()
  })

  test('settings show the integrity line, version and hash from the engine', async ($, on) => {
    const w = world(on)
    await start($, w, true)
    await quay($, '')
    const ui = await $.ui.mount({ ...PANE, surface: 'terminal' })
    await ui.press({ key: 'tab:settings' })
    expect(await ui.find({ type: 'Text', text: /Integrity: not code-signed; provenance-attested · version 0\.1\.0 · SHA-256 ab12cd34/ })).toBeDefined()
    expect(await ui.find({ type: 'Text', text: /quayflow update/ })).toBeDefined()
    await ui.unmount()
  })
})
