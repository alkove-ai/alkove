package ai.alkove.livekit

import ai.alkove.Event
import ai.alkove.Live
import ai.alkove.Models
import ai.alkove.resample
import java.io.File
import java.nio.ByteBuffer
import java.nio.ByteOrder
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertTrue
import org.junit.Assume.assumeTrue
import org.junit.Test

class AlkoveFocusTest {
    private val repo = File(System.getProperty("alkove.repo") ?: "../..")

    private fun models(): Models {
        val dir = File(repo, "packages/alkove/.cache/models")
        assumeTrue("no models in packages/alkove/.cache/models", File(dir, "alkove-focus-1-preview.alk").exists())
        return Models.load(listOf("focus", "cue"), dir = dir)
    }

    // the fixture's data chunk starts at byte 44
    private fun speech(): FloatArray {
        val b = File(repo, "packages/alkove/test/fixtures/speech.wav").readBytes()
        val s = ByteBuffer.wrap(b, 44, b.size - 44).order(ByteOrder.LITTLE_ENDIAN).asShortBuffer()
        return FloatArray(s.remaining()) { (s.get(it) / 32768.0).toFloat() }
    }

    // LiveKit's 10 ms frames at 48 kHz, float at int16 scale in a direct buffer: what comes back is Live's output, LEAD behind
    @Test
    fun framesComeBackAsLiveGivesThem() {
        val models = models()
        val pcm = resample(speech(), 16000, 48000)
        val events = ArrayList<Event>()
        val focus = AlkoveFocus(models) { events.add(it) }
        focus.initializeAudioProcessing(48000, 1)
        val buf = ByteBuffer.allocateDirect(480 * 4).order(ByteOrder.nativeOrder())
        val out = FloatArray(pcm.size / 480 * 480)
        for (k in 0 until pcm.size / 480) {
            buf.clear()
            for (i in 0 until 480) buf.putFloat(pcm[k * 480 + i] * 32768f)
            focus.processAudio(3, 480, buf)
            buf.rewind()
            for (i in 0 until 480) out[k * 480 + i] = buf.getFloat() / 32768f
        }
        // the resampler rounds by the block it is given: Live is fed the same frames
        val live = Live(models, 48000)
        val expected = FloatArray(out.size)
        var at = 960
        for (k in 0 until pcm.size / 480) {
            val (y, _) = live.push(pcm.copyOfRange(k * 480, k * 480 + 480))
            if (at < out.size) y.copyInto(expected, at, 0, minOf(y.size, out.size - at))
            at += y.size
        }
        assertArrayEquals(expected, out, 0f)
        assertTrue(events.any { it is Event.SpeechStart })
    }
}
