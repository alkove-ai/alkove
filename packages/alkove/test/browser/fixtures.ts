import { readFileSync } from 'node:fs'
import { join } from 'node:path'
import { pathToFileURL } from 'node:url'

import { test as base, expect, type Page } from '@playwright/test'

export { expect }

const pkg = join(import.meta.dirname, '../..')

declare global {
  interface Window {
    // the page library (pages/lib.js) and the built package, typed loosely: the tests drive the public API as an app would
    lib: any
    libReady: boolean
  }
}

export interface Hit {
  path: string
  status: number
  at: number
}

interface Fixtures {
  origin: string
  // requests that left the test origin; the page fixture fails the test on any, unless a test empties it itself
  foreign: string[]
  page: Page
  serverLog: (token?: string) => Promise<Hit[]>
}

export const test = base.extend<Fixtures>({
  origin: async ({}, use) => use(process.env.ALKOVE_ORIGIN!),
  foreign: async ({}, use) => use([]),
  page: async ({ page, origin, foreign }, use) => {
    await page.context().route('**/*', (route) => {
      const url = route.request().url()
      if (new URL(url).origin === origin) return route.continue()
      foreign.push(url)
      return route.abort('blockedbyclient')
    })
    const errors: string[] = []
    page.on('pageerror', (e) => errors.push(String(e)))
    await use(page)
    expect(foreign, 'requests outside the test origin').toEqual([])
    expect(errors, 'uncaught errors in the page').toEqual([])
  },
  serverLog: async ({ origin }, use) => use(async (token = '') => (await fetch(`${origin}/__log?token=${encodeURIComponent(token)}`)).json() as Promise<Hit[]>),
})

// the test page, its library loaded, the package configured on the test origin
export async function open(page: Page, origin: string, config: Record<string, unknown> = {}): Promise<void> {
  await page.goto(`${origin}/`)
  await page.waitForFunction(() => window.libReady === true)
  await page.evaluate((c) => window.lib.setup(c), config)
}

export const fromB64 = (s: string) => {
  const b = Buffer.from(s, 'base64')
  return new Float32Array(b.buffer.slice(b.byteOffset, b.byteOffset + b.byteLength))
}

export function wavPcm(name: string): Float32Array {
  const b = readFileSync(join(pkg, 'test/fixtures', name))
  const v = new DataView(b.buffer, b.byteOffset, b.byteLength)
  let o = 12
  while (o + 8 <= b.length) {
    const id = b.toString('latin1', o, o + 4)
    const size = v.getUint32(o + 4, true)
    if (id === 'data') {
      const pcm = new Float32Array(size >> 1)
      for (let i = 0; i < pcm.length; i++) pcm[i] = v.getInt16(o + 8 + 2 * i, true) / 32768
      return pcm
    }
    o += 8 + size + (size & 1)
  }
  throw new Error('no data chunk')
}

export const SECONDS = 92352 / 16000
export const SLOW_FIREFOX = 'Firefox runs wasm on its baseline compiler under Playwright (Juggler debugs the page): Focus is slower than real time there'

let node: Promise<any> | null = null

// the same built package in Node: the reference the browser must match
export function nodeAlkove(): Promise<any> {
  return (node ??= (async () => {
    const dist = process.env.ALKOVE_DIST!
    const m = await import(pathToFileURL(join(dist, 'index.js')).href)
    m.Alkove.configure({ modelDir: join(pkg, '.cache/models') })
    return m
  })())
}

// a fixture through Alkove.process in Node, with Cue and the word « amendement »
export async function reference(name: string, opts: Record<string, unknown> = {}): Promise<any> {
  const { Alkove, Spot } = await nodeAlkove()
  const word = await Spot.compile('amendement', { lang: 'fr' })
  return Alkove.process(new Uint8Array(readFileSync(join(pkg, 'test/fixtures', name))), { cue: true, spot: { words: [word] }, ...opts })
}

// ---- comparing audio

// 48 kHz to 16 kHz: windowed-sinc low-pass at 7.2 kHz, then one sample in three (an output that came from 16 kHz has nothing above 8 kHz)
export function decimate(x: Float32Array, factor: number): Float32Array {
  const taps = 16 * factor + 1
  const mid = (taps - 1) / 2
  const cut = 0.9 / factor / 2
  const h = new Float32Array(taps)
  let sum = 0
  for (let i = 0; i < taps; i++) {
    const t = i - mid
    const sinc = t === 0 ? 2 * cut : Math.sin(2 * Math.PI * cut * t) / (Math.PI * t)
    h[i] = sinc * (0.42 - 0.5 * Math.cos((2 * Math.PI * i) / (taps - 1)) + 0.08 * Math.cos((4 * Math.PI * i) / (taps - 1)))
    sum += h[i]!
  }
  const out = new Float32Array(Math.floor(x.length / factor))
  for (let n = 0; n < out.length; n++) {
    let s = 0
    const c = n * factor
    for (let k = 0; k < taps; k++) {
      const j = c + k - mid
      if (j >= 0 && j < x.length) s += h[k]! * x[j]!
    }
    out[n] = s / sum
  }
  return out
}

const dot = (a: Float32Array, b: Float32Array, ao: number, bo: number, n: number) => {
  let s = 0
  for (let i = 0; i < n; i++) s += a[ao + i]! * b[bo + i]!
  return s
}

// the shift of `x` against `ref` over [from, from + n) of ref, searched in [lo, hi] samples, and the correlation there
export function align(ref: Float32Array, x: Float32Array, from: number, n: number, lo: number, hi: number): { lag: number; corr: number } {
  let best = { lag: lo, corr: -Infinity }
  const rr = dot(ref, ref, from, from, n)
  for (let lag = lo; lag <= hi; lag++) {
    if (from + lag < 0 || from + lag + n > x.length) continue
    const c = dot(ref, x, from, from + lag, n) / Math.sqrt(rr * dot(x, x, from + lag, from + lag, n) + 1e-20)
    if (c > best.corr) best = { lag, corr: c }
  }
  return best
}

// how far `x` (shifted by `lag`) is from `ref` once scaled to it: residual energy in dB under the reference
export function residualDb(ref: Float32Array, x: Float32Array, from: number, n: number, lag: number): number {
  const g = dot(ref, x, from, from + lag, n) / (dot(x, x, from + lag, from + lag, n) + 1e-20)
  let e = 0
  for (let i = 0; i < n; i++) e += (ref[from + i]! - g * x[from + lag + i]!) ** 2
  return 10 * Math.log10(e / (dot(ref, ref, from, from, n) + 1e-20))
}

// where `ref` (16 kHz) starts in a 48 kHz recording, near `approx` s, to the 48 kHz sample, and how alike they are there
export function locate(ref: Float32Array, rec48: Float32Array, approx: number, before: number, after: number, from = 16000, n = 32000): { at: number; corr: number; residualDb: number } {
  let best = { at: 0, corr: -Infinity, residualDb: 0 }
  for (const ph of [0, 1, 2]) {
    const x = decimate(rec48.subarray(ph), 3)
    const c = Math.round(((approx * 48000 - ph) / 3))
    const a = align(ref, x, from, n, c - Math.round(before * 16000), c + Math.round(after * 16000))
    if (a.corr > best.corr) best = { at: (3 * a.lag + ph) / 48000, corr: a.corr, residualDb: residualDb(ref, x, from, n, a.lag) }
  }
  return best
}

export const rms = (x: Float32Array, from = 0, to = x.length) => {
  let s = 0
  for (let i = from; i < to; i++) s += x[i]! * x[i]!
  return Math.sqrt(s / Math.max(1, to - from))
}
