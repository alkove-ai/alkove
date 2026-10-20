package ai.alkove

import kotlin.math.ceil

/**
 * Focus and Cue inside a call's own audio path: the WebRTC stack holds the microphone and hands its frames over, each comes back
 * the size it came in, about 37 ms behind ([LEAD], Focus's lag and the resamplers'). For a capture post-processor (LiveKit), or any stack with a hook on the microphone's frames.
 */
class CallProcessor @JvmOverloads constructor(
    private val models: Models,
    voiceprint: Voiceprint? = null,
    strength: Double = 1.0,
    normalize: Boolean = false,
    speech: SpeechParams? = null,
) : AutoCloseable {
    private var print = voiceprint
    private var strength = strength
    private val normalize = normalize
    private val speech = speech
    private var enabled = true
    private var live: Live? = null
    private var rate = 0
    private var held = FloatArray(0)
    private var count = 0
    private var mono = FloatArray(0)
    private var scratch = FloatArray(0)
    private var closed = false

    init {
        models.need("focus", "cue")
        require(voiceprint == null || voiceprint.embedding.size == 192) { "a print of ${voiceprint?.embedding?.size} values: Focus takes 192" }
    }

    /** A fresh stream from the next frame: another track, another call. */
    @Synchronized
    fun reset() {
        live?.close()
        live = null
    }

    /** Frames of any size at `rate`, interleaved over `channels`, processed in place; the speech and diag events they gave. Closed: left as they are. */
    @Synchronized
    @JvmOverloads
    fun process(frame: FloatArray, rate: Int, channels: Int = 1, n: Int = frame.size / channels): List<Event> {
        if (closed) return emptyList()
        val l = stream(rate)
        if (mono.size < n) mono = FloatArray(n)
        for (i in 0 until n) {
            var s = 0.0
            for (c in 0 until channels) s += frame[i * channels + c]
            mono[i] = (s / channels).toFloat()
        }
        val got = l.pushInto(mono, n)
        val events = l.events
        val out = l.output
        if (held.size < count + got) held = held.copyOf(maxOf(2 * held.size, count + got))
        out.copyInto(held, count, 0, got)
        count += got
        // the first frames, before Focus's output catches up, start with silence
        val lack = maxOf(0, n - count)
        for (i in 0 until n) {
            val x = if (i < lack) 0f else held[i - lack]
            for (c in 0 until channels) frame[i * channels + c] = x
        }
        val used = n - lack
        held.copyInto(held, 0, used, count)
        count -= used
        return events
    }

    /** Int16 frames, as WebRTC stacks give them. */
    @Synchronized
    @JvmOverloads
    fun process(frame: ShortArray, rate: Int, channels: Int = 1, n: Int = frame.size / channels): List<Event> {
        val m = n * channels
        if (scratch.size < m) scratch = FloatArray(m)
        for (i in 0 until m) scratch[i] = (frame[i] / 32768.0).toFloat()
        val events = process(scratch, rate, channels, n)
        // as the web writes int16: clipped, ×32768 below zero, ×32767 above, truncated
        for (i in 0 until m) {
            val x = scratch[i].toDouble().coerceIn(-1.0, 1.0)
            frame[i] = (if (x < 0) x * 32768 else x * 32767).toInt().toShort()
        }
        return events
    }

    private fun stream(r: Int): Live {
        live?.let { if (r == rate) return it }
        live?.close()
        rate = r
        val l = Live(models, r, normalize = normalize, strength = strength, voiceprint = print, speech = speech)
        if (!enabled) l.setEnabled(false)
        live = l
        count = ceil(r * LEAD).toInt()
        held = FloatArray(2 * count)
        return l
    }

    /** Applied at the user's next silence; null goes back to the closest voice. */
    @Synchronized
    fun setVoiceprint(p: Voiceprint?) {
        print = p
        live?.setVoiceprint(p)
    }

    @Synchronized
    fun setStrength(s: Double) {
        strength = s
        live?.setStrength(s)
    }

    /** Off: the raw microphone, at the same delay, with a short fade. */
    @Synchronized
    fun setEnabled(on: Boolean) {
        enabled = on
        live?.setEnabled(on)
    }

    @Synchronized
    override fun close() {
        closed = true
        reset()
    }

    companion object {
        /** Held ahead of Live's output, in seconds: one hop's wait, so each frame goes out the size it came in. */
        const val LEAD = 0.02
    }
}
