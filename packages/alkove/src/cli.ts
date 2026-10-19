import { copyFile, mkdir, writeFile } from 'node:fs/promises'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'

import { configure, loadModel } from './loader'
import { ENGINE_WASM, type ModelName, MODELS } from './manifest'

const USAGE = `alkove models <dir> [focus cue spot timbre lex]

Copies what the installed version of alkove loads into <dir>, for self-hosting:
the models (downloaded from Hugging Face at the pinned revision, sha256 checked),
alkove-engine.wasm, alkove-worker.js and alkove-worklet.js.

Then: Alkove.configure({ modelBaseUrl: '/<dir>/', assetBaseUrl: '/<dir>/' })`

async function main(argv: string[]): Promise<void> {
  const [command, dir, ...names] = argv
  if (command !== 'models' || !dir) {
    console.log(USAGE)
    process.exitCode = command ? 1 : 0
    return
  }
  const wanted = (names.length ? names : Object.keys(MODELS)) as ModelName[]
  for (const n of wanted) if (!(n in MODELS)) throw new Error(`unknown model ${n}`)
  await mkdir(dir, { recursive: true })
  configure({})
  for (const n of wanted) {
    const bytes = await loadModel(n)
    await writeFile(join(dir, MODELS[n].file), bytes)
    console.log(`${MODELS[n].file}  ${(bytes.length / 1e6).toFixed(2)} MB  sha256 ok`)
  }
  const here = dirname(fileURLToPath(import.meta.url))
  for (const f of [ENGINE_WASM.file, 'alkove-worker.js', 'alkove-worklet.js']) {
    await copyFile(join(here, f), join(dir, f))
    console.log(f)
  }
}

main(process.argv.slice(2)).catch((e) => {
  console.error(String(e?.message ?? e))
  process.exitCode = 1
})
