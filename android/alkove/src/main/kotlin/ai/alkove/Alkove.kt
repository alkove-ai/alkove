package ai.alkove

import android.content.Context
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Deferred
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.async
import kotlinx.coroutines.withContext

object Alkove {
    /** The models' rate, and the rate of every output, in Hz. */
    const val RATE = 16000

    /** `near = NEAR_GAP_DB`'s gap: a voice this many dB under the session's main voice is far. */
    const val NEAR_GAP_DB = 10.0

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

    /**
     * The models the real-time inputs share in this process: loaded once, from the cache or Hugging Face (`baseUrl` for your own host).
     * Closing what it returns frees nothing.
     */
    @JvmStatic
    suspend fun models(context: Context, names: Collection<String>, baseUrl: String? = null, onProgress: ((Progress) -> Unit)? = null): Models {
        val cache = Models.cacheDir(context.applicationContext)
        val loading = synchronized(shared) {
            names.associateWith { n -> shared.getOrPut(n) { loader.async { Models.load(listOf(n), cache = cache, baseUrl = baseUrl, onProgress = onProgress).model(n) } } }
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

    /** Downloads the models of a usage ahead of time (a settings screen): `call` and `agent` need Focus and Cue, `wake` Spot, plus Lex for a typed word. */
    @JvmStatic
    suspend fun preload(context: Context, names: Collection<String>, baseUrl: String? = null, onProgress: ((Progress) -> Unit)? = null) {
        withContext(Dispatchers.IO) { Models.fetch(names, Models.cacheDir(context.applicationContext), baseUrl = baseUrl, onProgress = onProgress) }
    }

    /** True when these models are in the cache: an input can start without network. */
    @JvmStatic
    fun cached(context: Context, names: Collection<String>): Boolean = supported && Models.cached(names, Models.cacheDir(context.applicationContext))
}
