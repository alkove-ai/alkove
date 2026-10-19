import type { Engine } from '../core/engine'
import type { NativeLearner, Value } from '../core/native'
import type { VoiceprintJSON } from '../core/timbre'
import type { Lang } from '../manifest'
import { type ProcessOptions, processPcm, Processor } from '../process'

// what the background worker (or Node, in its own thread) answers, without state of its own beyond the loaded models
export function makeCalls(engine: () => Engine) {
  const learners = new Map<number, NativeLearner>()
  const streams = new Map<number, Processor>()
  let next = 0
  return {
    embed: (pcm: Float32Array, source?: 'raw' | 'focus') => engine().timbre().embed(pcm, source ?? 'raw') as unknown as VoiceprintJSON,
    // a learner for a page's own screen (VoiceprintLearner): its print comes back raw, float32 as they are
    learnerOpen: (start: Value) => {
      const l = engine().learner('raw', start)
      learners.set(++next, l)
      return { id: next, progress: l.progress(), ready: l.ready(), voiceprint: l.voiceprint(true) }
    },
    learnerOffer: (id: number, pcm: Float32Array) => {
      const l = learners.get(id)!
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
    streamOpen: (opts: Omit<ProcessOptions, 'onProgress' | 'signal'>) => {
      streams.set(++next, new Processor(engine(), opts))
      return next
    },
    streamPush: (id: number, pcm: Float32Array) => streams.get(id)!.push(pcm),
    streamEnd: (id: number) => {
      const part = streams.get(id)!.end()
      streams.delete(id)
      return part
    },
  }
}

export type Calls = ReturnType<typeof makeCalls>
export type CallName = keyof Calls
