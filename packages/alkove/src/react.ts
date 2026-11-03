import { useCallback, useEffect, useLayoutEffect, useRef, useState } from 'react'

import type { Voiceprint, VoiceprintJSON } from './core/timbre'
import { createAgentInput, createCallInput, createWakeListener } from './recipes/input'
import type { AgentInput, AgentOptions, CallInput, CallOptions, InputEvents, Mode, Status, WakeListener, WakeOptions } from './recipes/input'
import { same } from './same'

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
type Forwarded = Exclude<keyof InputEvents, 'diag'>

// a Record, so that an event added to InputEvents fails to compile until it is forwarded
const FORWARDED: Record<Forwarded, true> = { speechstart: true, speechend: true, speechcancel: true, turn: true, turnend: true, interruptstart: true, interrupt: true, interruptcancel: true, mode: true, status: true, voiceprint: true, learn: true, mic: true, trackchange: true, wake: true, heard: true, trace: true, audio: true, utterance: true, take: true, enrolled: true }
const EVENTS = Object.keys(FORWARDED) as Forwarded[]

const DEFAULT_STRENGTH = 1

// every hook's inputs open one after the other: a remount opening while the previous input still loads would hold the mic twice
let queue: Promise<unknown> = Promise.resolve()

const useCommit = typeof window === 'undefined' ? useEffect : useLayoutEffect

function useInput<O extends object, I extends Input>(create: (o: O) => Promise<I>, options: O & HookOptions, live: readonly (keyof Live)[]): InputState<I> {
  const { enabled = true, on, ...rest } = options
  const opts = rest as unknown as O & Live
  const handlers = useRef(on)
  const latest = useRef(opts)

  const structural = Object.fromEntries(Object.entries(opts).filter(([k]) => !(live as readonly string[]).includes(k)))
  const shape = useRef<{ value: object; version: number }>({ value: structural, version: 0 })
  const version = same(shape.current.value, structural) ? shape.current.version : shape.current.version + 1

  // refs written once React commits: a render it throws away changes nothing
  useCommit(() => {
    handlers.current = on
    latest.current = opts
    if (version !== shape.current.version) shape.current = { value: structural, version }
  })

  const [input, setInput] = useState<I | null>(null)
  const [status, setStatus] = useState<Status | 'off'>('off')
  const [mode, setMode] = useState<Mode | null>(null)
  const [error, setError] = useState<Error | null>(null)
  const [track, setTrack] = useState<MediaStreamTrack | null>(null)
  const current = useRef<I | null>(null)
  const applied = useRef<Live>({})
  const diagOff = useRef<(() => void) | null>(null)

  // subscribed only while the app listens: an input with no diag listener warns in the console
  const syncDiag = useCallback(() => {
    const want = !!current.current && !!handlers.current?.diag
    if (want && !diagOff.current) diagOff.current = current.current!.on('diag', (d) => handlers.current?.diag?.(d))
    else if (!want && diagOff.current) {
      diagOff.current()
      diagOff.current = null
    }
  }, [])

  useEffect(() => {
    if (!enabled) return
    const abort = new AbortController()
    const offs: (() => void)[] = []
    setStatus('starting')
    setError(null)
    const opening = queue.then(() => {
      if (abort.signal.aborted) return
      const start = { ...latest.current, signal: abort.signal }
      applied.current = { strength: start.strength ?? DEFAULT_STRENGTH, voiceprint: start.voiceprint ?? null }
      return create(start)
    })
    queue = opening.catch(() => {})
    opening.then(
      (i) => {
        if (!i) return
        if (abort.signal.aborted) return i.dispose()
        current.current = i
        // before the task ends: the input delivers the events it held while opening on the next one
        syncDiag()
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
      diagOff.current?.()
      diagOff.current = null
      current.current?.dispose()
      current.current = null
      setInput(null)
      setStatus('off')
      setMode(null)
      setTrack(null)
      setError(null)
    }
  }, [enabled, version, create, syncDiag])

  const wantsDiag = !!on?.diag
  useEffect(syncDiag, [input, wantsDiag, syncDiag])

  const strength = live.includes('strength') ? opts.strength : undefined
  const voiceprint = live.includes('voiceprint') ? opts.voiceprint : undefined
  useEffect(() => {
    const i = current.current as Opened | null
    if (!i) return
    if (live.includes('strength') && (strength ?? DEFAULT_STRENGTH) !== applied.current.strength) {
      applied.current.strength = strength ?? DEFAULT_STRENGTH
      i.setStrength(applied.current.strength)
    }
    if (live.includes('voiceprint') && !same(voiceprint ?? null, applied.current.voiceprint ?? null)) {
      applied.current.voiceprint = voiceprint ?? null
      i.setVoiceprint(applied.current.voiceprint)
    }
  }, [input, strength, voiceprint, live])

  const resume = useCallback(async () => {
    await current.current?.resume()
  }, [])

  return { input, status, mode, error, track, resume }
}

const LIVE = ['strength', 'voiceprint'] as const
const NONE = [] as const

export function useAgentInput(options: Omit<AgentOptions, 'signal'> & HookOptions = {}): InputState<AgentInput> {
  return useInput(createAgentInput, options, LIVE)
}

export function useCallInput(options: Omit<CallOptions, 'signal'> & HookOptions = {}): InputState<CallInput> {
  return useInput(createCallInput, options, LIVE)
}

export function useWakeListener(options: Omit<WakeOptions, 'signal'> & HookOptions): InputState<WakeListener> {
  return useInput(createWakeListener, options, NONE)
}
