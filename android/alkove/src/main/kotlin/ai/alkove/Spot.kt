package ai.alkove

/** What to fix in a word's text or takes (`word.too-short`…), with the warning's own fields in `data`. */
data class Warning(val code: String, val message: String, val data: Map<String, Any?> = emptyMap()) {
    internal companion object {
        fun of(v: Any?): Warning = v.obj().let { Warning(it.str("code"), it.str("message"), it.filterKeys { k -> k != "code" && k != "message" }) }
    }
}

/** One way to say a word, in IPA, with its probability. */
data class Pronunciation(val ipa: String, val p: Double)

/** A word Spot listens for: its text, pronunciations and thresholds, bound to this version's Spot. Store [toJson], read it back with [Word.fromJson]. */
class Word internal constructor(internal val value: Map<String, Any?>, warnings: List<Warning> = emptyList()) {
    /** What to fix in the text or the takes; empty for a word read from JSON. */
    val warnings: List<Warning> = warnings

    /** The word as JSON, the text the web SDK writes for it. */
    fun toJson(): String = Json.stringify(value)

    /** The word as it was typed. */
    val text: String
        get() = value.str("text")

    /** The languages it was compiled in; empty for phonemes written between slashes. */
    val lang: List<String>
        get() = value["lang"].arr().map { it as String }

    /** Its pronunciations, each with its probability. */
    val phonemes: List<Pronunciation>
        get() = value["phonemes"].arr().map { it.obj().let { p -> Pronunciation(p.str("ipa"), (p["p"] as Number).toDouble()) } }

    /** The score that wakes in each context (`anywhere`, `pause`): log-probabilities, a word said clearly scores near 0. */
    val thresholds: Map<String, Double>
        get() = value["thresholds"].obj().mapValues { (it.value as Number).toDouble() }

    /** True once enrolled on the user's voice, false for a typed word (its language's default thresholds). */
    val calibrated: Boolean
        get() = value["calibrated"] as Boolean

    /** The audio its thresholds suit: `raw` (the microphone) or `focus` (Focus's output). */
    val chain: String
        get() = value.str("chain")

    override fun toString() = "Word(\"$text\", ${phonemes.map { it.ipa }})"

    companion object {
        /** Null for a word compiled for another Spot: its thresholds no longer hold, compile its text again. */
        @JvmStatic
        fun fromJson(json: String): Word? = Values.decode(Native.word(Values.encode(Json.parse(json))))?.let { Word(it.obj()) }

        // a Word, or its JSON; word.outdated for one compiled for another Spot
        internal fun of(w: Any): Word =
            when (w) {
                is Word -> w
                is String -> fromJson(w) ?: throw AlkoveException("word.outdated", "Compiled for another Spot: compile \"${(Json.parse(w) as? Map<*, *>)?.get("text") ?: ""}\" again.")
                else -> throw IllegalArgumentException("a Word or its JSON")
            }
    }
}

private fun checkLangs(langs: List<String>) {
    for (l in langs) if (l !in Lex.langs) throw AlkoveException("lang.unsupported", "No $l: ${Lex.langs.joinToString()} only.")
}

/** Text to phonemes in fr, en, es, pt, it, ca, ro: Lex's beam search, word by word. */
class Lex(models: Models) : AutoCloseable {
    private val handle: Handle

    init {
        models.need("lex")
        handle = models.pinned { Handle(Native.lexNew(models.ptr("lex")), Native::lexFree) }
    }

    internal val ptr: Long
        get() = handle.value

    /** The likeliest pronunciations first. */
    fun phonemize(text: String, lang: String): List<Pronunciation> = phonemize(listOf(text), lang)[0]

    /** The likeliest pronunciations of each text, first first. */
    @Synchronized
    fun phonemize(texts: List<String>, lang: String): List<List<Pronunciation>> {
        checkLangs(listOf(lang))
        val out = Values.decode(Native.lexPhonemize(handle.value, Values.encode(mapOf("texts" to texts, "lang" to lang))))
        return out.arr().map { ps -> ps.arr().map { it.obj().let { p -> Pronunciation(p.str("ipa"), (p["p"] as Number).toDouble()) } } }
    }

    /** [compile] with this Lex: a typed word in one or more of its languages. */
    @JvmOverloads
    fun compile(text: String, lang: List<String>, chain: String? = null): Word = compile(text, lang, this, chain)

    fun compile(text: String, lang: String): Word = compile(text, listOf(lang))

    @Synchronized
    override fun close() = handle.close()

    protected fun finalize() = handle.close()

    companion object {
        /** The languages Lex reads. */
        @JvmStatic
        val langs: List<String> by lazy { Alkove.info["langs"].arr().map { it as String } }

        /** Spot's 58 phonemes in IPA: the symbols a word between slashes may use (stress and length marks are ignored). */
        @JvmStatic
        val inventory: List<String> by lazy { Alkove.info["inventory"].arr().map { it as String } }

        /** 0 = the same sound; a substitution between interchangeable phonemes costs half. */
        @JvmStatic
        fun distance(a: String, b: String): Double = Alkove.native().let { Native.distance(Values.encode(mapOf("a" to a, "b" to b))) }
    }
}

