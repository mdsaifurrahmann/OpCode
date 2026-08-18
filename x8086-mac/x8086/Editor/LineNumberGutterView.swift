import AppKit

protocol LineNumberGutterViewDelegate: AnyObject {
    func lineNumberGutterView(_ gutterView: LineNumberGutterView, didClickLine line: Int)
}

/// A classic Cocoa side-gutter: line numbers, breakpoint dots, and a
/// current-execution-line highlight.
///
/// Built on a plain `NSView`, not `NSRulerView` - `NSRulerView` looked
/// like the purpose-built tool (it scrolls in lockstep with its client
/// view for free), but empirically its `mouseDown` never fires when
/// it's installed as a scroll view's `verticalRulerView` here, likely
/// because Cocoa routes ruler events through its own marker-dragging
/// machinery rather than the plain responder chain. A plain `NSView`
/// receives mouse events in the completely standard, well-documented
/// way; the cost is that it has to be told when to redraw itself as the
/// text view scrolls, since (unlike a ruler) it isn't wired into the
/// scroll view's layout automatically - see `SourceEditorView`, which
/// observes the clip view's bounds and calls `needsDisplay` here.
final class LineNumberGutterView: NSView {
    weak var gutterDelegate: LineNumberGutterViewDelegate?
    weak var textView: NSTextView?

    static let width: CGFloat = 44

    var breakpointLines: Set<Int> = [] {
        didSet {
            needsDisplay = true
            // No sub-element here has an individually clickable/readable
            // accessibility identity (the gutter is hand-drawn) - this
            // string value exists purely so XCUITest can observe that a
            // gutter click actually toggled a breakpoint.
            setAccessibilityValue(breakpointLines.sorted().map(String.init).joined(separator: ","))
        }
    }

    var currentExecutionLine: Int? {
        didSet { needsDisplay = true }
    }

    override var isFlipped: Bool { true }

    override init(frame frameRect: NSRect) {
        super.init(frame: frameRect)
        setAccessibilityElement(true)
        setAccessibilityIdentifier("lineNumberGutter")
        setAccessibilityRole(.group)
        setAccessibilityValue("")
    }

    required init?(coder: NSCoder) {
        super.init(coder: coder)
    }

    override func draw(_ dirtyRect: NSRect) {
        guard let textView, let layoutManager = textView.layoutManager, let textContainer = textView.textContainer else { return }

        // A redraw triggered mid-scroll (see `SourceEditorView`'s bounds-
        // change observer) computes line rects for the *new* scroll
        // position, which can briefly fall just outside `bounds` before
        // the next layout pass catches up. Clipping the graphics context
        // itself (rather than relying on layer masking, which turned out
        // to suppress this view's drawing entirely under SwiftUI's
        // hosting) guarantees nothing painted below ever escapes the
        // gutter's own rectangle, regardless of backing-store details.
        NSGraphicsContext.current?.saveGraphicsState()
        defer { NSGraphicsContext.current?.restoreGraphicsState() }
        NSBezierPath(rect: bounds).addClip()

        NSColor.controlBackgroundColor.setFill()
        dirtyRect.fill()

        let fullGlyphRange = layoutManager.glyphRange(for: textContainer)
        var lineNumber = 1
        layoutManager.enumerateLineFragments(forGlyphRange: fullGlyphRange) { lineRect, _, _, _, _ in
            let lineRectInGutter = self.convert(lineRect, from: textView)
            if lineRectInGutter.intersects(dirtyRect) {
                self.drawLine(number: lineNumber, in: lineRectInGutter)
            }
            lineNumber += 1
        }
    }

    private func drawLine(number: Int, in lineRect: NSRect) {
        if currentExecutionLine == number {
            NSColor.systemYellow.withAlphaComponent(0.3).setFill()
            NSRect(x: 0, y: lineRect.minY, width: Self.width, height: lineRect.height).fill()
        }
        if breakpointLines.contains(number) {
            NSColor.systemRed.setFill()
            let dotSize: CGFloat = 8
            let dotRect = NSRect(x: 4, y: lineRect.minY + (lineRect.height - dotSize) / 2, width: dotSize, height: dotSize)
            NSBezierPath(ovalIn: dotRect).fill()
        }
        let attributes: [NSAttributedString.Key: Any] = [
            .font: NSFont.monospacedDigitSystemFont(ofSize: 10, weight: .regular),
            .foregroundColor: NSColor.secondaryLabelColor,
        ]
        let numberString = "\(number)" as NSString
        let size = numberString.size(withAttributes: attributes)
        let numberRect = NSRect(x: Self.width - size.width - 6, y: lineRect.minY + (lineRect.height - size.height) / 2, width: size.width, height: size.height)
        numberString.draw(in: numberRect, withAttributes: attributes)
    }

    override func mouseDown(with event: NSEvent) {
        guard let textView, let layoutManager = textView.layoutManager, let textContainer = textView.textContainer else { return }
        let clickPoint = convert(event.locationInWindow, from: nil)

        let fullGlyphRange = layoutManager.glyphRange(for: textContainer)
        var lineNumber = 1
        var clickedLine: Int?
        layoutManager.enumerateLineFragments(forGlyphRange: fullGlyphRange) { lineRect, _, _, _, stop in
            let lineRectInGutter = self.convert(lineRect, from: textView)
            if clickPoint.y >= lineRectInGutter.minY, clickPoint.y <= lineRectInGutter.maxY {
                clickedLine = lineNumber
                stop.pointee = true
            }
            lineNumber += 1
        }

        if let clickedLine {
            gutterDelegate?.lineNumberGutterView(self, didClickLine: clickedLine)
        }
    }
}
