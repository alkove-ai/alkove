import type { ConsoleMessage } from '@playwright/test'

import { MODELS } from '../../src/manifest'
import { expect, fromB64, locate, open, test, wavPcm } from './fixtures'

const FOCUS_FILE = 'alkove-focus-1-preview.alk'

test.describe('failures', () => {
  test('models in 404: raw "failed", the raw mic at the same delay, new tries after 5 s then 10 s, focus once they are back', async ({ page, origin, serverLog, browserName }) => {
    const token = `t${Date.now()}${browserName}`
    await open(page, origin, { modelBaseUrl: `/flaky/${token}/` })
    const run = page.evaluate(async () => {
      const lib = window.lib
      const ctx = new AudioContext({ sampleRate: 48000 })
      const player = new lib.Player(ctx)
      const buf = await player.load('speech.wav')
      const input = await lib.sdk.createCallInput({ stream: player.stream, audioContext: ctx })
      const log = lib.watch(input, ['mode', 'diag'])
      await lib.until(() => input.mode === 'raw' && log.some((e: any) => e.type === 'mode'), 10000, 'raw failed')
      // the raw mic goes out while the models are missing
      const recOut = await lib.record(ctx, input.track)
      const recIn = await lib.record(ctx, player.stream.getAudioTracks()[0])
      await lib.sleep(200)
      const when = player.play(buf, { delay: 0.2 })
      await lib.until(() => ctx.currentTime > player.end + 0.5, 30000, 'played')
      const out = recOut.stop()
      const inp = recIn.stop()
      await lib.until(() => input.mode === 'focus', 40000, 'models back')
      // the worklet tells its delay just after the models connect
      await lib.until(() => input.latency > 0, 2000, 'latency')
      const latency = input.latency
      input.dispose()
      return { log, latency, out: lib.b64(out), outAt: when - recOut.startTime, inp: lib.b64(inp), inAt: when - recIn.startTime }
    })
    // two failed tries, then the models come back
    await expect.poll(async () => (await serverLog(token)).filter((h) => h.path.endsWith(FOCUS_FILE) && h.status === 404).length, { timeout: 30_000, intervals: [200] }).toBeGreaterThanOrEqual(2)
    await fetch(`${origin}/__flaky/${token}/on`)
    const r: any = await run

    const tries = (await serverLog(token)).filter((h) => h.path.endsWith(FOCUS_FILE))
    expect(tries.map((h) => h.status)).toEqual([404, 404, 200])
    expect(Math.abs(tries[1]!.at - tries[0]!.at - 5000)).toBeLessThan(700)
    expect(Math.abs(tries[2]!.at - tries[1]!.at - 10000)).toBeLessThan(700)
    const failed = r.log.filter((e: any) => e.type === 'diag' && e.v.code === 'models.failed').map((e: any) => e.v.message)
    expect(failed).toHaveLength(2)
    expect(failed[0]).toContain('retry in 5 s')
    expect(failed[1]).toContain('retry in 10 s')
    const modes = r.log.filter((e: any) => e.type === 'mode').map((e: any) => e.v)
    expect(modes[0]).toEqual({ mode: 'raw', reason: 'failed' })
    expect(modes.at(-1)).toEqual({ mode: 'focus' })
    expect(r.latency).toBe(149)
    // raw, the mic comes out delayed as the processed path will be: the switch to Focus moves nothing in time
    const wav = wavPcm('speech.wav')
    const hopIn = locate(wav, fromB64(r.inp), r.inAt, 0.01, 0.15).at - r.inAt
    const outAt = locate(wav, fromB64(r.out), r.outAt + 0.149, 0.05, 0.3)
    expect(outAt.corr).toBeGreaterThan(0.95)
    expect(Math.abs((outAt.at - r.outAt - 2 * hopIn) * 1000 - r.latency)).toBeLessThanOrEqual(12)
  })

  test('agent input with the models in 404: its audio keeps coming, the raw mic in the asked format', async ({ page, origin, browserName }) => {
    const token = `a${Date.now()}${browserName}`
    await open(page, origin, { modelBaseUrl: `/flaky/${token}/` })
    const r: any = await page.evaluate(async () => {
      const lib = window.lib
      const ctx = new AudioContext({ sampleRate: 48000 })
      const player = new lib.Player(ctx)
      const buf = await player.load('speech.wav')
      const agent = await lib.sdk.createAgentInput({ stream: player.stream, audioContext: ctx, output: { format: 's16', rate: 24000 } })
      await lib.until(() => agent.mode === 'raw', 10000, 'raw')
      const packets: ArrayBuffer[] = []
      agent.on('audio', (pcm: ArrayBuffer) => packets.push(pcm))
      player.play(buf, { delay: 0.1 })
      await lib.until(() => ctx.currentTime > player.end + 0.3, 30000, 'played')
      const mode = { mode: agent.mode, reason: (agent as any)._mode?.reason }
      agent.dispose()
      let peak = 0
      for (const p of packets) for (const x of new Int16Array(p)) peak = Math.max(peak, Math.abs(x))
      return { n: packets.length, sizes: [...new Set(packets.map((p) => p.byteLength))], peak, seconds: buf.duration, mode }
    })
    expect(r.mode.mode).toBe('raw')
    // 32 ms packets of 24 kHz s16 for the whole playback
    expect(r.sizes).toEqual([1536])
    expect(r.n).toBeGreaterThanOrEqual(Math.floor((r.seconds * 1000) / 32) - 2)
    expect(r.peak).toBeGreaterThan(3000)
  })

  test('input.ready: rejects while the models fail, resolves once they are back; the SDK\'s own mic gets the browser\'s noise suppression and gain while raw', async ({ page, origin, browserName }) => {
    test.skip(browserName !== 'chromium', 'the SDK opens its own mic: fake devices, Chromium')
    const token = `r${Date.now()}${browserName}`
    await open(page, origin, { modelBaseUrl: `/flaky/${token}/` })
    const first: any = await page.evaluate(async () => {
      const lib = window.lib
      const input = await lib.sdk.createAgentInput()
      const w = window as any
      w.agent = input
      w.browser = []
      input.on('diag', (d: any) => d.code === 'mic.browser' && w.browser.push(d.noiseSuppression))
      const failed = await input.ready.then(
        () => 'resolved',
        (e: any) => `${e.name} ${e.code}`,
      )
      const settings = () => input.stream.getAudioTracks()[0].getSettings()
      const before = settings().noiseSuppression
      await lib.until(() => w.browser.length === 1, 4000, 'browser processing on')
      const raw = settings()
      return { failed, before, raw: { ns: raw.noiseSuppression, agc: raw.autoGainControl, ec: raw.echoCancellation }, mode: input.mode }
    })
    expect(first.failed).toMatch(/^AlkoveError (models\.fetch|engine\.failed)$/)
    // a later call waits for the next try instead of failing at once
    await page.evaluate(() => {
      const w = window as any
      w.later = 'waiting'
      w.agent.ready.then(
        () => (w.later = 'resolved'),
        (e: any) => (w.later = `${e.name} ${e.code}`),
      )
    })
    expect(first.mode).toBe('raw')
    expect(first.before).toBe(false)
    expect(first.raw).toEqual({ ns: true, agc: true, ec: true })
    await fetch(`${origin}/__flaky/${token}/on`)
    const back: any = await page.evaluate(async () => {
      const lib = window.lib
      const w = window as any
      const input = w.agent
      await lib.until(() => input.mode === 'focus', 40000, 'models back')
      const ready = await input.ready.then(() => 'resolved', (e: any) => `${e.name} ${e.code}`)
      const later = w.later
      await lib.until(() => w.browser.length === 2, 4000, 'browser processing off')
      const s = input.stream.getAudioTracks()[0].getSettings()
      // the mic opened twice more: its sound still reaches the models
      let peak = 0
      input.on('audio', (pcm: ArrayBuffer) => (peak = Math.max(peak, lib.rms(new Float32Array(pcm)))))
      await lib.sleep(3000)
      input.dispose()
      return { ready, later, browser: w.browser, ns: s.noiseSuppression, agc: s.autoGainControl, ec: s.echoCancellation, sound: peak > 0.001 }
    })
    expect(back).toEqual({ ready: 'resolved', later: 'resolved', browser: [true, false], ns: false, agc: false, ec: true, sound: true })
  })

  test('a slow first load: the browser\'s noise suppression after 2 s, off once the models run, the mic live and heard', async ({ page, origin, browserName }) => {
    test.skip(browserName !== 'chromium', 'the SDK opens its own mic: fake devices, Chromium')
    const token = `h${Date.now()}${browserName}`
    await open(page, origin, { modelBaseUrl: `/held/${token}/` })
    await page.evaluate(async () => {
      const w = window as any
      w.agent = await w.lib.sdk.createAgentInput()
      w.browser = []
      w.agent.on('diag', (d: any) => d.code === 'mic.browser' && w.browser.push(d.applied))
      await w.lib.until(() => w.browser.length === 1, 4000, 'browser processing on while loading')
    })
    await fetch(`${origin}/__flaky/${token}/on`)
    const r: any = await page.evaluate(async () => {
      const lib = window.lib
      const w = window as any
      const input = w.agent
      await input.ready
      await lib.until(() => w.browser.length === 2, 4000, 'browser processing off')
      const track = input.stream.getAudioTracks()[0]
      let peak = 0
      input.on('audio', (pcm: ArrayBuffer) => (peak = Math.max(peak, lib.rms(new Float32Array(pcm)))))
      await lib.sleep(3000)
      const out = { applied: w.browser, mode: input.mode, live: track.readyState, ns: track.getSettings().noiseSuppression, sound: peak > 0.001 }
      input.dispose()
      return out
    })
    expect(r).toEqual({ applied: [true, false], mode: 'focus', live: 'live', ns: false, sound: true })
  })

  test('input.latency is the real delay before the models are in too', async ({ page, origin, browserName }) => {
    const token = `l${Date.now()}${browserName}`
    await open(page, origin, { modelBaseUrl: `/flaky/${token}/` })
    const latency = await page.evaluate(async () => {
      const lib = window.lib
      const ctx = new AudioContext({ sampleRate: 48000 })
      const input = await lib.sdk.createCallInput({ stream: new lib.Player(ctx).stream, audioContext: ctx })
      await lib.until(() => input.mode === 'raw' && input.status === 'running', 10000, 'raw')
      await lib.sleep(500)
      const ms = input.latency
      input.dispose()
      return ms
    })
    expect(latency).toBe(149)
  })

  test('a model with changed bytes is refused (models.hash): raw "failed", Alkove.process rejects, nothing cached', async ({ page, origin }) => {
    await open(page, origin, { modelBaseUrl: '/corrupt/' })
    const r: any = await page.evaluate(async () => {
      const lib = window.lib
      const ctx = new AudioContext({ sampleRate: 48000 })
      const input = await lib.sdk.createCallInput({ stream: new lib.Player(ctx).stream, audioContext: ctx })
      const log = lib.watch(input, ['mode', 'diag'])
      await lib.until(() => log.some((e: any) => e.type === 'diag' && e.v.code === 'models.failed'), 10000, 'models.failed')
      const mode = { mode: input.mode, last: log.filter((e: any) => e.type === 'mode').at(-1)?.v }
      input.dispose()
      const error = await lib.sdk.Alkove.process(await lib.bytes('/fixtures/speech.wav'), { cue: true }).then(
        () => 'resolved',
        (e: any) => ({ name: e.name, code: e.code, message: e.message }),
      )
      return { mode, failed: log.find((e: any) => e.v.code === 'models.failed').v, error, cached: await lib.sdk.Alkove.cached('call') }
    })
    expect(r.mode).toEqual({ mode: 'raw', last: { mode: 'raw', reason: 'failed' } })
    expect(r.failed.error).toContain('sha256')
    expect(r.error).toMatchObject({ name: 'AlkoveError', code: 'models.hash' })
    expect(r.error.message).toMatch(new RegExp(`alkove-cue-1-preview: sha256 [0-9a-f]{64}, expected ${MODELS.cue.sha256}`))
    expect(r.cached).toBe(false)
  })

  test('Alkove.supported(), and what a missing AudioWorklet or SIMD does', async ({ page, origin, context }) => {
    await open(page, origin)
    expect(await page.evaluate(() => window.lib.sdk.Alkove.supported())).toEqual({ worklet: true, simd: true, secure: true })

    const noWorklet = await context.newPage()
    await noWorklet.addInitScript(() => delete (window as any).AudioWorkletNode)
    await open(noWorklet, origin)
    const w: any = await noWorklet.evaluate(async () => {
      const lib = window.lib
      const error = await lib.sdk.createCallInput({ stream: new lib.Player().stream }).then(
        (i: any) => (i.dispose(), 'resolved'),
        (e: any) => `${e.name} ${e.code}`,
      )
      return { supported: lib.sdk.Alkove.supported(), error }
    })
    expect(w).toEqual({ supported: { worklet: false, simd: true, secure: true }, error: 'AlkoveError browser.unsupported' })

    const noSimd = await context.newPage()
    await noSimd.addInitScript(() => (WebAssembly.validate = () => false))
    await open(noSimd, origin)
    const s: any = await noSimd.evaluate(async () => {
      const lib = window.lib
      const input = await lib.sdk.createCallInput({ stream: new lib.Player().stream })
      await lib.sleep(500)
      const out = { supported: lib.sdk.Alkove.supported(), mode: input.mode, status: input.status }
      input.dispose()
      return out
    })
    // the input still gives sound: the raw mic
    expect(s).toEqual({ supported: { worklet: true, simd: false, secure: true }, mode: 'raw', status: 'running' })
  })

  test('without SIMD, the app learns why the input stays raw (mode reason "unsupported", diag engine.unsupported)', async ({ context, origin }) => {
    const page = await context.newPage()
    await page.addInitScript(() => (WebAssembly.validate = () => false))
    await open(page, origin)
    const seen: any = await page.evaluate(async () => {
      const lib = window.lib
      const seen: unknown[] = []
      lib.sdk.Alkove.onDiag((d: any) => seen.push(d.code))
      const input = await lib.sdk.createCallInput({ stream: new lib.Player().stream })
      input.on('mode', (m: any) => seen.push(m))
      input.on('diag', (d: any) => seen.push(d.code))
      await lib.sleep(500)
      input.dispose()
      return seen
    })
    expect(seen).toContainEqual({ mode: 'raw', reason: 'unsupported' })
    expect(seen).toContain('engine.unsupported')
  })

  test('Alkove.preload and Alkove.cached: everything for a usage ahead, with progress; each option asks for more', async ({ page, origin }) => {
    await open(page, origin)
    const r: any = await page.evaluate(async () => {
      const { sdk } = window.lib
      const progress: { loaded: number; total: number }[] = []
      const before = await sdk.Alkove.cached('call')
      await sdk.Alkove.preload('call', { onProgress: (p: { loaded: number; total: number }) => progress.push(p) })
      const after = { call: await sdk.Alkove.cached('call'), print: await sdk.Alkove.cached('call', { voiceprint: true }), wake: await sdk.Alkove.cached('wake') }
      await sdk.Alkove.preload('wake', { typedWords: true })
      const wake = { plain: await sdk.Alkove.cached('wake'), typed: await sdk.Alkove.cached('wake', { typedWords: true }), print: await sdk.Alkove.cached('call', { voiceprint: true }) }
      return { before, after, wake, progress }
    })
    expect(r.before).toBe(false)
    expect(r.after).toEqual({ call: true, print: false, wake: false })
    expect(r.wake).toEqual({ plain: true, typed: true, print: false })
    // Focus and Cue, in bytes
    expect(r.progress.at(-1)).toEqual({ loaded: MODELS.focus.bytes + MODELS.cue.bytes, total: MODELS.focus.bytes + MODELS.cue.bytes })
  })

  test('status "blocked" when the page has had no user gesture, "running" after resume() in a click (Safari)', async ({ page, origin, browserName }) => {
    test.skip(browserName !== 'webkit', 'Chromium and Firefox run here with autoplay allowed')
    const blocked = page.waitForEvent('console', { predicate: (m: ConsoleMessage) => /^ALKOVE_BLOCKED/.test(m.text()) })
    await page.goto(`${origin}/browser/blocked.html`)
    const b = (await blocked).text()
    expect(b.startsWith('ALKOVE_BLOCKED_ERROR'), b).toBe(false)
    expect(JSON.parse(b.slice('ALKOVE_BLOCKED '.length))).toEqual({ status: 'blocked', ctx: 'suspended' })
    const resumed = page.waitForEvent('console', { predicate: (m: ConsoleMessage) => m.text().startsWith('ALKOVE_RESUMED ') })
    await page.click('#resume')
    const after = JSON.parse((await resumed).text().slice('ALKOVE_RESUMED '.length))
    // WebKit passes through its own "interrupted" state on the way
    expect(after.status).toBe('running')
    expect(after.statuses[0]).toBe('blocked')
    expect(after.statuses.at(-1)).toBe('running')
  })

  test('the packaged wasm is found next to the package, without models.engine or assetBaseUrl', async ({ page, origin, serverLog }) => {
    const since = Date.now()
    await open(page, origin)
    const r = await page.evaluate(async () => {
      const { sdk, bytes } = window.lib
      return sdk.Alkove.process(await bytes('/fixtures/speech.wav'), { cue: true }).then(
        (x: any) => x.speech.length,
        (e: any) => `${e.name} ${e.code}: ${e.message}`,
      )
    })
    expect(r).toBe(1)
    const wasm = (await serverLog()).filter((h) => h.at >= since && h.path.endsWith('.wasm'))
    expect(wasm.map((h) => [h.path, h.status])).toEqual([['/dist/alkove-engine.wasm', 200]])
  })
})
