// Math.exp, Math.log, Math.sin, ** … may round differently from one JS engine to another: these use + - * / only (fdlibm), so every browser gives the same bits

const LN2_HI = 6.93147180369123816490e-1
const LN2_LO = 1.90821492927058770002e-10
const INV_LN2 = 1.44269504088896338700
const LN10 = 2.302585092994045684

const view = new DataView(new ArrayBuffer(8))

// 2^n for an integer n of the normal range, built from its bits
function pow2(n: number): number {
  view.setUint32(0, (n + 1023) << 20)
  view.setUint32(4, 0)
  return view.getFloat64(0)
}

export function exp(x: number): number {
  if (x !== x) return x
  if (x > 709.782712893384) return Infinity
  if (x < -745.1332191019411) return 0
  const k = Math.round(x * INV_LN2)
  const hi = x - k * LN2_HI
  const lo = k * LN2_LO
  const r = hi - lo
  const z = r * r
  const c = r - z * (1.66666666666666019037e-1 + z * (-2.77777777770155933842e-3 + z * (6.61375632143793436117e-5 + z * (-1.65339022054652515390e-6 + z * 4.13813679705723846039e-8))))
  const y = 1 - (lo - (r * c) / (2 - c) - hi)
  // in two steps: 2^k alone leaves the normal range near the ends
  const h = k >> 1
  return y * pow2(h) * pow2(k - h)
}

export function ln(x: number): number {
  if (x !== x || x < 0) return NaN
  if (x === 0) return -Infinity
  if (x === Infinity) return x
  let k = 0
  if (x < 2.2250738585072014e-308) {
    x *= 18014398509481984
    k = -54
  }
  view.setFloat64(0, x)
  const high = view.getUint32(0)
  k += (high >>> 20) - 1023
  // the mantissa in [sqrt(2)/2, sqrt(2)): k one more when it starts above sqrt(2)
  let mant = high & 0xfffff
  const i = (mant + 0x95f64) & 0x100000
  k += i >>> 20
  mant |= i ^ 0x3ff00000
  view.setUint32(0, mant)
  const f = view.getFloat64(0) - 1
  const s = f / (2 + f)
  const z = s * s
  const w = z * z
  const t1 = w * (3.999999999940941908e-1 + w * (2.222219843214978396e-1 + w * 1.531383769920937332e-1))
  const t2 = z * (6.666666666666735130e-1 + w * (2.857142874366239149e-1 + w * (1.818357216161805012e-1 + w * 1.479819860511658591e-1)))
  const hfsq = 0.5 * f * f
  return k * LN2_HI - (hfsq - (s * (hfsq + t2 + t1) + k * LN2_LO) - f)
}

export const log10 = (x: number): number => ln(x) / LN10

// b^e for b > 0
export const pow = (b: number, e: number): number => exp(e * ln(b))

const PIO2_HI = 1.57079632673412561417
const PIO2_LO = 6.07710050650619224932e-11
const INV_PIO2 = 6.36619772367581382433e-1

function sinK(x: number): number {
  const z = x * x
  return x + x * z * (-1.66666666666666324348e-1 + z * (8.33333333332248946124e-3 + z * (-1.98412698298579493134e-4 + z * (2.75573137070700676789e-6 + z * (-2.50507602534068634195e-8 + z * 1.58969099521155010221e-10)))))
}

function cosK(x: number): number {
  const z = x * x
  const r = z * (4.16666666666666019037e-2 + z * (-1.38888888888741095749e-3 + z * (2.48015872894767294178e-5 + z * (-2.75573143513906633035e-7 + z * (2.08757232129817482790e-9 + z * -1.13596475577881948265e-11)))))
  const hz = 0.5 * z
  const w = 1 - hz
  return w + (1 - w - hz + z * r)
}

// for the arguments the SDK uses (|x| up to about 10^5): reduced by pi/2 in two parts
function quadrant(x: number): [number, number] {
  const n = Math.round(x * INV_PIO2)
  return [n & 3, x - n * PIO2_HI - n * PIO2_LO]
}

export function sin(x: number): number {
  const [q, r] = quadrant(x)
  return q === 0 ? sinK(r) : q === 1 ? cosK(r) : q === 2 ? -sinK(r) : -cosK(r)
}

export function cos(x: number): number {
  const [q, r] = quadrant(x)
  return q === 0 ? cosK(r) : q === 1 ? -sinK(r) : q === 2 ? -cosK(r) : sinK(r)
}
