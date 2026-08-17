//! Pure decode: bytes -> `x8086_isa::Instruction`.
//!
//! No execution semantics live here, only recognizing byte sequences and
//! producing the shared instruction model. The full opcode table (every
//! ModRM/displacement form, string ops, 80186 additions) is built out
//! during the CPU-core phase; this scaffold wires up the shape of the
//! API other crates will call.

use x8086_isa::{Instruction, Mnemonic};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecodeError {
    /// Ran out of bytes while decoding an instruction that needed more.
    UnexpectedEndOfInput,
    /// The opcode byte doesn't map to any known 8086/80186 instruction.
    InvalidOpcode(u8),
}

/// Decode a single instruction starting at the front of `bytes`.
/// Returns the instruction and how many bytes it consumed.
pub fn decode_one(bytes: &[u8]) -> Result<(Instruction, usize), DecodeError> {
    let opcode = *bytes.first().ok_or(DecodeError::UnexpectedEndOfInput)?;
    match opcode {
        0xF4 => Ok((
            Instruction {
                mnemonic: Mnemonic::Hlt,
                operands: vec![],
                byte_len: Some(1),
            },
            1,
        )),
        other => Err(DecodeError::InvalidOpcode(other)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_hlt() {
        let (instr, len) = decode_one(&[0xF4]).expect("HLT should decode");
        assert_eq!(instr.mnemonic, Mnemonic::Hlt);
        assert_eq!(len, 1);
    }

    #[test]
    fn empty_input_is_an_error() {
        assert_eq!(decode_one(&[]), Err(DecodeError::UnexpectedEndOfInput));
    }

    #[test]
    fn unknown_opcode_is_reported() {
        assert_eq!(
            decode_one(&[0xFF]).unwrap_err(),
            DecodeError::InvalidOpcode(0xFF)
        );
    }
}
