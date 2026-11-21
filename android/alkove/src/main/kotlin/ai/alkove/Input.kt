package ai.alkove

import android.content.Context
import android.media.AudioDeviceInfo
import java.util.concurrent.ConcurrentLinkedQueue
import java.util.concurrent.ExecutorService
import java.util.concurrent.Executors
import java.util.concurrent.RejectedExecutionException
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicLong
import kotlinx.coroutines.CancellationException
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
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock

/** The agent's audio: float32 or int16, little-endian, at `rate`, in chunks of `chunkMs` (a multiple of 16 ms). */
data class Output(val format: Format = Format.F32, val rate: Int = Alkove.RATE, val chunkMs: Int = 32)

enum class Format { F32, S16 }

/** Speech heard, audio processed and noise removed since the input started. */
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
abstract class AlkoveInput internal constructor(context: Context, private val echoCancellation: Boolean, private val device: AudioDeviceInfo?) : AutoCloseable {
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

    // the recording given as the voiceprint, Focus's sample
    @Volatile
    internal var sample: FloatArray? = null

    /** The print in use, or the one learned so far. */
    val voiceprint: Voiceprint?
        get() = print

    // the print learns while the models run: an agent's from its first farEndActive() on
    @Volatile
    internal var learning = false
    private val learnStarted = AtomicBoolean(false)

    @Volatile
    internal var farCalled = false

    @Volatile
    private var modelsRan = false

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
    private var rawRetryAt = 0L
    private var rawFailing = false
    private var failHops = 0

    internal fun failNextHops(n: Int) = command { failHops = n }

    @Volatile
    private var farActive = false

    // a playback started and stopped within one hop still counts
    @Volatile
    private var farStarted = false

    @Volatile
    private var enabled = true

    // the background thread's own: Timbre for the print or the voice of a wake
    private var bg: ExecutorService? = null
    private val bgMade = Mutex()
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

    internal open fun learns(): Boolean = false

    internal open fun spots(): Boolean = false

    // the words it listens for from its start, compiled off the capture thread before the models load
    internal open suspend fun startWords(): Map<String, Any?>? = null

    // a recording given as a print, made into one before the models load
    internal open suspend fun resolveVoice() {}

    internal suspend fun embedRecording(recording: FloatArray): Voiceprint = Timbre(Alkove.models(app, listOf("timbre"))).use { it.embed(recording) }

    // the agent's audio still goes out raw while the models load or the device is too slow
    internal open fun rawOutput(): Map<String, Any?>? = null

    internal open fun audio(chunk: ByteArray) {}

    // without the native library: the microphone as it comes, chunked for the agent, at its rate
    internal open fun rawRate(): Int = Alkove.RATE

    internal open fun rawChunk(pcm: FloatArray) {}

    internal fun start() {
        if (!Alkove.supported) return startUnsupported()
        printed = voiceprint != null
        makeRawPipe()
        try {
            mic.open(::silenced)
        } catch (e: Throwable) {
            if (rawPipe != 0L) Native.pipelineFree(rawPipe)
            throw e
        }
        setStatus(Status.RUNNING)
        setMode(Mode.RAW, RawReason.LOADING)
        scope.launch { startModels(MODELS_RETRY_MS) }
        thread = Thread(::run, "alkove-mic").apply { start() }
    }

