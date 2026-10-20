package ai.alkove

/** A voice, bound to this version's Timbre: give it to Focus (`voiceprint`), compare it ([Timbre.verify]), store [toJson]. */
class Voiceprint internal constructor(internal val value: Map<String, Any?>) {
    /** The print as JSON, the text the web SDK writes for it. */
    fun toJson(): String = Json.stringify(value)

    /** What Focus takes: 192 numbers. */
    val embedding: FloatArray
        get() = value["centers"].arr()[0].obj()["embedding"].let { e -> if (e is FloatArray) e else e.arr().map { (it as Number).toFloat() }.toFloatArray() }

    /** Raw audio or Focus's output. */
    val source: String
        get() = value.str("source")

    /** The cosine of the closest pair of centers. */
    fun similarity(other: Voiceprint): Double = (Values.decode(Native.voiceprint(Values.encode(mapOf("op" to "verify", "a" to value, "b" to other.value)))).obj()["score"] as Number).toDouble()

    override fun toString() = "Voiceprint($source, ${value["centers"].arr().map { it.obj()["seconds"] }} s)"

    companion object {
        /** Null for a print of another Timbre: it no longer means anything, learn it again. */
        @JvmStatic
        fun fromJson(json: String): Voiceprint? = read(Json.parse(json))

        internal fun read(v: Any?): Voiceprint? = Values.decode(Native.voiceprint(Values.encode(mapOf("op" to "read", "print" to v))))?.let { Voiceprint(it.obj()) }

        /** One print with the centers of several (near and far, phone and headset): a voice is as close as its closest center. */
        @JvmStatic
        fun merge(prints: List<Voiceprint>): Voiceprint = Voiceprint(Values.decode(Native.voiceprint(Values.encode(mapOf("op" to "merge", "prints" to prints.map { it.value })))).obj())
    }
}

/** How sure [Timbre.verify] and [Timbre.identify] must be. */
enum class Level { LENIENT, BALANCED, STRICT }

/** Who is speaking: a print from 16 kHz audio (1.5 s or more), compared with others. */
class Timbre(models: Models) : AutoCloseable {
    private val handle: Handle

    init {
        models.need("timbre")
        handle = models.pinned { Handle(Native.timbreNew(models.ptr("timbre")), Native::timbreFree) }
    }

    internal val ptr: Long
        get() = handle.value

    /** Windows of 1.5 s, half overlapping, their mean without the 10 % least alike; `source` says the audio was `raw` or Focus's output (`focus`). */
    @Synchronized
    @JvmOverloads
    fun embed(pcm: FloatArray, source: String = "raw"): Voiceprint =
        Voiceprint(Values.decode(Native.timbreEmbed(handle.value, pcm, pcm.size, Values.encode(mapOf("source" to source)))).obj())

    @Synchronized
    override fun close() = handle.close()

    protected fun finalize() = handle.close()

    companion object {
        /** `a`: the known print, `b`: the voice heard; thresholds set for how much voice each holds. Whether they match, and the score. */
        @JvmStatic
        @JvmOverloads
        fun verify(a: Voiceprint, b: Voiceprint, level: Level = Level.BALANCED): Pair<Boolean, Double> {
            val r = Values.decode(Native.voiceprint(Values.encode(mapOf("op" to "verify", "a" to a.value, "b" to b.value, "level" to level.name.lowercase())))).obj()
            return (r["same"] as Boolean) to (r["score"] as Number).toDouble()
        }

        /** The known voice that matches best, if any matches, with its score. */
        @JvmStatic
        @JvmOverloads
        fun identify(voice: Voiceprint, known: Map<String, Voiceprint>, level: Level = Level.BALANCED): Pair<String, Double>? {
            val r = Values.decode(Native.voiceprint(Values.encode(mapOf("op" to "identify", "voice" to voice.value, "known" to known.map { listOf(it.key, it.value.value) }, "level" to level.name.lowercase()))))
            return r?.obj()?.let { it.str("who") to (it["score"] as Number).toDouble() }
        }
    }
}

/** What a learner took from the speech it was offered. */
data class Offered(val accepted: Int, val rejected: Int, val restarted: Boolean, val progress: Double, val ready: Boolean)

/** A print learned from the user's speech, as the inputs of the web SDK learn it: a window joins only if it sounds like the others. */
class VoiceprintLearner @JvmOverloads constructor(models: Models, voiceprint: Voiceprint? = null, source: String = "raw") : AutoCloseable {
    private val timbre = Timbre(models)
    private val handle = Handle(Native.learnerNew(Values.encode(mapOf("source" to source, "start" to voiceprint?.value))), Native::learnerFree)

    /** Speech of any length, cut into 1.5 s windows, quiet ones skipped. */
    @Synchronized
    fun offer(pcm: FloatArray): Offered {
        val r = Values.decode(Native.learnerOffer(handle.value, timbre.ptr, pcm, pcm.size)).obj()
        return Offered((r["accepted"] as Number).toInt(), (r["rejected"] as Number).toInt(), r["restarted"] as Boolean, (r["progress"] as Number).toDouble(), r["ready"] as Boolean)
    }

    private val state: Map<String, Any?>
        get() = Values.decode(Native.learnerState(handle.value)).obj()

    /** 0 to 1: the windows accepted so far, out of the 4 (6 s of voice) a print needs. */
    val progress: Double
        @Synchronized get() = (state["progress"] as Number).toDouble()

    /** The print has its 4 windows: Focus can use it. */
    val ready: Boolean
        @Synchronized get() = state["ready"] as Boolean

    /** The print learned so far once [ready], else null. */
    val voiceprint: Voiceprint?
        @Synchronized get() = Values.decode(Native.learnerVoiceprint(handle.value, false))?.let { Voiceprint(it.obj()) }

    /** Drops every window: learning starts again from nothing. */
    @Synchronized
    fun forget() = Native.learnerForget(handle.value)

    @Synchronized
    override fun close() {
        handle.close()
        timbre.close()
    }

    protected fun finalize() = handle.close()
}
