// The identity hash of the account this session's requests use, read through
// the client's own authentication path: an opaque handle from
// `$.session.authorize()` spent on the first-party profile endpoint. The token
// never reaches the Mod; only the 12-hex-character hash leaves this file.
import type { EngineInterface } from 'claude-code'

import { withTimeout } from './engine'

const PROFILE_URL = 'https://api.anthropic.com/api/oauth/profile'

/** First 12 hex characters of SHA-256, as `qf_model::identity_hash`. */
export async function identityHash(value: string): Promise<string> {
  const bytes = await crypto.subtle.digest('SHA-256', new TextEncoder().encode(value))
  return Array.from(new Uint8Array(bytes).slice(0, 6), b => b.toString(16).padStart(2, '0')).join('')
}

/** Hash of the account the session's credential belongs to now; undefined when unknown. */
export async function liveAccountHash($: EngineInterface, timeoutMs = 8_000): Promise<string | undefined> {
  const auth = await $.session.authorize()
  if (!auth || auth.kind !== 'bearer') return undefined
  const res = await withTimeout(
    $,
    $.http.fetch(PROFILE_URL, { headers: { 'anthropic-beta': 'oauth-2025-04-20' }, auth: auth.handle }),
    timeoutMs,
    'profile lookup',
  )
  if (!res.ok) return undefined
  let uuid = ''
  try {
    uuid = String((JSON.parse(res.text) as { account?: { uuid?: string } })?.account?.uuid ?? '')
  } catch {
    return undefined
  }
  return uuid ? identityHash(uuid) : undefined
}