    // the agent's audio while no model runs: loading, failed, crashed
    private fun makeRawPipe() {
        if (rawPipe == 0L) rawOutput()?.let { rawPipe = Native.pipelineNew(0, 0, 0, 0, Values.encode(mapOf("chain" to null, "output" to it))) }
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
            resolveVoice()
            if (spotWords == null) spotWords = startWords()
            val m = Alkove.models(app, models() + if (spotWords != null) listOf("spot") else emptyList())
            if ("spot" in m) spotModel = m.ptr("spot")
            startLearning()
            if (wantsBackground()) makeBackground()
            val version = settings
            val withPrint = print != null
            val h = Native.pipelineNew(m.ptr("focus"), m.ptr("cue"), m.ptr("spot"), m.ptr("timbre"), Values.encode(pipelineOptions()))
            command({ Native.pipelineFree(h) }) {
                if (closed) Native.pipelineFree(h)
                else {
                    pipe = h
                    built = version
                    printed = withPrint
                    applySettings()
                    if (!listening || holding) Native.pipelineSet(h, Values.encode(buildMap { if (!listening) put("listen", false); if (holding) put("hold", true) }))
                    applyWords()
                    if (rawPipe != 0L) Native.pipelineFree(rawPipe).also { rawPipe = 0 }
                    activate()
                    modelsRan = true
                    if (learns() && !learnStarted.get()) scope.launch { startLearning() }
                }
            }
        } catch (e: Exception) {
            if (e is CancellationException) throw e
            if (closed) return
            diag("models.failed", "warn", mapOf("message" to "Models not loaded, raw microphone; retry in ${wait / 1000} s.", "error" to (e.message ?: e.toString())))
            setMode(Mode.RAW, RawReason.FAILED)
            delay(wait)
            startModels(minOf(2 * wait, MODELS_RETRY_MAX_MS))
        }
    }

    // Timbre on its own thread, once per input: the learning windows and the voice checks
    private suspend fun makeBackground() {
        bgMade.withLock {
            if (synchronized(life) { bg != null }) return
            val t = Timbre(Alkove.models(app, listOf("timbre")))
            val reference = reference()?.value
            val ex = Executors.newSingleThreadExecutor { Thread(it, "alkove-background") }
            ex.execute {
                timbre = t
                coach = Native.coachNew(Values.encode(mapOf("reference" to reference)))
            }
            synchronized(life) {
                if (bgClosed) {
                    ex.execute(::freeBackground)
                    ex.shutdown()
                } else {
                    bg = ex
                }
            }
        }
    }

    // once the SDK knows when the agent speaks, for the input's life: the learner from the print in use, then the pipeline's windows
    private suspend fun startLearning() {
        if (!learns() || !learnStarted.compareAndSet(false, true)) return
        learning = true
        try {
            makeBackground()
        } catch (e: Exception) {
            if (e is CancellationException) throw e
            learning = false
            learnStarted.set(false)
            diag("models.failed", "warn", mapOf("message" to "Timbre not loaded: the print does not learn.", "error" to (e.message ?: e.toString())))
            return
        }
        val start = print?.value
        background("learner") { Native.coachLearner(coach, Values.encode(mapOf("start" to start))) }
        changed(mapOf("learn" to true))
    }

    // the input's clock goes on while the hops pause: events are dated on it, from the hop about to be stepped
    private fun activate() {
        shift = (frames - HOP).toDouble() / Alkove.RATE - Native.pipelineGet(pipe, 0)
        activeSince = System.currentTimeMillis()
        retryAt = 0
        rawRetryAt = 0
        rawFailing = false
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
        if (pipe == 0L && rawPipe == 0L && rawRetryAt != 0L && now >= rawRetryAt) {
            rawRetryAt = 0
            try {
                makeRawPipe()
            } catch (_: Throwable) {
                rawRetryAt = now + 1000
            }
        }
        val active = pipe != 0L && retryAt == 0L
        val h = if (pipe != 0L) pipe else rawPipe
        try {
            var pending = 0
            if (failHops > 0 && h != 0L) {
                failHops--
                throw IllegalStateException("a hop failed (test)")
            }
            if (active) {
                val t0 = System.nanoTime()
                pending = Native.pipelineStep(pipe, hop, farQuietMs, out)
                val dt = System.nanoTime() - t0
                hint?.report(dt)
                timed(dt / 1e6)
            } else if (h != 0L && rawOutput() != null) pending = Native.pipelineRaw(h, hop)
            if (pending > 0) drain(h)
        } catch (e: Throwable) {
            if (pipe != 0L) crashed(e) else rawFailed(e, now)
            return
        }
        if (active && (frames / HOP) % 8 == 0L) {
            val backlog = mic.backlogMs()
            if (backlog != null && backlog > OVERLOAD_MS) overload(backlog, now)
        }
    }

    private fun rawFailed(e: Throwable, now: Long) {
        if (!rawFailing) failed("raw", e)
        rawFailing = true
        if (rawPipe != 0L) Native.pipelineFree(rawPipe)
        rawPipe = 0
        rawRetryAt = now + 1000
    }

    // the core failed on a hop: raw microphone, the pipeline dropped, the models again later
    private fun crashed(e: Throwable) {
        diag("engine.crash", "warn", mapOf("message" to "The engine stopped: raw microphone; retry in ${MODELS_RETRY_MS / 1000} s.", "error" to (e.message ?: e.toString())))
        setMode(Mode.RAW, RawReason.FAILED)
        if (pipe != 0L) Native.pipelineFree(pipe)
        pipe = 0
        record?.completeExceptionally(AlkoveException("engine.failed", "The engine stopped during the recording."))
        record = null
        try {
            makeRawPipe()
        } catch (t: Throwable) {
            rawFailed(t, System.currentTimeMillis())
        }
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
        val d = m["data"] as? Map<*, *>
        if (m["type"] == "diag" && d != null) m["data"] = LinkedHashMap(d.obj()).also { x -> for (k in arrayOf("start", "end")) (x[k] as? Number)?.let { x[k] = it.toDouble() + shift } }
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
                    // unanswered, as on the web: a held wake does not wake, a held phrase is the user's after a second
                    background("verify") {
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
        Native.pipelineSet(pipe, Values.encode(mapOf("enabled" to enabled, "print" to print?.embedding) + (sample?.let { mapOf("sample" to it) } ?: emptyMap()) + (if (learning) mapOf("learn" to true) else emptyMap())))
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

    /** Focus keeps this voice from the next silence on; null goes back to the closest voice. While the print learns, learning starts again from it. */
    fun setVoiceprint(p: Voiceprint?) = applyPrint(p, null)

    /** A few seconds of the user alone at 16 kHz: its print, made by Timbre in the background, and the recording as Focus's sample. */
    fun setVoiceprint(recording: FloatArray) {
        scope.launch {
            try {
                applyPrint(embedRecording(recording), recording)
            } catch (e: Exception) {
                if (e is CancellationException) throw e
                diag("voiceprint.reset", "warn", mapOf("message" to "A recording as the voiceprint: ${e.message ?: e}"))
            }
        }
    }

    private fun applyPrint(p: Voiceprint?, s: FloatArray?) {
        print = p
        sample = s
        changed(mapOf("print" to p?.embedding) + (if (s != null) mapOf("sample" to s) else emptyMap()))
        if (learning) background("learner") { Native.coachLearner(coach, Values.encode(mapOf("start" to p?.value))) }
    }

    /** Drops the print and what was learned of it. */
    fun forgetVoiceprint() {
        print = null
        sample = null
        changed(mapOf("print" to null))
        background("forget") { Native.coachForget(coach) }
    }

    /** The app's own playback starts or stops (an agent's voice): the interruptions come while it plays; the first call starts learning the print, which skips what the far end may have put in the microphone. */
    fun farEndActive(on: Boolean) {
        farActive = on
        if (on) farStarted = true
        if (farCalled) return
        farCalled = true
        if (modelsRan) scope.launch { startLearning() }
    }

    /** Speech heard and noise removed so far, null before the models run or once closed. */
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
                spotModel = Alkove.models(app, listOf("spot")).ptr("spot")
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

/** An agent's wake words on the raw microphone: [Word]s, their JSON, or texts typed in `lang`; each [Wake] decides whether the user is far for what follows (diag `wake.far`), its voice the print unless the app gave one (diag `wake.print`). */
data class WakeWords @JvmOverloads constructor(val words: List<Any>, val lang: List<String> = emptyList(), val context: String = "pause") {
    @JvmOverloads
    constructor(words: List<Any>, lang: String, context: String = "pause") : this(words, listOf(lang), context)

    init {
        require(words.isNotEmpty()) { "wake.words is empty" }
        require(lang.isNotEmpty() || words.none(::typed)) { "wake.lang is needed for a typed word" }
    }

    // at the start: a language Lex lacks, a word compiled for another Spot
    internal fun check() {
        checkLangs(lang)
        words.filterNot(::typed).forEach { Word.of(it) }
    }

    internal fun compiled(lex: Lex?): List<Word> = words.map { if (typed(it)) compile(it as String, lang, lex) else Word.of(it) }
}

private fun typed(w: Any) = w is String && !w.trimStart().startsWith("{")

/** What an agent input does with the user's voice; it learns the print from the first [AlkoveInput.farEndActive] call on. */
data class AgentOptions @JvmOverloads constructor(
    /** A [Voiceprint], or a few seconds of the user alone at 16 kHz (FloatArray): its print made by Timbre before the models run, and the recording as Focus's sample. */
    val voiceprint: Any? = null,
    val output: Output = Output(),
    val keepRecent: Double = 0.0,
    val trace: Boolean = false,
    val device: AudioDeviceInfo? = null,
    /** A voice 10 dB under the session's main voice opens no phrase, but a diag `speech.far`. */
    val near: Boolean = false,
    /** When [Event.Turn] and [Event.TurnEnd] come. */
    val turn: TurnParams = TurnParams(),
    /** [Wake]s on the raw microphone, each one deciding whether the user is far; null: off. */
    val wake: WakeWords? = null,
    /** Phrases, turns and interruptions only in the print's voice (`voiceprint`, or the one learned): Timbre hears 0.6 s of each first, another voice gives a diag `speech.other-voice`. */
    val onlyMyVoice: Boolean = false,
) {
    init {
        checkVoice(voiceprint)
    }
}

internal fun agentPipelineOptions(opts: AgentOptions, print: Voiceprint?, output: Map<String, Any?>, sample: FloatArray? = null, learn: Boolean = false) =
    mapOf(
        "chain" to mapOf("gate" to false, "normalize" to true, "strength" to 1.0, "print" to print?.embedding, "sample" to sample, "levels" to true, "near" to opts.near),
        "learn" to learn,
        "keepRecent" to opts.keepRecent,
        "trace" to opts.trace,
        "stats" to true,
        "output" to output,
        "turn" to opts.turn.value(),
        "interrupt" to true,
        "wake" to (opts.wake != null),
        "onlyMyVoice" to opts.onlyMyVoice,
    )

/**
 * The user's voice for a voice agent: Focus and Cue on the microphone, the audio at the rate and format the agent's API takes,
 * speech start and end, and each utterance with its pre-roll. `onAudio` runs on the audio thread: hand the chunk over, do not block.
 */
class AgentInput private constructor(context: Context, private val opts: AgentOptions, private val onAudio: ((ByteArray) -> Unit)?) : AlkoveInput(context, true, opts.device) {
    init {
        print = opts.voiceprint as? Voiceprint
    }

    private val output = opts.output.let { it.copy(chunkMs = maxOf(16, Math.round(it.chunkMs / 16.0).toInt() * 16)) }

    private fun outputValue(utterance: Boolean) =
        mapOf("format" to if (output.format == Format.S16) "s16" else "f32", "rate" to output.rate, "chunkMs" to output.chunkMs, "audio" to true, "utterance" to utterance)

    override fun models() = listOf("focus", "cue")

    override fun wantsBackground() = learning || opts.onlyMyVoice

    override fun learns() = farCalled

    @Volatile
    private var recorded: Voiceprint? = null

    // a setVoiceprint() that came first wins
    override suspend fun resolveVoice() {
        val r = opts.voiceprint as? FloatArray ?: return
        if (recorded != null) return
        val p = embedRecording(r)
        recorded = p
        if (print == null && sample == null) {
            print = p
            sample = r
        }
    }

    // the print Timbre checks the phrases against; while it learns, the one being learned
    override fun reference() = if (opts.onlyMyVoice && !learning) opts.voiceprint as? Voiceprint ?: recorded else null

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

    override fun pipelineOptions() = agentPipelineOptions(opts, print, outputValue(true), sample, learning)

    // Lex only for a typed word: loaded once for the process
    override suspend fun startWords() =
        opts.wake?.let { w ->
            val lex = if (w.words.any(::typed)) Lex(Alkove.models(app, listOf("lex"))) else null
            lex.use { mapOf("words" to w.compiled(it).map(Word::value), "context" to w.context) }
        }

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
            if (Alkove.supported) options.wake?.check()
            return AgentInput(context, options, onAudio).also { it.start() }
        }
    }
}

