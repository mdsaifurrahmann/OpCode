//! Shared 8086/80186 instruction, register, and operand model.
//!
//! This crate is the single source of truth for what an "instruction" is.
//! `x8086-decoder` turns bytes into these types; `x8086-assembler` turns
//! these types into bytes. Keeping both on one model is what keeps them
//! from drifting apart.

/// A 16-bit general-purpose or segment register.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Reg16 {
    Ax,
    Bx,
    Cx,
    Dx,
    Sp,
    Bp,
    Si,
    Di,
    Cs,
    Ds,
    Es,
    Ss,
    Ip,
}

/// An 8-bit general-purpose register (the high/low halves of AX/BX/CX/DX).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Reg8 {
    Al,
    Ah,
    Bl,
    Bh,
    Cl,
    Ch,
    Dl,
    Dh,
}

/// A single bit of the 8086 FLAGS register.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Flag {
    Carry,
    Parity,
    AuxCarry,
    Zero,
    Sign,
    Trap,
    Interrupt,
    Direction,
    Overflow,
}

/// Operand width, decoded from an opcode's `w` bit (or implied by the
/// instruction). Determines both how many bytes a memory operand reads/
/// writes and how flags (overflow, carry) are computed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Width {
    Byte,
    Word,
}

/// The 16 condition codes that gate conditional jumps (opcodes 70h-7Fh).
/// Kept as data on `Mnemonic::Jcc` rather than 16 separate mnemonics,
/// since they're one instruction family with one encoding pattern.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Condition {
    Overflow,
    NotOverflow,
    Below,
    AboveOrEqual,
    Equal,
    NotEqual,
    BelowOrEqual,
    Above,
    Sign,
    NotSign,
    Parity,
    NotParity,
    Less,
    GreaterOrEqual,
    LessOrEqual,
    Greater,
}

/// Instruction mnemonics. Populated incrementally as decoder coverage
/// grows; `Unknown` is a placeholder for bytes that don't yet map to a
/// real variant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Mnemonic {
    // Data transfer
    Mov,
    Push,
    Pop,
    Xchg,
    Lea,
    // Arithmetic
    Add,
    Adc,
    Sub,
    Sbb,
    Cmp,
    Inc,
    Dec,
    // Logic
    And,
    Or,
    Xor,
    Test,
    // Control transfer
    Jmp,
    Jcc(Condition),
    Loop,
    Loope,
    Loopne,
    Jcxz,
    Call,
    Ret,
    Int,
    Int3,
    Iret,
    // Processor control
    Hlt,
    Nop,
    Clc,
    Stc,
    Cmc,
    Cld,
    Std,
    Cli,
    Sti,
    Unknown,
}

/// An operand to an instruction: a register, an immediate value, or a
/// memory reference expressed as segment:offset addressing components.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Operand {
    Reg16(Reg16),
    Reg8(Reg8),
    Immediate(i32),
    Memory {
        segment_override: Option<Reg16>,
        base: Option<Reg16>,
        index: Option<Reg16>,
        displacement: i32,
    },
}

impl Operand {
    /// A direct-addressed memory operand: `[disp16]`, no base or index.
    pub fn mem_direct(displacement: i32) -> Self {
        Operand::Memory {
            segment_override: None,
            base: None,
            index: None,
            displacement,
        }
    }

    /// A base/index/displacement memory operand, e.g. `[BX+SI+4]`.
    pub fn mem(base: Option<Reg16>, index: Option<Reg16>, displacement: i32) -> Self {
        Operand::Memory {
            segment_override: None,
            base,
            index,
            displacement,
        }
    }

    pub fn with_segment_override(self, segment: Reg16) -> Self {
        match self {
            Operand::Memory {
                base,
                index,
                displacement,
                ..
            } => Operand::Memory {
                segment_override: Some(segment),
                base,
                index,
                displacement,
            },
            other => other,
        }
    }
}

