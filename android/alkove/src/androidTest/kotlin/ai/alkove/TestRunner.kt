package ai.alkove

import android.os.Bundle
import androidx.test.runner.AndroidJUnitRunner
import java.io.File

// the fixtures and models out of the test assets, where the host's tests look for them
class TestRunner : AndroidJUnitRunner() {
    override fun onCreate(arguments: Bundle?) {
        val ctx = context
        val root = File(targetContext.cacheDir, "repo")
        fun copy(path: String, to: File) {
            val children = ctx.assets.list(path).orEmpty()
            if (children.isEmpty()) {
                to.parentFile!!.mkdirs()
                ctx.assets.open(path).use { i -> to.outputStream().use { i.copyTo(it) } }
            } else children.forEach { copy("$path/$it", File(to, it)) }
        }
        copy("repo", root)
        copy("models", File(root, "packages/alkove/.cache/models"))
        System.setProperty("alkove.repo", root.path)
        super.onCreate(arguments)
    }
}
