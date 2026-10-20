package ai.alkove

import android.content.Context
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.channels.BufferOverflow
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.receiveAsFlow
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

/** A word set on the user's voice, and the voice of its takes as a print when they hold enough of it. */
data class Enrolled(val word: Word, val voiceprint: Voiceprint?)

/**
 * The user says the word a few times: its thresholds come from these takes and from what is heard between them; the takes' voice
 * comes back apart, never in the word. Each take and warning comes on [events]; [await] gives the word.
 */
class WordEnrollment internal constructor(
    context: Context,
    private val text: String,
    private val lang: List<String>,
    private val takes: Int,
    private val baseUrl: String?,
) : AutoCloseable {
    private val app = context.applicationContext
    private val channel = Channel<Event>(256, BufferOverflow.DROP_OLDEST)
    private val result = CompletableDeferred<Enrolled>()
    private val job: Job

    @Volatile
    private var input: EnrollInput? = null

    /** The takes ([Take]) as they are heard, and the warnings as [Event.Diag] (`code` the warning's). */
    val events: Flow<Event> = channel.receiveAsFlow()

    init {
        job = CoroutineScope(Dispatchers.Default).launch {
            try {
                result.complete(run())
            } catch (e: Throwable) {
                result.completeExceptionally(e)
            } finally {
                input?.close()
                channel.close()
            }
        }
    }

    private fun warn(w: Warning) = channel.trySend(Event.Diag(w.code, "warn", w.data + ("message" to w.message)))

    private suspend fun run(): Enrolled {
        val typed = withContext(Dispatchers.IO) {
            if (text.trim().startsWith("/")) compile(text, lang)
            else Lex(Alkove.models(app, listOf("lex"), baseUrl)).use { compile(text, lang, it) }
        }
        typed.warnings.forEach(::warn)
        val i = EnrollInput(app, typed, takes, baseUrl)
        input = i
        i.start()
        var done: Event.Enrolled? = null
        i.events.collect { e ->
            when (e) {
                is Take -> channel.trySend(e)
                is Event.Enrolled -> {
                    done = e
                    i.close()
                }
                else -> {}
            }
        }
        val r = done ?: throw AlkoveException("engine.failed", "the enrollment stopped before its takes")
        if (r.error != null || r.word == null) throw AlkoveException("engine.failed", r.error ?: "enrollment failed")
        r.warnings.forEach(::warn)
        val audio = r.audio
        val voiceprint = if (audio != null && audio.size >= 4000) withContext(Dispatchers.IO) { Timbre(Alkove.models(app, listOf("timbre"), baseUrl)).use { it.embed(audio, "raw") } } else null
        return Enrolled(Word(r.word.value, typed.warnings + r.warnings), voiceprint)
    }

    /** The word once its takes were heard; AlkoveException `engine.failed` if it could not be set. */
    suspend fun await(): Enrolled = result.await()

    /** Stops listening; [await] then fails. */
    override fun close() {
        job.cancel()
        input?.close()
    }
}

object Spot {
    /** An enrollment on the raw microphone, as a wake listener hears it: the word typed in `lang` (phonemes between slashes need none), said `takes` times. */
    @JvmStatic
    @JvmOverloads
    fun enroll(context: Context, text: String, lang: List<String> = emptyList(), takes: Int = 3, baseUrl: String? = null): WordEnrollment =
        WordEnrollment(context, text, lang, takes, baseUrl)
}
