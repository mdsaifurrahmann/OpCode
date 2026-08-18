import SwiftUI

/// A clickable list of assemble-time diagnostics; selecting one jumps
/// the editor to that line via `onSelect`.
struct ErrorListView: View {
    let diagnostics: [Diagnostic]
    let onSelect: (Diagnostic) -> Void

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 2) {
                ForEach(Array(diagnostics.enumerated()), id: \.offset) { _, diagnostic in
                    Button {
                        onSelect(diagnostic)
                    } label: {
                        Text("Line \(diagnostic.line): \(diagnostic.message)")
                            .foregroundColor(diagnostic.isError ? .red : .orange)
                            .font(.system(.caption, design: .monospaced))
                            .frame(maxWidth: .infinity, alignment: .leading)
                    }
                    .buttonStyle(.plain)
                    .accessibilityIdentifier("diagnosticRow_\(diagnostic.line)")
                }
            }
            .padding(4)
        }
        .accessibilityIdentifier("diagnosticsList")
    }
}
