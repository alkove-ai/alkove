import { AlkoveError } from '../../errors'
import { type Lang, modelId } from '../../manifest'
import type { Warning } from '../lex'
import { toIds } from './phones'

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
      spot: modelId('spot'),
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
    if (j.spot !== modelId('spot')) return null
    const phonemes = j.phonemes.map((v) => ({ ...v, ids: toIds(v.ipa, v.lang) ?? [] })).filter((v) => v.ids.length)
    return new Word(j.text, j.lang, phonemes, j.thresholds, j.calibrated, j.chain)
  }
}

export const wordOf = (w: Word | WordJSON | string): Word => {
  if (w instanceof Word) return w
  const word = Word.fromJSON(w)
  if (!word) throw new AlkoveError('word.outdated', `Compiled for another Spot: compile "${typeof w === 'string' ? JSON.parse(w).text : w.text}" again.`)
  return word
}

export interface CompileOptions {
  lang?: Lang | Lang[]
  chain?: Chain
}

// one time the user said the word while enrolling it: its best score, whether it counts, where, and the pronunciation that won
export interface Take {
  score: number
  ok: boolean
  start: number
  end: number
  pronunciation: string
}

export interface WakeEvent {
  word: string
  score: number
  threshold: number
  context: 'pause' | 'anywhere'
  start: number
  end: number
  // a wake listener's: the word's raw audio (16 kHz), and with onlyMyVoice the likeness to the print
  audio?: Float32Array
  voice?: number
}