/** A typed word in one or more languages (needs `lex`), or phonemes between slashes like "/kamij/" (does not). */
@JvmOverloads
fun compile(text: String, lang: List<String> = emptyList(), lex: Lex? = null, chain: String? = null): Word {
    checkLangs(lang)
    val r = if (lex == null) Native.compile(0, Values.encode(mapOf("text" to text, "lang" to lang, "chain" to chain)))
    else synchronized(lex) { Native.compile(lex.ptr, Values.encode(mapOf("text" to text, "lang" to lang, "chain" to chain))) }
    val v = Values.decode(r).obj()
    return Word(v["word"].obj(), v["warnings"].arr().map(Warning::of))
}

/** A word heard: its score against its threshold, in seconds from the stream's start; a wake listener's carries the word's raw audio (16 kHz), and with onlyMyVoice the likeness to the print. */
data class Wake(
    val word: String,
    val score: Double,
    val threshold: Double,
    val context: String,
    val start: Double,
    val end: Double,
    val audio: FloatArray? = null,
    val voice: Double? = null,
) : Event {
    internal companion object {
        fun of(v: Map<String, Any?>) = Wake(
            v.str("word"),
            (v["score"] as Number).toDouble(),
            (v["threshold"] as Number).toDouble(),
            v.str("context"),
            (v["start"] as Number).toDouble(),
            (v["end"] as Number).toDouble(),
            v["audio"] as FloatArray?,
            (v["voice"] as Number?)?.toDouble(),
        )
    }
}

/** One of Spot's frames, every 30 ms: its step, the speech probability (null for the first), each word's score, the wakes. */
data class Heard(val step: Long, val speech: Double?, val scores: List<Pair<String, Double>>, val wakes: List<Wake>)

/** Spot and the words it listens for, on 16 kHz audio of any block size: a wake word on a raw microphone. `words` are [Word]s or their JSON. */
class Listener @JvmOverloads constructor(models: Models, words: List<Any>, context: String = "pause") : AutoCloseable {
    private val handle: Handle
    private var names: List<String> = emptyList()

    init {
        models.need("spot")
        val a = args(words, context)
        handle = models.pinned { Handle(Native.listenerNew(models.ptr("spot"), a), Native::listenerFree) }
    }

    private fun args(words: List<Any>, context: String): ByteArray {
        val ws = words.map(Word::of)
        names = ws.map { it.text }
        return Values.encode(mapOf("words" to ws.map { it.value }, "context" to context))
    }

    /** Listen for other words from the next frame on, on the same clock. */
    @Synchronized
    @JvmOverloads
    fun setWords(words: List<Any>, context: String = "pause") = Native.listenerWords(handle.value, args(words, context))

    /** 16 kHz audio, any length; one [Heard] per 30 ms frame it completes. */
    @Synchronized
    @JvmOverloads
    fun push(pcm: FloatArray, n: Int = pcm.size): List<Heard> =
        Values.decode(Native.listenerPush(handle.value, pcm, n)).arr().map { f ->
            val a = f.arr()
            Heard(
                (a[0] as Number).toLong(),
                (a[1] as Number?)?.toDouble(),
                names.zip(a[2].arr().map { (it as Number).toDouble() }),
                a[3].arr().map { Wake.of(it.obj()) },
            )
        }

    fun push(pcm: ShortArray): List<Heard> = push(samples(pcm))

    @Synchronized
    override fun close() = handle.close()

    protected fun finalize() = handle.close()
}

/** A take of an enrollment: its score, whether it counts, where it was in the audio, how it was said. */
data class Take(val score: Double, val ok: Boolean, val start: Double, val end: Double, val pronunciation: String) : Event {
    internal companion object {
        fun of(v: Map<String, Any?>) = Take((v["score"] as Number).toDouble(), v["ok"] as Boolean, (v["start"] as Number).toDouble(), (v["end"] as Number).toDouble(), v.str("pronunciation"))
    }
}

/** The user says the word `takes` times: thresholds set from these takes and from what is heard between them. */
class Enroller @JvmOverloads constructor(models: Models, word: Any, takes: Int = 3, chain: String = "raw") : AutoCloseable {
    private val handle: Handle

    init {
        models.need("spot")
        val a = Values.encode(mapOf("word" to Word.of(word).value, "takes" to takes, "chain" to chain))
        handle = models.pinned { Handle(Native.enrollerNew(models.ptr("spot"), a), Native::enrollerFree) }
    }

    /** True once `takes` takes with the word were heard. */
    @Volatile
    var done = false
        private set

    /** 16 kHz audio; the takes heard in it. */
    @Synchronized
    @JvmOverloads
    fun push(pcm: FloatArray, n: Int = pcm.size): List<Take> {
        val r = Values.decode(Native.enrollerPush(handle.value, pcm, n)).obj()
        done = r["done"] as Boolean
        return r["takes"].arr().map { Take.of(it.obj()) }
    }

    /** The word set on the takes, the takes, and what to warn about; IllegalArgumentException (word.no-take) when no take was heard. */
    @Synchronized
    fun result(): EnrollResult {
        val r = Values.decode(Native.enrollerResult(handle.value)).obj()
        val warnings = r["warnings"].arr().map(Warning::of)
        return EnrollResult(Word(r["word"].obj(), warnings), r["takes"].arr().map { Take.of(it.obj()) }, warnings)
    }

    @Synchronized
    override fun close() = handle.close()

    protected fun finalize() = handle.close()
}

/** What an enrollment gives: the word with its thresholds set on the user's voice, the takes, the warnings. */
data class EnrollResult(val word: Word, val takes: List<Take>, val warnings: List<Warning>)
