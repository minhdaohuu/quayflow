// Quayflow Mod type contract.
//
// 1. The loopback API v1 schema, mirrored from crates/qf-model (serde camelCase).
//    Keep in step with crates/qf-model/src/{lib,config,api}.rs.
// 2. The values the Mod keeps in `$.state` (PluginState['quayflow']).
//
// Self-contained on purpose: a plugin contract has no import or reference.

// ---------------------------------------------------------------- qf-model

export type AccountId = string
export type EnvId = string
export type AccountKind = 'organization' | 'personal'
export type Health = 'ok' | 'stale_access' | 'refresh_rejected' | 'vault_error'
export type Rank = 'preferred' | 'normal' | 'backup'
export type Mode = 'manual' | 'suggest' | 'automatic'
export type SwitchHow = 'manual' | 'accepted' | 'automatic' | 'return_home' | 'recovery' | 'model_fallback' | 'canary'
export type TransitionState =
  | 'requested'
  | 'preparing'
  | 'idle_wait'
  | 'applying'
  | 'verifying_adoption'
  | 'completed'
  | 'cancelled'
  | 'failed'
  | 'uncertain'
export type AdoptionState = 'pending_next_request' | 'adopted' | 'mismatch'
/** Why a transition ended without applying, or after the write without certainty. */
export type TransitionFailure = 'lock_held' | 'vault' | 'pre_write' | 'post_write' | 'cancelled'

export type Window = { key: string; pct: number; resetsAt?: number | null }

export type SessionInfo = {
  pid: number
  sessionId: string
  env: EnvId
  name?: string | null
  kind?: string | null
  status?: string | null
  cwd?: string | null
  version?: string | null
  group?: string | null
}

export type Transition = {
  id: string
  env: EnvId
  from?: AccountId | null
  to: AccountId
  how: SwitchHow
  reason: string
  state: TransitionState
  requestedAt: number
  writtenAt?: number | null
  endedAt?: number | null
  lockedMs?: number | null
  detail?: string | null
  failure?: TransitionFailure | null
}

export type ClientAdoption = {
  sessionId: string
  pid: number
  transition: string
  destination: AccountId
  lastRequestAccount?: AccountId | null
  state: AdoptionState
  version?: string | null
  at?: number | null
}

export type Pin = { account: AccountId; until: number; at: number }

export type Suggestion = {
  id: string
  ts: number
  env: EnvId
  from?: AccountId | null
  to: AccountId
  how: SwitchHow
  reason: string
}

export type CompatRecord = {
  version: string
  checkedAt: number
  staticPass: boolean
  staticDetail?: string[]
  livePass?: boolean | null
  liveDetail?: string | null
}

export type ManagedLogin = {
  forceOrg?: string[]
  forceMethod?: string | null
  sources?: string[]
}

export type MixedApproval = { accounts: AccountId[]; at: number }

export type Group = {
  name: string
  accounts: AccountId[]
  preferred?: AccountId | null
  ranks?: Record<AccountId, Rank>
  mode: Mode
  automaticApprovedAt?: number | null
  threshold: number
  margin: number
  mixedApproval?: MixedApproval | null
}

export type ProjectAssignment = { path: string; group: string }

export type Tuning = {
  cooldownS: number
  pinMinutes: number
  pinReleasePct: number
  hardLimitPct: number
  idleWaitS: number
  lockWaitS: number
  livePollS: number
  idlePollS: number
  liveFreshS: number
  idleFreshS: number
  backoffMaxS: number
  holdMaxS: number
  logRetentionDays: number
  logMaxMb: number
  concurrentWindowS: number
}

export type Consents = {
  engineStart?: number | null
  signinStartup?: number | null
  credentialStorage?: number | null
  terms?: number | null
  promptHook?: number | null
  liveCanary?: number | null
}

export type DOrgSetting = { enabled: boolean; confirmedOrg?: string | null; confirmedAt?: number | null }

// ---------------------------------------------------------------- api.rs

export type HelloRequest = { protocol: number; modVersion?: string; sessionId?: string; headless: boolean; claudeVersion?: string }
export type HelloResponse = {
  compatible: boolean
  protocol: number
  acceptedProtocols: number[]
  engineVersion: string
  message?: string | null
}

