import AppKit

/// Bridges the source editor's `UndoManager` to the app's Edit menu.
///
/// SwiftUI's automatic Undo/Redo commands read the `\.undoManager`
/// environment value, which is a read-only key populated by SwiftUI
/// itself (via `DocumentGroup`/`NSDocument` machinery) - there's no
/// public way to feed it a manager from a plain `WindowGroup` app.
/// Confirmed by instrumentation that neither the classic AppKit
/// responder chain (`NSResponder.validateUserInterfaceItem` is never
/// called for undo:/redo:, only for cut:/copy:/paste:/etc.) nor
/// `NSWindowDelegate.windowWillReturnUndoManager` is consulted either,
/// even with a delegate proxy installed - so this app defines its own
/// Undo/Redo commands (see `x8086App`) instead of relying on the
/// system-provided ones, and drives them from this coordinator.
final class EditorUndoCoordinator: ObservableObject {
    @Published private(set) var canUndo = false
    @Published private(set) var canRedo = false

    var undoManager: UndoManager? {
        didSet {
            observers.forEach(NotificationCenter.default.removeObserver)
            observers = []
            guard let undoManager else {
                canUndo = false
                canRedo = false
                return
            }
            refresh(undoManager)
            let center = NotificationCenter.default
            let names: [Notification.Name] = [
                .NSUndoManagerDidUndoChange,
                .NSUndoManagerDidRedoChange,
                .NSUndoManagerDidCloseUndoGroup,
            ]
            observers = names.map { name in
                center.addObserver(forName: name, object: undoManager, queue: .main) { [weak self] _ in
                    self?.refresh(undoManager)
                }
            }
        }
    }

    private var observers: [NSObjectProtocol] = []

    func undo() { undoManager?.undo() }
    func redo() { undoManager?.redo() }

    private func refresh(_ undoManager: UndoManager) {
        canUndo = undoManager.canUndo
        canRedo = undoManager.canRedo
    }
}
