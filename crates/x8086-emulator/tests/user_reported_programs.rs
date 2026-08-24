//! End-to-end regression tests built from real-world `.asm` programs
//! users reported as failing to assemble/run: two written in classic
//! emu8086/MASM style (`.MODEL`/`.STACK`/`.DATA`/`.CODE`, `OFFSET`, shift/
//! rotate instructions), one in NASM style (`section .text`/
//! `section .data`, `$`, bare `WORD [...]` without `PTR`, `DIV`/`IMUL`,
//! string instructions with `REP`/`REPE`). The first two are
//! self-checking: they print "...: PASS$" and exit via `INT 21h AH=4Ch`
//! with AL=0 on success, or print "...: FAIL$" and exit with AL=1 the
//! moment any check fails. The buffered-input one is a real lab
//! assignment with no such self-check - it's verified by asserting the
//! exact console transcript instead.

use x8086_emulator::{Emulator, StepOutcome};

const SHIFT_ROTATE_PROGRAM: &str = "\
.model small
.stack 100h

.data
msg     db  \"shift/rotate test: PASS$\"
errmsg  db  \"shift/rotate test: FAIL$\"

.code
org 100h

main proc
    clc
    jc fail
    stc
    jnc fail
    cmc
    jc fail

    mov al, 1
    mov cl, 3
    shl al, cl
    cmp al, 8
    jne fail

    mov al, 1
    mov cl, 3
    sal al, cl
    cmp al, 8
    jne fail

    mov al, 80h
    mov cl, 3
    shr al, cl
    cmp al, 16
    jne fail

    mov al, 0F0h
    mov cl, 2
    sar al, cl
    cmp al, 0FCh
    jne fail

    mov al, 81h
    rol al, 1
    cmp al, 03h
    jne fail

    mov al, 03h
    ror al, 1
    cmp al, 81h
    jne fail

    mov ax, 8000h
    mov dx, 0001h
    shl ax, 1
    rcl dx, 1
    cmp ax, 0000h
    jne fail
    cmp dx, 0003h
    jne fail

    mov dx, 0003h
    mov ax, 0000h
    shr dx, 1
    rcr ax, 1
    cmp dx, 0001h
    jne fail
    cmp ax, 8000h
    jne fail

    mov al, 0FFh
    mov cl, 4
    rol al, cl
    cmp al, 0FFh
    jne fail

    stc
    mov al, 00h
    rcl al, 1
    cmp al, 01h
    jne fail

    jmp pass

pass:
    mov ah, 09h
    mov dx, offset msg
    int 21h
    mov ax, 4c00h
    int 21h

fail:
    mov ah, 09h
    mov dx, offset errmsg
    int 21h
    mov ax, 4c01h
    int 21h
main endp

end main
";

const NASM_STYLE_PROGRAM: &str = "\
org 100h

section .text

start:
    mov ax, 37
    mov bx, 5
    mov cx, ax
    add cx, bx
    mov dx, ax
    sub dx, bx
    cmp cx, 42
    jne fail
    cmp dx, 32
    jne fail

    mov ax, 37
    xor dx, dx
    mov bx, 5
    div bx
    cmp ax, 7
    jne fail
    cmp dx, 2
    jne fail

    mov si, iarray
    mov cx, 5
    xor bx, bx
sum_loop:
    lodsw
    add bx, ax
    loop sum_loop
    cmp bx, 150
    jne fail

    mov ax, 6
    call factorial
    cmp ax, 720
    jne fail

    push bp
    mov bp, sp
    sub sp, 4
    mov word [bp-2], 111
    mov word [bp-4], 222
    mov ax, [bp-2]
    add ax, [bp-4]
    mov sp, bp
    pop bp
    cmp ax, 333
    jne fail

    mov si, msg
    mov di, dst
    mov cx, msg_len
    cld
    rep movsb

    mov si, dst
    mov di, msg
    mov cx, msg_len
    repe cmpsb
    jne fail

    jmp pass

factorial:
    push bx
    cmp ax, 1
    jle fact_base
    mov bx, ax
    dec ax
    call factorial
    imul bx
    jmp fact_done
