import XCTest

@testable import x8086

final class X8086Tests: XCTestCase {
    func testPingReturnsPong() {
        XCTAssertEqual(ping(), "pong")
    }

    func testEmulatorAssemblesAndRunsASimpleProgram() {
        let emulator = Emulator()
        let result = emulator.assembleAndLoad(source: "MOV AX, 5\nADD AX, 3\nHLT\n")
        XCTAssertTrue(result.diagnostics.isEmpty, "unexpected diagnostics: \(result.diagnostics)")

        var outcome: StepResult = .continued
        while case .continued = outcome {
            outcome = emulator.step()
        }
        guard case .halted = outcome else {
            XCTFail("expected the program to halt, got \(outcome)")
            return
        }
        XCTAssertEqual(emulator.registers().ax, 8)
    }

    @MainActor
    func testEmulatorControllerPublishesRegistersAndConsoleOutputAfterRun() {
        let controller = EmulatorController()
        controller.run(source: "MOV DX, 65\nMOV AH, 2\nINT 21h\nHLT\n") // AL/DL=65='A'
        XCTAssertTrue(controller.diagnostics.isEmpty, "unexpected diagnostics: \(controller.diagnostics)")
        XCTAssertTrue(controller.isHalted)
        XCTAssertEqual(controller.consoleOutput, "A")
        XCTAssertEqual(controller.registers.dx, 65)
    }
}
