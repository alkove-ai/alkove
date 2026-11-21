package ai.alkove

/** Cue's state machine: speech opens on one frame over `confidence`, counts after `startSecs`, closes after `stopSecs` under `exitConfidence`. */
data class SpeechParams(
    val confidence: Double = 0.5,
    val exitConfidence: Double = 0.1,
    val startSecs: Double = 0.2,
    val stopSecs: Double = 0.6,
) {
    internal fun value(): Map<String, Any?> = mapOf("confidence" to confidence, "exitConfidence" to exitConfidence, "startSecs" to startSecs, "stopSecs" to stopSecs)
}

/** The end of the user's turn: Cue's head read every 0.1 s of each pause, a [Event.TurnEnd] on the first read over `threshold` past `minPause` seconds, or at `maxPause`. */
data class TurnParams(
    val threshold: Double = 0.6,
    val minPause: Double = 0.3,
    val maxPause: Double = 1.5,
) {
    internal fun value(): Map<String, Any?> = mapOf("threshold" to threshold, "minPause" to minPause, "maxPause" to maxPause)
}

/** A stretch of speech, in seconds from the stream's start. */
data class Speech(val start: Double, val end: Double)

/** One block of a stream: its audio at 16 kHz, and what was found while it went through, in seconds from the stream's start. */
class Part internal constructor(
    val start: Double,
    val audio: FloatArray,
    val speech: List<Speech>,
    val diag: List<Event.Diag>,
    val trace: Map<String, Any?>?,
    val wakes: List<Wake>,
)

private val WHY = listOf("silence", "forced", "wake")

internal fun samples(pcm: ShortArray): FloatArray = FloatArray(pcm.size) { (pcm[it] / 32768.0).toFloat() }

/** Float samples to int16 as the web SDK writes them: clipped, scaled by 32768 below zero and 32767 above, truncated. */
fun toInt16(pcm: FloatArray): ShortArray =
    ShortArray(pcm.size) {
        val x = pcm[it].toDouble().coerceIn(-1.0, 1.0)
        (if (x < 0) x * 32768 else x * 32767).toInt().toShort()
    }

// an event of the core as a value: { type, … }, the diag's fields under `data`
internal fun eventOf(v: Map<String, Any?>): Event? =
    when (v["type"]) {
        "speechstart" -> Event.SpeechStart(v.num("start"))
        "speechend" -> Event.SpeechEnd(v.num("start"), v.num("end"), levelOf(v))
        "speechcancel" -> Event.SpeechCancel(v.num("start"))
        "turn" -> Event.Turn(v.num("pause"), v.num("probability"), v.num("hesitation"), v.num("at"))
        "turnend" -> Event.TurnEnd(v.num("start"), v.num("end"), v.num("probability"), v.num("hesitation"), v.num("at"))
        "interruptstart" -> Event.InterruptStart(v.num("start"))
        "interrupt" -> Event.Interrupt(v.num("start"), v.num("at"))
        "interruptcancel" -> Event.InterruptCancel(v.num("start"), v.num("end"))
        "idle" -> Event.Diag("idle", "info", mapOf("asleep" to v["asleep"], "at" to v["at"]))
        "diag" -> Event.Diag(v.str("code"), v["level"] as String? ?: "info", (v["data"] as Map<*, *>?)?.obj() ?: v.filterKeys { it != "type" && it != "code" && it != "level" })
        "wake" -> Wake.of(v)
        "take" -> Take.of(v)
        else -> null
    }

private fun Map<String, Any?>.num(k: String): Double = (this[k] as Number).toDouble()

internal fun levelOf(v: Map<String, Any?>?): PhraseLevel? =
    if (v == null || "voiceDb" !in v) null else PhraseLevel((v["voiceDb"] as Number?)?.toDouble(), (v["noiseDb"] as Number).toDouble(), (v["refDb"] as Number?)?.toDouble())

private fun orNull(x: Double): Double? = if (x.isNaN()) null else x

