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

            Button("Pause", action: controller.pause)
                .disabled(!controller.isRunning)
                .accessibilityIdentifier("pauseButton")

            Button("Restart", action: onRestart)
                .accessibilityIdentifier("restartButton")

            Divider().frame(height: 16)

            Button("Step Into", action: controller.stepInto)
                .disabled(!controller.canStep)
                .accessibilityIdentifier("stepIntoButton")

            Button("Step Over", action: controller.stepOver)
                .disabled(!controller.canStep)
                .accessibilityIdentifier("stepOverButton")

            Button("Step Back", action: controller.stepBack)
                .disabled(!controller.canStepBack || controller.isRunning)
                .accessibilityIdentifier("stepBackButton")

            Button("Run to Cursor", action: onRunToCursor)
                .disabled(!canRunToCursor || !controller.canStep)
                .accessibilityIdentifier("runToCursorButton")

            Divider().frame(height: 16)

            HStack(spacing: 4) {
                Text("Speed").font(.caption).foregroundColor(.secondary)
                Slider(value: $controller.executionSpeed, in: 0...1)
                    .frame(width: 100)
                    .accessibilityIdentifier("speedSlider")
            }

            Spacer()
        }
        .padding(8)
    }
}
