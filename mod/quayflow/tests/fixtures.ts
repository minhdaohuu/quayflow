// Test fixtures: an engine StateView and model step results.
import type { TurnStepInput, TurnStepResult } from 'claude-code'

import type { ExplainView, StateView } from '../types'

/** No response arrived: the shape FR-15 recovers from. */
export function failed(e: Pick<TurnStepInput, 'turnId' | 'index'>): TurnStepResult {
  return { turnId: e.turnId, index: e.index, answer: '', toolUses: [], stopReason: null, usage: null }
}

export function ok(e: Pick<TurnStepInput, 'turnId' | 'index'>): TurnStepResult {
  return { turnId: e.turnId, index: e.index, answer: 'done', toolUses: [], stopReason: 'end_turn', usage: null }
}

export const STATE: StateView = {
  engine: {
    version: '0.1.0',
    protocol: 1,
    pid: 9,
    startedAt: 1_799_999_000_000,
    dataDir: 'C:\\Users\\t\\AppData\\Local\\Quayflow',
    integrity: 'not code-signed; provenance-attested',
    exeSha256: 'ab12cd34ef56ab12cd34ef56ab12cd34ef56ab12cd34ef56ab12cd34ef56ab12',
  },
  onboarded: true,
  consents: { engineStart: 1, credentialStorage: 1, terms: 1 },
  session: { sessionId: 'S1', env: 'default', group: 'Default', groupSource: 'default', verifiedAccount: 'a1a1a1a1a1a1', verifiedAt: 1_799_999_900_000 },
  envs: [
    {
      id: 'default',
      configDir: 'C:\\Users\\t\\.claude',
      live: { account: 'a1a1a1a1a1a1', known: true, label: 'work' },
      mode: 'suggest',
      governingGroups: ['Default'],
      inPlace: { allowed: true, reason: 'Claude 2.1.316 passed the static check', governingVersion: '2.1.316' },
      managed: {},
      transition: null,
      lastSwitch: null,
      pin: null,
      suggestion: { id: 'sg-1', ts: 1_799_999_950_000, env: 'default', from: 'a1a1a1a1a1a1', to: 'b2b2b2b2b2b2', how: 'accepted', reason: 'work at 91% of 5 h; personal at 12%' },
      adoption: [],
      sessions: [
        { pid: 100, sessionId: 'S1', env: 'default', cwd: 'C:\\work\\app', status: 'busy', version: '2.1.316', group: 'Default' },
        { pid: 101, sessionId: 'S2', env: 'default', name: 'docs', status: 'idle', version: '2.1.316', group: 'Default' },
      ],
      decision: 'work is above the 85% threshold; personal is eligible',
      automationPaused: false,
      notices: [],
    },
  ],
  accounts: [
    {
      id: 'a1a1a1a1a1a1',
      label: 'work',
      kind: 'organization',
      org: '0f0f0f0f0f0f',
      orgName: 'Acme Corp',
      subscription: 'team',
      health: 'ok',
      groups: ['Default'],
      liveIn: ['default'],
      usage: { binding: { key: '5h', pct: 91, resetsAt: 1_800_003_600_000 }, windows: [{ key: '5h', pct: 91, resetsAt: 1_800_003_600_000 }, { key: '7d', pct: 40 }], ageS: 40, fresh: true, source: 'engine' },
      eligibility: { eligible: true, reason: 'live account' },
    },
    {
      id: 'b2b2b2b2b2b2',
      label: 'personal',
      kind: 'personal',
      org: '1e1e1e1e1e1e',
      subscription: 'max',
      health: 'stale_access',
      groups: ['Default'],
      liveIn: [],
      usage: { binding: { key: '7d', pct: 12 }, windows: [{ key: '5h', pct: 3 }, { key: '7d', pct: 12 }], ageS: 480, fresh: true, source: 'engine' },
      eligibility: { eligible: true, reason: 'lowest binding usage in the group' },
    },
  ],
  groups: [
    {
      name: 'Default',
      accounts: ['a1a1a1a1a1a1', 'b2b2b2b2b2b2'],
      preferred: 'a1a1a1a1a1a1',
      ranks: {},
      mode: 'suggest',
      threshold: 85,
      margin: 10,
      isDefault: true,
      mixed: true,
      mixedCovered: true,
    },
  ],
  defaultGroup: 'Default',
  projects: [],
  tuning: {
    cooldownS: 300,
    pinMinutes: 120,
    pinReleasePct: 98,
    hardLimitPct: 99,
    idleWaitS: 60,
    lockWaitS: 9,
    livePollS: 60,
    idlePollS: 600,
    liveFreshS: 300,
    idleFreshS: 3600,
    backoffMaxS: 900,
    holdMaxS: 900,
    logRetentionDays: 7,
    logMaxMb: 20,
    concurrentWindowS: 30,
  },
  compat: [{ version: '2.1.316', checkedAt: 1_799_990_000_000, staticPass: true, livePass: true }],
  dOrg: { enabled: false },
  lanes: [],
  activity: [{ ts: 1_799_999_950_000, kind: 'suggestion', text: 'Suggested personal (work at 91% of 5 h)' }],
  seq: 42,
}

/** GET /v1/explain for STATE: work live at 91%, personal eligible, home = work (live). */
export const EXPLAIN: ExplainView = {
  env: 'default',
  mode: 'suggest',
  governingGroups: ['Default'],
  threshold: 85,
  margin: 10,
  cooldownS: 300,
  cooldownLeftS: 0,
  hardLimitPct: 99,
  live: 'a1a1a1a1a1a1',
  home: 'a1a1a1a1a1a1',
  homeState: 'home',
  homeDetail: 'work is the home account and is live.',
  pinned: false,
  allExhausted: false,
  soonest: null,
  decision: 'work is above the 85% threshold; personal is eligible',
  candidates: [
    { account: 'a1a1a1a1a1a1', status: 'live', reason: 'live here: 91% 5h (40 s old)', pct: 91, window: '5h', ageS: 40, fresh: true, exhausted: false },
    { account: 'b2b2b2b2b2b2', status: 'eligible', reason: '12% 7d (8 min old)', pct: 12, window: '7d', ageS: 480, fresh: true, exhausted: false },
  ],
}