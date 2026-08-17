import XCTest

final class X8086UITests: XCTestCase {
    func testAppLaunchesAndShowsFfiRoundTrip() {
        let app = XCUIApplication()
        app.launch()

        let label = app.staticTexts["pingResultLabel"]
        XCTAssertTrue(label.waitForExistence(timeout: 5))
        // On macOS (unlike iOS), AppKit-backed Text elements report their
        // displayed text via the accessibility `value`, not `label` - `label`
        // is often empty unless a separate accessibility label is set.
        let text = (label.value as? String) ?? label.label
        XCTAssertTrue(text.contains("pong"), "expected the FFI round-trip text to contain \"pong\", got: \(text)")
    }
}
