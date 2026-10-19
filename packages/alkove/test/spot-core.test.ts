import { createHash } from 'node:crypto'
import { readFileSync, writeFileSync } from 'node:fs'
import { join } from 'node:path'

import { afterAll, beforeAll, describe, expect, it } from 'vitest'

import { WasmEngine } from '../src/core/engine'
import { checkText, clean, Lex, splitWords } from '../src/core/lex'
import { compileNative, receive, send } from '../src/core/native'
import { Enroller, type Take } from '../src/core/spot/enroll'
import { PHONES, toIds, toIpa } from '../src/core/spot/phones'
import { NEG, Search } from '../src/core/spot/search'
import { compileWord, type Context, Spot, Spotter, type Word } from '../src/core/spot/spot'
import type { Alk } from '../src/engine/alk'
import { coreOptions, Processor, type ProcessOptions } from '../src/process'
import { loadHarness, modelBytes, noise, wav } from './helpers'

// Spot and Lex in the core against the TypeScript they replace, on the same wasm engine

let alk: Alk
let engine: WasmEngine
let lex: () => Lex
beforeAll(async () => {
  alk = await loadHarness()
  engine = new WasmEngine(alk, modelBytes)
  let l: Lex | null = null
  lex = () => (l ??= new Lex(engine.net('lex', 'encoder'), engine.net('lex', 'decoder')))
}, 300_000)

const core = (v: Record<string, unknown>) => {
  send(alk, v)
  return receive(alk, (alk.x as unknown as { alk_h_value(): number }).alk_h_value())
}

// a run's outputs in one sha256, keys sorted: what the TypeScript gave, kept once it is gone
const goldenPath = join(import.meta.dirname, 'fixtures/core.json')
const GOLDEN: Record<string, string> = JSON.parse(readFileSync(goldenPath, 'utf8'))
const made: Record<string, string> = {}
const canon = (v: unknown): unknown =>
  ArrayBuffer.isView(v)
    ? Array.from(v as Float32Array)
    : Array.isArray(v)
      ? v.map(canon)
      : v && typeof v === 'object'
        ? Object.fromEntries(Object.keys(v).sort().map((k) => [k, canon((v as Record<string, unknown>)[k])]))
        : v
function digest(key: string, outputs: unknown[]): void {
  const d = createHash('sha256').update(JSON.stringify(canon(outputs))).digest('hex')
  if (GOLDEN[key]) expect(d, key).toBe(GOLDEN[key])
  else made[key] = d
}
afterAll(() => {
  if (Object.keys(made).length) writeFileSync(goldenPath, JSON.stringify({ ...GOLDEN, ...made }, null, 1) + '\n')
})

const rng = (seed: number) => {
  let s = seed >>> 0
  return () => (s = (s * 1664525 + 1013904223) >>> 0) / 2 ** 32
}
const breathe = () => new Promise((r) => setImmediate(r))

// strings drawn from what Lex, IPA and the regular expressions meet: letters of several scripts, marks, quotes, spaces of every kind
function strings(seed: number, n: number, alphabet: string[]): string[] {
  const r = rng(seed)
  return Array.from({ length: n }, () => Array.from({ length: 1 + Math.floor(r() * 14) }, () => alphabet[Math.floor(r() * alphabet.length)]!).join(''))
}
const TEXT = [...'aAzZéÉèàâçÇñÑüÜœŒßŀĿl·.,;:!?-\'’/()0123456789 \t\n  　_ΣσςαάΆΏ̧̀̃̈ДдЖж中文ひİıǅǄǆẞÅΩﬁ', 'l·l', 'ŀl', 'é', 'Å', '\u{1d400}', '\u{10400}', 'Mr.', 'etc.', 'NASA', 'ONU', 'J.-P.']
const COVERED = [
  [0, 0x3ff],
  [0x1d00, 0x1fff],
  [0x2000, 0x214f],
  [0x2c60, 0x2c7f],
  [0xa720, 0xa7ff],
  [0xab30, 0xab6f],
] as const
const IPA = [...new Set([...PHONES, ...'ᵻɚɝʝɟcɫɬχɦʋɱɻʀgɘɵʏɯɤqɕʑçʔɒɸʙãõẽĩũɐxθðhβɣʎɨ', 'œ̃', 'ʊ̃', 'ɐ̃', ...'ˈˌːˑ‿-.|ʲʰʷ\' \t()‍', '̃', '́', '̩', '˞', 'Q', '9', 'é'])]