/** What a wake listener listens for. `voiceprint`: only this voice wakes it, checked by Timbre on the word and the 0.5 s after it; a [Voiceprint], or a few seconds of the user alone at 16 kHz (FloatArray), made a print before the models run. */
data class WakeOptions @JvmOverloads constructor(
    val words: List<Any>,
    val context: String = "pause",
    val voiceprint: Any? = null,
    val trace: Boolean = false,
    val device: AudioDeviceInfo? = null,
) {
    init {
        checkVoice(voiceprint)
    }
}

/** Spot alone on the raw microphone: wakes on [events], each with the word's raw audio. */
class WakeListener private constructor(context: Context, private val opts: WakeOptions) : AlkoveInput(context, false, opts.device) {
    private val words = if (Alkove.supported) opts.words.map(Word::of) else emptyList()

    init {
        require(opts.words.isNotEmpty()) { "WakeListener: no word" }
        for (w in words) if (w.chain == "focus") emit(Event.Diag("word.chain", "warn", mapOf("message" to "\"${w.text}\" was enrolled behind Focus: its thresholds are off on the raw microphone.")))
    }

    override fun models() = listOf("spot")

    override fun spots() = true

    override fun wantsBackground() = opts.voiceprint != null

    @Volatile
    private var recorded: Voiceprint? = null

    override suspend fun resolveVoice() {
        val r = opts.voiceprint as? FloatArray ?: return
        if (recorded == null) recorded = embedRecording(r)
    }

    override fun reference() = opts.voiceprint as? Voiceprint ?: recorded

    override fun pipelineOptions() =
        mapOf("chain" to null, "words" to words.map { it.value }, "context" to opts.context, "trace" to opts.trace, "verify" to wantsBackground())

    /** Listen for other words from now on. */
    @JvmOverloads
    fun setWords(words: List<Any>, context: String = opts.context) = listenFor(words.map(Word::of), context)

    companion object {
        @JvmStatic
        fun start(context: Context, options: WakeOptions): WakeListener = WakeListener(context, options).also { it.start() }
    }
}

// Spot on the raw microphone, the three takes counted in the pipeline
internal class EnrollInput(context: Context, private val word: Word) : AlkoveInput(context, false, null) {
    override fun models() = listOf("spot")

    override fun spots() = true

    override fun pipelineOptions() = mapOf("chain" to null, "enroll" to mapOf("word" to word.value))
}
