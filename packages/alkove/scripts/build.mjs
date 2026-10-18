// dist/: index.js (no side effect), a chunk with the worker and worklet sources, alkove-ort.wasm, the .d.ts
import { createHash } from 'node:crypto'
import { copyFileSync, existsSync, mkdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { createRequire } from 'node:module'
import { dirname, join } from 'node:path'
import { execFileSync } from 'node:child_process'

import { build } from 'esbuild'

const root = join(import.meta.dirname, '..')
const dist = join(root, 'dist')
const stock = dirname(createRequire(import.meta.url).resolve('onnxruntime-web'))
// vendor/ort: onnxruntime reduced to the models' operators (a separate build); ALKOVE_ORT=stock packs onnxruntime-web's own, for comparison
const ortDir = process.env.ALKOVE_ORT === 'stock' ? null : (process.env.ALKOVE_ORT ?? join(root, 'vendor/ort'))
const glue = ortDir ? join(ortDir, 'alkove-ort.mjs') : join(stock, 'ort-wasm-simd-threaded.mjs')
const wasm = ortDir ? join(ortDir, 'alkove-ort.wasm') : join(stock, 'ort-wasm-simd-threaded.wasm')
if (!existsSync(glue) || !existsSync(wasm)) throw new Error(`onnxruntime build not found: ${glue}, ${wasm}`)

const bytes = readFileSync(wasm)
const generated = `// written by scripts/build.mjs from the onnxruntime build it packs
export const ORT_WASM = {
  name: 'alkove-ort',
  file: 'alkove-ort.wasm',
  bytes: ${bytes.length},
  sha256: '${createHash('sha256').update(bytes).digest('hex')}',
}
`
const genPath = join(root, 'src/ort.generated.ts')
if (readFileSync(genPath, 'utf8') !== generated) writeFileSync(genPath, generated)

rmSync(dist, { recursive: true, force: true })
mkdirSync(dist, { recursive: true })

const alias = {
  name: 'alias',
  setup(b) {
    b.onResolve({ filter: /^alkove:ort-glue$/ }, () => ({ path: glue }))
  },
}
const common = { bundle: true, format: 'esm', minify: true, legalComments: 'none', target: 'es2022', logLevel: 'warning' }

const script = async (entry) => {
  const r = await build({ ...common, entryPoints: [join(root, entry)], write: false, platform: 'browser', external: ['module', 'worker_threads', 'node:*'], plugins: [alias] })
  return r.outputFiles[0].text
}
const worklet = await script('src/realtime/worklet.ts')
const worker = await script('src/realtime/worker.ts')
writeFileSync(join(dist, 'alkove-worklet.js'), worklet)
writeFileSync(join(dist, 'alkove-worker.js'), worker)

const assets = {
  name: 'assets',
  setup(b) {
    b.onResolve({ filter: /^\.\/assets$/ }, (a) => (a.importer.includes('runtime') ? { path: 'assets', namespace: 'alkove-assets' } : undefined))
    b.onLoad({ filter: /.*/, namespace: 'alkove-assets' }, () => ({ contents: `export const WORKER = ${JSON.stringify(worker)}\nexport const WORKLET = ${JSON.stringify(worklet)}\n`, loader: 'js' }))
  },
}
await build({
  ...common,
  entryPoints: [join(root, 'src/index.ts')],
  outdir: dist,
  splitting: true,
  platform: 'neutral',
  external: ['node:*', 'module', 'worker_threads'],
  // next to alkove-ort.wasm, which the runtime finds through its own module URL
  chunkNames: '[name]-[hash]',
  plugins: [alias, assets],
})
await build({ ...common, entryPoints: [join(root, 'src/cli.ts')], outfile: join(dist, 'cli.js'), platform: 'node', banner: { js: '#!/usr/bin/env node' } })
copyFileSync(wasm, join(dist, 'alkove-ort.wasm'))

execFileSync('npx', ['tsc', '-p', 'tsconfig.build.json'], { cwd: root, stdio: 'inherit' })

const kb = (p) => `${(readFileSync(p).length / 1024).toFixed(0)} KB`
console.log(`index.js ${kb(join(dist, 'index.js'))}, worker ${kb(join(dist, 'alkove-worker.js'))}, worklet ${kb(join(dist, 'alkove-worklet.js'))}, wasm ${kb(join(dist, 'alkove-ort.wasm'))}`)
