import Alkove
@testable import AlkoveLiveKit
import Foundation
import XCTest

private let repo = URL(fileURLWithPath: #filePath).deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
private let modelDir = repo.appendingPathComponent("packages/alkove/.cache/models")

private func models() throws -> Models {
    try XCTSkipUnless(FileManager.default.fileExists(atPath: modelDir.appendingPathComponent("alkove-focus-1-preview.alk").path), "no models in packages/alkove/.cache/models")
    return try Models.load(["focus", "cue"], directory: modelDir)
}

private func wav(_ name: String) throws -> [Float] {
    let b = try Data(contentsOf: repo.appendingPathComponent("packages/alkove/test/fixtures/\(name)"))
    func u32(_ at: Int) -> Int { Int(b[at]) | Int(b[at + 1]) << 8 | Int(b[at + 2]) << 16 | Int(b[at + 3]) << 24 }
    var at = 12
    while String(decoding: b[at ..< at + 4], as: UTF8.self) != "data" { at += 8 + u32(at + 4) }
    return (0 ..< u32(at + 4) / 2).map { i in Float(Double(Int16(bitPattern: UInt16(b[at + 8 + 2 * i]) | UInt16(b[at + 9 + 2 * i]) << 8)) / 32768) }
}

final class AlkoveFocusTests: XCTestCase {
    // frames at int16 scale in, at int16 scale out: CallProcessor's output, scaled back
    func testFramesAtInt16ScaleAreCallProcessorsOutput() throws {
        let m = try models()
        let pcm = try resample(try wav("mix.wav"), from: 16000, to: 48000)
        let focus = try AlkoveFocus(models: m)
        let call = try CallProcessor(models: m)
        var got = [Float]()
        var want = [Float]()
        var stereo = [Float](repeating: 0, count: 480)
        var i = 0
        while i + 480 <= pcm.count {
            var frame = pcm[i ..< i + 480].map { $0 * 32768 }
            frame.withUnsafeMutableBufferPointer { a in
                stereo.withUnsafeMutableBufferPointer { b in
                    let ptrs = [a.baseAddress!, b.baseAddress!]
                    focus.process(channels: 2, frames: 480) { ptrs[$0] }
                }
            }
            XCTAssertEqual(stereo, frame, "every channel gets channel 0's output")
            got += frame
            var ref = Array(pcm[i ..< i + 480])
            try call.process(&ref, rate: 48000)
            want += ref.map { $0 * 32768 }
            i += 480
        }
        XCTAssertEqual(got, want)
        XCTAssertTrue(got.contains { $0 != 0 })
    }

    func testOffIsTheRawMicrophoneAtTheSameDelay() throws {
        let m = try models()
        let focus = try AlkoveFocus(models: m)
        focus.enabled = false
        XCTAssertFalse(focus.enabled)
        let call = try CallProcessor(models: m)
        try call.setEnabled(false)
        let pcm = try resample(try wav("mix.wav"), from: 16000, to: 48000)
        var i = 0
        while i + 480 <= pcm.count {
            var frame = pcm[i ..< i + 480].map { $0 * 32768 }
            frame.withUnsafeMutableBufferPointer { b in focus.process(channels: 1, frames: 480) { _ in b.baseAddress! } }
            var ref = Array(pcm[i ..< i + 480])
            try call.process(&ref, rate: 48000)
            XCTAssertEqual(frame, ref.map { $0 * 32768 })
            i += 480
        }
    }
}
