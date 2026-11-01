// Shared pieces of the /quay pane: the render context, navigation, form
// fields and the action runner. Every handler writes `$.state` (never a value
// captured at draw time) and reports its outcome in one flash line.
import type { Elements, EngineInterface, RenderSurface } from 'claude-code'

import type { QuayDisclosure, QuayFlash, QuayForm, QuayLink, QuayReport, QuaySignin, QuayView, StateView } from '../types'
import { clearFlash, clearFormKeys, errorText, flash, setFormValue, setView } from './context'

export type Els = Elements['terminal']

export type Ctx = {
  els: Els
  surface: RenderSurface
  /** Input and Select exist on this surface (every surface but mobile). */
  canType: boolean
  cols: number
  now: number
  link: QuayLink
  s: StateView | null
  view: QuayView
  form: QuayForm
  flash: QuayFlash | null
  signin: QuaySignin | null
  disclosure: QuayDisclosure | null
  report: QuayReport | null
  claudeVersion: string
}

export function go($: EngineInterface, view: QuayView): void {
  void clearFlash($)
  void setView($, view)
}

export function setForm($: EngineInterface, key: string, value: string): void {
  void setFormValue($, key, value)
}

export function clearForm($: EngineInterface, ...keys: string[]): Promise<void> {
  return clearFormKeys($, keys)
}

/**
 * Runs one operation from a press. Shows "Working: ..." first (an actual
 * bounded call, no fake progress), then the operation's own words, or the
 * engine's error. `handle` may take an error over (a refusal that opens a
 * review, such as `needs_disclosure`) by answering true. Returns the
 * operation's value, or undefined on error.
 */
export async function run<T>(
  $: EngineInterface,
  doing: string,
  op: () => Promise<T>,
  describe?: (v: T) => string | undefined,
  handle?: (err: unknown) => Promise<boolean>,
): Promise<T | undefined> {
  await flash($, 'info', `Working: ${doing}`)
  try {
    const v = await op()
    const text = describe?.(v)
    if (text) await flash($, 'info', text)
    else await clearFlash($)
    return v
  } catch (err) {
    if (handle && (await handle(err).catch(() => false))) return undefined
    await flash($, 'error', errorText(err))
    return undefined
  }
}

/** A titled block of lines. */
export function Section(c: Ctx, title: string, ...children: unknown[]): JSX.Element {
  const { Box, Text } = c.els
  return (
    <Box flexDirection="column" marginTop={1}>
      <Text bold>{title}</Text>
      {children as JSX.Element[]}
    </Box>
  )
}

export function Line(c: Ctx, text: string, opts: { dim?: boolean; tone?: 'warn' | 'error' | 'ok' } = {}): JSX.Element {
  const { Text } = c.els
  const color = opts.tone === 'error' ? 'error' : opts.tone === 'warn' ? 'warning' : opts.tone === 'ok' ? 'success' : undefined
  return (
    <Text dimColor={opts.dim} color={color}>
      {text}
    </Text>
  )
}

export function Btn(c: Ctx, key: string, labelText: string, onPress: () => void, primary = false): JSX.Element {
  const { Button } = c.els
  return <Button key={key} label={labelText} variant={primary ? 'primary' : undefined} onPress={onPress} />
}

export function Row(c: Ctx, ...children: unknown[]): JSX.Element {
  const { Box } = c.els
  return (
    <Box flexDirection="row" flexWrap="wrap" columnGap={1}>
      {children as JSX.Element[]}
    </Box>
  )
}

/** A text field; falls back to a note on a surface without Input. */
export function Field($: EngineInterface, c: Ctx, key: string, labelText: string, opts: { placeholder?: string; submitLabel?: string; onSubmit: (value: string) => void }): JSX.Element {
  if (!c.canType) return Line(c, `${labelText}: type this in a terminal or desktop session (/quay).`, { dim: true })
  const { Input } = c.els
  return (
    <Input
      key={key}
      label={`${labelText}: `}
      placeholder={opts.placeholder}
      value={c.form[key] ?? ''}
      submitLabel={opts.submitLabel}
      onInput={(value: string) => setForm($, key, value)}
      onSubmit={(value: string) => {
        setForm($, key, value)
        opts.onSubmit(value)
      }}
    />
  )
}

/** A pick list; falls back to one button per option on a surface without Select. */
export function Pick(c: Ctx, key: string, labelText: string, options: { value: string; label: string }[], value: string | undefined, onSelect: (value: string) => void): JSX.Element {
  if (!options.length) return Line(c, `${labelText}: nothing to choose`, { dim: true })
  if (!c.canType) {
    return Row(c, Line(c, `${labelText}:`), ...options.map(o => Btn(c, `${key}:${o.value}`, o.value === value ? `* ${o.label}` : o.label, () => onSelect(o.value))))
  }
  const { Select } = c.els
  return <Select key={key} label={`${labelText}: `} options={options} value={value ?? options[0]!.value} onSelect={(v: string) => onSelect(v)} />
}

/** A checkbox drawn as a button whose label carries its state in words. */
export function Toggle(c: Ctx, key: string, labelText: string, on: boolean, onChange: (on: boolean) => void): JSX.Element {
  const { Button } = c.els
  return <Button key={key} plain label={`[${on ? 'x' : ' '}] ${labelText}`} onPress={() => onChange(!on)} />
}
