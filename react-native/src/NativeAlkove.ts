import { TurboModuleRegistry, type CodegenTypes, type TurboModule } from 'react-native';

export type NativeEvent = { id: number; type: string; json: string };

export interface Spec extends TurboModule {
  compile(text: string, langs: string[]): Promise<string>;
  startWake(id: number, words: string[], context: string): Promise<string>;
  close(id: number): void;
  readonly onEvent: CodegenTypes.EventEmitter<NativeEvent>;
}

export default TurboModuleRegistry.getEnforcing<Spec>('Alkove');
