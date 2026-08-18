import SwiftUI

/// Every `DB`/`DW` symbol from the last assemble, with its live value -
/// emu8086's Variables window, driven directly by `Emulator.variables()`
/// rather than a Swift-side re-derivation of the symbol table.
struct VariablesView: View {
    let variables: [VariableValue]

    var body: some View {
        ScrollView {
            LazyVStack(alignment: .leading, spacing: 1) {
                ForEach(variables, id: \.name) { variable in
                    HStack {
                        Text(variable.name).bold()
                        Spacer()
                        Text(DebuggerFormatting.variableValueText(variable.value, isWord: variable.isWord))
                    }
                    .font(.system(.caption, design: .monospaced))
                    .accessibilityIdentifier("variable_\(variable.name)")
                }
            }
            .padding(4)
        }
        .accessibilityIdentifier("variablesView")
    }
}