/// A decoded (or about-to-be-encoded) instruction, independent of whether
/// it came from bytes (decoder) or source text (assembler).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Instruction {
    pub mnemonic: Mnemonic,
    pub operands: Vec<Operand>,
    /// Operand width for instructions where it matters (arithmetic,
    /// logic, data transfer); `None` for width-independent instructions
    /// like `HLT` or `JMP`.
    pub width: Option<Width>,
    /// Length in bytes once encoded/decoded.
    pub byte_len: u8,
}

impl Instruction {
    pub fn new(
        mnemonic: Mnemonic,
        operands: Vec<Operand>,
        width: Option<Width>,
        byte_len: u8,
    ) -> Self {
        Self {
            mnemonic,
            operands,
            width,
            byte_len,
        }
    }
}

impl Reg16 {
    pub fn is_segment(self) -> bool {
        matches!(self, Reg16::Cs | Reg16::Ds | Reg16::Es | Reg16::Ss)
    }

    /// Decode a 3-bit general-purpose register field (used as both the
    /// ModRM `reg` field and the low 3 bits of register-form opcodes)
    /// into a 16-bit register.
    pub fn from_index(index: u8) -> Reg16 {
        match index & 0b111 {
            0 => Reg16::Ax,
            1 => Reg16::Cx,
            2 => Reg16::Dx,
            3 => Reg16::Bx,
            4 => Reg16::Sp,
            5 => Reg16::Bp,
            6 => Reg16::Si,
            7 => Reg16::Di,
            _ => unreachable!("index & 0b111 is always in 0..=7"),
        }
    }

    /// Decode a 2-bit segment-register field (bits 4-7 are reserved/
    /// undefined on 8086/80186), as used by `MOV Sreg,r/m16` and
    /// segment-register `PUSH`/`POP`.
    pub fn segment_from_index(index: u8) -> Option<Reg16> {
        match index & 0b111 {
            0 => Some(Reg16::Es),
            1 => Some(Reg16::Cs),
            2 => Some(Reg16::Ss),
            3 => Some(Reg16::Ds),
            _ => None,
        }
    }

    /// The inverse of `from_index` - the encoder's counterpart to the
    /// decoder's register-field lookup. `None` for registers that have
    /// no plain 3-bit general-purpose encoding (`IP` is never directly
    /// addressable this way).
    pub fn to_index(self) -> Option<u8> {
        match self {
            Reg16::Ax => Some(0),
            Reg16::Cx => Some(1),
            Reg16::Dx => Some(2),
            Reg16::Bx => Some(3),
            Reg16::Sp => Some(4),
            Reg16::Bp => Some(5),
            Reg16::Si => Some(6),
            Reg16::Di => Some(7),
            Reg16::Cs | Reg16::Ds | Reg16::Es | Reg16::Ss | Reg16::Ip => None,
        }
    }

    /// The inverse of `segment_from_index`.
    pub fn to_segment_index(self) -> Option<u8> {
        match self {
            Reg16::Es => Some(0),
            Reg16::Cs => Some(1),
            Reg16::Ss => Some(2),
            Reg16::Ds => Some(3),
            _ => None,
        }
    }
}

impl Reg8 {
    /// Decode a 3-bit general-purpose register field into an 8-bit
    /// register (the same field layout as `Reg16::from_index`, just
    /// interpreted under `w=0`).
    pub fn from_index(index: u8) -> Reg8 {
        match index & 0b111 {
            0 => Reg8::Al,
            1 => Reg8::Cl,
            2 => Reg8::Dl,
            3 => Reg8::Bl,
            4 => Reg8::Ah,
            5 => Reg8::Ch,
            6 => Reg8::Dh,
            7 => Reg8::Bh,
            _ => unreachable!("index & 0b111 is always in 0..=7"),
        }
    }

