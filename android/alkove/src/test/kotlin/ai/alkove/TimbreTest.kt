package ai.alkove

import ai.alkove.Fixtures.digest
import ai.alkove.Fixtures.noise
import ai.alkove.Fixtures.rng
import ai.alkove.Fixtures.wav
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

// the Timbre cases of packages/alkove/test/timbre-core.test.ts, drawn the same way
class TimbreTest {
    private val models by lazy { Fixtures.load("timbre") }
    private val timbre by lazy { Timbre(models) }

    private fun clips() = listOf(wav("speech.wav"), wav("mix.wav"), wav("enroll.wav"))

    private fun windows(pcm: FloatArray) = Values.decode(Native.timbreWindows(timbre.ptr, pcm, pcm.size))

    private fun embed(pcm: FloatArray, source: String) = Values.decode(Native.timbreEmbed(timbre.ptr, pcm, pcm.size, Values.encode(mapOf("source" to source))))

    private fun learner(source: String, start: Any? = null) = Native.learnerNew(Values.encode(mapOf("source" to source, "start" to start)))

    // the digests were taken on what Python's learner gives, without the window count the native one adds
    private fun state(v: Any?) = v.obj() - "windows"

    private fun learn(l: Long, pcm: FloatArray) = state(Values.decode(Native.learnerLearn(l, timbre.ptr, pcm, pcm.size)))

    @Test
    fun windowsAndPrints() {
        val r = rng(1)
        val out = ArrayList<Any?>()
        for (c in clips()) {
            for (k in 0 until 6) {
                val start = Math.floor(r() * c.size * 0.5).toInt()
                val n = listOf(280, 4000, 24000, 24001, 36000, c.size - start)[k]
                val pcm = c.copyOfRange(start, minOf(c.size, start + n))
                out.add(windows(pcm))
                for (source in listOf("raw", "focus")) out.add(embed(pcm, source))
            }
        }
        digest("timbre: windows and prints", out)
    }

    @Test
    fun learningWindowByWindow() {
        val r = rng(3)
        val cs = clips()
        val out = ArrayList<Any?>()
        for (start in listOf(null, "embedded", "learned")) {
            val prior = learner("focus")
            var i = 0
            while (i < cs[2].size - 23999) {
                learn(prior, cs[2].copyOfRange(i, i + 24000))
                i += 12000
            }
            val first = when (start) {
                "embedded" -> embed(cs[2], "focus")
                "learned" -> Values.decode(Native.learnerVoiceprint(prior, false))
                else -> null
            }
            Native.learnerFree(prior)
            val l = learner("focus", first)
            for (k in 0 until 60) {
                val x = r()
                val c = if (x < 0.7) cs[2] else if (x < 0.9) cs[Math.floor(r() * 2).toInt()] else noise(30000, k.toLong(), 0.05)
                val n = 20000 + Math.floor(r() * 4000).toInt()
                val at = Math.floor(r() * (c.size - n)).toInt()
                out.add(learn(l, c.copyOfRange(at, at + n)))
                out.add(Values.decode(Native.learnerVoiceprint(l, false)))
            }
            Native.learnerFree(l)
        }
        digest("timbre: learning window by window", out)
    }

    @Test
    fun learningOfferedSpeech() {
        val c = wav("enroll.wav")
        val l = learner("raw")
        val out = ArrayList<Any?>()
        for (pcm in listOf(c, FloatArray(30000), c.copyOfRange(5000, c.size), c.copyOfRange(0, 21000), c.copyOfRange(0, 21700))) {
            out.add(state(Values.decode(Native.learnerOffer(l, timbre.ptr, pcm, pcm.size))))
            out.add(Values.decode(Native.learnerVoiceprint(l, false)))
            out.add(Values.decode(Native.learnerVoiceprint(l, true)))
        }
        Native.learnerFree(l)
        digest("timbre: learning offered speech", out)
    }

    // a wake word and its 0.5 s past one Timbre window: checked on its windows instead of refused
    @Test
    fun aLongWakeIsVerified() {
        val me = timbre.embed(wav("enroll.wav"))
        val coach = Native.coachNew(Values.encode(mapOf("reference" to me.value)))
        try {
            val pcm = wav("speech.wav").copyOfRange(0, 40000)
            val r = Values.decode(Native.coachVerify(coach, timbre.ptr, pcm, pcm.size)).obj()
            assertTrue(r.toString(), r["same"] is Boolean && (r["score"] as Number).toDouble() > 0)
        } finally {
            Native.coachFree(coach)
        }
    }

    @Test
    fun theApi() {
        val me = timbre.embed(wav("enroll.wav"))
        val (same, score) = Timbre.verify(me, timbre.embed(wav("speech.wav").copyOfRange(8000, 72000)))
        val other = Timbre.verify(me, timbre.embed(wav("mix.wav"))).second
        assertTrue(same && score > other)
        assertEquals("me", Timbre.identify(timbre.embed(wav("speech.wav")), mapOf("me" to me, "nobody" to timbre.embed(noise(30000, 3, 0.1))))!!.first)
        assertEquals(me.toJson(), Voiceprint.fromJson(me.toJson())!!.toJson())
        assertNull(Voiceprint.fromJson(Json.stringify(me.value + ("model" to "alkove-timbre-1-preview"))))
        assertEquals(192, me.embedding.size)
        assertEquals(2, Voiceprint.merge(listOf(me, me)).value["centers"].arr().size)
        VoiceprintLearner(models).use { learner ->
            learner.offer(wav("enroll.wav"))
            val r = learner.offer(wav("speech.wav"))
            assertTrue(r.accepted >= 1 && learner.ready && learner.voiceprint != null)
            learner.forget()
            assertTrue(learner.progress == 0.0 && learner.voiceprint == null)
        }
    }
}
