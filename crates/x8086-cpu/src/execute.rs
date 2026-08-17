//! Instruction execution: given a decoded `Instruction` and the current
//! CPU/memory state, apply its effect.
//!
//! Callers are expected to have already advanced `regs.ip` past the
//! instruction being executed (i.e. `regs.ip` holds the address of the
//! *next* instruction when `execute` is called) - relative jumps, calls,
//! and loops all measure their displacement from that address, matching
//! how the 8086 itself computes them.

use crate::operand::{effective_offset, read_operand, write_operand};
use crate::{flags, Registers};
use x8086_isa::{Condition, Flag, Instruction, Mnemonic, Operand, Width};
use x8086_memory::Memory;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecutionEffect {
    Continue,
    Halted,
    /// A software interrupt was invoked. Simulating what the vector
    /// actually does (console I/O, etc.) is `x8086-interrupts`' job, not
    /// this crate's - `x8086-cpu` only reports that it happened.
    Interrupt(u8),
}

fn push_word(regs: &mut Registers, memory: &mut Memory, value: u16) {
    regs.sp = regs.sp.wrapping_sub(2);
    let addr = Memory::resolve(regs.ss, regs.sp);
    memory.write_u16(addr, value);
}

fn pop_word(regs: &mut Registers, memory: &Memory) -> u16 {
    let addr = Memory::resolve(regs.ss, regs.sp);
    let value = memory.read_u16(addr);
    regs.sp = regs.sp.wrapping_add(2);
    value
}

/// Relative jumps/calls/loops all store their displacement as the sole
/// operand; this applies it to the already-advanced `regs.ip`.
fn jump_relative(instr: &Instruction, regs: &mut Registers) {
    if let Some(Operand::Immediate(rel)) = instr.operands.first() {
        regs.ip = regs.ip.wrapping_add(*rel as u16);
    }
}

fn condition_holds(condition: Condition, regs: &Registers) -> bool {
    let carry = regs.get_flag(Flag::Carry);
    let zero = regs.get_flag(Flag::Zero);
    let sign = regs.get_flag(Flag::Sign);
    let overflow = regs.get_flag(Flag::Overflow);
    let parity = regs.get_flag(Flag::Parity);
    match condition {
        Condition::Overflow => overflow,
        Condition::NotOverflow => !overflow,
        Condition::Below => carry,
        Condition::AboveOrEqual => !carry,
        Condition::Equal => zero,
        Condition::NotEqual => !zero,
        Condition::BelowOrEqual => carry || zero,
        Condition::Above => !carry && !zero,
        Condition::Sign => sign,
        Condition::NotSign => !sign,
        Condition::Parity => parity,
        Condition::NotParity => !parity,
        Condition::Less => sign != overflow,
        Condition::GreaterOrEqual => sign == overflow,
        Condition::LessOrEqual => zero || (sign != overflow),
        Condition::Greater => !zero && (sign == overflow),
    }
}

