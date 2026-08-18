import AppKit
import SwiftUI

/// An `NSTextView` with its own dedicated `UndoManager`, since a bare
/// code-created `NSTextView` has neither `allowsUndo` nor a usable
/// `undoManager` by default. This is necessary but not sufficient for
/// Undo/Redo to work end to end - see `EditorUndoCoordinator` for why the
/// Edit menu needs its own explicit commands rather than the automatic
/// ones, and `bindUndoManager` below for how this manager reaches them.
private final class UndoableTextView: NSTextView {
    private let editorUndoManager = UndoManager()
    override var undoManager: UndoManager? { editorUndoManager }
}

/// A code-aware editor panel: an `NSTextView` with tokenizer-driven
/// syntax highlighting, inline diagnostic squiggles, and a line-number/
/// breakpoint gutter - built directly on AppKit rather than a
/// third-party text-editor package (see the Editor module's design
/// notes) so every piece of behavior here is on APIs with complete,
/// verified documentation.
struct SourceEditorView: NSViewRepresentable {
    @Binding var text: String
    @Binding var breakpointLines: Set<Int>
    /// Set by the caller to request a scroll-and-select; cleared once
    /// handled, so it acts as a one-shot command rather than persistent
    /// state.
    @Binding var scrollToLine: Int?
    var tokens: [Token]
    var diagnostics: [Diagnostic]
    var currentExecutionLine: Int?
    /// Called once, right after the text view is created, with its
    /// `UndoManager` - the caller hands this to `EditorUndoCoordinator`,
    /// which the app's explicit Undo/Redo commands act on.
    var bindUndoManager: (UndoManager?) -> Void = { _ in }

