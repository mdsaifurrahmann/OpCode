//! MASM/emu8086's `symbol[expr]` postfix indexing operator - sugar for
//! `[symbol+expr]`. A real user program (`buff1[1]`, `buff1[2]`, same
//! shape as a DOS buffered-input structure lookup) used to fail assembly
//! entirely with "unexpected token '[' after operand" on every line that
//! used it, since the parser only recognized a *leading* `[`.

use x8086_assembler::assemble;

/// A reduced version of the exact reported program shape: read a
/// buffered-input structure and index into it with `buff1[1]`/`buff1[2]`.
const BUFFERED_INPUT_INDEXING: &str = "\
.MODEL SMALL
.STACK 100h
.DATA
buff1 DB 7,0,7 DUP('$')
.CODE
MOV AX, @DATA
MOV DS, AX
MOV CL, buff1[1]
XOR CH, CH
LEA SI, buff1[2]
MOV AH, 4Ch
INT 21h
END
";

#[test]
fn buffered_input_style_program_assembles_with_no_diagnostics() {
    let result = assemble(BUFFERED_INPUT_INDEXING);
    assert!(
        result.diagnostics.is_empty(),
        "diagnostics: {:?}",
        result.diagnostics
    );
}

#[test]
fn symbol_bracket_indexing_encodes_identically_to_the_equivalent_bracket_expression() {
    let indexed = assemble("MOV CL, buff1[1]\nHLT\nbuff1 DB 10 DUP(0)\n");
    let bracketed = assemble("MOV CL, [buff1+1]\nHLT\nbuff1 DB 10 DUP(0)\n");
    assert!(indexed.diagnostics.is_empty(), "{:?}", indexed.diagnostics);
    assert!(
        bracketed.diagnostics.is_empty(),
        "{:?}",
        bracketed.diagnostics
    );
    assert_eq!(indexed.machine_code, bracketed.machine_code);
}

#[test]
fn register_bracket_indexing_encodes_identically_to_the_equivalent_bracket_expression() {
    let indexed = assemble("MOV AL, BX[2]\nHLT\n");
    let bracketed = assemble("MOV AL, [BX+2]\nHLT\n");
    assert!(indexed.diagnostics.is_empty(), "{:?}", indexed.diagnostics);
    assert!(
        bracketed.diagnostics.is_empty(),
        "{:?}",
        bracketed.diagnostics
    );
    assert_eq!(indexed.machine_code, bracketed.machine_code);
}