fact_base:
    mov ax, 1
fact_done:
    pop bx
    ret

pass:
    mov ah, 09h
    mov dx, msg
    int 21h
    mov ax, 4C00h
    int 21h

fail:
    mov ah, 09h
    mov dx, errmsg
    int 21h
    mov ax, 4C01h
    int 21h

section .data

msg     db  \"8086 emulator test: PASS$\"
msg_len equ $ - msg - 1
errmsg  db  \"8086 emulator test: FAIL$\"
iarray  dw  10, 20, 30, 40, 50
dst     times 32 db 0
";

/// Reported as failing with "unexpected token '[' after operand" on
/// every line using `buff1[1]`/`buff1[2]` (MASM's `symbol[expr]`
/// indexing sugar for `[symbol+expr]`), and separately - once that parse
/// error was fixed - would have gone on to read garbage input, since
/// `INT 21h AH=0Ah` (buffered line input) wasn't simulated at all. Kept
/// byte-for-byte as originally reported, including the `MODEL SMALL`
/// capitalization and the mixed-case `H`-suffix hex literals.
const BUFFERED_MULTIPLY_PROGRAM: &str = "\
.MODEL SMALL
.STACK 100H

.DATA
    msg1    DB \"Enter Multiplier: $\"
    msg2    DB 0DH,0AH,\"Enter Multiplicand: $\"
    msg3    DB 0DH,0AH,\"Product = $\"
    crlf    DB 0DH,0AH,\"$\"

    buff1   DB 7,0,7 DUP('$')
    buff2   DB 7,0,7 DUP('$')

    num1    DW 0
    num2    DW 0

.CODE
MAIN PROC
    MOV AX, @DATA
    MOV DS, AX

    LEA DX, msg1
    MOV AH, 09H
    INT 21H

    LEA DX, buff1
    MOV AH, 0AH
    INT 21H

    MOV CL, buff1[1]
    XOR CH, CH
    LEA SI, buff1[2]
    CALL STR_TO_NUM
    MOV num1, BX

    LEA DX, msg2
    MOV AH, 09H
    INT 21H

    LEA DX, buff2
    MOV AH, 0AH
    INT 21H

    MOV CL, buff2[1]
    XOR CH, CH
    LEA SI, buff2[2]
    CALL STR_TO_NUM
    MOV num2, BX

    MOV AX, num1
    MOV CX, num2
    MUL CX

    LEA DX, msg3
    MOV AH, 09H
    INT 21H

    CALL PRINT_NUM

    LEA DX, crlf
    MOV AH, 09H
    INT 21H

    MOV AH, 4CH
    INT 21H
MAIN ENDP

STR_TO_NUM PROC
    PUSH AX
    PUSH DX
    XOR BX, BX
S2N_LOOP:
    MOV AL, [SI]
    SUB AL, '0'
    XOR AH, AH
    PUSH AX
    MOV AX, BX
    MOV DX, 10
    MUL DX
    MOV BX, AX
    POP AX
    ADD BX, AX
    INC SI
    LOOP S2N_LOOP
    POP DX
    POP AX
    RET
STR_TO_NUM ENDP

PRINT_NUM PROC
    PUSH AX
    PUSH BX
    PUSH CX
    PUSH DX

    MOV CX, 0
    MOV BX, 10

    CMP AX, 0
    JNE PN_LOOP
    MOV DL, '0'
    MOV AH, 02H
    INT 21H
    JMP PN_DONE

PN_LOOP:
    CMP AX, 0
    JE PN_PRINT
    XOR DX, DX
    DIV BX
    PUSH DX
    INC CX
    JMP PN_LOOP

PN_PRINT:
    CMP CX, 0
    JE PN_DONE
    POP DX
    ADD DL, '0'
    MOV AH, 02H
    INT 21H
    DEC CX
    JMP PN_PRINT

PN_DONE:
    POP DX
    POP CX
    POP BX
    POP AX
    RET
PRINT_NUM ENDP

END MAIN
";

