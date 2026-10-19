import type { ConsoleMessage } from '@playwright/test'

import { MODELS } from '../../src/manifest'
import { expect, open, test } from './fixtures'

// every request the page may make: the test page, its scripts, the package, the models, the fixtures
const ALLOWED = [/^\/csp\.html$/, /^\/browser\/(csp|lib)\.js$/, /^\/dist\/[\w./-]+$/, /^\/models\/alkove-[a-z]+-1-preview\.alk$/, /^\/fixtures\/[\w.]+$/, /^\/favicon\.ico$/]

test.describe('network guarantee', () => {
  test('under the strict CSP, cache empty: every model and script from the page origin, no violation, no other request', async ({ page, origin, serverLog, browserName }) => {
    test.slow()
    const policy: { text: string; probe: boolean }[] = []
    let probe = false
    page.on('console', (m: ConsoleMessage) => {
      const text = m.text()
      if (text.startsWith('ALKOVE_PROBE')) probe = true
      else if (/content.security.policy|refused to/i.test(text) && !/favicon/i.test(text)) policy.push({ text, probe })
    })
    const result = page.waitForEvent('console', { predicate: (m) => /^ALKOVE_CSP(_ERROR)? /.test(m.text()), timeout: 150_000 })
    const done = page.waitForEvent('console', { predicate: (m) => m.text().startsWith('ALKOVE_DONE '), timeout: 160_000 })
    const ready = page.waitForEvent('console', { predicate: (m) => m.text().startsWith('ALKOVE_READY ') })
    const since = Date.now()
    await page.goto(`${origin}/csp.html`)
    await ready
    await page.click('#go')
    const msg = (await result).text()
    expect(msg.startsWith('ALKOVE_CSP_ERROR'), msg).toBe(false)
    const r = JSON.parse(msg.slice('ALKOVE_CSP '.length))

    // a fresh profile: nothing cached before, everything after
    expect(r.cachedBefore).toBe(false)
    expect(r.cachedAfter).toBe(true)
    expect(r.process).toEqual({ speech: 1, wakes: ['amendement'] })
    expect(r.timbre).toBe(192)
    expect(r.wake).toEqual(['amendement'])
    expect(r.call.modes).toContain('focus')
    // Firefox falls behind on Focus under Playwright and goes raw: no speech events to count there
    if (browserName !== 'firefox') expect(r.call.speech).toBe(1)
    expect(r.diags).not.toContain('models.default-host')

    // nothing refused by the policy while the SDK worked
    expect(r.violations.filter((v: any) => !/favicon/.test(v.blocked))).toEqual([])
    expect(policy.filter((p) => !p.probe)).toEqual([])
    // and the policy is really applied
    expect(JSON.parse((await done).text().slice('ALKOVE_DONE '.length))).toEqual({ evalBlocked: true })

    // the server saw only expected paths, all the models among them; the route guard of the fixture saw nothing else
    const hits = (await serverLog()).filter((h) => h.at >= since)
    expect(hits.filter((h) => !ALLOWED.some((re) => re.test(h.path))).map((h) => h.path)).toEqual([])
    for (const f of Object.values(MODELS)) expect(hits.some((h) => h.path === `/models/${f.file}` && h.status === 200)).toBe(true)
    expect(hits.some((h) => h.path === '/dist/alkove-engine.wasm' && h.status === 200)).toBe(true)
  })

  test('without modelBaseUrl the models go to Hugging Face: the guard catches it, the SDK says so', async ({ page, origin, foreign }) => {
    await open(page, origin, { modelBaseUrl: undefined })
    const r: any = await page.evaluate(async () => {
      const { sdk } = window.lib
      const diags: string[] = []
      sdk.Alkove.onDiag((d: any) => diags.push(d.code))
      const error = await sdk.Spot.compile('amendement', { lang: 'fr' }).then(
        () => 'resolved',
        (e: any) => `${e.name} ${e.code}`,
      )
      return { error, diags }
    })
    expect(r.error).toBe('AlkoveError models.fetch')
    const lex = MODELS.lex
    // before the files are on Hugging Face (no revision), the SDK refuses at once and asks for modelBaseUrl
    expect(r.diags.includes('models.default-host')).toBe(!!lex.revision)
    expect(foreign).toEqual(lex.revision ? [`https://huggingface.co/alkove-ai/${lex.name}/resolve/${lex.revision}/${lex.file}`] : [])
    // seen and expected: the fixture's check is for requests nobody asked for
    foreign.length = 0
  })
})
