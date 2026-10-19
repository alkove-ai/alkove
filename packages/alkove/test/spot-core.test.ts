import { createHash } from 'node:crypto'
import { readFileSync } from 'node:fs'
import { join } from 'node:path'

import { beforeAll, describe, expect, it } from 'vitest'

import { WasmEngine } from '../src/core/engine'
import { Lex } from '../src/core/lex'
import { receive, send, type Value } from '../src/core/native'
import { PHONES, toIds, toIpa } from '../src/core/spot/phones'
import type { Context, WordJSON } from '../src/core/spot/spot'
import type { Alk } from '../src/engine/alk'
import { coreOptions, type ProcessOptions } from '../src/process'
import { loadHarness, modelBytes, noise, wav } from './helpers'

// Spot and Lex in the core: against JavaScript itself and the main thread's phones.ts where they remain, else against digests of what the TypeScript gave

let alk: Alk
let engine: WasmEngine
beforeAll(async () => {
  alk = await loadHarness()
  engine = new WasmEngine(alk, modelBytes)
}, 300_000)

const core = (v: Record<string, unknown>) => {
  send(alk, v)
  return receive(alk, (alk.x as unknown as { alk_h_value(): number }).alk_h_value())
}

// a run's outputs in one sha256, keys sorted: what the TypeScript gave before the core replaced it
const GOLDEN: Record<string, string> = JSON.parse(readFileSync(join(import.meta.dirname, 'fixtures/core.json'), 'utf8'))
const canon = (v: unknown): unknown =>
  ArrayBuffer.isView(v)
    ? Array.from(v as Float32Array)
    : Array.isArray(v)
      ? v.map(canon)
      : v && typeof v === 'object'
        ? Object.fromEntries(Object.keys(v).sort().map((k) => [k, canon((v as Record<string, unknown>)[k])]))
        : v
function digest(key: string, outputs: unknown[]): void {
  expect(createHash('sha256').update(JSON.stringify(canon(outputs))).digest('hex'), key).toBe(GOLDEN[key])
}

// what the TypeScript's clean, splitWords and checkText did
const clean = (t: string) => t.normalize('NFC').toLowerCase().replace(/’/g, "'")
const splitWords = (t: string) => clean(t).replace(/l·l|ŀl/g, 'l').match(/\p{L}+(?:['-]\p{L}+)*'?/gu) ?? []
function checkText(t: string) {
  const out: Record<string, unknown>[] = []
  if (/\p{N}/u.test(t)) out.push({ code: 'text.digits', message: 'Numbers are not read: write them in letters.' })
  const acronyms = t.match(/\b\p{Lu}{2,}\b/gu)
  if (acronyms) out.push({ code: 'text.acronym', message: 'Acronyms are read as words: write them as they sound.', words: acronyms })
  const abbreviations = t.match(/\b\p{L}{1,3}\.(?=\s|$)/gu)
  if (abbreviations) out.push({ code: 'text.abbreviation', message: 'Abbreviations are read as written.', words: abbreviations })
  return out
}
const NEG = -1e9

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
      const got = core({ op: 'search', ids, lang, frames: frames.map((f) => Array.from(f)) }) as number[][]
      expect(got.some(([s]) => s! > NEG / 2)).toBe(true)
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
      all.push(engine.lex().phonemize(words, lang))
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
      let got: unknown
      try {
        const r = engine.compile(text, lang, chain)
        got = { word: r.word, warnings: (r.warnings as { code: string }[]).filter((x) => x.code !== 'text.script') }
      } catch (e) {
        got = { error: String(e) }
      }
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
  ].map(([t, l]) => engine.compile(t as string, l as string[]).word as unknown as WordJSON)

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
      const native = engine.processor(coreOptions(opts))
      const r = rng(seed * 7)
      let wakes = 0
      const parts: unknown[] = []
      for (let i = 0; i < pcm.length; ) {
        const n = Math.min(pcm.length - i, 1 + Math.floor(r() * 6000))
        const b = native.push(pcm.subarray(i, i + n))
        parts.push(b)
        wakes += (b.wakes as unknown[]).length
        i += n
        await breathe()
      }
      parts.push(native.end())
      native.release()
      expect(wakes).toBeGreaterThan(0)
      digest(`spot: process, ${name}`, parts)
    }, 120_000)
})

describe('listening and enrolling in real time, as the worker did', () => {
  it('Spot and its words frame by frame: speech head, scores, wakes; words replaced on the way', () => {
    const ws = words()
    const pcm = stream(15, 40)
    const native = engine.listener(ws as unknown as Value[], 'both')
    const r = rng(16)
    const frames: unknown[] = []
    for (let i = 0; i < pcm.length; ) {
      const n = Math.min(pcm.length - i, 256 * (1 + Math.floor(r() * 8)))
      if (i > 0 && r() < 0.01) {
        const some = ws.slice(0, 1 + Math.floor(r() * ws.length))
        const context = (['pause', 'anywhere', 'both'] as const)[Math.floor(r() * 3)]!
        native.setWords(some as unknown as Value[], context)
      }
      frames.push(native.push(pcm.subarray(i, i + n)))
      i += n
    }
    native.release()
    digest('spot: listening frame by frame', frames)
  })

  it('enrolling: the takes as they come, then the word and its warnings, or the error', () => {
    const typed = engine.compile('amendement', ['fr']).word
    const speech = wav('speech.wav')
    const quiet = new Float32Array(16000)
    const parts = [speech, quiet, stream(17, 6), speech, quiet, speech, quiet, speech]
    const all: unknown[] = []
    for (const takes of [3, 2, 9]) {
      const native = engine.enroller(typed, takes, 'focus')
      for (const p of parts) all.push(native.push(p))
      try {
        all.push(native.result())
      } catch (e) {
        all.push({ error: String(e) })
      }
      native.release()
    }
    digest('spot: enrolling', all)
    const none = engine.enroller(typed, 3, 'raw')
    none.push(new Float32Array(32000))
    expect(() => none.result()).toThrow('word.no-take')
  })
})
