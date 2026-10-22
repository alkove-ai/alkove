package ai.alkove

/** What the processing raw ([Mode.RAW]) is for: off, models loading or failed, a device too slow, no native library for this device. */
enum class RawReason { OFF, LOADING, FAILED, OVERLOAD, UNSUPPORTED }

/** What an input sends out: the raw microphone, Focus (with a print or not), Spot (verified by a print or not). */
enum class Mode { RAW, FOCUS, FOCUS_PRINT, SPOT, SPOT_PRINT }

/** An input's life: starting, running, silenced by the system for a while, closed. */
enum class Status { STARTING, RUNNING, INTERRUPTED, CLOSED }

/** The microphone's own events: taken by another app or a call for a while, lost, reopened, permission withdrawn. */
enum class MicState { MUTED, UNMUTED, LOST, REOPENED, DENIED }

/** Focus's output before the leveling over the phrase (null for a held phrase without voice), the mic's noise floor, the session's main voice (null until it has heard it twice), in dBFS. */
data class PhraseLevel(val voiceDb: Double?, val noiseDb: Double, val refDb: Double?)

/** Everything the primitives and the inputs report, with the names and fields of the web SDK's events. */
sealed interface Event {
    data class SpeechStart(val start: Double) : Event

    /** [level] with `levels` or `near`. */
    data class SpeechEnd(val start: Double, val end: Double, val level: PhraseLevel? = null) : Event

    /** The phrase sent out from [start] will have no end: listening went off. */
    data class SpeechCancel(val start: Double) : Event

    /** `idle`, `focus.print`, `focus.reset`, `timing`, `overload`… with their fields in `data`; see the errors and diagnostics guide. */
    data class Diag(val code: String, val level: String, val data: Map<String, Any?> = emptyMap()) : Event

    data class ModeChange(val mode: Mode, val reason: RawReason? = null) : Event

    data class StatusChange(val status: Status) : Event

    data class Mic(val state: MicState) : Event

    /** A print learned, or learned better, from the user's speech. */
    data class VoiceprintLearned(val voiceprint: Voiceprint) : Event

    /** 0 to 1: how far the print is from its 4 windows. */
    data class Learn(val progress: Double) : Event

    /** Spot's speech probability and each word's score, every 30 ms, with `trace`. */
    data class HeardScores(val speech: Double, val scores: List<Pair<String, Double>>) : Event

    /** Every step of every model, with `trace`. */
    data class Trace(val rows: List<Map<String, Any?>>) : Event

    /** The user's words from speech start to speech end, with their pre-roll, in the output's format and rate; [level] with `levels` or `near`. */
    class Utterance(val pcm: ByteArray, val start: Double, val end: Double, val level: PhraseLevel? = null) : Event

    /** An enrollment's end: the word set on the takes, or the error that stopped it. */
    data class Enrolled(val word: Word?, val takes: List<Take>, val warnings: List<Warning>, val audio: FloatArray?, val error: String?) : Event
}
