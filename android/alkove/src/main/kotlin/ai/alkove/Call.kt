package ai.alkove

import kotlin.math.ceil

/**
 * Focus and Cue inside a call's own audio path: the WebRTC stack holds the microphone and hands its frames over, each comes back
 * the size it came in, [LEAD] behind. For a capture post-processor (LiveKit), or any stack with a hook on the microphone's frames.
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

    /** A fresh stream from the next frame: another track, another call. */
    @Synchronized
    fun reset() {
        live?.close()
        live = null
    }

    /** Frames of any size at `rate`, interleaved over `channels`, processed in place; the speech and diag events they gave. */
    @Synchronized
    @JvmOverloads
    fun process(frame: FloatArray, rate: Int, channels: Int = 1, n: Int = frame.size / channels): List<Event> {
        val l = stream(rate)
        if (mono.size < n) mono = FloatArray(n)
        for (i in 0 until n) {
            var s = 0.0
            for (c in 0 until channels) s += frame[i * channels + c]
            mono[i] = (s / channels).toFloat()
        }
        val (got, events) = l.pushInto(mono, n)
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
    @JvmOverloads
    fun process(frame: ShortArray, rate: Int, channels: Int = 1, n: Int = frame.size / channels): List<Event> {
        val f = FloatArray(n * channels) { (frame[it] / 32768.0).toFloat() }
        val events = process(f, rate, channels, n)
        toInt16(f).copyInto(frame, 0, 0, n * channels)
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
    override fun close() = reset()

    companion object {
        /** Each frame goes out the size it came in, this far behind: Focus's lag and the resamplers', in seconds. */
        const val LEAD = 0.02
    }
}
