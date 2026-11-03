import Alkove
import Foundation

@MainActor
final class Demo: ObservableObject {
    @Published var lines: [String] = []
    @Published var agentOn = false
    @Published var focusOn = true
    @Published var wakeOn = false
    @Published var busy = false
    @Published var agentStarting = false
    @Published var wakeStarting = false
    private var agent: AgentInput?
    private var wake: WakeListener?

    init() {
        say("Alkove \(Alkove.version)")
    }

    func say(_ s: String) {
        lines.insert(s, at: 0)
        if lines.count > 200 { lines.removeLast() }
    }

    private func f(_ x: Double, _ d: Int = 2) -> String { String(format: "%.\(d)f", x) }

    private func show(_ e: Event) {
        switch e {
        case let .speechStart(s): say("speech from \(f(s)) s")
        case let .speechEnd(s, en, _): say("speech \(f(s)) to \(f(en)) s")
        case let .turnEnd(_, en, p, _, _): say("turn end at \(f(en)) s (p \(f(p)))")
        case let .mode(m, r): say("mode \(m.rawValue)\(r.map { " (\($0.rawValue))" } ?? "")")
        case let .status(s): say("status \(s.rawValue)")
        case let .mic(m): say("mic \(m.rawValue)")
        case let .utterance(u): say("utterance \(f(u.end - u.start)) s, \(u.pcm.count) bytes")
        case let .wake(w): say("WAKE \(w.word) score \(f(w.score)) (threshold \(f(w.threshold))) at \(f(w.start)) s\(w.voice.map { ", voice \(f($0))" } ?? "")")
        case let .diag(d) where d.code == "timing":
            say("timing \(d.data["meanMs"]?.num ?? 0) ms per 16 ms hop (max \(d.data["maxMs"]?.num ?? 0), slow \(Int(d.data["slow"]?.num ?? 0)), queue \(Int(d.data["queueMs"]?.num ?? 0)) ms)")
        case let .diag(d): say("diag \(d.code) \(d.level)")
        default: break
        }
    }

    func toggleAgent() {
        if let a = agent {
            a.close()
            agent = nil
            agentOn = false
            return
        }
        guard !agentStarting else { return }
        agentStarting = true
        Task {
            do {
                let a = try await AgentInput.start(AgentOptions(keepRecent: 30, stats: true))
                if !focusOn { a.setEnabled(false) }
                agent = a
                agentOn = true
                agentStarting = false
                for await e in a.events { show(e) }
            } catch {
                agentStarting = false
                say("agent failed: \(error)")
            }
        }
    }

    func toggleFocus() {
        focusOn.toggle()
        agent?.setEnabled(focusOn)
    }

    func recordTen() {
        guard let a = agent else { return say("start the agent first") }
        Task {
            say("recording 10 s…")
            do {
                let wav = try await a.record(seconds: 10)
                let out = FileManager.default.urls(for: .documentDirectory, in: .userDomainMask)[0].appendingPathComponent("alkove-\(Int(Date().timeIntervalSince1970)).wav")
                try wav.write(to: out)
                say("saved \(out.lastPathComponent) (Files, On My iPhone, Alkove)")
                if let s = await a.stats() { say("stats: \(f(s.speechSeconds, 1)) s of speech out of \(f(s.seconds, 1)) s, \(s.noiseRemovedDb.map { f($0, 1) } ?? "-") dB removed") }
            } catch {
                say("record failed: \(error)")
            }
        }
    }

    private func word(_ text: String, _ lang: String) async throws -> Word {
        if text.trimmingCharacters(in: .whitespaces).hasPrefix("/") { return try compile(text) }
        return try Lex(models: try await Alkove.models(["lex"])).compile(text, lang: lang)
    }

    func compileWord(_ text: String, _ lang: String) {
        Task {
            do {
                say("compiling \"\(text)\"…")
                let w = try await word(text, lang)
                say("\(w), thresholds \(w.thresholds.sorted { $0.key < $1.key }.map { "\($0.key) \(f($0.value))" }.joined(separator: ", "))")
                for warning in w.warnings { say("warning \(warning.code): \(warning.message)") }
            } catch {
                say("compile failed: \(error)")
            }
        }
    }

    func toggleWake(_ text: String, _ lang: String) {
        if let l = wake {
            l.close()
            wake = nil
            wakeOn = false
            return
        }
        guard !wakeStarting else { return }
        wakeStarting = true
        Task {
            do {
                let w = try await word(text, lang)
                say("listening for \(w)")
                let l = try await WakeListener.start(WakeOptions(words: [w], context: "anywhere"))
                wake = l
                wakeOn = true
                wakeStarting = false
                for await e in l.events { show(e) }
            } catch {
                wakeStarting = false
                say("wake failed: \(error)")
            }
        }
    }

    func bench() {
        busy = true
        say("bench: loading Focus, Cue, Spot…")
        Task.detached(priority: .userInitiated) {
            let r: String
            do {
                let models = try await Alkove.models(["focus", "cue", "spot"])
                let seconds = 20
                var s: UInt64 = 1
                let pcm = (0 ..< seconds * Alkove.rate).map { _ -> Float in
                    s = s &* 6_364_136_223_846_793_005 &+ 1
                    return (Float(s >> 40) / Float(1 << 24) * 2 - 1) * 0.1
                }
                let chain = try Chain(models: models, sleep: false)
                var out = [Float](repeating: 0, count: Chain.hop)
                let t0 = Date()
                var i = 0
                while i + Chain.hop <= pcm.count {
                    _ = try chain.step(Array(pcm[i ..< i + Chain.hop]), into: &out)
                    i += Chain.hop
                }
                let chainMs = Date().timeIntervalSince(t0) * 1000 / Double(seconds)
                // the same hops one every 16 ms, as the microphone gives them: a phone keeps its clock low for such a load
                var paced = 0.0
                for k in 0 ..< 300 {
                    let hop = Array(pcm[k * Chain.hop ..< (k + 1) * Chain.hop])
                    let s0 = Date()
                    _ = try chain.step(hop, into: &out)
                    paced += Date().timeIntervalSince(s0)
                    try await Task.sleep(nanoseconds: 16_000_000)
                }
                let listener = try Listener(models: models, words: [try compile("/kamij/")], context: "anywhere")
                let t1 = Date()
                _ = try listener.push(pcm)
                let spotMs = Date().timeIntervalSince(t1) * 1000 / Double(seconds)
                r = String(format: "bench: Focus + Cue %.2f ms per second of audio, Spot %.2f ms; one hop every 16 ms: %.2f ms per hop", chainMs, spotMs, paced * 1000 / 300)
            } catch {
                r = "bench failed: \(error)"
            }
            await MainActor.run {
                self.say(r)
                self.busy = false
            }
        }
    }
}