/// Execute one already-decoded instruction. `regs.ip` must already point
/// past it (see module docs) before calling this.
pub fn execute(instr: &Instruction, regs: &mut Registers, memory: &mut Memory) -> ExecutionEffect {
    match instr.mnemonic {
        Mnemonic::Mov => {
            let width = instr.width.expect("MOV always carries a width");
            let value = read_operand(&instr.operands[1], width, regs, memory);
            write_operand(&instr.operands[0], width, value, regs, memory);
        }
        Mnemonic::Xchg => {
            let width = instr.width.expect("XCHG always carries a width");
            let a = read_operand(&instr.operands[0], width, regs, memory);
            let b = read_operand(&instr.operands[1], width, regs, memory);
            write_operand(&instr.operands[0], width, b, regs, memory);
            write_operand(&instr.operands[1], width, a, regs, memory);
        }
        Mnemonic::Lea => {
            if let Operand::Memory {
                base,
                index,
                displacement,
                ..
            } = instr.operands[1]
            {
                let offset = effective_offset(base, index, displacement, regs);
                write_operand(&instr.operands[0], Width::Word, offset, regs, memory);
            }
        }
        Mnemonic::Push => {
            let value = read_operand(&instr.operands[0], Width::Word, regs, memory);
            push_word(regs, memory, value);
        }
        Mnemonic::Pop => {
            let value = pop_word(regs, memory);
            write_operand(&instr.operands[0], Width::Word, value, regs, memory);
        }

        Mnemonic::Add | Mnemonic::Adc | Mnemonic::Sub | Mnemonic::Sbb | Mnemonic::Cmp => {
            let width = instr.width.expect("arithmetic ops always carry a width");
            let dst = &instr.operands[0];
            let src = &instr.operands[1];
            let a = read_operand(dst, width, regs, memory);
            let b = read_operand(src, width, regs, memory);
            let carry_in = regs.get_flag(Flag::Carry);
            let result = match instr.mnemonic {
                Mnemonic::Add => flags::add_with_flags(regs, a, b, false, width),
                Mnemonic::Adc => flags::add_with_flags(regs, a, b, carry_in, width),
                Mnemonic::Sbb => flags::sub_with_flags(regs, a, b, carry_in, width),
                Mnemonic::Sub | Mnemonic::Cmp => flags::sub_with_flags(regs, a, b, false, width),
                _ => unreachable!(),
            };
            if !matches!(instr.mnemonic, Mnemonic::Cmp) {
                write_operand(dst, width, result, regs, memory);
            }
        }
        Mnemonic::Inc | Mnemonic::Dec => {
            // INC/DEC affect OF/SF/ZF/AF/PF but leave CF untouched - a
            // well-known 8086 quirk (so a loop counter's INC doesn't
            // clobber a carry from surrounding arithmetic).
            let width = instr.width.expect("INC/DEC always carry a width");
            let op = &instr.operands[0];
            let a = read_operand(op, width, regs, memory);
            let saved_carry = regs.get_flag(Flag::Carry);
            let result = match instr.mnemonic {
                Mnemonic::Inc => flags::add_with_flags(regs, a, 1, false, width),
                Mnemonic::Dec => flags::sub_with_flags(regs, a, 1, false, width),
                _ => unreachable!(),
            };
            regs.set_flag(Flag::Carry, saved_carry);
            write_operand(op, width, result, regs, memory);
        }

        Mnemonic::And | Mnemonic::Or | Mnemonic::Xor | Mnemonic::Test => {
            let width = instr.width.expect("logic ops always carry a width");
            let dst = &instr.operands[0];
            let src = &instr.operands[1];
            let a = read_operand(dst, width, regs, memory);
            let b = read_operand(src, width, regs, memory);
            let result = match instr.mnemonic {
                Mnemonic::And | Mnemonic::Test => a & b,
                Mnemonic::Or => a | b,
                Mnemonic::Xor => a ^ b,
                _ => unreachable!(),
            };
            flags::set_flags_after_logic(regs, result, width);
            if !matches!(instr.mnemonic, Mnemonic::Test) {
                write_operand(dst, width, result, regs, memory);
            }
        }

        Mnemonic::Jmp => jump_relative(instr, regs),
        Mnemonic::Jcc(condition) => {
            if condition_holds(condition, regs) {
                jump_relative(instr, regs);
            }
        }
        Mnemonic::Loop => {
            regs.cx = regs.cx.wrapping_sub(1);
            if regs.cx != 0 {
                jump_relative(instr, regs);
            }
        }
        Mnemonic::Loope => {
            regs.cx = regs.cx.wrapping_sub(1);
            if regs.cx != 0 && regs.get_flag(Flag::Zero) {
                jump_relative(instr, regs);
            }
        }
        Mnemonic::Loopne => {
            regs.cx = regs.cx.wrapping_sub(1);
            if regs.cx != 0 && !regs.get_flag(Flag::Zero) {
                jump_relative(instr, regs);
            }
        }
        Mnemonic::Jcxz => {
            if regs.cx == 0 {
                jump_relative(instr, regs);
            }
        }
        Mnemonic::Call => {
            let return_addr = regs.ip;
            push_word(regs, memory, return_addr);
            jump_relative(instr, regs);
        }
        Mnemonic::Ret => {
            regs.ip = pop_word(regs, memory);
            if let Some(Operand::Immediate(pop_count)) = instr.operands.first() {
                regs.sp = regs.sp.wrapping_add(*pop_count as u16);
            }
        }

        Mnemonic::Int => {
            let vector = match instr.operands.first() {
                Some(Operand::Immediate(n)) => *n as u8,
                _ => 0,
            };
            return ExecutionEffect::Interrupt(vector);
        }
        Mnemonic::Int3 => return ExecutionEffect::Interrupt(3),
        Mnemonic::Iret => {
            regs.ip = pop_word(regs, memory);
            regs.cs = pop_word(regs, memory);
            regs.flags = pop_word(regs, memory);
        }

        Mnemonic::Hlt => return ExecutionEffect::Halted,
        Mnemonic::Nop => {}
        Mnemonic::Clc => regs.set_flag(Flag::Carry, false),
        Mnemonic::Stc => regs.set_flag(Flag::Carry, true),
        Mnemonic::Cmc => {
            let carry = regs.get_flag(Flag::Carry);
            regs.set_flag(Flag::Carry, !carry);
        }
        Mnemonic::Cld => regs.set_flag(Flag::Direction, false),
        Mnemonic::Std => regs.set_flag(Flag::Direction, true),
        Mnemonic::Cli => regs.set_flag(Flag::Interrupt, false),
        Mnemonic::Sti => regs.set_flag(Flag::Interrupt, true),

        Mnemonic::Unknown => {}
    }
    ExecutionEffect::Continue
}

