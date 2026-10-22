package ai.alkove

internal object Native {
    init {
        System.loadLibrary("alkove")
    }

    @JvmStatic external fun info(): ByteArray

    @JvmStatic external fun modelLoad(bytes: ByteArray): Long
    @JvmStatic external fun modelFree(m: Long)

    @JvmStatic external fun chainNew(focus: Long, cue: Long, options: ByteArray): Long
    @JvmStatic external fun chainFree(h: Long)
    @JvmStatic external fun chainStep(h: Long, hop: FloatArray, out: FloatArray, events: DoubleArray, focus: FloatArray?): Int
    @JvmStatic external fun chainSetPrint(h: Long, print: FloatArray?)
    @JvmStatic external fun chainSetStrength(h: Long, s: Double)
    @JvmStatic external fun chainSetEnabled(h: Long, on: Boolean)
    @JvmStatic external fun chainGet(h: Long, what: Int): Double
    @JvmStatic external fun chainListen(h: Long, on: Boolean)
    @JvmStatic external fun chainHold(h: Long, on: Boolean)

    @JvmStatic external fun liveNew(focus: Long, cue: Long, rate: Double, options: ByteArray): Long
    @JvmStatic external fun liveFree(h: Long)
    @JvmStatic external fun livePush(h: Long, pcm: FloatArray, n: Int, out: FloatArray): Long
    @JvmStatic external fun liveEvents(h: Long): DoubleArray
    @JvmStatic external fun liveSetPrint(h: Long, print: FloatArray?)
    @JvmStatic external fun liveSetStrength(h: Long, s: Double)
    @JvmStatic external fun liveSetEnabled(h: Long, on: Boolean)
    @JvmStatic external fun liveGet(h: Long, what: Int): Double
    @JvmStatic external fun liveListen(h: Long, on: Boolean)
    @JvmStatic external fun liveHold(h: Long, on: Boolean)

    @JvmStatic external fun trackerNew(confidence: Double, exitConfidence: Double, startSecs: Double, stopSecs: Double, frameSecs: Double): Long
    @JvmStatic external fun trackerFree(h: Long)
    @JvmStatic external fun trackerPush(h: Long, p: Double, k: Long, out: DoubleArray): Int
    @JvmStatic external fun trackerReset(h: Long)
    @JvmStatic external fun trackerGet(h: Long, what: Int): Boolean

    @JvmStatic external fun processorNew(focus: Long, cue: Long, spot: Long, options: ByteArray): Long
    @JvmStatic external fun processorFree(h: Long)
    @JvmStatic external fun processorPush(h: Long, pcm: FloatArray, n: Int): ByteArray
    @JvmStatic external fun processorEnd(h: Long): ByteArray
    @JvmStatic external fun process(focus: Long, cue: Long, spot: Long, pcm: FloatArray, n: Int, options: ByteArray): ByteArray

    @JvmStatic external fun resample(pcm: FloatArray, n: Int, inRate: Double, outRate: Double): FloatArray
    @JvmStatic external fun resamplerNew(inRate: Double, outRate: Double, aligned: Boolean): Long
    @JvmStatic external fun resamplerFree(h: Long)
    @JvmStatic external fun resamplerPush(h: Long, pcm: FloatArray, n: Int): FloatArray

    @JvmStatic external fun lexNew(m: Long): Long
    @JvmStatic external fun lexFree(h: Long)
    @JvmStatic external fun lexPhonemize(h: Long, args: ByteArray): ByteArray
    @JvmStatic external fun compile(lex: Long, args: ByteArray): ByteArray
    @JvmStatic external fun word(json: ByteArray): ByteArray
    @JvmStatic external fun distance(args: ByteArray): Double

    @JvmStatic external fun listenerNew(spot: Long, args: ByteArray): Long
    @JvmStatic external fun listenerFree(h: Long)
    @JvmStatic external fun listenerWords(h: Long, args: ByteArray)
    @JvmStatic external fun listenerPush(h: Long, pcm: FloatArray, n: Int): ByteArray

    @JvmStatic external fun enrollerNew(spot: Long, args: ByteArray): Long
    @JvmStatic external fun enrollerFree(h: Long)
    @JvmStatic external fun enrollerPush(h: Long, pcm: FloatArray, n: Int): ByteArray
    @JvmStatic external fun enrollerResult(h: Long): ByteArray

    @JvmStatic external fun timbreNew(m: Long): Long
    @JvmStatic external fun timbreFree(h: Long)
    @JvmStatic external fun timbreWindows(h: Long, pcm: FloatArray, n: Int): ByteArray
    @JvmStatic external fun timbreEmbed(h: Long, pcm: FloatArray, n: Int, args: ByteArray): ByteArray
    @JvmStatic external fun voiceprint(op: ByteArray): ByteArray

    @JvmStatic external fun learnerNew(args: ByteArray): Long
    @JvmStatic external fun learnerFree(h: Long)
    @JvmStatic external fun learnerLearn(h: Long, timbre: Long, pcm: FloatArray, n: Int): ByteArray
    @JvmStatic external fun learnerOffer(h: Long, timbre: Long, pcm: FloatArray, n: Int): ByteArray
    @JvmStatic external fun learnerVoiceprint(h: Long, raw: Boolean): ByteArray
    @JvmStatic external fun learnerState(h: Long): ByteArray
    @JvmStatic external fun learnerForget(h: Long)

    @JvmStatic external fun pipelineNew(focus: Long, cue: Long, spot: Long, options: ByteArray): Long
    @JvmStatic external fun pipelineFree(h: Long)
    @JvmStatic external fun pipelineStep(h: Long, hop: FloatArray, farQuietMs: Double, out: FloatArray): Int
    @JvmStatic external fun pipelineRaw(h: Long, hop: FloatArray): Int
    @JvmStatic external fun pipelineOuts(h: Long): ByteArray
    @JvmStatic external fun pipelineSpot(h: Long, m: Long)
    @JvmStatic external fun pipelineSet(h: Long, change: ByteArray)
    @JvmStatic external fun pipelineVerified(h: Long, key: Int, same: Boolean, score: Double)
    @JvmStatic external fun pipelineRecord(h: Long, seconds: Double)
    @JvmStatic external fun pipelineRecent(h: Long): ByteArray?
    @JvmStatic external fun pipelineStats(h: Long): ByteArray
    @JvmStatic external fun pipelineGet(h: Long, what: Int): Double

    @JvmStatic external fun coachNew(args: ByteArray): Long
    @JvmStatic external fun coachFree(h: Long)
    @JvmStatic external fun coachLearner(h: Long, args: ByteArray): Double
    @JvmStatic external fun coachForget(h: Long)
    @JvmStatic external fun coachWindow(h: Long, timbre: Long, pcm: FloatArray, n: Int, farQuietMs: Double, t: Double): ByteArray
    @JvmStatic external fun coachVerify(h: Long, timbre: Long, pcm: FloatArray, n: Int): ByteArray

    @JvmStatic external fun jsonStringify(value: ByteArray): ByteArray
    @JvmStatic external fun jsonParse(text: ByteArray): ByteArray
}
