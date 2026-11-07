// The engine operations the /quay command and the pane share, mapped to API v1
// (tech-spec §6). Each refreshes the snapshot after a change. Result wording
// follows UI copy rules: a stored destination is not adoption, a saved account is
// not an activation.
import type { EngineInterface } from 'claude-code'

import type {
  AccountView,
  CancelResponse,
  CanaryStatus,
  CompatRecord,
  ExplainView,
  GroupChangeResponse,
  HandoffCheck,
  HandoffResponse,
  IdentityView,
  LaneView,
  Mode,
  OkResponse,
  OperationView,
  Pin,
  ReportPreview,
  ReportResponse,
  SigninStatus,
  StateView,
  SwitchResponse,
} from '../types'
import { Q, requestId } from './context'
import { call, dataDir } from './engine'
import { label, myEnv, sessionName, transitionWords } from './format'
import { refresh } from './watcher'

function post<T>($: EngineInterface, route: string, body: unknown, timeoutMs?: number): Promise<T> {
  return call<T>($, 'POST', route, body, { timeoutMs })
}

async function after<T>($: EngineInterface, p: Promise<T>): Promise<T> {
  const out = await p
  await refresh($).catch(() => undefined)
  return out
}

export function envId(): string | undefined {
  return Q.state ? myEnv(Q.state)?.id : undefined
}

/** Plain-language outcome of a switch request. */
export function switchOutcome(s: StateView | undefined, r: SwitchResponse, pinned: boolean): string {
  if (!r.ok) {
    if (r.blockedBy) {
      const b = r.blockedBy
      return `Not switched: session "${sessionName(b)}" (group ${b.group ?? '?'}) does not permit that account, and a shared switch moves every session. Move that session to a lane (/quay lanes, then /quay replace <lane> in that session), or keep the shared login.`
    }
    return `Not switched: ${r.message ?? 'refused by the engine'}.`
  }
  const t = r.transition
  if (!t) return `Quayflow: ${r.message ?? 'nothing to do'}.`
  const pin = pinned ? ' Your choice is pinned: no suggestion or automatic switch away from it until you unpin (/quay unpin), its window resets, or the pin period ends.' : ''
  return `Switch to "${label(s, t.to)}" for environment ${t.env}: ${transitionWords(t.state)}. Every session of this environment moves at its next request; requests in progress finish on the current account.${pin}`
}


export function actSwitchTo($: EngineInterface, to: string, idleWait = false, env?: string) {
  return after($, post<SwitchResponse>($, '/v1/switch', { to, env: env ?? envId(), sessionId: Q.sessionId, idleWait, requestId: requestId() }, 30_000))
}

export function actAccept($: EngineInterface) {
  return after($, post<SwitchResponse>($, '/v1/accept', { sessionId: Q.sessionId, env: envId(), requestId: requestId() }, 30_000))
}

export function actCancelTransition($: EngineInterface, id: string) {
  return after($, post<CancelResponse>($, '/v1/transition/cancel', { id }))
}

export function actUnpin($: EngineInterface) {
  return after($, post<OkResponse>($, '/v1/unpin', { env: envId() }))
}

export function actSetMode($: EngineInterface, group: string, mode: Mode, approveAutomatic: boolean) {
  return after($, post<OkResponse>($, '/v1/groups/mode', { group, mode, approveAutomatic }))
}

export function actMembers($: EngineInterface, group: string, add: string[], remove: string[], acceptDisclosure: boolean) {
  return after($, post<GroupChangeResponse>($, '/v1/groups/members', { group, add, remove, acceptDisclosure }))
}

export function actGroupCreate($: EngineInterface, name: string) {
  return after($, post<OkResponse>($, '/v1/groups/create', { name }))
}

export function actGroupDelete($: EngineInterface, name: string) {
  return after($, post<OkResponse>($, '/v1/groups/delete', { name }))
}

export function actPreferred($: EngineInterface, group: string, account: string | null) {
  return after($, post<OkResponse>($, '/v1/groups/preferred', { group, account }))
}

