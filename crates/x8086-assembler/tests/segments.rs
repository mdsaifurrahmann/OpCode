//! `.STACK`/`.DATA`/`.CODE` segment support - see `codegen`'s module docs
//! for the design. These directives used to be pure no-ops; this file
//! exercises the real segment layout they now produce.

use x8086_assembler::{assemble, SymbolKind};

/// The exact program shape a real emu8086 `.MODEL SMALL` tutorial
/// program takes, including the standard `MOV AX,@DATA`/`MOV DS,AX`
/// boilerplate - this is the concrete case that was previously broken
/// (`@DATA` was an undefined-symbol error).
const MODEL_SMALL_HELLO_WORLD: &str = "\
.MODEL SMALL
.STACK 100h
.DATA
msg DB \"Hi$\"
.CODE
start:
MOV AX, @DATA
MOV DS, AX
LEA DX, msg
MOV AH, 9
INT 21h
MOV AH, 4Ch
INT 21h
END start
";

#[test]
fn model_small_program_with_boilerplate_assembles_with_no_diagnostics() {
    let result = assemble(MODEL_SMALL_HELLO_WORLD);
    assert!(
        result.diagnostics.is_empty(),
        "diagnostics: {:?}",
        result.diagnostics
    );
    assert!(result.data_segment_base.is_some());
    assert!(result.stack_segment.is_some());
}

#[test]
fn data_segment_is_placed_after_code_at_a_paragraph_boundary() {
    // MOV AX,1 (3 bytes) then HLT (1 byte) = 4 bytes of code.
    let result = assemble(".CODE\nMOV AX, 1\nHLT\n.DATA\nvalue DB 42\n");
    assert!(
        result.diagnostics.is_empty(),
        "diagnostics: {:?}",
        result.diagnostics
    );
    let data_base = result.data_segment_base.expect("a .DATA section was used");
    assert_eq!(
        data_base, 16,
        "must round up to the next 16-byte paragraph after 4 bytes of code"
    );
    assert_eq!(
        result.machine_code[16], 42,
        "the DB byte must land at the data segment's base"
    );
}

#[test]
fn at_data_resolves_to_the_data_segments_paragraph_value() {
    // MOV AX,imm16 -> B8 <lo> <hi>; the immediate must equal data_base/16.
    let result = assemble(".CODE\nMOV AX, @DATA\nHLT\n.DATA\nvalue DB 1\n");
    assert!(
        result.diagnostics.is_empty(),
        "diagnostics: {:?}",
        result.diagnostics
    );
    let data_base = result.data_segment_base.unwrap();
    let expected_paragraph = (data_base / 16) as u16;
    assert_eq!(result.machine_code[0], 0xB8); // MOV AX, imm16
    let encoded = u16::from_le_bytes([result.machine_code[1], result.machine_code[2]]);
    assert_eq!(encoded, expected_paragraph);
}

#[test]
fn stack_directive_captures_size_and_places_the_stack_after_data() {
    let result = assemble(".STACK 100h\n.DATA\nvalue DW 0\n.CODE\nHLT\n");
    assert!(
        result.diagnostics.is_empty(),
        "diagnostics: {:?}",
        result.diagnostics
    );
    let data_base = result.data_segment_base.unwrap();
    let (stack_base, stack_size) = result.stack_segment.expect(".STACK was used");
    assert_eq!(stack_size, 0x100);
    assert!(
        stack_base >= data_base + 2,
        "stack must sit after the data segment"
    );
    assert_eq!(
        stack_base % 16,
        0,
        "segment bases must be paragraph-aligned"
    );
}

#[test]
fn program_without_any_segment_directives_has_no_data_segment_or_stack() {
    // A plain flat-style program (the style every pre-existing test and
    // sample program uses) must not suddenly grow a phantom segment.
    let result = assemble("MOV AX, 1\nHLT\n");
    assert!(result.diagnostics.is_empty());
    assert!(result.data_segment_base.is_none());
    assert!(result.stack_segment.is_none());
}

#[test]
fn data_symbol_values_are_relative_to_the_data_segment_not_flat_addresses() {
    // 8 bytes of code before .DATA - if `value`'s symbol table entry were
    // a flat address it would be 8; relative to its own segment it must
    // be 0 (the first thing declared in .DATA).
    let result = assemble(".CODE\nMOV AX, 1234h\nMOV BX, 5678h\n.DATA\nvalue DW 0\n");
    assert!(
        result.diagnostics.is_empty(),
        "diagnostics: {:?}",
        result.diagnostics
    );
    let entry = result.symbols.iter().find(|s| s.name == "value").unwrap();
    assert_eq!(entry.kind, SymbolKind::DataWord);
    assert_eq!(
        entry.value, 0,
        "must be an offset within the data segment, not a flat address"
    );
}

#[test]
fn entry_point_defaults_to_the_first_code_statement_even_when_data_precedes_code_in_source() {
    // No `END` label - entry point defaults to 0, which must land on the
    // first *code* statement (HLT), not on the data bytes physically
    // written earlier in the source text.
    let result = assemble(".DATA\nmsg DB \"hi\"\n.CODE\nHLT\n");
    assert!(
        result.diagnostics.is_empty(),
        "diagnostics: {:?}",
        result.diagnostics
    );
    assert_eq!(result.entry_point, 0);
    assert_eq!(
        result.machine_code[0], 0xF4,
        "address 0 must be the HLT opcode, not data"
    );
}
