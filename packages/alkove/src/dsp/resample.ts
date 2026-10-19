import { cos, sin } from './math'

const TAPS = 24
const PHASES = 128

// polyphase windowed sinc at a fractional read position kept across chunks: any ratio (44.1 -> 16 kHz) without drift
export class Resampler {
  // the delay it adds, in input samples
  static readonly HALF = TAPS / 2
  readonly identity: boolean
  private ratio = 1
  private half = TAPS / 2
  private table = new Float32Array(0)
  private work = new Float32Array(0)
  private histLen = TAPS
  private t = TAPS / 2

  // aligned: output k sits exactly on input position k × ratio (files), else half a window late with less latency (real time)
  constructor(
    readonly inRate: number,
    readonly outRate: number,
    aligned = false,
  ) {
    this.identity = inRate === outRate
    if (aligned) this.t = TAPS
    if (this.identity) return
    this.ratio = inRate / outRate
    const half = this.half
    // just below the tighter Nyquist of the two rates, Blackman window, unit gain per phase
    const cutoff = 0.9 / Math.max(1, this.ratio)
    const table = new Float32Array((PHASES + 1) * TAPS)
    for (let p = 0; p <= PHASES; p++) {
      const frac = p / PHASES
      let sum = 0
      for (let i = 0; i < TAPS; i++) {
        const d = i - (half - 1) - frac
        const x = cutoff * d
        const sinc = x === 0 ? 1 : sin(Math.PI * x) / (Math.PI * x)
        const w = 0.42 + 0.5 * cos((Math.PI * d) / half) + 0.08 * cos((2 * Math.PI * d) / half)
        table[p * TAPS + i] = sinc * w
        sum += table[p * TAPS + i]!
      }
      for (let i = 0; i < TAPS; i++) table[p * TAPS + i]! /= sum
    }
    this.table = table
  }

  // the samples `chunk` completes, appended through `out`
  push(chunk: Float32Array, out: (x: number) => void): void {
    if (this.identity) {
      for (let i = 0; i < chunk.length; i++) out(chunk[i]!)
      return
    }
    if (this.work.length < TAPS + 8 + chunk.length) {
      const w = new Float32Array(TAPS + 8 + chunk.length)
      w.set(this.work.subarray(0, this.histLen))
      this.work = w
    }
    const buf = this.work
    buf.set(chunk, this.histLen)
    const total = this.histLen + chunk.length
    const limit = total - 1 - this.half
    let t = this.t
    while (t <= limit) {
      const n = Math.floor(t)
      const row = Math.round((t - n) * PHASES) * TAPS
      const base = n - (this.half - 1)
      let acc = 0
      for (let i = 0; i < TAPS; i++) acc += buf[base + i]! * this.table[row + i]!
      out(acc)
      t += this.ratio
    }
    const keep = Math.floor(t) - (this.half - 1)
    buf.copyWithin(0, keep, total)
    this.histLen = total - keep
    this.t = t - keep
  }

  // a whole signal at once, its edges padded so nothing is cut
  static all(pcm: Float32Array, inRate: number, outRate: number): Float32Array {
    if (inRate === outRate) return pcm
    const r = new Resampler(inRate, outRate, true)
    const n = Math.round((pcm.length * outRate) / inRate)
    const out = new Float32Array(n)
    let o = 0
    const put = (x: number) => {
      if (o < n) out[o++] = x
    }
    r.push(pcm, put)
    r.push(new Float32Array(Math.ceil(TAPS * r.ratio) + 16), put)
    return out
  }
}
