import XCTest

@testable import OpCode

/// Controller-level correctness for the Phase 6 debugger surface,
/// exercising the real `EmulatorController` (backed by the real Rust
/// core, not a mock) rather than the full UI - the corresponding
/// `x8086UITests` flows verify the same behaviors are actually wired to
/// the toolbar/panels.
@MainActor
final class DebuggerControllerTests: XCTestCase {
    func testStepBackRestoresThePreviousRegisterState() async {
        let controller = EmulatorController()
        controller.run(source: "MOV AX, 1\nMOV AX, 2\nHLT\n", breakpointLines: [])
        await controller.waitUntilIdle()
        XCTAssertTrue(controller.isHalted)
        XCTAssertEqual(controller.registers.ax, 2)

        controller.stepBack() // undoes HLT
        XCTAssertFalse(controller.isHalted)
        XCTAssertEqual(controller.registers.ax, 2)

        controller.stepBack() // undoes the second MOV AX, 2
        XCTAssertEqual(controller.registers.ax, 1)
    }

    func testSetRegistersLiveEditsWhilePausedAndSurvivesTheNextStep() {
        let controller = EmulatorController()
        controller.restart(source: "NOP\nHLT\n", breakpointLines: [])
        XCTAssertFalse(controller.diagnostics.contains { $0.isError })

        var edited = controller.registers
        edited.ax = 0xABCD
        controller.setRegisters(edited)
        XCTAssertEqual(controller.registers.ax, 0xABCD)

        controller.stepInto() // NOP - must not clobber the manual edit
        XCTAssertEqual(controller.registers.ax, 0xABCD)
    }

    func testWriteMemoryByteLiveEditsWhilePaused() {
        let controller = EmulatorController()
        controller.restart(source: "HLT\n", breakpointLines: [])
        XCTAssertFalse(controller.diagnostics.contains { $0.isError })

        controller.writeMemoryByte(address: 0x10, value: 0x99)
        XCTAssertEqual(controller.readMemory(address: 0x10, len: 1), Data([0x99]))
    }

    func testRunToCursorStopsAtTheRequestedLineWithoutRunningPastIt() async {
        let controller = EmulatorController()
        controller.restart(
            source: "MOV AX, 1\nMOV BX, 2\nMOV CX, 3\nHLT\n",
            breakpointLines: []
        )
        XCTAssertFalse(controller.diagnostics.contains { $0.isError })
        guard let target = controller.lineToAddress.first(where: { $0.line == 3 }) else {
            XCTFail("expected an address for line 3 (MOV CX, 3)")
            return
        }

        controller.runToCursor(address: target.address)
        await controller.waitUntilIdle()

        XCTAssertEqual(UInt32(controller.registers.ip), target.address)
        XCTAssertEqual(controller.registers.cx, 0, "must stop before MOV CX,3 runs")
        XCTAssertFalse(controller.isHalted)
    }

    func testStepOverSkipsPastACallRatherThanEnteringIt() async {
        let controller = EmulatorController()
        controller.restart(
            source: "CALL sub\nHLT\nsub:\nMOV AX, 99\nRET\n",
            breakpointLines: []
        )
        XCTAssertFalse(controller.diagnostics.contains { $0.isError })

        controller.stepOver()
        await controller.waitUntilIdle()

        XCTAssertEqual(controller.registers.ax, 99, "the subroutine must have run to completion")
        XCTAssertFalse(controller.isHalted, "must stop at HLT, right after the CALL, not run past it")
    }
}
