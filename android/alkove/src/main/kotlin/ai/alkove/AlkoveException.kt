package ai.alkove

/** An error with the code the web SDK gives it (`models.hash`, `word.outdated`, `mic.denied`…): see the errors guide. */
class AlkoveException(val code: String, message: String) : Exception(message) {
    // what the native library throws: the code, a newline, the message
    internal constructor(packed: String) : this(packed.substringBefore('\n'), packed.substringAfter('\n', ""))
}

// a native object freed once, by close() or else when it is collected
internal class Handle(h: Long, private val free: (Long) -> Unit) {
    @Volatile
    private var h = h

    val value: Long
        get() = h.also { check(it != 0L) { "used after close()" } }

    val open: Boolean
        get() = h != 0L

    fun close() {
        val x = h
        h = 0
        if (x != 0L) free(x)
    }
}
