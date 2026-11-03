@testable import Alkove
import Foundation
import XCTest

// the microphone's path past AVAudioEngine's tap, without opening one: the ring at 48 kHz, the hops at 16 kHz
final class MicTests: XCTestCase {
    func testTheTapsSamplesComeOutAsTheCoresResamplerGivesThem() throws {
        let pcm = try resample(Fixtures.wav("mix.wav"), from: 16000, to: 48000)
        let mic = Mic(echoCancellation: false)
        let ring = Ring(capacity: 96000)
        try mic.attach(ring, rate: 48000)
        let feeder = Thread {
            var i = 0
            while i < pcm.count {
                let n = min(1024, pcm.count - i)
                pcm.withUnsafeBufferPointer { ring.write($0.baseAddress! + i, n) }
                i += n
                Thread.sleep(forTimeInterval: 0.001)
            }
        }
        feeder.start()
        let hops = (pcm.count / 3 - 2000) / 256
        var got = [Float]()
        var hop = [Float](repeating: 0, count: 256)
        for _ in 0 ..< hops {
            XCTAssertEqual(hop.withUnsafeMutableBufferPointer { mic.read($0) }, 256)
            got += hop
        }
        let r = try Resampler(from: 48000, to: 16000)
        var want = [Float]()
        var i = 0
        while want.count < got.count {
            want += try r.push(Array(pcm[i ..< min(pcm.count, i + 1024)]))
            i += 1024
        }
        XCTAssertEqual(got, Array(want[..<got.count]))
        XCTAssertNotNil(mic.backlogMs())
        mic.close()
        XCTAssertEqual(hop.withUnsafeMutableBufferPointer { mic.read($0) }, -1)
    }

    func testTheRingKeepsTheNewestTwoSeconds() {
        let ring = Ring(capacity: 4)
        let x: [Float] = [1, 2, 3, 4, 5, 6]
        x.withUnsafeBufferPointer { ring.write($0.baseAddress!, 6) }
        var out = [Float](repeating: 0, count: 8)
        XCTAssertEqual(ring.take(into: &out, timeout: 0.01), 4)
        XCTAssertEqual(Array(out[..<4]), [3, 4, 5, 6])
        XCTAssertEqual(ring.take(into: &out, timeout: 0.01), 0)
    }

    func testAWakeEndsTheWaitAtOnce() {
        let ring = Ring(capacity: 4)
        var out = [Float](repeating: 0, count: 4)
        DispatchQueue.global().asyncAfter(deadline: .now() + 0.1) { ring.wake() }
        let t0 = Date()
        XCTAssertEqual(ring.take(into: &out, timeout: 2), 0)
        XCTAssertLessThan(Date().timeIntervalSince(t0), 0.5)
    }

    // a route change: the reader waiting on the old engine's ring goes on with the new one at once
    func testANewRingWakesTheReader() throws {
        let mic = Mic(echoCancellation: false)
        try mic.attach(Ring(capacity: 32000), rate: 16000)
        let fresh = Ring(capacity: 32000)
        DispatchQueue.global().asyncAfter(deadline: .now() + 0.1) {
            _ = try? mic.attach(fresh, rate: 16000)
            let x = [Float](repeating: 0.5, count: 256)
            x.withUnsafeBufferPointer { fresh.write($0.baseAddress!, 256) }
        }
        var hop = [Float](repeating: 0, count: 256)
        let t0 = Date()
        XCTAssertEqual(hop.withUnsafeMutableBufferPointer { mic.read($0) }, 256)
        XCTAssertLessThan(Date().timeIntervalSince(t0), 0.5)
        XCTAssertEqual(hop, [Float](repeating: 0.5, count: 256))
    }

    func testAFractionalHardwareRateOpens() {
        XCTAssertNoThrow(try Mic(echoCancellation: false).attach(Ring(capacity: 4), rate: 47999.99))
    }
}