export type Notice = { id: string; level: 'info' | 'warn' | 'error' | string; kind: string; text: string; persistent?: boolean }

export type UsageView = { binding?: Window | null; windows: Window[]; ageS: number; fresh: boolean; source: string }
export type Eligibility = { eligible: boolean; reason: string }

export type AccountView = {
  id: AccountId
  label: string
  kind: AccountKind
  subscription?: string | null
  health: Health
  groups: string[]
  liveIn: EnvId[]
  usage?: UsageView | null
  usageNote?: string | null
  eligibility?: Eligibility | null
  unavailableModels?: string[]
  rank?: Rank | null
  /** Opaque organization hash, and its local display name. */
  org: string
  orgName?: string | null
}

export type GroupView = Group & { isDefault: boolean; mixed: boolean; mixedCovered: boolean }

export type LiveLogin = {
  account?: AccountId | null
  known: boolean
  label?: string | null
  signedOut?: boolean
  /** Filled for an unknown live login too. */
  kind?: AccountKind | null
  subscription?: string | null
}
export type InPlace = { allowed: boolean; reason: string; governingVersion?: string | null }

export type EnvView = {
  id: EnvId
  configDir: string
  live: LiveLogin
  mode: Mode
  governingGroups: string[]
  inPlace: InPlace
  managed: ManagedLogin
  /** Display names of the enforced organizations, where a registered account belongs to them. */
  managedOrgNames?: string[]
  transition?: Transition | null
  lastSwitch?: Transition | null
  pin?: Pin | null
  suggestion?: Suggestion | null
  adoption: ClientAdoption[]
  sessions: SessionInfo[]
  decision: string
  automationPaused: boolean
  pausedReason?: string | null
  notices: Notice[]
}

export type SessionView = {
  sessionId: string
  env: EnvId
  group: string
  /** Whether `group` comes from an explicit project assignment or the default group. */
  groupSource: 'assigned' | 'default' | string
  verifiedAccount?: AccountId | null
  verifiedAt?: number | null
  adoption?: ClientAdoption | null
}

export type EngineInfo = {
  version: string
  protocol: number
  pid: number
  startedAt: number
  dataDir: string
  integrity: string
  exeSha256: string
}

export type ActivityItem = { ts: number; kind: string; text: string }

export type LaneView = {
  name: string
  configDir: string
  live?: AccountId | null
  linksOk: boolean
  linkDetail: string[]
  canaryPassed: boolean
  sessions: number
}

export type StateView = {
  engine: EngineInfo
  onboarded: boolean
  consents: Consents
  session?: SessionView | null
  envs: EnvView[]
  accounts: AccountView[]
  groups: GroupView[]
  defaultGroup: string
  projects: ProjectAssignment[]
  tuning: Tuning
  compat: CompatRecord[]
  dOrg: DOrgSetting
  lanes: LaneView[]
  activity: ActivityItem[]
  seq: number
}

export type EngineEvent = { seq: number; ts: number; kind: string; env?: EnvId | null; text?: string | null }
export type EventsResponse = { seq: number; events: EngineEvent[] }

export type SwitchRequest = { to: string; env?: EnvId; sessionId?: string; idleWait?: boolean; requestId?: string }
export type SwitchResponse = {
  ok: boolean
  transition?: Transition | null
  message?: string | null
  blockedBy?: SessionInfo | null
}

/** Rate-limit entry as `$.session.usage().rateLimits` reports it. */
export type SessionRateLimitWire = { kind?: string; percentUsed?: number; resetsAt?: string | number | null }

export type StepReport = {
  sessionId: string
  turnId?: string
  index?: number
  agentId?: string | null
  ok: boolean
  stopReason?: string | null
  model?: string
  rateLimits: SessionRateLimitWire[]
  /** When this request started (ms since the epoch); requests in flight at a switch are not adoption evidence. */
  startedAt?: number | null
}
export type StepResponse = { verify: boolean; transition?: string | null }

export type AdoptionReport = { sessionId: string; account?: AccountId | null; transition?: string | null; startedAt?: number | null }

