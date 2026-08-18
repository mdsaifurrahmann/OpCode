import SwiftUI

/// A window of words near SP, growing from the current stack pointer
/// upward through higher addresses - "what's currently on the stack,"
/// which is what a debugger's Stack panel is for.
struct StackView: View {
    let entries: [StackEntry]
    /// The physical (flat) address SS:SP resolves to - not the raw SP
    /// value, since SP alone isn't a flat address once SS is nonzero.
    let spPhysicalAddress: UInt32

    var body: some View {
        ScrollView {
            LazyVStack(alignment: .leading, spacing: 1) {
                ForEach(entries) { entry in
                    HStack(spacing: 6) {
                        Text(String(format: "%04X:", entry.address))
                            .foregroundColor(.secondary)
                        Text(String(format: "%04X", entry.value))
                        if entry.address == spPhysicalAddress {
                            Text("← SP")
                                .foregroundColor(.accentColor)
                        }
                    }
                    .font(.system(.caption, design: .monospaced))
                    .accessibilityIdentifier("stackRow_\(entry.address)")
                }
            }
            .padding(4)
        }
        .accessibilityIdentifier("stackView")
    }
}