export function actRank($: EngineInterface, group: string, account: string, rank: string) {
  return after($, post<OkResponse>($, '/v1/groups/rank', { group, account, rank }))
}

export function actSetDefault($: EngineInterface, group: string) {
  return after($, post<OkResponse>($, '/v1/groups/default', { group }))
}

export function actTuning($: EngineInterface, group: string, threshold?: number, margin?: number) {
  return after($, post<OkResponse>($, '/v1/groups/tuning', { group, threshold, margin }))
}

export function actAssign($: EngineInterface, path: string, group: string) {
  return after($, post<OkResponse>($, '/v1/projects/assign', { path, group }))
}

export function actUnassign($: EngineInterface, path: string) {
  return after($, post<OkResponse>($, '/v1/projects/unassign', { path }))
}

export function actEngine($: EngineInterface, action: 'pause' | 'resume' | 'stop') {
  return after($, post<OkResponse>($, '/v1/engine', { action }))
}

export function actOnboarding($: EngineInterface, consents: Record<string, boolean>, complete: boolean) {
  return after($, post<OkResponse>($, '/v1/onboarding', { consents, complete }))
}

/** Registers the live login. HTTP 409 `needs_disclosure` (its text in the error) when the group would mix identities. */
export function actRegister($: EngineInterface, labelText: string, group: string, acceptDisclosure = false) {
  return after($, post<AccountView & { note?: string }>($, '/v1/accounts/register', { label: labelText, group, env: envId(), acceptDisclosure }))
}

export function actSigninStart($: EngineInterface, reauth?: string) {
  return post<SigninStatus>($, '/v1/accounts/signin/start', reauth ? { reauth } : {}, 30_000)
}

export function actSigninStatus($: EngineInterface, id: string) {
  return call<SigninStatus>($, 'GET', `/v1/accounts/signin/status?id=${encodeURIComponent(id)}`)
}

/** Saves a completed sign-in. HTTP 409 `needs_disclosure` as for register. */
export function actSigninConfirm($: EngineInterface, id: string, labelText: string, group: string, acceptDisclosure = false, asNew = false) {
  return after($, post<OkResponse>($, '/v1/accounts/signin/confirm', { id, label: labelText, group, acceptDisclosure, ...(asNew ? { asNew: true } : {}) }))
}

export function actSigninCancel($: EngineInterface, id: string) {
  return post<OkResponse>($, '/v1/accounts/signin/cancel', { id })
}

export function actRemove($: EngineInterface, account: string) {
  return after($, post<OkResponse>($, '/v1/accounts/remove', { account }))
}

export function actRelabel($: EngineInterface, account: string, labelText: string) {
  return after($, post<OkResponse>($, '/v1/accounts/label', { account, label: labelText }))
}

export function actIgnoreUnknown($: EngineInterface) {
  return after($, post<OkResponse>($, '/v1/unknown', { env: envId(), action: 'ignore' }))
}

export function actLanes($: EngineInterface) {
  return call<LaneView[]>($, 'GET', '/v1/lanes')
}

export function actLaneCreate($: EngineInterface, name: string) {
  return after($, post<LaneView>($, '/v1/lanes/create', { name }, 60_000))
}

export function actLaneDelete($: EngineInterface, name: string) {
  return after($, post<OkResponse>($, '/v1/lanes/delete', { name }))
}

export function actLaneCheck($: EngineInterface, name: string) {
  return after($, post<OkResponse>($, '/v1/lanes/check', { name }, 60_000))
}

export function actHandoff($: EngineInterface, how: 'open' | 'replace' | 'prepare', toEnv: string) {
  return post<HandoffResponse>($, `/v1/handoff/${how}`, { sessionId: Q.sessionId, toEnv }, 120_000)
}

export function actReport($: EngineInterface) {
  return post<ReportResponse>($, '/v1/report', {}, 60_000)
}

export function actDorg($: EngineInterface, enable: boolean, confirmOrg: string, acknowledge: string) {
  return after($, post<OkResponse>($, '/v1/policy/dorg', { enable, confirmOrg, acknowledge }))
}

export function actPoll($: EngineInterface) {
  return after($, post<OkResponse>($, '/v1/poll', {}, 60_000))
}