export type RecoverRequest = {
  sessionId: string
  turnId?: string
  index?: number
  agentId?: string | null
  model?: string
  simulated: boolean
  rateLimits: SessionRateLimitWire[]
  errorKind?: string
  requestId?: string
}
export type RecoverResponse = {
  retry: boolean
  switched: boolean
  to?: AccountId | null
  toLabel?: string | null
  why: string
  transition?: string | null
}

export type FailureShape = {
  sessionId: string
  model?: string
  stopReason?: string | null
  answerEmpty: boolean
  toolUses: number
  hasUsage: boolean
  errorKind?: string
  fields: string[]
  rateLimits: SessionRateLimitWire[]
  simulated: boolean
}

export type OkResponse = { ok: boolean; message?: string | null }
/** POST /v1/transition/cancel: `started` when the write had already begun. */
export type CancelResponse = { ok: boolean; message?: string | null; started: boolean }
export type ErrorBody = { error: string; code: string }

export type SigninIdentity = {
  account: AccountId
  kind: AccountKind
  subscription?: string | null
  duplicateOf?: string | null
  unexpected?: boolean
  display?: string | null
}
export type SigninStatus = {
  id: string
  state: 'waiting' | 'ready' | 'failed' | 'cancelled' | 'saved' | string
  configDir: string
  command: string
  identity?: SigninIdentity | null
  message?: string | null
}

export type GroupChangeResponse = { ok: boolean; needsDisclosure?: boolean; disclosure?: string | null; message?: string | null }
export type HandoffResponse = { ok: boolean; fileCount: number; command: string; launched?: boolean; message?: string | null }
export type ReportResponse = { path: string; preview: string }
export type PolicyEntry = { env: EnvId; managed: ManagedLogin }

/** GET /v1/canary while the manual live canary records (null otherwise). */
export type CanaryStatus = { since: number; steps: unknown[] }

/** `{"state": ...}` line printed by `quayflow start [--if-consented]`. */
export type StartState = 'running' | 'started' | 'failed' | 'no_consent' | 'stopped_by_user'

// ---------------------------------------------------------------- Mod state

/** Why Quayflow is off for this session (everything it does is off then). */
export type QuayOffReason =
  | 'connecting'
  | 'unreachable'
  | 'not_installed'
  | 'no_consent'
  | 'stopped_by_user'
  | 'start_failed'
  | 'incompatible'

/** The Mod's link to the engine, as the pane and status line read it. */
export type QuayLink = {
  on: boolean
  reason?: QuayOffReason
  /** Engine's own words (incompatibility message, start failure). */
  message?: string
  engineVersion?: string
}

/** Pane navigation. `tab` is a top tab or a flow; `sub` and `arg` pick a view inside it. */
export type QuayView = { tab: string; sub?: string; arg?: string }

/** Text the person typed or picked in the pane, kept until submitted. */
export type QuayForm = Record<string, string>

/** One line of feedback from the last pane action. `tone` drives the label, never color alone. */
export type QuayFlash = { tone: 'info' | 'warn' | 'error'; text: string; at: number }

/** A native sign-in the pane is following. */
export type QuaySignin = { status: SigninStatus; label: string; group: string; reauth?: string }

/**
 * A pending mixed-identity disclosure waiting for an explicit answer, and the
 * change to resend with `acceptDisclosure: true` once accepted.
 */
export type QuayDisclosure = {
  group: string
  text: string
  /** `members`: add `add` to the group; `register`: register the live login as `label`; `confirm`: save sign-in `signinId` as `label`. */
  then: 'members' | 'register' | 'confirm'
  add?: string[]
  label?: string
  signinId?: string
}

/** A support report built for review. */
export type QuayReport = ReportResponse

declare module 'claude-code' {
  interface PluginState {
    quayflow: {
      link: QuayLink
      snapshot: Shaped<StateView | null>
      view: QuayView
      form: QuayForm
      flash: QuayFlash | null
      signin: QuaySignin | null
      disclosure: QuayDisclosure | null
      report: QuayReport | null
      /** Ids of suggestions and notices already announced in this session (no repeats). */
      announced: string[]
    }
  }
}
