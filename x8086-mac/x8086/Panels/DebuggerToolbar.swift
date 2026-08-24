import SwiftUI

/// The debugger's control strip: Run/Pause/Restart, Step Into/Over/Back,
/// Run-to-cursor, and a speed slider that controls how visibly slow a
/// `run` animates (see `EmulatorController.currentChunkParameters`).
struct DebuggerToolbar: View {
    @ObservedObject var controller: EmulatorController
    let onRun: () -> Void
    let onRestart: () -> Void
    let onRunToCursor: () -> Void
    let canRunToCursor: Bool

    /// Run to Here has two preconditions - a loaded, stopped program and
    /// a selected Disassembly line - and the second one is invisible:
    /// nothing on screen says a selection is required, so the button
    /// reads as permanently broken (a normal Run ends `halted`, which
    /// also disables it). A single static tooltip can only ever describe
    /// one of those cases, so it names whichever one is actually
    /// blocking right now.
    private var runToHereHelp: String {
        switch controller.executionState {
        case .notLoaded:
            return "Load a program first - click Run or Restart, then pick a line in the Disassembly panel to run to."
        case .running:
            return "Pause first, then pick a line in the Disassembly panel to run to."
        case .waitingForKeyboard:
            return "The program is waiting for a keystroke - type into the Console first."
        case .halted:
            return "The program has finished. Click Restart, then pick a line in the Disassembly panel to run to."
        case .stopped:
            return canRunToCursor
                ? "Run straight to the selected Disassembly line - a one-off breakpoint that isn't saved."
                : "Pick a line in the Disassembly panel first - this then runs straight to it, like a one-off breakpoint that isn't saved."
        }
    }

    var body: some View {
        HStack(spacing: 10) {
            Button(controller.isRunning ? "Running…" : "Run", action: onRun)
                .disabled(!controller.canRun)
                .accessibilityIdentifier("runButton")
                .keyboardShortcut(.return, modifiers: .command)
                .help("Assemble and run from the start, or continue from wherever it's currently paused.")

            Button("Pause", action: controller.pause)
                .disabled(!controller.isRunning)
                .accessibilityIdentifier("pauseButton")
                .help("Pause a running program without losing its state - Run continues right where it left off.")

            Button("Restart", action: onRestart)
                .accessibilityIdentifier("restartButton")
                .help("Reassemble and reload from the start, stopping at the first instruction without running it.")

            Divider().frame(height: 16)

            Button("Step In", action: controller.stepInto)
                .disabled(!controller.canStep)
                .accessibilityIdentifier("stepIntoButton")
                .help("Execute one instruction. If it's a CALL, follow it into the subroutine.")

            Button("Step Over", action: controller.stepOver)
                .disabled(!controller.canStep)
                .accessibilityIdentifier("stepOverButton")
                .help("Execute one instruction. If it's a CALL, run the whole subroutine and stop right after it returns.")

            Button("Undo Step", action: controller.stepBack)
                .disabled(!controller.canStepBack || controller.isRunning)
                .accessibilityIdentifier("stepBackButton")
                .help("Undo the last executed instruction, restoring registers, flags, and memory exactly as they were.")

            Button("Run to Here", action: onRunToCursor)
                .disabled(!canRunToCursor || !controller.canStep)
                .accessibilityIdentifier("runToCursorButton")
                .help(runToHereHelp)

            Divider().frame(height: 16)

            HStack(spacing: 4) {
                Text("Speed")
                    .font(.caption)
                    .foregroundColor(.secondary)
                    .fixedSize()
                Slider(value: $controller.executionSpeed, in: 0...1)
                    .frame(width: 100)
                    .accessibilityIdentifier("speedSlider")
            }
            .help("How fast Run animates: drag left to watch it execute one visible step at a time, right for effectively instant.")

            Spacer(minLength: 0)
        }
        .padding(8)
    }
}
