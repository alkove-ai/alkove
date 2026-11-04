import type { AudioProcessorOptions, Room, Track, TrackProcessor } from 'livekit-client';

import Native from './NativeAlkoveLiveKit';

export interface FocusOptions {
  voiceprint?: Record<string, unknown> | null;
  strength?: number;
  normalize?: boolean;
  speech?: Partial<{ confidence: number; exitConfidence: number; startSecs: number; stopSecs: number }>;
}

// Focus as the microphone track's processor: localAudioTrack.setProcessor(AlkoveFocus())
export class AlkoveFocusProcessor implements TrackProcessor<Track.Kind.Audio, AudioProcessorOptions> {
  readonly name = 'alkove-focus';
  processedTrack?: AudioProcessorOptions['track'];
  #enabled = true;

  constructor(private readonly options: FocusOptions = {}) {}

  init = async (opts: AudioProcessorOptions) => {
    this.processedTrack = opts.track;
    await Native.attach(JSON.stringify(this.options));
    Native.setEnabled(this.#enabled);
  };

  restart = async (opts: AudioProcessorOptions) => {
    this.processedTrack = opts.track;
  };

  destroy = async () => {
    Native.detach();
  };

  async onPublish(_room: Room) {}

  // off: the raw microphone at the same delay, with a short fade
  setEnabled(on: boolean) {
    this.#enabled = on;
    Native.setEnabled(on);
  }

  isEnabled() {
    return this.#enabled;
  }

  setStrength(strength: number) {
    Native.setStrength(strength);
  }

  // applied at the user's next silence; null goes back to the closest voice
  setVoiceprint(voiceprint: Record<string, unknown> | null) {
    Native.setVoiceprint(JSON.stringify(voiceprint));
  }

  // frames processed since attach, to check Focus is in the path
  processed(): number {
    return Native.processed();
  }
}

export function AlkoveFocus(options: FocusOptions = {}): AlkoveFocusProcessor {
  return new AlkoveFocusProcessor(options);
}
