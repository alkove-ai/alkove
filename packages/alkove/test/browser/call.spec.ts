import { align, expect, fromB64, locate, open, reference, residualDb, SECONDS, SLOW_FIREFOX, test, wavPcm } from './fixtures'

// the call's chain: Focus's window, then the gate's look-ahead (100 ms)
const CHAIN = 256 + 1600
// a MediaStream hop (destination to source) varies from one connection to the next, by up to ~10 ms in Chromium
const HOP_SPREAD_MS = 12
// Cue's and Spot's frame (CUE.chunk, SPOT.chunk)
const FRAME = 480 / 16000

test.describe('createCallInput', () => {
  test('on getUserMedia with a fake microphone: focus, speech and the word where the file has them, every loop', async ({ page, origin, browserName }) => {
    test.skip(browserName !== 'chromium', 'only Chromium plays a file as its fake microphone')
    const ref = await reference('speech.wav', { focus: true })
    await open(page, origin)
    const r: any = await page.evaluate(async () => {
      const lib = window.lib
      const md = navigator.mediaDevices
      const asked: MediaStreamConstraints[] = []
      let got: MediaStream | null = null
      const gum = md.getUserMedia.bind(md)
      md.getUserMedia = async (c?: MediaStreamConstraints) => {
        asked.push(c!)
        return (got = await gum(c))
      }
      const word = await lib.sdk.Spot.compile('amendement', { lang: 'fr' })
      const ctx = new AudioContext({ sampleRate: 48000 })
      const input = await lib.sdk.createCallInput({ audioContext: ctx })
      const log = lib.watch(input)
      const mic = await lib.record(ctx, got!.getAudioTracks()[0])
      const wakes: any[] = []
      input.spot([word]).on('wake', (w: any) => wakes.push(lib.plain(w)))
      await lib.until(() => input.mode === 'focus', 20000, 'focus mode')
      const recStart = input.now()
      const toInput = ctx.currentTime - recStart
      const recorded = input.record(8)
      const rec = await lib.record(ctx, input.track)
      const { left, right } = await lib.stereo(await recorded)
      const proc = await lib.sdk.Alkove.process({ pcm: left, rate: 16000 }, { focus: true, normalize: true })
      await lib.until(() => input.now() > 2 * 5.772 + 1.5, 30000, 'two loops of the file')
      const out = rec.stop()
      const micPcm = mic.stop()
      input.dispose()
      return { asked, settings: got!.getAudioTracks()[0]!.getSettings(), state: got!.getAudioTracks()[0]!.readyState, ctxState: ctx.state, log, wakes, latency: input.latency, recStart, toInput, mic: lib.b64(micPcm), micAt: mic.startTime, left: lib.b64(left), right: lib.b64(right), proc: lib.b64(proc.audio), out: lib.b64(out) }
    })
    // the SDK opened the mic itself, the browser's processing off but echo cancellation; dispose stopped it, not the app's context
    expect(r.asked).toHaveLength(1)
    expect(r.asked[0]).toMatchObject({ audio: { echoCancellation: true, noiseSuppression: false, autoGainControl: false, channelCount: 1 } })
    expect(r.settings).toMatchObject({ noiseSuppression: false, autoGainControl: false })
    expect(r.state).toBe('ended')
    expect(r.ctxState).toBe('running')
    expect(r.log.filter((e: any) => e.type === 'diag' && e.v.code === 'mic.processed')).toEqual([])
    expect(r.log.find((e: any) => e.type === 'mode').v).toEqual({ mode: 'focus' })
    // the announced delay, plus the queue's debt when the worker was late
    expect(r.latency).toBeGreaterThanOrEqual(149)
    expect(r.latency).toBeLessThan(149 + 100)

    const starts = r.log.filter((e: any) => e.type === 'speechstart').map((e: any) => e.v.start)
    const ends = r.log.filter((e: any) => e.type === 'speechend').map((e: any) => e.v.end)
    const words = r.wakes.map((w: any) => w.start)
    expect(starts.length).toBeGreaterThanOrEqual(2)
    expect(ends.length).toBeGreaterThanOrEqual(2)
    expect(words.length).toBeGreaterThanOrEqual(2)
    expect(r.wakes.every((w: any) => w.word === 'amendement')).toBe(true)
    // where the loop started in the input's clock, from the word; speech and its end agree within a 30 ms frame
    const offset = words[0] - ref.wakes[0].start
    expect(offset).toBeGreaterThan(-0.1)
    expect(offset).toBeLessThan(1)
    expect(Math.abs(starts[0] - offset - ref.speech[0].start)).toBeLessThanOrEqual(0.04)
    expect(Math.abs(ends[0] - offset - ref.speech[0].end)).toBeLessThanOrEqual(0.04)
    // Chromium's fake mic loops a little later than the file (16 ms here): the loop measured on the track itself, in the context's clock
    const at48 = Math.round((starts[0] + 0.1 + r.toInput - r.micAt) * 48000)
    const loop48 = align(fromB64(r.mic), fromB64(r.mic), at48, 24000, Math.round(SECONDS * 48000), Math.round((SECONDS + 0.05) * 48000)).lag
    // the mic the worker got loops the same, to the 16 kHz sample: no block dropped or doubled between the track and the worker
    const left = fromB64(r.left)
    const at16 = Math.round((starts[0] + 0.1 - r.recStart) * 16000)
    const loop = align(left, left, at16, 8000, Math.round(SECONDS * 16000), Math.round((SECONDS + 0.05) * 16000))
    expect(loop.corr).toBeGreaterThan(0.99)
    expect(Math.abs(3 * loop.lag - loop48)).toBeLessThanOrEqual(3)
    // the events one loop later: each lands on the frame its score crosses a threshold on, the grid slides along the loop and the score there varies a little, a frame each
    for (const x of [starts, ends, words]) expect(Math.abs(x[1] - x[0] - loop.lag / 16000)).toBeLessThanOrEqual(2 * FRAME + 1e-6)

    // input.record(): the output is Alkove.process of the mic the worker got (WebRTC's echo canceller ran on it), CHAIN samples later, where the gate is open
    const right = fromB64(r.right)
    const proc = fromB64(r.proc)
    const from = Math.round((starts[0] - r.recStart + 0.15) * 16000)
    const n = Math.round((ends[0] - starts[0] - 0.3) * 16000)
    expect(align(proc, right, from, n, CHAIN - 400, CHAIN + 400).lag).toBe(CHAIN)
    expect(residualDb(proc, right, from, n, CHAIN)).toBeLessThan(-25)
    expect(residualDb(left, right, from, n, CHAIN)).toBeGreaterThan(-20)
    // the output track carries that output (one more resampling, the worklet's round trip, a MediaStream hop); half a second, the worklet may drop quiet blocks around it
    expect(locate(right, fromB64(r.out), 0.05, 0.1, 0.3, from + 8000, 8000).corr).toBeGreaterThan(0.9)
  })

  test('getUserMedia options: deviceId is asked for, an unknown one is mic.not-found', async ({ page, origin, browserName }) => {
    test.skip(browserName !== 'chromium', 'fake devices: Chromium')
    await open(page, origin)
    const r: any = await page.evaluate(async () => {
      const { sdk } = window.lib
      const md = navigator.mediaDevices
      const asked: any[] = []
      const gum = md.getUserMedia.bind(md)
      md.getUserMedia = (c?: MediaStreamConstraints) => (asked.push(c), gum(c))
      const devices = (await md.enumerateDevices()).filter((d) => d.kind === 'audioinput' && d.deviceId !== 'default')
      const id = devices[0]!.deviceId
      const input = await sdk.createCallInput({ deviceId: id })
      const used = input.track ? asked[0].audio.deviceId : null
      input.dispose()
      const plain = await sdk.createCallInput({})
      const unset = asked[1].audio.deviceId
      plain.dispose()
      const unknown = await sdk.createCallInput({ deviceId: 'no-such-device' }).then(
        (i: any) => (i.dispose(), 'resolved'),
        (e: any) => `${e.name} ${e.code}`,
      )
      return { id, used, unset, unknown }
    })
    expect(r.used).toEqual({ exact: r.id })
    expect(r.unset).toBeUndefined()
    expect(r.unknown).toBe('AlkoveError mic.not-found')
  })

  test('an aborted signal rejects with the signal\'s AbortError', async ({ page, origin, browserName }) => {
    test.skip(browserName !== 'chromium', 'fake devices: Chromium')
    await open(page, origin)
    const r: any = await page.evaluate(async () => {
      const ac = new AbortController()
      ac.abort()
      return window.lib.sdk.createCallInput({ signal: ac.signal }).then(
        (i: any) => (i.dispose(), 'resolved'),
        (e: any) => (e.name === 'AlkoveError' ? `${e.name} ${e.code}` : e.name),
      )
    })
    expect(r).toBe('AbortError')
  })

  test('diags raised while the input is created reach the app: learn.no-far-end, mic.twice, mic.processed', async ({ page, origin, browserName }) => {
    test.skip(browserName === 'firefox', SLOW_FIREFOX)
    await open(page, origin)
    const r: any = await page.evaluate(async () => {
      const lib = window.lib
      const seen: string[] = []
      lib.sdk.Alkove.onDiag((d: any) => seen.push(d.code))
      const ctx = new AudioContext({ sampleRate: 48000 })
      const player = new lib.Player(ctx)
      const a = await lib.sdk.createCallInput({ stream: player.stream, audioContext: ctx, learn: true })
      a.on('diag', (d: any) => seen.push(d.code))
      const b = await lib.sdk.createCallInput({ stream: player.stream, audioContext: ctx })
      b.on('diag', (d: any) => seen.push(d.code))
      const inputs = [a, b]
      if (navigator.userAgent.includes('Chrome')) {
        // a stream the app opened with the browser's noise suppression on
        const ns = await navigator.mediaDevices.getUserMedia({ audio: { noiseSuppression: true } })
        const c = await lib.sdk.createCallInput({ stream: ns, audioContext: ctx })
        c.on('diag', (d: any) => seen.push(d.code))
        inputs.push(c)
      }
      await lib.until(() => inputs.every((i) => i.mode === 'focus'), 20000, 'focus mode')
      for (const i of inputs) i.dispose()
      return { seen, chrome: inputs.length === 3 }
    })
    expect(r.seen).toContain('learn.no-far-end')
    expect(r.seen).toContain('mic.twice')
    if (r.chrome) expect(r.seen).toContain('mic.processed')
  })

  test('a warning the app does not listen to goes to the console, once; one it listens to stays out of it', async ({ page, origin, browserName }) => {
    test.skip(browserName !== 'chromium', 'fake devices: Chromium')
    const warnings: string[] = []
    page.on('console', (m) => m.type() === 'warning' && m.text().startsWith('alkove:') && warnings.push(m.text()))
    await open(page, origin)
    const seen: any = await page.evaluate(async () => {
      const lib = window.lib
      const ctx = new AudioContext({ sampleRate: 48000 })
      const ns = await navigator.mediaDevices.getUserMedia({ audio: { noiseSuppression: true } })
      const deaf = await lib.sdk.createCallInput({ stream: ns, audioContext: ctx })
      await lib.sleep(300)
      deaf.dispose()
      const again = await lib.sdk.createCallInput({ stream: ns, audioContext: ctx })
      await lib.sleep(300)
      again.dispose()
      const seen: string[] = []
      const heard = await lib.sdk.createCallInput({ stream: ns, audioContext: ctx })
      heard.on('diag', (d: any) => seen.push(d.code))
      await lib.sleep(300)
      heard.dispose()
      return seen
    })
    expect(seen).toContain('mic.processed')
    // one per input that does not listen: two
    expect(warnings.filter((w) => w.includes('processes the stream'))).toHaveLength(2)
  })

  test('on a played stream: speech, word and output where Alkove.process puts them, at the announced delay', async ({ page, origin, browserName }) => {
    test.skip(browserName === 'firefox', SLOW_FIREFOX)
    const ref = await reference('speech.wav', { focus: true })
    await open(page, origin)
    const r: any = await page.evaluate(async () => {
      const lib = window.lib
      const ctx = new AudioContext({ sampleRate: 48000 })
      const player = new lib.Player(ctx)
      const speech = await player.load('speech.wav')
      const mix = await player.load('mix.wav')
      const word = await lib.sdk.Spot.compile('amendement', { lang: 'fr' })
      const input = await lib.sdk.createCallInput({ stream: player.stream, audioContext: ctx, normalize: false })
      const log = lib.watch(input)
      const wakes: any[] = []
      input.spot([word]).on('wake', (w: any) => wakes.push(lib.plain(w)))
      await lib.until(() => input.mode === 'focus', 20000, 'focus mode')
      // first the voice with another voice and noise, while the chain is still fresh and awake: process() starts from the same state
      const recorded = input.record(mix.duration + 1.5)
      const whenMix = player.play(mix, { delay: 0.3 })
      const { left, right } = await lib.stereo(await recorded)
      const proc = await lib.sdk.Alkove.process({ pcm: left, rate: 16000 }, { focus: true })
      await lib.until(() => ctx.currentTime > whenMix + mix.duration + 3, 30000, 'quiet after the first file')
      // then the clean voice, for the times and the delay: the worklet has paid back what it owed from the start (see the latency test)
      const recOut = await lib.record(ctx, input.track)
      // the same MediaStream hop the SDK's input and output take, measured on the played track itself
      const recIn = await lib.record(ctx, player.stream.getAudioTracks()[0])
      await lib.sleep(300)
      const toInput = ctx.currentTime - input.now()
      const when = player.play(speech, { delay: 0.3 })
      await lib.until(() => ctx.currentTime > player.end + 1.5, 30000, 'second file played')
      const out = recOut.stop()
      const inp = recIn.stop()
      input.dispose()
      return { t0: when - toInput, log, wakes, latency: input.latency, left: lib.b64(left), right: lib.b64(right), proc: lib.b64(proc.audio), out: lib.b64(out), outAt: when - recOut.startTime, inp: lib.b64(inp), inAt: when - recIn.startTime }
    })
    // events: the file's times from Node, + where the file started, + the MediaStream hop (10 ms in Chromium, 40 ms in WebKit)
    const start = r.log.findLast((e: any) => e.type === 'speechstart').v.start - r.t0
    const end = r.log.findLast((e: any) => e.type === 'speechend').v.end - r.t0
    const words = r.wakes.filter((w: any) => w.start > r.t0)
    expect(words).toHaveLength(1)
    // Spot's 30 ms frames round it: within a frame of the hop
    const hop = words[0].start - r.t0 - ref.wakes[0].start
    expect(hop).toBeGreaterThanOrEqual(-0.02)
    expect(hop).toBeLessThan(0.08)
    expect(Math.abs(start - hop - ref.speech[0].start)).toBeLessThanOrEqual(0.035)
    expect(Math.abs(end - hop - ref.speech[0].end)).toBeLessThanOrEqual(0.035)

    // input.record(): the output is Alkove.process of the mic the worker got, CHAIN samples later, to float noise
    const left = fromB64(r.left)
    const right = fromB64(r.right)
    const proc = fromB64(r.proc)
    expect(align(proc, right, 16000, 64000, CHAIN - 400, CHAIN + 400).lag).toBe(CHAIN)
    expect(residualDb(proc, right, 16000, 64000, CHAIN)).toBeLessThan(-30)
    // and not the raw mic: Focus took the other voice and the noise out
    expect(residualDb(left, right, 16000, 64000, CHAIN)).toBeGreaterThan(-20)

    // the output track: the announced delay once the two MediaStream hops (in, out) are taken off
    const hopIn = locate(wavPcm('speech.wav'), fromB64(r.inp), r.inAt, 0.01, 0.15).at - r.inAt
    const outAt = locate(ref.audio, fromB64(r.out), r.outAt + r.latency / 1000, 0.05, 0.3)
    expect(r.latency).toBeGreaterThanOrEqual(149)
    expect(Math.abs((outAt.at - r.outAt - 2 * hopIn) * 1000 - r.latency)).toBeLessThanOrEqual(HOP_SPREAD_MS)
    // close to Alkove.process of the file itself: the same voice, through two resamplers and two MediaStream hops
    expect(outAt.corr).toBeGreaterThan(0.9)
  })

  test('input.latency follows the real delay when the worklet starved (call.spot() mid-call blocks the worker)', async ({ page, origin, browserName }) => {
    test.skip(browserName === 'firefox', SLOW_FIREFOX)
    const ref = await reference('speech.wav', { focus: true })
    await open(page, origin)
    const r: any = await page.evaluate(async () => {
      const lib = window.lib
      const ctx = new AudioContext({ sampleRate: 48000 })
      const player = new lib.Player(ctx)
      const buf = await player.load('speech.wav')
      const word = await lib.sdk.Spot.compile('amendement', { lang: 'fr' })
      const input = await lib.sdk.createCallInput({ stream: player.stream, audioContext: ctx, normalize: false })
      const log = lib.watch(input, ['diag'])
      await lib.until(() => input.mode === 'focus', 20000, 'focus mode')
      await lib.sleep(4000)
      const recOut = await lib.record(ctx, input.track)
      const recIn = await lib.record(ctx, player.stream.getAudioTracks()[0])
      await lib.sleep(300)
      // the worker builds Spot's session: tens of ms without replies
      input.spot([word])
      await lib.sleep(800)
      const when = player.play(buf, { delay: 0.2 })
      await lib.until(() => ctx.currentTime > when + 3, 30000, 'played')
      const debt = Math.max(0, ...log.filter((e: any) => e.v.code === 'queue').map((e: any) => e.v.debtMs))
      return { debt, latency: input.latency, out: lib.b64(recOut.stop()), inp: lib.b64(recIn.stop()), outAt: when - recOut.startTime, inAt: when - recIn.startTime }
    })
    test.skip(r.debt < 25, `the worker did not starve the worklet this time (debt ${r.debt} ms)`)
    const hopIn = locate(wavPcm('speech.wav'), fromB64(r.inp), r.inAt, 0.01, 0.15).at - r.inAt
    const outAt = locate(ref.audio, fromB64(r.out), r.outAt + 0.149, 0.05, 0.4, 20800, 16000)
    const real = (outAt.at - r.outAt - 2 * hopIn) * 1000
    expect(Math.abs(real - r.latency)).toBeLessThanOrEqual(HOP_SPREAD_MS)
  })

  test('too slow for Focus (Firefox under Playwright): the raw mic at the same delay, overload, a new try 10 s later', async ({ page, origin, browserName }) => {
    test.skip(browserName !== 'firefox', 'only Firefox under Playwright is slow enough')
    await open(page, origin)
    const r: any = await page.evaluate(async () => {
      const lib = window.lib
      const ctx = new AudioContext({ sampleRate: 48000 })
      const player = new lib.Player(ctx)
      const buf = await player.load('speech.wav')
      const input = await lib.sdk.createCallInput({ stream: player.stream, audioContext: ctx })
      const log = lib.watch(input)
      await lib.until(() => input.mode === 'focus', 30000, 'models')
      // the chain sleeps on silence and keeps up: speech wakes Focus, which falls behind
      player.play(buf, { delay: 0.1 })
      await lib.until(() => input.mode === 'raw' || ctx.currentTime > player.end + 1, 30000, 'overload')
      if (input.mode !== 'raw') return { kept: true, log }
      player.stop()
      const recOut = await lib.record(ctx, input.track)
      const recIn = await lib.record(ctx, player.stream.getAudioTracks()[0])
      await lib.sleep(200)
      // sound until after the next try: Firefox gives the worklet no channel at all on silence (see the test after)
      const when = player.play(buf, { delay: 0.2, times: 2 })
      await lib.until(() => log.filter((e: any) => e.type === 'mode').length >= 3 || input.now() > 40, 40000, 'retry')
      const out = recOut.stop()
      const inp = recIn.stop()
      input.dispose()
      return { kept: false, log, latency: input.latency, out: lib.b64(out), outAt: when - recOut.startTime, inp: lib.b64(inp), inAt: when - recIn.startTime }
    })
    test.skip(r.kept, 'Firefox kept up with Focus: the overload path is not reached')
    const modes = r.log.filter((e: any) => e.type === 'mode')
    expect(modes.slice(0, 3).map((e: any) => e.v)).toEqual([{ mode: 'focus' }, { mode: 'raw', reason: 'overload' }, { mode: 'focus' }])
    expect(r.log.some((e: any) => e.type === 'diag' && e.v.code === 'overload')).toBe(true)
    expect(Math.abs(modes[2].now - modes[1].now - 10)).toBeLessThan(0.5)
    // raw, the mic comes out delayed as much as the processed path would be
    const wav = wavPcm('speech.wav')
    const hopIn = locate(wav, fromB64(r.inp), r.inAt, 0.01, 0.15).at - r.inAt
    const outAt = locate(wav, fromB64(r.out), r.outAt + r.latency / 1000, 0.05, 0.3)
    expect(outAt.corr).toBeGreaterThan(0.95)
    expect(Math.abs((outAt.at - r.outAt - 2 * hopIn) * 1000 - r.latency)).toBeLessThanOrEqual(HOP_SPREAD_MS)
  })

  test('after an overload, the new try comes 10 s later even on silence', async ({ page, origin, browserName }) => {
    test.skip(browserName !== 'firefox', 'only Firefox under Playwright is slow enough, and gives a silent stream no channel')
    await open(page, origin)
    const r: any = await page.evaluate(async () => {
      const lib = window.lib
      const ctx = new AudioContext({ sampleRate: 48000 })
      const player = new lib.Player(ctx)
      const buf = await player.load('speech.wav')
      const input = await lib.sdk.createCallInput({ stream: player.stream, audioContext: ctx })
      const log = lib.watch(input, ['mode'])
      await lib.until(() => input.mode === 'focus', 30000, 'models')
      player.play(buf, { delay: 0.1 })
      await lib.until(() => input.mode === 'raw' || ctx.currentTime > player.end + 1, 30000, 'overload')
      if (input.mode !== 'raw') return { kept: true }
      player.stop()
      const at = input.now()
      await lib.until(() => input.mode !== 'raw' || input.now() > at + 14, 20000, 'retry')
      input.dispose()
      return { kept: false, modes: log.map((e: any) => e.v) }
    })
    test.skip(r.kept, 'Firefox kept up with Focus: the overload path is not reached')
    expect(r.modes).toEqual([{ mode: 'focus' }, { mode: 'raw', reason: 'overload' }, { mode: 'focus' }])
  })

  test('normalize, speech, keepRecent, stats and trace each change something', async ({ page, origin, browserName }) => {
    test.skip(browserName === 'firefox', SLOW_FIREFOX)
    await open(page, origin)
    const r: any = await page.evaluate(async () => {
      const lib = window.lib
      const ctx = new AudioContext({ sampleRate: 48000 })
      const player = new lib.Player(ctx)
      const buf = await player.load('speech.wav')
      const base = await lib.sdk.createCallInput({ stream: player.stream, audioContext: ctx })
      const other = await lib.sdk.createCallInput({ stream: player.stream, audioContext: ctx, normalize: false, speech: { stopSecs: 2 }, keepRecent: 10, stats: true, trace: true })
      const logs = [lib.watch(base, ['speechend', 'diag']), lib.watch(other, ['speechend', 'diag'])]
      const traces = { base: 0, other: 0 }
      base.on('trace', (t: any[]) => (traces.base += t.length))
      other.on('trace', (t: any[]) => (traces.other += t.length))
      await lib.until(() => base.mode === 'focus' && other.mode === 'focus', 20000, 'focus mode')
      const recs = [base.record(buf.duration + 3), other.record(buf.duration + 3)]
      // a phone-like level, under the leveler's target
      player.play(buf, { delay: 0.2, gain: 0.25 })
      const [a, b] = await Promise.all(recs.map(async (p) => (await lib.stereo(await p)).right))
      const recent = [await base.recent(), await other.recent()]
      const stats = [await base.stats(), await other.stats()]
      base.dispose()
      other.dispose()
      const ends = logs.map((l: any[]) => l.filter((e) => e.type === 'speechend').map((e) => ({ end: e.v.end, at: e.now })))
      return { rms: [lib.rms(a, 32000, 64000), lib.rms(b, 32000, 64000)], ends, recent: recent.map((x: Blob | null) => x?.size ?? null), stats, traces }
    })
    // normalize (on by default): the quiet voice comes out louder
    expect(20 * Math.log10(r.rms[0] / r.rms[1])).toBeGreaterThan(3)
    // speech.stopSecs: the same last speech frame, told 1.4 s later (2 s of quiet asked instead of 0.6)
    expect(r.ends[0]).toHaveLength(1)
    expect(r.ends[1]).toHaveLength(1)
    expect(Math.abs(r.ends[1][0].end - r.ends[0][0].end)).toBeLessThan(0.05)
    expect(r.ends[1][0].at - r.ends[0][0].at).toBeGreaterThan(1.2)
    // keepRecent: the last seconds as a stereo WAV; none without it
    expect(r.recent[0]).toBeNull()
    expect(r.recent[1]).toBeGreaterThan(44 + 4 * 16000 * 5)
    // stats
    expect(r.stats[0]).toBeNull()
    expect(r.stats[1].seconds).toBeGreaterThan(5)
    expect(r.stats[1].speechSeconds).toBeGreaterThan(1.5)
    expect(r.stats[1].speechSeconds).toBeLessThan(r.stats[1].seconds)
    // trace
    expect(r.traces.base).toBe(0)
    expect(r.traces.other).toBeGreaterThan(20)
  })

  test('strength 0 and setEnabled(false): the raw mic at the chain delay, mode raw "off", then back', async ({ page, origin, browserName }) => {
    test.skip(browserName === 'firefox', SLOW_FIREFOX)
    await open(page, origin)
    const r: any = await page.evaluate(async () => {
      const lib = window.lib
      const ctx = new AudioContext({ sampleRate: 48000 })
      const player = new lib.Player(ctx)
      const buf = await player.load('mix.wav')
      const weak = await lib.sdk.createCallInput({ stream: player.stream, audioContext: ctx, strength: 0, normalize: false })
      const off = await lib.sdk.createCallInput({ stream: player.stream, audioContext: ctx, normalize: false })
      const modes = lib.watch(off, ['mode'])
      await lib.until(() => weak.mode === 'focus' && off.mode === 'focus', 20000, 'focus mode')
      const running = await off.setEnabled(false)
      const recs = [weak.record(buf.duration + 1), off.record(buf.duration + 1)]
      player.play(buf, { delay: 0.2 })
      const [a, b] = await Promise.all(recs.map(async (p) => lib.stereo(await p)))
      const back = await off.setEnabled(true)
      weak.dispose()
      off.dispose()
      return { running, back, modes: modes.map((e: any) => e.v), weak: [lib.b64(a.left), lib.b64(a.right)], off: [lib.b64(b.left), lib.b64(b.right)] }
    })
    expect(r.running).toBe(false)
    expect(r.back).toBe(true)
    expect(r.modes).toEqual([{ mode: 'focus' }, { mode: 'raw', reason: 'off' }, { mode: 'focus' }])
    for (const [l, rt] of [r.weak, r.off]) {
      const left = fromB64(l)
      const right = fromB64(rt)
      // in speech, where the gate is open: the mic itself, CHAIN samples later
      expect(residualDb(left, right, 24000, 32000, CHAIN)).toBeLessThan(-30)
    }
  })

  test('voiceprint, learn and farEnd: focus+print, the print learned while the user talks, not while the far end talks', async ({ page, origin, browserName }) => {
    test.skip(browserName === 'firefox', SLOW_FIREFOX)
    test.slow()
    await open(page, origin)
    const r: any = await page.evaluate(async () => {
      const lib = window.lib
      const ctx = new AudioContext({ sampleRate: 48000 })
      const player = new lib.Player(ctx)
      const buf = await player.load('speech.wav')
      const timbre = await lib.sdk.Timbre.create()
      const print = await timbre.embed((await lib.fixture('enroll.wav')).pcm)
      const printed = await lib.sdk.createCallInput({ stream: player.stream, audioContext: ctx, voiceprint: print })
      await lib.until(() => printed.mode !== 'raw', 20000, 'printed mode')
      const printedMode = printed.mode
      printed.dispose()

      // the far end: quiet for one input, talking (noise) the whole time for the other
      const quiet = ctx.createMediaStreamDestination()
      const talking = ctx.createMediaStreamDestination()
      const noise = ctx.createBuffer(1, ctx.sampleRate, ctx.sampleRate)
      noise.getChannelData(0).forEach((_: number, i: number, d: Float32Array) => (d[i] = (Math.random() * 2 - 1) * 0.1))
      const src = ctx.createBufferSource()
      src.buffer = noise
      src.loop = true
      src.connect(talking)
      src.start()
      const me = await lib.sdk.createCallInput({ stream: player.stream, audioContext: ctx, learn: true, farEnd: quiet.stream.getAudioTracks()[0] })
      const busy = await lib.sdk.createCallInput({ stream: player.stream, audioContext: ctx, learn: true, farEnd: talking.stream.getAudioTracks()[0] })
      const logs = [lib.watch(me, ['learn', 'voiceprint', 'mode']), lib.watch(busy, ['learn', 'voiceprint', 'mode'])]
      await lib.until(() => me.mode === 'focus' && busy.mode === 'focus', 20000, 'focus mode')
      await lib.sleep(2200)
      player.play(buf, { delay: 0.2, times: 4, gap: 0.5 })
      await lib.until(() => me.mode === 'focus+print' || ctx.currentTime > player.end + 2, 60000, 'learned')
      await lib.sleep(500)
      const out = { printedMode, me: logs[0], busy: logs[1], print: me.voiceprint ? timbre.verify(me.voiceprint, print) : null }
      me.dispose()
      busy.dispose()
      return out
    })
    expect(r.printedMode).toBe('focus+print')
    const progress = r.me.filter((e: any) => e.type === 'learn').map((e: any) => e.v.progress)
    expect(progress.length).toBeGreaterThan(0)
    expect(Math.max(...progress)).toBe(1)
    expect(r.me.some((e: any) => e.type === 'voiceprint')).toBe(true)
    expect(r.me.filter((e: any) => e.type === 'mode').map((e: any) => e.v.mode)).toContain('focus+print')
    // the print learned is the speaker's: the same voice as enroll.wav
    expect(r.print.same).toBe(true)
    expect(r.busy.filter((e: any) => e.type === 'learn' && e.v.progress > 0)).toEqual([])
    expect(r.busy.some((e: any) => e.type === 'voiceprint')).toBe(false)
  })
})
