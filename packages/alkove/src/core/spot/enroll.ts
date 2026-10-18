import { type Lang, SPOT } from '../../manifest'
import { SpeechTracker } from '../cue'
import type { Warning } from '../lex'
import { NEG, Search } from './search'
import { BOUNDS, type Chain, clampThreshold, PAUSE_GAP, type SpotFrame, Word } from './spot'

// provisional: to set on the calibrated words of the bench by drawing 3 takes of one voice
export const ENROLL = {
  takes: 3,
  // the threshold stays this far under the worst take
  k: 0.3,
  // and this far over the best score of everything that is not a take
  margin: 0.15,
  // a speech segment whose best score stays under this is not the word
  detect: BOUNDS.min,
  // takes further apart than this (best minus worst) were said differently
  spread: 0.8,
}

export interface Take {
  score: number
  ok: boolean
  start: number
  end: number
  pronunciation: string
}

export interface EnrollResult {
  word: Word
  takes: Take[]
  warnings: Warning[]
}

// the takes are speech segments of Spot's own speech head; everything outside them sets the floor
export class Enroller {
  private searches: { s: Search; ipa: string; i: number }[]
  private tracker = new SpeechTracker({ confidence: 0.5, exitConfidence: 0.1, startSecs: 0.1, stopSecs: 0.3 }, SPOT.frame)
  private t = 0
  private seg: { best: number; which: number; frames: number } | null = null
  private rest = NEG
  readonly takes: Take[] = []
  private wins: number[]

  constructor(
    readonly text: string,
    readonly lang: Lang[],
    readonly phonemes: Word['phonemes'],
    readonly chain: Chain,
    readonly opts = ENROLL,
  ) {
    this.searches = phonemes.map((v, i) => ({ s: new Search(v.ids, v.lang, SPOT.frame), ipa: v.ipa, i }))
    this.wins = phonemes.map(() => 0)
  }

  get done(): boolean {
    return this.takes.filter((t) => t.ok).length >= this.opts.takes
  }

  frame(f: SpotFrame, onTake: (t: Take) => void): void {
    const t = this.t++
    let best = NEG
    let which = 0
    for (const { s, i } of this.searches) {
      const r = s.step(f.logp, t)
      if (r.score > best) {
        best = r.score
        which = i
      }
    }
    // the speech head runs `speechLagSteps` behind, the scores `lagSteps` behind: both on the audio's frame clock
    if (f.speech !== null) {
      const e = this.tracker.push(f.speech, t - SPOT.speechLagSteps)
      if (e?.type === 'speechstart') this.seg = { best: NEG, which: 0, frames: 0 }
      if (e?.type === 'speechend' && this.seg) {
        const ok = this.seg.best >= this.opts.detect
        const take: Take = { score: this.seg.best, ok, start: e.start, end: e.end, pronunciation: this.phonemes[this.seg.which]!.ipa }
        if (ok) {
          this.takes.push(take)
          this.wins[this.seg.which]!++
        } else this.rest = Math.max(this.rest, this.seg.best)
        onTake(take)
        this.seg = null
        for (const { s } of this.searches) s.reset()
      }
    }
    if (this.seg || this.tracker.active) {
      if (this.seg && best > this.seg.best) {
        this.seg.best = best
        this.seg.which = which
      }
    } else this.rest = Math.max(this.rest, best)
  }

  result(): EnrollResult {
    const ok = this.takes.filter((t) => t.ok)
    const warnings: Warning[] = []
    if (!ok.length) throw new RangeError('word.no-take: the word was not heard in any take')
    const worst = Math.min(...ok.map((t) => t.score))
    const bestTake = Math.max(...ok.map((t) => t.score))
    const ceiling = worst - this.opts.k
    const floor = this.rest + this.opts.margin
    if (bestTake - worst > this.opts.spread) warnings.push({ code: 'word.takes-differ', message: 'The takes were said differently: record them again.', spread: +(bestTake - worst).toFixed(2) })
    if (floor >= ceiling) warnings.push({ code: 'word.no-margin', message: 'Other speech scores as high as the word: it is too short, too close to a common word, or mispronounced.', floor: +floor.toFixed(2), ceiling: +ceiling.toFixed(2) })
    const raw = floor < ceiling ? ceiling : (floor + ceiling) / 2
    const anywhere = clampThreshold(raw)
    if (anywhere !== raw) warnings.push({ code: 'word.bounded', message: 'The threshold hit its bounds.', threshold: +raw.toFixed(2) })
    // the pronunciations that won a take; the likeliest one when none did
    const kept = this.phonemes.filter((_, i) => this.wins[i]! > 0)
    const word = new Word(this.text, this.lang, kept.length ? kept : [this.phonemes[0]!], { anywhere, pause: clampThreshold(anywhere + PAUSE_GAP) }, true, this.chain, warnings)
    return { word, takes: this.takes, warnings }
  }
}
