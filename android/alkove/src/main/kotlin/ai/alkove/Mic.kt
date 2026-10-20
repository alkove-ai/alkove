package ai.alkove

import android.Manifest
import android.annotation.SuppressLint
import android.content.Context
import android.content.pm.PackageManager
import android.media.AudioDeviceInfo
import android.media.AudioFormat
import android.media.AudioManager
import android.media.AudioRecord
import android.media.AudioRecordingConfiguration
import android.media.AudioTimestamp
import android.media.MediaRecorder
import android.media.audiofx.AcousticEchoCanceler
import android.media.audiofx.AudioEffect
import android.media.audiofx.AutomaticGainControl
import android.media.audiofx.NoiseSuppressor
import android.os.Build
import android.os.Process

// the microphone at 16 kHz, mono, float: anti-echo when asked, nothing else of the platform's in front of the models
internal class Mic(private val context: Context, private val echoCancellation: Boolean, @Volatile var device: AudioDeviceInfo?, private val rate: Int = RATE) {
    private var record: AudioRecord? = null
    private val effects = ArrayList<AudioEffect>()
    private val ts = AudioTimestamp()
    private var callback: Any? = null
    private var base = Double.MAX_VALUE

    // frames read since open
    var read = 0L
        private set

    @SuppressLint("MissingPermission")
    fun open(onSilenced: (Boolean) -> Unit) {
        if (context.checkSelfPermission(Manifest.permission.RECORD_AUDIO) != PackageManager.PERMISSION_GRANTED) {
            throw AlkoveException("mic.denied", "RECORD_AUDIO is not granted: ask for it before opening an input.")
        }
        val source = if (echoCancellation) MediaRecorder.AudioSource.VOICE_COMMUNICATION else MediaRecorder.AudioSource.VOICE_RECOGNITION
        val min = AudioRecord.getMinBufferSize(rate, AudioFormat.CHANNEL_IN_MONO, AudioFormat.ENCODING_PCM_FLOAT)
        if (min <= 0) throw AlkoveException("mic.not-found", "No microphone takes $rate Hz mono float here ($min).")
        // two seconds of room: a slow hop delays the audio instead of losing it, until the overload rule goes raw
        val r = AudioRecord(source, rate, AudioFormat.CHANNEL_IN_MONO, AudioFormat.ENCODING_PCM_FLOAT, maxOf(min, 2 * rate * 4))
        if (r.state != AudioRecord.STATE_INITIALIZED) {
            r.release()
            throw AlkoveException("mic.busy", "The microphone could not be opened: another app or a call may hold it.")
        }
        device?.let { r.preferredDevice = it }
        val id = r.audioSessionId
        if (echoCancellation && AcousticEchoCanceler.isAvailable()) AcousticEchoCanceler.create(id)?.let { it.enabled = true; effects.add(it) }
        if (NoiseSuppressor.isAvailable()) NoiseSuppressor.create(id)?.let { it.enabled = false; effects.add(it) }
        if (AutomaticGainControl.isAvailable()) AutomaticGainControl.create(id)?.let { it.enabled = false; effects.add(it) }
        if (Build.VERSION.SDK_INT >= 29) {
            val cb = object : AudioManager.AudioRecordingCallback() {
                override fun onRecordingConfigChanged(configs: MutableList<AudioRecordingConfiguration>) {
                    configs.firstOrNull { it.clientAudioSessionId == id }?.let { onSilenced(it.isClientSilenced) }
                }
            }
            r.registerAudioRecordingCallback(context.mainExecutor, cb)
            callback = cb
        }
        try {
            r.startRecording()
        } catch (e: IllegalStateException) {
            close()
            throw AlkoveException("mic.busy", "The microphone could not start: ${e.message}")
        }
        if (r.recordingState != AudioRecord.RECORDSTATE_RECORDING) {
            record = r
            close()
            throw AlkoveException("mic.busy", "The microphone did not start: another app or a call may hold it.")
        }
        record = r
        read = 0
        base = Double.MAX_VALUE
    }

    // a full hop, or a negative AudioRecord error (ERROR_DEAD_OBJECT: the microphone is gone)
    fun read(hop: FloatArray): Int {
        val r = record ?: return AudioRecord.ERROR_INVALID_OPERATION
        var got = 0
        while (got < hop.size) {
            val n = r.read(hop, got, hop.size - got, AudioRecord.READ_BLOCKING)
            if (n < 0) return n
            if (n == 0) return AudioRecord.ERROR
            got += n
        }
        read += got
        return got
    }

    // how far the reads lag behind the capture, in ms; the platform's frame count starts anywhere, so it is measured from its lowest
    fun backlogMs(): Double? {
        val r = record ?: return null
        if (r.getTimestamp(ts, AudioTimestamp.TIMEBASE_MONOTONIC) != AudioRecord.SUCCESS) return null
        val captured = ts.framePosition + (System.nanoTime() - ts.nanoTime) * rate / 1e9
        val ahead = captured - read
        base = minOf(base, ahead)
        return (ahead - base) * 1000 / rate
    }

    fun close() {
        val r = record ?: return
        record = null
        if (Build.VERSION.SDK_INT >= 29) (callback as AudioManager.AudioRecordingCallback?)?.let { r.unregisterAudioRecordingCallback(it) }
        callback = null
        for (e in effects) e.release()
        effects.clear()
        try {
            r.stop()
        } catch (_: IllegalStateException) {
        }
        r.release()
    }

    companion object {
        const val RATE = 16000

        fun urgent() = Process.setThreadPriority(Process.THREAD_PRIORITY_URGENT_AUDIO)
    }
}
