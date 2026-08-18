import Foundation

/// One bundled sample program, listed under File > Open Sample. Written
/// fresh for this project rather than adapted from emu8086's own bundled
/// samples, to avoid any copyright question.
struct SampleProgram: Identifiable {
    var id: String { name }
    let name: String
    let source: String
}

enum Samples {
    static let helloWorld = SampleProgram(
        name: "Hello, World",
        source: """
            LEA DX, msg
            MOV AH, 9
            INT 21h
            HLT
            msg DB "Hello, x8086!$"
            """
    )

    static let arithmeticAndFlags = SampleProgram(
        name: "Arithmetic & Flags",
        source: """
            ; Adds two 16-bit numbers into AX. CF/ZF/SF/OF reflect the
            ; addition directly - Step Into this one instruction and
            ; watch the Flags panel to see them get computed.
            MOV AX, 1234h
            MOV BX, 0FFFFh
            ADD AX, BX
            HLT
            """
    )

    static let countingLoop = SampleProgram(
        name: "Counting Loop",
        source: """
            ; Sums 1+2+3+4+5 into AX using LOOP/CX as the counter - the
            ; classic 8086 loop idiom.
            MOV AX, 0
            MOV CX, 5
            MOV BX, 1
            top:
            ADD AX, BX
            INC BX
            LOOP top
            HLT
            """
    )

    static let keyboardEcho = SampleProgram(
        name: "Keyboard Echo",
        source: """
            ; Reads one keystroke (INT 16h) and prints it right back out
            ; (INT 21h) - Run this one and type a key when prompted.
            MOV AH, 0
            INT 16h
            MOV DL, AL
            MOV AH, 2
            INT 21h
            HLT
            """
    )

    static let subroutineCall = SampleProgram(
        name: "Subroutine Call",
        source: """
            ; CALL/RET via the stack: doubles AX by calling a subroutine.
            ; A good one to try Step Over vs. Step Into on.
            MOV AX, 21
            CALL double
            HLT
            double:
            ADD AX, AX
            RET
            """
    )

    static let all: [SampleProgram] = [
        helloWorld, arithmeticAndFlags, countingLoop, keyboardEcho, subroutineCall,
    ]
}
