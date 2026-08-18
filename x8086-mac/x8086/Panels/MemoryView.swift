import SwiftUI

/// A jump-to-address hex dump, editable per byte - the Memory panel's
/// live-edit path. Reads are on-demand (`controller.readMemory`) for
/// only the visible rows, never the whole 1MB address space.
struct MemoryView: View {
    @ObservedObject var controller: EmulatorController

    @State private var baseAddressText: String = "0000"
    @State private var baseAddress: UInt32 = 0
    @State private var editingAddress: UInt32?
    @State private var editingText: String = ""
    @FocusState private var focusedAddress: UInt32?

    private let bytesPerRow = 16
    private let rowCount = 6

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            HStack {
                Text("Address").font(.caption).foregroundColor(.secondary)
                TextField("0000", text: $baseAddressText, onCommit: jump)
                    .textFieldStyle(.roundedBorder)
                    .font(.system(.caption, design: .monospaced))
                    .frame(width: 60)
                    .accessibilityIdentifier("memoryAddressField")
            }
            ScrollView {
                VStack(alignment: .leading, spacing: 2) {
                    ForEach(0..<rowCount, id: \.self) { row in
                        memoryRow(startingAt: baseAddress &+ UInt32(row * bytesPerRow))
                    }
                }
            }
        }
        .accessibilityIdentifier("memoryView")
    }

    private func memoryRow(startingAt rowStart: UInt32) -> some View {
        let bytes = [UInt8](controller.readMemory(address: rowStart, len: UInt32(bytesPerRow)))
        return HStack(spacing: 5) {
            Text(String(format: "%04X:", rowStart))
                .foregroundColor(.secondary)
            ForEach(0..<bytes.count, id: \.self) { offset in
                byteCell(address: rowStart &+ UInt32(offset), value: bytes[offset])
            }
        }
        .font(.system(.caption, design: .monospaced))
    }

    @ViewBuilder
    private func byteCell(address: UInt32, value: UInt8) -> some View {
        if editingAddress == address {
            TextField("", text: $editingText, onCommit: { commitEdit(address: address) })
                .textFieldStyle(.plain)
                .frame(width: 18)
                .focused($focusedAddress, equals: address)
                .onAppear { focusedAddress = address }
                .accessibilityIdentifier("memoryByte_\(address)")
        } else {
            Text(String(format: "%02X", value))
                .frame(width: 18)
                .onTapGesture {
                    editingAddress = address
                    editingText = String(format: "%02X", value)
                }
                .accessibilityIdentifier("memoryByte_\(address)")
        }
    }

    private func jump() {
        baseAddress = UInt32(baseAddressText, radix: 16) ?? baseAddress
    }

    private func commitEdit(address: UInt32) {
        if let value = UInt8(editingText, radix: 16) {
            controller.writeMemoryByte(address: address, value: value)
        }
        editingAddress = nil
    }
}
