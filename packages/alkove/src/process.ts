import type { ChainEvent } from './core/chain'
import { type SpeechParams, SPEECH_DEFAULTS } from './core/cue'
import type { Engine } from './core/engine'
import type { NativeProcessor } from './core/native'
import { type Context, type WakeEvent, type Word, type WordJSON, wordOf } from './core/spot/spot'
import { Voiceprint, type VoiceprintJSON } from './core/timbre'
import { encodeWav } from './dsp/wav'
import { RATE } from './manifest'

export interface ProcessOptions {
  focus?: boolean | { voiceprint?: VoiceprintInput; strength?: number }
  cue?: boolean | Partial<SpeechParams>
  spot?: { words: readonly (Word | WordJSON | string)[]; context?: Context }
  normalize?: boolean
  trace?: boolean
  onProgress?: (p: { done: number; total: number }) => void
  signal?: AbortSignal
}

export interface TraceRow {
  t: number
  [key: string]: number
}

export interface Trace {
  focus: TraceRow[]
  cue: TraceRow[]
  spot: TraceRow[]
}

export interface ProcessResult {
  audio: Float32Array
  raw: Float32Array
  speech: { start: number; end: number }[]
  wakes: WakeEvent[]
  diag: ChainEvent[]
  trace?: Trace
  toWav(which?: 'audio' | 'raw' | 'both'): Uint8Array
}

// one block of a stream: its audio, and what was found while it went through (times from the stream's start)
export interface ProcessPart {
  start: number
  audio: Float32Array
  speech: { start: number; end: number }[]
  wakes: WakeEvent[]
  diag: ChainEvent[]
  trace?: Trace
}

// the focus options on their way to the core: a recording given as the voiceprint becomes its print and this sample
type FocusWire = { voiceprint?: VoiceprintInput; strength?: number; sample?: Float32Array }

// the core's options for these: { focus, cue, speech, normalize, trace, rate, spot }
export function coreOptions(opts: Omit<ProcessOptions, 'onProgress' | 'signal'>, rate = RATE) {
  const focus = opts.focus !== undefined && opts.focus !== false
  const f: FocusWire = typeof opts.focus === 'object' ? opts.focus : {}
  const print = printOf(f.voiceprint)?.embedding
  const words = opts.spot?.words.map(wordOf) ?? []
  return {
    focus: focus ? { print: print ? Array.from(print) : null, strength: f.strength ?? 1, ...(f.sample ? { sample: Array.from(f.sample) } : {}) } : null,
    // an object of settings turns Cue on, as true does
    cue: opts.cue === undefined || opts.cue === null ? null : opts.cue !== false,
    speech: { ...SPEECH_DEFAULTS, ...(typeof opts.cue === 'object' ? opts.cue : {}) },
    normalize: opts.normalize ?? false,
    trace: !!opts.trace,
    rate,
    spot: words.length ? { words: words.map((w) => w.toJSON()), context: opts.spot?.context ?? 'anywhere' } : null,
  }
}

// a print, or a recording of the user at 16 kHz (voiceOf makes its print)
export type VoiceprintInput = Voiceprint | VoiceprintJSON | string | Float32Array | null

export function printOf(p: VoiceprintInput | undefined): Voiceprint | null {
  if (!p || p instanceof Float32Array) return null
  if (p instanceof Voiceprint) return p
  return Voiceprint.fromJSON(p)
}

// a recording given as the voiceprint: the print Timbre makes of it, and the recording itself as Focus's sample
export async function voiceOf(p: VoiceprintInput | undefined): Promise<{ print: Voiceprint | VoiceprintJSON | string | null | undefined; sample?: Float32Array }> {
  if (!(p instanceof Float32Array)) return { print: p }
  const { Timbre } = await import('./api')
  return { print: await (await Timbre.create()).embed(p), sample: p }
}

// process() and stream() options, a recording as the voiceprint turned into a print and a sample
export async function withVoice<T extends ProcessOptions>(opts: T): Promise<T> {
  const f = opts.focus as FocusWire | boolean | undefined
  if (typeof f !== 'object' || !(f.voiceprint instanceof Float32Array)) return opts
  const v = await voiceOf(f.voiceprint)
  return { ...opts, focus: { ...f, voiceprint: v.print, sample: f.sample ?? v.sample } }
}

// every chosen primitive over 16 kHz audio pushed in blocks, as real time feeds them, in the core; the output comes back aligned with the input
export class Processor {
  private core: NativeProcessor
  private closed = false

  constructor(
    engine: Engine,
    readonly opts: Omit<ProcessOptions, 'onProgress' | 'signal'> = {},
  ) {
    this.core = engine.processor(coreOptions(opts))
  }

  push(pcm: Float32Array): ProcessPart {
    if (this.closed) throw new Error('push after end')
    return this.core.push(pcm) as unknown as ProcessPart
  }

  // the samples still inside the primitives come out, and the speech still open closes at the end
  end(): ProcessPart {
    if (this.closed) throw new Error('push after end')
    this.closed = true
    try {
      return this.core.end() as unknown as ProcessPart
    } finally {
      this.core.release()
    }
  }

  release(): void {
    this.closed = true
    this.core.release()
  }
}

const BLOCK = 16000

export function processPcm(engine: Engine, pcm: Float32Array, opts: ProcessOptions = {}): ProcessResult {
  const { onProgress, signal, ...rest } = opts
  const p = new Processor(engine, rest)
  const parts: ProcessPart[] = []
  try {
    for (let i = 0; i < pcm.length; i += BLOCK) {
      signal?.throwIfAborted()
      parts.push(p.push(pcm.subarray(i, Math.min(pcm.length, i + BLOCK))))
      onProgress?.({ done: Math.min(pcm.length, i + BLOCK), total: pcm.length })
    }
    parts.push(p.end())
  } finally {
    p.release()
  }
  const audio = new Float32Array(pcm.length)
  let o = 0
  for (const part of parts) {
    audio.set(part.audio.subarray(0, audio.length - o), o)
    o += part.audio.length
  }
  const raw = pcm
  const merged = parts.reduce<Omit<ProcessPart, 'start' | 'audio'>>(
    (m, x) => ({
      speech: [...m.speech, ...x.speech],
      wakes: [...m.wakes, ...x.wakes],
      diag: [...m.diag, ...x.diag],
      trace: x.trace || m.trace ? { focus: [...(m.trace?.focus ?? []), ...(x.trace?.focus ?? [])], cue: [...(m.trace?.cue ?? []), ...(x.trace?.cue ?? [])], spot: [...(m.trace?.spot ?? []), ...(x.trace?.spot ?? [])] } : undefined,
    }),
    { speech: [], wakes: [], diag: [] },
  )
  return {
    audio: rest.focus ? audio : pcm,
    raw,
    ...merged,
    toWav(which = 'audio') {
      return encodeWav(which === 'both' ? [raw, this.audio] : [which === 'raw' ? raw : this.audio], RATE)
    },
  }
}
