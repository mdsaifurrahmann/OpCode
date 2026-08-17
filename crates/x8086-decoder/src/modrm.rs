//! ModRM byte decoding: the addressing-mode machinery shared by almost
//! every multi-byte 8086 instruction.
//!
//! Byte layout: `mod(2) reg(3) rm(3)`. `reg` is either the second operand
//! register or an opcode-extension selector (the caller decides which);
//! `mod`+`rm` together select a register (`mod==11`) or one of the eight
//! base+index addressing forms, with 0/8/16-bit displacement.

use crate::{read_i16, read_i8, DecodeError};
use x8086_isa::{Operand, Reg16, Reg8, Width};

#[derive(Debug)]
pub struct ModRmResult {
    /// The raw 3-bit `reg` field - either a register operand or an
    /// opcode-extension selector, depending on the instruction.
    pub reg_field: u8,
    pub rm_operand: Operand,
    /// Bytes consumed by the ModRM byte itself plus any displacement.
    pub consumed: usize,
}

pub fn decode_modrm(bytes: &[u8], width: Width) -> Result<ModRmResult, DecodeError> {
    let modrm_byte = *bytes.first().ok_or(DecodeError::UnexpectedEndOfInput)?;
    let mod_bits = (modrm_byte >> 6) & 0b11;
    let reg_field = (modrm_byte >> 3) & 0b111;
    let rm_field = modrm_byte & 0b111;

    if mod_bits == 0b11 {
        let rm_operand = match width {
            Width::Byte => Operand::Reg8(Reg8::from_index(rm_field)),
            Width::Word => Operand::Reg16(Reg16::from_index(rm_field)),
        };
        return Ok(ModRmResult {
            reg_field,
            rm_operand,
            consumed: 1,
        });
    }

    // mod==00, rm==110 is the one irregular case: no base/index register
    // at all, just a direct 16-bit address.
    if mod_bits == 0b00 && rm_field == 0b110 {
        let disp = read_i16(bytes, 1)?;
        return Ok(ModRmResult {
            reg_field,
            rm_operand: Operand::mem_direct(disp as i32),
            consumed: 3,
        });
    }

    let (base, index) = match rm_field {
        0b000 => (Some(Reg16::Bx), Some(Reg16::Si)),
        0b001 => (Some(Reg16::Bx), Some(Reg16::Di)),
        0b010 => (Some(Reg16::Bp), Some(Reg16::Si)),
        0b011 => (Some(Reg16::Bp), Some(Reg16::Di)),
        0b100 => (Some(Reg16::Si), None),
        0b101 => (Some(Reg16::Di), None),
        0b110 => (Some(Reg16::Bp), None), // mod==00 case already handled above
        0b111 => (Some(Reg16::Bx), None),
        _ => unreachable!("rm_field is masked to 3 bits"),
    };

    let (displacement, disp_len) = match mod_bits {
        0b00 => (0i32, 0usize),
        0b01 => (read_i8(bytes, 1)? as i32, 1usize),
        0b10 => (read_i16(bytes, 1)? as i32, 2usize),
        _ => unreachable!("mod_bits is masked to 2 bits and 0b11 was handled above"),
    };

    Ok(ModRmResult {
        reg_field,
        rm_operand: Operand::mem(base, index, displacement),
        consumed: 1 + disp_len,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mod11_decodes_register_direct() {
        // 11 000 001 -> mod=11, reg=000, rm=001
        let result = decode_modrm(&[0b11_000_001], Width::Word).unwrap();
        assert_eq!(result.reg_field, 0);
        assert_eq!(result.rm_operand, Operand::Reg16(Reg16::Cx));
        assert_eq!(result.consumed, 1);
    }

    #[test]
    fn mod11_respects_byte_width() {
        let result = decode_modrm(&[0b11_000_001], Width::Byte).unwrap();
        assert_eq!(result.rm_operand, Operand::Reg8(Reg8::Cl));
    }

    #[test]
    fn mod00_rm110_is_direct_address() {
        // 00 000 110 + disp16(0x1234 little-endian)
        let result = decode_modrm(&[0b00_000_110, 0x34, 0x12], Width::Word).unwrap();
        assert_eq!(result.rm_operand, Operand::mem_direct(0x1234));
        assert_eq!(result.consumed, 3);
    }

    #[test]
    fn mod00_bx_si_has_no_displacement() {
        // [BX+SI]: mod=00, rm=000
        let result = decode_modrm(&[0b00_000_000], Width::Word).unwrap();
        assert_eq!(
            result.rm_operand,
            Operand::mem(Some(Reg16::Bx), Some(Reg16::Si), 0)
        );
        assert_eq!(result.consumed, 1);
    }

    #[test]
    fn mod01_adds_signed_8bit_displacement() {
        // [BP+DI-2]: mod=01, rm=011, disp8 = 0xFE (-2)
        let result = decode_modrm(&[0b01_000_011, 0xFE], Width::Word).unwrap();
        assert_eq!(
            result.rm_operand,
            Operand::mem(Some(Reg16::Bp), Some(Reg16::Di), -2)
        );
        assert_eq!(result.consumed, 2);
    }

    #[test]
    fn mod10_adds_16bit_displacement() {
        // [SI+0x0100]: mod=10, rm=100
        let result = decode_modrm(&[0b10_000_100, 0x00, 0x01], Width::Word).unwrap();
        assert_eq!(
            result.rm_operand,
            Operand::mem(Some(Reg16::Si), None, 0x0100)
        );
        assert_eq!(result.consumed, 3);
    }

    #[test]
    fn mod00_bp_alone_is_disallowed_falls_back_to_bp_base() {
        // [BP] with no displacement doesn't exist in the encoding table;
        // mod=00 rm=110 is reserved for direct addressing instead. This
        // test documents mod=01 rm=110 (0-displacement-in-practice via
        // disp8=0) as the real way to encode a bare [BP].
        let result = decode_modrm(&[0b01_000_110, 0x00], Width::Word).unwrap();
        assert_eq!(result.rm_operand, Operand::mem(Some(Reg16::Bp), None, 0));
    }

    #[test]
    fn reports_unexpected_end_of_input_when_displacement_is_missing() {
        assert_eq!(
            decode_modrm(&[0b01_000_000], Width::Word).unwrap_err(),
            DecodeError::UnexpectedEndOfInput
        );
    }
}
