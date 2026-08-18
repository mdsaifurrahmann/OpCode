//! Lookup tables for opcode "groups": instruction families that share one
//! encoding pattern and are distinguished only by a 3-bit selector (either
//! the top 3 bits of the opcode byte itself, for the arithmetic/logic
//! group, or the ModRM `reg` field, for the 80h/81h/83h immediate group).

use x8086_isa::Mnemonic;

/// The eight arithmetic/logic operations that share the `00-3D` encoding
/// pattern (`ADD=0x00, OR=0x08, ADC=0x10, SBB=0x18, AND=0x20, SUB=0x28,
/// XOR=0x30, CMP=0x38`), indexed by `(opcode >> 3) & 0b111`.
pub const ARITHMETIC_GROUP: [Mnemonic; 8] = [
    Mnemonic::Add,
    Mnemonic::Or,
    Mnemonic::Adc,
    Mnemonic::Sbb,
    Mnemonic::And,
    Mnemonic::Sub,
    Mnemonic::Xor,
    Mnemonic::Cmp,
];

/// Same eight operations, indexed by the ModRM `reg` field for the
/// `80h`/`81h`/`83h` immediate-group opcodes.
pub fn arithmetic_group_from_reg_field(reg_field: u8) -> Mnemonic {
    ARITHMETIC_GROUP[(reg_field & 0b111) as usize]
}

/// The eight shift/rotate operations sharing the `D0-D3`/`C0-C1` encoding
/// pattern, indexed by the ModRM `reg` field. Reg field `6` is reserved on
/// real 8086/80186 hardware; we map it to `SHL` (matching documented
/// behavior of undefined-but-not-trapping opcode extensions) rather than
/// rejecting it outright.
pub const SHIFT_ROTATE_GROUP: [Mnemonic; 8] = [
    Mnemonic::Rol,
    Mnemonic::Ror,
    Mnemonic::Rcl,
    Mnemonic::Rcr,
    Mnemonic::Shl,
    Mnemonic::Shr,
    Mnemonic::Shl,
    Mnemonic::Sar,
];

pub fn shift_rotate_group_from_reg_field(reg_field: u8) -> Mnemonic {
    SHIFT_ROTATE_GROUP[(reg_field & 0b111) as usize]
}

/// The `F6`/`F7` unary group, indexed by the ModRM `reg` field. Reg fields
/// `0` and `1` are both `TEST r/m, imm` (a duplicated encoding on real
/// hardware).
pub const UNARY_GROUP: [Mnemonic; 8] = [
    Mnemonic::Test,
    Mnemonic::Test,
    Mnemonic::Not,
    Mnemonic::Neg,
    Mnemonic::Mul,
    Mnemonic::Imul,
    Mnemonic::Div,
    Mnemonic::Idiv,
];

pub fn unary_group_from_reg_field(reg_field: u8) -> Mnemonic {
    UNARY_GROUP[(reg_field & 0b111) as usize]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arithmetic_group_matches_the_intel_encoding_table() {
        let expected = [
            Mnemonic::Add,
            Mnemonic::Or,
            Mnemonic::Adc,
            Mnemonic::Sbb,
            Mnemonic::And,
            Mnemonic::Sub,
            Mnemonic::Xor,
            Mnemonic::Cmp,
        ];
        assert_eq!(ARITHMETIC_GROUP, expected);
        for (index, mnemonic) in expected.into_iter().enumerate() {
            assert_eq!(arithmetic_group_from_reg_field(index as u8), mnemonic);
        }
    }

    #[test]
    fn shift_rotate_group_matches_the_intel_encoding_table() {
        assert_eq!(shift_rotate_group_from_reg_field(0), Mnemonic::Rol);
        assert_eq!(shift_rotate_group_from_reg_field(1), Mnemonic::Ror);
        assert_eq!(shift_rotate_group_from_reg_field(2), Mnemonic::Rcl);
        assert_eq!(shift_rotate_group_from_reg_field(3), Mnemonic::Rcr);
        assert_eq!(shift_rotate_group_from_reg_field(4), Mnemonic::Shl);
        assert_eq!(shift_rotate_group_from_reg_field(5), Mnemonic::Shr);
        assert_eq!(shift_rotate_group_from_reg_field(7), Mnemonic::Sar);
    }

    #[test]
    fn unary_group_matches_the_intel_encoding_table() {
        assert_eq!(unary_group_from_reg_field(0), Mnemonic::Test);
        assert_eq!(unary_group_from_reg_field(1), Mnemonic::Test);
        assert_eq!(unary_group_from_reg_field(2), Mnemonic::Not);
        assert_eq!(unary_group_from_reg_field(3), Mnemonic::Neg);
        assert_eq!(unary_group_from_reg_field(4), Mnemonic::Mul);
        assert_eq!(unary_group_from_reg_field(5), Mnemonic::Imul);
        assert_eq!(unary_group_from_reg_field(6), Mnemonic::Div);
        assert_eq!(unary_group_from_reg_field(7), Mnemonic::Idiv);
    }
}