// four numbers per event as the native chain gives them, the kind first; a level goes with the speech end or the far voice after it, as on the web
internal fun chainEvents(r: DoubleArray, count: Int): List<Event> {
    if (count == 0) return emptyList()
    val out = ArrayList<Event>(count)
    var level: PhraseLevel? = null
    for (i in 0 until count) {
        val a = r[4 * i + 1]
        val b = r[4 * i + 2]
        val c = r[4 * i + 3]
        when (r[4 * i].toInt()) {
            0 -> out.add(Event.SpeechStart(a))
            1 -> out.add(Event.SpeechEnd(a, b, level)).also { level = null }
            2 -> out.add(Event.Diag("idle", "info", mapOf("asleep" to (a == 1.0), "at" to b)))
            3 -> out.add(Event.Diag("focus.print", "info", mapOf("print" to (a == 1.0), "after" to b, "why" to WHY[c.toInt()])))
            7 -> level = PhraseLevel(orNull(a), b, orNull(c))
            8 -> {
                val l = level
                val data = mapOf("start" to a, "end" to b) + if (l == null) emptyMap() else mapOf("voiceDb" to l.voiceDb, "noiseDb" to l.noiseDb, "refDb" to l.refDb)
                out.add(Event.Diag("speech.far", "info", data))
                level = null
            }
            9 -> out.add(Event.SpeechCancel(a))
        }
    }
    return out
}

internal fun partOf(bytes: ByteArray): Part {
    val p = Values.decode(bytes).obj()
    return Part(
        start = (p["start"] as Number).toDouble(),
        audio = p["audio"] as FloatArray,
        speech = p["speech"].arr().map { s -> s.obj().let { Speech(it.num("start"), it.num("end")) } },
        diag = p["diag"].arr().mapNotNull { eventOf(it.obj()) as? Event.Diag },
        trace = p["trace"]?.obj(),
        wakes = p["wakes"].arr().map { Wake.of(it.obj()) },
    )
}

internal fun checkVoice(voiceprint: Any?) = require(voiceprint == null || voiceprint is Voiceprint || voiceprint is FloatArray) { "voiceprint: a Voiceprint or a recording of the user at 16 kHz (FloatArray)" }

// a recording of the user at 16 kHz, whatever its length: the print Timbre makes of it, and the recording as Focus's sample
internal fun voiceOf(models: Models, voiceprint: Any?): Pair<Voiceprint?, FloatArray?> {
    checkVoice(voiceprint)
    if (voiceprint !is FloatArray) return voiceprint as Voiceprint? to null
    models.need("timbre")
    return Timbre(models).use { it.embed(voiceprint) } to voiceprint
}

// a print runs Focus; `sample` apart from the print only for the tests
internal fun processOptions(focus: Boolean, print: Voiceprint?, strength: Double, cue: Boolean?, speech: SpeechParams?, normalize: Boolean, trace: Boolean, rate: Int, words: List<Any>?, context: String, sample: FloatArray? = null): Map<String, Any?> {
    val ws = words.orEmpty().map { Word.of(it).value }
    return mapOf(
        // ProcessOptions reads the print and the sample as JSON numbers, not as a FloatArray
        "focus" to if (focus || print != null) mapOf("print" to print?.embedding?.map(Float::toDouble), "strength" to strength) + (if (sample != null) mapOf("sample" to sample.map(Float::toDouble)) else emptyMap()) else null,
        "cue" to cue,
        "speech" to (speech ?: SpeechParams()).value(),
        "normalize" to normalize,
        "trace" to trace,
        "rate" to rate.toDouble(),
        "spot" to if (ws.isEmpty()) null else mapOf("words" to ws, "context" to context),
    )
}

private fun check(models: Models, opts: Map<String, Any?>) {
    if (opts["focus"] != null) models.need("focus", "cue") else if (opts["cue"] == true) models.need("cue")
    if (opts["spot"] != null) models.need("spot")
}