export function actMigratePoc($: EngineInterface, group: string, acceptDisclosure: boolean) {
  return after($, post<OkResponse>($, '/v1/migrate-poc', { group, acceptDisclosure }, 60_000))
}

export function actCanary($: EngineInterface) {
  return call<CanaryStatus | null>($, 'GET', '/v1/canary')
}

export function actCompatCheck($: EngineInterface) {
  return after($, post<CompatRecord[]>($, '/v1/compat/check', {}, 60_000))
}

export function handoffOutcome(how: 'open' | 'replace' | 'prepare', lane: string, r: HandoffResponse): string {
  if (!r.ok) return `Not moved: ${r.message ?? 'the engine refused'}. This session and its history are unchanged.`
  const files = `${r.fileCount} history file${r.fileCount === 1 ? '' : 's'} copied with integrity checks; the source is kept.`
  if (how === 'replace') {
    return `Replacement into lane "${lane}" prepared. ${files} Exit this session (/exit); the PowerShell prompt hook resumes it in this terminal under the lane. Without the hook, run: ${r.command}`
  }
  if (how === 'open') {
    return r.launched
      ? `Opened a new window resuming this session in lane "${lane}". ${files} Keep only one of the two working on the task.`
      : `Could not open a window. ${files} Run this to continue in lane "${lane}": ${r.command}`
  }
  return `${files} Run this to continue in lane "${lane}": ${r.command}`
}

// ---- reads and settings for the review screens

export function actExplain($: EngineInterface) {
  return call<ExplainView>($, 'GET', `/v1/explain?env=${encodeURIComponent(envId() ?? '')}&sessionId=${encodeURIComponent(Q.sessionId)}`)
}

/** The full identity of the live login (`env`) or of a registered account's protected copy. */
export function actIdentity($: EngineInterface, of: { env?: string; account?: string }) {
  const q = of.account ? `account=${encodeURIComponent(of.account)}` : `env=${encodeURIComponent(of.env ?? envId() ?? '')}`
  return call<IdentityView>($, 'GET', `/v1/identity?${q}`)
}

export function actHandoffCheck($: EngineInterface, toEnv: string) {
  return call<HandoffCheck>($, 'GET', `/v1/handoff/check?sessionId=${encodeURIComponent(Q.sessionId)}&toEnv=${encodeURIComponent(toEnv)}`)
}

export function actOperations($: EngineInterface, limit = 20) {
  return call<OperationView[]>($, 'GET', `/v1/operations?limit=${limit}`)
}

export function actReportPreview($: EngineInterface) {
  return post<ReportPreview>($, '/v1/report/preview', {}, 60_000)
}

export function actReportSave($: EngineInterface, id: string) {
  return post<ReportResponse>($, '/v1/report/save', { id }, 60_000)
}

/** Pins the live account of this environment without a switch. */
export function actPin($: EngineInterface) {
  return after($, post<Pin>($, '/v1/pin', { env: envId() }))
}

export function actSetTuning($: EngineInterface, t: { cooldownS?: number; pinMinutes?: number }) {
  return after($, post<OkResponse>($, '/v1/tuning', t))
}

/** Records one consent (true gives it, false withdraws it). */
export function actConsent($: EngineInterface, name: 'promptHook' | 'liveCanary', on: boolean) {
  return after($, post<OkResponse>($, '/v1/onboarding', { consents: { [name]: on }, complete: false }))
}

/**
 * Installs the PowerShell prompt hook (`quayflow shell install`) after its own opt-in.
 * It loads in PowerShell windows opened afterwards; nothing else in the shell changes.
 */
export async function actShellInstall($: EngineInterface): Promise<{ ok: boolean; text: string }> {
  const exe = `${await dataDir($)}\\bin\\quayflow.exe`
  const r = await $.process.run([exe, 'shell', 'install'], { timeoutMs: 30_000 })
  const text = `${r.stdout}\n${r.stderr}`.trim()
  await refresh($).catch(() => undefined)
  return { ok: r.exitCode === 0, text: text.slice(0, 400) }
}
