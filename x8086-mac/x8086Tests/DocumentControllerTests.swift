import XCTest

@testable import OpCode

@MainActor
final class DocumentControllerTests: XCTestCase {
    /// A fresh, isolated `UserDefaults` suite per test so recent-files
    /// persistence never touches the real app's preferences or leaks
    /// state between tests.
    private func makeController(initialSource: String = "") -> DocumentController {
        let defaults = UserDefaults(suiteName: "DocumentControllerTests.\(UUID().uuidString)")!
        return DocumentController(initialSource: initialSource, userDefaults: defaults)
    }

    private func makeTempURL(named name: String = UUID().uuidString) -> URL {
        FileManager.default.temporaryDirectory.appendingPathComponent("\(name).asm")
    }

    func testOpenURLLoadsFileContentsAndTracksItAsRecent() throws {
        let controller = makeController(initialSource: "old")
        let url = makeTempURL()
        try "MOV AX, 1\nHLT\n".write(to: url, atomically: true, encoding: .utf8)
        defer { try? FileManager.default.removeItem(at: url) }

        controller.openURL(url)

        XCTAssertEqual(controller.sourceCode, "MOV AX, 1\nHLT\n")
        XCTAssertEqual(controller.fileURL, url)
        XCTAssertEqual(controller.recentFiles.first, url)
    }

    func testWriteToURLPersistsCurrentSourceAndUpdatesFileURL() throws {
        let controller = makeController(initialSource: "MOV AX, 1\nHLT\n")
        let url = makeTempURL()
        defer { try? FileManager.default.removeItem(at: url) }

        controller.write(to: url)

        XCTAssertEqual(controller.fileURL, url)
        XCTAssertEqual(try String(contentsOf: url, encoding: .utf8), "MOV AX, 1\nHLT\n")
    }

    func testRecentFilesDeduplicatesAndCapsAtTen() throws {
        let controller = makeController()
        var urls: [URL] = []
        for i in 0..<12 {
            let url = makeTempURL(named: "recent-\(i)")
            try "x".write(to: url, atomically: true, encoding: .utf8)
            urls.append(url)
        }
        defer { urls.forEach { try? FileManager.default.removeItem(at: $0) } }

        for url in urls {
            controller.openURL(url)
        }
        XCTAssertEqual(controller.recentFiles.count, 10, "must cap at 10 entries")
        XCTAssertEqual(controller.recentFiles.first, urls.last, "most recently opened must lead")

        // Re-opening an already-tracked file must move it to the front,
        // not create a duplicate entry.
        let alreadyTracked = urls[urls.count - 3]
        controller.openURL(alreadyTracked)
        XCTAssertEqual(controller.recentFiles.first, alreadyTracked)
        XCTAssertEqual(controller.recentFiles.filter { $0 == alreadyTracked }.count, 1)
    }

    func testNewDocumentClearsFileURLAndChangesDocumentIDButNotOnOrdinaryEdits() {
        let controller = makeController(initialSource: "old")
        let url = makeTempURL()
        defer { try? FileManager.default.removeItem(at: url) }
        controller.write(to: url) // give it a fileURL to clear
        let idBeforeNew = controller.documentID

        controller.sourceCode = "old, edited a bit"
        XCTAssertEqual(controller.documentID, idBeforeNew, "ordinary editing must not change documentID")

        controller.newDocument(defaultSource: "new")
        XCTAssertEqual(controller.sourceCode, "new")
        XCTAssertNil(controller.fileURL)
        XCTAssertNotEqual(controller.documentID, idBeforeNew, "loading a new document must change documentID")
    }
}
