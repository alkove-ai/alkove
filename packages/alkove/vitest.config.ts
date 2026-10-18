import { createRequire } from 'node:module'
import { dirname, join } from 'node:path'

import { defineConfig } from 'vitest/config'

const stock = dirname(createRequire(import.meta.url).resolve('onnxruntime-web'))
const glue = process.env.ALKOVE_ORT === 'stock' ? join(stock, 'ort-wasm-simd-threaded.mjs') : join(import.meta.dirname, 'vendor/ort/alkove-ort.mjs')

export default defineConfig({
  resolve: { alias: { 'alkove:ort-glue': glue } },
  test: { testTimeout: 60_000, exclude: ['test/browser/**', 'node_modules/**'] },
})
