// src/models.generated.ts from the manifest every SDK reads (crates/core/models.json)
import { readFileSync } from 'node:fs'
import { join } from 'node:path'

export const manifestPath = join(import.meta.dirname, '../../../crates/core/models.json')

export function modelsSource() {
  const { org, models } = JSON.parse(readFileSync(manifestPath, 'utf8'))
  const value = (v) => (typeof v === 'number' ? String(v) : `'${v}'`)
  const entries = Object.entries(models).map(([key, m]) => `  ${key}: { ${Object.entries(m).map(([k, v]) => `${k}: ${value(v)}`).join(', ')} },`)
  return `// written by scripts/build.mjs from crates/core/models.json
export const HF_ORG = '${org}'

export const MODELS_JSON = {
${entries.join('\n')}
}
`
}
