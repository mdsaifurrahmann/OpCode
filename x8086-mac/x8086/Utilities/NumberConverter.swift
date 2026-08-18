import Foundation

/// Pure number-base conversion logic for the Number Converter window,
/// split out from the view so it's unit-testable without a SwiftUI
/// rendering harness. Accepts the same suffix convention the assembler
/// itself does (`h` hex, `b` binary, `o`/`q` octal, bare decimal) - see
/// `x8086_assembler`'s `numbers.rs` - so a value typed here reads the
/// same way it would in source.
enum NumberConverter {
    static func parse(_ text: String) -> UInt32? {
        let trimmed = text.trimmingCharacters(in: .whitespaces)
        guard !trimmed.isEmpty else { return nil }
        let upper = trimmed.uppercased()

        if let hex = stripSuffix(upper, "H") {
            return UInt32(hex, radix: 16)
        }
        if let binary = stripSuffix(upper, "B") {
            return UInt32(binary, radix: 2)
        }
        if let octal = stripSuffix(upper, "O") ?? stripSuffix(upper, "Q") {
            return UInt32(octal, radix: 8)
        }
        return UInt32(upper, radix: 10)
    }

    private static func stripSuffix(_ text: String, _ suffix: String) -> Substring? {
        guard text.hasSuffix(suffix), text.count > suffix.count else { return nil }
        return text.dropLast(suffix.count)
    }

    static func decimalText(_ value: UInt32) -> String { String(value) }
    static func hexText(_ value: UInt32) -> String { String(value, radix: 16, uppercase: true) }
    static func octalText(_ value: UInt32) -> String { String(value, radix: 8) }
    static func binaryText(_ value: UInt32) -> String { String(value, radix: 2) }
}
