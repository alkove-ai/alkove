import { expect, open, reference, SLOW_FIREFOX, test } from './fixtures'

test.describe('createAgentInput', () => {
  test('listen(false) right after creating the input holds once the models run', async ({ page, origin, browserName }) => {
    test.skip(browserName === 'firefox', SLOW_FIREFOX)
    await open(page, origin)
    const r: any = await page.evaluate(async () => {
      const lib = window.lib
      const ctx = new AudioContext({ sampleRate: 48000 })
      const player = new lib.Player(ctx)
      const buf = await player.load('speech.wav')
      const input = await lib.sdk.createAgentInput({ stream: player.stream, audioContext: ctx })
      input.listen(false)
      const log = lib.watch(input, ['speechstart'])
      await input.ready
      player.play(buf, { delay: 0.2 })
      await lib.until(() => ctx.currentTime > player.end + 1, 30000, 'played muted')
      const muted = log.length
      input.listen(true)
      player.play(buf, { delay: 0.2 })
      await lib.until(() => ctx.currentTime > player.end + 1, 30000, 'played')
      input.dispose()
      return { muted, after: log.length - muted }
    })
    expect(r).toEqual({ muted: 0, after: 1 })
  })

  test('input.ready resolves once the models run; a stream the app passes is left as it is', async ({ page, origin }) => {
    await open(page, origin)
    const r: any = await page.evaluate(async () => {
      const lib = window.lib
      const ctx = new AudioContext({ sampleRate: 48000 })
      const input = await lib.sdk.createAgentInput({ stream: new lib.Player(ctx).stream, audioContext: ctx })
      const seen: string[] = []
      input.on('diag', (d: any) => seen.push(d.code))
      await input.ready
      const mode = input.mode
      const again = await input.ready.then(() => 'resolved')
      await lib.sleep(2500)
      input.dispose()
      const gone = await lib.sdk.createAgentInput({ stream: new lib.Player(ctx).stream, audioContext: ctx })
      const early = gone.ready.then(
        () => 'resolved',
        (e: any) => `${e.name} ${e.code}`,
      )
      gone.dispose()
      return { mode, again, browser: seen.includes('mic.browser'), early: await early }
    })
    expect(r).toEqual({ mode: 'focus', again: 'resolved', browser: false, early: 'AlkoveError engine.failed' })
  })

  test('audio packets and utterances in the format, rate and size asked; the track made on its first access', async ({ page, origin, browserName }) => {
    test.skip(browserName === 'firefox', SLOW_FIREFOX)
    const ref = await reference('speech.wav', { focus: true })
    await open(page, origin)
    const r: any = await page.evaluate(async () => {
      const lib = window.lib
      const ctx = new AudioContext({ sampleRate: 48000 })
      const player = new lib.Player(ctx)
      const buf = await player.load('speech.wav')
      const plain = await lib.sdk.createAgentInput({ stream: player.stream, audioContext: ctx })
      const asked = await lib.sdk.createAgentInput({ stream: player.stream, audioContext: ctx, output: { format: 's16', rate: 24000, chunkMs: 48 } })
      const got = [plain, asked].map((input) => {
        const g = { packets: [] as number[], utterances: [] as any[], speech: lib.watch(input, ['speechstart', 'speechend']), first: null as number | null, last: 0 }
        input.on('audio', (pcm: ArrayBuffer) => {
          g.packets.push(pcm.byteLength)
          g.first ??= input.now()
          g.last = input.now()
        })
        input.on('utterance', (u: any) => g.utterances.push({ bytes: u.pcm.byteLength, start: u.start, end: u.end }))
        return g
      })
      // the utterance's content, for its level
      let level = 0
      plain.on('utterance', (u: any) => (level = lib.rms(new Float32Array(u.pcm))))
      await lib.until(() => plain.mode === 'focus' && asked.mode === 'focus', 20000, 'focus mode')
      const tracks = [asked.track?.readyState ?? null]
      const when = player.play(buf, { delay: 0.3 })
      const t0 = when - (ctx.currentTime - plain.now())
      await lib.until(() => ctx.currentTime > player.end + 1.2, 30000, 'played')
      plain.dispose()
      asked.dispose()
      return { t0, got: got.map(({ packets, utterances, speech, first, last }) => ({ packets, utterances, speech, span: last - (first ?? 0) })), tracks, level }
    })
    const [plain, asked] = r.got
    // f32 at 16 kHz in 32 ms (512 samples); s16 at 24 kHz in 48 ms (1152 samples)
    expect(new Set(plain.packets)).toEqual(new Set([2048]))
    expect(new Set(asked.packets)).toEqual(new Set([2304]))
    // continuous: one packet per chunk of time, whatever is said
    expect(Math.abs(plain.packets.length * 0.032 - plain.span)).toBeLessThan(0.3)
    expect(Math.abs(plain.packets.length / asked.packets.length - 1.5)).toBeLessThan(0.1)
    // one utterance each, from 0.3 s before the speech to its end, in the asked format
    for (const [g, bytesPerSecond] of [
      [plain, 16000 * 4],
      [asked, 24000 * 2],
    ] as const) {
      expect(g.utterances).toHaveLength(1)
      const u = g.utterances[0]
      expect(Math.abs(u.start - r.t0 - (ref.speech[0].start - 0.3))).toBeLessThan(0.08)
      expect(Math.abs(u.end - r.t0 - ref.speech[0].end)).toBeLessThan(0.08)
      expect(Math.abs(u.bytes / bytesPerSecond - (u.end - u.start))).toBeLessThan(0.02)
      expect(g.speech.map((e: any) => e.type)).toEqual(['speechstart', 'speechend'])
    }
    expect(r.level).toBeGreaterThan(0.01)
    expect(r.tracks).toEqual(['live'])
  })

  test('agent.spot() hears the word, output.port takes the packets off the main thread, wake is reserved', async ({ page, origin, browserName }) => {
    test.skip(browserName === 'firefox', SLOW_FIREFOX)
    const ref = await reference('speech.wav', { focus: true })
    await open(page, origin)
    const r: any = await page.evaluate(async () => {
      const lib = window.lib
      const ctx = new AudioContext({ sampleRate: 48000 })
      const player = new lib.Player(ctx)
      const buf = await player.load('speech.wav')
      const word = await lib.sdk.Spot.compile('amendement', { lang: 'fr' })
      const channel = new MessageChannel()
      const onPort: string[] = []
      channel.port1.onmessage = (e) => onPort.push(e.data.type)
      const input = await lib.sdk.createAgentInput({ stream: player.stream, audioContext: ctx, output: { port: channel.port2 } })
      let onMain = 0
      input.on('audio', () => onMain++)
      input.on('utterance', () => onMain++)
      const wakes: any[] = []
      // right after creation, before the models are in: kept and sent once the pipeline starts
      input.spot([word]).on('wake', (w: any) => wakes.push(lib.plain(w)))
      await lib.until(() => input.mode === 'focus', 20000, 'focus mode')
      const when = player.play(buf, { delay: 0.3 })
      const t0 = when - (ctx.currentTime - input.now())
      await lib.until(() => ctx.currentTime > player.end + 1.2, 30000, 'played')
      input.dispose()
      const wake = await lib.sdk.createAgentInput({ stream: player.stream, audioContext: ctx, wake: true }).then(
        (i: any) => (i.dispose(), 'resolved'),
        (e: Error) => e.name,
      )
      return { t0, wakes, onMain, port: { audio: onPort.filter((t) => t === 'audio').length, utterance: onPort.filter((t) => t === 'utterance').length }, wake }
    })
    expect(r.wakes).toHaveLength(1)
    expect(r.wakes[0].word).toBe('amendement')
    expect(Math.abs(r.wakes[0].start - r.t0 - ref.wakes[0].start)).toBeLessThan(0.08)
    expect(r.onMain).toBe(0)
    expect(r.port.audio).toBeGreaterThan(100)
    expect(r.port.utterance).toBe(1)
    expect(r.wake).toBe('TypeError')
  })

  test('the print learns nothing before the first farEndActive(), then learns while the agent is quiet', async ({ page, origin, browserName }) => {
    test.skip(browserName === 'firefox', SLOW_FIREFOX)
    test.slow()
    await open(page, origin)
    const r: any = await page.evaluate(async () => {
      const lib = window.lib
      const ctx = new AudioContext({ sampleRate: 48000 })
      const player = new lib.Player(ctx)
      const buf = await player.load('speech.wav')
      const input = await lib.sdk.createAgentInput({ stream: player.stream, audioContext: ctx })
      const log = lib.watch(input, ['learn', 'voiceprint', 'diag'])
      await lib.until(() => input.mode === 'focus', 20000, 'focus mode')
      player.play(buf, { delay: 0.2, times: 2, gap: 0.5 })
      await lib.until(() => ctx.currentTime > player.end + 0.5, 30000, 'played before')
      const before = log.length
      // the agent says something, then is quiet for 2 s
      input.farEndActive(true)
      await lib.sleep(300)
      input.farEndActive(false)
      await lib.sleep(2200)
      player.play(buf, { delay: 0.2, times: 4, gap: 0.5 })
      await lib.until(() => log.some((e: any) => e.type === 'voiceprint') || ctx.currentTime > player.end + 2, 60000, 'learned')
      const timbre = await lib.sdk.Timbre.create()
      const same = input.voiceprint ? timbre.verify(input.voiceprint, await timbre.embed((await lib.fixture('enroll.wav')).pcm)).same : null
      input.dispose()
      return { before: log.slice(0, before), after: log.slice(before), same }
    })
    const learned = (l: any[]) => l.filter((e: any) => e.type === 'learn' || e.type === 'voiceprint' || (e.type === 'diag' && /^print\./.test(e.v.code)))
    expect(learned(r.before)).toEqual([])
    expect(r.after.some((e: any) => e.type === 'learn' && e.v.progress > 0)).toBe(true)
    expect(r.after.some((e: any) => e.type === 'voiceprint')).toBe(true)
    expect(r.same).toBe(true)
  })
})
