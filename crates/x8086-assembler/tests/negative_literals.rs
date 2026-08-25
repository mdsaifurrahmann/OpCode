//! Signed/negative literals. A leading `-` or `+` on a term was
//! previously rejected everywhere it could appear ("unexpected token '-'
//! in operand"), because only the *binary* minus between two terms was
//! recognized - so `MOV BX, -10`, `DW -1`, `EQU -3`, `[-10]`, and
//! `10 - -3` all failed to assemble.

use x8086_assembler::assemble;

fn machine_code(source: &str) -> Vec<u8> {
    let result = assemble(source);
    assert!(
        result.diagnostics.is_empty(),
        "unexpected diagnostics for {source:?}: {:?}",
        result.diagnostics
    );
    result.machine_code
}

#[test]
fn negative_immediate_encodes_as_its_twos_complement() {
    // MOV BX, -10 -> BB F6 FF (0xFFF6 = -10 in 16-bit two's complement).
    assert_eq!(machine_code("MOV BX, -10\n"), vec![0xBB, 0xF6, 0xFF]);
}

#[test]
fn a_leading_plus_assembles_identically_to_no_sign() {
    assert_eq!(machine_code("MOV AX, +5\n"), machine_code("MOV AX, 5\n"));
}

#[test]
fn negative_values_work_in_data_directives_and_equates() {
    assert_eq!(machine_code("val DW -1\n"), vec![0xFF, 0xFF]);
    assert_eq!(
        machine_code("k EQU -3\nMOV AX, k\n"),
        vec![0xB8, 0xFD, 0xFF]
    );
}

#[test]
fn a_negative_direct_address_is_accepted_in_a_memory_operand() {
    assert_eq!(
        machine_code("MOV AL, [-10]\n"),
        machine_code("MOV AL, [0FFF6h]\n")
    );
}

#[test]
fn a_binary_minus_followed_by_a_negative_term_adds() {
    // 10 - -3 = 13.
    assert_eq!(
        machine_code("MOV AX, 10 - -3\n"),
        machine_code("MOV AX, 13\n")
    );
}

#[test]
fn existing_binary_subtraction_still_works() {
    assert_eq!(machine_code("MOV AX, 10-3\n"), machine_code("MOV AX, 7\n"));
}

/// The reported program: signed comparison against a negative literal.
/// Every `MOV BX, -10` line failed to assemble, so none of the signed
/// conditional jumps it exercises could run at all.
const SIGNED_COMPARISON_PROGRAM: &str = "\
start:
MOV AX, 20
MOV BX, -10
CMP AX, BX
JG AX_GREATER_THAN_BX
MOV AX, 0
AX_GREATER_THAN_BX:
MOV AX, 0

MOV AX, 20
MOV BX, -10
CMP AX, BX
JL AX_LESSER_THAN_BX
MOV AX, 0
AX_LESSER_THAN_BX:
MOV AX, 0
HLT
";

#[test]
fn the_reported_signed_comparison_program_assembles_cleanly() {
    let result = assemble(SIGNED_COMPARISON_PROGRAM);
    assert!(
        result.diagnostics.is_empty(),
        "diagnostics: {:?}",
        result.diagnostics
    );
}
