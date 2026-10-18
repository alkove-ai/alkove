import { Chain, type ChainEvent } from './core/chain'
import { Cue, type SpeechParams, SpeechTracker, SPEECH_DEFAULTS } from './core/cue'
import type { Engine } from './core/engine'
import { Focus } from './core/focus'
import { type Context, Spot, Spotter, type WakeEvent, type Word, type WordJSON, wordOf } from './core/spot/spot'
import { Voiceprint, type VoiceprintJSON } from './core/timbre'
import { encodeWav } from './dsp/wav'
import { CUE, FOCUS, RATE } from './manifest'

export interface ProcessOptions {
  focus?: boolean | { voiceprint?: Voiceprint | VoiceprintJSON | string | null; strength?: number }
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

export function printOf(p: Voiceprint | VoiceprintJSON | string | null | undefined): Voiceprint | null {
  if (!p) return null
  if (p instanceof Voiceprint) return p
  return Voiceprint.fromJSON(p)
}

// every chosen primitive over 16 kHz audio pushed in blocks, as real time feeds them; the output comes back aligned with the input
export class Processor {
  private chain: Chain | null = null
  private cue: Cue | null = null
  private tracker: SpeechTracker | null = null
  private spot: Spot | null = null
  private spotter: Spotter | null = null
  private focus: Focus | null = null
  private shift: number
  private skip: number
  private delay: number
  private pushed = 0
  private given = 0
  private open: number | null = null
  private part: Omit<ProcessPart, 'start' | 'audio'> = { speech: [], wakes: [], diag: [] }
  private closed = false

  constructor(
    engine: Engine,
    readonly opts: Omit<ProcessOptions, 'onProgress' | 'signal'> = {},
  ) {
    const useFocus = opts.focus !== undefined && opts.focus !== false
    const focusOpts = typeof opts.focus === 'object' ? opts.focus : {}
    const speech = { ...SPEECH_DEFAULTS, ...(typeof opts.cue === 'object' ? opts.cue : {}) }
    const words = opts.spot?.words.map(wordOf) ?? []
    if (words.length) {
      this.spotter = new Spotter(words, opts.spot?.context ?? 'anywhere')
      this.spot = new Spot(engine.stream('spot'))
    }
    this.shift = useFocus ? FOCUS.lag / RATE : 0
    if (useFocus) {
      this.focus = new Focus(engine.stream('focus'))
      this.cue = new Cue(engine.stream('cue'))
      this.chain = new Chain(this.focus, this.cue, {
        gate: false,
        normalize: opts.normalize ?? false,
        strength: focusOpts.strength ?? 1,
        print: printOf(focusOpts.voiceprint)?.embedding ?? null,
        speech,
      })
      if (opts.trace) this.chain.onCue = (speech, t) => this.trace().cue.push({ t: +t.toFixed(3), speech })
      this.chain.onFocus = (hop, t) => {
        if (this.spot) this.feedSpot(hop)
        if (opts.trace) {
          let s = 0
          for (const x of hop) s += x * x
          this.trace().focus.push({ t: t / RATE - this.shift, db: +(10 * Math.log10(s / hop.length + 1e-12)).toFixed(2) })
        }
      }
    } else if (opts.cue) {
      this.cue = new Cue(engine.stream('cue'))
      this.tracker = new SpeechTracker(speech)
    }
    this.delay = this.chain?.delay ?? 0
    this.skip = this.delay
  }

  private trace(): Trace {
    return (this.part.trace ??= { focus: [], cue: [], spot: [] })
  }

  private onEvent = (e: ChainEvent) => {
    if (e.type === 'speechstart') this.open = e.start
    else if (e.type === 'speechend') {
      if (this.opts.cue !== false) this.part.speech.push({ start: e.start, end: e.end })
      this.open = null
    } else this.part.diag.push(e)
  }

  private feedSpot(pcm: Float32Array): void {
    this.spot!.push(pcm, (f, step) => {
      this.spotter!.frame(f, (e) => this.part.wakes.push({ ...e, start: Math.max(0, e.start - this.shift), end: Math.max(0, e.end - this.shift) }))
      if (this.opts.trace) this.trace().spot.push({ t: step * 0.03 - this.shift, speech: f.speech ?? 0, ...Object.fromEntries(this.spotter!.scores.map((s) => [s.word, s.score])) })
    })
  }

  private run(pcm: Float32Array): Float32Array {
    if (this.chain) return this.chain.push(pcm, this.onEvent)
    if (this.cue)
      this.cue.push(pcm, (p, k) => {
        if (this.opts.trace) this.trace().cue.push({ t: k * 0.03, speech: p })
        const e = this.tracker!.push(p, k)
        if (e) this.onEvent(e)
      })
    if (this.spot) this.feedSpot(pcm)
    return pcm
  }

  private take(out: Float32Array, final: boolean): ProcessPart {
    const cut = Math.min(this.skip, out.length)
    this.skip -= cut
    let audio = out.subarray(cut)
    if (final) audio = audio.subarray(0, Math.max(0, this.pushed - this.given))
    const part: ProcessPart = { start: this.given / RATE, audio: audio.slice(), ...this.part }
    this.given += audio.length
    this.part = { speech: [], wakes: [], diag: [] }
    return part
  }

  push(pcm: Float32Array): ProcessPart {
    if (this.closed) throw new Error('push after end')
    this.pushed += pcm.length
    return this.take(this.run(pcm), false)
  }

  // the samples still inside the primitives come out, and the speech still open closes at the end
  end(): ProcessPart {
    this.closed = true
    const tail = this.run(new Float32Array(this.delay + CUE.chunk * (CUE.lagSteps + 2) + FOCUS.chunk + 480 * 4))
    if (this.open !== null && this.opts.cue !== false && (this.chain || this.cue)) this.part.speech.push({ start: this.open, end: this.pushed / RATE })
    const part = this.take(tail, true)
    for (const n of [this.focus, this.cue, this.spot]) n?.release()
    return part
  }
}

const BLOCK = 16000

export function processPcm(engine: Engine, pcm: Float32Array, opts: ProcessOptions = {}): ProcessResult {
  const { onProgress, signal, ...rest } = opts
  const p = new Processor(engine, rest)
  const parts: ProcessPart[] = []
  for (let i = 0; i < pcm.length; i += BLOCK) {
    signal?.throwIfAborted()
    parts.push(p.push(pcm.subarray(i, Math.min(pcm.length, i + BLOCK))))
    onProgress?.({ done: Math.min(pcm.length, i + BLOCK), total: pcm.length })
  }
  parts.push(p.end())
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
