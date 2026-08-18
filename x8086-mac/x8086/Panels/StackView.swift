import SwiftUI

/// A window of words near SP, growing from the current stack pointer
/// upward through higher addresses - "what's currently on the stack,"
/// which is what a debugger's Stack panel is for.
struct StackView: View {
    let entries: [StackEntry]
    let stackPointer: UInt16

    var body: some View {
        ScrollView {
            LazyVStack(alignment: .leading, spacing: 1) {
                ForEach(entries) { entry in
                    HStack(spacing: 6) {
                        Text(String(format: "%04X:", entry.address))
                            .foregroundColor(.secondary)
                        Text(String(format: "%04X", entry.value))
                        if entry.address == UInt32(stackPointer) {
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
