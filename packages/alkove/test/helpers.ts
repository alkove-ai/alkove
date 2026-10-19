import { readFileSync } from 'node:fs'
import { join } from 'node:path'

import { Alk } from '../src/engine/alk'
import { MODELS, type ModelName } from '../src/manifest'

const root = join(import.meta.dirname, '..')

// the wasm `pnpm build` makes (crates/wasm)
export const engineWasm = () => readFileSync(join(root, '../../target/wasm32-unknown-unknown/release/alkove_wasm.wasm'))

export const loadEngine = async (): Promise<Alk> => Alk.create(await WebAssembly.compile(engineWasm()))

export const workspace = join(root, '../..')

// the core with its test exports, apart from the SDK's wasm (built by test/harness.ts)
export const harnessWasm = (): Uint8Array<ArrayBuffer> => Uint8Array.from(readFileSync(join(workspace, 'target/harness/wasm32-unknown-unknown/release/alkove_wasm.wasm')))

export const loadHarness = async (): Promise<Alk> => Alk.create(await WebAssembly.compile(harnessWasm()))

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
