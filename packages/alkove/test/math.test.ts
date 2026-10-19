import { describe, expect, it } from 'vitest'

import { cos, exp, ln, log10, pow, sin } from '../src/dsp/math'

const bits = new Float64Array(1)
const asInt = new BigInt64Array(bits.buffer)
const ordered = (x: number) => {
  bits[0] = x
  const v = asInt[0]!
  return v < 0n ? -(v & 0x7fffffffffffffffn) : v
}
const ulps = (a: number, b: number) => Number(ordered(a) > ordered(b) ? ordered(a) - ordered(b) : ordered(b) - ordered(a))

describe('math without the host’s rounding', () => {
  it('stays within an ulp or two of Math on exp, ln and log10, and 3e-14 on pow', () => {
    let worst = { exp: 0, ln: 0, log10: 0, pow: 0 }
    let s = 1
    const next = () => (s = (s * 1664525 + 1013904223) >>> 0) / 2 ** 32
    for (let i = 0; i < 200_000; i++) {
      const x = (next() - 0.5) * 1400
      const y = 10 ** ((next() - 0.5) * 600)
      const b = 10 ** ((next() - 0.5) * 8)
      const e = (next() - 0.5) * 20
      worst = {
        exp: Math.max(worst.exp, ulps(exp(x), Math.exp(x))),
        ln: Math.max(worst.ln, ulps(ln(y), Math.log(y))),
        log10: Math.max(worst.log10, ulps(log10(y), Math.log10(y))),
        pow: Math.max(worst.pow, Math.abs(pow(b, e) / b ** e - 1) / 2 ** -52),
      }
    }
    expect(worst.exp).toBeLessThanOrEqual(1)
    expect(worst.ln).toBeLessThanOrEqual(1)
    expect(worst.log10).toBeLessThanOrEqual(2)
    // exp(e ln b): the error of ln b grows with e
    expect(worst.pow).toBeLessThan(128)
  })

  it('stays within 2.3e-16 of Math on sin and cos, out to the resampler’s widest argument', () => {
    let worst = 0
    for (let i = -200_000; i <= 200_000; i++) {
      const x = i * 1e-4 * Math.PI
      worst = Math.max(worst, Math.abs(sin(x) - Math.sin(x)), Math.abs(cos(x) - Math.cos(x)))
    }
    expect(worst).toBeLessThan(2 ** -52)
  })

  it('gives the edges exactly', () => {
    expect(exp(0)).toBe(1)
    expect(exp(-Infinity)).toBe(0)
    expect(exp(Infinity)).toBe(Infinity)
    expect(exp(800)).toBe(Infinity)
    expect(exp(-800)).toBe(0)
    expect(exp(-740)).toBeGreaterThan(0)
    expect(ln(1)).toBe(0)
    expect(ln(0)).toBe(-Infinity)
    expect(ln(Infinity)).toBe(Infinity)
    expect(ln(-1)).toBeNaN()
    expect(ln(5e-324)).toBeCloseTo(Math.log(5e-324), 12)
    expect(Number.isNaN(exp(NaN)) && Number.isNaN(ln(NaN))).toBe(true)
    expect(sin(0)).toBe(0)
    expect(cos(0)).toBe(1)
  })
})
