//! Cross-validates our hand-written decoder against `iced-x86`, a mature,
//! independently-implemented x86 decoder, for every opcode form we
//! support. `iced-x86` is a dev-dependency only (see the comment in
//! Cargo.toml) - its disassembly conventions don't match emu8086's, and
//! it decodes far more of the x86 surface than 8086/80186, so it isn't
//! suitable as our runtime decoder. It's an excellent oracle, though: if
//! our decoder and iced-x86 agree on instruction length and mnemonic
//! family for a given byte sequence, that's strong independent evidence
//! our ModRM/displacement/immediate parsing is correct.
//!
//! The check is intentionally two-tiered: instruction *length* agreement
//! is the strongest signal (it means our addressing-mode/displacement/
//! immediate-size parsing lines up byte-for-byte with a mature decoder),
//! checked for every case; mnemonic *family* agreement is checked too,
//! mapped through `matches_iced_mnemonic` since the two crates don't
//! share a mnemonic type.

use iced_x86::{Decoder, DecoderOptions, Mnemonic as Iced};
use x8086_decoder::decode_one;
use x8086_isa::Mnemonic as Ours;

fn iced_decode(bytes: &[u8]) -> iced_x86::Instruction {
    let mut decoder = Decoder::new(16, bytes, DecoderOptions::NONE);
    decoder.decode()
}

fn matches_iced_mnemonic(ours: &Ours, iced: Iced) -> bool {
    match ours {
        Ours::Mov => iced == Iced::Mov,
        Ours::Push => iced == Iced::Push,
        Ours::Pop => iced == Iced::Pop,
        Ours::Xchg => iced == Iced::Xchg,
        Ours::Lea => iced == Iced::Lea,
        Ours::Add => iced == Iced::Add,
        Ours::Adc => iced == Iced::Adc,
        Ours::Sub => iced == Iced::Sub,
        Ours::Sbb => iced == Iced::Sbb,
        Ours::Cmp => iced == Iced::Cmp,
        Ours::Inc => iced == Iced::Inc,
        Ours::Dec => iced == Iced::Dec,
        Ours::And => iced == Iced::And,
        Ours::Or => iced == Iced::Or,
        Ours::Xor => iced == Iced::Xor,
        Ours::Test => iced == Iced::Test,
        Ours::Jmp => iced == Iced::Jmp,
        Ours::Jcc(_) => matches!(
            iced,
            Iced::Ja
                | Iced::Jae
                | Iced::Jb
                | Iced::Jbe
                | Iced::Je
                | Iced::Jne
                | Iced::Jg
                | Iced::Jge
                | Iced::Jl
                | Iced::Jle
                | Iced::Jo
                | Iced::Jno
                | Iced::Jp
                | Iced::Jnp
                | Iced::Js
                | Iced::Jns
        ),
        Ours::Loop => iced == Iced::Loop,
        Ours::Loope => iced == Iced::Loope,
        Ours::Loopne => iced == Iced::Loopne,
        Ours::Jcxz => iced == Iced::Jcxz,
        Ours::Call => iced == Iced::Call,
        Ours::Ret => iced == Iced::Ret,
        Ours::Int => iced == Iced::Int,
        Ours::Int3 => iced == Iced::Int3,
        Ours::Iret => iced == Iced::Iret,
        Ours::Hlt => iced == Iced::Hlt,
        Ours::Nop => iced == Iced::Nop,
        Ours::Clc => iced == Iced::Clc,
        Ours::Stc => iced == Iced::Stc,
        Ours::Cmc => iced == Iced::Cmc,
        Ours::Cld => iced == Iced::Cld,
        Ours::Std => iced == Iced::Std,
        Ours::Cli => iced == Iced::Cli,
        Ours::Sti => iced == Iced::Sti,
        Ours::Shl => matches!(iced, Iced::Shl),
        Ours::Shr => iced == Iced::Shr,
        Ours::Sar => iced == Iced::Sar,
        Ours::Rol => iced == Iced::Rol,
        Ours::Ror => iced == Iced::Ror,
        Ours::Rcl => iced == Iced::Rcl,
        Ours::Rcr => iced == Iced::Rcr,
        Ours::Mul => iced == Iced::Mul,
        Ours::Imul => iced == Iced::Imul,
        Ours::Div => iced == Iced::Div,
        Ours::Idiv => iced == Iced::Idiv,
        Ours::Neg => iced == Iced::Neg,
        Ours::Not => iced == Iced::Not,
        Ours::Movsb => iced == Iced::Movsb,
        Ours::Movsw => iced == Iced::Movsw,
        Ours::Cmpsb => iced == Iced::Cmpsb,
        Ours::Cmpsw => iced == Iced::Cmpsw,
        Ours::Stosb => iced == Iced::Stosb,
        Ours::Stosw => iced == Iced::Stosw,
        Ours::Lodsb => iced == Iced::Lodsb,
        Ours::Lodsw => iced == Iced::Lodsw,
        Ours::Scasb => iced == Iced::Scasb,
        Ours::Scasw => iced == Iced::Scasw,
        Ours::Unknown => false,
    }
}

