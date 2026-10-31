# wyoming-alkove (preview)

A [Wyoming](https://github.com/rhasspy/wyoming) wake word server for Home Assistant with [Alkove](https://alkove.ai)'s Spot: any word you type is a wake word, in French, English, Spanish, Italian, Portuguese, Catalan or Romanian, with no training and no recording. It takes the place of the openWakeWord add-on for a satellite (a Raspberry Pi with a microphone, Wyoming Satellite), and runs on its CPU.

Maintained by Alkove.

## Run

```sh
pip install wyoming-alkove
wyoming-alkove --uri tcp://0.0.0.0:10400 --word "ok nabu" --word "fr:salut maison" --zeroconf
```

The models (Spot 2.8 MB, Lex 1.8 MB) are downloaded once from Hugging Face and checked by their sha256; `--models-dir` reads them from a folder. Then add the Wyoming integration in Home Assistant (found by `--zeroconf`, or by host and port) and pick the word in the satellite's settings, or as the wake word engine of an Assist pipeline. Tried with Home Assistant 2026.10.

- `--word "text"` (repeatable): a word in `--lang` (default `en`); `"fr:text"` for another language. Lex compiles it when the server starts. Its id for Wyoming is the text in lower case with underscores: `ok_nabu`.
- `--any-word`: a name a client asks for that no `--word` gave is compiled on demand, in `--lang`.
- `--context`: `pause` (default, the word said after a pause: far fewer false alarms), `anywhere` or `both`.
- `--threshold-pause`, `--threshold-anywhere`: every word's threshold, instead of Spot's default for its language.

## End of turn for your speech-to-text

Home Assistant ends a voice command after a fixed silence (0.7 s, or 1.25 s and 0.25 s with the satellite's "finished speaking detection"). In front of a Wyoming speech-to-text, `wyoming-alkove` ends it when Cue's end of turn head hears that the sentence is over, and waits when it is left hanging:

```sh
wyoming-alkove --uri tcp://0.0.0.0:10400 --stt-uri tcp://127.0.0.1:10300 --zeroconf
```

- `--stt-uri`: the Wyoming speech-to-text that transcribes, for example the Whisper add-on on port 10300. The audio goes on to it untouched; when the command is over, it gets the end of the audio and its transcript goes back to Home Assistant.
- In Home Assistant, the Wyoming integration then offers the speech-to-text `alkove-turn`: pick it in the Assist pipeline. It tells Home Assistant that it finds the end of speech itself, so the satellite's silence setting no longer applies to it.
- The command is over on the first read of the head over `--turn-threshold` (0.6) once the pause reaches `--min-pause` (0.3 s), or at `--max-pause` (1.5 s) anyway; with no speech for `--no-speech` seconds (5); after 15 s at most.
- Cue runs alone on the raw microphone, without Focus: a satellite's microphone is across the room, and Focus is made for a near one.
- Only a Wyoming speech-to-text can be behind, not Home Assistant Cloud's. The head was measured on calls to voice agents, not yet on a satellite across a room.

## Test

`pytest tests` (silence gives `not-detected`; the end of turn in front of a speech-to-text the test runs); with `ALKOVE_SCENE=a.wav` (16 kHz mono, saying the word) and `ALKOVE_WORD`, `ALKOVE_EXPECT`, a run on real speech.
