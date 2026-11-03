@testable import Alkove
import Foundation
import XCTest

final class ModelsTests: XCTestCase {
    func testFetchFromHuggingFaceThenTheCache() async throws {
        try XCTSkipIf(ProcessInfo.processInfo.environment["ALKOVE_FETCH"] == nil, "ALKOVE_FETCH to download from Hugging Face")
        let cache = FileManager.default.temporaryDirectory.appendingPathComponent("alkove-test-\(UUID().uuidString)")
        defer { try? FileManager.default.removeItem(at: cache) }
        XCTAssertFalse(Models.cached(["cue"], cache: cache))
        let seen = Seen()
        let m = try await Models.load(["cue", "cue"], cache: cache) { seen.last = $0 }
        XCTAssertTrue(m.contains("cue") && Models.cached(["cue"], cache: cache))
        XCTAssertEqual(seen.last?.loaded, seen.last?.total)
        let path = try Models.path(cache, "cue")
        try Data("short".utf8).write(to: path)
        XCTAssertThrowsError(try Models.checked("cue", Data(contentsOf: path))) { XCTAssertEqual(($0 as! AlkoveError).code, "models.hash") }
        _ = try await Models.load(["cue"], cache: cache)
        XCTAssertNoThrow(try Models.checked("cue", Data(contentsOf: path)))
        do {
            _ = try await Models.load(["spot"], cache: cache, download: false)
            XCTFail("no download asked")
        } catch let e as AlkoveError {
            XCTAssertEqual(e.code, "models.fetch")
        }
    }
}

private final class Seen: @unchecked Sendable {
    var last: DownloadProgress?
}
