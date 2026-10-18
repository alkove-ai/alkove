import { RATE, TIMBRE } from '../manifest'
import { type Center, dot, type Source, threshold, trimmedMean, Voiceprint } from './timbre'

export const LEARN = {
  ready: 4,
  max: 200,
  // a window joins a print still being built over this cosine, a ready print over the measured `balanced` one for 1.5 s
  match: 0.25,
  restart: 5,
  level: 0.005,
  resend: 0.98,
}

export interface Offer {
  accepted: number
  rejected: number
  progress: number
  ready: boolean
  restarted: boolean
}

// a print learned window by window: a window joins only if it looks like the windows so far, 5 rejects in a row before the print is ready start over
export class VoiceprintLearner {
  private ws: Center[] = []
  private rejects = 0

  constructor(
    private window: (pcm: Float32Array) => Center,
    readonly source: Source = 'focus',
    start?: Voiceprint | null,
  ) {
    // a print without its windows (made by Timbre.embed) counts as `ready` windows sharing its voice
    if (start) this.ws = start.windows.length ? [...start.windows] : Array.from({ length: LEARN.ready }, () => ({ ...start.centers[0]!, seconds: start.centers[0]!.seconds / LEARN.ready }))
  }

  get progress(): number {
    return Math.min(1, this.ws.length / LEARN.ready)
  }

  private get seconds(): number {
    return this.ws.reduce((n, w) => n + w.seconds, 0)
  }

  get ready(): boolean {
    return this.ws.length >= LEARN.ready
  }

  get voiceprint(): Voiceprint | null {
    return this.ready ? new Voiceprint([trimmedMean(this.ws, this.seconds)], this.source, TIMBRE.model, this.ws.slice(-64)) : null
  }

  // one window of my speech: its Timbre outputs, from a 1.5 s window the caller chose
  learn(w: Center): { accepted: boolean; score: number; restarted: boolean } {
    const score = this.ws.length ? dot(w.id, trimmedMean(this.ws, this.seconds).id) : 1
    if (score < (this.ready ? threshold('balanced', w.seconds, this.seconds) : LEARN.match)) {
      const restarted = !this.ready && ++this.rejects >= LEARN.restart
      if (restarted) {
        this.ws = []
        this.rejects = 0
      }
      return { accepted: false, score, restarted }
    }
    this.rejects = 0
    this.ws = [...this.ws, w].slice(-LEARN.max)
    return { accepted: true, score, restarted: false }
  }

  // any length of speech, cut into 1.5 s windows; quiet windows skipped
  offer(pcm: Float32Array): Offer {
    const out: Offer = { accepted: 0, rejected: 0, progress: 0, ready: false, restarted: false }
    const n = TIMBRE.window
    for (let i = 0; i + 0.9 * n <= pcm.length; i += n) {
      const win = pcm.subarray(i, Math.min(pcm.length, i + n))
      if (rms(win) < LEARN.level) continue
      const r = this.learn(this.window(win))
      if (r.accepted) out.accepted++
      else out.rejected++
      out.restarted ||= r.restarted
    }
    out.progress = this.progress
    out.ready = this.ready
    return out
  }

  forget(): void {
    this.ws = []
    this.rejects = 0
  }
}

export function rms(pcm: Float32Array): number {
  let s = 0
  for (let i = 0; i < pcm.length; i++) s += pcm[i]! * pcm[i]!
  return Math.sqrt(s / Math.max(1, pcm.length))
}

export const WINDOW_SECONDS = TIMBRE.window / RATE
