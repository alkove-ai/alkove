package ai.alkove

import android.content.Context
import android.content.ContextWrapper
import java.io.File
import java.nio.file.Files
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

// Alkove.configure: where every input, Models.load(context), preload and cached take the models
class ConfigTest {
    private val dir = File(Fixtures.repo, "packages/alkove/.cache/models")
    private val home = Files.createTempDirectory("alkove-config").toFile()
    private val context =
        object : ContextWrapper(null) {
            override fun getApplicationContext(): Context = this

            override fun getNoBackupFilesDir(): File = home
        }

    @After
    fun reset() {
        Alkove.configure()
        home.deleteRecursively()
    }

    private fun code(block: () -> Unit) = (runCatching(block).exceptionOrNull() as AlkoveException).code

    @Test
    fun aFolderGivesEveryModel() {
        Fixtures.load("focus", "cue").close()
        val names = Usage.AGENT.names(false, false)
        assertEquals(listOf("focus", "cue", "timbre"), names)
        assertFalse(Alkove.cached(context, Usage.AGENT))
        Alkove.configure(dir = dir)
        assertTrue(Alkove.cached(context, Usage.AGENT))
        assertEquals(emptyList<String>(), Models.remote(context, names))
        Models.configured(context, names, null).use { assertEquals(names.toSet(), it.names) }
        Alkove.configure(dir = home)
        assertFalse(Alkove.cached(context, Usage.AGENT))
        assertEquals("models.fetch", code { Models.remote(context, names) })
        assertEquals("models.fetch", code { Models.configured(context, names, null) })
    }

    @Test
    fun aModelGivenGoesFirst() {
        Fixtures.load("spot").close()
        val spot = File(dir, Models.MANIFEST["models"].obj()["spot"].obj().str("file"))
        Alkove.configure(models = mapOf("spot" to spot))
        assertTrue(Alkove.cached(context, Usage.WAKE))
        assertFalse(Alkove.cached(context, Usage.WAKE, typedWords = true))
        assertEquals(listOf("lex"), Models.remote(context, Usage.WAKE.names(false, true)))
        assertEquals(listOf("spot", "timbre"), Usage.WAKE.names(true, false))
        Models.configured(context, listOf("spot"), null).use { assertTrue("spot" in it) }
        assertEquals("models[\"spot\"]: a File or a ByteArray", runCatching { Alkove.configure(models = mapOf("spot" to "x")) }.exceptionOrNull()?.message)
        assertEquals("models.hash", code { Alkove.configure(models = mapOf("spot" to ByteArray(4))); Models.configured(context, listOf("spot"), null) })
    }
}