/** Focus, Cue and Spot over audio pushed in blocks of any size, as real time feeds them: the output aligned with the input, at 16 kHz; `voiceprint` (a [Voiceprint], or a recording of the user at 16 kHz: its print and Focus's sample) runs Focus, `words` are [Word]s or their JSON. */
class Processor @JvmOverloads constructor(
    models: Models,
    focus: Boolean = false,
    voiceprint: Any? = null,
    strength: Double = 1.0,
    cue: Boolean? = null,
    speech: SpeechParams? = null,
    normalize: Boolean = false,
    trace: Boolean = false,
    rate: Int = Alkove.RATE,
    words: List<Any>? = null,
    context: String = "anywhere",
) : AutoCloseable {
    private val handle: Handle

    init {
        val (print, sample) = voiceOf(models, voiceprint)
        val opts = processOptions(focus, print, strength, cue, speech, normalize, trace, rate, words, context, sample)
        check(models, opts)
        val o = Values.encode(opts)
        handle = models.pinned { Handle(Native.processorNew(models.ptr("focus"), models.ptr("cue"), models.ptr("spot"), models.ptr("timbre"), o), Native::processorFree) }
    }

    /** Float samples at the rate given, any length. */
    @Synchronized
    @JvmOverloads
    fun push(pcm: FloatArray, n: Int = pcm.size): Part = partOf(Native.processorPush(handle.value, pcm, n))

    /** Int16 samples at the rate given. */
    fun push(pcm: ShortArray): Part = push(samples(pcm))

    /** The samples still inside the models, and the speech still open, closed at the end. */
    @Synchronized
    fun end(): Part = partOf(Native.processorEnd(handle.value))

    @Synchronized
    override fun close() = handle.close()

    protected fun finalize() = handle.close()
}

/** A whole recording, as real time would go through it: the audio as long as the input at 16 kHz, the speech, the wakes, the diag. `voiceprint` as for [Processor]. */
@JvmOverloads
fun process(
    models: Models,
    pcm: FloatArray,
    rate: Int = Alkove.RATE,
    focus: Boolean = false,
    voiceprint: Any? = null,
    strength: Double = 1.0,
    cue: Boolean? = null,
    speech: SpeechParams? = null,
    normalize: Boolean = false,
    trace: Boolean = false,
    words: List<Any>? = null,
    context: String = "anywhere",
): Part {
    val (print, sample) = voiceOf(models, voiceprint)
    return processWith(models, pcm, rate, processOptions(focus, print, strength, cue, speech, normalize, trace, Alkove.RATE, words, context, sample))
}

// the options as they are, a sample apart from the print included
internal fun processWith(models: Models, pcm: FloatArray, rate: Int, opts: Map<String, Any?>): Part {
    check(models, opts)
    val x = if (rate != Alkove.RATE) resample(pcm, rate, Alkove.RATE) else pcm
    val o = Values.encode(opts)
    return partOf(models.pinned { Native.process(models.ptr("focus"), models.ptr("cue"), models.ptr("spot"), models.ptr("timbre"), x, x.size, o) })
}

private fun chainOptions(gate: Boolean, normalize: Boolean, strength: Double, print: Voiceprint?, sample: FloatArray?, speech: SpeechParams?, sleep: Boolean, near: Boolean) =
    Values.encode(mapOf("gate" to gate, "normalize" to normalize, "strength" to strength, "print" to print?.embedding, "sample" to sample, "speech" to (speech ?: SpeechParams()).value(), "sleep" to sleep, "levels" to true, "near" to near))

