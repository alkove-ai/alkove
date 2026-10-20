package ai.alkove

import ai.alkove.Fixtures.digest
import ai.alkove.Fixtures.noise
import ai.alkove.Fixtures.rng
import ai.alkove.Fixtures.wav
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

// the Spot and Lex cases of packages/alkove/test/spot-core.test.ts, drawn the same way
class SpotTest {
    private val models by lazy { Fixtures.load("focus", "cue", "spot", "lex") }
    private val lex by lazy { Lex(models) }

    @Test
    fun numbersAsJavaScriptWritesThem() {
        for ((x, s) in listOf(0.1 to "0.1", 1e-7 to "1e-7", 1e21 to "1e+21", 123.0 to "123", -0.00001 to "-0.00001", 1.5e-6 to "0.0000015", Math.pow(2.0, 70.0) to "1.1805916207174113e+21", 0.30000000000000004 to "0.30000000000000004")) {
            assertEquals(s, Json.stringify(x))
        }
    }

    private val words =
        mapOf(
            "fr" to listOf("amendement", "composition", "Haut conseil", "aujourd'hui", "porte-monnaie", "Camille", "Ŀluís", "inégalités territoriales", "santé", "œuvre", "Noël", "ça", "M. Dupont", "ONU", "deuxième", "ok luciole"),
            "en" to listOf("hey jarvis", "computer", "knight", "through", "don't", "well-being", "Ok Nabu", "colonel", "NASA", "café"),
            "es" to listOf("oye", "mañana", "ñandú", "güero", "cigüeña", "hola amigo"),
            "it" to listOf("ciao", "gnocchi", "perché", "sciarpa", "città"),
            "pt" to listOf("olá", "pão", "coração", "lhama", "nação"),
            "ca" to listOf("col·legi", "paral·lel", "Llorenç", "aigua"),
            "ro" to listOf("bună", "știință", "țară", "mulțumesc"),
        )

    @Test
    fun phonemizeInTheSevenLanguages() =
        digest("lex: phonemize in the seven languages", words.map { (lang, ws) -> Values.decode(Native.lexPhonemize(lex.ptr, Values.encode(mapOf("texts" to ws, "lang" to lang)))) })

    private fun stream(seed: Long, seconds: Int): FloatArray {
        val r = rng(seed)
        val voices = Fixtures.voices
        val out = FloatArray(seconds * 16000)
        var o = 0
        while (o < out.size) {
            val kind = r()
            val n = minOf(out.size - o, Math.floor((0.3 + r() * 4) * 16000).toInt())
            when {
                kind < 0.6 -> {
                    val v = voices[Math.floor(r() * voices.size).toInt()]
                    val start = if (r() < 0.5) 0 else Math.floor(r() * v.size).toInt()
                    val gain = 0.2 + r() * 1.5
                    for (k in 0 until n) out[o + k] = (v[(start + k) % v.size] * gain).toFloat()
                }
                kind < 0.85 -> noise(n, seed + o, 1e-5).copyInto(out, o)
                else -> noise(n, seed + o, 0.002 + r() * 0.02).copyInto(out, o)
            }
            o += n
        }
        return out
    }

    private fun words(): List<Word> =
        listOf("amendement" to listOf("fr"), "cet amendement" to listOf("fr"), "le dernier constat" to listOf("fr"), "composition" to listOf("fr"), "ministre" to listOf("fr", "en"), "territoriales" to listOf("fr"), "/kɔ̃sɛj/" to listOf("fr"), "hey jarvis" to listOf("en"))
            .map { (t, l) -> compile(t, l, lex) }

    private fun part(bytes: ByteArray): Map<String, Any?> {
        val p = Values.decode(bytes).obj()
        val out = linkedMapOf("start" to p["start"], "audio" to p["audio"], "speech" to p["speech"], "wakes" to p["wakes"], "diag" to p["diag"])
        if (p["trace"] != null) out["trace"] = p["trace"]
        return out
    }

    private fun spot(name: String, seed: Long, context: String, opts: Map<String, Any?>) {
        val o = processOptions(opts["focus"] == true, null, 1.0, opts["cue"] as Boolean?, null, opts["normalize"] == true, opts["trace"] == true, 16000, words(), context)
        val p = Native.processorNew(models.ptr("focus"), models.ptr("cue"), models.ptr("spot"), Values.encode(o))
        try {
            val pcm = stream(seed, 40)
            val r = rng(seed * 7)
            val parts = ArrayList<Map<String, Any?>>()
            var i = 0
            while (i < pcm.size) {
                val n = minOf(pcm.size - i, 1 + Math.floor(r() * 6000).toInt())
                parts.add(part(Native.processorPush(p, pcm.copyOfRange(i, i + n), n)))
                i += n
            }
            parts.add(part(Native.processorEnd(p)))
            assertTrue(parts.sumOf { it["wakes"].arr().size } > 0)
            digest("spot: process, $name", parts)
        } finally {
            Native.processorFree(p)
        }
    }

