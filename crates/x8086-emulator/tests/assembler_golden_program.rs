//! The same arithmetic/control-flow scenario as
//! `golden_programs.rs::arithmetic_and_control_flow_golden_program`, but
//! starting from real `.asm` source text through
//! `Emulator::assemble_and_load` instead of hand-assembled bytes - the
//! first end-to-end proof that source -> assembler -> decoder -> CPU
//! all agree with each other.

use x8086_emulator::{Emulator, StepOutcome};
use x8086_isa::Flag;

const SOURCE: &str = "\
        MOV CX, 5
        MOV AX, 0
        MOV BX, 1
SUM_LOOP:
        ADD AX, BX
        INC BX
        LOOP SUM_LOOP
        CALL DOUBLE
        CMP AX, 30
        JE SKIP
        MOV DX, 0DEADh
SKIP:
        MOV DX, 0BEEFh
        MOV [0050h], AX
        PUSH AX
        POP CX
        HLT
DOUBLE:
        ADD AX, AX
        RET
";

#[test]
fn source_assembles_cleanly_and_runs_to_the_same_final_state_as_the_hand_assembled_program() {
    let mut emulator = Emulator::new();
    let result = emulator.assemble_and_load(SOURCE);
    assert!(
        result.diagnostics.is_empty(),
        "unexpected diagnostics: {:?}",
        result.diagnostics
    );
    assert!(!result.machine_code.is_empty());

    emulator.registers.ss = 0;
    emulator.registers.sp = 0x2000;
    let initial_sp = emulator.registers.sp;

    const MAX_STEPS: usize = 1_000;
    let mut steps = 0;
    loop {
        match emulator
            .step()
            .expect("assembled program must decode cleanly")
        {
            StepOutcome::Halted => break,
            StepOutcome::Continued => {}
        }
        steps += 1;
        assert!(
            steps < MAX_STEPS,
            "program did not halt within {MAX_STEPS} steps - likely an infinite-loop bug"
        );
    }

    assert_eq!(emulator.registers.ax, 30);
    assert_eq!(emulator.registers.bx, 6);
    assert_eq!(emulator.registers.cx, 30);
    assert_eq!(emulator.registers.dx, 0xBEEF);
    assert!(emulator.registers.get_flag(Flag::Zero));
    assert_eq!(emulator.registers.sp, initial_sp);
    assert_eq!(emulator.memory.read_u16(0x0050), 30);
}