#[cfg(test)]
mod tests {
    use super::*;
    use x8086_isa::{Mnemonic, Reg16, Reg8};

    fn instr(mnemonic: Mnemonic, operands: Vec<Operand>, width: Option<Width>) -> Instruction {
        Instruction::new(mnemonic, operands, width, 0)
    }

    #[test]
    fn mov_copies_immediate_into_register() {
        let mut regs = Registers::new();
        let mut memory = Memory::new();
        let i = instr(
            Mnemonic::Mov,
            vec![Operand::Reg16(Reg16::Ax), Operand::Immediate(0x1234)],
            Some(Width::Word),
        );
        execute(&i, &mut regs, &mut memory);
        assert_eq!(regs.ax, 0x1234);
    }

    #[test]
    fn add_writes_result_and_sets_flags() {
        let mut regs = Registers::new();
        regs.ax = 5;
        let mut memory = Memory::new();
        let i = instr(
            Mnemonic::Add,
            vec![Operand::Reg16(Reg16::Ax), Operand::Immediate(3)],
            Some(Width::Word),
        );
        execute(&i, &mut regs, &mut memory);
        assert_eq!(regs.ax, 8);
        assert!(!regs.get_flag(Flag::Zero));
    }

    #[test]
    fn cmp_does_not_modify_the_destination() {
        let mut regs = Registers::new();
        regs.ax = 5;
        let mut memory = Memory::new();
        let i = instr(
            Mnemonic::Cmp,
            vec![Operand::Reg16(Reg16::Ax), Operand::Immediate(5)],
            Some(Width::Word),
        );
        execute(&i, &mut regs, &mut memory);
        assert_eq!(regs.ax, 5); // unchanged
        assert!(regs.get_flag(Flag::Zero)); // but the comparison still set flags
    }

    #[test]
    fn inc_does_not_disturb_carry_flag() {
        let mut regs = Registers::new();
        regs.set_flag(Flag::Carry, true);
        regs.ax = 5;
        let mut memory = Memory::new();
        let i = instr(
            Mnemonic::Inc,
            vec![Operand::Reg16(Reg16::Ax)],
            Some(Width::Word),
        );
        execute(&i, &mut regs, &mut memory);
        assert_eq!(regs.ax, 6);
        assert!(
            regs.get_flag(Flag::Carry),
            "INC must not clear a pre-existing carry flag"
        );
    }

