//! Flag computation for arithmetic and logic operations.
//!
//! Everything here works in `u32` intermediates so byte- and word-width
//! operations share one implementation: the width only changes which
//! mask/sign-bit constants apply, not the arithmetic itself.

use crate::Registers;
use x8086_isa::{Flag, Width};

fn mask_and_sign_bit(width: Width) -> (u32, u32) {
    match width {
        Width::Byte => (0xFF, 0x80),
        Width::Word => (0xFFFF, 0x8000),
    }
}

fn parity_even(low_byte: u8) -> bool {
    low_byte.count_ones().is_multiple_of(2)
}

fn set_common_result_flags(regs: &mut Registers, result: u32, sign_bit: u32) {
    let sign = result & sign_bit != 0;
    regs.set_flag(Flag::Zero, result == 0);
    regs.set_flag(Flag::Sign, sign);
    regs.set_flag(Flag::Parity, parity_even(result as u8));
}

/// `dst + src + carry_in`, with CF/AF/OF/ZF/SF/PF all set per the real
/// 8086 ADD/ADC semantics. Returns the (width-masked) result.
pub fn add_with_flags(
    regs: &mut Registers,
    dst: u16,
    src: u16,
    carry_in: bool,
    width: Width,
) -> u16 {
    let (mask, sign_bit) = mask_and_sign_bit(width);
    let a = dst as u32 & mask;
    let b = src as u32 & mask;
    let c = carry_in as u32;

    let full = a + b + c;
    let result = full & mask;

    let carry_out = full > mask;
    let aux_carry = (a & 0xF) + (b & 0xF) + c > 0xF;
    let a_sign = a & sign_bit != 0;
    let b_sign = b & sign_bit != 0;
    let overflow = (a_sign == b_sign) && (result & sign_bit != 0) != a_sign;

    regs.set_flag(Flag::Carry, carry_out);
    regs.set_flag(Flag::AuxCarry, aux_carry);
    regs.set_flag(Flag::Overflow, overflow);
    set_common_result_flags(regs, result, sign_bit);

    result as u16
}

/// `dst - src - borrow_in`, with CF/AF/OF/ZF/SF/PF all set per the real
/// 8086 SUB/SBB/CMP semantics (CMP just discards the result). Returns
/// the (width-masked) result.
pub fn sub_with_flags(
    regs: &mut Registers,
    dst: u16,
    src: u16,
    borrow_in: bool,
    width: Width,
) -> u16 {
    let (mask, sign_bit) = mask_and_sign_bit(width);
    let a = dst as u32 & mask;
    let b = src as u32 & mask;
    let c = borrow_in as u32;

    let subtrahend = b + c;
    let result = a.wrapping_sub(subtrahend) & mask;

    let borrow_out = subtrahend > a;
    let aux_borrow = (b & 0xF) + c > (a & 0xF);
    let a_sign = a & sign_bit != 0;
    let b_sign = b & sign_bit != 0;
    let overflow = (a_sign != b_sign) && (result & sign_bit != 0) != a_sign;

    regs.set_flag(Flag::Carry, borrow_out);
    regs.set_flag(Flag::AuxCarry, aux_borrow);
    regs.set_flag(Flag::Overflow, overflow);
    set_common_result_flags(regs, result, sign_bit);

    result as u16
}