/// Like `run_to_completion`, but feeds `keys` (one ASCII byte per
/// keystroke) to satisfy each `WaitingForKeyboard` pause in turn - for
/// programs that actually read from the console, as opposed to the
/// self-checking programs above which do no I/O at all.
fn run_to_completion_with_keyboard_input(source: &str, keys: &[u8]) -> String {
    let mut emulator = Emulator::new();
    let result = emulator.assemble_and_load(source);
    assert!(
        result.diagnostics.is_empty(),
        "unexpected diagnostics: {:?}",
        result.diagnostics
    );

    let mut keys = keys.iter().copied();
    const MAX_STEPS: usize = 200_000;
    let mut steps = 0;
    loop {
        match emulator.step().expect("program must decode cleanly") {
            StepOutcome::Halted => break,
            StepOutcome::Continued => {}
            StepOutcome::WaitingForKeyboard => {
                let ascii = keys
                    .next()
                    .expect("program asked for more keyboard input than the test supplied");
                emulator.feed_key(0, ascii);
            }
        }
        steps += 1;
        assert!(
            steps < MAX_STEPS,
            "program did not halt within {MAX_STEPS} steps - likely an infinite-loop bug"
        );
    }

    emulator.console_output().to_string()
}

#[test]
fn buffered_input_multiply_program_reads_two_numbers_and_prints_their_product() {
    // "12" <Enter> "34" <Enter>. 12 * 34 = 408 (AX = 0x0198 right after
    // `MUL CX`) - but the source's own `MOV AH, 09H` (to select the
    // "print string" DOS function for the "Product = " message) clobbers
    // just the *high* byte of AX before `CALL PRINT_NUM` ever reads it,
    // turning 0x0198 into 0x0998 = 2456. That's a bug in the reported
    // program itself (PRINT_NUM's own comment - "prints value currently
    // in AX" - is simply wrong given what runs between `MUL` and the
    // call), not in assembly or emulation: real DOS/emu8086 would print
    // the exact same wrong "2456" from this exact source. Asserting the
    // literal (buggy) output, not the mathematically "expected" one, is
    // what actually proves the emulation is faithful here.
    let console = run_to_completion_with_keyboard_input(BUFFERED_MULTIPLY_PROGRAM, b"12\r34\r");
    assert_eq!(
        console,
        "Enter Multiplier: 12\r\n\r\nEnter Multiplicand: 34\r\n\r\nProduct = 2456\r\n"
    );
}

/// A second reported program with the *same* underlying defect as
/// `BUFFERED_MULTIPLY_PROGRAM`, reduced to single-digit input, reported
/// as "the emulator computes 2*3 wrong". `{FIX}` is substituted to
/// produce two variants from one source, so the only difference between
/// the broken and working runs is provably the fix itself.
const SINGLE_DIGIT_MULTIPLY_PROGRAM: &str = "\
.MODEL SMALL
.STACK 100H

.DATA
    msg1 DB \"Enter 1st number: $\"
    msg2 DB 0DH,0AH,\"Enter 2nd number: $\"
    msg3 DB 0DH,0AH,\"Product = $\"

.CODE
MAIN PROC
    MOV AX, @DATA
    MOV DS, AX

    LEA DX, msg1
    MOV AH, 09H
    INT 21H
    MOV AH, 01H
    INT 21H
    SUB AL, '0'
    MOV BL, AL

    LEA DX, msg2
    MOV AH, 09H
    INT 21H
    MOV AH, 01H
    INT 21H
    SUB AL, '0'
    MOV CL, AL

    MOV AL, BL
    MUL CL

    LEA DX, msg3
    MOV AH, 09H
    INT 21H
{FIX}
    MOV BL, 10
    DIV BL
    MOV CL, AH

    CMP AL, 0
    JE ONES
    ADD AL, '0'
    MOV DL, AL
    MOV AH, 02H
    INT 21H
ONES:
    MOV DL, CL
    ADD DL, '0'
    MOV AH, 02H
    INT 21H

    MOV AH, 4CH
    INT 21H
MAIN ENDP
END MAIN
";

