import XCTest

final class X8086UITests: XCTestCase {
    /// The Phase 4 exit criterion from the approved plan: a real
    /// DOS-style hello-world program (INT 21h AH=09h) runs correctly
    /// inside the actual app, not just in `cargo test`.
    func testHelloWorldProgramRunsAndPrintsToTheConsole() {
        let app = XCUIApplication()
        app.launch()

        // The source editor starts pre-loaded with a small hello-world
        // sample (LEA/MOV/INT 21h/HLT + a DB string) - tapping Run
        // exercises the full pipeline without needing to type anything.
        let runButton = app.buttons["runButton"]
        XCTAssertTrue(runButton.waitForExistence(timeout: 5))
        runButton.click()

        let consoleOutput = app.staticTexts["consoleOutputLabel"]
        XCTAssertTrue(consoleOutput.waitForExistence(timeout: 5))

        let expectation = XCTNSPredicateExpectation(
            predicate: NSPredicate { _, _ in
                let text = (consoleOutput.value as? String) ?? consoleOutput.label
                return text.contains("Hello, x8086!")
            },
            object: nil
        )
        XCTAssertEqual(XCTWaiter().wait(for: [expectation], timeout: 5), .completed, "console output never showed the expected hello-world text")

        XCTAssertTrue(app.staticTexts["haltedLabel"].waitForExistence(timeout: 5), "program should have run to HLT")

        let diagnosticsList = app.otherElements["diagnosticsList"]
        XCTAssertFalse(diagnosticsList.exists, "a correctly-assembling program must not show any diagnostics")
    }
}
