package ai.alkove

import android.content.Context
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.withContext

object Alkove {
    /** The models' rate, and the rate of every output, in Hz. */
    const val RATE = 16000

    internal val info: Map<String, Any?> by lazy { Values.decode(Native.info()).obj() }

    /** This library's version: the same minor as the web and Python packages means the same core and models. */
    @JvmStatic
    val version: String
        get() = info.str("version")

    private val shared = HashMap<String, Model>()
    private val loading = Mutex()

    /**
     * The models the real-time inputs share in this process: loaded once, from the cache or Hugging Face (`baseUrl` for your own host).
     * Closing what it returns frees nothing.
     */
    @JvmStatic
    suspend fun models(context: Context, names: Collection<String>, baseUrl: String? = null, onProgress: ((Progress) -> Unit)? = null): Models =
        loading.withLock {
            val missing = names.filter { it !in shared }
            if (missing.isNotEmpty()) {
                val m = withContext(Dispatchers.IO) { Models.load(missing, cache = Models.cacheDir(context.applicationContext), baseUrl = baseUrl, onProgress = onProgress) }
                for (n in missing) shared[n] = m.model(n)
            }
            Models(names.associateWith { shared.getValue(it) }, owned = false)
        }

    /** Downloads the models of a usage ahead of time (a settings screen): `call` and `agent` need Focus and Cue, `wake` Spot, plus Lex for a typed word. */
    @JvmStatic
    suspend fun preload(context: Context, names: Collection<String>, baseUrl: String? = null, onProgress: ((Progress) -> Unit)? = null) {
        withContext(Dispatchers.IO) { Models.fetch(names, Models.cacheDir(context.applicationContext), baseUrl = baseUrl, onProgress = onProgress) }
    }

    /** True when these models are in the cache: an input can start without network. */
    @JvmStatic
    fun cached(context: Context, names: Collection<String>): Boolean = Models.cached(names, Models.cacheDir(context.applicationContext))
}
