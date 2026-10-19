import { MODELS_JSON } from './models.generated'

export { ENGINE_WASM } from './engine.generated'
export { HF_ORG } from './models.generated'

export type ModelName = 'focus' | 'cue' | 'spot' | 'timbre' | 'lex'

export interface ModelFile {
  name: string
  file: string
  bytes: number
  sha256: string
  // the Hugging Face commit a model is read at (sha256 checked either way); the wasm has none, it comes with the package
  revision?: string
}

export const RATE = 16000

// from crates/core/models.json, the manifest every SDK reads
export const MODELS: Record<ModelName, ModelFile> = MODELS_JSON

export const FOCUS = {
  chunk: 256,
  print: 192,
  lag: 256,
}

export const CUE = {
  chunk: 480,
  lagSteps: 1,
}

export const SPOT = {
  chunk: 480,
  classes: 59,
  lagSteps: 3,
  speechLagSteps: 1,
  frame: 0.03,
}

// what a print or a word is bound to: a model republished under the same name with other weights does not match
export const modelId = (m: ModelName) => `${MODELS[m].name}@${MODELS[m].sha256.slice(0, 12)}`

export const TIMBRE = {
  model: modelId('timbre'),
  window: 24000,
  min: 280,
  dim: 192,
}

export const LEX = {
  langs: ['fr', 'en', 'es', 'pt', 'it', 'ca', 'ro'] as const,
  beams: 3,
  bos: 59,
  eos: 60,
  vocab: 61,
  word: 33,
  steps: 39,
}

export type Lang = (typeof LEX.langs)[number]
