//! Resolving `x8086_isa::Operand` values against the live register file
//! and memory - the piece that turns "this operand is `[BX+SI+4]`" into
//! an actual 20-bit physical address and back into a value.

use crate::Registers;
use x8086_isa::{Operand, Reg16, Width};
use x8086_memory::Memory;

/// 8086 real-mode default-segment rule: `[BP+...]` addressing defaults to
/// the stack segment, everything else defaults to the data segment,
/// unless a segment-override prefix says otherwise.
fn default_segment(base: Option<Reg16>) -> Reg16 {
    match base {
        Some(Reg16::Bp) => Reg16::Ss,
        _ => Reg16::Ds,
    }
}

/// The 16-bit offset a memory operand's addressing components resolve
/// to, *before* combining with a segment. Used directly by `LEA`, which
/// loads this offset without ever dereferencing memory.
pub fn effective_offset(
    base: Option<Reg16>,
    index: Option<Reg16>,
    displacement: i32,
    regs: &Registers,
) -> u16 {
    let base_val = base.map(|r| regs.get16(r)).unwrap_or(0);
    let index_val = index.map(|r| regs.get16(r)).unwrap_or(0);
    base_val
        .wrapping_add(index_val)
        .wrapping_add(displacement as u16)
}

fn effective_physical_address(
    segment_override: Option<Reg16>,
    base: Option<Reg16>,
    index: Option<Reg16>,
    displacement: i32,
    regs: &Registers,
) -> u32 {
    let offset = effective_offset(base, index, displacement, regs);
    let segment_reg = segment_override.unwrap_or_else(|| default_segment(base));
    Memory::resolve(regs.get16(segment_reg), offset)
}

pub fn read_operand(operand: &Operand, width: Width, regs: &Registers, memory: &Memory) -> u16 {
    match *operand {
        Operand::Reg8(reg) => regs.get8(reg) as u16,
        Operand::Reg16(reg) => regs.get16(reg),
        Operand::Immediate(value) => value as u16,
        Operand::Memory {
            segment_override,
            base,
            index,
            displacement,
        } => {
            let addr =
                effective_physical_address(segment_override, base, index, displacement, regs);
            match width {
                Width::Byte => memory.read_u8(addr) as u16,
                Width::Word => memory.read_u16(addr),
            }
        }
    }
}

pub fn write_operand(
    operand: &Operand,
    width: Width,
    value: u16,
    regs: &mut Registers,
    memory: &mut Memory,
) {
    match *operand {
        Operand::Reg8(reg) => regs.set8(reg, value as u8),
        Operand::Reg16(reg) => regs.set16(reg, value),
        Operand::Immediate(_) => {
            unreachable!("the decoder never produces an immediate as a write destination")
        }
        Operand::Memory {
            segment_override,
            base,
            index,
            displacement,
        } => {
            let addr =
                effective_physical_address(segment_override, base, index, displacement, regs);
            match width {
                Width::Byte => memory.write_u8(addr, value as u8),
                Width::Word => memory.write_u16(addr, value),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn effective_offset_combines_base_index_and_displacement() {
        let mut regs = Registers::new();
        regs.bx = 0x0100;
        regs.si = 0x0010;
        assert_eq!(
            effective_offset(Some(Reg16::Bx), Some(Reg16::Si), 4, &regs),
            0x0114
        );
    }

    #[test]
    fn effective_offset_with_no_base_or_index_is_just_the_displacement() {
        let regs = Registers::new();
        assert_eq!(effective_offset(None, None, 0x1234, &regs), 0x1234);
    }

    #[test]
    fn bp_based_addressing_defaults_to_stack_segment() {
        let mut regs = Registers::new();
        regs.bp = 0x0010;
        regs.ss = 0x2000;
        regs.ds = 0x1000;
        let mut memory = Memory::new();
        memory.write_u8(Memory::resolve(0x2000, 0x0010), 0xAB);
        let operand = Operand::mem(Some(Reg16::Bp), None, 0);
        assert_eq!(read_operand(&operand, Width::Byte, &regs, &memory), 0xAB);
    }

    #[test]
    fn non_bp_addressing_defaults_to_data_segment() {
        let mut regs = Registers::new();
        regs.bx = 0x0010;
        regs.ss = 0x2000;
        regs.ds = 0x1000;
        let mut memory = Memory::new();
        memory.write_u8(Memory::resolve(0x1000, 0x0010), 0xCD);
        let operand = Operand::mem(Some(Reg16::Bx), None, 0);
        assert_eq!(read_operand(&operand, Width::Byte, &regs, &memory), 0xCD);
    }

    #[test]
    fn segment_override_wins_over_the_default() {
        let mut regs = Registers::new();
        regs.bx = 0x0010;
        regs.ds = 0x1000;
        regs.es = 0x3000;
        let mut memory = Memory::new();
        memory.write_u8(Memory::resolve(0x3000, 0x0010), 0xEF);
        let operand = Operand::mem(Some(Reg16::Bx), None, 0).with_segment_override(Reg16::Es);
        assert_eq!(read_operand(&operand, Width::Byte, &regs, &memory), 0xEF);
    }

    #[test]
    fn write_then_read_round_trips_through_memory() {
        let mut regs = Registers::new();
        regs.bx = 0x0050;
        regs.ds = 0x0000;
        let mut memory = Memory::new();
        let operand = Operand::mem(Some(Reg16::Bx), None, 2);
        write_operand(&operand, Width::Word, 0xBEEF, &mut regs, &mut memory);
        assert_eq!(read_operand(&operand, Width::Word, &regs, &memory), 0xBEEF);
    }

    #[test]
    fn byte_width_only_touches_the_low_byte_of_memory() {
        let mut regs = Registers::new();
        let mut memory = Memory::new();
        let operand = Operand::mem_direct(0x0010);
        memory.write_u16(0x0010, 0xFFFF);
        write_operand(&operand, Width::Byte, 0x00AB, &mut regs, &mut memory);
        assert_eq!(memory.read_u8(0x0010), 0xAB);
        assert_eq!(memory.read_u8(0x0011), 0xFF); // untouched
    }
}