    #[test]
    fn push_then_pop_round_trips_through_the_stack() {
        let mut regs = Registers::new();
        regs.ss = 0x1000;
        regs.sp = 0x0100;
        regs.bx = 0xBEEF;
        let mut memory = Memory::new();

        let push = instr(
            Mnemonic::Push,
            vec![Operand::Reg16(Reg16::Bx)],
            Some(Width::Word),
        );
        execute(&push, &mut regs, &mut memory);
        assert_eq!(regs.sp, 0x00FE);

        regs.bx = 0; // clobber so the POP has to actually restore it
        let pop = instr(
            Mnemonic::Pop,
            vec![Operand::Reg16(Reg16::Bx)],
            Some(Width::Word),
        );
        execute(&pop, &mut regs, &mut memory);
        assert_eq!(regs.bx, 0xBEEF);
        assert_eq!(regs.sp, 0x0100);
    }

    #[test]
    fn jmp_adds_relative_displacement_to_already_advanced_ip() {
        let mut regs = Registers::new();
        regs.ip = 0x0100; // simulates the emulator having already advanced past the JMP
        let mut memory = Memory::new();
        let i = instr(Mnemonic::Jmp, vec![Operand::Immediate(-2)], None);
        execute(&i, &mut regs, &mut memory);
        assert_eq!(regs.ip, 0x00FE);
    }

    #[test]
    fn jcc_only_jumps_when_condition_holds() {
        let mut regs = Registers::new();
        regs.ip = 0x0100;
        regs.set_flag(Flag::Zero, false);
        let mut memory = Memory::new();
        let i = instr(
            Mnemonic::Jcc(Condition::Equal),
            vec![Operand::Immediate(10)],
            None,
        );
        execute(&i, &mut regs, &mut memory);
        assert_eq!(regs.ip, 0x0100, "JE must not jump when ZF is clear");

        regs.set_flag(Flag::Zero, true);
        execute(&i, &mut regs, &mut memory);
        assert_eq!(regs.ip, 0x010A);
    }

    #[test]
    fn loop_decrements_cx_and_jumps_until_zero() {
        let mut regs = Registers::new();
        regs.cx = 2;
        regs.ip = 0x0100;
        let mut memory = Memory::new();
        let i = instr(Mnemonic::Loop, vec![Operand::Immediate(-5)], None);

        execute(&i, &mut regs, &mut memory); // cx: 2 -> 1, jumps
        assert_eq!(regs.cx, 1);
        assert_eq!(regs.ip, 0x00FB);

        regs.ip = 0x0100;
        execute(&i, &mut regs, &mut memory); // cx: 1 -> 0, does not jump
        assert_eq!(regs.cx, 0);
        assert_eq!(regs.ip, 0x0100);
    }

    #[test]
    fn call_pushes_return_address_and_jumps() {
        let mut regs = Registers::new();
        regs.ss = 0;
        regs.sp = 0x0100;
        regs.ip = 0x0050; // "address of the instruction after CALL"
        let mut memory = Memory::new();
        let i = instr(Mnemonic::Call, vec![Operand::Immediate(0x0010)], None);
        execute(&i, &mut regs, &mut memory);
        assert_eq!(regs.ip, 0x0060);
        assert_eq!(memory.read_u16(Memory::resolve(0, 0x00FE)), 0x0050);
    }

    #[test]
    fn call_then_ret_returns_to_the_caller() {
        let mut regs = Registers::new();
        regs.ss = 0;
        regs.sp = 0x0100;
        regs.ip = 0x0050;
        let mut memory = Memory::new();
        execute(
            &instr(Mnemonic::Call, vec![Operand::Immediate(0x0010)], None),
            &mut regs,
            &mut memory,
        );
        execute(&instr(Mnemonic::Ret, vec![], None), &mut regs, &mut memory);
        assert_eq!(regs.ip, 0x0050);
        assert_eq!(regs.sp, 0x0100);
    }

