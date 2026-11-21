package ai.alkove

import android.content.Context
import java.io.File
import java.io.IOException
import java.net.HttpURLConnection
import java.net.URL
import java.security.MessageDigest
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext

/** A model loaded from its .alk bytes, shared by every stream built on it. */
class Model internal constructor(val name: String, bytes: ByteArray) : AutoCloseable {
    private val handle = Alkove.native().let { Handle(Native.modelLoad(bytes), Native::modelFree) }

    internal val ptr: Long
        @Synchronized get() = handle.value

    @Synchronized
    override fun close() = handle.close()

    protected fun finalize() = handle.close()
}

/** Bytes downloaded so far, out of `total`. */
data class Progress(val loaded: Long, val total: Long)

/** Models loaded once and shared by every stream, in any thread: Focus, Cue, Spot, Timbre, Lex. */
class Models internal constructor(private val loaded: Map<String, Model>, private val owned: Boolean = true) : AutoCloseable {
    operator fun contains(name: String): Boolean = name in loaded

    /** The models loaded, by name. */
    val names: Set<String>
        get() = loaded.keys

    internal fun ptr(name: String): Long = loaded[name]?.ptr ?: 0L

    internal fun model(name: String): Model = loaded.getValue(name)

    // the models held, by name for one order everywhere, while a native object takes its own reference: neither close() nor the collector frees them meanwhile
    internal fun <T> pinned(block: () -> T): T = hold(loaded.values.sortedBy { it.name }, 0, block)

    private fun <T> hold(ms: List<Model>, i: Int, block: () -> T): T = if (i == ms.size) block() else synchronized(ms[i]) { hold(ms, i + 1, block) }

