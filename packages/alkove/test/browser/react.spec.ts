import { expect, SLOW_FIREFOX, test } from './fixtures'

declare global {
  interface Window {
    React: any
    hooks: any
  }
}

test.describe('alkove/react', () => {
  test.beforeEach(async ({ page, origin }) => {
    await page.goto(`${origin}/browser/react.html`)
    await page.waitForFunction(() => window.libReady === true)
    await page.evaluate(() => window.lib.setup())
  })

  test("useAgentInput in strict mode: one input, the user's phrase, disposed on unmount", async ({ page, browserName }) => {
    test.skip(browserName === 'firefox', SLOW_FIREFOX)
    const r: any = await page.evaluate(async () => {
      const { lib, React, hooks } = window
      const ctx = new AudioContext({ sampleRate: 48000 })
      const player = new lib.Player(ctx)
      const buf = await player.load('speech.wav')
      const phrases: number[] = []
      const inputs = new Set<any>()
      let state: any = null
      function Agent() {
        state = hooks.useAgentInput({ stream: player.stream, audioContext: ctx, on: { speechend: (e: any) => phrases.push(e.end - e.start) } })
        if (state.input) inputs.add(state.input)
        return React.createElement('p', null, state.status)
      }
      const root = React.createRoot(document.getElementById('root'))
      root.render(React.createElement(React.StrictMode, null, React.createElement(Agent)))
      await lib.until(() => state?.input, 30000, 'input opened')
      await state.input.ready
      await lib.until(() => state.mode === 'focus', 5000, 'focus')
      const opened = { status: state.status, mode: state.mode, text: document.getElementById('root')!.textContent }
      player.play(buf, { delay: 0.2 })
      await lib.until(() => ctx.currentTime > player.end + 1, 30000, 'played')
      const input = state.input
      root.unmount()
      return { opened, phrases: phrases.length, inputs: inputs.size, after: input.status }
    })
    expect(r.opened).toEqual({ status: 'running', mode: 'focus', text: 'running' })
    expect(r.phrases).toBeGreaterThan(0)
    expect(r.inputs).toBe(1)
    expect(r.after).toBe('disposed')
  })

  test('useCallInput gives the output track; enabled: false disposes the input', async ({ page }) => {
    const r: any = await page.evaluate(async () => {
      const { lib, React, hooks } = window
      const ctx = new AudioContext({ sampleRate: 48000 })
      const player = new lib.Player(ctx)
      let state: any = null
      let set: (on: boolean) => void = () => {}
      function Call() {
        const [on, setOn] = React.useState(true)
        set = setOn
        state = hooks.useCallInput({ enabled: on, stream: player.stream, audioContext: ctx })
        return null
      }
      const root = React.createRoot(document.getElementById('root'))
      root.render(React.createElement(Call))
      await lib.until(() => state?.track, 30000, 'track')
      const track = { kind: state.track.kind, live: state.track.readyState }
      const input = state.input
      set(false)
      await lib.until(() => state.status === 'off', 5000, 'off')
      root.unmount()
      return { track, after: input.status, input: state.input }
    })
    expect(r).toEqual({ track: { kind: 'audio', live: 'live' }, after: 'disposed', input: null })
  })
})