/// AND/OR/XOR/TEST: clears CF and OF (bitwise ops can't carry or
/// overflow), sets ZF/SF/PF from the result, and clears AF, which the
/// 8086 leaves undefined here - clearing it keeps behavior deterministic
/// rather than leaving stale state from a previous instruction.
pub fn set_flags_after_logic(regs: &mut Registers, result: u16, width: Width) {
    let (mask, sign_bit) = mask_and_sign_bit(width);
    let masked = result as u32 & mask;
    regs.set_flag(Flag::Carry, false);
    regs.set_flag(Flag::Overflow, false);
    regs.set_flag(Flag::AuxCarry, false);
    set_common_result_flags(regs, masked, sign_bit);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn add_sets_zero_flag_on_exact_wraparound() {
        let mut regs = Registers::new();
        let result = add_with_flags(&mut regs, 0xFFFF, 1, false, Width::Word);
        assert_eq!(result, 0);
        assert!(regs.get_flag(Flag::Zero));
        assert!(regs.get_flag(Flag::Carry));
    }

    #[test]
    fn add_byte_width_carries_out_of_bit_7_not_bit_15() {
        let mut regs = Registers::new();
        let result = add_with_flags(&mut regs, 0xFF, 0x01, false, Width::Byte);
        assert_eq!(result, 0x00);
        assert!(regs.get_flag(Flag::Carry));
        assert!(regs.get_flag(Flag::Zero));
    }

    #[test]
    fn add_detects_signed_overflow_positive_plus_positive() {
        // 0x7FFF + 1 = 0x8000: two positives producing a negative result.
        let mut regs = Registers::new();
        add_with_flags(&mut regs, 0x7FFF, 1, false, Width::Word);
        assert!(regs.get_flag(Flag::Overflow));
        assert!(regs.get_flag(Flag::Sign));
    }

    #[test]
    fn add_does_not_flag_overflow_when_signs_differ() {
        // A large positive plus a negative can never signed-overflow.
        let mut regs = Registers::new();
        add_with_flags(&mut regs, 0x7FFF, 0xFFFF, false, Width::Word); // 0x7FFF + (-1)
        assert!(!regs.get_flag(Flag::Overflow));
    }

    #[test]
    fn add_sets_aux_carry_on_nibble_boundary() {
        let mut regs = Registers::new();
        add_with_flags(&mut regs, 0x0F, 0x01, false, Width::Byte);
        assert!(regs.get_flag(Flag::AuxCarry));
    }

    #[test]
    fn adc_includes_incoming_carry() {
        let mut regs = Registers::new();
        let result = add_with_flags(&mut regs, 1, 1, true, Width::Word);
        assert_eq!(result, 3);
    }

    #[test]
    fn sub_sets_carry_as_borrow_when_subtrahend_is_larger() {
        let mut regs = Registers::new();
        let result = sub_with_flags(&mut regs, 0, 1, false, Width::Word);
        assert_eq!(result, 0xFFFF);
        assert!(regs.get_flag(Flag::Carry));
        assert!(regs.get_flag(Flag::Sign));
    }

    #[test]
    fn sub_equal_operands_sets_zero_and_clears_carry() {
        let mut regs = Registers::new();
        let result = sub_with_flags(&mut regs, 5, 5, false, Width::Word);
        assert_eq!(result, 0);
        assert!(regs.get_flag(Flag::Zero));
        assert!(!regs.get_flag(Flag::Carry));
    }

    #[test]
    fn sub_detects_signed_overflow_negative_minus_positive() {
        // 0x8000 (i16::MIN) - 1 overflows into positive territory.
        let mut regs = Registers::new();
        sub_with_flags(&mut regs, 0x8000, 1, false, Width::Word);
        assert!(regs.get_flag(Flag::Overflow));
    }

    #[test]
    fn sbb_includes_incoming_borrow() {
        let mut regs = Registers::new();
        let result = sub_with_flags(&mut regs, 5, 2, true, Width::Word);
        assert_eq!(result, 2); // 5 - 2 - 1
    }

    #[test]
    fn logic_ops_clear_carry_and_overflow_regardless_of_prior_state() {
        let mut regs = Registers::new();
        regs.set_flag(Flag::Carry, true);
        regs.set_flag(Flag::Overflow, true);
        set_flags_after_logic(&mut regs, 0x00FF, Width::Byte);
        assert!(!regs.get_flag(Flag::Carry));
        assert!(!regs.get_flag(Flag::Overflow));
        assert!(!regs.get_flag(Flag::Zero));
    }

    #[test]
    fn logic_ops_set_parity_from_low_byte() {
        let mut regs = Registers::new();
        set_flags_after_logic(&mut regs, 0b0000_0011, Width::Byte); // two set bits: even parity
        assert!(regs.get_flag(Flag::Parity));
        set_flags_after_logic(&mut regs, 0b0000_0111, Width::Byte); // three set bits: odd parity
        assert!(!regs.get_flag(Flag::Parity));
    }
}
