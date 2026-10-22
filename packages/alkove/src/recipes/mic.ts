import { AlkoveError } from '../errors'
import { report } from '../loader'

const WAITING_MS = 5000

export interface Support {
  worklet: boolean
  simd: boolean
  secure: boolean
}

// the smallest module using a SIMD instruction
const SIMD_PROBE = new Uint8Array([0, 97, 115, 109, 1, 0, 0, 0, 1, 5, 1, 96, 0, 1, 123, 3, 2, 1, 0, 10, 10, 1, 8, 0, 65, 0, 253, 15, 253, 98, 11])

export function supported(): Support {
  const g = globalThis as { AudioWorkletNode?: unknown; isSecureContext?: boolean }
  let simd = false
  try {
    simd = typeof WebAssembly !== 'undefined' && WebAssembly.validate(SIMD_PROBE)
  } catch {}
  return { worklet: typeof g.AudioWorkletNode === 'function', simd, secure: g.isSecureContext !== false }
}

export async function micPermission(): Promise<'granted' | 'denied' | 'prompt'> {
  try {
    const s = await navigator.permissions.query({ name: 'microphone' as PermissionName })
    return s.state === 'granted' || s.state === 'denied' ? s.state : 'prompt'
  } catch {
    return 'prompt'
  }
}

// echo cancellation yes; noise suppression, gain control and voice isolation no: nothing of the browser's in front of the models
export const CONSTRAINTS: MediaTrackConstraints & Record<string, unknown> = {
  echoCancellation: true,
  noiseSuppression: false,
  autoGainControl: false,
  voiceIsolation: false,
  channelCount: 1,
}

// `browser`: the browser's noise suppression and gain on, for the raw mic while the models do not run
export async function openMic(deviceId?: string, signal?: AbortSignal, browser = false): Promise<MediaStream> {
  if (!supported().secure) throw new AlkoveError('page.insecure', 'The microphone needs HTTPS (or localhost).')
  if (!navigator.mediaDevices?.getUserMedia) throw new AlkoveError('browser.unsupported', 'No getUserMedia in this browser.')
  const waiting = setTimeout(() => report({ code: 'mic.waiting', level: 'info', message: 'The permission prompt has had no answer for 5 s.' }), WAITING_MS)
  try {
    const processing = browser ? { noiseSuppression: true, autoGainControl: true } : {}
    const ask = navigator.mediaDevices.getUserMedia({ audio: { ...CONSTRAINTS, ...processing, ...(deviceId ? { deviceId: { exact: deviceId } } : {}) } })
    const stream = await (signal
      ? Promise.race([
          ask,
          new Promise<never>((_, reject) => {
            if (signal.aborted) reject(signal.reason)
            signal.addEventListener('abort', () => {
              void ask.then((s) => s.getTracks().forEach((t) => t.stop()), () => {})
              reject(signal.reason)
            })
          }),
        ])
      : ask)
    return stream
  } catch (e) {
    if (signal?.aborted) throw signal.reason
    const name = (e as Error)?.name
    if (name === 'NotAllowedError' || name === 'SecurityError') throw new AlkoveError('mic.denied', 'Microphone permission denied.', e)
    if (name === 'NotFoundError' || name === 'OverconstrainedError') throw new AlkoveError('mic.not-found', 'No microphone.', e)
    if (name === 'NotReadableError' || name === 'AbortError') throw new AlkoveError('mic.busy', 'The microphone is used by another app.', e)
    throw e
  } finally {
    clearTimeout(waiting)
  }
}

// browser processing left on in a stream the app gave: something of ours (theirs) runs in front of the models
export function processed(track: MediaStreamTrack): string[] {
  const s = track.getSettings() as MediaTrackSettings & Record<string, unknown>
  return ['noiseSuppression', 'autoGainControl', 'voiceIsolation'].filter((k) => s[k] === true)
}
