import { type Lang, LEX } from '../manifest'
import type { Net } from './engine'
import { PHONES, SAME, toIds, toIpa } from './spot/phones'

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

export const clean = (text: string) => text.normalize('NFC').toLowerCase().replace(/’/g, "'")

export function splitWords(text: string): string[] {
  return clean(text).replace(/l·l|ŀl/g, 'l').match(/\p{L}+(?:['-]\p{L}+)*'?/gu) ?? []
}

export function checkText(text: string): Warning[] {
  const out: Warning[] = []
  if (/\p{N}/u.test(text)) out.push({ code: 'text.digits', message: 'Numbers are not read: write them in letters.' })
  const acronyms = text.match(/\b\p{Lu}{2,}\b/gu)
  if (acronyms) out.push({ code: 'text.acronym', message: 'Acronyms are read as words: write them as they sound.', words: acronyms })
  const abbreviations = text.match(/\b\p{L}{1,3}\.(?=\s|$)/gu)
  if (abbreviations) out.push({ code: 'text.abbreviation', message: 'Abbreviations are read as written.', words: abbreviations })
  return out
}

// text -> phonemes: Lex's beam search, word by word (what happens between words is left out)
export class Lex {
  static readonly langs = LEX.langs
  static readonly inventory = PHONES.map((p) => p.normalize('NFC'))

  constructor(private net: Net) {}

  word(word: string, lang: Lang): Pronunciation[] {
    const li = LEX.langs.indexOf(lang)
    if (li < 0) throw new RangeError(`lang.unsupported: ${lang}`)
    const B = LEX.beams
    const T = LEX.prefix
    const letters = new Int32Array(LEX.word)
    ;[...clean(word)].slice(0, LEX.word).forEach((c, i) => (letters[i] = c.codePointAt(0)!))
    let beams: [number[], number][] = [[[LEX.bos], 0]]
    const done: [number[], number][] = []
    for (let t = 0; t < T; t++) {
      const prefix = new Int32Array(B * T)
      for (let j = 0; j < B; j++) beams[Math.min(j, beams.length - 1)]![0].forEach((v, i) => (prefix[j * T + i] = v))
      const out = this.net.run({
        word: { data: letters, dims: [1, LEX.word] },
        lang: { data: Int32Array.of(li), dims: [1] },
        prefix: { data: prefix, dims: [B, T] },
        pos: { data: Int32Array.of(t), dims: [1] },
      }).logp!.data as Float32Array
      const cand: [number[], number][] = []
      beams.forEach(([s, sc], j) => {
        const row = out.subarray(j * LEX.vocab, (j + 1) * LEX.vocab)
        const top = [...row.keys()].sort((a, b) => row[b]! - row[a]!).slice(0, B)
        for (const i of top) (i === LEX.eos ? done : cand).push([[...s, i], sc + row[i]!])
      })
      done.sort((a, b) => b[1] - a[1])
      beams = cand.sort((a, b) => b[1] - a[1]).slice(0, B)
      if (!beams.length || (done.length >= B && done[B - 1]![1] >= beams[0]![1])) break
    }
    return done.slice(0, B).map(([s, sc]) => {
      const ids = s.slice(1, -1)
      return { ids, ipa: toIpa(ids), p: Math.exp(sc) }
    })
  }

  // a phrase: each word's pronunciations multiplied, the `max` likeliest kept; `keep` filters each word's variants
  phonemize(text: string, lang: Lang, max = LEX.beams, keep: (p: Pronunciation, i: number) => boolean = () => true): Pronunciation[] {
    let out: Pronunciation[] = [{ ids: [], ipa: '', p: 1 }]
    for (const w of splitWords(text)) {
      const vs = this.word(w, lang).filter(keep)
      out = out
        .flatMap((a) => vs.map((v) => ({ ids: [...a.ids, ...v.ids], ipa: '', p: a.p * v.p })))
        .sort((a, b) => b.p - a.p)
        .slice(0, max)
    }
    return out
      .map((v) => {
        const ids = v.ids.filter((x, i, a) => i === 0 || x !== a[i - 1])
        return { ids, ipa: toIpa(ids), p: v.p }
      })
      .filter((v) => v.ids.length)
  }

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
