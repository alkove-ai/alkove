package com.alkove.reactnative

import ai.alkove.AgentInput
import ai.alkove.Alkove
import ai.alkove.AgentOptions
import ai.alkove.AlkoveException
import ai.alkove.AlkoveInput
import ai.alkove.Event
import ai.alkove.Format
import ai.alkove.InterruptParams
import ai.alkove.Lex
import ai.alkove.Mode
import ai.alkove.Models
import ai.alkove.Output
import ai.alkove.PhraseLevel
import ai.alkove.SpeechParams
import ai.alkove.Status
import ai.alkove.TurnParams
import ai.alkove.Voiceprint
import ai.alkove.Wake
import ai.alkove.WakeListener
import ai.alkove.WakeOptions
import android.util.Base64
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
        val word = Lex(Models.load(reactApplicationContext, listOf("lex"))).use { it.compile(text, (0 until langs.size()).map { i -> langs.getString(i)!! }) }
        val warnings = JSONArray(word.warnings.map { JSONObject(mapOf("code" to it.code, "message" to it.message)) })
        promise.resolve(JSONObject().put("word", JSONObject(word.toJson())).put("warnings", warnings).toString())
      } catch (e: Throwable) {
        reject(promise, e)
      }
    }
  }

  override fun start(id: Double, kind: String, options: String, promise: Promise) {
    scope.launch {
      try {
        val o = JSONObject(options)
        val input = when (kind) {
          "wake" -> WakeListener.start(reactApplicationContext, WakeOptions(strings(o.getJSONArray("words")), o.optString("context", "pause")))
          "agent" -> AgentInput.start(reactApplicationContext, agentOptions(o), if (o.optBoolean("audio")) { chunk -> emit(id, "audio", JSONObject.quote(base64(chunk))) } else null)
          else -> throw IllegalArgumentException("no input $kind")
        }
        val job = scope.launch { input.events.collect { send(id, it) } }
        inputs.put(id.toInt(), input to job)?.let { (old, oldJob) -> oldJob.cancel(); old.close() }
        promise.resolve(JSONObject().put("mode", mode(input.mode)).put("status", status(input.status)).toString())
      } catch (e: Throwable) {
        reject(promise, e)
      }
    }
  }

  override fun call(id: Double, method: String, arg: String) {
    val input = inputs[id.toInt()]?.first ?: return
    val v = JSONArray("[$arg]").opt(0)
    when (method) {
      "listen" -> input.listen(v as Boolean)
      "hold" -> input.hold()
      "release" -> input.release()
      "farEndActive" -> input.farEndActive(v as Boolean)
      "setStrength" -> input.setStrength((v as Number).toDouble())
      "setVoiceprint" -> input.setVoiceprint(if (v is JSONObject) Voiceprint.fromJson(v.toString()) else null)
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
    val (type, v) = when (e) {
      is Wake -> "wake" to JSONObject().put("word", e.word).put("score", e.score).put("threshold", e.threshold).put("context", e.context).put("start", e.start).put("end", e.end)
      is Event.SpeechStart -> "speechstart" to JSONObject().put("start", e.start)
      is Event.SpeechEnd -> "speechend" to level(JSONObject().put("start", e.start).put("end", e.end), e.level)
      is Event.SpeechCancel -> "speechcancel" to JSONObject().put("start", e.start)
      is Event.Turn -> "turn" to JSONObject().put("pause", e.pause).put("probability", e.probability).put("hesitation", e.hesitation).put("at", e.at)
      is Event.TurnEnd -> "turnend" to JSONObject().put("start", e.start).put("end", e.end).put("probability", e.probability).put("hesitation", e.hesitation).put("at", e.at)
      is Event.InterruptStart -> "interruptstart" to JSONObject().put("start", e.start)
      is Event.Interrupt -> "interrupt" to JSONObject().put("start", e.start).put("at", e.at)
      is Event.InterruptCancel -> "interruptcancel" to JSONObject().put("start", e.start).put("end", e.end)
      is Event.Utterance -> "utterance" to level(JSONObject().put("pcm", base64(e.pcm)).put("start", e.start).put("end", e.end), e.level)
      is Event.VoiceprintLearned -> "voiceprint" to JSONObject(e.voiceprint.toJson())
      is Event.Learn -> "learn" to JSONObject().put("progress", e.progress)
      is Event.ModeChange -> "mode" to JSONObject().put("mode", mode(e.mode)).put("reason", e.reason?.name?.lowercase())
      is Event.StatusChange -> "status" to status(e.status)
      is Event.Diag -> "diag" to JSONObject(e.data).put("code", e.code).put("level", e.level)
      is Event.Mic -> "mic" to JSONObject().put("state", e.state.name.lowercase())
      else -> return
    }
    emit(id, type, if (v is String) JSONObject.quote(v) else v.toString())
  }

  private fun emit(id: Double, type: String, json: String) {
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

    private fun strings(a: JSONArray) = (0 until a.length()).map { a.getString(it) }

    private fun base64(b: ByteArray) = Base64.encodeToString(b, Base64.NO_WRAP)

    private fun level(o: JSONObject, l: PhraseLevel?) =
      if (l == null) o else o.put("level", JSONObject().put("voiceDb", l.voiceDb ?: JSONObject.NULL).put("noiseDb", l.noiseDb).put("refDb", l.refDb ?: JSONObject.NULL))

    private fun JSONObject.num(k: String, d: Double) = if (has(k) && !isNull(k)) getDouble(k) else d

    // the web's AgentOptions as JSON; turn and interrupt false for off
    private fun agentOptions(o: JSONObject): AgentOptions {
      val out = o.optJSONObject("output") ?: JSONObject()
      val speech = o.optJSONObject("speech")
      val turn = o.opt("turn")
      val interrupt = o.opt("interrupt")
      return AgentOptions(
        voiceprint = o.optJSONObject("voiceprint")?.let { Voiceprint.fromJson(it.toString()) },
        learn = o.optBoolean("learn"),
        farEnd = o.optBoolean("farEnd"),
        strength = o.num("strength", 1.0),
        normalize = o.optBoolean("normalize", true),
        speech = speech?.let { SpeechParams(it.num("confidence", 0.5), it.num("exitConfidence", 0.1), it.num("startSecs", 0.2), it.num("stopSecs", 0.6)) },
        output = Output(if (out.optString("format") == "s16") Format.S16 else Format.F32, out.optInt("rate", 16000), out.optInt("chunkMs", 32)),
        levels = o.optBoolean("levels"),
        near = when (val n = o.opt("near")) {
          true -> Alkove.NEAR_GAP_DB
          is JSONObject -> n.num("gapDb", Alkove.NEAR_GAP_DB)
          else -> null
        },
        turn = when (turn) {
          false -> null
          is JSONObject -> TurnParams(turn.num("threshold", 0.6), turn.num("minPause", 0.3), turn.num("maxPause", 1.5))
          else -> TurnParams()
        },
        interrupt = when (interrupt) {
          false -> null
          is JSONObject -> InterruptParams(interrupt.num("minDuration", 0.7))
          else -> InterruptParams()
        },
      )
    }

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
