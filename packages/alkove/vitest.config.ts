import { createRequire } from 'node:module'
import { dirname, join } from 'node:path'

import { defineConfig } from 'vitest/config'

const stock = dirname(createRequire(import.meta.url).resolve('onnxruntime-web'))

export default defineConfig({
  resolve: { alias: { 'alkove:ort-glue': process.env.ALKOVE_ORT_MJS ?? join(stock, 'ort-wasm-simd-threaded.mjs') } },
  test: { testTimeout: 60_000, exclude: ['test/browser/**', 'node_modules/**'] },
})
