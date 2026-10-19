import { align, expect, fromB64, nodeAlkove, open, residualDb, test, wavPcm } from './fixtures'

const maxDiff = (a: Float32Array, b: Float32Array) => {
  let d = 0
  for (let i = 0; i < a.length; i++) d = Math.max(d, Math.abs(a[i]! - b[i]!))
  return a.length === b.length ? d : Infinity
}

const close = (a: { start: number; end: number }[], b: { start: number; end: number }[], tol: number) => {
  expect(a.length).toBe(b.length)
  a.forEach((x, i) => {
    expect(Math.abs(x.start - b[i]!.start)).toBeLessThanOrEqual(tol)
    expect(Math.abs(x.end - b[i]!.end)).toBeLessThanOrEqual(tol)
  })
}

test.describe('Alkove.process in the background worker', () => {
  test('gives what Node gives: speech, words, audio', async ({ page, origin }) => {
    const { Alkove, Spot } = await nodeAlkove()
    const word = await Spot.compile('amendement', { lang: 'fr' })
    const fs = await import('node:fs')
    const { join } = await import('node:path')
    const file = (n: string) => new Uint8Array(fs.readFileSync(join(import.meta.dirname, '../fixtures', n)))
    const nSpeech = await Alkove.process(file('speech.wav'), { cue: true, spot: { words: [word] } })
    const nMix = await Alkove.process(file('mix.wav'), { focus: true, cue: true, spot: { words: [word] } })

    await open(page, origin)
    const b = await page.evaluate(async () => {
      const { sdk, bytes, b64 } = window.lib
      const word = await sdk.Spot.compile('amendement', { lang: 'fr' })
      const s = await sdk.Alkove.process(await bytes('/fixtures/speech.wav'), { cue: true, spot: { words: [word] } })
      const m = await sdk.Alkove.process(await bytes('/fixtures/mix.wav'), { focus: true, cue: true, spot: { words: [word] } })
      const strip = (w: any[]) => w.map(({ audio: _, ...x }) => x)
      return { word: word.toJSON(), speech: { speech: s.speech, wakes: strip(s.wakes), audio: b64(s.audio) }, mix: { speech: m.speech, wakes: strip(m.wakes), audio: b64(m.audio), raw: b64(m.raw) } }
    })

    expect(b.word).toEqual(JSON.parse(JSON.stringify(word.toJSON())))
    expect(nSpeech.speech.length).toBe(1)
    close(b.speech.speech, nSpeech.speech, 0)
    close(b.mix.speech, nMix.speech, 0)
    expect(b.speech.wakes.map((w: any) => w.word)).toEqual(['amendement'])
    for (const [bw, nw] of [
      [b.speech.wakes, nSpeech.wakes],
      [b.mix.wakes, nMix.wakes],
    ] as const) {
      expect(bw.length).toBe(nw.length)
      bw.forEach((w: any, i: number) => {
        expect(w.word).toBe(nw[i].word)
        expect(w.start).toBe(nw[i].start)
        expect(w.score).toBe(nw[i].score)
      })
    }
    expect(maxDiff(fromB64(b.speech.audio), nSpeech.audio)).toBe(0)
    expect(maxDiff(fromB64(b.mix.raw), wavPcm('mix.wav'))).toBeLessThan(1e-6)
    // the same wasm engine: the processed audio to the bit of Node's
    expect(maxDiff(fromB64(b.mix.audio), nMix.audio)).toBe(0)
    expect(maxDiff(nMix.audio, nMix.raw)).toBeGreaterThan(0.05)
  })

  test('takes a Blob, an ArrayBuffer, an AudioBuffer and PCM at any rate', async ({ page, origin }) => {
    await open(page, origin)
    const r: any = await page.evaluate(async () => {
      const { sdk, bytes, b64 } = window.lib
      const u8 = await bytes('/fixtures/speech.wav')
      const run = async (input: unknown) => {
        const x = await sdk.Alkove.process(input, { cue: true })
        return { speech: x.speech, raw: b64(x.raw) }
      }
      const audioBuffer = await new OfflineAudioContext(1, 1, 44100).decodeAudioData(u8.slice().buffer)
      // the file played into a 48 kHz context: the browser's own resampler
      const off = new OfflineAudioContext(1, Math.ceil(audioBuffer.duration * 48000), 48000)
      const src = off.createBufferSource()
      src.buffer = audioBuffer
      src.connect(off.destination)
      src.start()
      const at48 = (await off.startRendering()).getChannelData(0)
      return {
        u8: await run(u8),
        blob: await run(new Blob([u8], { type: 'audio/wav' })),
        buffer: await run(u8.slice().buffer),
        audioBuffer: await run(audioBuffer),
        audioBufferRate: audioBuffer.sampleRate,
        pcm48: await run({ pcm: at48, rate: 48000 }),
      }
    })
    const wav = wavPcm('speech.wav')
    expect(r.u8.speech.length).toBe(1)
    expect(maxDiff(fromB64(r.u8.raw), wav)).toBeLessThan(1e-6)
    expect(r.blob).toEqual(r.u8)
    expect(r.buffer).toEqual(r.u8)
    expect(r.audioBufferRate).toBe(44100)
    // resampled by the browser then back to 16 kHz by the SDK: the same sound on the same samples
    for (const x of [r.audioBuffer, r.pcm48]) {
      const raw = fromB64(x.raw)
      expect(raw.length).toBe(wav.length)
      const a = align(wav, raw, 8000, 64000, -40, 40)
      expect(a.lag).toBe(0)
      expect(a.corr).toBeGreaterThan(0.9)
      expect(x.speech.length).toBe(1)
      expect(Math.abs(x.speech[0].end - r.u8.speech[0].end)).toBeLessThanOrEqual(0.06)
    }
    expect(residualDb(wav, fromB64(r.pcm48.raw), 8000, 64000, 0)).toBeLessThan(-20)
  })

  test('a 44.1 kHz AudioBuffer comes back on the same samples, without a sub-sample shift', async ({ page, origin }) => {
    await open(page, origin)
    const raw = fromB64(
      await page.evaluate(async () => {
        const { sdk, bytes, b64 } = window.lib
        const buf = await new OfflineAudioContext(1, 1, 44100).decodeAudioData((await bytes('/fixtures/speech.wav')).buffer)
        return b64((await sdk.Alkove.process(buf, {})).raw)
      }),
    )
    // at 48 kHz (a whole ratio) the same round trip gives -25 dB; here -9 dB, all of it from the shift
    expect(residualDb(wavPcm('speech.wav'), raw, 8000, 64000, 0)).toBeLessThan(-20)
  })

  // the options cross to the worker as a message: one dropped on the way does nothing only here
  test('every option reaches the worker and changes something', async ({ page, origin, browserName }) => {
    test.slow(browserName === 'firefox', 'Firefox runs wasm on its baseline compiler under Playwright')
    await open(page, origin)
    const r: any = await page.evaluate(async () => {
      const { sdk, bytes, fixture, rms } = window.lib
      const P = sdk.Alkove.process
      const mix = await bytes('/fixtures/mix.wav')
      const speech = await bytes('/fixtures/speech.wav')
      const diff = (a: Float32Array, b: Float32Array, from = 1000) => {
        let d = 0
        for (let i = from; i < a.length; i++) d = Math.max(d, Math.abs(a[i]! - b[i]!))
        return d
      }
      const full = (await P(mix, { focus: { strength: 1 } })).audio
      const off = await P(mix, { focus: { strength: 0 } })
      const half = (await P(mix, { focus: { strength: 0.5 } })).audio
      let mid = 0
      for (let i = 1000; i < full.length; i++) mid = Math.max(mid, Math.abs(half[i]! - (full[i]! + off.audio[i]!) / 2))

      const { pcm } = await fixture('mix.wav')
      const quiet = { pcm: pcm.map((v: number) => v * 0.05), rate: 16000 }
      const plain = (await P(quiet, { focus: true })).audio
      const leveled = (await P(quiet, { focus: true, normalize: true })).audio

      const timbre = await sdk.Timbre.create()
      const print = await timbre.embed((await fixture('enroll.wav')).pcm)
      const printed = (await P(mix, { focus: { voiceprint: print } })).audio
      const printedJson = (await P(mix, { focus: { voiceprint: JSON.stringify(print.toJSON()) } })).audio

      const total = (x: { start: number; end: number }[]) => x.reduce((n, v) => n + v.end - v.start, 0)
      const base = (await P(speech, { cue: true })).speech
      const strict = (await P(speech, { cue: { confidence: 0.99 } })).speech
      const late = (await P(speech, { cue: { stopSecs: 2 } })).speech

      const word = await sdk.Spot.compile('amendement', { lang: 'fr' })
      const anywhere = (await P(speech, { spot: { words: [word], context: 'anywhere' } })).wakes
      const pause = (await P(speech, { spot: { words: [word], context: 'pause' } })).wakes

      const traced = await P(speech, { focus: true, trace: true })
      const untraced = await P(speech, { focus: true })

      const progress: { done: number; total: number }[] = []
      await P(speech, { cue: true, onProgress: (p: { done: number; total: number }) => progress.push(p) })
      const ac = new AbortController()
      ac.abort()
      const aborted = await P(speech, { cue: true, signal: ac.signal }).then(
        () => 'resolved',
        (e: Error) => e.name,
      )
      return {
        strength: { offVsRaw: diff(off.audio, off.raw), fullVsOff: diff(full, off.audio), mid },
        normalize: { plain: rms(plain, 32000), leveled: rms(leveled, 32000) },
        voiceprint: { vsClosest: diff(printed, full, 0), jsonVsObject: diff(printedJson, printed, 0) },
        cue: { base: total(base), strict: total(strict), baseEnd: base.at(-1).end, lateEnd: late.at(-1).end },
        context: { anywhere: anywhere.length, pause: pause.map((w: any) => w.context) },
        trace: { rows: traced.trace?.focus.length ?? 0, none: untraced.trace === undefined },
        progress,
        aborted,
      }
    })
    expect(r.strength.offVsRaw).toBeLessThan(1e-6)
    expect(r.strength.fullVsOff).toBeGreaterThan(0.05)
    expect(r.strength.mid).toBeLessThan(1e-5)
    expect(r.normalize.leveled).toBeGreaterThan(3 * r.normalize.plain)
    expect(r.voiceprint.vsClosest).toBeGreaterThan(0.01)
    expect(r.voiceprint.jsonVsObject).toBe(0)
    expect(r.cue.strict).toBeLessThan(r.cue.base)
    expect(r.cue.lateEnd).toBeGreaterThan(r.cue.baseEnd + 1)
    expect(r.context.anywhere).toBe(1)
    expect(r.context.pause.every((c: string) => c === 'pause')).toBe(true)
    expect(r.trace.rows).toBeGreaterThan(300)
    expect(r.trace.none).toBe(true)
    expect(r.progress.at(-1)).toEqual({ done: 92352, total: 92352 })
    expect(r.aborted).toBe('AbortError')
  })

  test('Alkove.stream: 48 kHz blocks from a ReadableStream give what process gives', async ({ page, origin }) => {
    await open(page, origin)
    const r: any = await page.evaluate(async () => {
      const { sdk, fixture } = window.lib
      const { pcm } = await fixture('speech.wav')
      const at48 = new Float32Array(pcm.length * 3)
      for (let i = 0; i < at48.length; i++) at48[i] = pcm[Math.floor(i / 3)]
      let i = 0
      const source = new ReadableStream<Float32Array>({
        pull(c) {
          if (i >= at48.length) return c.close()
          c.enqueue(at48.slice(i, i + 4800))
          i += 4800
        },
      })
      let n = 0
      const starts: number[] = []
      const speech: { start: number; end: number }[] = []
      for await (const part of sdk.Alkove.stream(source, { rate: 48000, cue: true })) {
        starts.push(part.start - n / 16000)
        n += part.audio.length
        speech.push(...part.speech)
      }
      const whole = await sdk.Alkove.process({ pcm: at48, rate: 48000 }, { cue: true })
      return { n, len: pcm.length, drift: Math.max(...starts.map(Math.abs)), speech, whole: whole.speech }
    })
    expect(Math.abs(r.n - r.len)).toBeLessThanOrEqual(1)
    expect(r.drift).toBeLessThan(1e-6)
    close(r.speech, r.whole, 0.03)
  })

  test('Lex, Timbre and VoiceprintLearner in the background worker match Node', async ({ page, origin }) => {
    const { Timbre, Lex } = await nodeAlkove()
    const nodePrint = (await (await Timbre.create()).embed(wavPcm('speech.wav'))).toJSON()
    const nodeIpa = await (await Lex.create()).phonemize('amendement', 'fr')
    await open(page, origin)
    const r: any = await page.evaluate(async () => {
      const { sdk, fixture } = window.lib
      const lex = await sdk.Lex.create()
      const ipa = await lex.phonemize('amendement', 'fr')
      const timbre = await sdk.Timbre.create()
      const a = await timbre.embed((await fixture('speech.wav')).pcm)
      const b = await timbre.embed((await fixture('enroll.wav')).pcm)
      let s = 7
      const noise = new Float32Array(48000).map(() => ((s = (s * 1664525 + 1013904223) >>> 0) / 2 ** 32 - 0.5) * 0.2)
      const n = await timbre.embed(noise)
      const learner = await sdk.VoiceprintLearner.create()
      const offered = await learner.offer((await fixture('enroll.wav')).pcm)
      const silent = await (await sdk.VoiceprintLearner.create()).offer(new Float32Array(48000))
      return { ipa, print: a.toJSON(), same: timbre.verify(a, b), other: timbre.verify(a, n), offered, silent, who: await timbre.identify(b, { me: a, noise: n }) }
    })
    expect(r.ipa).toEqual(nodeIpa)
    expect(r.ipa[0].ipa).toBe('amɑ̃dmɑ̃')
    expect(r.print.centers[0].embedding).toEqual(nodePrint.centers[0].embedding)
    expect(r.print.centers[0].id).toEqual(nodePrint.centers[0].id)
    expect(r.same.same).toBe(true)
    expect(r.other.same).toBe(false)
    expect(r.who?.who).toBe('me')
    expect(r.offered.accepted).toBeGreaterThan(0)
    expect(r.offered.progress).toBeGreaterThan(0)
    expect(r.silent).toMatchObject({ accepted: 0, rejected: 0, progress: 0 })
  })
})
