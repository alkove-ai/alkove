package ai.alkove

import android.content.Context
import java.io.File
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Deferred
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.async
import kotlinx.coroutines.withContext

/** What [Alkove.preload] and [Alkove.cached] get the models of: a call ([CallProcessor], LiveKit) or an agent need Focus, Cue and Timbre, a wake listener Spot. */
enum class Usage {
    CALL,
    AGENT,
    WAKE;

    internal fun names(voiceprint: Boolean, typedWords: Boolean): List<String> =
        (if (this == WAKE) listOf("spot") + (if (voiceprint) listOf("timbre") else emptyList()) else listOf("focus", "cue", "timbre")) + if (typedWords) listOf("lex") else emptyList()
}

// where the models come from: Alkove.configure's
internal class ModelConfig(val modelBaseUrl: String?, val dir: File?, val assets: String?, val models: Map<String, Any>)

object Alkove {
    /** The models' rate, and the rate of every output, in Hz. */
    const val RATE = 16000

    @Volatile
    internal var config = ModelConfig(null, null, null, emptyMap())
        private set

    /** Where every input, [models], [preload], [cached] and `Models.load(context)` take the models: `models` a model's File or bytes, else `dir` (a folder of .alk files) or `assets` (a folder of the app's assets), else the app's cache, filled from `modelBaseUrl` or Hugging Face. Before the first input: a model loaded keeps its source. */
    @JvmStatic
    @JvmOverloads
    fun configure(modelBaseUrl: String? = null, dir: File? = null, assets: String? = null, models: Map<String, Any> = emptyMap()) {
        require(dir == null || assets == null) { "dir or assets, not both" }
        for ((n, v) in models) require(v is ByteArray || v is File) { "models[\"$n\"]: a File or a ByteArray" }
        config = ModelConfig(modelBaseUrl, dir, assets, models.toMap())
    }

    internal val info: Map<String, Any?> by lazy {
        native()
        Values.decode(Native.info()).obj()
    }

    // before any call into the native library
    internal fun native() {
        if (!supported) throw AlkoveException("engine.unsupported", "No native library for this device (${android.os.Build.SUPPORTED_ABIS.joinToString()}): Alkove ships arm64-v8a and x86_64.")
    }

    // for the tests of the fallback
    internal var supportedOverride: Boolean? = null

    private val loaded: Boolean by lazy {
        try {
            Native.info()
            true
        } catch (_: UnsatisfiedLinkError) {
            false
        } catch (_: NoClassDefFoundError) {
            false
        }
    }

    /** False on a device whose ABI has no native library (32-bit ARM): the inputs then send the raw microphone (`RAW`, `UNSUPPORTED`), the rest throws `engine.unsupported`. */
    @JvmStatic
    val supported: Boolean
        get() = supportedOverride ?: loaded

    /** This library's version: the same minor as the web and Python packages means the same core and models. */
    @JvmStatic
    val version: String
        get() = BuildConfig.VERSION

    // each model loaded once per process, the ones still loading shared by whoever waits for them
    private val shared = HashMap<String, Deferred<Model>>()
    private val loader = CoroutineScope(SupervisorJob() + Dispatchers.IO)

    /** The models the real-time inputs share in this process, from where [configure] says: loaded once; closing what it returns frees nothing. */
    @JvmStatic
    suspend fun models(context: Context, names: Collection<String>, onProgress: ((Progress) -> Unit)? = null): Models {
        val app = context.applicationContext
        val loading = synchronized(shared) {
            Models.withTimbre(names).associateWith { n -> shared.getOrPut(n) { loader.async { Models.configured(app, listOf(n), onProgress).model(n) } } }
        }
        return Models(
            loading.mapValues { (n, d) ->
                try {
                    d.await()
                } catch (e: Exception) {
                    // a failed load is tried again by the next caller
                    synchronized(shared) { if (shared[n] === d && d.isCompleted) shared.remove(n) }
                    throw e
                }
            },
            owned = false,
        )
    }

    /** Downloads what a usage needs ahead of time (a settings screen): `voiceprint` adds Timbre to a wake listener's, `typedWords` Lex; models from `dir`, `assets` or `models` are only checked. */
    @JvmStatic
    suspend fun preload(context: Context, usage: Usage, voiceprint: Boolean = false, typedWords: Boolean = false, onProgress: ((Progress) -> Unit)? = null) {
        val app = context.applicationContext
        withContext(Dispatchers.IO) { Models.fetchOnly(Models.remote(app, usage.names(voiceprint, typedWords)), Models.cacheDir(app), true, onProgress) }
    }

    /** True when what a usage needs is on the device: an input can start without network. */
    @JvmStatic
    @JvmOverloads
    fun cached(context: Context, usage: Usage, voiceprint: Boolean = false, typedWords: Boolean = false): Boolean = supported && Models.local(context.applicationContext, usage.names(voiceprint, typedWords))
}
