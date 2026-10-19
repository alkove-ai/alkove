import { LEX } from '../manifest'
import { PHONES, SAME, toIds } from './spot/phones'

export interface Pronunciation {
  ipa: string
  ids: number[]
  p: number
}

export interface Warning {
  code: string
  message: string
  [key: string]: unknown
}

// what the main thread knows of Lex without running it: its languages, its phonemes, how far two pronunciations are (the beam search is in the core)
export class Lex {
  static readonly langs = LEX.langs
  static readonly inventory = PHONES.map((p) => p.normalize('NFC'))

  // 0 = same sound; a substitution inside a class of interchangeable phonemes costs half
  static distance(a: string | number[], b: string | number[]): number {
    const x = typeof a === 'string' ? (toIds(a.replace(/^\/|\/$/g, '')) ?? []) : a
    const y = typeof b === 'string' ? (toIds(b.replace(/^\/|\/$/g, '')) ?? []) : b
    if (!x.length || !y.length) return x.length === y.length ? 0 : 1
    let prev = Array.from({ length: y.length + 1 }, (_, j) => j)
    for (let i = 1; i <= x.length; i++) {
      const row = [i]
      for (let j = 1; j <= y.length; j++) {
        const sub = x[i - 1] === y[j - 1] ? 0 : SAME[x[i - 1]! - 1]!.includes(y[j - 1]!) ? 0.5 : 1
        row.push(Math.min(prev[j]! + 1, row[j - 1]! + 1, prev[j - 1]! + sub))
      }
      prev = row
    }
    return prev[y.length]! / Math.max(x.length, y.length)
  }
}
