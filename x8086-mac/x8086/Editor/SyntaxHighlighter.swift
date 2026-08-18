import AppKit

/// Pure token/diagnostic -> range mapping, deliberately free of any
/// AppKit view code so it's testable without a running app. Both
/// `Token` and `Diagnostic` come from `tokenize_source`/
/// `assembleAndLoad`, so highlighting and the assembler can never
/// silently drift apart - there's exactly one lexer.
enum SyntaxHighlighter {
    static func color(for kind: TokenKind) -> NSColor? {
        switch kind {
        case .register: return .systemBlue
        case .number: return .systemPurple
        case .stringLiteral: return .systemRed
        case .comment: return .systemGreen
        case .identifier: return .labelColor
        case .punctuation: return .secondaryLabelColor
        case .newline: return nil
        }
    }

    /// One `NSRange` (UTF-16, matching `NSAttributedString`) per token
    /// that has an associated color - `byteOffset` is used directly as
    /// the range location, which is valid for ASCII source (see the
    /// field's doc comment on the Rust side).
    static func colorRuns(for tokens: [Token]) -> [(range: NSRange, color: NSColor)] {
        tokens.compactMap { token in
            guard let color = color(for: token.kind), token.len > 0 else { return nil }
            return (NSRange(location: Int(token.byteOffset), length: Int(token.len)), color)
        }
    }

    /// The range to underline for one diagnostic: the token at
    /// `diagnostic.line`/`diagnostic.col`, or (if none is found - e.g. a
    /// whole-line error with no specific column) the entire line's
    /// non-newline tokens.
    static func squiggleRange(for diagnostic: Diagnostic, in tokens: [Token]) -> NSRange? {
        if let exact = tokens.first(where: { $0.line == diagnostic.line && $0.col == diagnostic.col && $0.kind != .newline }) {
            return NSRange(location: Int(exact.byteOffset), length: Int(exact.len))
        }
        let lineTokens = tokens.filter { $0.line == diagnostic.line && $0.kind != .newline }
        guard let first = lineTokens.first, let last = lineTokens.last else { return nil }
        let start = Int(first.byteOffset)
        let end = Int(last.byteOffset) + Int(last.len)
        guard end > start else { return nil }
        return NSRange(location: start, length: end - start)
    }
}
