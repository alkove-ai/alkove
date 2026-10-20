package ai.alkove.example

import ai.alkove.AgentInput
import ai.alkove.AgentOptions
import ai.alkove.Alkove
import ai.alkove.Chain
import ai.alkove.Event
import ai.alkove.Lex
import ai.alkove.Listener
import ai.alkove.Models
import ai.alkove.Wake
import ai.alkove.WakeListener
import ai.alkove.WakeOptions
import ai.alkove.compile
import android.Manifest
import android.app.Activity
import android.content.pm.PackageManager
import android.os.Bundle
import android.util.Log
import android.view.ViewGroup.LayoutParams.MATCH_PARENT
import android.view.ViewGroup.LayoutParams.WRAP_CONTENT
import android.widget.Button
import android.widget.EditText
import android.widget.LinearLayout
import android.widget.ScrollView
import android.widget.TextView
import java.io.File
import java.util.Locale
import kotlin.random.Random
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.MainScope
import kotlinx.coroutines.cancel
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

class MainActivity : Activity() {
    private val scope = MainScope()
    private lateinit var log: TextView
    private val lines = ArrayDeque<String>()
    private var agent: AgentInput? = null
    private var wake: WakeListener? = null
    private var collecting: Job? = null

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        val pad = (12 * resources.displayMetrics.density).toInt()
        val root = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            // drawn edge to edge since targetSdk 35: keep clear of the system bars
            setOnApplyWindowInsetsListener { v, insets ->
                @Suppress("DEPRECATION")
                v.setPadding(pad, pad + insets.systemWindowInsetTop, pad, pad + insets.systemWindowInsetBottom)
                insets
            }
        }
        fun button(text: String, onClick: (Button) -> Unit) = Button(this).apply {
            this.text = text
            setOnClickListener { onClick(this) }
            root.addView(this, LinearLayout.LayoutParams(MATCH_PARENT, WRAP_CONTENT))
        }
        button("Agent: start") { b -> toggleAgent(b) }
        button("Focus: on") { b ->
            val a = agent ?: return@button
            val on = b.text.endsWith("off")
            a.setEnabled(on)
            b.text = if (on) "Focus: on" else "Focus: off"
        }
        button("Record 10 s (raw | Focus)") { recordTen() }
        val word = EditText(this).apply { setText("Camille"); hint = "word, or /phonemes/" }
        val lang = EditText(this).apply { setText("fr"); hint = "fr, en, es, pt, it, ca, ro" }
        root.addView(word)
        root.addView(lang)
        button("Wake word: start") { b -> toggleWake(b, word.text.toString(), lang.text.toString()) }
        button("Bench (native)") { bench() }
        log = TextView(this).apply { textSize = 13f; setTextIsSelectable(true) }
        root.addView(ScrollView(this).apply { addView(log) }, LinearLayout.LayoutParams(MATCH_PARENT, 0, 1f))
        setContentView(root)
        say("Alkove ${Alkove.version}, ${android.os.Build.MODEL}, ${android.os.Build.SUPPORTED_ABIS.first()}")
        if (checkSelfPermission(Manifest.permission.RECORD_AUDIO) != PackageManager.PERMISSION_GRANTED) requestPermissions(arrayOf(Manifest.permission.RECORD_AUDIO), 1)
    }

    private fun say(s: String) {
        Log.i("alkove", s)
        lines.addLast(s)
        while (lines.size > 200) lines.removeFirst()
        log.text = lines.reversed().joinToString("\n")
    }

    private fun f(x: Double, d: Int = 2) = String.format(Locale.ROOT, "%.${d}f", x)

    private fun show(e: Event) {
        when (e) {
            is Event.SpeechStart -> say("speech from ${f(e.start)} s")
            is Event.SpeechEnd -> say("speech ${f(e.start)} to ${f(e.end)} s")
            is Event.ModeChange -> say("mode ${e.mode}${e.reason?.let { " ($it)" } ?: ""}")
            is Event.StatusChange -> say("status ${e.status}")
            is Event.Mic -> say("mic ${e.state}")
            is Event.Utterance -> say("utterance ${f(e.end - e.start)} s, ${e.pcm.size} bytes")
            is Wake -> say("WAKE ${e.word} score ${f(e.score)} (threshold ${f(e.threshold)}) at ${f(e.start)} s${e.voice?.let { ", voice ${f(it)}" } ?: ""}")
            is Event.Diag -> when (e.code) {
                "timing" -> say("timing ${e.data["meanMs"]} ms per 16 ms hop (max ${e.data["maxMs"]}, slow ${e.data["slow"]}, queue ${e.data["queueMs"]} ms, asleep ${e.data["asleep"]}, adpf ${e.data["adpf"]})")
                else -> say("diag ${e.code} ${e.level} ${e.data}")
            }
            else -> {}
        }
    }

    private fun toggleAgent(b: Button) {
        agent?.let {
            it.close()
            agent = null
            b.text = "Agent: start"
            return
        }
        try {
            val a = AgentInput.start(this, AgentOptions(keepRecent = 30.0, stats = true))
            agent = a
            b.text = "Agent: stop"
            collecting = scope.launch { a.events.collect(::show) }
        } catch (e: Exception) {
            say("agent failed: ${e.message}")
        }
    }

    private fun recordTen() {
        val a = agent ?: return say("start the agent first")
        scope.launch {
            say("recording 10 s…")
            try {
                val wav = a.record(10.0)
                val out = File(getExternalFilesDir(null), "alkove-${System.currentTimeMillis()}.wav")
                withContext(Dispatchers.IO) { out.writeBytes(wav) }
                say("saved ${out.path} (adb pull it)")
                a.stats()?.let { say("stats: ${f(it.speechSeconds, 1)} s of speech out of ${f(it.seconds, 1)} s, ${it.noiseRemovedDb} dB removed") }
            } catch (e: Exception) {
                say("record failed: ${e.message}")
            }
        }
    }

    private fun toggleWake(b: Button, text: String, lang: String) {
        wake?.let {
            it.close()
            wake = null
            b.text = "Wake word: start"
            return
        }
        scope.launch {
            try {
                say("compiling \"$text\"…")
                val w = withContext(Dispatchers.Default) {
                    if (text.trim().startsWith("/")) compile(text)
                    else Lex(Alkove.models(this@MainActivity, listOf("lex"))).use { it.compile(text, lang) }
                }
                say("$w, thresholds ${w.thresholds}")
                for (warning in w.warnings) say("warning ${warning.code}: ${warning.message}")
                val l = WakeListener.start(this@MainActivity, WakeOptions(listOf(w), context = "anywhere"))
                wake = l
                b.text = "Wake word: stop"
                launch { l.events.collect(::show) }
            } catch (e: Exception) {
                say("wake failed: ${e.message}")
            }
        }
    }

    private var pacedMs = 0.0

    private fun bench() {
        scope.launch {
            say("bench: loading Focus, Cue, Spot…")
            val r = withContext(Dispatchers.Default) {
                val models = Alkove.models(this@MainActivity, listOf("focus", "cue", "spot"))
                val seconds = 20
                val rng = Random(1)
                val pcm = FloatArray(seconds * Alkove.RATE) { (rng.nextFloat() * 2 - 1) * 0.1f }
                val out = FloatArray(Chain.HOP)
                val hop = FloatArray(Chain.HOP)
                val chain = Chain(models, sleep = false)
                val t0 = System.nanoTime()
                var i = 0
                while (i + Chain.HOP <= pcm.size) {
                    pcm.copyInto(hop, 0, i, i + Chain.HOP)
                    chain.step(hop, out)
                    i += Chain.HOP
                }
                val chainMs = (System.nanoTime() - t0) / 1e6 / seconds
                // the same hops one every 16 ms, as the microphone gives them: the governor keeps the clock low, often ten times slower
                var paced = 0L
                var k = 0
                while (k < 300) {
                    pcm.copyInto(hop, 0, k * Chain.HOP, (k + 1) * Chain.HOP)
                    val s0 = System.nanoTime()
                    chain.step(hop, out)
                    paced += System.nanoTime() - s0
                    Thread.sleep(16)
                    k++
                }
                pacedMs = paced / 1e6 / k
                chain.close()
                val listener = Listener(models, listOf(compile("/kamij/")), context = "anywhere")
                val t1 = System.nanoTime()
                listener.push(pcm)
                val spotMs = (System.nanoTime() - t1) / 1e6 / seconds
                listener.close()
                chainMs to spotMs
            }
            say("bench: Focus + Cue ${f(r.first)} ms per second of audio, Spot ${f(r.second)} ms; one hop every 16 ms: ${f(pacedMs)} ms per hop")
        }
    }

    override fun onDestroy() {
        agent?.close()
        wake?.close()
        scope.cancel()
        super.onDestroy()
    }
}
