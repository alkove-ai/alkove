package com.alkove.reactnative.livekit

import ai.alkove.AlkoveException
import ai.alkove.CallProcessor
import ai.alkove.Models
import ai.alkove.SpeechParams
import ai.alkove.Voiceprint
import com.facebook.react.bridge.Promise
import com.facebook.react.bridge.ReactApplicationContext
import com.livekit.reactnative.LiveKitReactNative
import com.livekit.reactnative.audio.processing.AudioProcessorInterface
import java.nio.ByteBuffer
import java.nio.ByteOrder
import kotlinx.coroutines.CoroutineExceptionHandler
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.launch
import org.json.JSONObject

class AlkoveLiveKitModule(context: ReactApplicationContext) : NativeAlkoveLiveKitSpec(context) {
  private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Default + CoroutineExceptionHandler { _, _ -> })
  private val lock = Any()

  // each attach and detach moves it: an attach that finishes after a newer one, or after a detach, is dropped
  private var generation = 0

  @Volatile
  private var focus: Focus? = null

  override fun attach(options: String, promise: Promise) {
    val gen = synchronized(lock) { ++generation }
    scope.launch {
      try {
        val o = JSONObject(options)
        val speech = o.optJSONObject("speech")?.let {
          SpeechParams(it.optDouble("confidence", 0.5), it.optDouble("exitConfidence", 0.1), it.optDouble("startSecs", 0.2), it.optDouble("stopSecs", 0.6))
        }
        val processor = CallProcessor(
          Models.load(reactApplicationContext, listOf("focus", "cue")),
          o.optJSONObject("voiceprint")?.let { Voiceprint.fromJson(it.toString()) },
          o.optDouble("strength", 1.0),
          o.optBoolean("normalize", false),
          speech,
        )
        val next = Focus(processor)
        val old = synchronized(lock) {
          if (gen != generation) null.also { next.close() }
          else focus.also {
            focus = next
            LiveKitReactNative.audioProcessingController.capturePostProcessor = next
          }
        }
        old?.close()
        promise.resolve(null)
      } catch (e: Throwable) {
        if (e is AlkoveException) promise.reject(e.code, e.message, e) else promise.reject("engine.failed", e.message ?: e.toString(), e)
      }
    }
  }

  override fun detach() {
    val old = synchronized(lock) {
      generation++
      focus.also {
        focus = null
        if (it != null && LiveKitReactNative.audioProcessingController.capturePostProcessor === it) LiveKitReactNative.audioProcessingController.capturePostProcessor = null
      }
    }
    old?.close()
  }

  override fun setEnabled(on: Boolean) {
    focus?.enabled = on
  }

  // a bad argument is ignored, as on iOS: an exception out of a void method ends the app
  override fun setStrength(strength: Double) {
    runCatching { focus?.processor?.setStrength(strength) }
  }

  override fun setVoiceprint(voiceprint: String) {
    runCatching {
      val v = JSONObject("{\"v\":$voiceprint}").optJSONObject("v")
      focus?.processor?.setVoiceprint(v?.let { Voiceprint.fromJson(it.toString()) })
    }
  }

  override fun processed(): Double = (focus?.frames ?: 0L).toDouble()

  override fun invalidate() {
    detach()
    scope.cancel()
    super.invalidate()
  }

  companion object {
    const val NAME = NativeAlkoveLiveKitSpec.NAME
  }
}

// as ai.alkove.livekit.AlkoveFocus, on LiveKit React Native's own processor interface
internal class Focus(val processor: CallProcessor) : AudioProcessorInterface, AutoCloseable {
  private var frame = FloatArray(480)

  @Volatile
  var frames = 0L

  @Volatile
  var enabled = true
    set(on) {
      field = on
      processor.setEnabled(on)
    }

  @Volatile
  private var closed = false

  override fun isEnabled() = enabled

  override fun getName() = "alkove-focus"

  override fun initializeAudioProcessing(sampleRateHz: Int, numChannels: Int) = processor.reset()

  override fun resetAudioProcessing(newRate: Int) = processor.reset()

  // channel 0, full band, float at int16 scale: 10 ms at numFrames × 100 Hz, read and written by index whatever the buffer's position
  override fun processAudio(numBands: Int, numFrames: Int, buffer: ByteBuffer) {
    if (closed) return
    buffer.order(ByteOrder.nativeOrder())
    val n = minOf(numFrames, buffer.capacity() / 4)
    if (frame.size < n) frame = FloatArray(n)
    for (i in 0 until n) frame[i] = buffer.getFloat(4 * i) / 32768f
    // an exception must not reach WebRTC's audio thread: the frame goes through as it came
    try {
      processor.process(frame, numFrames * 100, 1, n)
    } catch (e: Throwable) {
      return
    }
    for (i in 0 until n) buffer.putFloat(4 * i, frame[i] * 32768f)
    frames++
  }

  override fun close() {
    closed = true
    processor.close()
  }
}
