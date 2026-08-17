import Foundation

/// The only file that touches the raw generated uniffi types directly -
/// everything else in the app works with plain Swift state published
/// here. Keeping the generated-binding surface contained to one file
/// limits blast radius if that generated API shape changes later.
@MainActor
final class EmulatorController: ObservableObject {
    private let emulator: Emulator

    @Published private(set) var consoleOutput: String = ""
    @Published private(set) var registers: Registers
    @Published private(set) var diagnostics: [Diagnostic] = []
    @Published private(set) var isHalted: Bool = false
    @Published private(set) var isWaitingForKeyboard: Bool = false

    /// Upper bound on how many instructions a single `run()` call
    /// executes before yielding back to the UI - a real runaway program
    /// (or a decoder/emulator bug) must not hang the app.
    private let maxStepsPerRun = 200_000

    init() {
        let emulator = Emulator()
        self.emulator = emulator
        self.registers = emulator.registers()
    }

    /// Assembles `source` and runs it to completion (halt, a decode
    /// error, or a blocking keyboard read), publishing the resulting
    /// console output, registers, and diagnostics.
    func run(source: String) {
        consoleOutput = ""
        diagnostics = []
        isHalted = false
        isWaitingForKeyboard = false

        let result = emulator.assembleAndLoad(source: source)
        diagnostics = result.diagnostics
        guard !diagnostics.contains(where: { $0.isError }) else {
            return
        }

        stepUntilStoppedOrWaiting()
    }

    /// Supplies one keystroke to a program blocked on keyboard input via
    /// `isWaitingForKeyboard`, then resumes it.
    func sendKey(_ character: Character) {
        guard isWaitingForKeyboard, let ascii = character.asciiValue else { return }
        emulator.feedKey(scancode: 0, ascii: ascii)
        isWaitingForKeyboard = false
        stepUntilStoppedOrWaiting()
    }

    private func stepUntilStoppedOrWaiting() {
        var stepsRemaining = maxStepsPerRun
        while stepsRemaining > 0 {
            stepsRemaining -= 1
            switch emulator.step() {
            case .continued:
                continue
            case .halted:
                isHalted = true
                refreshSnapshot()
                return
            case .waitingForKeyboard:
                isWaitingForKeyboard = true
                refreshSnapshot()
                return
            case .decodeError(let message):
                diagnostics.append(Diagnostic(line: 0, col: 0, isError: true, message: "decode error: \(message)"))
                refreshSnapshot()
                return
            }
        }
        // Hit the runaway-program ceiling without halting or waiting -
        // stop here and surface the state as-is rather than hanging.
        refreshSnapshot()
    }

    private func refreshSnapshot() {
        consoleOutput = emulator.consoleOutput()
        registers = emulator.registers()
    }
}
