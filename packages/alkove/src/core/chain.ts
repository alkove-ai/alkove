import type { Alk, Model } from '../engine/alk'
import { FOCUS } from '../manifest'
import { Chunker } from './chunker'
import type { SpeechEvent, SpeechParams } from './cue'
import { SPEECH_DEFAULTS } from './cue'

const HOP = FOCUS.chunk
const WHY = ['silence', 'forced', 'wake']

export type Diag = { type: 'diag'; code: string; level: 'info' | 'warn'; data?: Record<string, unknown> }
export type ChainEvent = SpeechEvent | Diag | { type: 'idle'; asleep: boolean; at: number }

export interface ChainOptions {
  gate?: boolean
  normalize?: boolean
  strength?: number
  print?: ArrayLike<number> | null
  speech?: Partial<SpeechParams>
  sleep?: boolean
}

const bytes = (x: Float32Array) => new Uint8Array(x.buffer, x.byteOffset, x.byteLength)

// Focus -> Cue -> mix with the raw -> level -> gate, in the core (crates/core): real time and `process` give the same output
export class Chain {
  readonly delay: number
  private ptr: number
  private hops = new Chunker(HOP)
  private mix = new Float32Array(HOP)
  private focusOut = new Float32Array(HOP)
  onFocus: ((hop: Float32Array, t: number) => void) | null = null
  onCue: ((speech: number, t: number) => void) | null = null

  constructor(
    private alk: Alk,
    focus: Model,
    cue: Model,
    opts: ChainOptions = {},
  ) {
    const o = { gate: false, normalize: true, strength: 1, sleep: true, ...opts }
    const s = { ...SPEECH_DEFAULTS, ...opts.speech }
    const make = (p: number, n: number) => alk.x.alk_chain(focus.ptr, cue.ptr, +o.gate, +o.normalize, +o.sleep, o.strength, s.confidence, s.exitConfidence, s.startSecs, s.stopSecs, p, n)
    const print = opts.print ? Float32Array.from(opts.print) : null
    this.ptr = print ? alk.with(bytes(print), (p) => make(p, print.length)) : make(0, 0)
    if (!this.ptr) alk.fail('chain')
    this.delay = alk.x.alk_chain_get(this.ptr, 4)
  }

  get speaking(): boolean {
    return this.alk.x.alk_chain_get(this.ptr, 0) === 1
  }

  get sleeping(): boolean {
    return this.alk.x.alk_chain_get(this.ptr, 1) === 1
  }

  get time(): number {
    return this.alk.x.alk_chain_get(this.ptr, 2)
  }

  get gainDb(): number {
    return this.alk.x.alk_chain_get(this.ptr, 3)
  }

  release(): void {
    if (this.ptr) this.alk.x.alk_chain_free(this.ptr)
    this.ptr = 0
  }

  // applied at the next silence, from a fresh state
  setPrint(embedding: ArrayLike<number> | null): void {
    const x = this.alk.x
    if (!embedding) x.alk_chain_print(this.ptr, 0, 0)
    else {
      const p = Float32Array.from(embedding)
      if (this.alk.with(bytes(p), (at) => x.alk_chain_print(this.ptr, at, p.length)) < 0) this.alk.fail('print')
    }
  }

  setStrength(s: number): void {
    this.alk.x.alk_chain_strength(this.ptr, s)
  }

  setEnabled(on: boolean): void {
    this.alk.x.alk_chain_enabled(this.ptr, +on)
  }

  // exactly one hop: the real-time worker's path; the result is valid until the next call
  step(hop: Float32Array, emit: (e: ChainEvent) => void): Float32Array {
    if (this.hops.pending || hop.length !== HOP) throw new RangeError('step() takes whole hops only')
    return this.hop(hop, emit)
  }

  push(pcm: Float32Array, emit: (e: ChainEvent) => void): Float32Array {
    const out = new Float32Array(Math.floor((this.hops.pending + pcm.length) / HOP) * HOP)
    let o = 0
    this.hops.push(pcm, (hop) => {
      out.set(this.hop(hop, emit), o)
      o += HOP
    })
    return out
  }

  private hop(hop: Float32Array, emit: (e: ChainEvent) => void): Float32Array {
    const { alk, ptr } = this
    const x = alk.x
    new Float32Array(alk.heap, x.alk_chain_hop(ptr), HOP).set(hop)
    const out = x.alk_chain_step(ptr)
    this.mix.set(new Float32Array(alk.heap, out, HOP))
    // copied first: a listener may grow the wasm memory, which detaches views
    const rec = new Float64Array(alk.heap, x.alk_chain_events(ptr), 4 * x.alk_chain_event_count(ptr)).slice()
    for (let i = 0; i < rec.length; i += 4) {
      const [a, b, c] = [rec[i + 1]!, rec[i + 2]!, rec[i + 3]!]
      switch (rec[i]) {
        case 0:
          emit({ type: 'speechstart', start: a })
          break
        case 1:
          emit({ type: 'speechend', start: a, end: b })
          break
        case 2:
          emit({ type: 'idle', asleep: a === 1, at: b })
          break
        case 3:
          emit({ type: 'diag', code: 'focus.print', level: 'info', data: { print: a === 1, after: b, why: WHY[c]! } })
          break
        case 4:
          emit({ type: 'diag', code: 'focus.reset', level: 'info', data: { after: a, why: WHY[b]! } })
          break
        case 5:
          if (this.onFocus) {
            this.focusOut.set(new Float32Array(alk.heap, x.alk_chain_focus(ptr), HOP))
            this.onFocus(this.focusOut, a)
          }
          break
        case 6:
          this.onCue?.(a, b)
      }
    }
    return this.mix
  }
}
