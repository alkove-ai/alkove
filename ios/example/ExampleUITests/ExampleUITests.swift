import XCTest

// the app as a user taps it, with no microphone: the models from Hugging Face, a typed word, the native bench
final class ExampleUITests: XCTestCase {
    func testCompileAndBench() throws {
        let app = XCUIApplication()
        app.launch()
        let log = app.staticTexts["log"]
        XCTAssertTrue(log.waitForExistence(timeout: 10))
        app.buttons["compile"].tap()
        XCTAssertTrue(NSPredicate(format: "label CONTAINS 'kamij'").evaluate(with: log) || waitFor(log, "kamij", 120), log.label)
        app.buttons["bench"].tap()
        XCTAssertTrue(waitFor(log, "bench: Focus + Cue", 300), log.label)
        print(log.label)
    }

    private func waitFor(_ e: XCUIElement, _ text: String, _ timeout: Double) -> Bool {
        let p = NSPredicate(format: "label CONTAINS %@", text)
        return XCTWaiter().wait(for: [XCTNSPredicateExpectation(predicate: p, object: e)], timeout: timeout) == .completed
    }
}
