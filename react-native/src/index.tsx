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

export interface InputEvents {
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

export interface InputState {
  status: Status | 'off';
  mode: Mode | null;
  error: Error | null;
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

export function useWakeListener(options: WakeOptions & HookOptions): InputState {
  const { enabled = true, on, ...rest } = options;
  const handlers = useRef(on);
  const latest = useRef(rest);
  const shape = useRef({ value: rest as object, version: 0 });
  const version = same(shape.current.value, rest) ? shape.current.version : shape.current.version + 1;

  // refs written once React commits: a render it throws away changes nothing
  useEffect(() => {
    handlers.current = on;
    latest.current = rest;
    if (version !== shape.current.version) shape.current = { value: rest, version };
  });

  const [status, setStatus] = useState<Status | 'off'>('off');
  const [mode, setMode] = useState<Mode | null>(null);
  const [error, setError] = useState<Error | null>(null);

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
        const v = JSON.parse(e.json);
        if (e.type === 'status') setStatus(v);
        if (e.type === 'mode') setMode(v.mode);
        (handlers.current?.[e.type as keyof InputEvents] as ((v: unknown) => void) | undefined)?.(v);
      });
      const r = JSON.parse(await Native.startWake(id, o.words.map(wordJSON), o.context ?? 'pause'));
      if (cancelled) return;
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
      // after its opening: a close sent before the start would leave the input open
      void opening.finally(() => {
        off?.();
        if (id) Native.close(id);
      });
      setStatus('off');
      setMode(null);
      setError(null);
    };
  }, [enabled, version]);

  return { status, mode, error };
}