#[test]
fn single_digit_multiply_reproduces_the_reported_garbled_output_as_written() {
    // 2 * 3. `MUL CL` correctly leaves AX = 6, but the program's own
    // `MOV AH, 09H` (selecting the DOS print-string function for the
    // "Product = " label) rewrites the *high half of AX* before `DIV BL`
    // consumes it - so DIV divides 0x0906 = 2310, not 6, yielding
    // AL = 231. `ADD AL, '0'` then overflows 8 bits (231 + 48 = 279)
    // and wraps to 0x17, an unprintable control character.
    //
    // Every step of that is documented 8086 behavior - see
    // x8086-cpu's `mov_ah_between_mul_and_div_corrupts_the_product_
    // exactly_as_real_hardware_does`, which asserts each instruction
    // individually, and the iced-x86 cross-validation confirming the
    // encodings. Real DOS/emu8086 print the same garbage from this
    // source. Pinning the literal broken output is what proves the
    // emulation stays faithful rather than silently "helpfully" wrong.
    let source = SINGLE_DIGIT_MULTIPLY_PROGRAM.replace("{FIX}", "");
    let console = run_to_completion_with_keyboard_input(&source, b"23");
    assert_eq!(
        console,
        "Enter 1st number: 2\r\nEnter 2nd number: 3\r\nProduct = \u{17}0"
    );
}

#[test]
fn single_digit_multiply_prints_6_once_the_product_is_preserved_across_the_dos_call() {
    // The one-line fix: stash the product before the DOS call clobbers
    // AH, restore it right before DIV. Same program otherwise.
    let source = SINGLE_DIGIT_MULTIPLY_PROGRAM.replace("{FIX}", "    MOV AL, BL\n    MUL CL\n");
    let console = run_to_completion_with_keyboard_input(&source, b"23");
    assert_eq!(
        console, "Enter 1st number: 2\r\nEnter 2nd number: 3\r\nProduct = 6",
        "with AX intact, DIV BL sees 6: AL=0 (tens, skipped by the \
         CMP/JE) and AH=6 (ones), printing exactly \"6\""
    );
}

/// Runs `source` to completion (HLT or an `INT 21h AH=4Ch` termination),
/// asserting it assembled with zero diagnostics and returning the final
/// AL value (the program's own self-reported exit code: 0 = pass, 1 =
/// fail) alongside everything printed to the console.
fn run_to_completion(source: &str) -> (u8, String) {
    let mut emulator = Emulator::new();
    let result = emulator.assemble_and_load(source);
    assert!(
        result.diagnostics.is_empty(),
        "unexpected diagnostics: {:?}",
        result.diagnostics
    );

    const MAX_STEPS: usize = 20_000;
    let mut steps = 0;
    loop {
        match emulator.step().expect("program must decode cleanly") {
            StepOutcome::Halted => break,
            StepOutcome::Continued => {}
            StepOutcome::WaitingForKeyboard => panic!("this program does no keyboard I/O"),
        }
        steps += 1;
        assert!(
            steps < MAX_STEPS,
            "program did not halt within {MAX_STEPS} steps - likely an infinite-loop bug"
        );
    }

    let al = (emulator.registers.ax & 0xFF) as u8;
    (al, emulator.console_output().to_string())
}

#[test]
fn emu8086_style_shift_rotate_program_passes_every_check() {
    let (al, console) = run_to_completion(SHIFT_ROTATE_PROGRAM);
    // Exact match, not just `.contains("PASS")`: a garbage-prefixed
    // dereference bug (e.g. `OFFSET msg` resolving to a stray memory
    // read instead of the address) can still print the right message
    // as a *substring* after printing junk from wherever it actually
    // pointed - a `.contains` check alone would miss that regression
    // silently.
    assert_eq!(console, "shift/rotate test: PASS");
    assert_eq!(al, 0, "AL must be the program's own PASS exit code (0)");
}

#[test]
fn nasm_style_program_passes_every_check() {
    let (al, console) = run_to_completion(NASM_STYLE_PROGRAM);
    assert_eq!(console, "8086 emulator test: PASS");
    assert_eq!(al, 0, "AL must be the program's own PASS exit code (0)");
}