/** The real-time chain, one hop of 256 samples at 16 kHz at a time: Focus, Cue, the mix with the raw mic, the level, the gate; its output is [delay] samples late, each speech end carries its [PhraseLevel]. `voiceprint` as for [Processor]; `near`: a voice 10 dB under the session's main one is no speech, but a diag `speech.far`. */
class Chain @JvmOverloads constructor(
    private val models: Models,
    gate: Boolean = false,
    normalize: Boolean = true,
    strength: Double = 1.0,
    voiceprint: Any? = null,
    speech: SpeechParams? = null,
    sleep: Boolean = true,
    near: Boolean = false,
) : AutoCloseable {
    private val handle: Handle
    private val records = DoubleArray(4 * 16)

    init {
        models.need("focus", "cue")
        val (print, sample) = voiceOf(models, voiceprint)
        val o = chainOptions(gate, normalize, strength, print, sample, speech, sleep, near)
        handle = models.pinned { Handle(Native.chainNew(models.ptr("focus"), models.ptr("cue"), models.ptr("timbre"), o), Native::chainFree) }
    }

    /** One hop in, one hop out in `out` (256 samples), and the events it gave: speech, idle and diag. */
    @Synchronized
    fun step(hop: FloatArray, out: FloatArray): List<Event> = chainEvents(records, Native.chainStep(handle.value, hop, out, records, null))

    /** Applied at the next silence, from a fresh state; null goes back to the closest voice. A recording gives the print and Focus's sample. */
    fun setVoiceprint(voiceprint: Any?) {
        // Timbre runs outside the lock: the audio thread's steps go on meanwhile
        val (print, sample) = voiceOf(models, voiceprint)
        synchronized(this) {
            Native.chainSetPrint(handle.value, print?.embedding)
            if (sample != null) Native.chainSetSample(handle.value, sample)
        }
    }

    /** How much of Focus goes out, 0 to 1, the rest being the raw mic: 0.8 is 80 % Focus. */
    @Synchronized
    fun setStrength(s: Double) = Native.chainSetStrength(handle.value, s)

    /** Off: the raw mic, at the same delay, with a short fade. */
    @Synchronized
    fun setEnabled(on: Boolean) = Native.chainSetEnabled(handle.value, on)

    /** Off: no speech until it is on again, the phrase under way dropped ([Event.SpeechCancel]); back on, the speech under way is no phrase. */
    @Synchronized
    fun setListen(on: Boolean) = Native.chainListen(handle.value, on)

    /** A phrase from now until [release], whatever Cue hears (hold to talk); one Cue had opened goes on as the held one. */
    @Synchronized
    fun hold() = Native.chainHold(handle.value, true)

    /** Its speech end comes once the output holds the whole phrase, [delay] samples later. */
    @Synchronized
    fun release() = Native.chainHold(handle.value, false)

    /** The gain a quiet mic gets before Focus, which takes a voice under -45 dBFS for noise: 0 dB at a normal level, up to 24. */
    val inputGainDb: Double
        @Synchronized get() = Native.chainGet(handle.value, 5)

    /** Cue hears speech now. */
    val speaking: Boolean
        @Synchronized get() = Native.chainGet(handle.value, 0) == 1.0

    /** Focus and Cue paused in a silence (`sleep`): after 2 s of quiet, until the mic rises 9 dB over its noise floor. */
    val sleeping: Boolean
        @Synchronized get() = Native.chainGet(handle.value, 1) == 1.0

    /** The gain `normalize` applies now, in dB. */
    val gainDb: Double
        @Synchronized get() = Native.chainGet(handle.value, 3)

    /** Samples between a sample going in and coming out: Focus's lag, plus the gate's look-ahead with `gate`. */
    val delay: Int
        @Synchronized get() = Native.chainGet(handle.value, 4).toInt()

    @Synchronized
    override fun close() = handle.close()

    protected fun finalize() = handle.close()

    companion object {
        /** The samples of one [step], at 16 kHz: 16 ms. */
        const val HOP = 256
    }
}

/** The chain on a live stream at any rate, as the web SDK runs it in real time: frames in, the processed samples that are ready out, at the same rate. `voiceprint` and `near` as for [Chain], a recording at 16 kHz whatever `rate`. */
class Live @JvmOverloads constructor(
    private val models: Models,
    /** The stream's rate, in and out, in Hz. */
    val rate: Int,
    gate: Boolean = false,
    normalize: Boolean = false,
    strength: Double = 1.0,
    voiceprint: Any? = null,
    speech: SpeechParams? = null,
    sleep: Boolean = true,
    near: Boolean = false,
) : AutoCloseable {
    private val handle: Handle
    private var out = FloatArray(0)

    init {
        models.need("focus", "cue")
        val (print, sample) = voiceOf(models, voiceprint)
        val o = chainOptions(gate, normalize, strength, print, sample, speech, sleep, near)
        handle = models.pinned { Handle(Native.liveNew(models.ptr("focus"), models.ptr("cue"), models.ptr("timbre"), rate.toDouble(), o), Native::liveFree) }
    }

    // the events of the last pushInto
    internal var events: List<Event> = emptyList()

    // the samples ready land in `out` (grown as needed); returns their count
    internal fun pushInto(pcm: FloatArray, n: Int): Int {
        val room = n + 4 * (rate / 1000 + 1) * 32
        if (out.size < room) out = FloatArray(room)
        val r = Native.livePush(handle.value, pcm, n, out)
        val count = (r and 0xFFFFFFFFL).toInt()
        events = if (count == 0) emptyList() else chainEvents(Native.liveEvents(handle.value), count)
        return (r ushr 32).toInt()
    }

    /** Samples in, the ones ready out, and the events (speech, idle, diag), in seconds from the stream's start. */
    @Synchronized
    @JvmOverloads
    fun push(pcm: FloatArray, n: Int = pcm.size): Pair<FloatArray, List<Event>> {
        val count = pushInto(pcm, n)
        return out.copyOf(count) to events
    }

    fun push(pcm: ShortArray): Pair<FloatArray, List<Event>> = push(samples(pcm))

    // the buffer pushInto wrote into
    internal val output: FloatArray
        get() = out

    /** As [Chain.setVoiceprint]. */
    fun setVoiceprint(voiceprint: Any?) {
        // Timbre runs outside the lock: the audio thread's steps go on meanwhile
        val (print, sample) = voiceOf(models, voiceprint)
        synchronized(this) {
            Native.liveSetPrint(handle.value, print?.embedding)
            if (sample != null) Native.liveSetSample(handle.value, sample)
        }
    }

    @Synchronized
    fun setStrength(s: Double) = Native.liveSetStrength(handle.value, s)

    @Synchronized
    fun setEnabled(on: Boolean) = Native.liveSetEnabled(handle.value, on)

    /** As [Chain.setListen]. */
    @Synchronized
    fun setListen(on: Boolean) = Native.liveListen(handle.value, on)

    /** As [Chain.hold]. */
    @Synchronized
    fun hold() = Native.liveHold(handle.value, true)

    /** As [Chain.release]. */
    @Synchronized
    fun release() = Native.liveHold(handle.value, false)

    /** As [Chain.inputGainDb]. */
    val inputGainDb: Double
        @Synchronized get() = Native.liveGet(handle.value, 5)

    val speaking: Boolean
        @Synchronized get() = Native.liveGet(handle.value, 0) == 1.0

    val sleeping: Boolean
        @Synchronized get() = Native.liveGet(handle.value, 1) == 1.0

    @Synchronized
    override fun close() = handle.close()

    protected fun finalize() = handle.close()
}

