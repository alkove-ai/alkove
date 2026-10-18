// runs under the strict CSP of the CSP guide (served by test/browser/server.ts), reports through the console
import * as lib from '/browser/lib.js'

const out = { violations: [], ua: navigator.userAgent }
document.addEventListener('securitypolicyviolation', (e) => out.violations.push({ directive: e.effectiveDirective, blocked: e.blockedURI, source: e.sourceFile }))
const send = (tag, value) => console.log(`${tag} ${JSON.stringify(value)}`)

// a click first: Safari keeps an AudioContext made without a user gesture suspended
const clicked = new Promise((resolve) => document.getElementById('go').addEventListener('click', resolve, { once: true }))
send('ALKOVE_READY', true)
await clicked

try {
  const { sdk } = lib
  out.cachedBefore = await sdk.Alkove.cached('call')
  lib.setup()
  const diags = []
  sdk.Alkove.onDiag((d) => diags.push(d.code))

  // background worker: Lex, Cue, Spot, Timbre
  const word = await sdk.Spot.compile('amendement', { lang: 'fr' })
  const p = await sdk.Alkove.process(await lib.bytes('/fixtures/speech.wav'), { cue: true, spot: { words: [word] } })
  out.process = { speech: p.speech.length, wakes: p.wakes.map((w) => w.word) }
  const timbre = await sdk.Timbre.create()
  out.timbre = (await timbre.embed((await lib.fixture('enroll.wav')).pcm)).embedding.length

  // real time: the worklet from a blob, the real-time worker, Spot on the raw mic
  const ctx = new AudioContext({ sampleRate: 48000 })
  const player = new lib.Player(ctx)
  const buf = await player.load('speech.wav')
  const wake = await sdk.createWakeListener({ stream: player.stream, audioContext: ctx, words: [word], context: 'anywhere' })
  const wakes = []
  wake.on('wake', (w) => wakes.push(w.word))
  await lib.until(() => wake.mode === 'spot', 20000, 'spot mode')
  player.play(buf, { delay: 0.2 })
  await lib.until(() => ctx.currentTime > player.end + 1, 30000, 'played')
  wake.dispose()
  out.wake = wakes

  // Focus and Cue behind it
  const call = await sdk.createCallInput({ stream: player.stream, audioContext: ctx })
  const modes = []
  let speech = 0
  call.on('mode', (m) => modes.push(m.mode))
  call.on('speechend', () => speech++)
  await lib.until(() => modes.includes('focus'), 20000, 'focus mode')
  player.play(buf, { delay: 0.2 })
  await lib.until(() => ctx.currentTime > player.end + 1, 30000, 'played')
  call.dispose()
  out.call = { modes, speech }

  out.cachedAfter = await sdk.Alkove.cached('call')
  out.diags = diags
  send('ALKOVE_CSP', out)

  // the CSP is really on: a string run as code is refused
  send('ALKOVE_PROBE', 'next')
  let evalBlocked = false
  try {
    new Function('return 1')()
  } catch {
    evalBlocked = true
  }
  send('ALKOVE_DONE', { evalBlocked })
} catch (e) {
  send('ALKOVE_CSP_ERROR', `${e?.name}: ${e?.message} ${e?.stack ?? ''}`)
}
