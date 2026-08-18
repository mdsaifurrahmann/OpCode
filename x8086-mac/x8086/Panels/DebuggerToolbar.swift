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
                .help("Select a line in the Disassembly panel first, then use this to run straight to it - a one-off breakpoint that isn't saved.")

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
