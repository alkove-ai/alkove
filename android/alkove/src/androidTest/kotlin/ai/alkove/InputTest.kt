package ai.alkove

import android.Manifest
import androidx.test.platform.app.InstrumentationRegistry
import java.io.File
import java.util.concurrent.atomic.AtomicLong
import kotlinx.coroutines.launch
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test

// the real-time inputs on the device's microphone: what the host cannot run
class InputTest {
    private val context = InstrumentationRegistry.getInstrumentation().targetContext

    // the test models into the app's cache, where the inputs look first: nothing downloaded
    @Before
    fun setUp() {
        InstrumentationRegistry.getInstrumentation().uiAutomation.grantRuntimePermission(context.packageName, Manifest.permission.RECORD_AUDIO)
        val from = File(System.getProperty("alkove.repo")!!, "packages/alkove/.cache/models")
        for (name in Models.NAMES) {
            val m = Models.MANIFEST["models"].obj()[name].obj()
            val to = File(File(Models.cacheDir(context), m.str("sha256")), m.str("file"))
            to.parentFile!!.mkdirs()
            File(from, m.str("file")).copyTo(to, overwrite = true)
        }
    }

    @Test
    fun agentRunsFocusOnTheMicrophone() = runBlocking {
        val bytes = AtomicLong()
        val input = AgentInput.start(context, AgentOptions(output = Output(Format.S16, 24000), stats = true)) { bytes.addAndGet(it.size.toLong()) }
        val events = ArrayList<Event>()
        val job = launch { input.events.collect { events.add(it) } }
        val t0 = System.nanoTime()
        withTimeout(10_000) { while (events.none { it is Event.Diag && it.code == "timing" }) kotlinx.coroutines.delay(100) }
        val seconds = (System.nanoTime() - t0) / 1e9
        val stats = input.stats()
        input.close()
        job.join()
        // raw while loading is the mode it starts in, as on the web: no event for it
        assertEquals(events.toString(), listOf(Event.StatusChange(Status.RUNNING), Event.ModeChange(Mode.FOCUS)), events.filter { it is Event.StatusChange || it is Event.ModeChange || it is Event.Mic }.take(2))
        assertEquals(Event.StatusChange(Status.CLOSED), events.last())
        // int16 at 24 kHz for as long as it ran, give or take the start and a chunk
        val expected = seconds * 24000 * 2
        assertTrue("$bytes bytes in $seconds s", bytes.get() > 0.7 * expected && bytes.get() < 1.1 * expected)
        assertTrue("$stats", stats != null && stats.seconds > 1.5)
        val timing = events.first { it is Event.Diag && it.code == "timing" } as Event.Diag
        println("timing on this device: $timing")
    }

    @Test
    fun listenAndHoldAskedBeforeTheModelsHoldOnceTheyRun() = runBlocking {
        val input = AgentInput.start(context, AgentOptions(levels = true))
        input.listen(false)
        input.hold()
        val events = ArrayList<Event>()
        val job = launch { input.events.collect { events.add(it) } }
        withTimeout(10_000) { while (events.none { it is Event.ModeChange && it.mode == Mode.FOCUS }) kotlinx.coroutines.delay(100) }
        kotlinx.coroutines.delay(1000)
        input.release()
        withTimeout(5_000) { while (events.none { it is Event.Utterance }) kotlinx.coroutines.delay(100) }
        input.close()
        job.join()
        val speech = events.filter { it is Event.SpeechStart || it is Event.SpeechEnd || it is Event.SpeechCancel }
        assertEquals(events.toString(), 2, speech.size)
        val end = speech[1] as Event.SpeechEnd
        assertEquals((speech[0] as Event.SpeechStart).start, end.start, 0.0)
        assertTrue("$end", end.end - end.start > 0.9 && end.level != null)
        val utterance = events.first { it is Event.Utterance } as Event.Utterance
        assertEquals(end.level, utterance.level)
    }

    @Test
    fun wakeListenerStartsAndCloses() = runBlocking {
        val word = compile("/kamij/")
        val listener = WakeListener.start(context, WakeOptions(listOf(word), context = "anywhere"))
        val events = ArrayList<Event>()
        val job = launch { listener.events.collect { events.add(it) } }
        withTimeout(10_000) { while (events.none { it is Event.ModeChange && it.mode == Mode.SPOT }) kotlinx.coroutines.delay(100) }
        listener.close()
        job.join()
        assertEquals(Event.StatusChange(Status.CLOSED), events.last())
    }
}

// a phone with no native library for its ABI: the agent still sends the raw microphone, in its format
class UnsupportedTest {
    @Test
    fun agentSendsTheRawMicrophone() = runBlocking {
        val context = InstrumentationRegistry.getInstrumentation().targetContext
        InstrumentationRegistry.getInstrumentation().uiAutomation.grantRuntimePermission(context.packageName, Manifest.permission.RECORD_AUDIO)
        Alkove.supportedOverride = false
        try {
            val bytes = AtomicLong()
            val input = AgentInput.start(context, AgentOptions(output = Output(Format.S16, 24000))) { bytes.addAndGet(it.size.toLong()) }
            val events = ArrayList<Event>()
            val job = launch { input.events.collect { events.add(it) } }
            val t0 = System.nanoTime()
            kotlinx.coroutines.delay(2000)
            val seconds = (System.nanoTime() - t0) / 1e9
            input.close()
            job.join()
            assertTrue(events.toString(), Event.ModeChange(Mode.RAW, RawReason.UNSUPPORTED) in events)
            assertTrue(events.toString(), events.any { it is Event.Diag && it.code == "engine.unsupported" })
            val expected = seconds * 24000 * 2
            assertTrue("$bytes bytes in $seconds s", bytes.get() > 0.7 * expected && bytes.get() < 1.1 * expected)
        } finally {
            Alkove.supportedOverride = null
        }
    }
}
