import { exp, log10, pow } from '../dsp/math'
import { RATE } from '../manifest'

const TARGET_DB = -23
const MAX_BOOST_DB = 18
const MAX_CUT_DB = -12
const FAST_SECS = 1
const GAIN_TAU = 0.2
const LIMIT_DB = -6
const LIMIT_RATIO = 20
const LIMIT_ATTACK = 0.003
const LIMIT_RELEASE = 0.1

const coef = (tau: number) => 1 - exp(-1 / (tau * RATE))
const perTick = (k: number, secs: number) => 1 - pow(1 - k, secs / 0.05)

// AGC on the sending side: follows speech only (Cue), fast on the first second of speech then slow, bounded gain, limiter
export class Level {
  private level = TARGET_DB
  private heard = 0
  private target = 1
  private gain = 1
  private env = 0
  private readonly limit = pow(10, LIMIT_DB / 20)

  get db(): number {
    return 20 * log10(this.gain)
  }

  // one frame of the audio about to be leveled, and whether Cue heard speech in it
  frame(pcm: Float32Array, speech: boolean): void {
    if (!speech) return
    let s = 0
    for (let i = 0; i < pcm.length; i++) s += pcm[i]! * pcm[i]!
    const db = 10 * log10(s / pcm.length + 1e-12)
    const secs = pcm.length / RATE
    const fast = this.heard < FAST_SECS
    this.heard += secs
    const k = perTick(fast ? 0.25 : db > this.level ? 0.08 : 0.025, secs)
    this.level += (db - this.level) * k
    const g = Math.max(MAX_CUT_DB, Math.min(MAX_BOOST_DB, TARGET_DB - this.level))
    this.target = pow(10, g / 20)
  }

  apply(pcm: Float32Array): void {
    const kg = coef(GAIN_TAU)
    const ka = coef(LIMIT_ATTACK)
    const kr = coef(LIMIT_RELEASE)
    for (let i = 0; i < pcm.length; i++) {
      this.gain += (this.target - this.gain) * kg
      const x = pcm[i]! * this.gain
      const a = Math.abs(x)
      this.env += (a - this.env) * (a > this.env ? ka : kr)
      pcm[i] = this.env > this.limit ? x * (this.limit * pow(this.env / this.limit, 1 / LIMIT_RATIO)) / this.env : x
    }
  }
}

const OPEN_TAU = 0.005
const CLOSE_TAU = 0.04

// call gate: opens on speech, closes after it, with a fade
export class Gate {
  private g = 0

  apply(pcm: Float32Array, open: boolean): void {
    const target = open ? 1 : 0
    const k = coef(target >= this.g ? OPEN_TAU : CLOSE_TAU)
    for (let i = 0; i < pcm.length; i++) {
      this.g += (target - this.g) * k
      pcm[i]! *= this.g
    }
  }
}

// a fixed delay in samples, for the raw signal aligned with Focus and the gate's look-ahead
export class Delay {
  private buf: Float32Array
  private pos = 0

  constructor(readonly samples: number) {
    this.buf = new Float32Array(Math.max(1, samples))
  }

  // in place: each sample comes out `samples` later
  apply(pcm: Float32Array): void {
    if (!this.samples) return
    for (let i = 0; i < pcm.length; i++) {
      const out = this.buf[this.pos]!
      this.buf[this.pos] = pcm[i]!
      pcm[i] = out
      this.pos = (this.pos + 1) % this.samples
    }
  }
}
