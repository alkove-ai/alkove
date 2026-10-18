import { type Lang, MODELS, SPOT } from '../../manifest'
import { Chunker } from '../chunker'
import type { StreamNet } from '../engine'
import type { Lex, Warning } from '../lex'
import { checkText } from '../lex'
import { toIds, toIpa } from './phones'
import { NEG, Pause, Search } from './search'

export type Context = 'pause' | 'anywhere' | 'both'
export type Chain = 'raw' | 'focus'

export interface Thresholds {
  anywhere: number
  pause: number
}

export interface WordJSON {
  alkove: 'word'
  spot: string
  text: string
  lang: Lang[]
  phonemes: { ipa: string; lang: Lang | ''; p: number }[]
  thresholds: Thresholds
  calibrated: boolean
  chain: Chain
}

// median of ~30 typed words per language at 0.5 false alarm / h on the published Spot; ro's test set is too short, set by hand
const UNCALIBRATED: Partial<Record<Lang, number>> = { fr: -1.23, en: -1.21, es: -1.14, it: -1.01, pt: -1.39, ca: -1.12, ro: -1.2 }
const UNCALIBRATED_ANY = -1.17
// the threshold after a pause sits this much under the one anywhere (mean over the typed words)
export const PAUSE_GAP = -0.77
// shared by both thresholds: a hard word's pause threshold (under -2.2) is clamped, stricter than 0.5 false alarm / h
// a threshold after a pause often sits under −2.2 (10 to 43 % of the typed words benchmarked): it has its own floor
export const BOUNDS = { max: -0.6, min: -2.2, pauseMin: -2.9 }
// a Lex variant under this probability is not searched: on common words variants only add false alarms
export const VARIANT = 0.25
export const PAUSE = { wait: 10, guard: 7, level: 0.5 }
const REFRACTORY_S = 1
const VOWELS = 27

export class Word {
  readonly warnings: Warning[]

  constructor(
    readonly text: string,
    readonly lang: Lang[],
    readonly phonemes: { ipa: string; ids: number[]; lang: Lang | ''; p: number }[],
    readonly thresholds: Thresholds,
    readonly calibrated: boolean,
    readonly chain: Chain,
    warnings: Warning[] = [],
  ) {
    this.warnings = warnings
  }

  toJSON(): WordJSON {
    return {
      alkove: 'word',
      spot: MODELS.spot.name,
      text: this.text,
      lang: this.lang,
      phonemes: this.phonemes.map(({ ipa, lang, p }) => ({ ipa, lang, p: +p.toFixed(4) })),
      thresholds: this.thresholds,
      calibrated: this.calibrated,
      chain: this.chain,
    }
  }

  // null when the word was compiled for another Spot: its thresholds no longer hold, compile its text again
  static fromJSON(json: WordJSON | string): Word | null {
    const j: WordJSON = typeof json === 'string' ? JSON.parse(json) : json
    if (j.alkove !== 'word') throw new TypeError('not an Alkove word')
    if (j.spot !== MODELS.spot.name) return null
    const phonemes = j.phonemes.map((v) => ({ ...v, ids: toIds(v.ipa, v.lang) ?? [] })).filter((v) => v.ids.length)
    return new Word(j.text, j.lang, phonemes, j.thresholds, j.calibrated, j.chain)
  }
}

export const wordOf = (w: Word | WordJSON | string): Word => {
  if (w instanceof Word) return w
  const word = Word.fromJSON(w)
  if (!word) throw new TypeError(`word.version: compiled for another Spot, compile "${typeof w === 'string' ? JSON.parse(w).text : w.text}" again`)
  return word
}

export function defaultThresholds(lang: Lang[]): Thresholds {
  const known = lang.map((l) => UNCALIBRATED[l]).filter((v): v is number => v !== undefined)
  const anywhere = known.length ? Math.min(...known) : UNCALIBRATED_ANY
  return { anywhere, pause: clampPause(anywhere + PAUSE_GAP) }
}

export const clampThreshold = (v: number) => Math.max(BOUNDS.min, Math.min(BOUNDS.max, v))
export const clampPause = (v: number) => Math.max(BOUNDS.pauseMin, Math.min(BOUNDS.max, v))

export interface CompileOptions {
  lang?: Lang | Lang[]
  chain?: Chain
}