    @Test
    fun rawAnywhere() = spot("raw, anywhere, traced", 11, "anywhere", mapOf("cue" to true, "trace" to true))

    @Test
    fun rawPause() = spot("raw, after a pause", 12, "pause", emptyMap())

    @Test
    fun behindFocusBoth() = spot("behind Focus, both, traced", 13, "both", mapOf("focus" to true, "trace" to true))

    @Test
    fun behindFocusWithCue() = spot("behind Focus with Cue, pause, normalize", 14, "pause", mapOf("focus" to true, "cue" to true, "normalize" to true))

    @Test
    fun listeningFrameByFrame() {
        val ws = words()
        val pcm = stream(15, 40)
        val args = { some: List<Word>, context: String -> Values.encode(mapOf("words" to some.map { it.value }, "context" to context)) }
        val l = Native.listenerNew(models.ptr("spot"), args(ws, "both"))
        try {
            val r = rng(16)
            val frames = ArrayList<Any?>()
            var i = 0
            while (i < pcm.size) {
                val n = minOf(pcm.size - i, 256 * (1 + Math.floor(r() * 8).toInt()))
                if (i > 0 && r() < 0.01) {
                    val some = ws.subList(0, 1 + Math.floor(r() * ws.size).toInt())
                    val context = listOf("pause", "anywhere", "both")[Math.floor(r() * 3).toInt()]
                    Native.listenerWords(l, args(some, context))
                }
                frames.add(Values.decode(Native.listenerPush(l, pcm.copyOfRange(i, i + n), n)))
                i += n
            }
            digest("spot: listening frame by frame", frames)
        } finally {
            Native.listenerFree(l)
        }
    }

    @Test
    fun enrolling() {
        val typed = compile("amendement", listOf("fr"), lex)
        val speech = wav("speech.wav")
        val quiet = FloatArray(16000)
        val parts = listOf(speech, quiet, stream(17, 6), speech, quiet, speech, quiet, speech)
        val out = ArrayList<Any?>()
        for (takes in listOf(3, 2, 9)) {
            val e = Native.enrollerNew(models.ptr("spot"), Values.encode(mapOf("word" to typed.value, "takes" to takes, "chain" to "focus")))
            try {
                for (p in parts) out.add(Values.decode(Native.enrollerPush(e, p, p.size)))
                try {
                    out.add(Values.decode(Native.enrollerResult(e)))
                } catch (err: IllegalArgumentException) {
                    out.add(mapOf("error" to "RangeError: ${err.message}"))
                }
            } finally {
                Native.enrollerFree(e)
            }
        }
        digest("spot: enrolling", out)
        Enroller(models, typed).use { e ->
            e.push(FloatArray(32000))
            val err = runCatching { e.result() }.exceptionOrNull()
            assertTrue(err is IllegalArgumentException && "word.no-take" in err.message!!)
        }
    }

    @Test
    fun theApi() {
        val w = lex.compile("Camille", "fr")
        assertTrue(w.text == "Camille" && w.phonemes[0].ipa.isNotEmpty() && w.thresholds.getValue("pause") < w.thresholds.getValue("anywhere"))
        assertEquals(w.toJson(), Word.fromJson(w.toJson())!!.toJson())
        assertNull(Word.fromJson(Json.stringify(w.value + ("spot" to "alkove-spot-1-preview"))))
        val outdated = runCatching { Processor(models, words = listOf(Json.stringify(w.value + ("spot" to "old")))) }.exceptionOrNull()
        assertEquals("word.outdated", (outdated as AlkoveException).code)
        assertEquals("kamij", compile("/kamij/").phonemes[0].ipa)
        assertEquals("lang.unsupported", (runCatching { lex.phonemize("x", "de") }.exceptionOrNull() as AlkoveException).code)
        assertEquals("amɑ̃dmɑ̃", lex.phonemize("amendement", "fr")[0].ipa)
        assertTrue(Lex.distance("/amɑ̃dmɑ̃/", "amɑ̃dəmɑ̃") > 0)
        assertEquals(58, Lex.inventory.size)
        assertEquals(listOf("fr", "en", "es", "pt", "it", "ca", "ro"), Lex.langs)
        val r = process(models, wav("speech.wav"), cue = true, words = listOf(lex.compile("amendement", "fr")))
        assertEquals(listOf("amendement"), r.wakes.map { it.word })
        Listener(models, listOf(w), context = "anywhere").use { listener ->
            val heard = listener.push(wav("speech.wav"))
            assertTrue(heard[0].speech == null && heard[1].speech != null && heard[5].scores[0].first == "Camille")
        }
    }
}
