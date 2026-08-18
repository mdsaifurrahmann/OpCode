//! End-to-end regression tests built from two real-world `.asm` programs
//! a user reported as failing to assemble/run: one written in classic
//! emu8086/MASM style (`.MODEL`/`.STACK`/`.DATA`/`.CODE`, `OFFSET`, shift/
//! rotate instructions), the other in NASM style (`section .text`/
//! `section .data`, `$`, bare `WORD [...]` without `PTR`, `DIV`/`IMUL`,
//! string instructions with `REP`/`REPE`). Both are self-checking: they
//! print "...: PASS$" and exit via `INT 21h AH=4Ch` with AL=0 on success,
//! or print "...: FAIL$" and exit with AL=1 the moment any check fails.

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
