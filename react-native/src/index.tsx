import { useEffect, useRef, useState } from 'react';
import { PermissionsAndroid, Platform } from 'react-native';

import Native, { type NativeEvent } from './NativeAlkove';
import { same } from './same';

export type Lang = 'fr' | 'en' | 'es' | 'pt' | 'it' | 'ca' | 'ro';
export type Mode = 'raw' | 'focus' | 'focus+print' | 'spot' | 'spot+print';
export type Status = 'starting' | 'running' | 'interrupted' | 'disposed';
export type Context = 'pause' | 'anywhere';

export interface Warning {
  code: string;
  message: string;
}

export interface WakeEvent {
  word: string;
  score: number;
  threshold: number;
  context: Context;
  start: number;
  end: number;
}

export interface Diag {
  code: string;
  level: 'info' | 'warn';
  message?: string;
  [key: string]: unknown;
}

export interface PhraseLevel {
  voiceDb: number | null;
  noiseDb: number;
  refDb: number | null;
}

export type VoiceprintJSON = Record<string, unknown>;

export interface InputEvents {
  speechstart: { start: number };
  speechend: { start: number; end: number; level?: PhraseLevel };
  speechcancel: { start: number };
  turn: { pause: number; probability: number; hesitation: number; at: number };
  turnend: { start: number; end: number; probability: number; hesitation: number; at: number };
  interruptstart: { start: number };
  interrupt: { start: number; at: number };
  interruptcancel: { start: number; end: number };
  utterance: { pcm: ArrayBuffer; start: number; end: number; level?: PhraseLevel };
  audio: ArrayBuffer;
  voiceprint: VoiceprintJSON;
  learn: { progress: number };
  wake: WakeEvent;
  mode: { mode: Mode; reason?: string };
  status: Status;
  diag: Diag;
  mic: { state: 'muted' | 'unmuted' | 'lost' | 'reopened' | 'denied' };
}

export class AlkoveError extends Error {
  override readonly name = 'AlkoveError';

  constructor(
    readonly code: string,
    message: string,
  ) {
    super(message);
  }
}

// a word compiled by Spot, as the web SDK's: store toJSON(), give it back as is
export class Word {
  readonly text: string;

  constructor(
    private readonly json: Record<string, unknown>,
    readonly warnings: Warning[] = [],
  ) {
    this.text = String(json.text);
  }

  toJSON(): Record<string, unknown> {
    return this.json;
  }

  static fromJSON(json: Record<string, unknown> | string): Word {
    return new Word(typeof json === 'string' ? JSON.parse(json) : json);
  }
}

const errorOf = (e: unknown): Error => {
  const n = e as { code?: string; message?: string };
  return n && typeof n.code === 'string' && n.code.includes('.') ? new AlkoveError(n.code, n.message ?? n.code) : e instanceof Error ? e : new Error(String(e));
};

export const Spot = {
  // any typed word, no training: Lex turns it into phonemes for Spot
  async compile(text: string, opts: { lang?: Lang | Lang[] } = {}): Promise<Word> {
    const langs = opts.lang === undefined ? [] : Array.isArray(opts.lang) ? opts.lang : [opts.lang];
    try {
      const r = JSON.parse(await Native.compile(text, langs));
      return new Word(r.word, r.warnings);
    } catch (e) {
      throw errorOf(e);
    }
  },
};

export type Handlers = { [K in keyof InputEvents]?: (value: InputEvents[K]) => void };

export interface HookOptions {
  enabled?: boolean;
  on?: Handlers;
}

export interface WakeOptions {
  words: readonly (Word | Record<string, unknown> | string)[];
  context?: Context;
}

export interface AgentOptions {
  voiceprint?: VoiceprintJSON | null;
  learn?: boolean;
  farEnd?: boolean;
  strength?: number;
  normalize?: boolean;
  speech?: Partial<{ confidence: number; exitConfidence: number; startSecs: number; stopSecs: number }>;
  output?: Partial<{ format: 'f32' | 's16'; rate: number; chunkMs: number }>;
  levels?: boolean;
  near?: boolean | { gapDb?: number };
  turn?: boolean | Partial<{ threshold: number; minPause: number; maxPause: number }>;
  interrupt?: boolean | Partial<{ minDuration: number }>;
}

// what a running input takes from the app, as the web input's methods
export interface InputHandle {
  listen(on: boolean): void;
  hold(): void;
  release(): void;
  farEndActive(on: boolean): void;
}

export interface InputState {
  status: Status | 'off';
  mode: Mode | null;
  error: Error | null;
  input: InputHandle | null;
}

let ids = 0;
const listeners = new Map<number, (e: NativeEvent) => void>();
let subscribed = false;

function listen(id: number, fn: (e: NativeEvent) => void): () => void {
  if (!subscribed) {
    subscribed = true;
    Native.onEvent((e) => listeners.get(e.id)?.(e));
  }
  listeners.set(id, fn);
  return () => void listeners.delete(id);
}

// every hook's inputs open one after the other: a remount opening while the previous input still loads would hold the mic twice
let queue: Promise<unknown> = Promise.resolve();

async function micAllowed(): Promise<void> {
  if (Platform.OS !== 'android') return;
  const r = await PermissionsAndroid.request(PermissionsAndroid.PERMISSIONS.RECORD_AUDIO);
  if (r !== PermissionsAndroid.RESULTS.GRANTED) throw new AlkoveError('mic.denied', 'The microphone is not allowed.');
}

