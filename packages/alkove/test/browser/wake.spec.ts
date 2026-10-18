import { expect, nodeAlkove, open, reference, test } from './fixtures'

test.describe('createWakeListener', () => {
  test('the word where Node finds it on the raw mic, with its audio; context "pause" (the default) against "anywhere"', async ({ page, origin, browserName }) => {
    const ref = await onRaw('anywhere')
    const pause = await onRaw('pause')
    await open(page, origin)
    const r: any = await page.evaluate(async () => {
      const lib = window.lib
      const ctx = new AudioContext({ sampleRate: 48000 })
      const player = new lib.Player(ctx)
      const buf = await player.load('speech.wav')
      const word = await lib.sdk.Spot.compile('amendement', { lang: 'fr' })
      const any = await lib.sdk.createWakeListener({ track: player.stream.getAudioTracks()[0], audioContext: ctx, words: [word], context: 'anywhere' })
      const def = await lib.sdk.createWakeListener({ stream: player.stream, audioContext: ctx, words: [word] })
      const logs = [lib.watch(any, ['wake', 'mode']), lib.watch(def, ['wake', 'mode'])]
      let heard = 0
      let scored = 0
      any.on('heard', (h: any) => {
        heard++
        if (typeof h.speech === 'number' && h.scores.some((s: any) => s.word === 'amendement' && typeof s.score === 'number')) scored++
      })
      const audio: number[] = []
      any.on('wake', (w: any) => audio.push(w.audio.length))
      await lib.until(() => any.mode === 'spot' && def.mode === 'spot', 20000, 'spot mode')
      const when = player.play(buf, { delay: 0.3 })
      const t0 = when - (ctx.currentTime - any.now())
      await lib.until(() => ctx.currentTime > player.end + 1, 30000, 'played')
      any.dispose()
      def.dispose()
      return { t0, any: logs[0], def: logs[1], audio, heard, scored }
    })
    const wakes = r.any.filter((e: any) => e.type === 'wake').map((e: any) => e.v)
    expect(r.any.find((e: any) => e.type === 'mode').v).toEqual({ mode: 'spot' })
    expect(wakes).toHaveLength(1)
    expect(wakes[0]).toMatchObject({ word: 'amendement', context: 'anywhere' })
    // the MediaStream hop puts it 10 to 40 ms late, Spot's 30 ms frames round it (Firefox: see the silent stream test)
    const late = wakes[0].start - r.t0 - ref.wakes[0].start
    if (browserName !== 'firefox') {
      expect(late).toBeGreaterThan(-0.02)
      expect(late).toBeLessThan(0.08)
    }
    expect(Math.abs(wakes[0].end - wakes[0].start - (ref.wakes[0].end - ref.wakes[0].start))).toBeLessThan(0.035)
    // its audio: the word itself, 16 kHz
    expect(Math.abs(r.audio[0] / 16000 - (wakes[0].end - wakes[0].start))).toBeLessThan(0.02)
    // "heard": Spot's scores as they go, for a settings screen
    expect(r.heard).toBeGreaterThan(10)
    expect(r.scored).toBe(r.heard)
    // after a pause only: as many as Node finds that way, and none mid-sentence
    const def = r.def.filter((e: any) => e.type === 'wake').map((e: any) => e.v)
    expect(def.length).toBe(pause.wakes.length)
    expect(def.every((w: any) => w.context === 'pause')).toBe(true)
  })

  test('times stay on the input clock after a silent stream (2 s before the file)', async ({ page, origin, browserName }) => {
    const ref = await onRaw('anywhere')
    await open(page, origin)
    const r: any = await page.evaluate(async () => {
      const lib = window.lib
      const ctx = new AudioContext({ sampleRate: 48000 })
      const player = new lib.Player(ctx)
      const buf = await player.load('speech.wav')
      const word = await lib.sdk.Spot.compile('amendement', { lang: 'fr' })
      const input = await lib.sdk.createWakeListener({ stream: player.stream, audioContext: ctx, words: [word], context: 'anywhere' })
      const wakes: any[] = []
      input.on('wake', (w: any) => wakes.push(w.start))
      await lib.until(() => input.mode === 'spot', 20000, 'spot mode')
      const when = player.play(buf, { delay: 2 })
      const t0 = when - (ctx.currentTime - input.now())
      await lib.until(() => ctx.currentTime > player.end + 1, 30000, 'played')
      input.dispose()
      return { t0, wakes }
    })
    expect(r.wakes).toHaveLength(1)
    expect(r.wakes[0] - r.t0 - ref.wakes[0].start).toBeGreaterThan(-0.02)
    expect(r.wakes[0] - r.t0 - ref.wakes[0].start).toBeLessThan(0.08)
  })

  test('onlyMyVoice: the same voice wakes it (with its score), another does not (wake.other-voice)', async ({ page, origin }) => {
    await open(page, origin)
    const r: any = await page.evaluate(async () => {
      const lib = window.lib
      const ctx = new AudioContext({ sampleRate: 48000 })
      const player = new lib.Player(ctx)
      const buf = await player.load('speech.wav')
      const word = await lib.sdk.Spot.compile('amendement', { lang: 'fr' })
      const timbre = await lib.sdk.Timbre.create()
      const me = await timbre.embed((await lib.fixture('enroll.wav')).pcm)
      let s = 3
      const noise = await timbre.embed(new Float32Array(48000).map(() => ((s = (s * 1664525 + 1013904223) >>> 0) / 2 ** 32 - 0.5) * 0.2))
      const mine = await lib.sdk.createWakeListener({ stream: player.stream, audioContext: ctx, words: [word], context: 'anywhere', onlyMyVoice: me })
      // a print as stored JSON works the same
      const other = await lib.sdk.createWakeListener({ stream: player.stream, audioContext: ctx, words: [word], context: 'anywhere', onlyMyVoice: JSON.stringify(noise.toJSON()) })
      const logs = [lib.watch(mine, ['wake', 'mode', 'diag']), lib.watch(other, ['wake', 'mode', 'diag'])]
      await lib.until(() => mine.mode === 'spot+print' && other.mode === 'spot+print', 20000, 'spot+print mode')
      const when = player.play(buf, { delay: 0.3 })
      const t0 = when - (ctx.currentTime - mine.now())
      await lib.until(() => ctx.currentTime > player.end + 1, 30000, 'played')
      mine.dispose()
      other.dispose()
      return { t0, mine: logs[0], other: logs[1] }
    })
    const mine = r.mine.filter((e: any) => e.type === 'wake')
    expect(mine).toHaveLength(1)
    expect(mine[0].v.word).toBe('amendement')
    expect(typeof mine[0].v.voice).toBe('number')
    expect(mine[0].v.audio.samples).toBeGreaterThan(3000)
    // Timbre runs on the word + 0.5 s: the wake comes about 0.5 s after the word ends
    expect(mine[0].now - (mine[0].v.end + 0.5)).toBeGreaterThan(-0.05)
    expect(r.other.filter((e: any) => e.type === 'wake')).toEqual([])
    const refused = r.other.filter((e: any) => e.type === 'diag' && e.v.code === 'wake.other-voice')
    expect(refused).toHaveLength(1)
    expect(refused[0].v.word).toBe('amendement')
    expect(refused[0].v.voice).toBeLessThan(mine[0].v.voice)
  })
})

// Spot alone on the raw file, in Node
async function onRaw(context: 'anywhere' | 'pause') {
  const { Spot } = await nodeAlkove()
  return reference('speech.wav', { cue: false, spot: { words: [await Spot.compile('amendement', { lang: 'fr' })], context } })
}
