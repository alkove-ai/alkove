import type { Engine } from '../core/engine'
import type { NativeLearner, Value } from '../core/native'
import type { VoiceprintJSON } from '../core/timbre'
import type { Lang } from '../manifest'
import { type ProcessOptions, processPcm, Processor } from '../process'

// what the background worker (or Node, in its own thread) answers, without state of its own beyond the loaded models
export function makeCalls(engine: () => Engine) {
  // ids come from the page: a worker started again never hands out one still held by an older object
  const learners = new Map<number, NativeLearner>()
  const streams = new Map<number, Processor>()
  const held = <T>(map: Map<number, T>, id: number, what: string): T => {
    const x = map.get(id)
    if (!x) throw new Error(`${what} ${id} is closed, or its worker was started again`)
    return x
  }
  return {
    embed: (pcm: Float32Array, source?: 'raw' | 'focus') => engine().timbre().embed(pcm, source ?? 'raw') as unknown as VoiceprintJSON,
    // a learner for a page's own screen (VoiceprintLearner): its print comes back raw, float32 as they are
    learnerOpen: (id: number, start: Value) => {
      const l = engine().learner('raw', start)
      learners.set(id, l)
      return { progress: l.progress(), ready: l.ready(), voiceprint: l.voiceprint(true) }
    },
    learnerOffer: (id: number, pcm: Float32Array) => {
      const l = held(learners, id, 'learner')
      return { ...l.offer(engine().timbre(), pcm), voiceprint: l.voiceprint(true) }
    },
    learnerForget: (id: number) => {
      learners.get(id)?.forget()
      return null
    },
    learnerClose: (id: number) => {
      learners.get(id)?.release()
      learners.delete(id)
      return null
    },
    phonemize: (texts: string[], lang: Lang) => engine().lex().phonemize(texts, lang),
    // the warnings stay in the worker, as they did
    compile: (text: string, opts: { lang?: Lang | Lang[]; chain?: string }) => engine().compile(text, opts.lang === undefined ? [] : [opts.lang].flat(), opts.chain).word,
    process: (pcm: Float32Array, opts: Omit<ProcessOptions, 'onProgress' | 'signal'>, onProgress?: ProcessOptions['onProgress']) => {
      const r = processPcm(engine(), pcm, { ...opts, onProgress })
      return { audio: r.audio, raw: r.raw, speech: r.speech, wakes: r.wakes, diag: r.diag, trace: r.trace }
    },
    streamOpen: (id: number, opts: Omit<ProcessOptions, 'onProgress' | 'signal'>) => {
      streams.set(id, new Processor(engine(), opts))
      return null
    },
    streamPush: (id: number, pcm: Float32Array) => held(streams, id, 'stream').push(pcm),
    streamEnd: (id: number) => {
      const s = held(streams, id, 'stream')
      streams.delete(id)
      return s.end()
    },
  }
}

export type Calls = ReturnType<typeof makeCalls>
export type CallName = keyof Calls