    internal fun need(vararg names: String) {
        val missing = names.filter { it !in loaded }
        require(missing.isEmpty()) { "${missing.joinToString()} not loaded: Models.load(context, listOf(${names.joinToString { "\"$it\"" }}))" }
    }

    /** Frees the models; streams already built on them keep theirs. A shared set (`Alkove.models`) stays loaded. */
    override fun close() {
        if (owned) loaded.values.forEach(Model::close)
    }

    override fun toString() = "Models(${loaded.keys.joinToString()})"

    companion object {
        /** The models this version runs, by name: their file, size, sha256 and revision on Hugging Face. */
        @JvmStatic
        val MANIFEST: Map<String, Any?> by lazy { Json.parse(Alkove.info.str("manifest")).obj() }

        /** Every model name, in the manifest's order. */
        @JvmStatic
        val NAMES: List<String> by lazy { MANIFEST["models"].obj().keys.toList() }

        // Focus takes its user's voice through Timbre: loaded with it
        internal fun withTimbre(names: Collection<String>): List<String> = names.distinct().let { if ("focus" in it && "timbre" !in it) it + "timbre" else it }

        private fun spec(name: String): Map<String, Any?> =
            MANIFEST["models"].obj()[name]?.obj() ?: throw IllegalArgumentException("no model named \"$name\": ${NAMES.joinToString()}")

        /** Where `Models.load(context)` keeps the files: the app's no-backup folder, so Auto Backup never uploads them. */
        @JvmStatic
        fun cacheDir(context: Context): File = File(context.noBackupFilesDir, "alkove")

        internal fun checked(name: String, data: ByteArray): ByteArray {
            val m = spec(name)
            val digest = MessageDigest.getInstance("SHA-256").digest(data).joinToString("") { "%02x".format(it) }
            if (digest != m.str("sha256")) throw AlkoveException("models.hash", "${m.str("name")}: sha256 $digest, expected ${m.str("sha256")} (a truncated download, or the file of another version)")
            return data
        }

        private fun url(name: String, base: String?): String {
            val m = spec(name)
            if (base != null) return base.trimEnd('/') + "/" + m.str("file")
            val revision = m["revision"] as String? ?: throw AlkoveException("models.fetch", "${m.str("name")}: not on Hugging Face in this version, set modelBaseUrl, dir or assets (Alkove.configure)")
            return "https://huggingface.co/${MANIFEST.str("org")}/${m.str("name")}/resolve/$revision/${m.str("file")}"
        }

        // `before` bytes of the other files already counted, `total` of them all: one progress across the files
        private fun download(url: String, name: String, before: Long, total: Long, onProgress: ((Progress) -> Unit)?): ByteArray {
            try {
                val c = URL(url).openConnection() as HttpURLConnection
                c.connectTimeout = 30_000
                c.readTimeout = 60_000
                try {
                    if (c.responseCode !in 200..299) throw AlkoveException("models.fetch", "HTTP ${c.responseCode}: $url")
                    val out = java.io.ByteArrayOutputStream(spec(name).num("bytes").toInt())
                    val buf = ByteArray(64 * 1024)
                    c.inputStream.use { s ->
                        while (true) {
                            val n = s.read(buf)
                            if (n < 0) break
                            out.write(buf, 0, n)
                            onProgress?.invoke(Progress(before + out.size(), total))
                        }
                    }
                    return out.toByteArray()
                } finally {
                    c.disconnect()
                }
            } catch (e: IOException) {
                throw AlkoveException("models.fetch", "${spec(name).str("name")}: ${e.message}")
            }
        }

        /** Downloads the models into `cache` once, from `modelBaseUrl` ([Alkove.configure]) or Hugging Face, checked by their sha256, and gives their files; `download = false` only reads the cache. */
        @JvmStatic
        @JvmOverloads
        fun fetch(names: Collection<String>, cache: File, download: Boolean = true, onProgress: ((Progress) -> Unit)? = null): Map<String, File> = fetchOnly(withTimbre(names), cache, download, onProgress)

        // these models and no other
        internal fun fetchOnly(names: Collection<String>, cache: File, download: Boolean, onProgress: ((Progress) -> Unit)?): Map<String, File> {
            val baseUrl = Alkove.config.modelBaseUrl
            val missing = names.filter { !path(cache, it).exists() }
            val total = missing.sumOf { spec(it).num("bytes") }
            var before = 0L
            return names.associateWith { name ->
                val m = spec(name)
                val path = path(cache, name)
                if (!path.exists()) {
                    if (!download) throw AlkoveException("models.fetch", "${m.str("name")}: not in the cache $cache, fetch it first")
                    val data = checked(name, download(url(name, baseUrl), name, before, total, onProgress))
                    before += data.size
                    path.parentFile!!.mkdirs()
                    // whole or not at all: another process may read the cache meanwhile; on disk before the rename, a power cut leaves no short file
                    val tmp = File.createTempFile(m.str("file"), ".part", path.parentFile)
                    try {
                        java.io.FileOutputStream(tmp).use {
                            it.write(data)
                            it.fd.sync()
                        }
                        if (!tmp.renameTo(path) && !path.exists()) throw AlkoveException("models.fetch", "${m.str("name")}: cannot write $path")
                    } finally {
                        tmp.delete()
                    }
                }
                path
            }
        }

        private fun path(cache: File, name: String) = File(File(cache, spec(name).str("sha256")), spec(name).str("file"))

        /** True when every model is in the cache: an input can start without network. */
        @JvmStatic
        fun cached(names: Collection<String>, cache: File): Boolean = withTimbre(names).all { path(cache, it).exists() }

        /** Blocking: `files` gives a model's File or bytes, `dir` a folder of .alk files, else they come from `cache`, downloaded there when missing; each file checked against the sha256 this version pins (`models.hash`). */
        @JvmStatic
        @JvmOverloads
        fun load(
            names: Collection<String>,
            cache: File? = null,
            dir: File? = null,
            files: Map<String, Any> = emptyMap(),
            download: Boolean = true,
            onProgress: ((Progress) -> Unit)? = null,
        ): Models = loadOnly(withTimbre(names), cache, dir, files, download, onProgress)

        // these models and no other: Alkove.models loads and shares each one apart
        internal fun loadOnly(names: Collection<String>, cache: File?, dir: File?, files: Map<String, Any>, download: Boolean, onProgress: ((Progress) -> Unit)?): Models {
            val loaded = LinkedHashMap<String, Model>()
            try {
                for (name in names.distinct()) {
                    val data = when (val src = files[name]) {
                        is ByteArray -> src
                        is File -> src.readBytes()
                        null -> when {
                            dir != null -> File(dir, spec(name).str("file")).readBytes()
                            cache != null -> fromCache(name, cache, download, onProgress)
                            else -> throw IllegalArgumentException("$name: no file, folder or cache given")
                        }
                        else -> throw IllegalArgumentException("$name: a File or a ByteArray")
                    }
                    loaded[name] = Model(name, if (files[name] == null && dir == null && cache != null) data else checked(name, data))
                }
            } catch (e: Throwable) {
                loaded.values.forEach(Model::close)
                throw e
            }
            return Models(loaded)
        }

        // a cached file that no longer matches (a disk error, a file left short) is fetched once more
        private fun fromCache(name: String, cache: File, download: Boolean, onProgress: ((Progress) -> Unit)?): ByteArray {
            val f = fetchOnly(listOf(name), cache, download, onProgress).getValue(name)
            return try {
                checked(name, f.readBytes())
            } catch (e: AlkoveException) {
                if (!download) throw e
                f.delete()
                checked(name, fetchOnly(listOf(name), cache, true, onProgress).getValue(name).readBytes())
            }
        }

        /** The models from where [Alkove.configure] says, by default from the app's cache, filled from Hugging Face at this version's revision. Call it ahead on a settings screen, on Wi-Fi if you can. */
        @JvmStatic
        suspend fun load(context: Context, names: Collection<String> = listOf("focus", "cue"), onProgress: ((Progress) -> Unit)? = null): Models =
            withContext(Dispatchers.IO) { configured(context.applicationContext, withTimbre(names), onProgress) }

        // a model given, or the folder or the assets configured: nothing on the network
        private fun given(name: String): Boolean = Alkove.config.let { name in it.models || it.dir != null || it.assets != null }

        private fun missing(name: String, where: String) = AlkoveException("models.fetch", "${spec(name).str("name")}: no ${spec(name).str("file")} in $where")

        private fun <T> asset(context: Context, folder: String, name: String, read: (java.io.InputStream) -> T): T =
            try {
                context.assets.open("$folder/${spec(name).str("file")}").use(read)
            } catch (_: IOException) {
                throw missing(name, "the assets' $folder")
            }

        private fun inDir(dir: File, name: String): File = File(dir, spec(name).str("file")).takeIf { it.exists() } ?: throw missing(name, dir.path)

        // these models from where Alkove.configure says
        internal fun configured(context: Context, names: Collection<String>, onProgress: ((Progress) -> Unit)?): Models {
            val c = Alkove.config
            val files = names.filter { it !in c.models }.mapNotNull { n ->
                when {
                    c.assets != null -> n to asset(context, c.assets, n) { it.readBytes() }
                    c.dir != null -> n to inDir(c.dir, n)
                    else -> null
                }
            }
            return loadOnly(names, cacheDir(context), null, c.models + files, true, onProgress)
        }

        // the ones to download; a model configured off the network must be there
        internal fun remote(context: Context, names: Collection<String>): List<String> {
            val c = Alkove.config
            for (n in names.filter { it !in c.models }) {
                if (c.assets != null) asset(context, c.assets, n) {} else if (c.dir != null) inDir(c.dir, n)
            }
            return names.filterNot(::given)
        }

        // every model on the device, given or in the cache
        internal fun local(context: Context, names: Collection<String>): Boolean {
            val c = Alkove.config
            return names.all { n ->
                when {
                    n in c.models -> true
                    c.assets != null -> runCatching { asset(context, c.assets, n) {} }.isSuccess
                    c.dir != null -> File(c.dir, spec(n).str("file")).exists()
                    else -> path(cacheDir(context), n).exists()
                }
            }
        }
    }
}

private fun Map<String, Any?>.num(k: String): Long = (this[k] as Number).toLong()