/** Cue's state machine on its probabilities, frame by frame: what [Processor] and the web SDK decide, for frameworks that want each step. */
class SpeechTracker @JvmOverloads constructor(speech: SpeechParams? = null, frameSecs: Double = 0.03) : AutoCloseable {
    private val p = speech ?: SpeechParams()
    private val handle = Alkove.native().let { Handle(Native.trackerNew(p.confidence, p.exitConfidence, p.startSecs, p.stopSecs, frameSecs), Native::trackerFree) }
    private val at = DoubleArray(2)

    /** Frame k's probability; a speech start or end when speech starts or ends. */
    @Synchronized
    fun push(p: Double, k: Long): Event? =
        when (Native.trackerPush(handle.value, p, k, at)) {
            0 -> Event.SpeechStart(at[0])
            1 -> Event.SpeechEnd(at[0], at[1])
            else -> null
        }

    /** Back to silence, with no speech end for the speech it was in. */
    @Synchronized
    fun reset() = Native.trackerReset(handle.value)

    /** Speech is open: a frame went over `confidence`, and `stopSecs` under `exitConfidence` have not passed yet. */
    val active: Boolean
        @Synchronized get() = Native.trackerGet(handle.value, 0)

    /** The open speech lasted `startSecs`: its start was given. */
    val confirmed: Boolean
        @Synchronized get() = Native.trackerGet(handle.value, 1)

    @Synchronized
    override fun close() = handle.close()

    protected fun finalize() = handle.close()
}

/** Windowed sinc between any two rates, across chunks; `aligned` puts output k exactly on input k × ratio (files), else half a window late (real time). */
class Resampler @JvmOverloads constructor(inRate: Int, outRate: Int, aligned: Boolean = false) : AutoCloseable {
    private val handle = Alkove.native().let { Handle(Native.resamplerNew(inRate.toDouble(), outRate.toDouble(), aligned), Native::resamplerFree) }

    /** Samples at `inRate`, any length; the ones ready at `outRate`. */
    @Synchronized
    @JvmOverloads
    fun push(pcm: FloatArray, n: Int = pcm.size): FloatArray = Native.resamplerPush(handle.value, pcm, n)

    @Synchronized
    override fun close() = handle.close()

    protected fun finalize() = handle.close()
}

/** A whole signal from one rate to another, as long as the input at the new rate. */
fun resample(pcm: FloatArray, inRate: Int, outRate: Int): FloatArray = Alkove.native().let { Native.resample(pcm, pcm.size, inRate.toDouble(), outRate.toDouble()) }
