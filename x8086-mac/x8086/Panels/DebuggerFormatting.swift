import Foundation

/// Pure formatting rules for the Watch/Variables panels, split out from
/// the views themselves so they're unit-testable without a SwiftUI
/// rendering harness.
enum DebuggerFormatting {
    /// A variable's value, sized per its declared width - `DB` symbols
    /// print as 2 hex digits, `DW` symbols as 4.
    static func variableValueText(_ value: UInt16, isWord: Bool) -> String {
        String(format: isWord ? "%04X" : "%02X", value)
    }

    /// A watch's value, or `?` when the expression currently doesn't
    /// resolve to anything (see `WatchValue.value`'s doc comment).
    static func watchValueText(_ value: UInt16?) -> String {
        guard let value else { return "?" }
        return String(format: "%04X", value)
    }
}
