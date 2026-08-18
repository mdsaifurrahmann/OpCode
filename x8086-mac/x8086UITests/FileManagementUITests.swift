import XCTest

/// File-menu flows that are safely automatable without driving a real
/// system `NSOpenPanel`/`NSSavePanel` (that automation is flaky enough
/// in CI that it isn't worth it here) - Open Sample and New. Actual
/// disk I/O (open/save/recent-files) is covered directly against
/// `DocumentController` in `x8086Tests/DocumentControllerTests.swift`.
final class FileManagementUITests: XCTestCase {
    func testOpeningASampleReplacesTheEditorContent() {
        let app = XCUIApplication()
        app.launch()

        let editor = app.textViews["sourceEditorTextView"]
        XCTAssertTrue(editor.waitForExistence(timeout: 5))

        app.menuBarItems["File"].click()
        app.menuItems["Open Sample"].click()
        app.menuItems["Counting Loop"].click()

        let becameCountingLoop = XCTNSPredicateExpectation(
            predicate: NSPredicate { _, _ in
                ((editor.value as? String) ?? "").contains("LOOP top")
            },
            object: nil
        )
        XCTAssertEqual(XCTWaiter().wait(for: [becameCountingLoop], timeout: 5), .completed)
    }

    func testNewResetsToTheDefaultHelloWorldSample() {
        let app = XCUIApplication()
        app.launch()

        let editor = app.textViews["sourceEditorTextView"]
        XCTAssertTrue(editor.waitForExistence(timeout: 5))
        editor.click()
        editor.typeKey("a", modifierFlags: .command)
        editor.typeText("HLT\n")

        app.menuBarItems["File"].click()
        app.menuItems["New"].click()

        let becameHelloWorld = XCTNSPredicateExpectation(
            predicate: NSPredicate { _, _ in
                ((editor.value as? String) ?? "").contains("Hello, x8086!")
            },
            object: nil
        )
        XCTAssertEqual(XCTWaiter().wait(for: [becameHelloWorld], timeout: 5), .completed)
    }
}
