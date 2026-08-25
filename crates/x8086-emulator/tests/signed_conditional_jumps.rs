//! Signed (`JG`/`JGE`/`JL`/`JLE`) vs unsigned (`JA`/`JB`) conditional
//! jumps against negative operands.
//!
//! This coverage only became possible once negative literals assembled -
//! a reported program testing exactly these jumps failed at `MOV BX, -10`
//! with "unexpected token '-' in operand", so the jump behavior it meant
//! to exercise had never actually run.

use x8086_emulator::{Emulator, StepOutcome};

/// Assembles a `CMP a, b` followed by `mnemonic`, and reports whether the
/// jump was taken (CX = 1) or fell through (CX = 0).
fn jump_taken(mnemonic: &str, a: i32, b: i32) -> bool {
    let source = format!(
        "MOV AX, {a}\n\
         MOV BX, {b}\n\
         MOV CX, 0\n\
         CMP AX, BX\n\
         {mnemonic} TAKEN\n\
         JMP DONE\n\
         TAKEN:\n\
         MOV CX, 1\n\
         DONE:\n\
         HLT\n"
    );
    let mut emulator = Emulator::new();
    let result = emulator.assemble_and_load(&source);
    assert!(
        result.diagnostics.is_empty(),
        "diagnostics for {mnemonic} {a},{b}: {:?}",
        result.diagnostics
    );

    let mut steps = 0;
    loop {
        match emulator.step().expect("program must decode cleanly") {
            StepOutcome::Halted => break,
            StepOutcome::Continued => {}
            StepOutcome::WaitingForKeyboard => panic!("this program does no keyboard I/O"),
        }
        steps += 1;
        assert!(steps < 10_000, "program did not halt");
    }
    emulator.registers.cx == 1
}

#[test]
fn signed_jumps_treat_a_negative_operand_as_less_than_a_positive_one() {
    // 20 > -10, so the "greater" jumps fire and the "lesser" ones don't.
    assert!(jump_taken("JG", 20, -10), "20 > -10");
    assert!(jump_taken("JGE", 20, -10), "20 >= -10");
    assert!(!jump_taken("JL", 20, -10), "20 is not < -10");
    assert!(!jump_taken("JLE", 20, -10), "20 is not <= -10");

    // ... and the reverse holds with the operands swapped.
    assert!(!jump_taken("JG", -10, 20), "-10 is not > 20");
    assert!(jump_taken("JL", -10, 20), "-10 < 20");
}

#[test]
fn signed_jumps_handle_the_equal_case() {
    assert!(!jump_taken("JG", 5, 5));
    assert!(jump_taken("JGE", 5, 5));
    assert!(!jump_taken("JL", 5, 5));
    assert!(jump_taken("JLE", 5, 5));
}

#[test]
fn unsigned_jumps_read_the_same_bits_the_opposite_way() {
    // The distinction the signed jumps exist for: as *unsigned* values
    // -10 is 0FFF6h = 65526, which is far above 20 - so JA/JB reach the
    // opposite conclusion from JG/JL on identical operands. Both are
    // correct; they're interpreting the same bit pattern differently.
    assert!(
        !jump_taken("JA", 20, -10),
        "unsigned: 20 is not above 65526"
    );
    assert!(jump_taken("JB", 20, -10), "unsigned: 20 is below 65526");
}

/// `IMUL` against a negative operand written as a raw hex byte - the
/// second half of the same reported snippet. `0xF9` is -7 as a signed
/// byte, so the signed product is -14 (0FFF2h), which is what makes
/// IMUL differ from MUL here: MUL would read the same bits as 249 and
/// produce 498 (01F2h).
#[test]
fn imul_treats_a_high_bit_operand_as_negative() {
    let mut emulator = Emulator::new();
    let result = emulator.assemble_and_load("MOV AL, 0xF9\nMOV BL, 0x02\nIMUL BL\nHLT\n");
    assert!(
        result.diagnostics.is_empty(),
        "diagnostics: {:?}",
        result.diagnostics
    );
    while emulator.step().expect("decode") != StepOutcome::Halted {}
    assert_eq!(
        emulator.registers.ax, 0xFFF2,
        "-7 * 2 = -14, which is 0FFF2h in 16-bit two's complement"
    );
}

/// The same bits through `MUL` instead, to show the two are genuinely
/// different instructions rather than one being "wrong".
#[test]
fn mul_treats_the_same_operand_as_unsigned() {
    let mut emulator = Emulator::new();
    let result = emulator.assemble_and_load("MOV AL, 0xF9\nMOV BL, 0x02\nMUL BL\nHLT\n");
    assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
    while emulator.step().expect("decode") != StepOutcome::Halted {}
    assert_eq!(emulator.registers.ax, 498, "unsigned: 249 * 2 = 498");
}
