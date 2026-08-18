import XCTest

/// Phase 6 debugger-control flows: Step Into vs. Step Over across a
/// `CALL`, Step Back, live-editing a register/memory cell while paused,
/// and Run-to-cursor - each driven through the real toolbar/panels, not
/// the controller directly (see `DebuggerControllerTests` in
/// `x8086Tests` for the controller-level equivalents).
final class DebuggerFlowsUITests: XCTestCase {
    private func typeSource(_ app: XCUIApplication, _ source: String) {
        let editor = app.textViews["sourceEditorTextView"]
        XCTAssertTrue(editor.waitForExistence(timeout: 5))
        editor.click()
        editor.typeKey("a", modifierFlags: .command)
        editor.typeText(source)
    }

    private func waitForRegister(_ app: XCUIApplication, _ name: String, toEqual hex: String, timeout: TimeInterval = 5) {
        let field = app.textFields["register_\(name)"]
        let expectation = XCTNSPredicateExpectation(
            predicate: NSPredicate { _, _ in (field.value as? String) == hex },
            object: nil
        )
        XCTAssertEqual(
            XCTWaiter().wait(for: [expectation], timeout: timeout), .completed,
            "expected register \(name) to become \(hex), last saw \(field.value ?? "<nil>")"
        )
    }

    func testStepIntoEntersACallButStepOverRunsPastIt() {
        let app = XCUIApplication()
        app.launch()
        typeSource(app, "CALL sub\nHLT\nsub:\nMOV AX, 99\nRET\n")

        app.buttons["restartButton"].click()
        waitForRegister(app, "AX", toEqual: "0000")

        // Step Into the CALL only executes the CALL itself - the
        // subroutine body must not have run yet.
        app.buttons["stepIntoButton"].click()
        let axField = app.textFields["register_AX"]
        XCTAssertEqual(axField.value as? String, "0000", "Step Into a CALL must not execute the subroutine body")

        app.buttons["restartButton"].click()
        waitForRegister(app, "AX", toEqual: "0000")

        // Step Over the same CALL must run the whole subroutine.
        app.buttons["stepOverButton"].click()
        waitForRegister(app, "AX", toEqual: "0063") // 99 decimal
    }

    func testStepBackChangesTheRegistersPanel() {
        let app = XCUIApplication()
        app.launch()
        typeSource(app, "MOV AX, 1\nMOV AX, 2\nHLT\n")

        app.buttons["runButton"].click()
        XCTAssertTrue(app.staticTexts["haltedLabel"].waitForExistence(timeout: 5))
        waitForRegister(app, "AX", toEqual: "0002")

        app.buttons["stepBackButton"].click() // undoes HLT only
        waitForRegister(app, "AX", toEqual: "0002")
        XCTAssertFalse(app.staticTexts["haltedLabel"].exists, "stepping back over HLT must un-halt")

        app.buttons["stepBackButton"].click() // undoes the second MOV AX, 2
        waitForRegister(app, "AX", toEqual: "0001")
    }

    func testLiveEditingARegisterWhilePausedAffectsResumedExecution() {
        let app = XCUIApplication()
        app.launch()
        typeSource(app, "MOV BX, AX\nHLT\n")

        app.buttons["restartButton"].click()
        waitForRegister(app, "AX", toEqual: "0000")

        let axField = app.textFields["register_AX"]
        axField.click()
        axField.typeKey("a", modifierFlags: .command)
        axField.typeText("2A")
        axField.typeKey(.return, modifierFlags: [])
        waitForRegister(app, "AX", toEqual: "002A")

        app.buttons["stepIntoButton"].click() // MOV BX, AX
        waitForRegister(app, "BX", toEqual: "002A")
    }

    func testLiveEditingAMemoryCellWhilePausedAffectsResumedExecution() {
        let app = XCUIApplication()
        app.launch()
        typeSource(app, "MOV AL, [0010h]\nHLT\n")

        app.buttons["restartButton"].click()
        waitForRegister(app, "AX", toEqual: "0000")

        // Address 0x10 = 16 decimal, the first byte of the memory view's
        // second row (16 bytes/row, base address 0 by default) - no
        // need to jump the address field.
        let memoryByte = app.staticTexts["memoryByte_16"]
        XCTAssertTrue(memoryByte.waitForExistence(timeout: 5))
        memoryByte.click()

        let editField = app.textFields["memoryByte_16"]
        XCTAssertTrue(editField.waitForExistence(timeout: 5))
        editField.typeKey("a", modifierFlags: .command)
        editField.typeText("7F")
        editField.typeKey(.return, modifierFlags: [])

        app.buttons["stepIntoButton"].click() // MOV AL, [0x10]
        waitForRegister(app, "AX", toEqual: "007F", timeout: 5)
    }

    func testRunToCursorStopsAtTheSelectedDisassemblyLine() {
        let app = XCUIApplication()
        app.launch()
        typeSource(app, "MOV AX, 1\nMOV BX, 2\nMOV CX, 3\nHLT\n")

        app.buttons["restartButton"].click()
        waitForRegister(app, "AX", toEqual: "0000")

        // MOV AX,1 (3 bytes @ 0000), MOV BX,2 (3 bytes @ 0003),
        // MOV CX,3 @ 0006 - select that third instruction as the target.
        let targetLine = app.buttons["disasmLine_6"]
        XCTAssertTrue(targetLine.waitForExistence(timeout: 5))
        targetLine.click()

        app.buttons["runToCursorButton"].click()

        waitForRegister(app, "IP", toEqual: "0006")
        XCTAssertEqual(
            app.textFields["register_CX"].value as? String, "0000",
            "must stop before MOV CX,3 runs"
        )
    }
}
