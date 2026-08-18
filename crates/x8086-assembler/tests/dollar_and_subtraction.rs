//! Tests for NASM's `$` (current-location) pseudo-symbol and for
//! subtracting one symbol's address from another - the exact idiom real
//! NASM source uses to compute a string's length: `len equ $ - label - 1`.

use x8086_assembler::assemble;

#[test]
fn dollar_minus_label_computes_a_strings_length() {
    let source = "\
msg     db  \"hello$\"
msg_len equ $ - msg - 1
        mov cx, msg_len
";
    let result = assemble(source);
    assert!(
        result.diagnostics.is_empty(),
        "unexpected diagnostics: {:?}",
        result.diagnostics
    );
    let msg_len = result
        .symbols
        .iter()
        .find(|s| s.name == "msg_len")
        .unwrap_or_else(|| panic!("msg_len not in symbol table: {:?}", result.symbols));
    // "hello$" is 6 bytes; minus the trailing '$' delimiter, the real
    // text length is 5.
    assert_eq!(msg_len.value, 5);
}

#[test]
fn dollar_works_the_same_way_after_other_data_before_it() {
    let source = "\
prefix  db  \"xx\"
msg     db  \"hello$\"
msg_len equ $ - msg - 1
";
    let result = assemble(source);
    assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
    let msg_len = result.symbols.iter().find(|s| s.name == "msg_len").unwrap();
    assert_eq!(msg_len.value, 5);
}

#[test]
fn jmp_dollar_is_a_self_referencing_infinite_loop() {
    // JMP $ must encode a branch whose target is its own start address -
    // the classic "spin here forever" idiom.
    let result = assemble("start: JMP $\nHLT");
    assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
    // JMP near is 3 bytes (E9 + rel16); a self-target means rel = -3.
    assert_eq!(&result.machine_code[0..3], &[0xE9, 0xFD, 0xFF]);
}

#[test]
fn subtracting_two_labels_computes_the_distance_between_them() {
    let source = "\
a:      db  1, 2, 3
b:      db  4, 5
diff    equ b - a
        mov cx, diff
";
    let result = assemble(source);
    assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
    let diff = result.symbols.iter().find(|s| s.name == "diff").unwrap();
    assert_eq!(diff.value, 3);
}
