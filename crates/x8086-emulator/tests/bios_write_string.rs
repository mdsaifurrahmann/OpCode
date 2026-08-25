//! `INT 10h AH=13h` (BIOS write string), and the MASM `BYTE`/`WORD PTR`
//! size prefix on an unbracketed variable that a reported program used
//! alongside it (`MOV DI, word inputBufferAddr`).

use x8086_emulator::{Emulator, StepOutcome};

fn run_with_input(source: &str, keys: &[u8]) -> String {
    let mut emulator = Emulator::new();
    let result = emulator.assemble_and_load(source);
    assert!(
        result.diagnostics.is_empty(),
        "diagnostics: {:?}",
        result.diagnostics
    );
    let mut keys = keys.iter().copied();
    let mut steps = 0;
    loop {
        match emulator.step().expect("program must decode cleanly") {
            StepOutcome::Halted => break,
            StepOutcome::Continued => {}
            StepOutcome::WaitingForKeyboard => {
                emulator.feed_key(0, keys.next().expect("ran out of supplied keystrokes"));
            }
        }
        steps += 1;
        assert!(steps < 50_000, "program did not halt");
    }
    emulator.console_output().to_string()
}

/// Read a line with `INT 21h AH=0Ah`, then echo it back with
/// `INT 10h AH=13h` - the shape of the reported program, but with the
/// input buffer given real reserved space in `.DATA` instead of a
/// hardcoded address that lands inside the code.
const READ_THEN_WRITE_STRING: &str = "\
.MODEL SMALL
.STACK 100H
.DATA
    inputBuffer DB 64, 0, 64 DUP('$')
.CODE
MAIN PROC
    MOV AX, @DATA
    MOV DS, AX
    MOV ES, AX

    LEA DX, inputBuffer
    MOV AH, 0Ah
    INT 21h

    LEA DI, inputBuffer
    INC DI
    MOV CH, 0
    MOV CL, byte [DI]
    INC DI
    MOV BP, DI

    MOV AH, 13h
    MOV AL, 01h
    MOV BX, 0
    MOV DL, 0
    INT 10h

    MOV AH, 4Ch
    INT 21h
MAIN ENDP
END MAIN
";

#[test]
fn int10h_ah13_writes_back_exactly_the_characters_that_were_typed() {
    let console = run_with_input(READ_THEN_WRITE_STRING, b"Hi there\r");
    // AH=0Ah echoes as you type and ends the line on Enter; AH=13h then
    // reprints the stored characters. The count byte is exact, so no
    // off-by-one adjustment is needed before using it as CX.
    assert_eq!(console, "Hi there\r\nHi there");
}

#[test]
fn int10h_ah13_writes_nothing_when_the_line_was_empty() {
    let console = run_with_input(READ_THEN_WRITE_STRING, b"\r");
    assert_eq!(console, "\r\n", "CX = 0 means there is nothing to write");
}

#[test]
fn a_size_prefix_on_an_unbracketed_variable_reads_memory() {
    // `MOV DI, word v` / `MOV BL, byte v` - MASM allows the size in
    // front of a bare variable name, which is already a memory
    // reference in this dialect. All three spellings must agree.
    let mut emulator = Emulator::new();
    let result = emulator
        .assemble_and_load("MOV DI, word addr\nMOV BL, byte size\nHLT\nsize DB 64\naddr DW 10\n");
    assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
    while emulator.step().expect("decode") != StepOutcome::Halted {}
    assert_eq!(emulator.registers.di, 10, "loaded the word stored at addr");
    assert_eq!(
        emulator.registers.bx as u8, 64,
        "loaded the byte stored at size"
    );
}

#[test]
fn ptr_is_optional_and_every_spelling_encodes_identically() {
    let bare = Emulator::new()
        .assemble_and_load("MOV BL, v\nHLT\nv DB 5\n")
        .machine_code;
    let sized = Emulator::new()
        .assemble_and_load("MOV BL, byte v\nHLT\nv DB 5\n")
        .machine_code;
    let masm = Emulator::new()
        .assemble_and_load("MOV BL, BYTE PTR v\nHLT\nv DB 5\n")
        .machine_code;
    assert_eq!(bare, sized);
    assert_eq!(sized, masm);
}
