package ai.alkove

import android.content.Context
import android.media.AudioDeviceInfo
import java.util.concurrent.ConcurrentLinkedQueue
import java.util.concurrent.ExecutorService
import java.util.concurrent.Executors
import java.util.concurrent.RejectedExecutionException
import java.util.concurrent.atomic.AtomicLong
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.channels.BufferOverflow
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.onEach
import kotlinx.coroutines.flow.receiveAsFlow
import kotlinx.coroutines.launch

/** The agent's audio: float32 or int16, little-endian, at `rate`, in chunks of `chunkMs` (a multiple of 16 ms). */
data class Output(val format: Format = Format.F32, val rate: Int = Alkove.RATE, val chunkMs: Int = 32)

enum class Format { F32, S16 }

/** Speech heard, audio processed and noise removed since the input started (`stats = true`). */
data class Stats(val speechSeconds: Double, val seconds: Double, val noiseRemovedDb: Double?)

private const val HOP = 256
private const val TIMING_HOPS = 125
private const val MODELS_RETRY_MS = 5_000L
private const val MODELS_RETRY_MAX_MS = 60_000L
// raw after an overload, then the models again after 10 s, 20 s, 40 s…; back to 10 s once they held a minute
private const val OVERLOAD_MS = 250.0
private const val RETRY_MS = 10_000L
private const val RETRY_MAX_MS = 160_000L
private const val STABLE_MS = 60_000L
// ADPF's target per hop: under it the system may keep the thread slow and save power, over it it adds frequency
private const val HINT_TARGET_NS = 4_000_000L
private const val HELD_AUDIO = 16L shl 20

/**
 * A real-time input on the microphone: the SDK opens it at 16 kHz, runs the models on its own thread at audio priority, and reports
 * on [events]. Collect them once; they wait for you, up to a thousand (utterances up to 16 MB of audio). Close it when done.
 */
