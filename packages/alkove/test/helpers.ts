import { readFileSync } from 'node:fs'
import { createRequire } from 'node:module'
import { dirname, join } from 'node:path'
import { pathToFileURL } from 'node:url'

import { Ort, type OrtFactory } from '../src/engine/ort'
import { MODELS, type ModelName } from '../src/manifest'

const root = join(import.meta.dirname, '..')
const stock = dirname(createRequire(import.meta.url).resolve('onnxruntime-web'))

export const ortFiles = () => ({
  mjs: process.env.ALKOVE_ORT_MJS ?? join(stock, 'ort-wasm-simd-threaded.mjs'),
  wasm: process.env.ALKOVE_ORT_WASM ?? join(stock, 'ort-wasm-simd-threaded.wasm'),
})

export async function loadOrt(): Promise<Ort> {
  const { mjs, wasm } = ortFiles()
  const factory = (await import(pathToFileURL(mjs).href)).default as OrtFactory
  return Ort.create(factory, await WebAssembly.compile(readFileSync(wasm)))
}

export const modelBytes = (name: ModelName) => new Uint8Array(readFileSync(join(root, '.cache/models', MODELS[name].file)))

export function noise(n: number, seed = 1, scale = 0.1): Float32Array {
  let s = seed
  const out = new Float32Array(n)
  for (let i = 0; i < n; i++) {
    s = (s * 1664525 + 1013904223) >>> 0
    out[i] = ((s / 2 ** 32) * 2 - 1) * scale
  }
  return out
}

export const maxDiff = (a: ArrayLike<number>, b: ArrayLike<number>) => {
  let d = 0
  for (let i = 0; i < a.length; i++) d = Math.max(d, Math.abs(a[i]! - b[i]!))
  return a.length === b.length ? d : Infinity
}

import { decodeWav } from '../src/dsp/wav'

export const fixture = (name: string) => readFileSync(join(root, 'test/fixtures', name))
export const wav = (name: string) => decodeWav(new Uint8Array(fixture(name))).pcm
export const f32 = (name: string) => {
  const b = fixture(name)
  return new Float32Array(b.buffer.slice(b.byteOffset, b.byteOffset + b.byteLength))
}
