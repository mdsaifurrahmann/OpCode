import SwiftUI

/// A forward-only disassembly window anchored at the current IP (see
/// `Emulator.disassemble`'s doc comment for why forward-only). The
/// current instruction is highlighted; clicking any row selects it as
/// the Run-to-cursor target.
struct DisassemblyView: View {
    let lines: [DisassembledLine]
    let currentAddress: UInt32
    @Binding var selectedAddress: UInt32?

    var body: some View {
        ScrollView {
            LazyVStack(alignment: .leading, spacing: 0) {
                ForEach(lines, id: \.address) { line in
                    Button {
                        selectedAddress = line.address
                    } label: {
                        HStack(spacing: 8) {
                            Text(String(format: "%04X", line.address))
                                .foregroundColor(.secondary)
                            Text(line.text)
                        }
                        .font(.system(.caption, design: .monospaced))
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .padding(.horizontal, 4)
                        .padding(.vertical, 1)
                        .background(backgroundColor(for: line.address))
                    }
                    .buttonStyle(.plain)
                    .accessibilityIdentifier("disasmLine_\(line.address)")
                }
            }
        }
        .accessibilityIdentifier("disassemblyView")
    }

    private func backgroundColor(for address: UInt32) -> Color {
        if address == currentAddress {
            return Color.yellow.opacity(0.3)
        }
        if address == selectedAddress {
            return Color.accentColor.opacity(0.2)
        }
        return .clear
    }
}
