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
}
