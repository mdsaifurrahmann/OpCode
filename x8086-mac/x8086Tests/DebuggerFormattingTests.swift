import XCTest

@testable import OpCode

final class DebuggerFormattingTests: XCTestCase {
    func testVariableValueTextSizesToTheDeclaredWidth() {
        XCTAssertEqual(DebuggerFormatting.variableValueText(0x2A, isWord: false), "2A")
        XCTAssertEqual(DebuggerFormatting.variableValueText(0x2A, isWord: true), "002A")
        XCTAssertEqual(DebuggerFormatting.variableValueText(0xBEEF, isWord: true), "BEEF")
    }

    func testWatchValueTextShowsAPlaceholderWhenUnresolvedAndHexWhenPresent() {
        XCTAssertEqual(DebuggerFormatting.watchValueText(nil), "?")
        XCTAssertEqual(DebuggerFormatting.watchValueText(0x21), "0021")
        XCTAssertEqual(DebuggerFormatting.watchValueText(0), "0000")
    }
}
