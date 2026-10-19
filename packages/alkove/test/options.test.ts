import { readFileSync } from 'node:fs'
import { join } from 'node:path'

import { beforeAll, describe, expect, it } from 'vitest'

import { Alkove, Spot, Timbre, Voiceprint } from '../src/index'
import { WasmEngine } from '../src/core/engine'
import { processPcm } from '../src/process'
import { engineWasm, loadEngine, maxDiff, modelBytes, wav } from './helpers'

let engine: WasmEngine
beforeAll(async () => {
  engine = new WasmEngine(await loadEngine(), modelBytes)
  Alkove.configure({ modelDir: join(import.meta.dirname, '../.cache/models'), models: { engine: new Uint8Array(engineWasm()) } })
})

const rms = (x: Float32Array) => Math.sqrt(x.reduce((s, v) => s + v * v, 0) / x.length)

// each public option changes something (a typed option passed to nothing is the bug this catches)
describe('every option does something', () => {
  const mix = () => wav('mix.wav')

  it('focus.strength: 0 gives the raw back, 0.5 sits between', () => {
    const raw = mix()
    const off = processPcm(engine, raw, { focus: { strength: 0 } }).audio
    const full = processPcm(engine, raw, { focus: { strength: 1 } }).audio
    const half = processPcm(engine, raw, { focus: { strength: 0.5 } }).audio
    // the fade starts from the option's value: no ramp at the start
    expect(maxDiff(off.subarray(1000), raw.subarray(1000, raw.length))).toBeLessThan(1e-6)
    expect(maxDiff(full, off)).toBeGreaterThan(0.05)
    let err = 0
    for (let i = 1000; i < raw.length; i++) err = Math.max(err, Math.abs(half[i]! - (full[i]! + off[i]!) / 2))
    expect(err).toBeLessThan(1e-5)
  })

  it('normalize: a quiet voice comes out louder', () => {
    const quiet = mix().map((v) => v * 0.05)
    const plain = processPcm(engine, quiet, { focus: true }).audio
    const leveled = processPcm(engine, quiet, { focus: true, normalize: true }).audio
    expect(rms(leveled.subarray(32000))).toBeGreaterThan(3 * rms(plain.subarray(32000)))
  })

  it('focus.voiceprint: another output than the closest voice', async () => {
    const timbre = await Timbre.create()
    const print = await timbre.embed(wav('enroll.wav'))
    const a = processPcm(engine, mix(), { focus: true }).audio
    const b = processPcm(engine, mix(), { focus: { voiceprint: print } }).audio
    expect(maxDiff(a, b)).toBeGreaterThan(0.01)
  })

  it('cue params: a longer stopSecs joins segments, a higher confidence finds less speech', () => {
    const s = wav('speech.wav')
    const base = processPcm(engine, s, { cue: true }).speech
    const strict = processPcm(engine, s, { cue: { confidence: 0.99 } }).speech
    const total = (x: { start: number; end: number }[]) => x.reduce((n, v) => n + v.end - v.start, 0)
    expect(total(strict)).toBeLessThan(total(base))
    const late = processPcm(engine, s, { cue: { stopSecs: 2 } }).speech
    expect(late[late.length - 1]!.end).toBeGreaterThan(base[base.length - 1]!.end + 1)
  })

  it('spot.context: after a pause only, a word in mid-sentence does not count', async () => {
    const word = await Spot.compile('amendement', { lang: 'fr' })
    const s = wav('speech.wav')
    expect(processPcm(engine, s, { spot: { words: [word], context: 'anywhere' } }).wakes.length).toBe(1)
    const pause = processPcm(engine, s, { spot: { words: [word], context: 'pause' } }).wakes
    expect(pause.every((w) => w.context === 'pause')).toBe(true)
  })

  it('trace: one row per primitive step', () => {
    const r = processPcm(engine, wav('speech.wav'), { focus: true, trace: true })
    const n = wav('speech.wav').length
    expect(r.trace!.focus.length).toBeGreaterThanOrEqual(Math.floor(n / 256))
    expect(r.trace!.focus.at(-1)!.t).toBeGreaterThanOrEqual((n - 256) / 16000)
    // Cue's rows with Focus on too, one per 30 ms
    expect(r.trace!.cue.length).toBeGreaterThanOrEqual(Math.floor(n / 480) - 4)
    expect(Math.max(...r.trace!.cue.map((c) => c.speech ?? 0))).toBeGreaterThan(0.5)
  })

  it('setEnabled(false): the raw mic at the chain delay, without level or gate', () => {
    const raw = mix()
    const chain = engine.chain({ gate: true, normalize: true })
    chain.setEnabled(false)
    const out = chain.push(raw, () => {})
    chain.release()
    const d = chain.delay
    let err = 0
    for (let i = d + 2000; i < out.length; i++) err = Math.max(err, Math.abs(out[i]! - raw[i - d]!))
    expect(err).toBeLessThan(1e-4)
  })

  it('outdated words and prints: an AlkoveError for a word, null for a print', async () => {
    const word = (await Spot.compile('amendement', { lang: 'fr' })).toJSON()
    const old = { ...word, spot: 'alkove-spot-1-preview' }
    await expect(Alkove.process({ pcm: new Float32Array(16000), rate: 16000 }, { spot: { words: [old] } })).rejects.toMatchObject({ name: 'AlkoveError', code: 'word.outdated' })
    const print = (await (await Timbre.create()).embed(wav('speech.wav'))).toJSON()
    expect(Voiceprint.fromJSON({ ...print, model: 'alkove-timbre-1-preview' })).toBeNull()
    await expect(Spot.compile('amendement', { lang: 'xx' as never })).rejects.toMatchObject({ code: 'lang.unsupported' })
  })
})

