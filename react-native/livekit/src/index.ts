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
  // the latest settings, sent again with every attach: a call before it, or a rejoin, keeps them
  #options: FocusOptions;

  constructor(options: FocusOptions = {}) {
    this.#options = { ...options };
  }

  init = async (opts: AudioProcessorOptions) => {
    this.processedTrack = opts.track;
    await Native.attach(JSON.stringify(this.#options));
    Native.setEnabled(this.#enabled);
  };

  restart = async (opts: AudioProcessorOptions) => {
    this.processedTrack = opts.track;
  };

  // also drops an attach still loading: the native side counts attaches and detaches
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
    this.#options.strength = strength;
    Native.setStrength(strength);
  }

  // applied at the user's next silence; null goes back to the closest voice
  setVoiceprint(voiceprint: Record<string, unknown> | null) {
    this.#options.voiceprint = voiceprint;
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
