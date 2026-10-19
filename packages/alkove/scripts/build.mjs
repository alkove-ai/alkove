// dist/: index.js (no side effect), a chunk with the worker and worklet sources, alkove-engine.wasm, the .d.ts
import { createHash } from 'node:crypto'
import { copyFileSync, mkdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { join } from 'node:path'
import { execFileSync } from 'node:child_process'

import { build } from 'esbuild'

import { modelsSource } from './models.mjs'

const root = join(import.meta.dirname, '..')
const dist = join(root, 'dist')
const workspace = join(root, '../..')
const sysroot = execFileSync('rustc', ['--print', 'sysroot'], { encoding: 'utf8' }).trim()
// every browser with SIMD has bulk memory (without it, the engine's copies go through a loop); paths of panic locations remapped: no home directory in the package, the same bytes on every machine
const flags = ['-C', 'target-feature=+simd128,+bulk-memory', `--remap-path-prefix=${sysroot}=/rust`, `--remap-path-prefix=${workspace}=/alkove`]
execFileSync('cargo', ['build', '--release', '-p', 'alkove-wasm', '--target', 'wasm32-unknown-unknown'], { cwd: workspace, stdio: 'inherit', env: { ...process.env, CARGO_ENCODED_RUSTFLAGS: flags.join('\x1f') } })
const wasm = join(workspace, 'target/wasm32-unknown-unknown/release/alkove_wasm.wasm')

const bytes = readFileSync(wasm)
const generated = `// written by scripts/build.mjs from the engine's wasm it packs
export const ENGINE_WASM = {
  name: 'alkove-engine',
  file: 'alkove-engine.wasm',
  bytes: ${bytes.length},
  sha256: '${createHash('sha256').update(bytes).digest('hex')}',
}
`
const genPath = join(root, 'src/engine.generated.ts')
if (readFileSync(genPath, 'utf8') !== generated) writeFileSync(genPath, generated)
const modelsPath = join(root, 'src/models.generated.ts')
if (readFileSync(modelsPath, 'utf8') !== modelsSource()) writeFileSync(modelsPath, modelsSource())

rmSync(dist, { recursive: true, force: true })
mkdirSync(dist, { recursive: true })

const common = { bundle: true, format: 'esm', minify: true, legalComments: 'none', target: 'es2022', logLevel: 'warning' }

const script = async (entry) => {
  const r = await build({ ...common, entryPoints: [join(root, entry)], write: false, platform: 'browser', external: ['module', 'worker_threads', 'node:*'] })
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
  // next to alkove-engine.wasm, which the runtime finds through its own module URL
  chunkNames: '[name]-[hash]',
  plugins: [assets],
})
await build({ ...common, entryPoints: [join(root, 'src/cli.ts')], outfile: join(dist, 'cli.js'), platform: 'node', banner: { js: '#!/usr/bin/env node' } })
copyFileSync(wasm, join(dist, 'alkove-engine.wasm'))

execFileSync('npx', ['tsc', '-p', 'tsconfig.build.json'], { cwd: root, stdio: 'inherit' })

const kb = (p) => `${(readFileSync(p).length / 1024).toFixed(0)} KB`
console.log(`index.js ${kb(join(dist, 'index.js'))}, worker ${kb(join(dist, 'alkove-worker.js'))}, worklet ${kb(join(dist, 'alkove-worklet.js'))}, wasm ${kb(join(dist, 'alkove-engine.wasm'))}`)
