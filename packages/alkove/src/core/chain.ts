import { CUE, FOCUS, RATE } from '../manifest'
import { Chunker } from './chunker'
import type { Cue, SpeechEvent, SpeechParams } from './cue'
import { SPEECH_DEFAULTS, SpeechTracker } from './cue'
import type { Focus } from './focus'
import { Delay, Gate, Level } from './level'

const HOP = FOCUS.chunk
// fresh state after 1 s of silence, every 30 s at most, every 60 s even without silence
const RESET_SILENCE = 1 * RATE
const RESET_EVERY = 30 * RATE
const RESET_FORCE = 60 * RATE
// asleep once Focus's output stayed under QUIET for SLEEP_AFTER; woken when the mic rises WAKE_DB over its noise floor
const QUIET = 0.005
const SLEEP_AFTER = 2 * RATE
const WAKE_DB = 9
const WAKE_MIN = 1e-8
const FLOOR_RISE = 1.0006
const GATE_LOOKAHEAD = 0.1 * RATE
const FADE_TAU = 0.01

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

// Focus -> Cue -> mix with the raw -> level -> gate, every rule that touches state counted in samples: real time and `process` give the same output
export class Chain {
  readonly delay: number
  private hops = new Chunker(HOP)
  private cueIn = new Chunker(CUE.chunk)
  private tracker: SpeechTracker
  private rawDelay = new Delay(FOCUS.lag)
  private gateDelay: Delay | null
  private gate = new Gate()
  private level = new Level()
  private focusOut = new Float32Array(HOP)
  private raw = new Float32Array(HOP)
  private mix = new Float32Array(HOP)
  private cueFrames = 0
  private cueSteps = 0
  private sinceSpeech = RESET_SILENCE
  private sinceReset = 0
  private pending: ArrayLike<number> | null | undefined
  private strength: number
  private enabled = true
  private s: number
  private asleep = false
  private quiet = 0
  private micLevel = 0
  private floor = 1
  private t = 0
  private opts: Required<Omit<ChainOptions, 'print' | 'speech'>>
  onFocus: ((hop: Float32Array, t: number) => void) | null = null

  constructor(
    private focus: Focus,
    private cue: Cue,
    opts: ChainOptions = {},
  ) {
    this.opts = { gate: false, normalize: true, strength: 1, sleep: true, ...opts }
    this.strength = this.s = this.opts.strength
    this.tracker = new SpeechTracker({ ...SPEECH_DEFAULTS, ...opts.speech })
    this.gateDelay = this.opts.gate ? new Delay(GATE_LOOKAHEAD) : null
    this.delay = FOCUS.lag + (this.opts.gate ? GATE_LOOKAHEAD : 0)
    if (opts.print) focus.setPrint(opts.print)
  }

  get speaking(): boolean {
    return this.tracker.active
  }

  get sleeping(): boolean {
    return this.asleep
  }

  get time(): number {
    return this.t / RATE
  }

  get gainDb(): number {
    return this.level.db
  }

  release(): void {
    this.focus.release()
    this.cue.release()
  }

  // applied at the next silence, from a fresh state
  setPrint(embedding: ArrayLike<number> | null): void {
    this.pending = embedding
  }

  setStrength(s: number): void {
    this.strength = Math.max(0, Math.min(1, s))
  }

  setEnabled(on: boolean): void {
    this.enabled = on
  }

  // exactly one hop, no allocation: the real-time worker's path; the result is valid until the next call
  step(hop: Float32Array, emit: (e: ChainEvent) => void): Float32Array {
    if (this.hops.pending || hop.length !== HOP) throw new RangeError('step() takes whole hops only')
    this.hop(hop, emit)
    return this.mix
  }

  push(pcm: Float32Array, emit: (e: ChainEvent) => void): Float32Array {
    const out = new Float32Array(Math.floor((this.hops.pending + pcm.length) / HOP) * HOP)
    let o = 0
    this.hops.push(pcm, (hop) => {
      this.hop(hop, emit)
      out.set(this.mix, o)
      o += HOP
    })
    return out
  }

