//! Diagnostic accuracy (exact line, sensible message) and line-map
//! correctness (only lines that actually produce code get an address)
//! for the two-pass assembler.

use x8086_assembler::assemble;

#[test]
fn lea_with_a_forward_referenced_data_variable_computes_the_correct_address() {
    // Regression test: a bare reference to a DB/DW variable declared
    // *after* the instruction that uses it (the common "code first,
    // data section after" style) must resolve to a Memory-shaped
    // operand even while its address is still unknown during pass 1 -
    // getting the shape wrong there made LEA fail to encode during
    // pass 1 (silently defaulting its length to 0), which shifted every
    // subsequent address and corrupted the loaded pointer.
    let result = assemble("LEA DX, msg\nHLT\nmsg DB \"Hi$\"\n");
    assert!(
        result.diagnostics.is_empty(),
        "diagnostics: {:?}",
        result.diagnostics
    );
    // LEA DX,[msg] (direct address form) = 8D 16 <disp16>; HLT = F4;
    // then the "Hi$" bytes. msg must sit right after HLT, at offset 5.
    assert_eq!(
        result.machine_code,
        vec![0x8D, 0x16, 0x05, 0x00, 0xF4, b'H', b'i', b'$']
    );
}

#[test]
fn undefined_symbol_reports_the_correct_line() {
    let result = assemble("HLT\nMOV AX, missing_symbol");
    assert_eq!(
        result.diagnostics.len(),
        1,
        "diagnostics: {:?}",
        result.diagnostics
    );
    assert_eq!(result.diagnostics[0].line, 2);
    assert!(
        result.diagnostics[0].message.contains("missing_symbol"),
        "message: {}",
        result.diagnostics[0].message
    );
}

#[test]
fn ambiguous_memory_operand_size_is_a_diagnostic() {
    let result = assemble("MOV [BX], 5");
    assert_eq!(
        result.diagnostics.len(),
        1,
        "diagnostics: {:?}",
        result.diagnostics
    );
    assert!(
        result.diagnostics[0].message.contains("BYTE PTR")
            || result.diagnostics[0].message.contains("WORD PTR"),
        "message: {}",
        result.diagnostics[0].message
    );
}

#[test]
fn explicit_size_override_resolves_the_ambiguity() {
    let result = assemble("MOV BYTE PTR [BX], 5");
    assert!(
        result.diagnostics.is_empty(),
        "diagnostics: {:?}",
        result.diagnostics
    );
    assert_eq!(result.machine_code, vec![0xC6, 0b00_000_111, 0x05]);
}

#[test]
fn out_of_range_conditional_jump_is_a_diagnostic() {
    // Pad the distance between JE and its target past the 8-bit relative
    // range (-128..=127) that Jcc is hard-limited to on the 8086.
    let mut source = String::from("JE far_label\n");
    for _ in 0..200 {
        source.push_str("NOP\n");
    }
    source.push_str("far_label: HLT\n");

    let result = assemble(&source);
    assert_eq!(
        result.diagnostics.len(),
        1,
        "diagnostics: {:?}",
        result.diagnostics
    );
    assert_eq!(result.diagnostics[0].line, 1);
    assert!(
        result.diagnostics[0].message.contains("range"),
        "message: {}",
        result.diagnostics[0].message
    );
}

#[test]
fn in_range_conditional_jump_has_no_diagnostic() {
    let result = assemble("JE near_label\nNOP\nnear_label: HLT");
    assert!(
        result.diagnostics.is_empty(),
        "diagnostics: {:?}",
        result.diagnostics
    );
}

#[test]
fn a_bad_line_does_not_hide_diagnostics_on_other_lines() {
    // Line 1 is a parse error (unrecognized statement); line 3 is a
    // semantic error (undefined symbol). Both must be reported - one bad
    // line must not swallow the rest of the file's diagnostics.
    let result = assemble("FROBNICATE AX\nHLT\nMOV AX, nope");
    assert_eq!(
        result.diagnostics.len(),
        2,
        "diagnostics: {:?}",
        result.diagnostics
    );
    let lines: Vec<u32> = result.diagnostics.iter().map(|d| d.line).collect();
    assert!(lines.contains(&1));
    assert!(lines.contains(&3));
}

