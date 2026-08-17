//! Golden-program integration tests: hand-assembled byte programs run
//! through the real decode -> execute pipeline, asserting final
//! register/flag/memory state. This is the primary regression net for
//! "does the whole pipeline behave correctly together," independent of
//! any single crate's unit tests.

use x8086_emulator::{Emulator, StepOutcome};
use x8086_isa::Flag;

/// Sums 1..=5 in a loop, doubles the result via a CALLed subroutine,
/// takes a conditional branch, stores the result to memory, and proves
/// the stack is left balanced after a CALL/RET and a PUSH/POP pair.
///
/// ```text
///         MOV CX, 5
///         MOV AX, 0
///         MOV BX, 1
/// SUM_LOOP:
///         ADD AX, BX
///         INC BX
///         LOOP SUM_LOOP        ; AX = 1+2+3+4+5 = 15
///         CALL DOUBLE          ; AX = 30
///         CMP AX, 30
///         JE SKIP
///         MOV DX, 0xDEAD       ; must be skipped
/// SKIP:
///         MOV DX, 0xBEEF
///         MOV [0x0050], AX
///         PUSH AX
///         POP CX
///         HLT
/// DOUBLE:
///         ADD AX, AX
///         RET
/// ```
const ARITHMETIC_AND_CONTROL_FLOW_PROGRAM: &[u8] = &[
    0xB9, 0x05, 0x00, // MOV CX, 5
    0xB8, 0x00, 0x00, // MOV AX, 0
    0xBB, 0x01, 0x00, // MOV BX, 1
    0x01, 0xD8, // SUM_LOOP: ADD AX, BX
    0x43, // INC BX
    0xE2, 0xFB, // LOOP SUM_LOOP (rel8 = -5)
    0xE8, 0x11, 0x00, // CALL DOUBLE (rel16 = 17)
    0x3D, 0x1E, 0x00, // CMP AX, 30
    0x74, 0x03, // JE SKIP (rel8 = 3)
    0xBA, 0xAD, 0xDE, // MOV DX, 0xDEAD  (must be skipped)
    0xBA, 0xEF, 0xBE, // SKIP: MOV DX, 0xBEEF
    0xA3, 0x50, 0x00, // MOV [0x0050], AX
    0x50, // PUSH AX
    0x59, // POP CX
    0xF4, // HLT
    0x01, 0xC0, // DOUBLE: ADD AX, AX
    0xC3, // RET
];

#[test]
fn arithmetic_and_control_flow_golden_program_reaches_expected_final_state() {
    let mut emulator = Emulator::new();
    emulator.load_program(ARITHMETIC_AND_CONTROL_FLOW_PROGRAM);
    // load_program's default (SS=SP=0) works, but a stack away from the
    // code bytes makes the test's intent clearer and catches any bug
    // where the stack would otherwise stomp on the program itself.
    emulator.registers.ss = 0;
    emulator.registers.sp = 0x2000;
    let initial_sp = emulator.registers.sp;

    const MAX_STEPS: usize = 1_000;
    let mut steps = 0;
    loop {
        match emulator.step().expect("golden program must decode cleanly") {
            StepOutcome::Halted => break,
            StepOutcome::Continued => {}
            StepOutcome::WaitingForKeyboard => panic!("this program does no keyboard I/O"),
        }
        steps += 1;
        assert!(
            steps < MAX_STEPS,
            "program did not halt within {MAX_STEPS} steps - likely an infinite-loop bug"
        );
    }

    assert_eq!(
        emulator.registers.ax, 30,
        "1+2+3+4+5 doubled by the CALLed subroutine"
    );
    assert_eq!(
        emulator.registers.bx, 6,
        "BX is incremented once past the last value added"
    );
    assert_eq!(
        emulator.registers.cx, 30,
        "CX ends up holding AX's value via PUSH/POP"
    );
    assert_eq!(
        emulator.registers.dx, 0xBEEF,
        "the conditional jump must have skipped the 0xDEAD assignment"
    );
    assert!(
        emulator.registers.get_flag(Flag::Zero),
        "the final CMP AX,30 compares equal"
    );
    assert_eq!(
        emulator.registers.sp, initial_sp,
        "CALL/RET and PUSH/POP must leave the stack balanced"
    );
    assert_eq!(
        emulator.memory.read_u16(0x0050),
        30,
        "MOV [0x0050], AX must have stored the computed sum"
    );
}