describe('the public API in Node', () => {
  it('Alkove.process: WAV bytes in, the same result as the primitives', async () => {
    const bytes = new Uint8Array(readFileSync(join(import.meta.dirname, 'fixtures/speech.wav')))
    const r = await Alkove.process(bytes, { cue: true })
    expect(r.speech.length).toBe(1)
    expect(r.raw.length).toBe(wav('speech.wav').length)
    expect(r.toWav().length).toBe(44 + 2 * r.audio.length)
  })

  it('Alkove.process: any rate in, 16 kHz out', async () => {
    const s = wav('speech.wav')
    const at48 = new Float32Array(s.length * 3)
    for (let i = 0; i < at48.length; i++) at48[i] = s[Math.floor(i / 3)]!
    const r = await Alkove.process({ pcm: at48, rate: 48000 }, { cue: true })
    expect(r.raw.length).toBe(s.length)
    expect(r.speech.length).toBe(1)
  })
})

describe('a stream in blocks', () => {
  it('gives back what process gives, block by block, with absolute times', async () => {
    const { Processor } = await import('../src/process')
    const word = await Spot.compile('amendement', { lang: 'fr' })
    const pcm = wav('mix.wav')
    const opts = { focus: true, cue: true, spot: { words: [word] } } as const
    const whole = processPcm(engine, pcm, opts)
    const p = new Processor(engine, opts)
    const parts = []
    for (let i = 0; i < pcm.length; i += 7000) parts.push(p.push(pcm.subarray(i, Math.min(pcm.length, i + 7000))))
    parts.push(p.end())
    const audio = new Float32Array(pcm.length)
    let o = 0
    for (const part of parts) {
      expect(part.start).toBeCloseTo(o / 16000, 6)
      audio.set(part.audio, o)
      o += part.audio.length
    }
    expect(o).toBe(pcm.length)
    expect(maxDiff(audio, whole.audio)).toBe(0)
    expect(parts.flatMap((x) => x.speech)).toEqual(whole.speech)
    expect(parts.flatMap((x) => x.wakes)).toEqual(whole.wakes)
  })
})

describe('Alkove.stream', () => {
  it('48 kHz blocks from an async iterable: parts that cover the input once', async () => {
    const s = wav('speech.wav')
    const at48 = new Float32Array(s.length * 3)
    for (let i = 0; i < at48.length; i++) at48[i] = s[Math.floor(i / 3)]!
    async function* blocks() {
      for (let i = 0; i < at48.length; i += 4800) yield at48.subarray(i, i + 4800)
    }
    let n = 0
    const speech = []
    for await (const part of Alkove.stream(blocks(), { rate: 48000, cue: true })) {
      expect(part.start).toBeCloseTo(n / 16000, 6)
      n += part.audio.length
      speech.push(...part.speech)
    }
    expect(Math.abs(n - s.length)).toBeLessThanOrEqual(1)
    expect(speech.length).toBe(1)
  })
})
