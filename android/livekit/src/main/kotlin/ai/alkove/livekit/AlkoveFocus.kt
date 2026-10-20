package ai.alkove.livekit

import ai.alkove.CallProcessor
import ai.alkove.Event
import ai.alkove.Models
import ai.alkove.SpeechParams
import ai.alkove.Voiceprint
import io.livekit.android.audio.AudioProcessorInterface
import java.nio.ByteBuffer
import java.nio.ByteOrder

/**
 * Focus in LiveKit's microphone path: keeps the user's voice, the closest one or the voice of a print, removes the others and the
 * noise. `room.audioProcessingController.capturePostProcessor = AlkoveFocus(models)`, with Focus and Cue loaded
 * (`Models.load(context, listOf("focus", "cue"))`). Each 10 ms frame comes back about 37 ms behind; `onEvent` gets Cue's speech start and
 * end on WebRTC's audio thread.
 */
class AlkoveFocus @JvmOverloads constructor(
    models: Models,
    voiceprint: Voiceprint? = null,
    strength: Double = 1.0,
    normalize: Boolean = false,
    speech: SpeechParams? = null,
    private val onEvent: ((Event) -> Unit)? = null,
) : AudioProcessorInterface, AutoCloseable {
    private val processor = CallProcessor(models, voiceprint, strength, normalize, speech)
    private var frame = FloatArray(480)
    private var failed = false

    /** Off: the raw microphone at the same delay, with a short fade, so turning it on and off moves nothing in time. */
    @Volatile
    var enabled = true
        set(on) {
            field = on
            processor.setEnabled(on)
        }

    override fun isEnabled() = enabled

    override fun getName() = "alkove-focus"

    override fun initializeAudioProcessing(sampleRateHz: Int, numChannels: Int) = processor.reset()

    override fun resetAudioProcessing(newRate: Int) = processor.reset()

    // channel 0, full band, float at int16 scale: 10 ms at numFrames × 100 Hz, read and written by index whatever the buffer's position
    override fun processAudio(numBands: Int, numFrames: Int, buffer: ByteBuffer) {
        buffer.order(ByteOrder.nativeOrder())
        val n = minOf(numFrames, buffer.capacity() / 4)
        if (frame.size < n) frame = FloatArray(n)
        for (i in 0 until n) frame[i] = buffer.getFloat(4 * i) / 32768f
        // an exception must not reach WebRTC's audio thread: the frame goes through as it came
        val events = try {
            processor.process(frame, numFrames * 100, 1, n)
        } catch (e: Throwable) {
            if (!failed) onEvent?.invoke(Event.Diag("worker.error", "warn", mapOf("message" to (e.message ?: e.toString()), "on" to "processAudio")))
            failed = true
            return
        }
        failed = false
        for (i in 0 until n) buffer.putFloat(4 * i, frame[i] * 32768f)
        onEvent?.let { cb -> events.forEach(cb) }
    }

    /** Applied at the user's next silence; null goes back to the closest voice. */
    fun setVoiceprint(p: Voiceprint?) = processor.setVoiceprint(p)

    /** How much of Focus goes out, 0 to 1, the rest being the raw microphone. */
    fun setStrength(s: Double) = processor.setStrength(s)

    override fun close() = processor.close()
}