#[test]
fn line_map_only_covers_lines_that_produce_code() {
    let source = "\
; a leading comment
        MOV AX, 5

start:
COUNT   EQU 3
        ORG 10h
        ADD AX, 1
";
    let result = assemble(source);
    assert!(
        result.diagnostics.is_empty(),
        "diagnostics: {:?}",
        result.diagnostics
    );

    // Line 2: "MOV AX, 5" -> produces code, must be mapped.
    assert!(result.line_to_address.contains_key(&2));
    // Line 4 (label), line 5 (EQU), line 6 (ORG): produce no bytes.
    assert!(!result.line_to_address.contains_key(&4));
    assert!(!result.line_to_address.contains_key(&5));
    assert!(!result.line_to_address.contains_key(&6));
    // Line 7: "ADD AX, 1", placed at the address ORG just set (0x10).
    assert_eq!(result.line_to_address.get(&7), Some(&0x10));
}

#[test]
fn line_map_addresses_increase_by_each_instructions_encoded_length() {
    let result = assemble("MOV AX, 5\nADD AX, 1\nHLT");
    assert!(
        result.diagnostics.is_empty(),
        "diagnostics: {:?}",
        result.diagnostics
    );
    // MOV AX,5 (imm16 reg form) is 3 bytes; ADD AX,1 (imm8-sign-extended
    // form via the encoder's word-immediate path) starts right after.
    let addr1 = *result.line_to_address.get(&1).unwrap();
    let addr2 = *result.line_to_address.get(&2).unwrap();
    let addr3 = *result.line_to_address.get(&3).unwrap();
    assert_eq!(addr1, 0);
    assert!(addr2 > addr1);
    assert!(addr3 > addr2);
}

#[test]
fn db_and_dw_variables_are_addressable_by_name() {
    let result = assemble("msg DB \"hi\", 0\ncount DW 42\nMOV AX, [msg]");
    assert!(
        result.diagnostics.is_empty(),
        "diagnostics: {:?}",
        result.diagnostics
    );
    // "hi\0" = 3 bytes, so `count` sits at address 3, and the code
    // following the two data statements starts at address 5.
    assert_eq!(result.machine_code[0..3], [b'h', b'i', 0]);
    assert_eq!(&result.machine_code[3..5], &42u16.to_le_bytes());
}

#[test]
fn symbol_table_distinguishes_data_byte_word_label_and_constant_kinds() {
    use x8086_assembler::SymbolKind;

    let result = assemble("SIZE EQU 4\nmsg DB \"hi\", 0\ncount DW 42\nstart: NOP\nEND start");
    assert!(
        result.diagnostics.is_empty(),
        "diagnostics: {:?}",
        result.diagnostics
    );
    let kind_of = |name: &str| {
        result
            .symbols
            .iter()
            .find(|s| s.name == name)
            .unwrap_or_else(|| panic!("no symbol named {name}"))
            .kind
    };
    assert_eq!(kind_of("SIZE"), SymbolKind::Constant);
    assert_eq!(kind_of("msg"), SymbolKind::DataByte);
    assert_eq!(kind_of("count"), SymbolKind::DataWord);
    assert_eq!(kind_of("start"), SymbolKind::Label);
}

#[test]
fn end_with_label_sets_the_entry_point() {
    // HLT is 1 byte, so `main` sits at address 1.
    let result = assemble("HLT\nmain: NOP\nEND main");
    assert!(
        result.diagnostics.is_empty(),
        "diagnostics: {:?}",
        result.diagnostics
    );
    assert_eq!(result.entry_point, 1);
}

#[test]
fn no_end_statement_defaults_entry_point_to_zero() {
    let result = assemble("HLT");
    assert!(
        result.diagnostics.is_empty(),
        "diagnostics: {:?}",
        result.diagnostics
    );
    assert_eq!(result.entry_point, 0);
}

#[test]
fn end_with_undefined_label_is_a_diagnostic() {
    let result = assemble("HLT\nEND nowhere");
    assert_eq!(
        result.diagnostics.len(),
        1,
        "diagnostics: {:?}",
        result.diagnostics
    );
}
