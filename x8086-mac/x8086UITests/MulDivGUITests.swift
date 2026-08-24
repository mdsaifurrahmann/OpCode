import AppKit
import XCTest

/// GUI-level verification of a reported "the emulator computes 2*3
/// wrong" case, driven through the real app exactly as a user would:
/// source typed into the editor, Run clicked, digits typed at the
/// console prompt, output read back off the screen.
///
/// Two variants of one program, differing only in whether the product is
/// preserved across the intervening DOS call, prove the emulation is
/// correct and the original source was not: the as-written version
/// cannot print "6", and the one-line-fixed version does.
final class MulDivGUITests: XCTestCase {
    /// The reported program. `{FIX}` is substituted per-variant so the
    /// two runs differ by exactly one edit and nothing else.
    private static let programTemplate = """
    .MODEL SMALL
    .STACK 100H

    .DATA
        msg1 DB "Enter 1st number: $"
        msg2 DB 0DH,0AH,"Enter 2nd number: $"
        msg3 DB 0DH,0AH,"Product = $"

    .CODE
    MAIN PROC
        MOV AX, @DATA
        MOV DS, AX

        LEA DX, msg1
        MOV AH, 09H
        INT 21H
        MOV AH, 01H
        INT 21H
        SUB AL, '0'
        MOV BL, AL

        LEA DX, msg2
        MOV AH, 09H
        INT 21H
        MOV AH, 01H
        INT 21H
        SUB AL, '0'
        MOV CL, AL

        MOV AL, BL
        MUL CL

        LEA DX, msg3
        MOV AH, 09H
        INT 21H
    {FIX}
        MOV BL, 10
        DIV BL
        MOV CL, AH

        CMP AL, 0
        JE ONES
        ADD AL, '0'
        MOV DL, AL
        MOV AH, 02H
        INT 21H
    ONES:
        MOV DL, CL
        ADD DL, '0'
        MOV AH, 02H
        INT 21H

        MOV AH, 4CH
        INT 21H
    MAIN ENDP
    END MAIN
    """

    /// Loads `source` into the editor via the pasteboard rather than
    /// `typeText`: a ~40-line program is slow enough to type character by
    /// character that the test spends most of its time on keystrokes, and
    /// any editor-side auto-indent would corrupt the source in ways that
    /// have nothing to do with what's under test.
    private func loadSource(_ source: String, into app: XCUIApplication) {
        let editor = app.textViews["sourceEditorTextView"]
        XCTAssertTrue(editor.waitForExistence(timeout: 10))
        let pasteboard = NSPasteboard.general
        pasteboard.clearContents()
        pasteboard.setString(source, forType: .string)
        editor.click()
        editor.typeKey("a", modifierFlags: .command)
        editor.typeKey("v", modifierFlags: .command)
    }

    /// Waits for the program to block on `INT 21h AH=01h`, then sends one
    /// digit. The console grabs first-responder status itself while
    /// waiting (see `ConsoleView`), so the keystroke needs no extra
    /// click to land.
    private func answerPrompt(_ digit: String, in app: XCUIApplication) {
        let waiting = app.staticTexts["keyboardWaitingLabel"]
        XCTAssertTrue(
            waiting.waitForExistence(timeout: 10),
            "program should have paused for keyboard input before '\(digit)'"
        )
        app.typeText(digit)
    }

    private func consoleText(_ app: XCUIApplication) -> String {
        let console = app.staticTexts["consoleOutputLabel"]
        guard console.exists else { return "" }
        return (console.value as? String) ?? console.label
    }

    private func runProgram(fix: String, in app: XCUIApplication) -> String {
        loadSource(Self.programTemplate.replacingOccurrences(of: "{FIX}", with: fix), into: app)

        app.buttons["runButton"].click()
        answerPrompt("2", in: app)
        answerPrompt("3", in: app)

        XCTAssertTrue(
            app.staticTexts["haltedLabel"].waitForExistence(timeout: 10),
            "program should have run to termination"
        )
        XCTAssertFalse(
            app.otherElements["diagnosticsList"].exists,
            "this program must assemble cleanly - any diagnostic means the test is measuring the wrong thing"
        )
        return consoleText(app)
    }

    /// As written, the program's own `MOV AH, 09H` (selecting the DOS
    /// print-string function for the "Product = " label) overwrites the
    /// high half of AX between `MUL CL` and `DIV BL`, so DIV divides
    /// 0x0906 = 2310 instead of 6. It cannot print "6", and this pins
    /// that: the emulator faithfully reproduces the program's own defect
    /// rather than quietly second-guessing it.
    func testAsWrittenTheProgramCannotPrintSix() {
        let app = XCUIApplication()
        app.launch()

        let console = runProgram(fix: "", in: app)

        XCTAssertTrue(
            console.contains("Enter 1st number: 2"),
            "the first typed digit should have echoed - got: \(console.debugDescription)"
        )
        XCTAssertTrue(
            console.contains("Enter 2nd number: 3"),
            "the second typed digit should have echoed - got: \(console.debugDescription)"
        )
        XCTAssertTrue(console.contains("Product = "), "the label should still print")
        XCTAssertFalse(
            console.contains("Product = 6"),
            """
            As written this program clobbers AH between MUL and DIV, so \
            real 8086 hardware, real DOS, and emu8086 all print garbage \
            here too. If this ever starts printing "6", the emulator has \
            stopped being faithful. Got: \(console.debugDescription)
            """
        )
    }

    /// The same program with the product recomputed after the DOS call
    /// (the minimal fix) prints exactly "6" - which is what proves MUL,
    /// DIV, and the console path were all correct the whole time.
    func testPreservingTheProductMakesItPrintSix() {
        let app = XCUIApplication()
        app.launch()

        let console = runProgram(fix: "    MOV AL, BL\n    MUL CL\n", in: app)

        XCTAssertTrue(
            console.contains("Product = 6"),
            """
            With AX intact, DIV BL sees 6: AL=0 (tens digit, skipped by \
            the CMP/JE) and AH=6 (ones digit). Got: \(console.debugDescription)
            """
        )
    }
}