describe('text, as the TypeScript read it', () => {
  it('\\p{L}, \\p{Lu}, \\p{M}, \\p{N} and \\s on every code point', () => {
    const tests: [string, RegExp][] = [
      ['letter', /\p{L}/u],
      ['upper', /\p{Lu}/u],
      ['mark', /\p{M}/u],
      ['number', /\p{N}/u],
      ['space', /\s/u],
    ]
    for (let from = 0; from < 0x110000; from += 0x8000) {
      const cps = Array.from({ length: 0x8000 }, (_, i) => from + i).filter((c) => c < 0xd800 || c > 0xdfff)
      for (const [op, re] of tests) {
        const got = core({ op, cps }) as boolean[]
        const want = cps.map((c) => re.test(String.fromCodePoint(c)))
        const bad = cps.filter((_, i) => got[i] !== want[i])
        expect(bad.map((c) => `${op} U+${c.toString(16)}`)).toEqual([])
      }
    }
  })

  it('toLowerCase on every code point, NFC and NFD on every code point of the Latin, IPA and Greek blocks', () => {
    const lower: number[] = []
    for (let c = 0; c < 0x110000; c++) if (c < 0xd800 || c > 0xdfff) lower.push(c)
    for (let i = 0; i < lower.length; i += 4096) {
      const s = String.fromCodePoint(...lower.slice(i, i + 4096))
      expect(core({ op: 'lower', s })).toBe(s.toLowerCase())
    }
    for (const [a, b] of COVERED)
      for (let c = a!; c <= b!; c++) {
        const s = String.fromCodePoint(c)
        for (const op of ['nfd', 'nfc'] as const) expect(core({ op, s }), `${op} U+${c.toString(16)}`).toBe(s.normalize(op.toUpperCase() as 'NFC'))
      }
  })

  it('NFC, NFD, sigma and the regular expressions on drawn strings', () => {
    let normalized = 0
    for (const s of strings(1, 6000, TEXT)) {
      // the core normalizes the Latin, IPA and Greek blocks only (decided: Lex reads no other script)
      if ([...s, ...s.normalize('NFD')].every((c) => COVERED.some(([a, b]) => c.codePointAt(0)! >= a && c.codePointAt(0)! <= b))) {
        normalized++
        expect(core({ op: 'nfd', s }), s).toBe(s.normalize('NFD'))
        expect(core({ op: 'nfc', s }), s).toBe(s.normalize('NFC'))
        expect(core({ op: 'clean', s }), s).toBe(clean(s))
        expect(core({ op: 'split', s }), s).toEqual(splitWords(s))
      }
      expect(core({ op: 'lower', s }), s).toBe(s.toLowerCase())
      expect(core({ op: 'acronyms', s }), s).toEqual(s.match(/\b\p{Lu}{2,}\b/gu) ?? [])
      expect(core({ op: 'abbreviations', s }), s).toEqual(s.match(/\b\p{L}{1,3}\.(?=\s|$)/gu) ?? [])
      expect((core({ op: 'check', s }) as { code: string }[]).filter((w) => w.code !== 'text.script'), s).toEqual(checkText(s))
      const typed = /^\s*\/(.+)\/\s*$/.exec(s)?.[1] ?? null
      expect(core({ op: 'slashed', s }), s).toBe(typed)
    }
    expect(normalized).toBeGreaterThan(1000)
  })
})

