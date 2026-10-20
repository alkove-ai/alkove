package ai.alkove

import ai.alkove.Fixtures.golden
import ai.alkove.Fixtures.rng
import ai.alkove.Fixtures.stream
import org.junit.Assert.assertEquals
import org.junit.Test

class CoreTest {
    private val models by lazy { Fixtures.load("focus", "cue") }
    private val print = Fixtures.f32("enroll.timbre.f32", 192)

    private fun chain(name: String, seed: Long, seconds: Int, dense: Boolean, rate: Double, opts: Map<String, Any?>) {
        val o = mapOf("gate" to false, "normalize" to true, "strength" to 1.0, "sleep" to true, "speech" to SpeechParams().value(), "print" to null) + opts
        val c = Native.chainNew(models.ptr("focus"), models.ptr("cue"), Values.encode(o))
        try {
            val pcm = stream(seed, seconds, dense)
            val r = rng(seed * 31)
            val d = Digest()
            val hop = FloatArray(256)
            val out = FloatArray(256)
            val focus = FloatArray(256)
            val events = DoubleArray(64)
            var i = 0
            while (i + 256 <= pcm.size) {
                val v = r()
                val x = if (rate != 0.0) v / rate else Double.POSITIVE_INFINITY
                when {
                    x < 0.375 ->
                        if (r() < 0.5) Native.chainSetPrint(c, FloatArray(192) { j -> (print[j] * (1 + 0.1 * StrictMath.sin((i + j).toDouble()))).toFloat() })
                        else Native.chainSetPrint(c, null)
                    x < 0.75 -> Native.chainSetStrength(c, r() * 1.4 - 0.2)
                    x < 1 -> Native.chainSetEnabled(c, r() < 0.5)
                }
                pcm.copyInto(hop, 0, i, i + 256)
                val n = Native.chainStep(c, hop, out, events, focus)
                d.f32(out)
                for (k in 0 until n) {
                    d.f64(events[4 * k], events[4 * k + 1], events[4 * k + 2], events[4 * k + 3])
                    if (events[4 * k] == 5.0) d.f32(focus)
                }
                d.f64(Native.chainGet(c, 0), Native.chainGet(c, 1), Native.chainGet(c, 2), Native.chainGet(c, 3), Native.chainGet(c, 4))
                i += 256
            }
            assertEquals(name, golden["chain: $name"], d.hex())
        } finally {
            Native.chainFree(c)
        }
    }

    private fun speech(confidence: Double = 0.5, stopSecs: Double = 0.6) = SpeechParams(confidence = confidence, stopSecs = stopSecs).value()

    @Test
    fun chainLeftAlone() = chain("defaults, left alone past the 30 s reset", 1, 40, false, 0.0, emptyMap())

    @Test
    fun chainSettingsChanged() =
        chain("gate, a print, other speech settings, settings changed often", 2, 40, false, 0.008, mapOf("gate" to true, "print" to print, "speech" to speech(confidence = 0.6, stopSecs = 0.3)))

    @Test
    fun chainHalfStrength() = chain("no normalize, no sleep, half strength", 3, 30, false, 0.002, mapOf("normalize" to false, "sleep" to false, "strength" to 0.5))

    @Test
    fun chainNeverStops() = chain("speech that never stops, past the forced reset", 4, 75, true, 0.0, mapOf("speech" to speech(stopSecs = 5.0)))

    private val why = listOf("silence", "forced", "wake")

    private fun numbers(d: Digest, p: Part) {
        d.f64(p.start, p.audio.size.toDouble())
        for (x in p.audio) d.f64(x.toDouble())
        d.f64(p.speech.size.toDouble())
        for (s in p.speech) d.f64(s.start, s.end)
        d.f64(p.diag.size.toDouble())
        for (e in p.diag) {
            val f = e.data
            fun n(k: String) = (f[k] as Number).toDouble()
            fun b(k: String) = if (f[k] == true) 1.0 else 0.0
            when (e.code) {
                "idle" -> d.f64(2.0, b("asleep"), n("at"), 0.0)
                "focus.print" -> d.f64(3.0, b("print"), n("after"), why.indexOf(f["why"]).toDouble())
                else -> d.f64(4.0, n("after"), why.indexOf(f["why"]).toDouble(), 0.0)
            }
        }
        val t = p.trace
        if (t == null) d.f64(0.0)
        else {
            val focus = t["focus"].arr()
            d.f64(1.0, focus.size.toDouble())
            for (row in focus) row.obj().let { d.f64(it.num("t"), it.num("db")) }
            val cue = t["cue"].arr()
            d.f64(cue.size.toDouble())
            for (row in cue) row.obj().let { d.f64(it.num("t"), it.num("speech")) }
        }
    }

    private fun Map<String, Any?>.num(k: String) = (this[k] as Number).toDouble()

    private fun process(name: String, seed: Long, rate: Int, make: (Int) -> Processor) {
        var pcm = stream(seed, 25)
        if (rate != 16000) pcm = resample(pcm, 16000, rate)
        make(rate).use { p ->
            Resampler(rate, 16000, aligned = true).use { counter ->
                val r = rng(seed * 17)
                val d = Digest()
                var i = 0
                while (i < pcm.size) {
                    val n = minOf(pcm.size - i, 1 + Math.floor(r() * 5000).toInt())
                    val block = pcm.copyOfRange(i, i + n)
                    val part = p.push(block)
                    if (counter.push(block).isNotEmpty()) numbers(d, part)
                    i += n
                }
                numbers(d, p.end())
                assertEquals(name, golden["process: $name"], d.hex())
            }
        }
    }

    @Test
    fun processFocusAndCue() = process("focus and cue", 5, 16000) { Processor(models, focus = true, cue = true, rate = it) }

    @Test
    fun processWithPrint() =
        process("focus with a print, normalize, trace, other speech settings", 6, 16000) {
            Processor(models, focus = true, voiceprint = print, strength = 0.7, cue = true, speech = SpeechParams(confidence = 0.4, startSecs = 0.1), normalize = true, trace = true, rate = it)
        }

    @Test
    fun processFocusWithoutSpeech() = process("focus without speech", 7, 16000) { Processor(models, focus = true, cue = false, rate = it) }

    @Test
    fun processCueAlone() = process("cue alone, traced", 8, 16000) { Processor(models, cue = true, speech = SpeechParams(stopSecs = 1.0), trace = true, rate = it) }

    @Test
    fun processNothing() = process("nothing chosen", 9, 16000) { Processor(models, rate = it) }

    @Test
    fun processAt44k() = process("focus and cue, blocks at 44.1 kHz", 10, 44100) { Processor(models, focus = true, cue = true, trace = true, rate = it) }
}
