import SwiftUI

@main
struct X8086App: App {
    @StateObject private var documentController = DocumentController(initialSource: Samples.helloWorld.source)
    @StateObject private var emulatorController = EmulatorController()
    @Environment(\.openWindow) private var openWindow

    var body: some Scene {
        WindowGroup {
            ContentView()
                .environmentObject(documentController)
                .environmentObject(emulatorController)
        }
        .commands {
            CommandGroup(replacing: .newItem) {
                Button("New") {
                    documentController.newDocument(defaultSource: Samples.helloWorld.source)
                }
                .keyboardShortcut("n", modifiers: .command)

                Button("Open…") {
                    documentController.open()
                }
                .keyboardShortcut("o", modifiers: .command)

                Menu("Open Recent") {
                    if documentController.recentFiles.isEmpty {
                        Text("No Recent Files")
                    } else {
                        ForEach(documentController.recentFiles, id: \.self) { url in
                            Button(url.lastPathComponent) {
                                documentController.openURL(url)
                            }
                        }
                    }
                }

                Menu("Open Sample") {
                    ForEach(Samples.all) { sample in
                        Button(sample.name) {
                            documentController.newDocument(defaultSource: sample.source)
                        }
                        .accessibilityIdentifier("sampleMenuItem_\(sample.name)")
                    }
                }
            }
            CommandGroup(replacing: .saveItem) {
                Button("Save") {
                    documentController.save()
                }
                .keyboardShortcut("s", modifiers: .command)

                Button("Save As…") {
                    documentController.saveAs()
                }
                .keyboardShortcut("s", modifiers: [.command, .shift])

                Divider()

                Button("Export Listing…") {
                    documentController.exportText(currentListing(), suggestedName: "listing.txt")
                }
                .disabled(emulatorController.lineToAddress.isEmpty)
                .accessibilityIdentifier("exportListingMenuItem")
            }
            CommandGroup(replacing: .printItem) {
                Button("Print Listing…") {
                    documentController.printText(currentListing())
                }
                .disabled(emulatorController.lineToAddress.isEmpty)
                .keyboardShortcut("p", modifiers: .command)
            }
            CommandMenu("Tools") {
                Button("Number Converter") { openWindow(id: "numberConverter") }
                Button("ASCII Table") { openWindow(id: "asciiTable") }
            }
        }

        Window("Number Converter", id: "numberConverter") {
            NumberConverterView()
        }
        .defaultSize(width: 340, height: 260)

        Window("ASCII Table", id: "asciiTable") {
            AsciiTableView()
        }
        .defaultSize(width: 420, height: 480)
    }

    /// Builds the current listing text - requires the program to have
    /// been assembled at least once (`lineToAddress` non-empty), which
    /// both Export and Print menu items already gate on via `.disabled`.
    private func currentListing() -> String {
        ListingExporter.generate(
            source: documentController.sourceCode,
            lineToAddress: emulatorController.lineToAddress,
            machineCodeLength: emulatorController.machineCodeLength,
            readMemory: { address, len in emulatorController.readMemory(address: address, len: len) }
        )
    }
}