  private hop(hop: Float32Array, emit: (e: ChainEvent) => void): void {
    const at = this.t / RATE
    this.raw.set(hop)
    this.rawDelay.apply(this.raw)
    this.listen(hop)
    if (this.asleep && this.micLevel > WAKE_MIN && this.micLevel > this.floor * 10 ** (WAKE_DB / 10)) {
      this.asleep = false
      this.quiet = 0
      // the state is kept across the sleep, as chat's worklet did: a fresh one leaks the first frames of a voice the print refuses
      if (this.pending !== undefined) this.fresh(emit, 'wake')
      emit({ type: 'idle', asleep: false, at })
    }
    if (this.asleep) this.focusOut.fill(0)
    else {
      this.rules(emit)
      this.focus.step(hop, this.focusOut)
    }
    this.onFocus?.(this.focusOut, this.t)
    this.cueIn.push(this.focusOut, (c) => this.cueFrame(c, emit))
    let peak = 0
    for (let i = 0; i < HOP; i++) peak = Math.max(peak, Math.abs(this.focusOut[i]!))
    this.quiet = peak < QUIET ? this.quiet + HOP : 0
    if (this.opts.sleep && !this.asleep && this.quiet >= SLEEP_AFTER) {
      this.asleep = true
      emit({ type: 'idle', asleep: true, at })
    }
    const target = this.enabled ? this.strength : 0
    const k = 1 - Math.exp(-1 / (FADE_TAU * RATE))
    for (let i = 0; i < HOP; i++) {
      this.s += (target - this.s) * k
      this.mix[i] = this.s * this.focusOut[i]! + (1 - this.s) * this.raw[i]!
    }
    if (this.opts.normalize) {
      this.level.frame(this.mix, this.tracker.active)
      this.level.apply(this.mix)
    }
    if (this.gateDelay) {
      this.gateDelay.apply(this.mix)
      this.gate.apply(this.mix, this.tracker.active)
    }
    this.sinceSpeech = this.tracker.active ? 0 : this.sinceSpeech + HOP
    this.sinceReset += HOP
    this.t += HOP
  }

  private listen(hop: Float32Array): void {
    for (let h = 0; h < HOP; h += 128) {
      let sq = 0
      for (let i = h; i < h + 128; i++) sq += hop[i]! * hop[i]!
      this.micLevel += 0.1 * (sq / 128 - this.micLevel)
      this.floor = Math.min(this.floor * FLOOR_RISE, this.micLevel)
    }
  }

  private rules(emit: (e: ChainEvent) => void): void {
    if (this.sinceSpeech < RESET_SILENCE && this.sinceReset < RESET_FORCE) return
    if (this.pending !== undefined || this.sinceReset >= RESET_EVERY) this.fresh(emit, this.sinceSpeech >= RESET_SILENCE ? 'silence' : 'forced')
  }

  private fresh(emit: (e: ChainEvent) => void, why: string): void {
    const after = +(this.sinceReset / RATE).toFixed(1)
    if (this.pending !== undefined) {
      this.focus.setPrint(this.pending)
      emit({ type: 'diag', code: 'focus.print', level: 'info', data: { print: !!this.pending, after, why } })
      this.pending = undefined
    } else {
      this.focus.reset()
      emit({ type: 'diag', code: 'focus.reset', level: 'info', data: { after, why } })
    }
    this.sinceReset = 0
  }

  private cueFrame(c: Float32Array, emit: (e: ChainEvent) => void): void {
    const k = this.cueFrames++
    let p = 0
    if (!this.asleep) {
      const v = this.cue.step(c)
      if (this.cueSteps++ >= CUE.lagSteps && v !== null) p = v
    }
    const e = this.tracker.push(p, k - CUE.lagSteps)
    if (!e) return
    const shift = FOCUS.lag / RATE
    if (e.type === 'speechstart') emit({ ...e, start: Math.max(0, e.start - shift) })
    else emit({ ...e, start: Math.max(0, e.start - shift), end: Math.max(0, e.end - shift) })
  }
}