    #[test]
    fn ret_with_immediate_also_pops_extra_stack_space() {
        let mut regs = Registers::new();
        regs.ss = 0;
        regs.sp = 0x0100;
        regs.ip = 0x0050;
        let mut memory = Memory::new();
        execute(
            &instr(Mnemonic::Call, vec![Operand::Immediate(0x0010)], None),
            &mut regs,
            &mut memory,
        );
        execute(
            &instr(Mnemonic::Ret, vec![Operand::Immediate(4)], None),
            &mut regs,
            &mut memory,
        );
        assert_eq!(regs.sp, 0x0104);
    }

    #[test]
    fn int_reports_its_vector_without_touching_the_stack() {
        let mut regs = Registers::new();
        regs.sp = 0x0100;
        let mut memory = Memory::new();
        let i = instr(Mnemonic::Int, vec![Operand::Immediate(0x21)], None);
        let effect = execute(&i, &mut regs, &mut memory);
        assert_eq!(effect, ExecutionEffect::Interrupt(0x21));
        assert_eq!(
            regs.sp, 0x0100,
            "simulated interrupts don't push a real IVT frame"
        );
    }

    #[test]
    fn hlt_reports_halted() {
        let mut regs = Registers::new();
        let mut memory = Memory::new();
        let effect = execute(&instr(Mnemonic::Hlt, vec![], None), &mut regs, &mut memory);
        assert_eq!(effect, ExecutionEffect::Halted);
    }

    #[test]
    fn flag_control_instructions_set_the_right_bit() {
        let mut regs = Registers::new();
        let mut memory = Memory::new();
        execute(&instr(Mnemonic::Stc, vec![], None), &mut regs, &mut memory);
        assert!(regs.get_flag(Flag::Carry));
        execute(&instr(Mnemonic::Clc, vec![], None), &mut regs, &mut memory);
        assert!(!regs.get_flag(Flag::Carry));
        execute(&instr(Mnemonic::Cmc, vec![], None), &mut regs, &mut memory);
        assert!(regs.get_flag(Flag::Carry));
        execute(&instr(Mnemonic::Std, vec![], None), &mut regs, &mut memory);
        assert!(regs.get_flag(Flag::Direction));
        execute(&instr(Mnemonic::Sti, vec![], None), &mut regs, &mut memory);
        assert!(regs.get_flag(Flag::Interrupt));
    }

    #[test]
    fn lea_loads_the_offset_not_the_memory_contents() {
        let mut regs = Registers::new();
        regs.bx = 0x0010;
        regs.ds = 0x2000; // if LEA dereferenced memory, this segment would matter; it must not
        let mut memory = Memory::new();
        memory.write_u16(Memory::resolve(0x2000, 0x0014), 0xFFFF); // decoy value at the target address
        let i = instr(
            Mnemonic::Lea,
            vec![
                Operand::Reg16(Reg16::Ax),
                Operand::mem(Some(Reg16::Bx), None, 4),
            ],
            None,
        );
        execute(&i, &mut regs, &mut memory);
        assert_eq!(
            regs.ax, 0x0014,
            "LEA must load the computed offset, not read memory at that address"
        );
    }

    #[test]
    fn xchg_swaps_both_operands() {
        let mut regs = Registers::new();
        regs.ax = 1;
        regs.bx = 2;
        let mut memory = Memory::new();
        let i = instr(
            Mnemonic::Xchg,
            vec![Operand::Reg16(Reg16::Ax), Operand::Reg16(Reg16::Bx)],
            Some(Width::Word),
        );
        execute(&i, &mut regs, &mut memory);
        assert_eq!(regs.ax, 2);
        assert_eq!(regs.bx, 1);
    }

    #[test]
    fn byte_width_mov_only_touches_the_low_half_of_the_register() {
        let mut regs = Registers::new();
        regs.ax = 0xAAFF;
        let mut memory = Memory::new();
        let i = instr(
            Mnemonic::Mov,
            vec![Operand::Reg8(Reg8::Ah), Operand::Immediate(0x11)],
            Some(Width::Byte),
        );
        execute(&i, &mut regs, &mut memory);
        assert_eq!(regs.ax, 0x11FF);
    }
}
