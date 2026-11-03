@testable import Alkove
import XCTest

final class NativeTests: XCTestCase {
    func testInfo() {
        XCTAssertEqual(Alkove.version, "0.4.0")
        XCTAssertEqual(Models.all, ["focus", "cue", "spot", "timbre", "lex"])
    }

    func testErrorsReachSwift() {
        XCTAssertThrowsError(try Json.parse("{\"a\":")) { e in
            XCTAssertEqual(e as? AlkoveError, AlkoveError(.argument, "", "JSON: unexpected end"))
        }
    }

    func testValuesRoundTrip() throws {
        let v: Value = .obj([("a", 1.5), ("b", .arr(["x", nil, true])), ("f", .floats([0.25, -1])), ("d", .bytes(Data([1, 2, 3])))])
        XCTAssertEqual(try Value.decode(v.encoded()), v)
    }

    func testEachNameOnceInItsFirstPlace() {
        XCTAssertEqual(Models.unique(["focus", "cue", "focus", "spot", "cue"]), ["focus", "cue", "spot"])
    }
}
