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

## Test

`pytest tests` (silence gives `not-detected`); with `ALKOVE_SCENE=a.wav` (16 kHz mono, saying the word) and `ALKOVE_WORD`, `ALKOVE_EXPECT`, a run on real speech.
