@testable import Alkove
import Foundation
import XCTest

// the microphone's path past AVAudioEngine's tap, without opening one: the ring at 48 kHz, the hops at 16 kHz
final class MicTests: XCTestCase {
    func testTheTapsSamplesComeOutAsTheCoresResamplerGivesThem() throws {
        let pcm = try resample(Fixtures.wav("mix.wav"), from: 16000, to: 48000)
        let mic = Mic(echoCancellation: false)
        let ring = Ring(capacity: 96000)
        mic.attach(ring, rate: 48000)
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
}
