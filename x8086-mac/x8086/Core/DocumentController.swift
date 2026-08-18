import AppKit
import Foundation
import UniformTypeIdentifiers

/// Owns the currently open document's source text and file location, and
/// the small persistent "recent files" list - the app's one and only
/// piece of file-management state, shared between the menu bar's
/// commands (which don't otherwise have anywhere to route a "Save"
/// keystroke) and the editor.
@MainActor
final class DocumentController: ObservableObject {
    @Published var sourceCode: String
    @Published private(set) var fileURL: URL?
    @Published private(set) var recentFiles: [URL] = []
    /// Changes only when a whole new document replaces the editor's
    /// content (New/Open/Open Recent/Open Sample) - never on ordinary
    /// editing or Save. The editor's breakpoint set and scroll position
    /// are keyed to *this*, not to `sourceCode` itself, since they need
    /// to reset on a document swap but survive every single keystroke.
    @Published private(set) var documentID = UUID()

    private static let recentFilesDefaultsKey = "recentFiles"
    private static let maxRecentFiles = 10
    private let defaults: UserDefaults

    /// `userDefaults` defaults to `.standard` for real app use; tests
    /// inject an isolated suite so they don't read or write the user's
    /// actual recent-files preferences (or interfere with each other by
    /// sharing that same persistent key).
    init(initialSource: String, userDefaults: UserDefaults = .standard) {
        self.sourceCode = initialSource
        self.defaults = userDefaults
        loadRecentFiles()
    }

    var displayName: String {
        fileURL?.lastPathComponent ?? "Untitled"
    }

    func newDocument(defaultSource: String) {
        sourceCode = defaultSource
        fileURL = nil
        documentID = UUID()
    }

    func open() {
        let panel = NSOpenPanel()
        panel.allowedContentTypes = [.assemblySource, .plainText]
        panel.allowsMultipleSelection = false
        panel.canChooseDirectories = false
        guard panel.runModal() == .OK, let url = panel.url else { return }
        openURL(url)
    }

    /// Loads `url`'s contents into the editor. Split out from `open()`
    /// so both the file picker and "Open Recent" can share it, and so
    /// tests can exercise real file I/O without driving `NSOpenPanel`.
    func openURL(_ url: URL) {
        guard let text = try? String(contentsOf: url, encoding: .utf8) else { return }
        sourceCode = text
        fileURL = url
        documentID = UUID()
        addRecentFile(url)
    }

    func save() {
        if let url = fileURL {
            write(to: url)
        } else {
            saveAs()
        }
    }

    func saveAs() {
        let panel = NSSavePanel()
        panel.allowedContentTypes = [.assemblySource]
        panel.nameFieldStringValue = fileURL?.lastPathComponent ?? "Untitled.asm"
        guard panel.runModal() == .OK, let url = panel.url else { return }
        write(to: url)
    }

    /// Writes arbitrary text (not `sourceCode`) to a user-chosen
    /// location - the Export Listing command's save path, kept here
    /// since this is where file-panel logic already lives.
    func exportText(_ text: String, suggestedName: String) {
        let panel = NSSavePanel()
        panel.allowedContentTypes = [.plainText]
        panel.nameFieldStringValue = suggestedName
        guard panel.runModal() == .OK, let url = panel.url else { return }
        try? text.write(to: url, atomically: true, encoding: .utf8)
    }

    /// Prints arbitrary text via the standard macOS print sheet - the
    /// Print Listing command's path.
    func printText(_ text: String) {
        let textView = NSTextView(frame: NSRect(x: 0, y: 0, width: 612, height: 792))
        textView.string = text
        textView.font = .monospacedSystemFont(ofSize: 10, weight: .regular)
        NSPrintOperation(view: textView).run()
    }

    /// Writes `sourceCode` to `url`. Exposed (not private) so tests can
    /// verify the write/recent-files-tracking behavior directly, without
    /// driving the real `NSSavePanel`.
    func write(to url: URL) {
        guard (try? sourceCode.write(to: url, atomically: true, encoding: .utf8)) != nil else {
            return
        }
        fileURL = url
        addRecentFile(url)
    }

    private func addRecentFile(_ url: URL) {
        recentFiles.removeAll { $0 == url }
        recentFiles.insert(url, at: 0)
        if recentFiles.count > Self.maxRecentFiles {
            recentFiles.removeLast(recentFiles.count - Self.maxRecentFiles)
        }
        saveRecentFiles()
    }

    private func loadRecentFiles() {
        let paths = defaults.stringArray(forKey: Self.recentFilesDefaultsKey) ?? []
        recentFiles = paths.map { URL(fileURLWithPath: $0) }
    }

    private func saveRecentFiles() {
        defaults.set(recentFiles.map(\.path), forKey: Self.recentFilesDefaultsKey)
    }
}

extension UTType {
    /// `.asm` has no built-in system UTType, so this declares one purely
    /// for the open/save panels' file filtering - it's conformance-only,
    /// not registered in Info.plist, since the app doesn't need to be
    /// the default handler for `.asm` files, just able to filter for them.
    static let assemblySource = UTType(filenameExtension: "asm") ?? .plainText
}