abstract class AlkoveInput internal constructor(
    context: Context,
    private val echoCancellation: Boolean,
    private val device: AudioDeviceInfo?,
    private val baseUrl: String?,
) : AutoCloseable {
    protected val app: Context = context.applicationContext
    private val heldAudio = AtomicLong()
    private val channel = Channel<Event>(1024, BufferOverflow.DROP_OLDEST) { if (it is Event.Utterance) heldAudio.addAndGet(-it.pcm.size.toLong()) }

    /** Speech, wakes, mode and status changes, diagnostics: the web SDK's events. */
    val events: Flow<Event> = channel.receiveAsFlow().onEach { if (it is Event.Utterance) heldAudio.addAndGet(-it.pcm.size.toLong()) }

    @Volatile
    var mode: Mode = Mode.RAW
        private set

    @Volatile
    var status: Status = Status.STARTING
        private set

    @Volatile
    internal var print: Voiceprint? = null

    /** The print in use, or the one learned so far. */
    val voiceprint: Voiceprint?
        get() = print

    @Volatile
    internal var strength = 1.0

    // the app's listen() and hold(), sent again to every new pipeline: they hold across loads and crashes
    @Volatile
    private var listening = true

    @Volatile
    private var holding = false

    // what setters changed since the pipeline's options were built: sent once it is installed
    @Volatile
    private var settings = 0
    private var built = 0

    private val mic = Mic(app, echoCancellation, device)
    private val commands = ConcurrentLinkedQueue<() -> Unit>()
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)
    private var thread: Thread? = null

    // closing, the capture thread's end and the background's publication, under one lock
    private val life = Any()
    private var finished = false
    private var bgClosed = false

    @Volatile
    private var closed = false
    private var reason: RawReason? = RawReason.LOADING

    // the capture thread's own
    private var pipe = 0L
    private var rawPipe = 0L
    private var shift = 0.0
    private var frames = 0L
    private var farQuietMs = Double.POSITIVE_INFINITY
    private var activeSince = 0L
    private var retryAt = 0L
    private var retryMs = RETRY_MS
    private var printed = false
    private val hop = FloatArray(HOP)
    private val out = FloatArray(HOP)
    private var timing = DoubleArray(4)
    private var record: CompletableDeferred<ByteArray>? = null
    private var hint: Hint? = null
    private var dropped = false

    @Volatile
    private var farActive = false

    // a playback started and stopped within one hop still counts
    @Volatile
    private var farStarted = false

    @Volatile
    private var enabled = true

    // the background thread's own: Timbre for the print or the voice of a wake
    private var bg: ExecutorService? = null
    private var coach = 0L
    private var timbre: Timbre? = null
    // the words asked for and Spot's model, applied once both and the pipeline are there
    @Volatile
    private var spotWords: Map<String, Any?>? = null

    @Volatile
    private var spotModel = 0L

    internal abstract fun models(): List<String>

    internal abstract fun pipelineOptions(): Map<String, Any?>

    internal open fun wantsBackground(): Boolean = false

    internal open fun reference(): Voiceprint? = null

    internal open fun learning(): Boolean = false

    internal open fun spots(): Boolean = false

    // the agent's audio still goes out raw while the models load or the device is too slow
    internal open fun rawOutput(): Map<String, Any?>? = null

    internal open fun audio(chunk: ByteArray) {}

    // without the native library: the microphone as it comes, chunked for the agent, at its rate
    internal open fun rawRate(): Int = Alkove.RATE

    internal open fun rawChunk(pcm: FloatArray) {}

    internal fun start() {
        if (!Alkove.supported) return startUnsupported()
        printed = voiceprint != null
        rawOutput()?.let { rawPipe = Native.pipelineNew(0, 0, 0, Values.encode(mapOf("chain" to null, "output" to it))) }
        try {
            mic.open(::silenced)
        } catch (e: Throwable) {
            if (rawPipe != 0L) Native.pipelineFree(rawPipe)
            throw e
        }
        setStatus(Status.RUNNING)
        setMode(Mode.RAW, RawReason.LOADING)
        thread = Thread(::run, "alkove-mic").apply { start() }
        scope.launch { startModels(MODELS_RETRY_MS) }
    }

    private fun startUnsupported() {
        val m = Mic(app, echoCancellation, device, rawRate())
        m.open(::silenced)
        setStatus(Status.RUNNING)
        setMode(Mode.RAW, RawReason.UNSUPPORTED)
        diag("engine.unsupported", "warn", mapOf("message" to "No native library for this device (${android.os.Build.SUPPORTED_ABIS.joinToString()}): raw microphone for this session."))
        thread = Thread({
            val chunk = FloatArray(maxOf(1, rawRate() / 50))
            try {
                Mic.urgent()
                while (!closed) {
                    if (m.read(chunk) < 0 || closed) break
                    rawChunk(chunk)
                }
            } catch (e: Throwable) {
                diag("worker.error", "warn", mapOf("message" to (e.message ?: e.toString())))
            } finally {
                m.close()
            }
        }, "alkove-mic").apply { start() }
    }

    protected fun emit(e: Event) {
        channel.trySend(e)
    }

    private fun diag(code: String, level: String, data: Map<String, Any?> = emptyMap()) = emit(Event.Diag(code, level, data))

    private fun failed(on: String, e: Throwable) = diag("worker.error", "warn", mapOf("message" to (e.message ?: e.toString()), "on" to on))

    @Synchronized
    private fun setMode(m: Mode, r: RawReason? = null) {
        if (mode == m && reason == r) return
        mode = m
        reason = r
        emit(Event.ModeChange(m, r))
    }

    @Synchronized
    private fun setStatus(s: Status) {
        if (status == s) return
        status = s
        emit(Event.StatusChange(s))
    }

    private fun activeMode(): Mode =
        when {
            !enabled -> Mode.RAW
            spots() -> if (reference() != null) Mode.SPOT_PRINT else Mode.SPOT
            printed -> Mode.FOCUS_PRINT
            else -> Mode.FOCUS
        }

    private fun showActive() = setMode(activeMode(), if (enabled) null else RawReason.OFF)

    // on the capture thread, or once it ended `orElse` where it is called
    private fun command(orElse: () -> Unit = {}, c: () -> Unit) {
        synchronized(life) {
            if (!finished) {
                commands.add(c)
                return
            }
        }
        orElse()
    }

    private suspend fun startModels(wait: Long) {
        if (closed) return
        try {
            val m = Alkove.models(app, models() + if (spotWords != null) listOf("spot") else emptyList(), baseUrl)
            if ("spot" in m) spotModel = m.ptr("spot")
            if (wantsBackground() && bg == null) {
                val t = Timbre(Alkove.models(app, listOf("timbre"), baseUrl))
                val reference = reference()?.value
                val start = voiceprint?.value
                val learn = learning()
                val ex = Executors.newSingleThreadExecutor { Thread(it, "alkove-background") }
                ex.execute {
                    timbre = t
                    coach = Native.coachNew(Values.encode(mapOf("reference" to reference)))
                    if (learn) Native.coachLearner(coach, Values.encode(mapOf("start" to start)))
                }
                synchronized(life) {
                    if (bgClosed) {
                        ex.execute(::freeBackground)
                        ex.shutdown()
                        return
                    }
                    bg = ex
                }
            }
            val version = settings
            val h = Native.pipelineNew(m.ptr("focus"), m.ptr("cue"), m.ptr("spot"), Values.encode(pipelineOptions()))
            command({ Native.pipelineFree(h) }) {
                if (closed) Native.pipelineFree(h)
                else {
                    pipe = h
                    built = version
                    applySettings()
                    if (!listening || holding) Native.pipelineSet(h, Values.encode(buildMap { if (!listening) put("listen", false); if (holding) put("hold", true) }))
                    applyWords()
                    if (rawPipe != 0L) Native.pipelineFree(rawPipe).also { rawPipe = 0 }
                    activate()
                }
            }
        } catch (e: Exception) {
            if (closed) return
            diag("models.failed", "warn", mapOf("message" to "Models not loaded, raw microphone; retry in ${wait / 1000} s.", "error" to (e.message ?: e.toString())))
            setMode(Mode.RAW, RawReason.FAILED)
            delay(wait)
            startModels(minOf(2 * wait, MODELS_RETRY_MAX_MS))
        }
    }

    // the input's clock goes on while the hops pause: events are dated on it, from the hop about to be stepped
    private fun activate() {
        shift = (frames - HOP).toDouble() / Alkove.RATE - Native.pipelineGet(pipe, 0)
        activeSince = System.currentTimeMillis()
        retryAt = 0
        showActive()
    }

    private fun run() {
        try {
            Mic.urgent()
            hint = Hint.create(app, HINT_TARGET_NS)
            while (!closed) {
                val n = mic.read(hop)
                if (closed) break
                if (n < 0) {
                    lost(n)
                    continue
                }
                frames += HOP
                runCommands()
                farQuietMs = if (farActive || farStarted) 0.0 else farQuietMs + 1000.0 * HOP / Alkove.RATE
                farStarted = false
                step()
            }
        } catch (e: Throwable) {
            failed("capture", e)
        } finally {
            synchronized(life) { finished = true }
            runCommands()
            mic.close()
            if (pipe != 0L) Native.pipelineFree(pipe)
            if (rawPipe != 0L) Native.pipelineFree(rawPipe)
            pipe = 0
            rawPipe = 0
            record?.cancel()
            hint?.close()
        }
    }

    private fun runCommands() {
        while (true) {
            val c = commands.poll() ?: break
            try {
                c()
            } catch (e: Throwable) {
                failed("command", e)
            }
        }
    }

    private fun step() {
        val now = System.currentTimeMillis()
        if (retryAt != 0L && now >= retryAt && pipe != 0L) activate()
        val active = pipe != 0L && retryAt == 0L
        val h = if (pipe != 0L) pipe else rawPipe
        try {
            var pending = 0
            if (active) {
                val t0 = System.nanoTime()
                pending = Native.pipelineStep(pipe, hop, farQuietMs, out)
                val dt = System.nanoTime() - t0
                hint?.report(dt)
                timed(dt / 1e6)
            } else if (h != 0L && rawOutput() != null) pending = Native.pipelineRaw(h, hop)
            if (pending > 0) drain(h)
        } catch (e: Throwable) {
            crashed(e)
            return
        }
        if (active && (frames / HOP) % 8 == 0L) {
            val backlog = mic.backlogMs()
            if (backlog != null && backlog > OVERLOAD_MS) overload(backlog, now)
        }
    }

    // the core failed on a hop: raw microphone, the pipeline dropped, the models again later
    private fun crashed(e: Throwable) {
        diag("engine.crash", "warn", mapOf("message" to "The engine stopped: raw microphone; retry in ${MODELS_RETRY_MS / 1000} s.", "error" to (e.message ?: e.toString())))
        setMode(Mode.RAW, RawReason.FAILED)
        if (pipe != 0L) Native.pipelineFree(pipe)
        pipe = 0
        scope.launch {
            delay(MODELS_RETRY_MS)
            startModels(MODELS_RETRY_MS)
        }
    }

    private fun overload(backlogMs: Double, now: Long) {
        if (now - activeSince > STABLE_MS) retryMs = RETRY_MS
        retryAt = now + retryMs
        retryMs = minOf(2 * retryMs, RETRY_MAX_MS)
        diag("overload", "warn", mapOf("message" to "The device is too slow for the models: raw microphone for a while.", "queueMs" to Math.round(backlogMs)))
        setMode(Mode.RAW, RawReason.OVERLOAD)
    }

    private fun timed(ms: Double) {
        val t = timing
        t[0]++
        t[1] += ms
        t[2] = maxOf(t[2], ms)
        if (ms > 1000.0 * HOP / Alkove.RATE) t[3]++
        if (t[0] < TIMING_HOPS) return
        val chained = Native.pipelineGet(pipe, 4) == 1.0
        diag(
            "timing",
            if (t[3] > t[0] / 10) "warn" else "info",
            mapOf(
                "meanMs" to Math.round(100 * t[1] / t[0]) / 100.0,
                "maxMs" to Math.round(10 * t[2]) / 10.0,
                "slow" to t[3].toInt(),
                "asleep" to (chained && Native.pipelineGet(pipe, 1) == 1.0),
                "gainDb" to if (chained) Math.round(10 * Native.pipelineGet(pipe, 2)) / 10.0 else 0.0,
                "queueMs" to (mic.backlogMs()?.let { Math.round(it) } ?: 0L),
                "adpf" to (hint != null),
            ),
        )
        timing = DoubleArray(4)
    }

    private fun shifted(v: Map<String, Any?>): Map<String, Any?> {
        if (shift == 0.0) return v
        val m = LinkedHashMap(v)
        for (k in arrayOf("start", "end", "at")) (m[k] as? Number)?.let { m[k] = it.toDouble() + shift }
        return m
    }

    private fun drain(h: Long) {
        var traces: ArrayList<Map<String, Any?>>? = null
        for (o in Values.decode(Native.pipelineOuts(h)).arr()) {
            val a = o.arr()
            when (a[0]) {
                "event" -> {
                    val e = shifted(a[1].obj())
                    if (e["type"] == "trace") (traces ?: ArrayList<Map<String, Any?>>().also { traces = it }).add(e) else onEvent(e)
                }
                "window" -> {
                    val pcm = a[1] as FloatArray
                    val q = a[2].num()
                    val t = a[3].num()
                    background("window") { coached(Native.coachWindow(coach, timbre!!.ptr, pcm, pcm.size, q, t)) }
                }
                "verify" -> {
                    val key = a[1].num().toInt()
                    val pcm = a[2] as FloatArray
                    // a voice that cannot be checked does not wake
                    background("verify", { command { if (pipe != 0L) Native.pipelineVerified(pipe, key, false, 0.0) } }) {
                        val r = Values.decode(Native.coachVerify(coach, timbre!!.ptr, pcm, pcm.size)).obj()
                        command { if (pipe != 0L) Native.pipelineVerified(pipe, key, r["same"] as Boolean, r["score"].num()) }
                    }
                }
                "audio" -> audio(a[1] as ByteArray)
                "utterance" -> utterance(a[1] as ByteArray, a[2].num() + shift, a[3].num() + shift, (a.getOrNull(4) as Map<*, *>?)?.obj()?.let(::levelOf))
                "record" -> record?.complete(a[1] as ByteArray).also { record = null }
            }
        }
        traces?.let { emit(Event.Trace(it)) }
    }

    private fun utterance(pcm: ByteArray, start: Double, end: Double, level: PhraseLevel?) {
        if (heldAudio.get() + pcm.size > HELD_AUDIO) {
            if (!dropped) diag("utterance.dropped", "warn", mapOf("message" to "Utterances are not collected: dropped past 16 MB held."))
            dropped = true
            return
        }
        dropped = false
        heldAudio.addAndGet(pcm.size.toLong())
        emit(Event.Utterance(pcm, start, end, level))
    }

    // the background's answer to a learning window: events, and a better print for Focus
    private fun coached(bytes: ByteArray) {
        for (o in Values.decode(bytes).arr()) {
            val a = o.arr()
            if (a[0] == "print") {
                val p = a[1] as FloatArray
                command { if (pipe != 0L) Native.pipelineSet(pipe, Values.encode(mapOf("print" to p))) }
            } else onEvent(a[1].obj())
        }
    }

    private fun onEvent(e: Map<String, Any?>) {
        when (e["type"]) {
            "learn" -> emit(Event.Learn(e["progress"].num()))
            "voiceprint" -> Voiceprint.read(e["json"])?.let {
                print = it
                emit(Event.VoiceprintLearned(it))
            }
            "heard" -> emit(Event.HeardScores(e["speech"].num(), e["scores"].arr().map { s -> s.obj().let { it.str("word") to it["score"].num() } }))
            "enrolled" -> emit(
                Event.Enrolled(
                    (e["word"] as Map<*, *>?)?.let { Word(it.obj()) },
                    (e["takes"] as List<*>?)?.map { Take.of(it.obj()) }.orEmpty(),
                    (e["warnings"] as List<*>?)?.map(Warning::of).orEmpty(),
                    e["audio"] as FloatArray?,
                    e["error"] as String?,
                ),
            )
            else -> {
                val ev = eventOf(e) ?: return
                if (ev is Event.Diag && ev.code == "focus.print") {
                    printed = ev.data["print"] == true
                    if (mode != Mode.RAW) showActive()
                }
                emit(ev)
            }
        }
    }

    @Volatile
    private var muted = false

    // the platform calls back on every recording change, the first one included: only a change of state is an event
    private fun silenced(on: Boolean) {
        if (on == muted) return
        muted = on
        emit(Event.Mic(if (on) MicState.MUTED else MicState.UNMUTED))
    }

    // the microphone went away (audio server restarted, device gone): reopened after 5 s, 10 s… up to a minute
    private fun lost(error: Int) {
        emit(Event.Mic(MicState.LOST))
        diag("mic.lost", "warn", mapOf("error" to error))
        mic.close()
        var wait = MODELS_RETRY_MS
        while (!closed) {
            try {
                Thread.sleep(wait)
            } catch (_: InterruptedException) {
                return
            }
            if (closed) return
            try {
                mic.open(::silenced)
                emit(Event.Mic(MicState.REOPENED))
                return
            } catch (e: AlkoveException) {
                if (e.code == "mic.denied") {
                    emit(Event.Mic(MicState.DENIED))
                    close()
                    return
                }
            } catch (e: Throwable) {
                failed("mic", e)
            }
            wait = minOf(2 * wait, MODELS_RETRY_MAX_MS)
        }
    }

    // on the capture thread: what setters changed while the pipeline was built
    private fun applySettings() {
        if (built == settings) return
        built = settings
        Native.pipelineSet(pipe, Values.encode(mapOf("strength" to strength, "enabled" to enabled, "print" to print?.embedding)))
    }

    private fun changed(change: Map<String, Any?>) {
        settings++
        command {
            if (pipe == 0L) return@command
            built = settings
            Native.pipelineSet(pipe, Values.encode(change))
        }
    }

    /** Off: no phrase until it is on again, the one being said dropped ([Event.SpeechCancel]); back on, the speech under way is no phrase. Kept before the models run, and after a crash. */
    fun listen(on: Boolean) {
        listening = on
        command { if (pipe != 0L) Native.pipelineSet(pipe, Values.encode(mapOf("listen" to on))) }
    }

    /** A phrase from now until [release], whatever Cue hears: hold to talk. */
    fun hold() {
        holding = true
        command { if (pipe != 0L) Native.pipelineSet(pipe, Values.encode(mapOf("hold" to true))) }
    }

    /** The held phrase's end and utterance come once Focus has given its last samples. */
    fun release() {
        holding = false
        command { if (pipe != 0L) Native.pipelineSet(pipe, Values.encode(mapOf("hold" to false))) }
    }

    /** Off: the raw microphone, at the same delay, with a short fade. */
    fun setEnabled(on: Boolean) {
        enabled = on
        changed(mapOf("enabled" to on))
        command {
            when {
                !on -> setMode(Mode.RAW, RawReason.OFF)
                retryAt != 0L -> setMode(Mode.RAW, RawReason.OVERLOAD)
                pipe != 0L && reason == RawReason.OFF -> showActive()
            }
        }
    }

    /** How much of Focus goes out, 0 to 1, the rest being the raw microphone. */
    fun setStrength(s: Double) {
        strength = s
        changed(mapOf("strength" to s))
    }

    /** Focus keeps this voice from the next silence on; null goes back to the closest voice. With `learn`, learning starts again from it. */
    fun setVoiceprint(p: Voiceprint?) {
        print = p
        changed(mapOf("print" to p?.embedding))
        if (learning()) background("learner") { Native.coachLearner(coach, Values.encode(mapOf("start" to p?.value))) }
    }

    /** Drops the print and what was learned of it. */
    fun forgetVoiceprint() {
        print = null
        changed(mapOf("print" to null))
        background("forget") { Native.coachForget(coach) }
    }

    /** The app's own playback starts or stops (an agent's voice): learning skips what the far end may have put in the microphone. */
    fun farEndActive(on: Boolean) {
        farActive = on
        if (on) farStarted = true
    }

    /** Speech heard and noise removed so far (`stats = true`), null before the models run or once closed. */
    suspend fun stats(): Stats? {
        val d = CompletableDeferred<Stats?>()
        command({ d.complete(null) }) {
            val v = if (pipe != 0L) Values.decode(Native.pipelineStats(pipe)) as Map<*, *>? else null
            d.complete(v?.obj()?.let { Stats(it["speechSeconds"].num(), it["seconds"].num(), (it["noiseRemovedDb"] as Number?)?.toDouble()) })
        }
        return d.await()
    }

    /** The next `seconds` (0 to 600): a stereo WAV at 16 kHz, left the microphone, right the output. */
    suspend fun record(seconds: Double): ByteArray {
        val d = CompletableDeferred<ByteArray>()
        val notRunning = { d.completeExceptionally(AlkoveException("engine.failed", "The models are not running.")) }
        command({ notRunning() }) {
            if (pipe == 0L) notRunning()
            else {
                try {
                    Native.pipelineRecord(pipe, seconds)
                } catch (e: Throwable) {
                    d.completeExceptionally(e)
                    return@command
                }
                record?.cancel()
                record = d
            }
        }
        return d.await()
    }

    /** The last `keepRecent` seconds, as [record] gives them; null without `keepRecent`. */
    suspend fun recent(): ByteArray? {
        val d = CompletableDeferred<ByteArray?>()
        command({ d.complete(null) }) { d.complete(if (pipe != 0L) Native.pipelineRecent(pipe) else null) }
        return d.await()
    }

    // on the background thread once it has its coach; a closed input's background takes nothing more
    private fun background(on: String, orElse: () -> Unit = {}, task: () -> Unit) {
        val run = Runnable {
            if (coach == 0L) return@Runnable orElse()
            try {
                task()
            } catch (e: Throwable) {
                failed(on, e)
                orElse()
            }
        }
        val queued = synchronized(life) {
            val ex = bg
            if (bgClosed || ex == null) false
            else try {
                ex.execute(run)
                true
            } catch (_: RejectedExecutionException) {
                false
            }
        }
        if (!queued) orElse()
    }

    private fun freeBackground() {
        if (coach != 0L) Native.coachFree(coach)
        coach = 0
        timbre?.close()
    }

    // on the capture thread
    private fun applyWords() {
        val words = spotWords ?: return
        if (pipe == 0L || spotModel == 0L) return
        Native.pipelineSpot(pipe, spotModel)
        Native.pipelineSet(pipe, Values.encode(words))
    }

    internal fun listenFor(words: List<Word>, context: String) {
        spotWords = mapOf("words" to words.map { it.value }, "context" to context)
        scope.launch {
            try {
                spotModel = Alkove.models(app, listOf("spot"), baseUrl).ptr("spot")
                command(c = ::applyWords)
            } catch (e: Exception) {
                diag("models.failed", "warn", mapOf("message" to "Spot not loaded: no words.", "error" to (e.message ?: e.toString())))
            }
        }
    }

    /** Stops the microphone and the models. */
    override fun close() {
        synchronized(life) {
            if (closed) return
            closed = true
        }
        setStatus(Status.CLOSED)
        scope.cancel()
        thread?.let {
            if (it !== Thread.currentThread()) {
                it.interrupt()
                it.join(500)
            }
        }
        val ex = synchronized(life) {
            bgClosed = true
            bg
        }
        ex?.let {
            it.execute(::freeBackground)
            it.shutdown()
        }
        channel.close()
    }
}

