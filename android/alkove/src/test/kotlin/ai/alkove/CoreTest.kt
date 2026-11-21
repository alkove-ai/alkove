package ai.alkove

import ai.alkove.Fixtures.golden
import ai.alkove.Fixtures.rng
import ai.alkove.Fixtures.stream
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class CoreTest {
    private val models by lazy { Fixtures.load("focus", "cue") }
    private val print = Fixtures.f32("enroll.timbre.f32", 192)
    private val voiceprint = Voiceprint(mapOf("source" to "raw", "centers" to listOf(mapOf("embedding" to print, "id" to print, "seconds" to 3.0))))

    private fun chain(name: String, seed: Long, seconds: Int, dense: Boolean, rate: Double, opts: Map<String, Any?>) {
        val o = mapOf("gate" to false, "normalize" to true, "strength" to 1.0, "sleep" to true, "speech" to SpeechParams().value(), "print" to null) + opts
        // no Timbre, as the web's and Python's chains that made these digests
        val c = Native.chainNew(models.ptr("focus"), models.ptr("cue"), 0, Values.encode(o))
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
    fun chainLeftAlone() = chain("defaults, left alone 40 s", 1, 40, false, 0.0, emptyMap())

    @Test
    fun chainSettingsChanged() =
        chain("gate, a print, other speech settings, settings changed often", 2, 40, false, 0.008, mapOf("gate" to true, "print" to print, "speech" to speech(confidence = 0.6, stopSecs = 0.3)))

    @Test
    fun chainHalfStrength() = chain("no normalize, no sleep, half strength", 3, 30, false, 0.002, mapOf("normalize" to false, "sleep" to false, "strength" to 0.5))

    @Test
    fun chainNeverStops() = chain("speech that never stops, 75 s", 4, 75, true, 0.0, mapOf("speech" to speech(stopSecs = 5.0)))

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
            Processor(models, focus = true, voiceprint = voiceprint, strength = 0.7, cue = true, speech = SpeechParams(confidence = 0.4, startSecs = 0.1), normalize = true, trace = true, rate = it)
        }

    @Test
    fun processFocusWithoutSpeech() = process("focus without speech", 7, 16000) { Processor(models, focus = true, cue = false, rate = it) }

    @Test
    fun processCueAlone() = process("cue alone, traced", 8, 16000) { Processor(models, cue = true, speech = SpeechParams(stopSecs = 1.0), trace = true, rate = it) }

    @Test
    fun processNothing() = process("nothing chosen", 9, 16000) { Processor(models, rate = it) }

    @Test
    fun processAt44k() = process("focus and cue, blocks at 44.1 kHz", 10, 44100) { Processor(models, focus = true, cue = true, trace = true, rate = it) }

    // a sample apart from the print, as the lab's benches give it
    private fun sampled(pcm: FloatArray, print: Voiceprint, sample: FloatArray?, cue: Boolean? = null) =
        processWith(models, pcm, 16000, processOptions(false, print, 1.0, cue, null, false, false, 16000, null, "anywhere", sample))

    @Test
    fun aSampleGoesToFocus() {
        val sample = Fixtures.wav("enroll.wav").copyOfRange(0, 4 * 16000)
        val focus = { s: FloatArray? -> processOptions(true, voiceprint, 1.0, true, null, false, false, 16000, null, "anywhere", s).getValue("focus").obj() }
        assertEquals(sample.map(Float::toDouble), focus(sample)["sample"])
        assertFalse("sample" in focus(null))
        val pcm = stream(18, 12)
        val told = sampled(pcm, voiceprint, sample)
        assertEquals(pcm.size, told.audio.size)
        assertFalse(ai.alkove.process(models, pcm, focus = true, voiceprint = voiceprint).audio.contentEquals(told.audio))
        Processor(models, voiceprint = sample).use { p -> assertEquals(pcm.size, p.push(pcm).audio.size + p.end().audio.size) }
    }

    @Test
    fun aVoiceprintRunsFocus() {
        val pcm = stream(21, 6)
        val asked = ai.alkove.process(models, pcm, focus = true, voiceprint = voiceprint)
        assertTrue(asked.audio.contentEquals(ai.alkove.process(models, pcm, voiceprint = voiceprint).audio))
        assertFalse(asked.audio.contentEquals(ai.alkove.process(models, pcm).audio))
        assertTrue(runCatching { ai.alkove.process(models, pcm, voiceprint = print.toList()) }.exceptionOrNull() is IllegalArgumentException)
    }

    private fun onlyMyVoice(mine: Voiceprint, reference: Voiceprint, timbre: Timbre): List<Event> {
        val opts = agentPipelineOptions(AgentOptions(voiceprint = mine, onlyMyVoice = true), mine, mapOf("format" to "f32", "rate" to 16000, "chunkMs" to 32, "audio" to false, "utterance" to false))
        assertEquals(true, opts["onlyMyVoice"])
        val p = Native.pipelineNew(models.ptr("focus"), models.ptr("cue"), 0, models.ptr("timbre"), Values.encode(opts))
        val coach = Native.coachNew(Values.encode(mapOf("reference" to reference.value)))
        try {
            val quiet = FloatArray(32000)
            val enroll = Fixtures.wav("enroll.wav")
            val pcm = quiet + enroll + quiet + enroll + quiet
            val hop = FloatArray(256)
            val out = FloatArray(256)
            val events = ArrayList<Event>()
            for (i in 0 until pcm.size / 256) {
                pcm.copyInto(hop, 0, 256 * i, 256 * i + 256)
                if (Native.pipelineStep(p, hop, Double.POSITIVE_INFINITY, out) == 0) continue
                for (o in Values.decode(Native.pipelineOuts(p)).arr()) {
                    val a = o.arr()
                    if (a[0] == "event") eventOf(a[1].obj())?.let(events::add)
                    if (a[0] == "verify") {
                        val v = a[2] as FloatArray
                        val r = Values.decode(Native.coachVerify(coach, timbre.ptr, v, v.size)).obj()
                        Native.pipelineVerified(p, (a[1] as Number).toInt(), r["same"] as Boolean, (r["score"] as Number).toDouble())
                    }
                }
            }
            return events
        } finally {
            Native.coachFree(coach)
            Native.pipelineFree(p)
        }
    }

    @Test
    fun aRecordingIsItsPrintAndItsSample() {
        val recording = Fixtures.wav("enroll.wav").copyOfRange(0, 4 * 16000)
        val pcm = stream(19, 12)
        val given = ai.alkove.process(models, pcm, focus = true, cue = true, voiceprint = recording)
        val made = Timbre(models).use { t -> sampled(pcm, t.embed(recording), recording, cue = true) }
        assertTrue(given.audio.contentEquals(made.audio))
        assertEquals(made.speech, given.speech)
        assertEquals(made.diag, given.diag)
        assertFalse(ai.alkove.process(models, pcm, focus = true, cue = true, voiceprint = Timbre(models).use { it.embed(recording) }).audio.contentEquals(given.audio))
    }

    @Test
    fun aRecordingAtTheStartOrSetBeforeTheFirstHop() {
        val recording = Fixtures.wav("enroll.wav").copyOfRange(0, 4 * 16000)
        val pcm = stream(20, 15)
        val outs = { c: Chain ->
            val out = FloatArray(256)
            FloatArray(pcm.size / 256 * 256).also { all -> for (i in 0 until pcm.size / 256) c.step(pcm.copyOfRange(256 * i, 256 * i + 256), out).also { out.copyInto(all, 256 * i) } }
        }
        val first = Chain(models, voiceprint = recording).use(outs)
        assertTrue(first.contentEquals(Chain(models).use { it.setVoiceprint(recording); outs(it) }))
        val live = { l: Live -> l.push(pcm).first }
        assertTrue(Live(models, 16000, voiceprint = recording).use(live).contentEquals(Live(models, 16000).use { it.setVoiceprint(recording); live(it) }))
    }

    // an agent learns from its first farEndActive() on: the pipeline already running takes `learn`, its windows go to the coach's learner
    @Test
    fun anAgentLearnsOnceTold() =
        Timbre(models).use { t ->
            val opts = agentPipelineOptions(AgentOptions(), null, mapOf("format" to "f32", "rate" to 16000, "chunkMs" to 32, "audio" to false, "utterance" to false))
            assertEquals(false, opts["learn"])
            val p = Native.pipelineNew(models.ptr("focus"), models.ptr("cue"), 0, models.ptr("timbre"), Values.encode(opts))
            val coach = Native.coachNew(Values.encode(mapOf("reference" to null)))
            try {
                val pcm = FloatArray(16000) + Fixtures.wav("enroll.wav")
                val hop = FloatArray(256)
                val out = FloatArray(256)
                val windows = {
                    val ws = ArrayList<List<Any?>>()
                    for (i in 0 until pcm.size / 256) {
                        pcm.copyInto(hop, 0, 256 * i, 256 * i + 256)
                        if (Native.pipelineStep(p, hop, Double.POSITIVE_INFINITY, out) > 0) ws.addAll(Values.decode(Native.pipelineOuts(p)).arr().map { it.arr() }.filter { it[0] == "window" })
                    }
                    ws
                }
                assertTrue(windows().isEmpty())
                Native.pipelineSet(p, Values.encode(mapOf("learn" to true)))
                val ws = windows()
                assertTrue(ws.isNotEmpty())
                Native.coachLearner(coach, Values.encode(mapOf("start" to null)))
                val coached = ws.flatMap { w -> (w[1] as FloatArray).let { Values.decode(Native.coachWindow(coach, t.ptr, it, it.size, (w[2] as Number).toDouble(), (w[3] as Number).toDouble())).arr() } }
                assertTrue(coached.toString(), coached.any { (it.arr()[1] as? Map<*, *>)?.get("code") == "print.learn" })
            } finally {
                Native.coachFree(coach)
                Native.pipelineFree(p)
            }
        }

    @Test
    fun onlyMyVoiceTellsMinePassesOthersNot() =
        Timbre(models).use { t ->
            val mine = t.embed(Fixtures.wav("enroll.wav"))
            val heard = onlyMyVoice(mine, mine, t)
            assertTrue(heard.any { it is Event.SpeechStart } && heard.any { it is Event.SpeechEnd } && heard.any { it is Event.TurnEnd })
            assertFalse(heard.any { it is Event.Diag && it.code == "speech.other-voice" })
            val refused = onlyMyVoice(mine, t.embed(Fixtures.noise(48000, 11, 0.1)), t)
            assertFalse(refused.any { it is Event.SpeechStart || it is Event.TurnEnd })
            assertTrue(refused.any { it is Event.Diag && it.code == "speech.other-voice" && "voice" in it.data })
        }
}
