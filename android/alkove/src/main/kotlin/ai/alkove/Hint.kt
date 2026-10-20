package ai.alkove

import android.content.Context
import android.os.Build
import android.os.PerformanceHintManager
import android.os.Process

// the only way to ask Android for frequency or a bigger core: each hop's work reported against a target
internal class Hint private constructor(private val session: PerformanceHintManager.Session) {
    fun report(nanos: Long) = session.reportActualWorkDuration(nanos)

    fun close() = session.close()

    companion object {
        // on the thread doing the work; null where the device offers no session
        fun create(context: Context, targetNanos: Long): Hint? {
            if (Build.VERSION.SDK_INT < 31) return null
            val m = context.getSystemService(PerformanceHintManager::class.java) ?: return null
            return m.createHintSession(intArrayOf(Process.myTid()), targetNanos)?.let(::Hint)
        }
    }
}
