# voice-commands

Voice commands for [Claude Code](https://claude.com/claude-code), on your machine. Say "OK Claude", then an optional session number and a command: "OK Claude, stop", "OK Claude 2, compact". Anything that isn't a command goes to that session as a prompt, or into its prompt box. When a turn ends, the mod can read a one-sentence summary out loud.

The wake word, voice isolation, speech-to-text and the voice that reads the summary all run in one binary, `alkove-voice`, which the mod downloads and checks by its sha256. No Python, no server, no API key; the audio never leaves your machine.

## Install

At the prompt of a terminal session:

```
/plugin install voice-commands --marketplace alkove-ai/alkove
```

From a shell: `claude plugin install voice-commands --marketplace alkove-ai/alkove`. In the desktop app, where `/plugin` does not run: + next to the prompt box, Plugins, Add marketplace `alkove-ai/alkove`, then Add plugin. A mod installed from a terminal also loads in the desktop app's sessions.

The first session opens the settings. The first start downloads the models once, about 25 MB, cached and checked by their sha256; reading answers aloud downloads its voice the first time it is turned on, about 140 MB. macOS asks once for the microphone, for the terminal or the app.

## What you say

"OK Claude" is the trigger by default, in every language. Then:

- a session number, optional: each session shows its own (`🎙 2` under the prompt). Without one, the phrase goes to the terminal tab in front when it can tell, else to the session you last used.
- a command, as short as you like (4 words at most), or anything else, which is written as a prompt.

The default commands, in English:

| You say | It does |
|---|---|
| stop, cancel | stops the turn |
| send, send it | sends what the prompt holds |
| clear, clear the prompt | clears the prompt |
| continue, go on | sends `continue` |
| compact | runs `/compact` |
| run the tests | sends "Run the tests." |
| commit | sends "Commit the changes." |

Every command can be changed, removed or added in the settings: a slash command or a ready prompt, with the phrases that run it. A sound plays when a phrase ends.

## Settings

`/voice-commands` opens them, and changes apply at once:

- the language: English, French, Spanish or Italian, for the trigger, the speech-to-text and the default commands;
- the trigger, any words you type;
- the commands;
- what a phrase that isn't a command does: written in the prompt box, or sent;
- reading answers aloud: off, or a short summary in the chosen language, with its voice and quality;
- the microphone and the speaker, the system's default if the one you picked is gone;
- how long a silence ends a phrase, 2 s by default;
- listening on or off, which closes the microphone for every session.

## How it works

One process holds the microphone for every session. It runs Alkove's models: [Spot](https://alkove.ai/docs/models/spot/) hears the trigger as you typed it, [Focus](https://alkove.ai/docs/models/focus/) keeps the voice closest to the mic and drops the others, [Cue](https://alkove.ai/docs/models/cue/) finds where the phrase ends. [Whistle](https://huggingface.co/Cactus-Compute/whistle) by Cactus writes it down, and the session number is read from its text. [Supertonic 3](https://huggingface.co/csukuangfj2/sherpa-onnx-supertonic-3-tts-int8-2026-05-11) through [sherpa-onnx](https://github.com/k2-fsa/sherpa-onnx) reads the summary, which Claude Haiku writes from the answer through your session.

## Limits

- macOS on Apple silicon, and Linux on x86-64 and arm64. I haven't tested Linux with a microphone yet. No Windows yet.
- Made for a mic near you: a laptop, a headset. On a room mic, it may keep the wrong voice.
- A command runs once the phrase has ended, after the silence you set.

## License

Apache-2.0. See [NOTICE](./NOTICE).
