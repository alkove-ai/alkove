import { SAME, SCHWA } from './phones'

export const NEG = -1e9

// CTC keyword Viterbi over one pronunciation: score = mean log-likelihood ratio per phoneme (0 = perfect), and the frame the path starts on
export class Search {
  private L: number
  private S: number
  private sets: number[][]
  private allow: number[][]
  private ends: number[]
  private maxFrames: number
  private V!: Float64Array
  private start!: Int32Array
  private nV: Float64Array
  private nStart: Int32Array

  constructor(
    readonly ids: readonly number[],
    lang: string,
    frame: number,
  ) {
    const L = ids.length
    const S = 2 * L + 1
    const sets = ids.map((p) => SAME[p - 1]!)
    const opt = ids.map((p) => lang === 'fr' && p === SCHWA)
    const disjoint = (a: number[], b: number[]) => !a.some((x) => b.includes(x))
    const allow: number[][] = Array.from({ length: S }, () => [])
    for (let i = 0; i < L; i++) {
      const b = 2 * i
      const p = b + 1
      allow[p]!.push(0, 1)
      if (i >= 1 && disjoint(sets[i]!, sets[i - 1]!)) allow[p]!.push(2)
      if (i >= 1 && opt[i - 1]) {
        allow[p]!.push(3)
        if (i >= 2 && disjoint(sets[i]!, sets[i - 2]!)) allow[p]!.push(4)
      }
      if (i >= 1) {
        allow[b]!.push(0, 1)
        if (opt[i - 1]) allow[b]!.push(...(i >= 2 ? [2, 3] : [2]))
      }
    }
    allow[2 * L]!.push(0, 1)
    this.L = L
    this.S = S
    this.sets = sets
    this.allow = allow
    this.maxFrames = Math.floor(2.5 / frame)
    this.ends = [2 * L - 1, 2 * L, ...(opt[L - 1] && L >= 2 ? [2 * L - 3, 2 * L - 2] : [])]
    this.nV = new Float64Array(S)
    this.nStart = new Int32Array(S)
    this.reset()
  }

  reset(): void {
    this.V = new Float64Array(this.S).fill(NEG)
    this.start = new Int32Array(this.S)
  }

  step(logp: Float32Array, t: number): { score: number; start: number } {
    let best = -Infinity
    for (let k = 0; k < logp.length; k++) if (logp[k]! > best) best = logp[k]!
    const V = this.V
    const st = this.start
    const nV = this.nV.fill(NEG)
    const nst = this.nStart.fill(0)
    V[0] = 0
    st[0] = t
    for (let s = 1; s < this.S; s++) {
      let sc = NEG
      let from = -1
      for (const off of this.allow[s]!) if (s - off >= 0 && V[s - off]! > sc) {
        sc = V[s - off]!
        from = s - off
      }
      if (sc <= NEG / 2 || t - st[from]! >= this.maxFrames) continue
      let e: number
      if (s % 2) {
        let p = 0
        for (const k of this.sets[(s - 1) / 2]!) p += Math.exp(logp[k]!)
        e = Math.log(Math.max(p, 1e-10)) - best
      } else e = logp[0]! - best
      nV[s] = sc + e
      nst[s] = st[from]!
    }
    this.nV = V
    this.nStart = st
    this.V = nV
    this.start = nst
    let end = NEG
    let first = t
    for (const s of this.ends) if (nV[s]! > end) {
      end = nV[s]!
      first = nst[s]!
    }
    return { score: end > NEG / 2 ? end / this.L : NEG, start: first }
  }
}

// a path starting at frame `start` comes after `wait` quiet frames of the speech head, `guard` frames of speech allowed just before it
export class Pause {
  private runs: Int32Array
  private n = 0

  constructor(
    readonly wait: number,
    readonly guard: number,
    readonly level: number,
    keep = 512,
  ) {
    this.runs = new Int32Array(keep)
  }

  push(p: number): void {
    const k = this.runs.length
    this.runs[this.n % k] = p < this.level ? (this.n ? this.runs[(this.n - 1) % k]! : 0) + 1 : 0
    this.n++
  }

  after(start: number): boolean {
    let r = 0
    for (let g = 0; g <= this.guard; g++) {
      const i = start - 1 - g
      if (i >= 0 && i < this.n && i >= this.n - this.runs.length) r = Math.max(r, this.runs[i % this.runs.length]!)
    }
    return r >= this.wait
  }

  reset(): void {
    this.n = 0
  }
}
