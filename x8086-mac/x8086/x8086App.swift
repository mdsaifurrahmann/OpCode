import AppKit
import SwiftUI

@main
struct X8086App: App {
    @StateObject private var documentController = DocumentController(initialSource: Samples.helloWorld.source)
    @StateObject private var emulatorController = EmulatorController()
    @StateObject private var editorUndoCoordinator = EditorUndoCoordinator()
    @Environment(\.openWindow) private var openWindow

    init() {
        // macOS's "add period with double-space" substitution isn't
        // gated by any of NSTextView's own isAutomatic*Enabled flags
        // (SourceEditorView already disables every one of those) - it's
        // a separate, lower-level behavior keyed off this user default.
        // Without this, assembly source using multiple spaces for
        // column alignment (extremely common emu8086/NASM style,
        // including in real tutorial code) gets silently corrupted as
        // it's typed: "msg  db" becomes "msg. db".
        //
        // `set(_:forKey:)`, not `register(defaults:)`: this app's own
        // preferences domain outranks `NSGlobalDomain` in the search
        // order, while `register(defaults:)` only installs a lowest-
        // priority fallback - which a user who has actually turned this
        // feature on system-wide (i.e. exactly the state that triggers
        // this bug) stores in `NSGlobalDomain`, outranking a mere
        // fallback and leaving it in effect. `set` actually overrides it.
        UserDefaults.standard.set(false, forKey: "NSAutomaticPeriodSubstitutionEnabled")
    }

    var body: some Scene {
        WindowGroup {
            ContentView()
                .environmentObject(documentController)
                .environmentObject(emulatorController)
                .environmentObject(editorUndoCoordinator)
                // Fires when the app is asked to open a file - double-
                // clicking a .asm file in Finder, dragging one onto the
                // Dock icon, or "Open With" (all now possible thanks to
                // the CFBundleDocumentTypes registration in project.yml).
                // Reuses the exact same load path as File > Open.
                .onOpenURL { url in
                    documentController.openURL(url)
                    closeDuplicateMainWindows()
                }
        }
        .commands {
            // Replaces the system-automatic Undo/Redo commands, which
            // read a `\.undoManager` environment value this app (a plain
            // `WindowGroup`, not `DocumentGroup`) has no supported way to
            // populate - see `EditorUndoCoordinator` for the full story
            // on why they'd otherwise stay permanently disabled.
            CommandGroup(replacing: .undoRedo) {
                Button("Undo") {
                    editorUndoCoordinator.undo()
                }
                .keyboardShortcut("z", modifiers: .command)
                .disabled(!editorUndoCoordinator.canUndo)

                Button("Redo") {
                    editorUndoCoordinator.redo()
                }
                .keyboardShortcut("z", modifiers: [.command, .shift])
                .disabled(!editorUndoCoordinator.canRedo)
            }
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

    /// `documentController` is one shared object for the whole app - a
    /// second main window is never showing a different document, just
    /// the identical state a moment later, which makes it pure clutter,
    /// not a real second document. But the system's file-open event (the
    /// path `.onOpenURL` runs on) hands a `WindowGroup` app a *new*
    /// window before this code ever runs, since `WindowGroup` supports
    /// multiple windows by default. This closes every other main editor
    /// window after a file loads, leaving only the one that just
    /// received it - a plain-size check (`contentMinSize.width >= 1000`)
    /// distinguishes it from the much smaller Number Converter/ASCII
    /// Table utility windows, which are meant to coexist and multiply.
    private func closeDuplicateMainWindows() {
        DispatchQueue.main.async {
            let mainWindows = NSApp.windows.filter { $0.contentMinSize.width >= 1000 }
            guard mainWindows.count > 1, let keep = NSApp.keyWindow ?? mainWindows.last else { return }
            for window in mainWindows where window !== keep {
                window.close()
            }
        }
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