    func makeNSView(context: Context) -> NSView {
        let textView = UndoableTextView()
        textView.isRichText = false
        textView.isEditable = true
        // NSTextView instances created in code (as opposed to loaded from
        // a NIB, where Interface Builder sets this to true) default to
        // `false` here - without it, the view never calls into
        // `undoManager` to register typing edits in the first place, so
        // the dedicated `UndoManager` above sits permanently empty no
        // matter how it's vended.
        textView.allowsUndo = true
        textView.isAutomaticQuoteSubstitutionEnabled = false
        textView.isAutomaticDashSubstitutionEnabled = false
        textView.isAutomaticSpellingCorrectionEnabled = false
        textView.isAutomaticTextReplacementEnabled = false
        textView.font = .monospacedSystemFont(ofSize: 13, weight: .regular)
        textView.string = text
        textView.delegate = context.coordinator
        textView.textContainerInset = NSSize(width: 4, height: 4)
        textView.isVerticallyResizable = true
        textView.isHorizontallyResizable = false
        textView.autoresizingMask = [.width]
        textView.textContainer?.widthTracksTextView = true
        textView.setAccessibilityIdentifier("sourceEditorTextView")

        let scrollView = NSScrollView()
        scrollView.documentView = textView
        scrollView.hasVerticalScroller = true
        scrollView.translatesAutoresizingMaskIntoConstraints = false

        let gutter = LineNumberGutterView(frame: .zero)
        gutter.textView = textView
        gutter.gutterDelegate = context.coordinator
        gutter.translatesAutoresizingMaskIntoConstraints = false

        let container = NSView()
        container.addSubview(gutter)
        container.addSubview(scrollView)
        NSLayoutConstraint.activate([
            gutter.leadingAnchor.constraint(equalTo: container.leadingAnchor),
            gutter.topAnchor.constraint(equalTo: container.topAnchor),
            gutter.bottomAnchor.constraint(equalTo: container.bottomAnchor),
            gutter.widthAnchor.constraint(equalToConstant: LineNumberGutterView.width),
            scrollView.leadingAnchor.constraint(equalTo: gutter.trailingAnchor),
            scrollView.topAnchor.constraint(equalTo: container.topAnchor),
            scrollView.bottomAnchor.constraint(equalTo: container.bottomAnchor),
            scrollView.trailingAnchor.constraint(equalTo: container.trailingAnchor),
        ])

        context.coordinator.textView = textView
        context.coordinator.gutterView = gutter
        context.coordinator.applyHighlighting(tokens: tokens)

        // Deferred: this runs during SwiftUI's view-building phase, and
        // `bindUndoManager` typically writes to `@Published`/`@State` on
        // the caller's side - mutating that synchronously from inside
        // `makeNSView` triggers "Modifying state during view update".
        DispatchQueue.main.async { bindUndoManager(textView.undoManager) }

        // The gutter is a plain NSView, not an NSRulerView, so nothing
        // repositions or redraws it automatically as the text view
        // scrolls or its content reflows - these two notifications cover
        // both cases.
        scrollView.contentView.postsBoundsChangedNotifications = true
        NotificationCenter.default.addObserver(context.coordinator, selector: #selector(Coordinator.redrawGutter), name: NSView.boundsDidChangeNotification, object: scrollView.contentView)
        NotificationCenter.default.addObserver(context.coordinator, selector: #selector(Coordinator.redrawGutter), name: NSView.frameDidChangeNotification, object: textView)

        return container
    }

    func updateNSView(_ nsView: NSView, context: Context) {
        context.coordinator.parent = self
        guard let textView = context.coordinator.textView else { return }

        if textView.string != text {
            textView.string = text
            // This branch only runs when *something other than the user
            // typing* replaced the whole document (New/Open/Open Recent/
            // Open Sample - ordinary edits already match `text` by the
            // time this runs, via `textDidChange` updating the binding
            // first) - undo history from the previous file has no
            // business surviving into a different one.
            textView.undoManager?.removeAllActions()
        }
        context.coordinator.applyHighlighting(tokens: tokens)
        context.coordinator.applySquiggles(diagnostics: diagnostics, tokens: tokens)
        context.coordinator.gutterView?.breakpointLines = breakpointLines
        context.coordinator.gutterView?.currentExecutionLine = currentExecutionLine
        context.coordinator.gutterView?.needsDisplay = true

        if let line = scrollToLine {
            context.coordinator.selectAndReveal(line: line)
            DispatchQueue.main.async { scrollToLine = nil }
        }
    }

    func makeCoordinator() -> Coordinator {
        Coordinator(self)
    }

    final class Coordinator: NSObject, NSTextViewDelegate, LineNumberGutterViewDelegate {
        var parent: SourceEditorView
        weak var textView: NSTextView?
        weak var gutterView: LineNumberGutterView?

        init(_ parent: SourceEditorView) {
            self.parent = parent
        }

        deinit {
            NotificationCenter.default.removeObserver(self)
        }

        @objc func redrawGutter() {
            gutterView?.needsDisplay = true
        }

        func textDidChange(_ notification: Notification) {
            guard let textView = notification.object as? NSTextView else { return }
            parent.text = textView.string
            gutterView?.needsDisplay = true
        }

        func applyHighlighting(tokens: [Token]) {
            guard let textStorage = textView?.textStorage else { return }
            let fullRange = NSRange(location: 0, length: textStorage.length)
            textStorage.beginEditing()
            textStorage.addAttribute(.foregroundColor, value: NSColor.labelColor, range: fullRange)
            textStorage.addAttribute(.font, value: NSFont.monospacedSystemFont(ofSize: 13, weight: .regular), range: fullRange)
            for run in SyntaxHighlighter.colorRuns(for: tokens) where run.range.location + run.range.length <= textStorage.length {
                textStorage.addAttribute(.foregroundColor, value: run.color, range: run.range)
            }
            textStorage.endEditing()
        }

        func applySquiggles(diagnostics: [Diagnostic], tokens: [Token]) {
            guard let textStorage = textView?.textStorage else { return }
            let fullRange = NSRange(location: 0, length: textStorage.length)
            textStorage.removeAttribute(.underlineStyle, range: fullRange)
            textStorage.removeAttribute(.underlineColor, range: fullRange)
            for diagnostic in diagnostics {
                guard let range = SyntaxHighlighter.squiggleRange(for: diagnostic, in: tokens), range.location + range.length <= textStorage.length else { continue }
                textStorage.addAttribute(.underlineStyle, value: NSUnderlineStyle.thick.rawValue, range: range)
                textStorage.addAttribute(.underlineColor, value: diagnostic.isError ? NSColor.systemRed : NSColor.systemOrange, range: range)
            }
        }

        func selectAndReveal(line: Int) {
            guard let textView, let layoutManager = textView.layoutManager, let textContainer = textView.textContainer else { return }
            let fullGlyphRange = layoutManager.glyphRange(for: textContainer)
            var lineNumber = 1
            var targetRange: NSRange?
            layoutManager.enumerateLineFragments(forGlyphRange: fullGlyphRange) { _, _, _, glyphRange, stop in
                if lineNumber == line {
                    targetRange = layoutManager.characterRange(forGlyphRange: glyphRange, actualGlyphRange: nil)
                    stop.pointee = true
                }
                lineNumber += 1
            }
            guard let range = targetRange else { return }
            textView.setSelectedRange(range)
            textView.scrollRangeToVisible(range)
            textView.window?.makeFirstResponder(textView)
        }

        func lineNumberGutterView(_ gutterView: LineNumberGutterView, didClickLine line: Int) {
            if parent.breakpointLines.contains(line) {
                parent.breakpointLines.remove(line)
            } else {
                parent.breakpointLines.insert(line)
            }
        }
    }
}
