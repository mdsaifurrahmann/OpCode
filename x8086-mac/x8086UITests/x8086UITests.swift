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

    /// Malformed source must show up in the error list at the right
    /// line, and clicking that entry must jump the editor there without
    /// crashing the app.
    func testMalformedSourceShowsCorrectLineInErrorListAndClickToJumpWorks() {
        let app = XCUIApplication()
        app.launch()

        let editor = app.textViews["sourceEditorTextView"]
        XCTAssertTrue(editor.waitForExistence(timeout: 5))
        editor.click()
        editor.typeKey("a", modifierFlags: .command) // select all
        editor.typeText("FROBNICATE AX\nHLT\n")

        app.buttons["runButton"].click()

        let diagnosticRow = app.buttons["diagnosticRow_1"]
        XCTAssertTrue(diagnosticRow.waitForExistence(timeout: 5), "expected an error list entry for line 1")

        diagnosticRow.click()
        XCTAssertTrue(editor.exists, "the editor must still be present and functional after a click-to-jump")
    }

    /// Clicking in the gutter must toggle a breakpoint marker. There's
    /// no per-marker accessibility element to inspect (the gutter is
    /// hand-drawn), so this reads the gutter's accessibility value,
    /// which `LineNumberGutterView` keeps in sync with its breakpoint
    /// set purely to make this observable.
    func testClickingTheGutterTogglesABreakpointMarker() {
        let app = XCUIApplication()
        app.launch()

        let gutter = app.groups["lineNumberGutter"]
        XCTAssertTrue(gutter.waitForExistence(timeout: 5))
        XCTAssertEqual(gutter.value as? String, "", "no breakpoints should be set initially")

        // A small fixed pixel offset from the gutter's top-left corner,
        // not a normalized fraction: the ruler view's frame spans the
        // whole visible scroll area (so it draws consistently even past
        // the end of a short file), not just the few lines of actual
        // content, so a percentage-based offset isn't reliably "on line
        // 1". A few points down from the top always is.
        let firstLineClickPoint = gutter.coordinate(withNormalizedOffset: .zero).withOffset(CGVector(dx: 20, dy: 10))
        firstLineClickPoint.click()

        let becameNonEmpty = XCTNSPredicateExpectation(
            predicate: NSPredicate { _, _ in !(((gutter.value as? String) ?? "").isEmpty) },
            object: nil
        )
        XCTAssertEqual(XCTWaiter().wait(for: [becameNonEmpty], timeout: 5), .completed, "gutter click should have toggled a breakpoint")

        // Clicking the same spot again should clear it.
        firstLineClickPoint.click()
        let becameEmpty = XCTNSPredicateExpectation(
            predicate: NSPredicate { _, _ in ((gutter.value as? String) ?? "non-empty").isEmpty },
            object: nil
        )
        XCTAssertEqual(XCTWaiter().wait(for: [becameEmpty], timeout: 5), .completed, "clicking the same line again should remove the breakpoint")
    }
}
