import { cpSync, existsSync, mkdirSync, rmSync } from 'node:fs'
import { join } from 'node:path'

import { startServer } from './server'

const pkg = join(import.meta.dirname, '../..')

// dist/ is copied first: another build running meanwhile would empty it under the tests
export default async function globalSetup(): Promise<() => Promise<void>> {
  const dist = join(pkg, 'dist')
  if (!existsSync(join(dist, 'index.js'))) throw new Error('dist/ is missing: run `pnpm build` first')
  const models = join(pkg, '.cache/models')
  if (!existsSync(join(models, 'alkove-focus-1-preview.alk'))) throw new Error(`models not found in ${models}`)
  const snap = join(pkg, '.cache/browser', String(process.pid))
  mkdirSync(snap, { recursive: true })
  cpSync(dist, join(snap, 'dist'), { recursive: true })
  const { origin, server } = await startServer({ dist: join(snap, 'dist'), models, fixtures: join(pkg, 'test/fixtures'), pages: join(import.meta.dirname, 'pages') })
  process.env.ALKOVE_ORIGIN = origin
  process.env.ALKOVE_DIST = join(snap, 'dist')
  return async () => {
    server.closeAllConnections()
    await new Promise<void>((resolve) => server.close(() => resolve()))
    rmSync(snap, { recursive: true, force: true })
  }
}
