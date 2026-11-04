package com.alkove.reactnative

import ai.alkove.AlkoveException
import ai.alkove.AlkoveInput
import ai.alkove.Event
import ai.alkove.Lex
import ai.alkove.Mode
import ai.alkove.Models
import ai.alkove.Status
import ai.alkove.Wake
import ai.alkove.WakeListener
import ai.alkove.WakeOptions
import com.facebook.react.bridge.Arguments
import com.facebook.react.bridge.Promise
import com.facebook.react.bridge.ReactApplicationContext
import com.facebook.react.bridge.ReadableArray
import java.util.concurrent.ConcurrentHashMap
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.launch
import org.json.JSONArray
import org.json.JSONObject

class AlkoveModule(context: ReactApplicationContext) : NativeAlkoveSpec(context) {
  private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Default)
  private val inputs = ConcurrentHashMap<Int, Pair<AlkoveInput, Job>>()

  override fun compile(text: String, langs: ReadableArray, promise: Promise) {
    scope.launch {
      try {
        val word = Lex(Models.load(reactApplicationContext, listOf("lex"))).use { it.compile(text, strings(langs)) }
        val warnings = JSONArray(word.warnings.map { JSONObject(mapOf("code" to it.code, "message" to it.message)) })
        promise.resolve(JSONObject().put("word", JSONObject(word.toJson())).put("warnings", warnings).toString())
      } catch (e: Throwable) {
        reject(promise, e)
      }
    }
  }

  override fun startWake(id: Double, words: ReadableArray, context: String, promise: Promise) {
    scope.launch {
      try {
        val listener = WakeListener.start(reactApplicationContext, WakeOptions(strings(words), context))
        val job = scope.launch { listener.events.collect { send(id, it) } }
        inputs.put(id.toInt(), listener to job)?.let { (old, oldJob) -> oldJob.cancel(); old.close() }
        promise.resolve(JSONObject().put("mode", mode(listener.mode)).put("status", status(listener.status)).toString())
      } catch (e: Throwable) {
        reject(promise, e)
      }
    }
  }

  override fun close(id: Double) {
    inputs.remove(id.toInt())?.let { (input, job) ->
      job.cancel()
      input.close()
    }
  }

  override fun invalidate() {
    for (id in inputs.keys) close(id.toDouble())
    scope.cancel()
    super.invalidate()
  }

  private fun send(id: Double, e: Event) {
    val (type, json) = when (e) {
      is Wake -> "wake" to JSONObject().put("word", e.word).put("score", e.score).put("threshold", e.threshold).put("context", e.context).put("start", e.start).put("end", e.end).toString()
      is Event.ModeChange -> "mode" to JSONObject().put("mode", mode(e.mode)).put("reason", e.reason?.name?.lowercase()).toString()
      is Event.StatusChange -> "status" to JSONObject.quote(status(e.status))
      is Event.Diag -> "diag" to JSONObject(e.data).put("code", e.code).put("level", e.level).toString()
      is Event.Mic -> "mic" to JSONObject().put("state", e.state.name.lowercase()).toString()
      else -> return
    }
    emitOnEvent(Arguments.createMap().apply {
      putDouble("id", id)
      putString("type", type)
      putString("json", json)
    })
  }

  private fun reject(promise: Promise, e: Throwable) {
    if (e is AlkoveException) promise.reject(e.code, e.message, e) else promise.reject("engine.failed", e.message ?: e.toString(), e)
  }

  companion object {
    const val NAME = NativeAlkoveSpec.NAME

    private fun strings(a: ReadableArray) = (0 until a.size()).map { a.getString(it)!! }

    private fun mode(m: Mode) = when (m) {
      Mode.RAW -> "raw"
      Mode.FOCUS -> "focus"
      Mode.FOCUS_PRINT -> "focus+print"
      Mode.SPOT -> "spot"
      Mode.SPOT_PRINT -> "spot+print"
    }

    // the web's names: an input closed is disposed
    private fun status(s: Status) = if (s == Status.CLOSED) "disposed" else s.name.lowercase()
  }
}
