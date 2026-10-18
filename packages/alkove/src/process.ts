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
  spot?: { words: (Word | WordJSON | string)[]; context?: Context }
  normalize?: boolean
  trace?: boolean
  onProgress?: (p: { done: number; total: number }) => void
  signal?: AbortSignal
}

export interface TraceRow {
  t: number
  [key: string]: number
}

export interface ProcessResult {
  audio: Float32Array
  raw: Float32Array
  speech: { start: number; end: number }[]
  wakes: WakeEvent[]
  diag: ChainEvent[]
  trace?: { focus: TraceRow[]; cue: TraceRow[]; spot: TraceRow[] }
  toWav(which?: 'audio' | 'raw' | 'both'): Uint8Array
}

export function printOf(p: Voiceprint | VoiceprintJSON | string | null | undefined): Voiceprint | null {
  if (!p) return null
  if (p instanceof Voiceprint) return p
  return Voiceprint.fromJSON(p)
}

const BLOCK = 16000

// 16 kHz in, every chosen primitive over it in blocks as real time would feed them, delays taken out of the result
export function processPcm(engine: Engine, pcm: Float32Array, opts: ProcessOptions = {}): ProcessResult {
  const useFocus = opts.focus !== undefined && opts.focus !== false
  const focusOpts = typeof opts.focus === 'object' ? opts.focus : {}
  const speechParams = { ...SPEECH_DEFAULTS, ...(typeof opts.cue === 'object' ? opts.cue : {}) }
  const speech: { start: number; end: number }[] = []
  const wakes: WakeEvent[] = []
  const diag: ChainEvent[] = []
  const trace = opts.trace ? { focus: [] as TraceRow[], cue: [] as TraceRow[], spot: [] as TraceRow[] } : undefined
  const words = opts.spot?.words.map(wordOf) ?? []
  const spotter = words.length ? new Spotter(words, opts.spot?.context ?? 'anywhere') : null
  const spot = spotter ? new Spot(engine.stream('spot')) : null
  const shift = useFocus ? FOCUS.lag / RATE : 0
  const onSpot = (pcm16: Float32Array) =>
    spot!.push(pcm16, (f, step) => {
      spotter!.frame(f, (e) => wakes.push({ ...e, start: Math.max(0, e.start - shift), end: Math.max(0, e.end - shift) }))
      if (trace) trace.spot.push({ t: step * 0.03 - shift, speech: f.speech ?? 0, ...Object.fromEntries(spotter!.scores.map((s) => [s.word, s.score])) })
    })
  let open: number | null = null
  const onSpeech = (e: ChainEvent) => {
    if (e.type === 'speechstart') open = e.start
    else if (e.type === 'speechend') {
      speech.push({ start: e.start, end: e.end })
      open = null
    } else diag.push(e)
  }

  let audio: Float32Array
  const total = pcm.length
  if (useFocus) {
    const focus = new Focus(engine.stream('focus'))
    const cue = new Cue(engine.stream('cue'))
    const voiceprint = printOf(focusOpts.voiceprint)
    const chain = new Chain(focus, cue, {
      gate: false,
      normalize: opts.normalize ?? false,
      strength: focusOpts.strength ?? 1,
      print: voiceprint?.embedding ?? null,
      speech: speechParams,
    })
    if (spot || trace) {
      chain.onFocus = (hop, t) => {
        if (spot) onSpot(hop)
        if (trace) {
          let s = 0
          for (const x of hop) s += x * x
          trace.focus.push({ t: t / RATE - shift, db: +(10 * Math.log10(s / hop.length + 1e-12)).toFixed(2) })
        }
      }
    }
    const parts: Float32Array[] = []
    const run = (block: Float32Array) => parts.push(chain.push(block, onSpeech))
    for (let i = 0; i < total; i += BLOCK) {
      opts.signal?.throwIfAborted()
      run(pcm.subarray(i, Math.min(total, i + BLOCK)))
      opts.onProgress?.({ done: Math.min(total, i + BLOCK), total })
    }
    run(new Float32Array(chain.delay + CUE.chunk * 2 + FOCUS.chunk))
    audio = concat(parts).subarray(chain.delay, chain.delay + total)
    focus.release()
    cue.release()
  } else {
    audio = pcm
    if (opts.cue) {
      const cue = new Cue(engine.stream('cue'))
      const tracker = new SpeechTracker(speechParams)
      const feed = (block: Float32Array) =>
        cue.push(block, (p, k) => {
          if (trace) trace.cue.push({ t: k * 0.03, speech: p })
          const e = tracker.push(p, k)
          if (e) onSpeech(e)
        })
      for (let i = 0; i < total; i += BLOCK) {
        opts.signal?.throwIfAborted()
        feed(pcm.subarray(i, Math.min(total, i + BLOCK)))
        opts.onProgress?.({ done: Math.min(total, i + BLOCK), total })
      }
      feed(new Float32Array(CUE.chunk * (CUE.lagSteps + 1)))
      cue.release()
    }
    if (spot) {
      for (let i = 0; i < total; i += BLOCK) {
        opts.signal?.throwIfAborted()
        onSpot(pcm.subarray(i, Math.min(total, i + BLOCK)))
      }
      onSpot(new Float32Array(480 * 4))
    }
  }
  if (open !== null) speech.push({ start: open, end: total / RATE })
  spot?.release()
  const raw = pcm
  return {
    audio,
    raw,
    speech: opts.cue === false ? [] : speech,
    wakes,
    diag,
    trace,
    toWav(which = 'audio') {
      return encodeWav(which === 'both' ? [raw, audio] : [which === 'raw' ? raw : audio], RATE)
    },
  }
}

function concat(parts: Float32Array[]): Float32Array {
  const out = new Float32Array(parts.reduce((n, p) => n + p.length, 0))
  let o = 0
  for (const p of parts) {
    out.set(p, o)
    o += p.length
  }
  return out
}