describe('phonemes, as the TypeScript wrote them', () => {
  it('toIds in every language, toIpa, distance', () => {
    const langs = ['', 'fr', 'en', 'es', 'it', 'pt', 'ca', 'ro', 'xx']
    for (const [k, s] of strings(2, 8000, IPA).entries()) {
      const lang = langs[k % langs.length]!
      expect(core({ op: 'toIds', s, lang }), `${s} ${lang}`).toEqual(toIds(s, lang))
    }
    const r = rng(3)
    for (let k = 0; k < 3000; k++) {
      const ids = Array.from({ length: Math.floor(r() * 9) }, () => Math.floor(r() * 61))
      const b = Array.from({ length: Math.floor(r() * 9) }, () => 1 + Math.floor(r() * 58))
      expect(core({ op: 'toIpa', ids })).toBe(toIpa(ids))
      expect(core({ op: 'distance', a: ids.filter((i) => i >= 1 && i <= 58), b })).toBe(Lex.distance(ids.filter((i) => i >= 1 && i <= 58), b))
    }
    for (const [s, t] of [['/amɑ̃dmɑ̃/', 'amɑ̃dəmɑ̃'], ['kamij', '/kamil/'], ['//', 'a'], ['ʁ', 'r']]) expect(core({ op: 'distanceIpa', s, t })).toBe(Lex.distance(s!, t!))
  })

  it('the keyword search, frame by frame', () => {
    const r = rng(4)
    const runs: unknown[] = []
    for (let k = 0; k < 120; k++) {
      const ids = Array.from({ length: 1 + Math.floor(r() * 9) }, () => 1 + Math.floor(r() * 58))
      // French schwa on, so its optional paths are taken
      if (k % 3 === 0) ids.splice(Math.floor(r() * ids.length), 0, 7)
      const lang = k % 2 ? 'fr' : 'en'
      const frames = Array.from({ length: 150 }, () => {
        const target = r() < 0.6 ? ids[Math.floor(r() * ids.length)]! : 0
        const logits = Array.from({ length: 59 }, (_, c) => (c === target ? 6 : 0) + 3 * r())
        const m = Math.max(...logits)
        const z = Math.log(logits.reduce((a, x) => a + Math.exp(x - m), 0)) + m
        return Float32Array.from(logits, (x) => x - z)
      })
      const ts = new Search(ids, lang, 0.03)
      const want = frames.map((f, t) => {
        const x = ts.step(f, t)
        return [x.score, x.start]
      })
      const got = core({ op: 'search', ids, lang, frames: frames.map((f) => Array.from(f)) }) as number[][]
      expect(got).toEqual(want)
      expect(want.some(([s]) => s! > NEG / 2)).toBe(true)
      runs.push(got)
    }
    digest('spot: the keyword search', runs)
  })
})

const WORDS: Record<string, string[]> = {
  fr: ["amendement", 'composition', 'Haut conseil', "aujourd'hui", 'porte-monnaie', 'Camille', 'Ŀluís', 'inégalités territoriales', 'santé', 'œuvre', 'Noël', 'ça', 'M. Dupont', 'ONU', 'deuxième', 'ok luciole'],
  en: ['hey jarvis', 'computer', 'knight', 'through', "don't", 'well-being', 'Ok Nabu', 'colonel', 'NASA', 'café'],
  es: ['oye', 'mañana', 'ñandú', 'güero', 'cigüeña', 'hola amigo'],
  it: ['ciao', 'gnocchi', 'perché', 'sciarpa', 'città'],
  pt: ['olá', 'pão', 'coração', 'lhama', 'nação'],
  ca: ['col·legi', 'paral·lel', 'Llorenç', 'aigua'],
  ro: ['bună', 'știință', 'țară', 'mulțumesc'],
}

