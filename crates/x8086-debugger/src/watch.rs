//! Watch expressions: small, live-evaluated views into CPU/memory state
//! that a debugger panel can list alongside the fixed Registers/Flags
//! panels.
//!
//! This module only understands the *fixed* vocabulary that needs no
//! outside knowledge: register and flag names, and raw `byte`/`word`
//! memory addresses. A bare identifier (a variable name like `msg`)
//! cannot be resolved here, since that requires the assembler's symbol
//! table - `x8086-debugger` deliberately has no dependency on
//! `x8086-assembler` (see the crate dependency graph in the project
//! plan), so named-variable watches are resolved one layer up, in
//! `x8086-emulator`, which already holds the last-assembled symbol table.
//! That layer tries this parser first and falls back to a symbol lookup
//! only if it returns `None`.

use x8086_cpu::Registers;
use x8086_isa::{Flag, Reg16, Reg8};
use x8086_memory::Memory;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WatchSize {
    Byte,
    Word,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WatchTarget {
    Reg16(Reg16),
    Reg8(Reg8),
    /// The whole 16-bit FLAGS register.
    Flags,
    Flag(Flag),
    Memory {
        address: u32,
        size: WatchSize,
    },
}

/// Parses the fixed register/flag/raw-address watch vocabulary described
/// in the module docs. Returns `None` for anything else (in particular,
/// bare identifiers - those are a variable-name lookup for the caller to
/// attempt instead), never an error, since "not a register or address"
/// isn't a malformed-input condition here, just a different case.
pub fn parse_register_or_raw_address(text: &str) -> Option<WatchTarget> {
    let upper = text.trim().to_ascii_uppercase();

    register_or_flag_target(&upper).or_else(|| parse_memory_target(&upper))
}

fn register_or_flag_target(upper: &str) -> Option<WatchTarget> {
    let reg16 = match upper {
        "AX" => Some(Reg16::Ax),
        "BX" => Some(Reg16::Bx),
        "CX" => Some(Reg16::Cx),
        "DX" => Some(Reg16::Dx),
        "SP" => Some(Reg16::Sp),
        "BP" => Some(Reg16::Bp),
        "SI" => Some(Reg16::Si),
        "DI" => Some(Reg16::Di),
        "CS" => Some(Reg16::Cs),
        "DS" => Some(Reg16::Ds),
        "ES" => Some(Reg16::Es),
        "SS" => Some(Reg16::Ss),
        "IP" => Some(Reg16::Ip),
        _ => None,
    };
    if let Some(reg) = reg16 {
        return Some(WatchTarget::Reg16(reg));
    }

    let reg8 = match upper {
        "AL" => Some(Reg8::Al),
        "AH" => Some(Reg8::Ah),
        "BL" => Some(Reg8::Bl),
        "BH" => Some(Reg8::Bh),
        "CL" => Some(Reg8::Cl),
        "CH" => Some(Reg8::Ch),
        "DL" => Some(Reg8::Dl),
        "DH" => Some(Reg8::Dh),
        _ => None,
    };
    if let Some(reg) = reg8 {
        return Some(WatchTarget::Reg8(reg));
    }

    if upper == "FLAGS" {
        return Some(WatchTarget::Flags);
    }

    let flag = match upper {
        "CF" => Some(Flag::Carry),
        "PF" => Some(Flag::Parity),
        "AF" => Some(Flag::AuxCarry),
        "ZF" => Some(Flag::Zero),
        "SF" => Some(Flag::Sign),
        "TF" => Some(Flag::Trap),
        "IF" => Some(Flag::Interrupt),
        "DF" => Some(Flag::Direction),
        "OF" => Some(Flag::Overflow),
        _ => None,
    };
    flag.map(WatchTarget::Flag)
}

/// Matches `byte [addr]` / `word[addr]` (case-insensitive, whitespace
/// around the brackets optional). A bare `[addr]` with no size prefix is
/// deliberately unsupported - a silently guessed default width would be
/// exactly the kind of surprising behavior that makes a watch value
/// misleading, so the caller must say which one they mean.
fn parse_memory_target(upper: &str) -> Option<WatchTarget> {
    let (size, rest) = match upper.strip_prefix("BYTE") {
        Some(rest) => (WatchSize::Byte, rest),
        None => (WatchSize::Word, upper.strip_prefix("WORD")?),
    };
    let inner = rest.trim_start().strip_prefix('[')?.strip_suffix(']')?;
    let address = parse_numeric_literal(inner.trim())?;
    Some(WatchTarget::Memory { address, size })
}

/// Accepts `0x1234`, `1234h`/`1234H`, or a bare decimal like `1234`.
fn parse_numeric_literal(text: &str) -> Option<u32> {
    if let Some(hex) = text.strip_prefix("0X") {
        return u32::from_str_radix(hex, 16).ok();
    }
    if let Some(hex) = text.strip_suffix('H') {
        return u32::from_str_radix(hex, 16).ok();
    }
    text.parse::<u32>().ok()
}

/// Reads the current value of a resolved watch target. Byte-sized values
/// are zero-extended into the returned `u16`.
pub fn evaluate(target: WatchTarget, registers: &Registers, memory: &Memory) -> u16 {
    match target {
        WatchTarget::Reg16(reg) => registers.get16(reg),
        WatchTarget::Reg8(reg) => registers.get8(reg) as u16,
        WatchTarget::Flags => registers.flags,
        WatchTarget::Flag(flag) => registers.get_flag(flag) as u16,
        WatchTarget::Memory {
            address,
            size: WatchSize::Byte,
        } => memory.read_u8(address) as u16,
        WatchTarget::Memory {
            address,
            size: WatchSize::Word,
        } => memory.read_u16(address),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_16_bit_general_and_segment_registers_case_insensitively() {
        assert_eq!(
            parse_register_or_raw_address("ax"),
            Some(WatchTarget::Reg16(Reg16::Ax))
        );
        assert_eq!(
            parse_register_or_raw_address("Cs"),
            Some(WatchTarget::Reg16(Reg16::Cs))
        );
        assert_eq!(
            parse_register_or_raw_address("ip"),
            Some(WatchTarget::Reg16(Reg16::Ip))
        );
    }

    #[test]
    fn parses_8_bit_register_halves() {
        assert_eq!(
            parse_register_or_raw_address("bh"),
            Some(WatchTarget::Reg8(Reg8::Bh))
        );
    }

    #[test]
    fn parses_flags_register_and_individual_flag_bits() {
        assert_eq!(
            parse_register_or_raw_address("FLAGS"),
            Some(WatchTarget::Flags)
        );
        assert_eq!(
            parse_register_or_raw_address("zf"),
            Some(WatchTarget::Flag(Flag::Zero))
        );
        assert_eq!(
            parse_register_or_raw_address("of"),
            Some(WatchTarget::Flag(Flag::Overflow))
        );
    }

    #[test]
    fn parses_sized_raw_memory_addresses_in_hex_or_decimal() {
        assert_eq!(
            parse_register_or_raw_address("byte [0x10]"),
            Some(WatchTarget::Memory {
                address: 0x10,
                size: WatchSize::Byte
            })
        );
        assert_eq!(
            parse_register_or_raw_address("WORD[1234h]"),
            Some(WatchTarget::Memory {
                address: 0x1234,
                size: WatchSize::Word
            })
        );
        assert_eq!(
            parse_register_or_raw_address("byte [16]"),
            Some(WatchTarget::Memory {
                address: 16,
                size: WatchSize::Byte
            })
        );
    }

    #[test]
    fn bare_bracketed_address_without_a_size_prefix_is_unsupported() {
        assert_eq!(parse_register_or_raw_address("[0x10]"), None);
    }

    #[test]
    fn unrecognized_text_is_none_not_an_error() {
        assert_eq!(parse_register_or_raw_address("msg"), None);
        assert_eq!(parse_register_or_raw_address(""), None);
        assert_eq!(parse_register_or_raw_address("byte [zz]"), None);
    }

    #[test]
    fn evaluate_reads_registers_flags_and_memory() {
        let mut registers = Registers::new();
        registers.ax = 0xBEEF;
        registers.set_flag(Flag::Zero, true);
        let mut memory = Memory::new();
        memory.write_u8(0x10, 0x42);
        memory.write_u16(0x20, 0xCAFE);

        assert_eq!(
            evaluate(WatchTarget::Reg16(Reg16::Ax), &registers, &memory),
            0xBEEF
        );
        assert_eq!(
            evaluate(WatchTarget::Flag(Flag::Zero), &registers, &memory),
            1
        );
        assert_eq!(
            evaluate(
                WatchTarget::Memory {
                    address: 0x10,
                    size: WatchSize::Byte
                },
                &registers,
                &memory
            ),
            0x42
        );
        assert_eq!(
            evaluate(
                WatchTarget::Memory {
                    address: 0x20,
                    size: WatchSize::Word
                },
                &registers,
                &memory
            ),
            0xCAFE
        );
    }
}
