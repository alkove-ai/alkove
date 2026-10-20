package ai.alkove

import java.io.ByteArrayOutputStream
import java.nio.ByteBuffer
import java.nio.ByteOrder

// the core's tagged values: null, Boolean, Double, String, List, Map (in order), ByteArray, FloatArray
internal object Values {
    fun encode(v: Any?): ByteArray {
        val out = ByteArrayOutputStream()
        write(v, out)
        return out.toByteArray()
    }

    private fun u32(out: ByteArrayOutputStream, n: Int) {
        out.write(n and 0xFF)
        out.write((n ushr 8) and 0xFF)
        out.write((n ushr 16) and 0xFF)
        out.write((n ushr 24) and 0xFF)
    }

    private fun text(out: ByteArrayOutputStream, s: String) {
        val b = s.toByteArray(Charsets.UTF_8)
        u32(out, b.size)
        out.write(b)
    }

    private fun write(v: Any?, out: ByteArrayOutputStream) {
        when (v) {
            null -> out.write(0)
            is Boolean -> out.write(if (v) 2 else 1)
            is Number -> {
                out.write(3)
                out.write(ByteBuffer.allocate(8).order(ByteOrder.LITTLE_ENDIAN).putDouble(v.toDouble()).array())
            }
            is String -> {
                out.write(4)
                text(out, v)
            }
            is FloatArray -> {
                out.write(8)
                u32(out, v.size)
                val b = ByteBuffer.allocate(4 * v.size).order(ByteOrder.LITTLE_ENDIAN)
                b.asFloatBuffer().put(v)
                out.write(b.array())
            }
            is ByteArray -> {
                out.write(7)
                u32(out, v.size)
                out.write(v)
            }
            is Map<*, *> -> {
                out.write(6)
                u32(out, v.size)
                for ((k, x) in v) {
                    text(out, k as String)
                    write(x, out)
                }
            }
            is Collection<*> -> {
                out.write(5)
                u32(out, v.size)
                for (x in v) write(x, out)
            }
            is Array<*> -> write(v.asList(), out)
            is DoubleArray -> write(v.asList(), out)
            is IntArray -> write(v.asList(), out)
            else -> throw IllegalArgumentException("not a value: ${v::class.java.name}")
        }
    }

    fun decode(b: ByteArray): Any? {
        val buf = ByteBuffer.wrap(b).order(ByteOrder.LITTLE_ENDIAN)
        val v = read(buf)
        check(!buf.hasRemaining()) { "trailing bytes" }
        return v
    }

    private fun string(b: ByteBuffer): String {
        val n = b.int
        val s = String(b.array(), b.position(), n, Charsets.UTF_8)
        b.position(b.position() + n)
        return s
    }

    private fun read(b: ByteBuffer): Any? =
        when (val tag = b.get().toInt()) {
            0 -> null
            1 -> false
            2 -> true
            3 -> b.double
            4 -> string(b)
            5 -> List(b.int) { read(b) }
            6 -> {
                val n = b.int
                val m = LinkedHashMap<String, Any?>(n)
                repeat(n) { m[string(b)] = read(b) }
                m
            }
            7 -> ByteArray(b.int).also { b.get(it) }
            8 -> FloatArray(b.int).also { b.asFloatBuffer().get(it); b.position(b.position() + 4 * it.size) }
            else -> error("unknown tag $tag")
        }
}

// JSON read and written as JavaScript does: the same text as the web SDK for the same word or print
internal object Json {
    fun stringify(v: Any?): String {
        Alkove.native()
        return String(Native.jsonStringify(Values.encode(v)), Charsets.UTF_8)
    }

    fun parse(text: String): Any? {
        Alkove.native()
        return Values.decode(Native.jsonParse(text.toByteArray(Charsets.UTF_8)))
    }
}

@Suppress("UNCHECKED_CAST")
internal fun Any?.obj(): Map<String, Any?> = this as Map<String, Any?>

@Suppress("UNCHECKED_CAST")
internal fun Any?.arr(): List<Any?> = this as List<Any?>

internal fun Any?.num(): Double = (this as Number).toDouble()

internal fun Map<String, Any?>.str(k: String): String = this[k] as String
