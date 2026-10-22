package ai.alkove

import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

// speech.wav loud twice, 20 dB under, then loud: test/near.test.ts in the web SDK and test_api.py in Python, the same numbers
class NearTest {
    private val models by lazy { Fixtures.load("focus", "cue") }

    private fun silence(seconds: Double) = FloatArray(Math.round(seconds * 16000).toInt())

    private fun concat(vararg parts: FloatArray): FloatArray {
        val out = FloatArray(parts.sumOf { it.size })
        var o = 0
        for (p in parts) {
            p.copyInto(out, o)
            o += p.size
        }
        return out
    }

    private val audio by lazy {
        val x = Fixtures.wav("speech.wav")
        val gap = silence(1.5)
        concat(silence(1.0), x, gap, x, gap, FloatArray(x.size) { (x[it] * 0.1f) }, gap, x, gap)
    }

    private fun run(chain: Chain, pcm: FloatArray, at: Map<Int, (Chain) -> Unit> = emptyMap()): List<Event> {
        val out = FloatArray(Chain.HOP)
        val events = ArrayList<Event>()
        var k = 0
        while ((k + 1) * Chain.HOP <= pcm.size) {
            at[k]?.invoke(chain)
            events += chain.step(pcm.copyOfRange(k * Chain.HOP, (k + 1) * Chain.HOP), out)
            k++
        }
        chain.close()
        return events
    }

    @Test
    fun levelsAndNearGiveTheWebsEvents() {
        val ends = run(Chain(models, levels = true), audio).filterIsInstance<Event.SpeechEnd>()
        assertEquals(listOf(-19.2, -18.3, -40.2, -18.4), ends.map { it.level!!.voiceDb })
        assertEquals(listOf(null, null, -18.7, -18.7), ends.map { it.level!!.refDb })
        val near = run(Chain(models, near = Alkove.NEAR_GAP_DB), audio)
        val far = near.filterIsInstance<Event.Diag>().filter { it.code == "speech.far" }.map { it.data }
        assertEquals(listOf(Triple(16.814, -40.2, -18.7)), far.map { Triple(it["start"], it["voiceDb"], it["refDb"]) })
        assertEquals(listOf(2.264, 9.524, 24.074), near.filterIsInstance<Event.SpeechStart>().map { it.start })
    }

    @Test
    fun listenAndHoldMakeThePhrases() {
        val pcm = concat(silence(0.5), Fixtures.wav("mix.wav"), silence(1.0))
        val events = run(Chain(models), pcm, mapOf(10 to { c -> c.hold() }, 20 to { c -> c.release() }, 160 to { c -> c.setListen(false) }))
        val speech = events.filter { it is Event.SpeechStart || it is Event.SpeechEnd || it is Event.SpeechCancel }
        assertEquals(Event.SpeechStart(10 * 256 / 16000.0), speech[0])
        assertEquals(Event.SpeechEnd(10 * 256 / 16000.0, 20 * 256 / 16000.0), speech[1])
        assertEquals(listOf("SpeechStart", "SpeechCancel"), speech.drop(2).map { it::class.simpleName })
    }

    @Test
    fun aHoldInASilenceHasNoVoiceLevel() {
        val events = run(Chain(models, levels = true), silence(1.0), mapOf(5 to { c -> c.hold() }, 20 to { c -> c.release() }))
        val end = events.filterIsInstance<Event.SpeechEnd>().single()
        assertEquals(null, end.level!!.voiceDb)
        assertEquals(null, end.level!!.refDb)
    }

    @Test
    fun aQuietMicGetsAnInputGainAndANormalOneNone() {
        val mix = Fixtures.wav("mix.wav")
        val pcm = concat(mix, mix, mix, mix)
        for ((gain, expect) in listOf(1.0 to false, Math.pow(10.0, -40 / 20.0) to true)) {
            val live = Live(models, 16000)
            live.push(FloatArray(pcm.size) { (pcm[it] * gain).toFloat() })
            assertEquals(expect, live.inputGainDb > 0)
            assertTrue(live.inputGainDb in 0.0..24.0)
            live.close()
        }
    }
}
