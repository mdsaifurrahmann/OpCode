import SwiftUI

/// User-typed watch expressions (register/flag names, `byte`/`word
/// [addr]`, or a variable name) with live values, refreshed on every
/// snapshot by `EmulatorController`.
struct WatchListView: View {
    let watches: [WatchValue]
    let error: String?
    let onAdd: (String) -> Void
    let onRemove: (Int) -> Void

    @State private var newExpression: String = ""

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            HStack {
                TextField("Add watch (AX, ZF, a variable name...)", text: $newExpression, onCommit: submit)
                    .textFieldStyle(.roundedBorder)
                    .font(.caption)
                    .accessibilityIdentifier("watchExpressionField")
                Button("Add", action: submit)
                    .accessibilityIdentifier("addWatchButton")
            }
            if let error {
                Text(error)
                    .font(.caption2)
                    .foregroundColor(.red)
                    .accessibilityIdentifier("watchErrorLabel")
            }
            ScrollView {
                LazyVStack(alignment: .leading, spacing: 1) {
                    ForEach(Array(watches.enumerated()), id: \.offset) { index, watch in
                        HStack {
                            Text(watch.expression).bold()
                            Spacer()
                            Text(DebuggerFormatting.watchValueText(watch.value))
                            Button {
                                onRemove(index)
                            } label: {
                                Image(systemName: "xmark.circle")
                            }
                            .buttonStyle(.plain)
                            .accessibilityIdentifier("removeWatch_\(index)")
                        }
                        .font(.system(.caption, design: .monospaced))
                        .accessibilityIdentifier("watchRow_\(watch.expression)")
                    }
                }
            }
        }
        .accessibilityIdentifier("watchListView")
    }

    private func submit() {
        let trimmed = newExpression.trimmingCharacters(in: .whitespaces)
        guard !trimmed.isEmpty else { return }
        onAdd(trimmed)
        newExpression = ""
    }
}