/// One representative byte sequence per opcode form this decoder
/// supports - not exhaustive over every register/displacement
/// permutation (the unit tests in `src/lib.rs` already cover those), but
/// covering every *encoding pattern* at least once.
fn supported_fixtures() -> Vec<Vec<u8>> {
    vec![
        vec![0x88, 0b11_000_001],             // MOV r/m8, r8
        vec![0x89, 0b11_000_001],             // MOV r/m16, r16
        vec![0x8A, 0b00_000_111],             // MOV r8, r/m8 ([BX])
        vec![0x8B, 0b01_000_011, 0x02],       // MOV r16, r/m16 ([BP+DI+2])
        vec![0x8C, 0b11_011_000],             // MOV r/m16, Sreg
        vec![0x8E, 0b11_011_000],             // MOV Sreg, r/m16
        vec![0x8D, 0b00_000_000],             // LEA r16, m16
        vec![0xA0, 0x00, 0x01],               // MOV AL, [imm16]
        vec![0xA1, 0x00, 0x01],               // MOV AX, [imm16]
        vec![0xA2, 0x00, 0x01],               // MOV [imm16], AL
        vec![0xA3, 0x00, 0x01],               // MOV [imm16], AX
        vec![0xB0, 0x42],                     // MOV AL, imm8
        vec![0xB8, 0x34, 0x12],               // MOV AX, imm16
        vec![0xC6, 0b00_000_111, 0x05],       // MOV r/m8, imm8
        vec![0xC7, 0b00_000_111, 0x05, 0x00], // MOV r/m16, imm16
        vec![0x86, 0b11_000_001],             // XCHG r/m8, r8
        vec![0x87, 0b11_000_001],             // XCHG r/m16, r16
        vec![0x93],                           // XCHG AX, BX
        vec![0x50],                           // PUSH AX
        vec![0x58],                           // POP AX
        vec![0x06],                           // PUSH ES
        vec![0x1F],                           // POP DS
        vec![0x8F, 0b00_000_111],             // POP r/m16
        vec![0xFF, 0b00_110_111],             // PUSH r/m16 (group FF /6)
        vec![0x00, 0b11_000_001],             // ADD r/m8, r8
        vec![0x01, 0b11_000_001],             // ADD r/m16, r16
        vec![0x02, 0b11_000_001],             // ADD r8, r/m8
        vec![0x03, 0b11_000_001],             // ADD r16, r/m16
        vec![0x04, 0x05],                     // ADD AL, imm8
        vec![0x05, 0x00, 0x01],               // ADD AX, imm16
        vec![0x08, 0b11_000_001],             // OR r/m8, r8
        vec![0x10, 0b11_000_001],             // ADC r/m8, r8
        vec![0x18, 0b11_000_001],             // SBB r/m8, r8
        vec![0x20, 0b11_000_001],             // AND r/m8, r8
        vec![0x28, 0b11_000_001],             // SUB r/m8, r8
        vec![0x30, 0b11_000_001],             // XOR r/m8, r8
        vec![0x38, 0b11_000_001],             // CMP r/m8, r8
        vec![0x80, 0b00_000_111, 0x05],       // immediate group, imm8
        vec![0x81, 0b00_000_111, 0x05, 0x00], // immediate group, imm16
        vec![0x83, 0b11_111_001, 0xFF],       // immediate group, imm8 sign-extended
        vec![0x84, 0b11_000_001],             // TEST r/m8, r8
        vec![0x85, 0b11_000_001],             // TEST r/m16, r16
        vec![0xA8, 0x0F],                     // TEST AL, imm8
        vec![0xA9, 0x0F, 0x00],               // TEST AX, imm16
        vec![0x40],                           // INC AX
        vec![0x48],                           // DEC AX
        vec![0xFE, 0b00_000_111],             // INC r/m8 (group FE)
        vec![0xFE, 0b00_001_111],             // DEC r/m8 (group FE)
        vec![0xFF, 0b00_000_111],             // INC r/m16 (group FF)
        vec![0xFF, 0b00_001_111],             // DEC r/m16 (group FF)
        vec![0x70, 0x02],                     // JO rel8
        vec![0x74, 0x02],                     // JE rel8
        vec![0x7F, 0x02],                     // JG rel8
        vec![0xEB, 0xFE],                     // JMP short
        vec![0xE9, 0x00, 0x01],               // JMP near
        vec![0xE0, 0xFC],                     // LOOPNE
        vec![0xE1, 0xFC],                     // LOOPE
        vec![0xE2, 0xFC],                     // LOOP
        vec![0xE3, 0xFC],                     // JCXZ
        vec![0xE8, 0x00, 0x01],               // CALL near
        vec![0xC2, 0x04, 0x00],               // RET imm16
        vec![0xC3],                           // RET
        vec![0xCC],                           // INT3
        vec![0xCD, 0x21],                     // INT imm8
        vec![0xCF],                           // IRET
        vec![0xF4],                           // HLT
        vec![0x90],                           // NOP
        vec![0xF5],                           // CMC
        vec![0xF8],                           // CLC
        vec![0xF9],                           // STC
        vec![0xFA],                           // CLI
        vec![0xFB],                           // STI
        vec![0xFC],                           // CLD
        vec![0xFD],                           // STD
        vec![0xD0, 0b11_100_000],             // SHL r/m8, 1
        vec![0xD1, 0b11_101_001],             // SHR r/m16, 1
        vec![0xD2, 0b11_111_000],             // SAR r/m8, CL
        vec![0xD3, 0b11_000_001],             // ROL r/m16, CL
        vec![0xC0, 0b11_010_000, 0x03],       // RCL r/m8, imm8 (80186)
        vec![0xC1, 0b11_011_001, 0x02],       // RCR r/m16, imm8 (80186)
        vec![0xF6, 0b11_010_000],             // NOT r/m8
        vec![0xF6, 0b11_011_001],             // NEG r/m8
        vec![0xF7, 0b11_100_000],             // MUL r/m16
        vec![0xF7, 0b11_101_001],             // IMUL r/m16
        vec![0xF6, 0b11_110_010],             // DIV r/m8
        vec![0xF7, 0b11_111_011],             // IDIV r/m16
        vec![0xF6, 0b00_000_111, 0x0F],       // TEST r/m8, imm8 (group F6)
        vec![0xA4],                           // MOVSB
        vec![0xA5],                           // MOVSW
        vec![0xA6],                           // CMPSB
        vec![0xA7],                           // CMPSW
        vec![0xAA],                           // STOSB
        vec![0xAB],                           // STOSW
        vec![0xAC],                           // LODSB
        vec![0xAD],                           // LODSW
        vec![0xAE],                           // SCASB
        vec![0xAF],                           // SCASW
        vec![0xF3, 0xA4],                     // REP MOVSB
        vec![0xF3, 0xA6],                     // REPE CMPSB
        vec![0xF2, 0xAE],                     // REPNE SCASB
    ]
}

#[test]
fn every_supported_fixture_agrees_with_iced_on_length_and_mnemonic_family() {
    for bytes in supported_fixtures() {
        let (ours, our_len) = decode_one(&bytes)
            .unwrap_or_else(|e| panic!("our decoder failed on {bytes:02x?}: {e:?}"));
        let iced_instr = iced_decode(&bytes);

        assert!(
            !iced_instr.is_invalid(),
            "iced-x86 could not decode {bytes:02x?} at all"
        );
        assert_eq!(
            our_len,
            iced_instr.len(),
            "length mismatch for {bytes:02x?}: ours={our_len} ({:?}), iced={} ({:?})",
            ours.mnemonic,
            iced_instr.len(),
            iced_instr.mnemonic()
        );
        assert!(
            matches_iced_mnemonic(&ours.mnemonic, iced_instr.mnemonic()),
            "mnemonic family mismatch for {bytes:02x?}: ours={:?}, iced={:?}",
            ours.mnemonic,
            iced_instr.mnemonic()
        );
    }
}
