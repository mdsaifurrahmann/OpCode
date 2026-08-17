//! Shared 8086/80186 instruction, register, and operand model.
//!
//! This crate is the single source of truth for what an "instruction" is.
//! `x8086-decoder` turns bytes into these types; `x8086-assembler` turns
//! these types into bytes. Keeping both on one model is what keeps them
//! from drifting apart. The opcode table itself is filled in during the
//! CPU-core and assembler build phases; this scaffold establishes the
//! shapes everything else depends on.

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

/// Instruction mnemonics. Populated incrementally as the decoder and
/// assembler are built out; `Unknown` is a placeholder for bytes/text
/// that don't yet map to a real variant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Mnemonic {
    Mov,
    Add,
    Sub,
    Int,
    Hlt,
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

/// A decoded (or about-to-be-encoded) instruction, independent of whether
/// it came from bytes (decoder) or source text (assembler).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Instruction {
    pub mnemonic: Mnemonic,
    pub operands: Vec<Operand>,
    /// Length in bytes once encoded; `None` until known.
    pub byte_len: Option<u8>,
}

impl Reg16 {
    pub fn is_segment(self) -> bool {
        matches!(self, Reg16::Cs | Reg16::Ds | Reg16::Es | Reg16::Ss)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn segment_registers_are_classified_correctly() {
        assert!(Reg16::Cs.is_segment());
        assert!(Reg16::Ds.is_segment());
        assert!(!Reg16::Ax.is_segment());
        assert!(!Reg16::Ip.is_segment());
    }

    #[test]
    fn instruction_can_be_constructed_with_no_operands() {
        let hlt = Instruction {
            mnemonic: Mnemonic::Hlt,
            operands: vec![],
            byte_len: Some(1),
        };
        assert_eq!(hlt.mnemonic, Mnemonic::Hlt);
        assert!(hlt.operands.is_empty());
    }
}
