//! `PUSHF`/`POPF`/`LAHF`/`SAHF`/`XLAT` - core 8086 instructions that
//! were missing entirely (reported as "unrecognized statement starting
//! with 'pushf'"), so a program using any of them failed to assemble.

use x8086_emulator::{Emulator, StepOutcome};

fn run(source: &str) -> Emulator {
    let mut emulator = Emulator::new();
    let result = emulator.assemble_and_load(source);
    assert!(
        result.diagnostics.is_empty(),
        "diagnostics: {:?}",
        result.diagnostics
    );
    let mut steps = 0;
    while emulator.step().expect("program must decode cleanly") != StepOutcome::Halted {
        steps += 1;
        assert!(steps < 10_000, "program did not halt");
    }
    emulator
}

fn carry(emulator: &Emulator) -> bool {
    emulator.registers.flags & 1 == 1
}

#[test]
fn pushf_and_popf_round_trip_the_flag_word() {
    // CF is set, saved, deliberately cleared, then restored.
    let emulator = run("STC\nPUSHF\nCLC\nPOPF\nHLT\n");
    assert!(carry(&emulator), "POPF should have restored the saved CF");
}

#[test]
fn popf_restores_flags_saved_before_intervening_arithmetic() {
    // ADD sets ZF here; POPF must undo that, not merely leave it alone.
    let emulator = run("STC\nPUSHF\nMOV AX, 0\nADD AX, 0\nPOPF\nHLT\n");
    assert!(carry(&emulator), "CF from before the ADD must come back");
}

#[test]
fn nested_pushf_pops_in_last_in_first_out_order() {
    let emulator = run("CLC\nPUSHF\nSTC\nPUSHF\nPOPF\nHLT\n");
    assert!(carry(&emulator), "the inner (CF set) frame pops first");

    let emulator = run("CLC\nPUSHF\nSTC\nPUSHF\nPOPF\nPOPF\nHLT\n");
    assert!(!carry(&emulator), "the outer (CF clear) frame pops second");
}

#[test]
fn lahf_loads_the_low_flag_byte_into_ah() {
    let emulator = run("STC\nLAHF\nHLT\n");
    assert_eq!(
        (emulator.registers.ax >> 8) & 1,
        1,
        "CF is bit 0 of the low flag byte, so it lands in AH's bit 0"
    );
}

#[test]
fn sahf_writes_ah_back_into_the_low_flag_byte() {
    let emulator = run("CLC\nMOV AH, 0FFh\nSAHF\nHLT\n");
    assert!(carry(&emulator), "AH bit 0 set means SAHF must set CF");

    let emulator = run("STC\nMOV AH, 0\nSAHF\nHLT\n");
    assert!(!carry(&emulator), "AH bit 0 clear means SAHF must clear CF");
}

#[test]
fn lahf_then_sahf_is_lossless() {
    // The classic idiom: stash flags in AH across some work, restore.
    let emulator = run("STC\nLAHF\nCLC\nSAHF\nHLT\n");
    assert!(carry(&emulator), "a LAHF/SAHF round trip must preserve CF");
}

#[test]
fn xlat_translates_al_through_a_table_at_bx() {
    // AL indexes a byte table based at DS:BX - OFFSET (not a bare
    // reference, which MASM dialect would dereference) puts the table's
    // address in BX.
    let emulator = run("MOV BX, OFFSET tbl\nMOV AL, 2\nXLAT\nHLT\ntbl DB 10,20,30,40\n");
    assert_eq!(emulator.registers.ax as u8, 30, "tbl[2]");

    let emulator = run("LEA BX, tbl\nMOV AL, 3\nXLAT\nHLT\ntbl DB 10,20,30,40\n");
    assert_eq!(emulator.registers.ax as u8, 40, "tbl[3], reached via LEA");
}

#[test]
fn xlatb_is_accepted_as_a_synonym_for_xlat() {
    // NASM spells the no-operand form XLATB.
    let a = Emulator::new().assemble_and_load("XLAT\n").machine_code;
    let b = Emulator::new().assemble_and_load("XLATB\n").machine_code;
    assert_eq!(a, b);
}

/// The reported program verbatim (plus the `HLT` it was missing), which
/// previously produced six "unrecognized statement" diagnostics.
#[test]
fn the_reported_program_assembles_and_runs() {
    let emulator = run("\
start:  pushf
        add bx, 1
        pushf
        add bx, 2
        pushf
        add bx, 3
        popf
        add bx, 4
        lahf
        add bx, 5
        sahf
        add bx, 6
        xlat
        add bx, 7
        hlt
");
    // 1+2+3+4+5+6+7 = 28, and nothing in between writes BX.
    assert_eq!(emulator.registers.bx, 28);
}
