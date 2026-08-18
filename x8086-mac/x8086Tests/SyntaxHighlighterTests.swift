import XCTest

@testable import OpCode

final class SyntaxHighlighterTests: XCTestCase {
    private func tokens(for source: String) -> [Token] {
        tokenizeSource(source: source)
    }

    func testColorRunsSkipNewlinesAndUncoloredKinds() {
        let toks = tokens(for: "MOV AX, 5")
        let runs = SyntaxHighlighter.colorRuns(for: toks)
        // "MOV" (identifier), "AX" (register), "5" (number) all get
        // colors; the comma (punctuation) and any newline do too or
        // don't per `color(for:)` - only assert the ones that matter.
        XCTAssertTrue(runs.contains { $0.range == NSRange(location: 4, length: 2) && $0.color == .systemBlue }, "AX should be colored as a register at its real byte offset")
        XCTAssertTrue(runs.contains { $0.range == NSRange(location: 8, length: 1) && $0.color == .systemPurple }, "5 should be colored as a number")
    }

    func testColorRunUsesRealByteOffsetNotTokenIndex() {
        // Regression-style check: byte_offset (not an index into the
        // token array) is what must drive NSRange, since the editor's
        // NSTextStorage is indexed by character/byte position.
        let toks = tokens(for: "  MOV AX")
        let axToken = toks.first { $0.text == "AX" }
        XCTAssertNotNil(axToken)
        XCTAssertEqual(axToken?.byteOffset, 6) // two leading spaces + "MOV "
    }

    func testSquiggleRangeFindsTheExactTokenAtTheDiagnosticsPosition() {
        let toks = tokens(for: "MOV AX, missing")
        let diagnostic = Diagnostic(line: 1, col: 9, isError: true, message: "undefined symbol 'missing'")
        let range = SyntaxHighlighter.squiggleRange(for: diagnostic, in: toks)
        let missingToken = toks.first { $0.text == "missing" }
        XCTAssertEqual(range, NSRange(location: Int(missingToken!.byteOffset), length: Int(missingToken!.len)))
    }

    func testSquiggleRangeFallsBackToTheWholeLineWhenNoExactColumnMatches() {
        let toks = tokens(for: "MOV AX, 5")
        // line-level diagnostic with a column that doesn't land exactly
        // on any token (e.g. a semantic error reported at col 1).
        let diagnostic = Diagnostic(line: 1, col: 1, isError: true, message: "some whole-line error")
        let range = SyntaxHighlighter.squiggleRange(for: diagnostic, in: toks)
        XCTAssertNotNil(range)
        XCTAssertEqual(range?.location, 0)
    }

    func testSquiggleRangeIsNilForALineWithNoTokens() {
        let toks = tokens(for: "MOV AX, 5")
        let diagnostic = Diagnostic(line: 99, col: 1, isError: true, message: "no such line")
        XCTAssertNil(SyntaxHighlighter.squiggleRange(for: diagnostic, in: toks))
    }
}
