import SwiftUI

private let defaultSample = """
LEA DX, msg
MOV AH, 9
INT 21h
HLT
msg DB "Hello, x8086!$"
"""

struct ContentView: View {
    @StateObject private var controller = EmulatorController()
    @State private var sourceCode: String = defaultSample
    @State private var keyboardInput: String = ""

    var body: some View {
        HSplitView {
            editorPane
            outputPane
        }
        .frame(minWidth: 720, minHeight: 420)
    }

    private var editorPane: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text("Source").font(.headline)
            TextEditor(text: $sourceCode)
                .font(.system(.body, design: .monospaced))
                .accessibilityIdentifier("sourceEditor")
                .overlay(RoundedRectangle(cornerRadius: 4).stroke(Color.gray.opacity(0.3)))

            Button("Assemble && Run") {
                controller.run(source: sourceCode)
            }
            .accessibilityIdentifier("runButton")
            .keyboardShortcut(.return, modifiers: .command)

            if !controller.diagnostics.isEmpty {
                ScrollView {
                    VStack(alignment: .leading, spacing: 2) {
                        ForEach(Array(controller.diagnostics.enumerated()), id: \.offset) { _, diagnostic in
                            Text("Line \(diagnostic.line): \(diagnostic.message)")
                                .foregroundColor(diagnostic.isError ? .red : .orange)
                                .font(.system(.caption, design: .monospaced))
                        }
                    }
                    .frame(maxWidth: .infinity, alignment: .leading)
                }
                .accessibilityIdentifier("diagnosticsList")
                .frame(maxHeight: 100)
            }
        }
        .padding()
        .frame(minWidth: 340)
    }

    private var outputPane: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text("Console").font(.headline)
            ScrollView {
                Text(controller.consoleOutput)
                    .font(.system(.body, design: .monospaced))
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .accessibilityIdentifier("consoleOutputLabel")
            }
            .frame(minHeight: 120)
            .overlay(RoundedRectangle(cornerRadius: 4).stroke(Color.gray.opacity(0.3)))

            if controller.isWaitingForKeyboard {
                HStack {
                    TextField("keystroke", text: $keyboardInput)
                        .accessibilityIdentifier("keyboardInputField")
                        .frame(width: 80)
                        .onSubmit(sendKeyboardInput)
                    Text("waiting for keyboard input").foregroundColor(.secondary)
                }
            }

            if controller.isHalted {
                Text("Halted").foregroundColor(.green).accessibilityIdentifier("haltedLabel")
            }

            Text("Registers").font(.headline)
            RegistersGrid(registers: controller.registers)
                .accessibilityIdentifier("registersGrid")

            Spacer()
        }
        .padding()
        .frame(minWidth: 320)
    }

    private func sendKeyboardInput() {
        if let character = keyboardInput.first {
            controller.sendKey(character)
        }
        keyboardInput = ""
    }
}

private struct RegistersGrid: View {
    let registers: Registers

    private var pairs: [(String, UInt16)] {
        [
            ("AX", registers.ax), ("BX", registers.bx), ("CX", registers.cx), ("DX", registers.dx),
            ("SI", registers.si), ("DI", registers.di), ("BP", registers.bp), ("SP", registers.sp),
            ("CS", registers.cs), ("DS", registers.ds), ("ES", registers.es), ("SS", registers.ss),
            ("IP", registers.ip), ("FLAGS", registers.flags),
        ]
    }

    var body: some View {
        LazyVGrid(columns: [GridItem(.adaptive(minimum: 90), spacing: 8)], alignment: .leading, spacing: 4) {
            ForEach(pairs, id: \.0) { name, value in
                Text("\(name): \(String(format: "%04X", value))")
                    .font(.system(.caption, design: .monospaced))
            }
        }
    }
}

#Preview {
    ContentView()
}
