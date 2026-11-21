import { TurboModuleRegistry, type CodegenTypes, type TurboModule } from 'react-native';

export type NativeEvent = { id: number; type: string; json: string };

export interface Spec extends TurboModule {
  // config: { url?, dir?, bundled? } as JSON, from Alkove.configure's modelBaseUrl
  configure(config: string): void;
  compile(text: string, langs: string[]): Promise<string>;
  // usage: 'call' | 'agent' | 'wake'; the download's progress comes as onEvent { id, type: 'progress' }
  preload(id: number, usage: string, voiceprint: boolean, typedWords: boolean): Promise<void>;
  cached(usage: string, voiceprint: boolean, typedWords: boolean): Promise<boolean>;
  // kind: 'wake' | 'agent'; options: their JSON
  start(id: number, kind: string, options: string): Promise<string>;
  call(id: number, method: string, arg: string): void;
  // the last keepRecent seconds as a WAV in base64, '' without them
  recent(id: number): Promise<string>;
  // the next seconds as a WAV in base64
  record(id: number, seconds: number): Promise<string>;
  // { speechSeconds, seconds, noiseRemovedDb } as JSON, 'null' before the models run
  stats(id: number): Promise<string>;
  close(id: number): void;
  readonly onEvent: CodegenTypes.EventEmitter<NativeEvent>;
}

export default TurboModuleRegistry.getEnforcing<Spec>('Alkove');
