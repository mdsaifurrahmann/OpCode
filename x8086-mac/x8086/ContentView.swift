import SwiftUI

struct ContentView: View {
    @EnvironmentObject private var documentController: DocumentController
    @EnvironmentObject private var controller: EmulatorController
    @EnvironmentObject private var undoCoordinator: EditorUndoCoordinator
    @State private var tokens: [Token] = []
    @State private var breakpointLines: Set<Int> = []
    @State private var scrollToLine: Int?
    @State private var selectedDisassemblyAddress: UInt32?

    var body: some View {
        VStack(spacing: 0) {
            DebuggerToolbar(
                controller: controller,
                onRun: {
                    controller.run(source: documentController.sourceCode, breakpointLines: breakpointLines)
                },
                onRestart: {
                    controller.restart(source: documentController.sourceCode, breakpointLines: breakpointLines)
                },
                onRunToCursor: {
                    if let address = selectedDisassemblyAddress {
                        controller.runToCursor(address: address)
                    }
                },
                canRunToCursor: selectedDisassemblyAddress != nil
            )
            Divider()
            HSplitView {
                editorPane
                inspectorPane
            }
            Divider()
            HSplitView {
                outputPane
                MemoryView(controller: controller)
                    .frame(minWidth: 260)
                StackView(entries: controller.stack, spPhysicalAddress: controller.stackPointerPhysicalAddress)
                    .frame(minWidth: 160)
            }
            .frame(height: 220)
        }
        .frame(minWidth: 1150, minHeight: 760)
        .navigationTitle(documentController.displayName)
        .onAppear { tokens = controller.tokenize(documentController.sourceCode) }
        // One-parameter form: the two-parameter onChange(of:) overload
        // needs macOS 14+, and this project targets macOS 13.
        .onChange(of: documentController.sourceCode) { newValue in
            tokens = controller.tokenize(newValue)
        }
        .onChange(of: documentController.documentID) { _ in
            // A whole new document replaced the editor's content - the
            // old breakpoint/scroll state refers to a different file's
            // line numbers and is meaningless here.
            breakpointLines = []
            scrollToLine = nil
        }
    }

    private var editorPane: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text("Source").font(.headline)
            SourceEditorView(
                text: $documentController.sourceCode,
                breakpointLines: $breakpointLines,
                scrollToLine: $scrollToLine,
                tokens: tokens,
                diagnostics: controller.diagnostics,
                currentExecutionLine: controller.currentExecutionLine,
                bindUndoManager: { undoCoordinator.undoManager = $0 }
            )
            .overlay(RoundedRectangle(cornerRadius: 4).stroke(Color.gray.opacity(0.3)))

            if !controller.diagnostics.isEmpty {
                ErrorListView(diagnostics: controller.diagnostics) { diagnostic in
                    scrollToLine = Int(diagnostic.line)
                }
                .frame(maxHeight: 100)
            }
        }
        .padding()
        .frame(minWidth: 380)
    }

    private var inspectorPane: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 12) {
                sectionHeader("Registers")
                RegistersView(registers: controller.registers) { controller.setRegisters($0) }

                sectionHeader("Flags")
                FlagsView(flags: controller.registers.flags)

                if controller.isHalted {
                    Text("Halted").foregroundColor(.green).accessibilityIdentifier("haltedLabel")
                }

                sectionHeader("Disassembly")
                DisassemblyView(
                    lines: controller.disassembly,
                    currentAddress: UInt32(controller.registers.ip),
                    selectedAddress: $selectedDisassemblyAddress
                )
                .frame(height: 220)

                sectionHeader("Variables")
                VariablesView(variables: controller.variables)
                    .frame(height: 100)

                sectionHeader("Watches")
                WatchListView(
                    watches: controller.watches,
                    error: controller.watchError,
                    onAdd: { controller.addWatch($0) },
                    onRemove: { controller.removeWatch(at: $0) }
                )
                .frame(height: 140)
            }
            .padding()
        }
        .frame(minWidth: 320)
    }

    private var outputPane: some View {
        ConsoleView(
            output: controller.consoleOutput,
            isWaitingForKeyboard: controller.isWaitingForKeyboard,
            onKey: controller.sendKey
        )
        .padding()
        .frame(minWidth: 300)
    }

    private func sectionHeader(_ title: String) -> some View {
        Text(title).font(.subheadline).bold()
    }
}

#Preview {
    ContentView()
        .environmentObject(DocumentController(initialSource: Samples.helloWorld.source))
        .environmentObject(EmulatorController())
        .environmentObject(EditorUndoCoordinator())
}
