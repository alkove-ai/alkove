@testable import Alkove
import XCTest

final class NativeTests: XCTestCase {
    func testInfo() throws {
        XCTAssertGreaterThan(try Native.info().count, 1000)
    }

    func testErrorsReachSwift() {
        XCTAssertThrowsError(try Native.jsonParse(Data("{\"a\":".utf8))) { e in
            let n = e as! NativeError
            XCTAssertEqual(n.kind, 2)
            XCTAssertEqual(n.message, "JSON: unexpected end")
        }
    }
}
