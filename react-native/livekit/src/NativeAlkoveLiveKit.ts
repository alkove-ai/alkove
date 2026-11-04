import { TurboModuleRegistry, type TurboModule } from 'react-native';

export interface Spec extends TurboModule {
  // options: { voiceprint?, strength?, normalize?, speech? } as JSON
  attach(options: string): Promise<void>;
  detach(): void;
  setEnabled(on: boolean): void;
  setStrength(strength: number): void;
  setVoiceprint(voiceprint: string): void;
  // frames Focus has processed since attach: the processor is in the microphone's path
  processed(): number;
}

export default TurboModuleRegistry.getEnforcing<Spec>('AlkoveLiveKit');
