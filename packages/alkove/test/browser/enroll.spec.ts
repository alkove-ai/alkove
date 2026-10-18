import { expect, open, SLOW_FIREFOX, test } from './fixtures'

test.describe('enrolling a word', () => {
  test('Spot.enroll: three takes set the thresholds and give the print; takes: 2 stops at two; cancel() rejects', async ({ page, origin }) => {
    test.slow()
    await open(page, origin)
    const r: any = await page.evaluate(async () => {
      const lib = window.lib
      const ctx = new AudioContext({ sampleRate: 48000 })
      const player = new lib.Player(ctx)
      const buf = await player.load('speech.wav')
      const opts = { lang: 'fr', stream: player.stream, audioContext: ctx }
      const three = lib.sdk.Spot.enroll('amendement', opts)
      const two = lib.sdk.Spot.enroll('amendement', { ...opts, takes: 2 })
      const cancelled = lib.sdk.Spot.enroll('amendement', opts)
      cancelled.cancel()
      const cancel = await cancelled.done.then(
        () => 'resolved',
        (e: Error) => e.name,
      )
      const takes: any[][] = [[], []]
      three.on('take', (t: any) => takes[0]!.push(lib.plain(t)))
      two.on('take', (t: any) => takes[1]!.push(lib.plain(t)))
      const warnings: string[] = []
      three.on('warning', (w: any) => warnings.push(w.code ?? String(w)))
      // the inputs open after Lex has typed the word
      await lib.sleep(1500)
      player.play(buf, { delay: 0.2, times: 3, gap: 1 })
      const timeout = lib.sleep(40000).then(() => {
        throw new Error('enrollment not done')
      })
      const t2 = await Promise.race([two.done, timeout])
      const doneTwo = ctx.currentTime
      const t3 = await Promise.race([three.done, timeout])
      const doneThree = ctx.currentTime
      const timbre = await lib.sdk.Timbre.create()
      const same = timbre.verify(t3.voiceprint, await timbre.embed((await lib.fixture('enroll.wav')).pcm))

      // the enrolled word, heard by a wake listener on a fourth play
      const wake = await lib.sdk.createWakeListener({ stream: player.stream, audioContext: ctx, words: [t3.word], context: 'anywhere' })
      const wakes: any[] = []
      wake.on('wake', (w: any) => wakes.push(lib.plain(w)))
      await lib.until(() => wake.mode === 'spot', 20000, 'spot mode')
      player.play(buf, { delay: 0.2 })
      await lib.until(() => ctx.currentTime > player.end + 1, 30000, 'played')
      wake.dispose()
      return { cancel, takes, warnings, two: t2.word.toJSON(), three: t3.word.toJSON(), print: t3.voiceprint?.toJSON() ?? null, same, early: doneThree - doneTwo, wakes }
    })
    expect(r.cancel).toBe('AbortError')
    const ok = r.takes.map((t: any[]) => t.filter((x) => x.ok))
    expect(ok[0]).toHaveLength(3)
    expect(ok[1]).toHaveLength(2)
    // takes: 2 was done a play earlier
    expect(r.early).toBeGreaterThan(3)
    for (const w of [r.two, r.three]) {
      expect(w.calibrated).toBe(true)
      expect(w.chain).not.toBe('focus')
      expect(w.thresholds.pause).toBeLessThan(w.thresholds.anywhere)
      expect(w.thresholds.anywhere).toBeGreaterThanOrEqual(-2.2)
      expect(w.thresholds.anywhere).toBeLessThanOrEqual(-0.6)
    }
    expect(r.three.thresholds.anywhere).toBeLessThanOrEqual(Math.min(...ok[0].map((t: any) => t.score)))
    // the print, from the takes' sound on the raw mic: the voice of enroll.wav
    expect(r.print.source).toBe('raw')
    expect(r.same.same).toBe(true)
    expect(r.wakes).toHaveLength(1)
    expect(r.wakes[0].word).toBe('amendement')
  })

  test('call.enrollWord: takes read behind Focus, then call.spot() finds the word; such a word warns on the raw mic', async ({ page, origin, browserName }) => {
    test.skip(browserName === 'firefox', SLOW_FIREFOX)
    test.slow()
    await open(page, origin)
    const r: any = await page.evaluate(async () => {
      const lib = window.lib
      const ctx = new AudioContext({ sampleRate: 48000 })
      const player = new lib.Player(ctx)
      const buf = await player.load('speech.wav')
      const call = await lib.sdk.createCallInput({ stream: player.stream, audioContext: ctx })
      await lib.until(() => call.mode === 'focus', 20000, 'focus mode')
      const enrollment = call.enrollWord('amendement', { lang: 'fr' })
      const takes: any[] = []
      enrollment.on('take', (t: any) => takes.push(lib.plain(t)))
      await lib.sleep(1500)
      player.play(buf, { delay: 0.2, times: 3, gap: 1 })
      const done = await Promise.race([
        enrollment.done,
        lib.sleep(40000).then(() => {
          throw new Error('enrollment not done')
        }),
      ])
      const wakes: any[] = []
      call.spot([done.word]).on('wake', (w: any) => wakes.push(lib.plain(w)))
      await lib.sleep(500)
      player.play(buf, { delay: 0.2 })
      await lib.until(() => ctx.currentTime > player.end + 1, 30000, 'played')
      call.dispose()
      const diags: string[] = []
      const wake = await lib.sdk.createWakeListener({ stream: player.stream, audioContext: ctx, words: [done.word] })
      wake.on('diag', (d: any) => diags.push(d.code))
      await lib.sleep(50)
      wake.dispose()
      return { takes, word: done.word.toJSON(), print: done.voiceprint?.toJSON() ?? null, wakes, diags }
    })
    expect(r.takes.filter((t: any) => t.ok)).toHaveLength(3)
    expect(r.word.chain).toBe('focus')
    expect(r.word.calibrated).toBe(true)
    expect(r.print.source).toBe('focus')
    expect(r.wakes).toHaveLength(1)
    expect(r.diags).toContain('word.chain')
  })
})