describe('Lex and compiling, as the TypeScript did', () => {
  it('phonemize in the seven languages', () => {
    const all: unknown[] = []
    for (const [lang, words] of Object.entries(WORDS)) {
      const n = engine.lex().phonemize(words, lang)
      expect(n, lang).toEqual(words.map((w) => lex().phonemize(w, lang as 'fr')))
      all.push(n)
    }
    digest('lex: phonemize in the seven languages', all)
    expect(() => engine.lex().phonemize(['x'], 'de')).toThrow(RangeError)
  })

  it('compile: typed words in one or several languages, phonemes between slashes, the errors', () => {
    const cases: [string, string[], string | undefined][] = [
      ...Object.entries(WORDS).flatMap(([lang, ws]) => ws.map((w) => [w, [lang], undefined] as [string, string[], undefined])),
      ['Camille', ['fr', 'en'], 'focus'],
      ['luciole', ['fr', 'es', 'it'], undefined],
      ['/kamij/', ['fr'], undefined],
      [' /amɑ̃d(ə)mɑ̃/ ', [], undefined],
      ['/œ̃ ʁɛ̃/', ['fr'], 'raw'],
      ['/Q/', ['fr'], undefined],
      ['camille', [], undefined],
      ['123', ['fr'], undefined],
    ]
    const all: unknown[] = []
    for (const [text, lang, chain] of cases) {
      let want: unknown
      try {
        const w = compileWord(text, { lang: lang as 'fr'[], chain: chain as 'raw' }, lex)
        want = { word: w.toJSON(), warnings: w.warnings }
      } catch (e) {
        want = { error: String(e) }
      }
      let got: unknown
      try {
        const r = compileNative(alk, engine.lex(), text, lang, chain)
        got = { word: r.word, warnings: (r.warnings as { code: string }[]).filter((x) => x.code !== 'text.script') }
      } catch (e) {
        got = { error: String(e) }
      }
      expect(got, text).toEqual(want)
      all.push(got)
    }
    digest('lex: compile', all)
  })
})

// voices of the test files at any level, silences, noise
function stream(seed: number, seconds: number): Float32Array {
  const r = rng(seed)
  const voices = [wav('speech.wav'), wav('mix.wav'), wav('enroll.wav')]
  const out = new Float32Array(Math.round(seconds * 16000))
  let o = 0
  while (o < out.length) {
    const kind = r()
    const len = Math.min(out.length - o, Math.floor((0.3 + r() * 4) * 16000))
    if (kind < 0.6) {
      const v = voices[Math.floor(r() * voices.length)]!
      const from = r() < 0.5 ? 0 : Math.floor(r() * v.length)
      const gain = 0.2 + r() * 1.5
      for (let i = 0; i < len; i++) out[o + i] = v[(from + i) % v.length]! * gain
    } else if (kind < 0.85) out.set(noise(len, seed + o, 1e-5), o)
    else out.set(noise(len, seed + o, 0.002 + r() * 0.02), o)
    o += len
  }
  return out
}

const words = () =>
  [
    ['amendement', ['fr']],
    ['cet amendement', ['fr']],
    ['le dernier constat', ['fr']],
    ['composition', ['fr']],
    ['ministre', ['fr', 'en']],
    ['territoriales', ['fr']],
    ['/kɔ̃sɛj/', ['fr']],
    ['hey jarvis', ['en']],
  ].map(([t, l]) => compileWord(t as string, { lang: l as 'fr'[] }, lex))