/** What an agent input does with the user's voice. `farEnd`: the app calls [AlkoveInput.farEndActive] around its own playback, which `learn` needs. */
data class AgentOptions @JvmOverloads constructor(
    val voiceprint: Voiceprint? = null,
    val learn: Boolean = false,
    val farEnd: Boolean = false,
    val strength: Double = 1.0,
    val normalize: Boolean = true,
    val speech: SpeechParams? = null,
    val output: Output = Output(),
    val keepRecent: Double = 0.0,
    val trace: Boolean = false,
    val stats: Boolean = false,
    val echoCancellation: Boolean = true,
    val device: AudioDeviceInfo? = null,
    val baseUrl: String? = null,
    /** Speech ends and utterances carry their [PhraseLevel]. */
    val levels: Boolean = false,
    /** A voice this many dB under the session's main voice ([Alkove.NEAR_GAP_DB]) opens no phrase, but a diag `speech.far`; null: off. */
    val near: Double? = null,
)

/**
 * The user's voice for a voice agent: Focus and Cue on the microphone, the audio at the rate and format the agent's API takes,
 * speech start and end, and each utterance with its pre-roll. `onAudio` runs on the audio thread: hand the chunk over, do not block.
 */
class AgentInput private constructor(context: Context, private val opts: AgentOptions, private val onAudio: ((ByteArray) -> Unit)?) :
    AlkoveInput(context, opts.echoCancellation, opts.device, opts.baseUrl) {
    init {
        print = opts.voiceprint
        strength = opts.strength
    }

    private val output = opts.output.let { it.copy(chunkMs = maxOf(16, Math.round(it.chunkMs / 16.0).toInt() * 16)) }

    private fun outputValue(utterance: Boolean) =
        mapOf("format" to if (output.format == Format.S16) "s16" else "f32", "rate" to output.rate, "chunkMs" to output.chunkMs, "audio" to true, "utterance" to utterance)

    override fun models() = listOf("focus", "cue")

    override fun wantsBackground() = opts.learn

    override fun learning() = opts.learn

    override fun rawOutput() = outputValue(false)

    override fun audio(chunk: ByteArray) {
        onAudio?.invoke(chunk)
    }

    override fun rawRate() = output.rate

    private var raw = FloatArray(0)
    private var rawFill = 0

    // the chunks the core's packer would give: chunkMs of samples, float32 or int16 as the web writes them
    override fun rawChunk(pcm: FloatArray) {
        if (raw.isEmpty()) raw = FloatArray(Math.round(output.rate * output.chunkMs / 1000.0).toInt())
        for (x in pcm) {
            raw[rawFill++] = x
            if (rawFill == raw.size) {
                rawFill = 0
                val b = java.nio.ByteBuffer.allocate(raw.size * if (output.format == Format.S16) 2 else 4).order(java.nio.ByteOrder.LITTLE_ENDIAN)
                if (output.format == Format.S16) toInt16(raw).forEach { b.putShort(it) } else raw.forEach { b.putFloat(it) }
                audio(b.array())
            }
        }
    }

    override fun pipelineOptions() =
        mapOf(
            "chain" to
                mapOf("gate" to false, "normalize" to opts.normalize, "strength" to strength, "print" to print?.embedding, "speech" to (opts.speech ?: SpeechParams()).value(), "levels" to opts.levels) +
                (if (opts.near != null) mapOf("near" to mapOf("gapDb" to opts.near)) else emptyMap()),
            "learn" to opts.learn,
            "keepRecent" to opts.keepRecent,
            "trace" to opts.trace,
            "stats" to opts.stats,
            "output" to outputValue(true),
        )

    /** Words read in Focus's output, as a wake listener would on the raw microphone; their wakes come on [events]. Close the handle to stop. */
    @JvmOverloads
    fun spot(words: List<Any>, context: String = "anywhere"): AutoCloseable {
        listenFor(words.map(Word::of), context)
        return AutoCloseable { listenFor(emptyList(), "anywhere") }
    }

    companion object {
        /** Opens the microphone (RECORD_AUDIO granted first) and starts; the models load in the background, raw audio meanwhile. */
        @JvmStatic
        @JvmOverloads
        fun start(context: Context, options: AgentOptions = AgentOptions(), onAudio: ((ByteArray) -> Unit)? = null): AgentInput {
            require(!options.learn || options.farEnd) { "learn needs farEnd: the agent's voice would teach the print (call farEndActive around your playback)" }
            return AgentInput(context, options, onAudio).also { it.start() }
        }
    }
}

