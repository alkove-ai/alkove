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
            val revision = m["revision"] as String? ?: throw AlkoveException("models.fetch", "${m.str("name")}: not on Hugging Face in this version, pass baseUrl or dir")
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

        /**
         * Downloads the models into `cache` once, checked by their sha256, and gives their files. Nothing goes on the network afterwards;
         * `download = false` only reads the cache. `baseUrl` is your own host for the .alk files (self-hosting), else Hugging Face.
         */
        @JvmStatic
        @JvmOverloads
        fun fetch(names: Collection<String>, cache: File, download: Boolean = true, baseUrl: String? = null, onProgress: ((Progress) -> Unit)? = null): Map<String, File> {
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
        fun cached(names: Collection<String>, cache: File): Boolean = names.all { path(cache, it).exists() }

        /**
         * Blocking: `files` gives a model's File or bytes, `dir` a folder of .alk files, else they come from `cache`, downloaded there when missing.
         * Each file is checked against the sha256 this version pins (`models.hash`).
         */
        @JvmStatic
        @JvmOverloads
        fun load(
            names: Collection<String>,
            cache: File? = null,
            dir: File? = null,
            files: Map<String, Any> = emptyMap(),
            download: Boolean = true,
            baseUrl: String? = null,
            onProgress: ((Progress) -> Unit)? = null,
        ): Models {
            val loaded = LinkedHashMap<String, Model>()
            try {
                for (name in names) {
                    val data = when (val src = files[name]) {
                        is ByteArray -> src
                        is File -> src.readBytes()
                        null -> when {
                            dir != null -> File(dir, spec(name).str("file")).readBytes()
                            cache != null -> fromCache(name, cache, download, baseUrl, onProgress)
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
        private fun fromCache(name: String, cache: File, download: Boolean, baseUrl: String?, onProgress: ((Progress) -> Unit)?): ByteArray {
            val f = fetch(listOf(name), cache, download, baseUrl, onProgress).getValue(name)
            return try {
                checked(name, f.readBytes())
            } catch (e: AlkoveException) {
                if (!download) throw e
                f.delete()
                checked(name, fetch(listOf(name), cache, true, baseUrl, onProgress).getValue(name).readBytes())
            }
        }

        /** The models from Hugging Face at this version's revision (or `baseUrl`), then from the app's cache. Call it ahead on a settings screen, on Wi-Fi if you can. */
        @JvmStatic
        suspend fun load(context: Context, names: Collection<String> = listOf("focus", "cue"), baseUrl: String? = null, onProgress: ((Progress) -> Unit)? = null): Models =
            withContext(Dispatchers.IO) { load(names, cache = cacheDir(context), baseUrl = baseUrl, onProgress = onProgress) }

        /** The .alk files the app ships in its assets, under `path`. */
        @JvmStatic
        @JvmOverloads
        fun fromAssets(context: Context, names: Collection<String>, path: String = "alkove"): Models =
            load(names, files = names.associateWith { n -> context.assets.open("$path/${spec(n).str("file")}").use { it.readBytes() } })
    }
}

private fun Map<String, Any?>.num(k: String): Long = (this[k] as Number).toLong()
