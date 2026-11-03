import SwiftUI

struct ContentView: View {
    @StateObject private var demo = Demo()
    @State private var word = "Camille"
    @State private var lang = "fr"

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            Button(demo.agentOn ? "Agent: stop" : "Agent: start") { demo.toggleAgent() }.accessibilityIdentifier("agent")
            Button(demo.focusOn ? "Focus: on" : "Focus: off") { demo.toggleFocus() }.disabled(!demo.agentOn)
            Button("Record 10 s (raw | Focus)") { demo.recordTen() }.disabled(!demo.agentOn)
            HStack {
                TextField("word, or /phonemes/", text: $word).accessibilityIdentifier("word")
                TextField("fr, en, es, pt, it, ca, ro", text: $lang).frame(width: 60).accessibilityIdentifier("lang")
            }
            .textFieldStyle(.roundedBorder)
            .autocorrectionDisabled()
            HStack {
                Button("Compile") { demo.compileWord(word, lang) }.accessibilityIdentifier("compile")
                Button(demo.wakeOn ? "Wake word: stop" : "Wake word: start") { demo.toggleWake(word, lang) }.accessibilityIdentifier("wake")
            }
            Button("Bench (native)") { demo.bench() }.disabled(demo.busy).accessibilityIdentifier("bench")
            ScrollView {
                Text(demo.lines.joined(separator: "\n"))
                    .font(.system(size: 13, design: .monospaced))
                    .textSelection(.enabled)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .accessibilityIdentifier("log")
            }
        }
        .buttonStyle(.bordered)
        .padding()
    }
}