const wordJSON = (w: Word | Record<string, unknown> | string) => (typeof w === 'string' ? w : JSON.stringify(w instanceof Word ? w.toJSON() : w));

const B64 = 'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/';
const B64_INDEX = new Uint8Array(128).map((_, i) => Math.max(0, B64.indexOf(String.fromCharCode(i))));

function bytes(b64: string): ArrayBuffer {
  const n = b64.length;
  const pad = b64.endsWith('==') ? 2 : b64.endsWith('=') ? 1 : 0;
  const out = new Uint8Array((n * 3) / 4 - pad);
  let o = 0;
  for (let i = 0; i < n; i += 4) {
    const v = (B64_INDEX[b64.charCodeAt(i)]! << 18) | (B64_INDEX[b64.charCodeAt(i + 1)]! << 12) | (B64_INDEX[b64.charCodeAt(i + 2)]! << 6) | B64_INDEX[b64.charCodeAt(i + 3)]!;
    if (o < out.length) out[o++] = v >> 16;
    if (o < out.length) out[o++] = (v >> 8) & 255;
    if (o < out.length) out[o++] = v & 255;
  }
  return out.buffer;
}

function decode(type: string, json: string): unknown {
  const v = JSON.parse(json);
  if (type === 'audio') return bytes(v);
  if (type === 'utterance') return { ...v, pcm: bytes(v.pcm) };
  return v;
}

const LIVE = ['strength', 'voiceprint'] as const;
type Live = { strength?: number; voiceprint?: VoiceprintJSON | null };

function useInput(kind: 'wake' | 'agent', options: object & HookOptions, live: readonly (keyof Live)[]): InputState {
  const { enabled = true, on, ...opts } = options;
  // the continuous audio only crosses to JS for an app that listens to it
  const rest = { ...opts, audio: !!on?.audio } as Record<string, unknown> & Live;
  const handlers = useRef(on);
  const latest = useRef(rest);
  const structural = Object.fromEntries(Object.entries(rest).filter(([k]) => !(live as readonly string[]).includes(k)));
  const shape = useRef({ value: structural as object, version: 0 });
  const version = same(shape.current.value, structural) ? shape.current.version : shape.current.version + 1;

  // refs written once React commits: a render it throws away changes nothing
  useEffect(() => {
    handlers.current = on;
    latest.current = rest;
    if (version !== shape.current.version) shape.current = { value: structural, version };
  });

  const [status, setStatus] = useState<Status | 'off'>('off');
  const [mode, setMode] = useState<Mode | null>(null);
  const [error, setError] = useState<Error | null>(null);
  const [input, setInput] = useState<InputHandle | null>(null);
  const current = useRef(0);
  const applied = useRef<Live>({});

  useEffect(() => {
    if (!enabled) return;
    let cancelled = false;
    let id = 0;
    let off: (() => void) | null = null;
    setStatus('starting');
    setError(null);
    const opening = queue.then(async () => {
      if (cancelled) return;
      await micAllowed();
      if (cancelled) return;
      const o = latest.current;
      id = ++ids;
      off = listen(id, (e) => {
        const v = decode(e.type, e.json);
        if (e.type === 'status') setStatus(v as Status);
        if (e.type === 'mode') setMode((v as { mode: Mode }).mode);
        (handlers.current?.[e.type as keyof InputEvents] as ((v: unknown) => void) | undefined)?.(v);
      });
      const payload = kind === 'wake' ? { words: (o.words as WakeOptions['words']).map(wordJSON), context: o.context ?? 'pause' } : o;
      applied.current = { strength: o.strength ?? 1, voiceprint: o.voiceprint ?? null };
      const r = JSON.parse(await Native.start(id, kind, JSON.stringify(payload)));
      if (cancelled) return;
      current.current = id;
      const call = (method: string, arg: unknown = null) => Native.call(id, method, JSON.stringify(arg));
      setInput({ listen: (on) => call('listen', on), hold: () => call('hold'), release: () => call('release'), farEndActive: (on) => call('farEndActive', on) });
      setStatus(r.status);
      setMode(r.mode);
    });
    queue = opening.catch(() => {});
    opening.catch((e) => {
      if (cancelled) return;
      off?.();
      setError(errorOf(e));
      setStatus('off');
    });
    return () => {
      cancelled = true;
      current.current = 0;
      // after its opening: a close sent before the start would leave the input open
      void opening.finally(() => {
        off?.();
        if (id) Native.close(id);
      });
      setInput(null);
      setStatus('off');
      setMode(null);
      setError(null);
    };
  }, [enabled, version, kind]);

  const strength = live.includes('strength') ? rest.strength : undefined;
  const voiceprint = live.includes('voiceprint') ? rest.voiceprint : undefined;
  useEffect(() => {
    const id = current.current;
    if (!id) return;
    if (live.includes('strength') && (strength ?? 1) !== applied.current.strength) {
      applied.current.strength = strength ?? 1;
      Native.call(id, 'setStrength', JSON.stringify(applied.current.strength));
    }
    if (live.includes('voiceprint') && !same(voiceprint ?? null, applied.current.voiceprint ?? null)) {
      applied.current.voiceprint = voiceprint ?? null;
      Native.call(id, 'setVoiceprint', JSON.stringify(applied.current.voiceprint));
    }
  }, [input, strength, voiceprint, live]);

  return { status, mode, error, input };
}

export function useWakeListener(options: WakeOptions & HookOptions): InputState {
  return useInput('wake', options, []);
}

export function useAgentInput(options: AgentOptions & HookOptions = {}): InputState {
  return useInput('agent', options, LIVE);
}
