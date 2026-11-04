import { TurboModuleRegistry, type CodegenTypes, type TurboModule } from 'react-native';

export type NativeEvent = { id: number; type: string; json: string };

export interface Spec extends TurboModule {
  compile(text: string, langs: string[]): Promise<string>;
  // kind: 'wake' | 'agent'; options: their JSON
  start(id: number, kind: string, options: string): Promise<string>;
  call(id: number, method: string, arg: string): void;
  close(id: number): void;
  readonly onEvent: CodegenTypes.EventEmitter<NativeEvent>;
}

export default TurboModuleRegistry.getEnforcing<Spec>('Alkove');
