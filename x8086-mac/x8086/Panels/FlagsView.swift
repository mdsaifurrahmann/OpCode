import SwiftUI

/// Read-only decoded view of the FLAGS register: each named bit as a
/// lit/unlit chip. The Registers panel already shows the raw hex FLAGS
/// value; nobody can read individual condition bits out of that at a
/// glance, which is what this is for.
struct FlagsView: View {
    let flags: UInt16

    /// Bit positions per the 8086 FLAGS register layout (see
    /// `x8086_cpu`'s `flag_bit`).
    private static let bits: [(name: String, bit: UInt16)] = [
        ("CF", 0), ("PF", 2), ("AF", 4), ("ZF", 6),
        ("SF", 7), ("TF", 8), ("IF", 9), ("DF", 10), ("OF", 11),
    ]

    var body: some View {
        LazyVGrid(columns: [GridItem(.adaptive(minimum: 34), spacing: 4)], alignment: .leading, spacing: 4) {
            ForEach(Self.bits, id: \.name) { flag in
                let isSet = (flags >> flag.bit) & 1 == 1
                Text(flag.name)
                    .font(.system(.caption2, design: .monospaced))
                    .padding(.horizontal, 4)
                    .padding(.vertical, 1)
                    .background(isSet ? Color.accentColor.opacity(0.3) : Color.clear)
                    .foregroundColor(isSet ? .primary : .secondary)
                    .overlay(RoundedRectangle(cornerRadius: 3).stroke(Color.gray.opacity(0.3)))
                    .accessibilityIdentifier("flag_\(flag.name)")
                    .accessibilityValue(isSet ? "set" : "clear")
            }
        }
    }
}
