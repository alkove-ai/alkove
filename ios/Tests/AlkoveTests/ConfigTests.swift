@testable import Alkove
import Foundation
import XCTest

// Alkove.configure: where every input, Models.load, preload and cached take the models
final class ConfigTests: XCTestCase {
    override func tearDown() {
        Alkove.configure()
    }

    private func code(_ f: () async throws -> Void) async -> String? {
        do {
            try await f()
            return nil
        } catch {
            return (error as? AlkoveError)?.code
        }
    }

    func testAFolderGivesEveryModel() async throws {
        _ = try Fixtures.load("focus", "cue")
        let names = Usage.agent.names(voiceprint: false, typedWords: false)
        XCTAssertEqual(names, ["focus", "cue", "timbre"])
        Alkove.configure(directory: Fixtures.modelDir)
        XCTAssertTrue(Alkove.cached(.agent))
        XCTAssertEqual(try Models.remote(names), [])
        let all = try await Models.configured(names)
        XCTAssertEqual(all.names, Set(names))
        let focus = try await Models.load(["focus"])
        XCTAssertEqual(focus.names, ["focus", "timbre"], "Focus brings Timbre")
        let empty = FileManager.default.temporaryDirectory.appendingPathComponent("alkove-config-\(UUID().uuidString)")
        Alkove.configure(directory: empty)
        XCTAssertFalse(Alkove.cached(.agent))
        let remote = await code { _ = try Models.remote(names) }
        XCTAssertEqual(remote, "models.fetch")
        let configured = await code { _ = try await Models.configured(names) }
        XCTAssertEqual(configured, "models.fetch")
    }

    func testAModelGivenGoesFirst() async throws {
        _ = try Fixtures.load("spot")
        let spot = try Data(contentsOf: Fixtures.modelDir.appendingPathComponent(try Models.file("spot")))
        Alkove.configure(models: ["spot": spot])
        XCTAssertTrue(Alkove.cached(.wake))
        XCTAssertEqual(try Models.remote(Usage.wake.names(voiceprint: false, typedWords: true)), ["lex"])
        XCTAssertEqual(Usage.wake.names(voiceprint: true, typedWords: false), ["spot", "timbre"])
        let given = try await Models.configured(["spot"])
        XCTAssertTrue(given.contains("spot"))
        Alkove.configure(models: ["spot": Data(count: 4)])
        let hash = await code { _ = try await Models.configured(["spot"]) }
        XCTAssertEqual(hash, "models.hash")
    }

    func testAHostOfOneselfNamesTheFile() throws {
        let base = URL(string: "https://example.com/alkove/")!
        Alkove.configure(modelBaseURL: base)
        XCTAssertEqual(try Models.url("cue"), base.appendingPathComponent(try Models.file("cue")))
        Alkove.configure()
        XCTAssertTrue(try Models.url("cue").absoluteString.hasPrefix("https://huggingface.co/"))
    }
}