/** What a wake listener listens for. `onlyMyVoice`: only this voice wakes it, checked by Timbre on the word and the 0.5 s after it. */
data class WakeOptions @JvmOverloads constructor(
    val words: List<Any>,
    val context: String = "pause",
    val onlyMyVoice: Voiceprint? = null,
    val trace: Boolean = false,
    val device: AudioDeviceInfo? = null,
    val baseUrl: String? = null,
)

/** Spot alone on the raw microphone: wakes on [events], each with the word's raw audio. */
class WakeListener private constructor(context: Context, private val opts: WakeOptions) : AlkoveInput(context, false, opts.device, opts.baseUrl) {
    private val words = if (Alkove.supported) opts.words.map(Word::of) else emptyList()

    init {
        require(opts.words.isNotEmpty()) { "WakeListener: no word" }
        for (w in words) if (w.chain == "focus") emit(Event.Diag("word.chain", "warn", mapOf("message" to "\"${w.text}\" was enrolled behind Focus: its thresholds are off on the raw microphone.")))
    }

    override fun models() = listOf("spot")

    override fun spots() = true

    override fun wantsBackground() = opts.onlyMyVoice != null

    override fun reference() = opts.onlyMyVoice

    override fun pipelineOptions() =
        mapOf("chain" to null, "words" to words.map { it.value }, "context" to opts.context, "trace" to opts.trace, "verify" to (opts.onlyMyVoice != null))

    /** Listen for other words from now on. */
    @JvmOverloads
    fun setWords(words: List<Any>, context: String = opts.context) = listenFor(words.map(Word::of), context)

    companion object {
        @JvmStatic
        fun start(context: Context, options: WakeOptions): WakeListener = WakeListener(context, options).also { it.start() }
    }
}

// Spot on the raw microphone, the takes counted in the pipeline
internal class EnrollInput(context: Context, private val word: Word, private val takes: Int, baseUrl: String?) : AlkoveInput(context, false, null, baseUrl) {
    override fun models() = listOf("spot")

    override fun spots() = true

    override fun pipelineOptions() = mapOf("chain" to null, "enroll" to mapOf("word" to word.value, "takes" to takes))
}
