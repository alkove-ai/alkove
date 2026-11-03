import { useCallback, useEffect, useRef, useState } from 'react'

import type { Voiceprint, VoiceprintJSON } from './core/timbre'
import { createAgentInput, createCallInput, createWakeListener } from './recipes/input'
import type { AgentInput, AgentOptions, CallInput, CallOptions, InputEvents, Mode, Status, WakeListener, WakeOptions } from './recipes/input'

export type Handlers = { [K in keyof InputEvents]?: (value: InputEvents[K]) => void }

export interface HookOptions {
  enabled?: boolean
  on?: Handlers
}

export interface InputState<I> {
  input: I | null
  status: Status | 'off'
  mode: Mode | null
  error: Error | null
  track: MediaStreamTrack | null
  resume: () => Promise<void>
}

type Live = { strength?: number; voiceprint?: Voiceprint | VoiceprintJSON | string | null }
type Input = AgentInput | CallInput | WakeListener
type Opened = Input & { setStrength(s: number): void; setVoiceprint(p: Live['voiceprint']): void }

const EVENTS = ['speechstart', 'speechend', 'speechcancel', 'turn', 'turnend', 'interruptstart', 'interrupt', 'interruptcancel', 'mode', 'status', 'voiceprint', 'learn', 'mic', 'trackchange', 'wake', 'heard', 'trace', 'audio', 'utterance', 'take', 'enrolled'] as const satisfies readonly (keyof InputEvents)[]

const plain = (v: object) => Array.isArray(v) || Object.getPrototypeOf(v) === Object.prototype

// by content: an options object written inline gives a new object each render
export function same(a: unknown, b: unknown): boolean {
  if (Object.is(a, b)) return true
  if (typeof a !== 'object' || typeof b !== 'object' || !a || !b) return false
  const ja = (a as { toJSON?: () => unknown }).toJSON
  const jb = (b as { toJSON?: () => unknown }).toJSON
  if (typeof ja === 'function' || typeof jb === 'function') return same(typeof ja === 'function' ? ja.call(a) : a, typeof jb === 'function' ? jb.call(b) : b)
  if (!plain(a) || !plain(b) || Array.isArray(a) !== Array.isArray(b)) return false
  const ka = Object.keys(a).filter((k) => (a as Record<string, unknown>)[k] !== undefined)
  const kb = Object.keys(b).filter((k) => (b as Record<string, unknown>)[k] !== undefined)
  return ka.length === kb.length && ka.every((k) => same((a as Record<string, unknown>)[k], (b as Record<string, unknown>)[k]))
}

function useInput<O extends object, I extends Input>(create: (o: O) => Promise<I>, options: O & HookOptions, live: readonly (keyof Live)[]): InputState<I> {
  const { enabled = true, on, ...rest } = options
  const handlers = useRef(on)
  handlers.current = on
  const opts = rest as unknown as O & Live
  const latest = useRef(opts)
  latest.current = opts

  const structural = Object.fromEntries(Object.entries(opts).filter(([k]) => !(live as readonly string[]).includes(k)))
  const shape = useRef<{ value: object; version: number }>({ value: structural, version: 0 })
  if (!same(shape.current.value, structural)) shape.current = { value: structural, version: shape.current.version + 1 }
  const version = shape.current.version

  const [input, setInput] = useState<I | null>(null)
  const [status, setStatus] = useState<Status | 'off'>('off')
  const [mode, setMode] = useState<Mode | null>(null)
  const [error, setError] = useState<Error | null>(null)
  const [track, setTrack] = useState<MediaStreamTrack | null>(null)
  const applied = useRef<Live>({})
  const previous = useRef<Promise<unknown>>(Promise.resolve())

  useEffect(() => {
    if (!enabled) return
    const abort = new AbortController()
    let opened: I | null = null
    const offs: (() => void)[] = []
    setStatus('starting')
    setError(null)
    // after the previous input settled: React's strict mode mounts twice, and two inputs on one mic run everything twice
    const opening = previous.current.then(() => {
      if (abort.signal.aborted) return
      const start = { ...latest.current, signal: abort.signal }
      applied.current = { strength: start.strength, voiceprint: start.voiceprint }
      return create(start)
    })
    previous.current = opening.catch(() => {})
    opening.then(
      (i) => {
        if (!i) return
        if (abort.signal.aborted) return i.dispose()
        opened = i
        for (const type of EVENTS) offs.push(i.on(type, ((v: never) => (handlers.current?.[type] as ((v: never) => void) | undefined)?.(v)) as never))
        offs.push(i.on('status', setStatus), i.on('mode', (m) => setMode(m.mode)), i.on('trackchange', setTrack))
        setInput(i)
        setStatus(i.status)
        setMode(i.mode)
        setTrack(i.track)
      },
      (e: unknown) => {
        if (abort.signal.aborted) return
        setError(e instanceof Error ? e : new Error(String(e)))
        setStatus('off')
      },
    )
    return () => {
      abort.abort()
      for (const off of offs) off()
      opened?.dispose()
      setInput(null)
      setStatus('off')
      setMode(null)
      setTrack(null)
    }
  }, [enabled, version, create])

  // subscribed only while the app listens: an input with no diag listener warns in the console
  const wantsDiag = !!on?.diag
  useEffect(() => {
    if (!input || !wantsDiag) return
    return input.on('diag', (d) => handlers.current?.diag?.(d))
  }, [input, wantsDiag])

  useEffect(() => {
    if (!input) return
    const i = input as unknown as Opened
    if (live.includes('strength') && opts.strength !== undefined && opts.strength !== applied.current.strength) i.setStrength(opts.strength)
    if (live.includes('voiceprint') && !same(opts.voiceprint ?? null, applied.current.voiceprint ?? null)) i.setVoiceprint(opts.voiceprint ?? null)
    applied.current = { strength: opts.strength, voiceprint: opts.voiceprint }
  })

  const resume = useCallback(async () => {
    await input?.resume()
  }, [input])

  return { input, status, mode, error, track, resume }
}

const LIVE = ['strength', 'voiceprint'] as const

export function useAgentInput(options: Omit<AgentOptions, 'signal'> & HookOptions = {}): InputState<AgentInput> {
  return useInput(createAgentInput, options, LIVE)
}

export function useCallInput(options: Omit<CallOptions, 'signal'> & HookOptions = {}): InputState<CallInput> {
  return useInput(createCallInput, options, LIVE)
}

export function useWakeListener(options: Omit<WakeOptions, 'signal'> & HookOptions): InputState<WakeListener> {
  return useInput(createWakeListener, options, [])
}