    /// The inverse of `from_index`.
    pub fn to_index(self) -> u8 {
        match self {
            Reg8::Al => 0,
            Reg8::Cl => 1,
            Reg8::Dl => 2,
            Reg8::Bl => 3,
            Reg8::Ah => 4,
            Reg8::Ch => 5,
            Reg8::Dh => 6,
            Reg8::Bh => 7,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reg16_to_index_round_trips_with_from_index() {
        let all = [
            Reg16::Ax,
            Reg16::Cx,
            Reg16::Dx,
            Reg16::Bx,
            Reg16::Sp,
            Reg16::Bp,
            Reg16::Si,
            Reg16::Di,
        ];
        for reg in all {
            let index = reg.to_index().unwrap();
            assert_eq!(Reg16::from_index(index), reg);
        }
    }

    #[test]
    fn reg16_to_index_is_none_for_segment_and_ip_registers() {
        for reg in [Reg16::Cs, Reg16::Ds, Reg16::Es, Reg16::Ss, Reg16::Ip] {
            assert_eq!(reg.to_index(), None);
        }
    }

    #[test]
    fn reg16_to_segment_index_round_trips_with_segment_from_index() {
        for reg in [Reg16::Es, Reg16::Cs, Reg16::Ss, Reg16::Ds] {
            let index = reg.to_segment_index().unwrap();
            assert_eq!(Reg16::segment_from_index(index), Some(reg));
        }
    }

    #[test]
    fn reg8_to_index_round_trips_with_from_index() {
        let all = [
            Reg8::Al,
            Reg8::Cl,
            Reg8::Dl,
            Reg8::Bl,
            Reg8::Ah,
            Reg8::Ch,
            Reg8::Dh,
            Reg8::Bh,
        ];
        for reg in all {
            assert_eq!(Reg8::from_index(reg.to_index()), reg);
        }
    }

    #[test]
    fn segment_registers_are_classified_correctly() {
        assert!(Reg16::Cs.is_segment());
        assert!(Reg16::Ds.is_segment());
        assert!(!Reg16::Ax.is_segment());
        assert!(!Reg16::Ip.is_segment());
    }

    #[test]
    fn instruction_can_be_constructed_with_no_operands() {
        let hlt = Instruction::new(Mnemonic::Hlt, vec![], None, 1);
        assert_eq!(hlt.mnemonic, Mnemonic::Hlt);
        assert!(hlt.operands.is_empty());
    }

    #[test]
    fn reg16_from_index_matches_intel_encoding_table() {
        let expected = [
            (0, Reg16::Ax),
            (1, Reg16::Cx),
            (2, Reg16::Dx),
            (3, Reg16::Bx),
            (4, Reg16::Sp),
            (5, Reg16::Bp),
            (6, Reg16::Si),
            (7, Reg16::Di),
        ];
        for (index, reg) in expected {
            assert_eq!(Reg16::from_index(index), reg);
        }
    }

    #[test]
    fn reg8_from_index_matches_intel_encoding_table() {
        let expected = [
            (0, Reg8::Al),
            (1, Reg8::Cl),
            (2, Reg8::Dl),
            (3, Reg8::Bl),
            (4, Reg8::Ah),
            (5, Reg8::Ch),
            (6, Reg8::Dh),
            (7, Reg8::Bh),
        ];
        for (index, reg) in expected {
            assert_eq!(Reg8::from_index(index), reg);
        }
    }

    #[test]
    fn segment_from_index_covers_valid_range_and_rejects_the_rest() {
        assert_eq!(Reg16::segment_from_index(0), Some(Reg16::Es));
        assert_eq!(Reg16::segment_from_index(1), Some(Reg16::Cs));
        assert_eq!(Reg16::segment_from_index(2), Some(Reg16::Ss));
        assert_eq!(Reg16::segment_from_index(3), Some(Reg16::Ds));
        assert_eq!(Reg16::segment_from_index(4), None);
        assert_eq!(Reg16::segment_from_index(7), None);
    }

    #[test]
    fn with_segment_override_only_affects_memory_operands() {
        let mem = Operand::mem(Some(Reg16::Bx), None, 0).with_segment_override(Reg16::Es);
        assert_eq!(
            mem,
            Operand::Memory {
                segment_override: Some(Reg16::Es),
                base: Some(Reg16::Bx),
                index: None,
                displacement: 0
            }
        );
        let reg = Operand::Reg16(Reg16::Ax).with_segment_override(Reg16::Es);
        assert_eq!(reg, Operand::Reg16(Reg16::Ax));
    }
}