describe('Spot in process, as the TypeScript gave it', () => {
  const cases: [string, number, ProcessOptions][] = [
    ['raw, anywhere, traced', 11, { cue: true, trace: true }],
    ['raw, after a pause', 12, {}],
    ['behind Focus, both, traced', 13, { focus: true, trace: true }],
    ['behind Focus with Cue, pause, normalize', 14, { focus: true, cue: true, normalize: true }],
  ]
  const contexts: Record<string, Context> = { 11: 'anywhere', 12: 'pause', 13: 'both', 14: 'pause' }
  for (const [name, seed, base] of cases)
    it(name, async () => {
      const opts: ProcessOptions = { ...base, spot: { words: words(), context: contexts[seed] } }
      const pcm = stream(seed, 40)
      const ts = new Processor(engine, opts)
      const native = engine.processor(coreOptions(opts))
      const r = rng(seed * 7)
      let wakes = 0
      const parts: unknown[] = []
      for (let i = 0; i < pcm.length; ) {
        const n = Math.min(pcm.length - i, 1 + Math.floor(r() * 6000))
        const a = ts.push(pcm.subarray(i, i + n))
        const b = native.push(pcm.subarray(i, i + n))
        expect(b).toEqual(a)
        parts.push(b)
        wakes += a.wakes.length
        i += n
        await breathe()
      }
      const last = native.end()
      expect(last).toEqual(ts.end())
      parts.push(last)
      native.release()
      expect(wakes).toBeGreaterThan(0)
      digest(`spot: process, ${name}`, parts)
    }, 120_000)
})

describe('listening and enrolling in real time, as the worker did', () => {
  it('Spot and its words frame by frame: speech head, scores, wakes; words replaced on the way', () => {
    const ws = words()
    const pcm = stream(15, 40)
    const ts = { spot: new Spot(engine.stream('spot')), spotter: new Spotter(ws, 'both') }
    const native = engine.listener(ws.map((w) => w.toJSON() as unknown as null), 'both')
    const r = rng(16)
    const frames: unknown[] = []
    for (let i = 0; i < pcm.length; ) {
      const n = Math.min(pcm.length - i, 256 * (1 + Math.floor(r() * 8)))
      if (i > 0 && r() < 0.01) {
        const some = ws.slice(0, 1 + Math.floor(r() * ws.length))
        const context = (['pause', 'anywhere', 'both'] as const)[Math.floor(r() * 3)]!
        ts.spotter = new Spotter(some, context)
        native.setWords(some.map((w) => w.toJSON() as unknown as null), context)
      }
      const want: unknown[] = []
      ts.spot.push(pcm.subarray(i, i + n), (f, step) => {
        const wakes: unknown[] = []
        ts.spotter.frame(f, (e) => wakes.push(e))
        want.push([step, f.speech, ts.spotter.scores.map((s) => s.score), wakes])
      })
      const got = native.push(pcm.subarray(i, i + n))
      expect(got).toEqual(want)
      frames.push(got)
      i += n
    }
    native.release()
    digest('spot: listening frame by frame', frames)
  })

  it('enrolling: the takes as they come, then the word and its warnings, or the error', () => {
    const typed = compileWord('amendement', { lang: 'fr' }, lex)
    const speech = wav('speech.wav')
    const quiet = new Float32Array(16000)
    const parts = [speech, quiet, stream(17, 6), speech, quiet, speech, quiet, speech]
    const all: unknown[] = []
    for (const takes of [3, 2, 9]) {
      const ts = new Enroller(typed.text, typed.lang, typed.phonemes, 'focus', { takes, k: 0.8, margin: 1, detect: -2.2, spread: 1 })
      const spot = new Spot(engine.stream('spot'))
      const native = engine.enroller(typed.toJSON() as unknown as null, takes, 'focus')
      for (const p of parts) {
        const seen: Take[] = []
        spot.push(p, (f) => ts.frame(f, (t) => seen.push(t)))
        const got = native.push(p)
        expect(got).toEqual({ takes: seen, done: ts.done })
        all.push(got)
      }
      const res = (fn: () => { word: unknown; takes: unknown; warnings: unknown }) => {
        try {
          return fn()
        } catch (e) {
          return { error: String(e) }
        }
      }
      const result = res(() => native.result())
      expect(result).toEqual(
        res(() => {
          const x = ts.result()
          return { word: x.word.toJSON(), takes: x.takes, warnings: x.warnings }
        }),
      )
      all.push(result)
      native.release()
    }
    digest('spot: enrolling', all)
    const none = engine.enroller(typed.toJSON() as unknown as null, 3, 'raw')
    none.push(new Float32Array(32000))
    expect(() => none.result()).toThrow('word.no-take')
  })
})

export type { Word }
