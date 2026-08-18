import AppKit
import SwiftUI

/// An invisible, zero-drawing `NSView` that grabs first-responder status
/// and forwards every keystroke to `onKey` - lets the Console panel
/// itself capture a keypress directly (matching real `INT 16h`/`INT 21h`
/// semantics: one keystroke, no Enter needed) instead of routing through
/// a separate, easy-to-miss text field a first-time user has no reason
/// to expect or notice.
private final class KeyCaptureView: NSView {
    var onKey: ((Character) -> Void)?

    override var acceptsFirstResponder: Bool { true }

    override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        // This view only exists while the program is actually waiting on
        // a keystroke (see `ConsoleView`, which only creates it then) -
        // grabbing focus the moment it appears means the user can just
        // start typing, no click required.
        window?.makeFirstResponder(self)
    }

    override func mouseDown(with event: NSEvent) {
        // Reclaims focus if something else (e.g. clicking back into the
        // source editor) stole first-responder status while still
        // waiting on a keystroke.
        window?.makeFirstResponder(self)
    }

    override func keyDown(with event: NSEvent) {
        guard let characters = event.characters, let character = characters.first else {
            super.keyDown(with: event)
            return
        }
        onKey?(character)
    }
}

private struct KeyCaptureRepresentable: NSViewRepresentable {
    let onKey: (Character) -> Void

    func makeNSView(context: Context) -> KeyCaptureView {
        let view = KeyCaptureView()
        view.onKey = onKey
        view.setAccessibilityIdentifier("consoleKeyCapture")
        return view
    }

    func updateNSView(_ nsView: KeyCaptureView, context: Context) {
        nsView.onKey = onKey
        if nsView.window?.firstResponder !== nsView {
            DispatchQueue.main.async { nsView.window?.makeFirstResponder(nsView) }
        }
    }
}

/// The console/output panel: shows everything the running program has
/// printed, and - while it's blocked waiting on a keystroke - becomes
/// the keystroke target itself. There's nowhere else a first-time user
/// needs to find, and no separate field competing for attention.
struct ConsoleView: View {
    let output: String
    let isWaitingForKeyboard: Bool
    let onKey: (Character) -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            Text("Console").font(.headline)
            ZStack(alignment: .topLeading) {
                ScrollView {
                    Text(output)
                        .font(.system(.body, design: .monospaced))
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .padding(6)
                        .accessibilityIdentifier("consoleOutputLabel")
                }
                if isWaitingForKeyboard {
                    KeyCaptureRepresentable(onKey: onKey)
                }
            }
            .overlay(
                RoundedRectangle(cornerRadius: 4)
                    .stroke(
                        isWaitingForKeyboard ? Color.accentColor : Color.gray.opacity(0.3),
                        lineWidth: isWaitingForKeyboard ? 2 : 1
                    )
            )
            .accessibilityIdentifier("consoleView")

            if isWaitingForKeyboard {
                Label("Waiting for a keystroke - click the console and press any key", systemImage: "keyboard")
                    .font(.caption)
                    .foregroundColor(.accentColor)
                    .accessibilityIdentifier("keyboardWaitingLabel")
            }
        }
    }
}