export function compileWord(text: string, opts: CompileOptions, lex: () => Lex): Word {
  const langs: Lang[] = opts.lang === undefined ? [] : Array.isArray(opts.lang) ? opts.lang : [opts.lang]
  const warnings: Warning[] = []
  let phonemes: Word['phonemes']
  const typed = /^\s*\/(.+)\/\s*$/.exec(text)
  if (typed) {
    const ids = toIds(typed[1]!, langs[0] ?? '')
    if (!ids?.length) throw new TypeError(`word.phonemes: no known phoneme in ${text}`)
    phonemes = [{ ids, ipa: toIpa(ids), lang: langs[0] ?? '', p: 1 }]
  } else {
    if (!langs.length) throw new TypeError('lang is required for a typed word')
    warnings.push(...checkText(text))
    const seen = new Set<string>()
    phonemes = []
    for (const l of langs)
      for (const v of lex().phonemize(text, l, 3, (v, i) => i === 0 || v.p >= VARIANT)) {
        const key = v.ids.join(',')
        if (seen.has(key)) continue
        seen.add(key)
        phonemes.push({ ...v, lang: l })
      }
    if (!phonemes.length) throw new TypeError(`word.phonemes: no phoneme for "${text}"`)
  }
  const vowels = phonemes[0]!.ids.filter((i) => i <= VOWELS).length
  if (phonemes[0]!.ids.length < 6 || vowels < 3) warnings.push({ code: 'word.too-short', message: 'Short words give more false alarms.', syllables: vowels, phonemes: phonemes[0]!.ids.length })
  return new Word(text, langs, phonemes, defaultThresholds(langs), false, opts.chain ?? 'raw', warnings)
}

export interface SpotFrame {
  logp: Float32Array
  speech: number | null
}

// phoneme log-posteriors every 30 ms (`lagSteps` late) and the speech head (`speechLagSteps` late)
export class Spot {
  static readonly chunk = SPOT.chunk
  readonly delay = SPOT.chunk * SPOT.lagSteps
  private chunker = new Chunker(SPOT.chunk)
  private steps = 0

  constructor(private net: StreamNet) {}

  step(chunk: Float32Array): SpotFrame {
    this.net.f32('audio').set(chunk)
    this.net.run()
    const s = this.steps++
    return { logp: this.net.f32('logp'), speech: s >= SPOT.speechLagSteps ? this.net.f32('speech')[0]! : null }
  }

  push(pcm: Float32Array, onFrame: (f: SpotFrame, step: number) => void): void {
    this.chunker.push(pcm, (c) => onFrame(this.step(c), this.steps - 1))
  }

  reset(): void {
    this.net.reset()
    this.chunker.reset()
    this.steps = 0
  }

  release(): void {
    this.net.release()
  }
}

export interface WakeEvent {
  word: string
  score: number
  threshold: number
  context: 'pause' | 'anywhere'
  start: number
  end: number
}

interface Listening {
  word: Word
  searches: { s: Search; lang: Lang | '' }[]
  until: number
  last: number
}

// the words searched in Spot's frames, after a pause or anywhere; times in seconds on the clock of Spot's input
export class Spotter {
  private list: Listening[] = []
  private pause = new Pause(PAUSE.wait, PAUSE.guard, PAUSE.level)
  private t = 0

  constructor(
    words: Word[],
    readonly context: Context,
  ) {
    this.setWords(words)
  }

  setWords(words: Word[]): void {
    this.list = words.map((word) => ({
      word,
      searches: word.phonemes.map((v) => ({ s: new Search(v.ids, v.lang, SPOT.frame), lang: v.lang })),
      until: 0,
      last: NEG,
    }))
  }

  get scores(): { word: string; score: number }[] {
    return this.list.map((w) => ({ word: w.word.text, score: w.last }))
  }

  frame(f: SpotFrame, onWake: (e: WakeEvent) => void): void {
    const t = this.t++
    if (f.speech !== null) this.pause.push(f.speech)
    for (const w of this.list) {
      let best: WakeEvent | null = null
      let shown = NEG
      for (const { s } of w.searches) {
        const r = s.step(f.logp, t)
        if (r.score <= NEG / 2) continue
        const quiet = this.context !== 'anywhere' && this.pause.after(r.start - SPOT.lagSteps)
        if (this.context === 'pause' && !quiet) continue
        const context = quiet ? 'pause' : 'anywhere'
        const threshold = quiet ? w.word.thresholds.pause : w.word.thresholds.anywhere
        shown = Math.max(shown, r.score)
        if (r.score >= threshold && (!best || r.score - threshold > best.score - best.threshold))
          best = { word: w.word.text, score: r.score, threshold, context, start: Math.max(0, (r.start - SPOT.lagSteps) * SPOT.frame), end: (t - SPOT.lagSteps + 1) * SPOT.frame }
      }
      w.last = shown
      if (best && t >= w.until) {
        onWake(best)
        for (const { s } of w.searches) s.reset()
        w.until = t + Math.round(REFRACTORY_S / SPOT.frame)
      }
    }
  }

  reset(): void {
    this.t = 0
    this.pause.reset()
    for (const w of this.list) {
      for (const { s } of w.searches) s.reset()
      w.until = 0
    }
  }
}
