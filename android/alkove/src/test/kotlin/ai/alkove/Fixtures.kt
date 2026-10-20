package ai.alkove

import java.io.File
import java.nio.ByteBuffer
import java.nio.ByteOrder
import java.security.MessageDigest
import org.junit.Assume.assumeTrue

// the streams of packages/alkove/test/core.test.ts, drawn the same way: the native core must give the digests the web SDK gives
object Fixtures {
    val repo = File(System.getProperty("alkove.repo") ?: "../..")
    val dir = File(repo, "packages/alkove/test/fixtures")
    private val modelDir = File(repo, "packages/alkove/.cache/models")
    val golden: Map<String, Any?> by lazy { Json.parse(File(dir, "core.json").readText()).obj() }

    // the SDK's test models when they are there, else Hugging Face into a temporary cache
    fun load(vararg names: String): Models {
        val local = names.all { File(modelDir, Models.MANIFEST["models"].obj()[it].obj().str("file")).exists() }
        if (local) return Models.load(names.toList(), dir = modelDir)
        assumeTrue("no models in packages/alkove/.cache/models", System.getenv("ALKOVE_FETCH") != null)
        return Models.load(names.toList(), cache = File(System.getProperty("java.io.tmpdir"), "alkove-test"))
    }

    fun rng(seed: Long): () -> Double {
        var s = seed and 0xFFFFFFFFL
        return {
            s = (s * 1664525 + 1013904223) and 0xFFFFFFFFL
            s / 4294967296.0
        }
    }

    fun noise(n: Int, seed: Long, scale: Double): FloatArray {
        var s = seed and 0xFFFFFFFFL
        return FloatArray(n) {
            s = (s * 1664525 + 1013904223) and 0xFFFFFFFFL
            ((s / 4294967296.0 * 2 - 1) * scale).toFloat()
        }
    }

    fun wav(name: String): FloatArray {
        val b = File(dir, name).readBytes()
        // the data chunk of the fixtures' 16-bit WAV
        var at = 12
        while (String(b, at, 4, Charsets.US_ASCII) != "data") at += 8 + ByteBuffer.wrap(b, at + 4, 4).order(ByteOrder.LITTLE_ENDIAN).int
        val n = ByteBuffer.wrap(b, at + 4, 4).order(ByteOrder.LITTLE_ENDIAN).int / 2
        val s = ByteBuffer.wrap(b, at + 8, 2 * n).order(ByteOrder.LITTLE_ENDIAN).asShortBuffer()
        return FloatArray(n) { (s.get(it) / 32768.0).toFloat() }
    }

    fun f32(name: String, n: Int): FloatArray {
        val b = ByteBuffer.wrap(File(dir, name).readBytes()).order(ByteOrder.LITTLE_ENDIAN).asFloatBuffer()
        return FloatArray(n) { b.get(it) }
    }

    val voices by lazy { listOf(wav("speech.wav"), wav("mix.wav"), wav("enroll.wav")) }

    fun stream(seed: Long, seconds: Int, dense: Boolean = false): FloatArray {
        val r = rng(seed)
        val out = FloatArray(seconds * 16000)
        var o = 0
        while (o < out.size) {
            val kind = if (dense) 0.0 else r()
            val n = minOf(out.size - o, Math.floor((0.3 + r() * 4) * 16000).toInt())
            when {
                kind < 0.5 -> {
                    val v = voices[Math.floor(r() * voices.size).toInt()]
                    val start = Math.floor(r() * v.size).toInt()
                    val gain = 0.05 + r() * 2
                    for (k in 0 until n) out[o + k] = (v[(start + k) % v.size] * gain).toFloat()
                }
                kind < 0.75 -> if (r() >= 0.5) noise(n, seed + o, 1e-6).copyInto(out, o)
                else -> noise(n, seed + o, 0.002 + r() * 0.05).copyInto(out, o)
            }
            o += n
        }
        return out
    }

    // JSON.stringify of the canonical form: keys sorted, typed arrays as arrays
    fun canonical(v: Any?): Any? =
        when (v) {
            is Map<*, *> -> v.entries.sortedBy { it.key as String }.associate { it.key as String to canonical(it.value) }
            is List<*> -> v.map(::canonical)
            is Array<*> -> v.map(::canonical)
            is FloatArray -> v.map(Float::toDouble)
            else -> v
        }

    fun sha256(b: ByteArray): String = MessageDigest.getInstance("SHA-256").digest(b).joinToString("") { "%02x".format(it) }

    fun digest(key: String, outputs: Any?) {
        val hex = sha256(Json.stringify(canonical(outputs)).toByteArray(Charsets.UTF_8))
        org.junit.Assert.assertEquals(key, golden[key], hex)
    }
}

class Digest {
    private val md = MessageDigest.getInstance("SHA-256")
    private val buf = ByteBuffer.allocate(8).order(ByteOrder.LITTLE_ENDIAN)

    fun f64(vararg xs: Double) {
        for (x in xs) {
            buf.clear()
            md.update(buf.putDouble(x).array())
        }
    }

    fun f32(x: FloatArray, n: Int = x.size) {
        val b = ByteBuffer.allocate(4 * n).order(ByteOrder.LITTLE_ENDIAN)
        for (i in 0 until n) b.putFloat(x[i])
        md.update(b.array())
    }

    fun hex(): String = md.digest().joinToString("") { "%02x".format(it) }
}
