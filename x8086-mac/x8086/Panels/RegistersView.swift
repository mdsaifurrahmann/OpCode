import SwiftUI

/// The Registers panel, editable in place: every field is a hex text
/// field that commits back through `onCommit` when the user presses
/// Return - the live-edit-while-paused path the debugger needs.
struct RegistersView: View {
    let registers: Registers
    let onCommit: (Registers) -> Void

    private var fields: [(name: String, value: UInt16, keyPath: WritableKeyPath<Registers, UInt16>)] {
        [
            ("AX", registers.ax, \.ax), ("BX", registers.bx, \.bx),
            ("CX", registers.cx, \.cx), ("DX", registers.dx, \.dx),
            ("SI", registers.si, \.si), ("DI", registers.di, \.di),
            ("BP", registers.bp, \.bp), ("SP", registers.sp, \.sp),
            ("CS", registers.cs, \.cs), ("DS", registers.ds, \.ds),
            ("ES", registers.es, \.es), ("SS", registers.ss, \.ss),
            ("IP", registers.ip, \.ip), ("FLAGS", registers.flags, \.flags),
        ]
    }

    var body: some View {
        LazyVGrid(columns: [GridItem(.adaptive(minimum: 96), spacing: 8)], alignment: .leading, spacing: 4) {
            ForEach(fields, id: \.name) { field in
                RegisterField(name: field.name, value: field.value) { newValue in
                    var updated = registers
                    updated[keyPath: field.keyPath] = newValue
                    onCommit(updated)
                }
            }
        }
    }
}

private struct RegisterField: View {
    let name: String
    let value: UInt16
    let onCommit: (UInt16) -> Void

    @State private var text: String = ""
    @FocusState private var isFocused: Bool

    var body: some View {
        HStack(spacing: 2) {
            Text("\(name):")
                .font(.system(.caption, design: .monospaced))
                .foregroundColor(.secondary)
            TextField("", text: $text, onCommit: commit)
                .textFieldStyle(.plain)
                .font(.system(.caption, design: .monospaced))
                .frame(width: 44)
                .focused($isFocused)
                .accessibilityIdentifier("register_\(name)")
        }
        .onAppear { text = String(format: "%04X", value) }
        .onChange(of: value) { newValue in
            // Always reformat, even while focused: the only way `value`
            // changes while this field is focused is the commit below
            // (nothing runs in the background while paused, which is the
            // only time this field is editable at all), so there's no
            // "in-progress edit from the user" to protect here - and
            // relying on this callback rather than reformatting inline
            // in `commit()` matters because Return-to-commit doesn't
            // blur the field, and reassigning `text` while AppKit still
            // owns an active edit session on it doesn't reliably stick.
            text = String(format: "%04X", newValue)
        }
    }

    private func commit() {
        if let parsed = UInt16(text, radix: 16) {
            onCommit(parsed)
        } else {
            text = String(format: "%04X", value)
        }
    }
}
