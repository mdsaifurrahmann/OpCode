//! MASM/emu8086 and NASM flatly disagree on what a bare `DB`/`DW`
//! variable reference means: MASM/emu8086 dereferences it, NASM treats
//! it as that variable's address. `assemble()` picks a convention per
//! file based on whether `SECTION` (a NASM-only directive) appears
//! anywhere in the source - these tests pin down both sides of that.

use x8086_assembler::assemble;
use x8086_isa::{Mnemonic, Operand};

#[test]
fn bare_data_word_reference_dereferences_by_default_matching_masm() {
    let result = assemble("myvar dw 42\nmov ax, myvar\nhlt");
    assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
    // myvar's DW bytes occupy the first 2 bytes (flat code region, no
    // .DATA directive in play); the MOV starts right after them.
    let (instr, _) = x8086_decoder::decode_one(&result.machine_code[2..]).unwrap();
    assert_eq!(instr.mnemonic, Mnemonic::Mov);
    assert!(
        matches!(instr.operands[1], Operand::Memory { .. }),
        "expected a memory (dereferencing) operand, got {:?}",
        instr.operands[1]
    );
}

#[test]
fn bare_data_word_reference_is_the_address_in_nasm_dialect() {
    let result = assemble("section .data\nmyvar dw 42\nsection .text\nmov ax, myvar\nhlt");
    assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
    let (instr, _) =
        x8086_decoder::decode_one(&result.machine_code[result.entry_point as usize..]).unwrap();
    assert_eq!(instr.mnemonic, Mnemonic::Mov);
    assert!(
        matches!(instr.operands[1], Operand::Immediate(_)),
        "expected an immediate (address) operand, got {:?}",
        instr.operands[1]
    );
}

#[test]
fn masm_style_data_directive_does_not_trigger_nasm_dialect() {
    // `.DATA` is MASM/emu8086's own directive, not a NASM `SECTION` - it
    // must not accidentally flip dereference semantics.
    let result = assemble(".model small\n.data\nmyvar dw 42\n.code\nmov ax, myvar\nhlt");
    assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
    let (instr, _) =
        x8086_decoder::decode_one(&result.machine_code[result.entry_point as usize..]).unwrap();
    assert!(matches!(instr.operands[1], Operand::Memory { .. }));
}
