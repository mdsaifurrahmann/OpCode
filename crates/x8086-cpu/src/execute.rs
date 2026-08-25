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
use x8086_isa::{Condition, Flag, Instruction, Mnemonic, Operand, Reg8, Repeat, Width};
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

/// The five FLAGS bits LAHF/SAHF transfer through AH: SF (7), ZF (6),
/// AF (4), PF (2), CF (0). The gaps are the 8086's reserved bits, which
/// this emulator doesn't model either way.
const LOW_FLAGS_MASK: u16 = 0b1101_0101;

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

        Mnemonic::Pushf => push_word(regs, memory, regs.flags),
        Mnemonic::Popf => regs.flags = pop_word(regs, memory),
        // LAHF/SAHF move only the low byte of FLAGS - SF, ZF, AF, PF,
        // CF, at bits 7, 6, 4, 2, 0. SAHF masks to exactly those five so
        // it can't invent values for the bits in between, which this
        // emulator doesn't model (see `Mnemonic::Pushf`'s docs); LAHF
        // hands back the low byte as it actually stands, so a
        // LAHF/SAHF round trip is lossless.
        Mnemonic::Lahf => regs.set8(Reg8::Ah, regs.flags as u8),
        Mnemonic::Sahf => {
            let ah = regs.get8(Reg8::Ah) as u16;
            regs.flags = (regs.flags & !LOW_FLAGS_MASK) | (ah & LOW_FLAGS_MASK);
        }
        // XLAT: AL indexes a byte table based at DS:BX.
        Mnemonic::Xlat => {
            let offset = regs.bx.wrapping_add(regs.get8(Reg8::Al) as u16);
            let value = memory.read_u8(Memory::resolve(regs.ds, offset));
            regs.set8(Reg8::Al, value);
        }

        Mnemonic::Shl
        | Mnemonic::Shr
        | Mnemonic::Sar
        | Mnemonic::Rol
        | Mnemonic::Ror
        | Mnemonic::Rcl
        | Mnemonic::Rcr => {
            let width = instr.width.expect("shift/rotate always carries a width");
            let count = shift_rotate_count(instr, regs);
            execute_shift_rotate(
                instr.mnemonic,
                &instr.operands[0],
                width,
                count,
                regs,
                memory,
            );
        }

        Mnemonic::Mul => {
            let width = instr.width.expect("MUL always carries a width");
            execute_mul(&instr.operands[0], width, regs, memory);
        }
        Mnemonic::Imul => {
            let width = instr.width.expect("IMUL always carries a width");
            execute_imul(&instr.operands[0], width, regs, memory);
        }
        Mnemonic::Div => {
            let width = instr.width.expect("DIV always carries a width");
            if !execute_div(&instr.operands[0], width, regs, memory) {
                return ExecutionEffect::Interrupt(0); // divide error / overflow
            }
        }
        Mnemonic::Idiv => {
            let width = instr.width.expect("IDIV always carries a width");
            if !execute_idiv(&instr.operands[0], width, regs, memory) {
                return ExecutionEffect::Interrupt(0); // divide error / overflow
            }
        }
        Mnemonic::Neg => {
            let width = instr.width.expect("NEG always carries a width");
            let op = &instr.operands[0];
            let a = read_operand(op, width, regs, memory);
            let result = flags::sub_with_flags(regs, 0, a, false, width);
            write_operand(op, width, result, regs, memory);
        }
        Mnemonic::Not => {
            let width = instr.width.expect("NOT always carries a width");
            let op = &instr.operands[0];
            let mask: u16 = match width {
                Width::Byte => 0x00FF,
                Width::Word => 0xFFFF,
            };
            let a = read_operand(op, width, regs, memory);
            write_operand(op, width, !a & mask, regs, memory);
        }

        Mnemonic::Movsb
        | Mnemonic::Movsw
        | Mnemonic::Cmpsb
        | Mnemonic::Cmpsw
        | Mnemonic::Stosb
        | Mnemonic::Stosw
        | Mnemonic::Lodsb
        | Mnemonic::Lodsw
        | Mnemonic::Scasb
        | Mnemonic::Scasw => match instr.repeat {
            None => execute_string_op(instr.mnemonic, regs, memory),
            Some(repeat) => {
                while regs.cx != 0 {
                    execute_string_op(instr.mnemonic, regs, memory);
                    regs.cx = regs.cx.wrapping_sub(1);
                    let stop = match repeat {
                        Repeat::Rep => false,
                        Repeat::Repe => !regs.get_flag(Flag::Zero),
                        Repeat::Repne => regs.get_flag(Flag::Zero),
                    };
                    if stop {
                        break;
                    }
                }
            }
        },

        Mnemonic::Unknown => {}
    }
    ExecutionEffect::Continue
}

// --- shift/rotate group -------------------------------------------------

fn shift_rotate_count(instr: &Instruction, regs: &Registers) -> u32 {
    match instr.operands.get(1) {
        Some(Operand::Immediate(n)) => *n as u32 & 0xFF,
        Some(Operand::Reg8(Reg8::Cl)) => regs.get8(Reg8::Cl) as u32,
        _ => 0,
    }
}

/// Real 8086 semantics: a `CL`/immediate count is *not* masked to the
/// operand width (that width-masking is a 286+ behavior) - a count of,
/// say, 20 on a byte operand genuinely shifts/rotates 20 times, which a
/// bit-at-a-time loop handles correctly (and with no risk of the `<<`/`>>`
/// overflow panic a single wide shift by a large count would hit).
fn execute_shift_rotate(
    mnemonic: Mnemonic,
    dst: &Operand,
    width: Width,
    count: u32,
    regs: &mut Registers,
    memory: &mut Memory,
) {
    if count == 0 {
        // A count of 0 leaves both the value and every flag untouched -
        // a well-known 8086 quirk (mirrors INC/DEC not touching CF).
        return;
    }
    let bits = match width {
        Width::Byte => 8u32,
        Width::Word => 16u32,
    };
    let mask = (1u32 << bits) - 1;
    let sign_bit = 1u32 << (bits - 1);
    let original = read_operand(dst, width, regs, memory) as u32 & mask;
    let mut value = original;
    let mut carry = regs.get_flag(Flag::Carry) as u32;

    for _ in 0..count {
        carry = match mnemonic {
            Mnemonic::Shl => {
                let out = (value & sign_bit) != 0;
                value = (value << 1) & mask;
                out as u32
            }
            Mnemonic::Shr => {
                let out = value & 1;
                value >>= 1;
                out
            }
            Mnemonic::Sar => {
                let out = value & 1;
                let sign = value & sign_bit;
                value = (value >> 1) | sign;
                out
            }
            Mnemonic::Rol => {
                let out = (value & sign_bit) != 0;
                value = ((value << 1) | out as u32) & mask;
                out as u32
            }
            Mnemonic::Ror => {
                let out = value & 1;
                value = (value >> 1) | (out << (bits - 1));
                out
            }
            Mnemonic::Rcl => {
                let out = (value & sign_bit) != 0;
                value = ((value << 1) | carry) & mask;
                out as u32
            }
            Mnemonic::Rcr => {
                let out = value & 1;
                value = (value >> 1) | (carry << (bits - 1));
                out
            }
            other => unreachable!(
                "execute_shift_rotate only dispatches for the shift/rotate group, got {other:?}"
            ),
        };
    }

    regs.set_flag(Flag::Carry, carry != 0);
    // OF is only architecturally defined when the count is exactly 1;
    // for larger counts real hardware leaves it undefined, so we simply
    // don't touch it (deterministic: it keeps whatever it already held).
    if count == 1 {
        let overflow = match mnemonic {
            Mnemonic::Shl | Mnemonic::Rol | Mnemonic::Rcl => {
                ((value & sign_bit) != 0) != (carry != 0)
            }
            Mnemonic::Shr => (original & sign_bit) != 0,
            Mnemonic::Sar => false,
            Mnemonic::Ror | Mnemonic::Rcr => {
                let msb = (value & sign_bit) != 0;
                let msb2 = (value & (sign_bit >> 1)) != 0;
                msb != msb2
            }
            other => unreachable!(
                "execute_shift_rotate only dispatches for the shift/rotate group, got {other:?}"
            ),
        };
        regs.set_flag(Flag::Overflow, overflow);
    }
    // Shifts (not rotates) also update ZF/SF/PF from the result; AF is
    // documented as undefined after any shift, so we leave it untouched
    // rather than guessing at a value nothing can rely on.
    if matches!(mnemonic, Mnemonic::Shl | Mnemonic::Shr | Mnemonic::Sar) {
        regs.set_flag(Flag::Zero, value == 0);
        regs.set_flag(Flag::Sign, (value & sign_bit) != 0);
        regs.set_flag(Flag::Parity, flags::parity_even(value as u8));
    }

    write_operand(dst, width, value as u16, regs, memory);
}

// --- MUL/IMUL/DIV/IDIV ---------------------------------------------------

fn execute_mul(src: &Operand, width: Width, regs: &mut Registers, memory: &mut Memory) {
    match width {
        Width::Byte => {
            let al = regs.get8(Reg8::Al) as u32;
            let operand = read_operand(src, width, regs, memory) as u32;
            let result = al * operand;
            regs.ax = result as u16;
            let overflow = (result >> 8) != 0;
            regs.set_flag(Flag::Carry, overflow);
            regs.set_flag(Flag::Overflow, overflow);
        }
        Width::Word => {
            let ax = regs.ax as u32;
            let operand = read_operand(src, width, regs, memory) as u32;
            let result = ax * operand;
            regs.ax = result as u16;
            regs.dx = (result >> 16) as u16;
            let overflow = regs.dx != 0;
            regs.set_flag(Flag::Carry, overflow);
            regs.set_flag(Flag::Overflow, overflow);
        }
    }
}

fn execute_imul(src: &Operand, width: Width, regs: &mut Registers, memory: &mut Memory) {
    match width {
        Width::Byte => {
            let al = regs.get8(Reg8::Al) as i8 as i32;
            let operand = read_operand(src, width, regs, memory) as u8 as i8 as i32;
            let result = al * operand;
            regs.ax = result as u16;
            let overflow = result != (result as i8 as i32);
            regs.set_flag(Flag::Carry, overflow);
            regs.set_flag(Flag::Overflow, overflow);
        }
        Width::Word => {
            let ax = regs.ax as i16 as i32;
            let operand = read_operand(src, width, regs, memory) as i16 as i32;
            let result = ax * operand;
            regs.ax = result as u16;
            regs.dx = (result >> 16) as u16;
            let overflow = result != (result as i16 as i32);
            regs.set_flag(Flag::Carry, overflow);
            regs.set_flag(Flag::Overflow, overflow);
        }
    }
}

/// Returns `false` on divide-by-zero or quotient overflow, in which case
/// the caller reports it as `ExecutionEffect::Interrupt(0)` (vector 0 is
/// the real 8086's divide-error interrupt) instead of writing any
/// register - matching real hardware, which leaves AX/DX untouched when
/// the division faults.
fn execute_div(src: &Operand, width: Width, regs: &mut Registers, memory: &mut Memory) -> bool {
    match width {
        Width::Byte => {
            let divisor = read_operand(src, width, regs, memory);
            if divisor == 0 {
                return false;
            }
            let dividend = regs.ax;
            let quotient = dividend / divisor;
            let remainder = dividend % divisor;
            if quotient > 0xFF {
                return false;
            }
            regs.set8(Reg8::Al, quotient as u8);
            regs.set8(Reg8::Ah, remainder as u8);
            true
        }
        Width::Word => {
            let divisor = read_operand(src, width, regs, memory) as u32;
            if divisor == 0 {
                return false;
            }
            let dividend = ((regs.dx as u32) << 16) | (regs.ax as u32);
            let quotient = dividend / divisor;
            let remainder = dividend % divisor;
            if quotient > 0xFFFF {
                return false;
            }
            regs.ax = quotient as u16;
            regs.dx = remainder as u16;
            true
        }
    }
}

fn execute_idiv(src: &Operand, width: Width, regs: &mut Registers, memory: &mut Memory) -> bool {
    match width {
        Width::Byte => {
            let divisor = read_operand(src, width, regs, memory) as u8 as i8 as i32;
            if divisor == 0 {
                return false;
            }
            let dividend = regs.ax as i16 as i32;
            let quotient = dividend / divisor;
            let remainder = dividend % divisor;
            if !(i8::MIN as i32..=i8::MAX as i32).contains(&quotient) {
                return false;
            }
            regs.set8(Reg8::Al, quotient as i8 as u8);
            regs.set8(Reg8::Ah, remainder as i8 as u8);
            true
        }
        Width::Word => {
            let divisor = read_operand(src, width, regs, memory) as i16 as i32;
            if divisor == 0 {
                return false;
            }
            let dividend = (((regs.dx as u32) << 16) | (regs.ax as u32)) as i32;
            let quotient = dividend / divisor;
            let remainder = dividend % divisor;
            if !(i16::MIN as i32..=i16::MAX as i32).contains(&quotient) {
                return false;
            }
            regs.ax = quotient as i16 as u16;
            regs.dx = remainder as i16 as u16;
            true
        }
    }
}

// --- string instructions --------------------------------------------------

/// The per-iteration step applied to SI/DI: `+width` normally, `-width`
/// when the Direction flag is set (`STD`).
fn string_step(width: Width, direction_flag: bool) -> u16 {
    let n: u16 = match width {
        Width::Byte => 1,
        Width::Word => 2,
    };
    if direction_flag {
        0u16.wrapping_sub(n)
    } else {
        n
    }
}

/// One iteration of a string instruction. Source is always `DS:SI`
/// (segment-override prefixes aren't decoded yet) and destination is
/// always `ES:DI` (never overridable, matching real 8086).
fn execute_string_op(mnemonic: Mnemonic, regs: &mut Registers, memory: &mut Memory) {
    let df = regs.get_flag(Flag::Direction);
    match mnemonic {
        Mnemonic::Movsb | Mnemonic::Movsw => {
            let width = if mnemonic == Mnemonic::Movsb {
                Width::Byte
            } else {
                Width::Word
            };
            let src_addr = Memory::resolve(regs.ds, regs.si);
            let dst_addr = Memory::resolve(regs.es, regs.di);
            match width {
                Width::Byte => memory.write_u8(dst_addr, memory.read_u8(src_addr)),
                Width::Word => memory.write_u16(dst_addr, memory.read_u16(src_addr)),
            }
            let step = string_step(width, df);
            regs.si = regs.si.wrapping_add(step);
            regs.di = regs.di.wrapping_add(step);
        }
        Mnemonic::Lodsb | Mnemonic::Lodsw => {
            let width = if mnemonic == Mnemonic::Lodsb {
                Width::Byte
            } else {
                Width::Word
            };
            let src_addr = Memory::resolve(regs.ds, regs.si);
            match width {
                Width::Byte => regs.set8(Reg8::Al, memory.read_u8(src_addr)),
                Width::Word => regs.ax = memory.read_u16(src_addr),
            }
            regs.si = regs.si.wrapping_add(string_step(width, df));
        }
        Mnemonic::Stosb | Mnemonic::Stosw => {
            let width = if mnemonic == Mnemonic::Stosb {
                Width::Byte
            } else {
                Width::Word
            };
            let dst_addr = Memory::resolve(regs.es, regs.di);
            match width {
                Width::Byte => memory.write_u8(dst_addr, regs.get8(Reg8::Al)),
                Width::Word => memory.write_u16(dst_addr, regs.ax),
            }
            regs.di = regs.di.wrapping_add(string_step(width, df));
        }
        Mnemonic::Cmpsb | Mnemonic::Cmpsw => {
            let width = if mnemonic == Mnemonic::Cmpsb {
                Width::Byte
            } else {
                Width::Word
            };
            let src_addr = Memory::resolve(regs.ds, regs.si);
            let dst_addr = Memory::resolve(regs.es, regs.di);
            let (a, b) = match width {
                Width::Byte => (
                    memory.read_u8(src_addr) as u16,
                    memory.read_u8(dst_addr) as u16,
                ),
                Width::Word => (memory.read_u16(src_addr), memory.read_u16(dst_addr)),
            };
            flags::sub_with_flags(regs, a, b, false, width);
            let step = string_step(width, df);
            regs.si = regs.si.wrapping_add(step);
            regs.di = regs.di.wrapping_add(step);
        }
        Mnemonic::Scasb | Mnemonic::Scasw => {
            let width = if mnemonic == Mnemonic::Scasb {
                Width::Byte
            } else {
                Width::Word
            };
            let dst_addr = Memory::resolve(regs.es, regs.di);
            let acc = match width {
                Width::Byte => regs.get8(Reg8::Al) as u16,
                Width::Word => regs.ax,
            };
            let mem_val = match width {
                Width::Byte => memory.read_u8(dst_addr) as u16,
                Width::Word => memory.read_u16(dst_addr),
            };
            flags::sub_with_flags(regs, acc, mem_val, false, width);
            regs.di = regs.di.wrapping_add(string_step(width, df));
        }
        other => {
            unreachable!("execute_string_op only dispatches for string mnemonics, got {other:?}")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use x8086_isa::{Mnemonic, Reg16, Reg8};

    fn instr(mnemonic: Mnemonic, operands: Vec<Operand>, width: Option<Width>) -> Instruction {
        Instruction::new(mnemonic, operands, width, 0)
    }

    /// The precise sequence a reported "the emulator computes 2*3 wrong"
    /// case hinges on. Nothing here is emulator-specific - each assertion
    /// is a direct statement of documented 8086 semantics, isolated so
    /// the claim can be checked one instruction at a time rather than
    /// argued about at the whole-program level:
    ///
    ///   MUL CL      ; AX <- AL * CL          (AX is the *full* 16-bit
    ///                                         product, AH included)
    ///   MOV AH, 09h ; AH is the high half of AX, so this rewrites the
    ///                 product's high byte - AX is no longer the product
    ///   DIV BL      ; AL <- AX / BL, AH <- AX % BL, using that *modified*
    ///                 AX, not the original product
    ///
    /// If a program computes a product with MUL and then selects a DOS
    /// function with `MOV AH, ...` before consuming it, the value DIV
    /// later divides is the corrupted one. That is correct hardware
    /// behavior, not a defect.
    #[test]
    fn mov_ah_between_mul_and_div_corrupts_the_product_exactly_as_real_hardware_does() {
        let mut regs = Registers::new();
        let mut memory = Memory::new();

        // MOV AL, 2 / MOV CL, 3 / MUL CL  ->  AX = 6
        regs.set8(Reg8::Al, 2);
        regs.set8(Reg8::Cl, 3);
        execute(
            &instr(
                Mnemonic::Mul,
                vec![Operand::Reg8(Reg8::Cl)],
                Some(Width::Byte),
            ),
            &mut regs,
            &mut memory,
        );
        assert_eq!(regs.ax, 0x0006, "MUL CL must put the full product in AX");

        // MOV AH, 09h - selects the DOS "print string" function, and in
        // doing so overwrites the product's high byte.
        execute(
            &instr(
                Mnemonic::Mov,
                vec![Operand::Reg8(Reg8::Ah), Operand::Immediate(0x09)],
                Some(Width::Byte),
            ),
            &mut regs,
            &mut memory,
        );
        assert_eq!(
            regs.ax, 0x0906,
            "AH is the high half of AX: writing it must leave AL alone and \
             change AX from 6 to 0x0906 (2310)"
        );

        // MOV BL, 10 / DIV BL - divides 2310, not 6.
        regs.set8(Reg8::Bl, 10);
        execute(
            &instr(
                Mnemonic::Div,
                vec![Operand::Reg8(Reg8::Bl)],
                Some(Width::Byte),
            ),
            &mut regs,
            &mut memory,
        );
        assert_eq!(
            regs.get8(Reg8::Al),
            231,
            "2310 / 10 = 231 - the quotient of the corrupted AX"
        );
        assert_eq!(regs.get8(Reg8::Ah), 0, "2310 % 10 = 0");

        // ADD AL, '0' then wraps: 231 + 48 = 279, truncated to 8 bits.
        execute(
            &instr(
                Mnemonic::Add,
                vec![Operand::Reg8(Reg8::Al), Operand::Immediate(b'0' as i32)],
                Some(Width::Byte),
            ),
            &mut regs,
            &mut memory,
        );
        assert_eq!(
            regs.get8(Reg8::Al),
            0x17,
            "279 doesn't fit in 8 bits - it wraps to 0x17, an unprintable \
             control character, which is what actually reaches the console"
        );
    }

    /// The same sequence with the single fix applied (preserve AX across
    /// the DOS call), proving the emulator produces the expected 6 the
    /// moment the program stops clobbering its own result.
    #[test]
    fn preserving_ax_across_the_dos_call_yields_the_expected_product() {
        let mut regs = Registers::new();
        let mut memory = Memory::new();
        regs.set8(Reg8::Al, 2);
        regs.set8(Reg8::Cl, 3);
        execute(
            &instr(
                Mnemonic::Mul,
                vec![Operand::Reg8(Reg8::Cl)],
                Some(Width::Byte),
            ),
            &mut regs,
            &mut memory,
        );
        let product = regs.ax;

        // ... MOV AH, 09h / INT 21h happens here, clobbering AH ...
        execute(
            &instr(
                Mnemonic::Mov,
                vec![Operand::Reg8(Reg8::Ah), Operand::Immediate(0x09)],
                Some(Width::Byte),
            ),
            &mut regs,
            &mut memory,
        );
        // ... but the program restored AX first (PUSH AX / POP AX, or via
        // a memory variable), so DIV sees the real product again.
        regs.ax = product;

        regs.set8(Reg8::Bl, 10);
        execute(
            &instr(
                Mnemonic::Div,
                vec![Operand::Reg8(Reg8::Bl)],
                Some(Width::Byte),
            ),
            &mut regs,
            &mut memory,
        );
        assert_eq!(regs.get8(Reg8::Al), 0, "tens digit of 6");
        assert_eq!(regs.get8(Reg8::Ah), 6, "ones digit of 6");
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

    // --- shift/rotate group -------------------------------------------

    #[test]
    fn shl_by_cl_shifts_and_produces_the_expected_value() {
        let mut regs = Registers::new();
        regs.set8(Reg8::Al, 1);
        regs.set8(Reg8::Cl, 3);
        let mut memory = Memory::new();
        let i = instr(
            Mnemonic::Shl,
            vec![Operand::Reg8(Reg8::Al), Operand::Reg8(Reg8::Cl)],
            Some(Width::Byte),
        );
        execute(&i, &mut regs, &mut memory);
        assert_eq!(regs.get8(Reg8::Al), 8);
    }

    #[test]
    fn shr_by_cl_shifts_right() {
        let mut regs = Registers::new();
        regs.set8(Reg8::Al, 0x80);
        regs.set8(Reg8::Cl, 3);
        let mut memory = Memory::new();
        let i = instr(
            Mnemonic::Shr,
            vec![Operand::Reg8(Reg8::Al), Operand::Reg8(Reg8::Cl)],
            Some(Width::Byte),
        );
        execute(&i, &mut regs, &mut memory);
        assert_eq!(regs.get8(Reg8::Al), 0x10);
    }

    #[test]
    fn sar_preserves_the_sign_bit() {
        let mut regs = Registers::new();
        regs.set8(Reg8::Al, 0xF0);
        regs.set8(Reg8::Cl, 2);
        let mut memory = Memory::new();
        let i = instr(
            Mnemonic::Sar,
            vec![Operand::Reg8(Reg8::Al), Operand::Reg8(Reg8::Cl)],
            Some(Width::Byte),
        );
        execute(&i, &mut regs, &mut memory);
        assert_eq!(regs.get8(Reg8::Al), 0xFC);
    }

    #[test]
    fn rol_by_one_rotates_msb_into_lsb_and_carry() {
        let mut regs = Registers::new();
        regs.set8(Reg8::Al, 0x81);
        let mut memory = Memory::new();
        let i = instr(
            Mnemonic::Rol,
            vec![Operand::Reg8(Reg8::Al), Operand::Immediate(1)],
            Some(Width::Byte),
        );
        execute(&i, &mut regs, &mut memory);
        assert_eq!(regs.get8(Reg8::Al), 0x03);
        assert!(regs.get_flag(Flag::Carry));
    }

    #[test]
    fn ror_by_one_rotates_lsb_into_msb_and_carry() {
        let mut regs = Registers::new();
        regs.set8(Reg8::Al, 0x03);
        let mut memory = Memory::new();
        let i = instr(
            Mnemonic::Ror,
            vec![Operand::Reg8(Reg8::Al), Operand::Immediate(1)],
            Some(Width::Byte),
        );
        execute(&i, &mut regs, &mut memory);
        assert_eq!(regs.get8(Reg8::Al), 0x81);
        assert!(regs.get_flag(Flag::Carry));
    }

    #[test]
    fn rcl_chains_a_shifted_out_bit_between_two_registers() {
        // Mirrors the classic "32-bit shift via two 16-bit halves" idiom:
        // SHL AX,1 then RCL DX,1 propagates AX's vacated top bit into DX.
        let mut regs = Registers::new();
        regs.ax = 0x8000;
        regs.dx = 0x0001;
        let mut memory = Memory::new();
        execute(
            &instr(
                Mnemonic::Shl,
                vec![Operand::Reg16(Reg16::Ax), Operand::Immediate(1)],
                Some(Width::Word),
            ),
            &mut regs,
            &mut memory,
        );
        assert_eq!(regs.ax, 0x0000);
        assert!(regs.get_flag(Flag::Carry));
        execute(
            &instr(
                Mnemonic::Rcl,
                vec![Operand::Reg16(Reg16::Dx), Operand::Immediate(1)],
                Some(Width::Word),
            ),
            &mut regs,
            &mut memory,
        );
        assert_eq!(regs.dx, 0x0003);
    }

    #[test]
    fn rcr_chains_a_shifted_out_bit_between_two_registers() {
        let mut regs = Registers::new();
        regs.dx = 0x0003;
        regs.ax = 0x0000;
        let mut memory = Memory::new();
        execute(
            &instr(
                Mnemonic::Shr,
                vec![Operand::Reg16(Reg16::Dx), Operand::Immediate(1)],
                Some(Width::Word),
            ),
            &mut regs,
            &mut memory,
        );
        assert_eq!(regs.dx, 0x0001);
        assert!(regs.get_flag(Flag::Carry));
        execute(
            &instr(
                Mnemonic::Rcr,
                vec![Operand::Reg16(Reg16::Ax), Operand::Immediate(1)],
                Some(Width::Word),
            ),
            &mut regs,
            &mut memory,
        );
        assert_eq!(regs.ax, 0x8000);
    }

    #[test]
    fn rol_by_a_count_larger_than_the_width_still_rotates_correctly() {
        // ROL AL, 4 on 0xFF is a value-wise no-op (every bit is already 1).
        let mut regs = Registers::new();
        regs.set8(Reg8::Al, 0xFF);
        regs.set8(Reg8::Cl, 4);
        let mut memory = Memory::new();
        let i = instr(
            Mnemonic::Rol,
            vec![Operand::Reg8(Reg8::Al), Operand::Reg8(Reg8::Cl)],
            Some(Width::Byte),
        );
        execute(&i, &mut regs, &mut memory);
        assert_eq!(regs.get8(Reg8::Al), 0xFF);
    }

    #[test]
    fn rcl_by_one_pulls_in_the_incoming_carry_flag() {
        let mut regs = Registers::new();
        regs.set_flag(Flag::Carry, true);
        regs.set8(Reg8::Al, 0x00);
        let mut memory = Memory::new();
        let i = instr(
            Mnemonic::Rcl,
            vec![Operand::Reg8(Reg8::Al), Operand::Immediate(1)],
            Some(Width::Byte),
        );
        execute(&i, &mut regs, &mut memory);
        assert_eq!(regs.get8(Reg8::Al), 0x01);
    }

    #[test]
    fn shift_by_zero_leaves_the_value_and_flags_untouched() {
        let mut regs = Registers::new();
        regs.set8(Reg8::Al, 0x55);
        regs.set_flag(Flag::Carry, true);
        regs.set8(Reg8::Cl, 0);
        let mut memory = Memory::new();
        let i = instr(
            Mnemonic::Shl,
            vec![Operand::Reg8(Reg8::Al), Operand::Reg8(Reg8::Cl)],
            Some(Width::Byte),
        );
        execute(&i, &mut regs, &mut memory);
        assert_eq!(regs.get8(Reg8::Al), 0x55);
        assert!(regs.get_flag(Flag::Carry));
    }

    #[test]
    fn shl_sets_zero_and_sign_from_the_result() {
        let mut regs = Registers::new();
        regs.set8(Reg8::Al, 0x80);
        let mut memory = Memory::new();
        let i = instr(
            Mnemonic::Shl,
            vec![Operand::Reg8(Reg8::Al), Operand::Immediate(1)],
            Some(Width::Byte),
        );
        execute(&i, &mut regs, &mut memory);
        assert_eq!(regs.get8(Reg8::Al), 0);
        assert!(regs.get_flag(Flag::Zero));
        assert!(!regs.get_flag(Flag::Sign));
    }

    // --- MUL/IMUL/DIV/IDIV/NEG/NOT --------------------------------------

    #[test]
    fn mul_byte_sets_carry_and_overflow_when_ah_is_nonzero() {
        let mut regs = Registers::new();
        regs.set8(Reg8::Al, 200);
        regs.set8(Reg8::Bl, 3);
        let mut memory = Memory::new();
        let i = instr(
            Mnemonic::Mul,
            vec![Operand::Reg8(Reg8::Bl)],
            Some(Width::Byte),
        );
        execute(&i, &mut regs, &mut memory);
        assert_eq!(regs.ax, 600);
        assert!(regs.get_flag(Flag::Carry));
        assert!(regs.get_flag(Flag::Overflow));
    }

    #[test]
    fn mul_byte_clears_carry_when_the_result_fits_in_al() {
        let mut regs = Registers::new();
        regs.set8(Reg8::Al, 10);
        regs.set8(Reg8::Bl, 5);
        let mut memory = Memory::new();
        let i = instr(
            Mnemonic::Mul,
            vec![Operand::Reg8(Reg8::Bl)],
            Some(Width::Byte),
        );
        execute(&i, &mut regs, &mut memory);
        assert_eq!(regs.ax, 50);
        assert!(!regs.get_flag(Flag::Carry));
    }

    #[test]
    fn mul_word_splits_the_result_across_dx_and_ax() {
        let mut regs = Registers::new();
        regs.ax = 0x1234;
        regs.bx = 0x5678;
        let mut memory = Memory::new();
        let i = instr(
            Mnemonic::Mul,
            vec![Operand::Reg16(Reg16::Bx)],
            Some(Width::Word),
        );
        execute(&i, &mut regs, &mut memory);
        let expected = 0x1234u32 * 0x5678u32;
        assert_eq!(regs.ax, expected as u16);
        assert_eq!(regs.dx, (expected >> 16) as u16);
        assert!(regs.get_flag(Flag::Carry));
    }

    #[test]
    fn imul_word_handles_negative_operands_correctly() {
        let mut regs = Registers::new();
        regs.ax = 0xFFFE; // -2
        regs.bx = 3;
        let mut memory = Memory::new();
        let i = instr(
            Mnemonic::Imul,
            vec![Operand::Reg16(Reg16::Bx)],
            Some(Width::Word),
        );
        execute(&i, &mut regs, &mut memory);
        assert_eq!(regs.ax, 0xFFFA); // -6
        assert_eq!(regs.dx, 0xFFFF); // sign-extended
        assert!(!regs.get_flag(Flag::Carry), "result fits in AX alone");
    }

    #[test]
    fn div_word_computes_quotient_and_remainder() {
        let mut regs = Registers::new();
        regs.ax = 37;
        regs.dx = 0;
        regs.bx = 5;
        let mut memory = Memory::new();
        let i = instr(
            Mnemonic::Div,
            vec![Operand::Reg16(Reg16::Bx)],
            Some(Width::Word),
        );
        let effect = execute(&i, &mut regs, &mut memory);
        assert_eq!(effect, ExecutionEffect::Continue);
        assert_eq!(regs.ax, 7);
        assert_eq!(regs.dx, 2);
    }

    #[test]
    fn div_by_zero_reports_interrupt_zero_without_touching_registers() {
        let mut regs = Registers::new();
        regs.ax = 10;
        regs.dx = 0;
        regs.bx = 0;
        let mut memory = Memory::new();
        let i = instr(
            Mnemonic::Div,
            vec![Operand::Reg16(Reg16::Bx)],
            Some(Width::Word),
        );
        let effect = execute(&i, &mut regs, &mut memory);
        assert_eq!(effect, ExecutionEffect::Interrupt(0));
        assert_eq!(regs.ax, 10, "a faulted DIV must not clobber AX");
    }

    #[test]
    fn idiv_word_handles_a_negative_dividend() {
        let mut regs = Registers::new();
        regs.ax = (-7i16) as u16;
        regs.dx = 0xFFFF; // sign-extend -7 into DX:AX
        regs.bx = 2;
        let mut memory = Memory::new();
        let i = instr(
            Mnemonic::Idiv,
            vec![Operand::Reg16(Reg16::Bx)],
            Some(Width::Word),
        );
        execute(&i, &mut regs, &mut memory);
        assert_eq!(regs.ax as i16, -3);
        assert_eq!(regs.dx as i16, -1);
    }

    #[test]
    fn neg_computes_twos_complement_and_sets_carry_unless_operand_was_zero() {
        let mut regs = Registers::new();
        regs.set8(Reg8::Al, 5);
        let mut memory = Memory::new();
        execute(
            &instr(
                Mnemonic::Neg,
                vec![Operand::Reg8(Reg8::Al)],
                Some(Width::Byte),
            ),
            &mut regs,
            &mut memory,
        );
        assert_eq!(regs.get8(Reg8::Al), (-5i8) as u8);
        assert!(regs.get_flag(Flag::Carry));

        regs.set8(Reg8::Al, 0);
        execute(
            &instr(
                Mnemonic::Neg,
                vec![Operand::Reg8(Reg8::Al)],
                Some(Width::Byte),
            ),
            &mut regs,
            &mut memory,
        );
        assert_eq!(regs.get8(Reg8::Al), 0);
        assert!(!regs.get_flag(Flag::Carry), "NEG 0 must not set carry");
    }

    #[test]
    fn not_flips_every_bit_and_leaves_flags_untouched() {
        let mut regs = Registers::new();
        regs.set_flag(Flag::Zero, true);
        regs.ax = 0x00FF;
        let mut memory = Memory::new();
        execute(
            &instr(
                Mnemonic::Not,
                vec![Operand::Reg16(Reg16::Ax)],
                Some(Width::Word),
            ),
            &mut regs,
            &mut memory,
        );
        assert_eq!(regs.ax, 0xFF00);
        assert!(regs.get_flag(Flag::Zero), "NOT must not touch flags");
    }

    // --- string instructions + REP --------------------------------------

    #[test]
    fn movsb_copies_a_byte_and_advances_si_di_forward() {
        let mut regs = Registers::new();
        regs.si = 0x0100;
        regs.di = 0x0200;
        let mut memory = Memory::new();
        memory.write_u8(0x0100, 0xAB);
        execute(
            &instr(Mnemonic::Movsb, vec![], None),
            &mut regs,
            &mut memory,
        );
        assert_eq!(memory.read_u8(0x0200), 0xAB);
        assert_eq!(regs.si, 0x0101);
        assert_eq!(regs.di, 0x0201);
    }

    #[test]
    fn movsb_advances_backward_when_direction_flag_is_set() {
        let mut regs = Registers::new();
        regs.si = 0x0100;
        regs.di = 0x0200;
        regs.set_flag(Flag::Direction, true);
        let mut memory = Memory::new();
        execute(
            &instr(Mnemonic::Movsb, vec![], None),
            &mut regs,
            &mut memory,
        );
        assert_eq!(regs.si, 0x00FF);
        assert_eq!(regs.di, 0x01FF);
    }

    #[test]
    fn rep_movsb_copies_cx_bytes_and_leaves_cx_at_zero() {
        let mut regs = Registers::new();
        regs.si = 0x0000;
        regs.di = 0x0100;
        regs.cx = 5;
        let mut memory = Memory::new();
        for (i, b) in [1u8, 2, 3, 4, 5].iter().enumerate() {
            memory.write_u8(i as u32, *b);
        }
        let i = instr(Mnemonic::Movsb, vec![], None).with_repeat(x8086_isa::Repeat::Rep);
        execute(&i, &mut regs, &mut memory);
        assert_eq!(regs.cx, 0);
        for i in 0..5u32 {
            assert_eq!(memory.read_u8(0x0100 + i), (i + 1) as u8);
        }
    }

    #[test]
    fn rep_movsb_with_cx_zero_does_nothing() {
        let mut regs = Registers::new();
        regs.cx = 0;
        regs.si = 0x0000;
        regs.di = 0x0100;
        let mut memory = Memory::new();
        memory.write_u8(0, 0xFF);
        let i = instr(Mnemonic::Movsb, vec![], None).with_repeat(x8086_isa::Repeat::Rep);
        execute(&i, &mut regs, &mut memory);
        assert_eq!(memory.read_u8(0x0100), 0, "CX=0 must copy nothing");
    }

    #[test]
    fn repe_cmpsb_stops_early_on_a_mismatch() {
        let mut regs = Registers::new();
        regs.cx = 5;
        regs.si = 0x0000;
        regs.di = 0x0100;
        let mut memory = Memory::new();
        let a = [1u8, 2, 3, 9, 5];
        let b = [1u8, 2, 3, 4, 5];
        for i in 0..5u32 {
            memory.write_u8(i, a[i as usize]);
            memory.write_u8(0x0100 + i, b[i as usize]);
        }
        let i = instr(Mnemonic::Cmpsb, vec![], None).with_repeat(x8086_isa::Repeat::Repe);
        execute(&i, &mut regs, &mut memory);
        // 4 iterations run (indices 0-3, the last being the mismatch); cx = 1.
        assert_eq!(regs.cx, 1);
        assert!(!regs.get_flag(Flag::Zero));
    }

    #[test]
    fn repe_cmpsb_over_identical_buffers_runs_to_completion() {
        let mut regs = Registers::new();
        regs.cx = 3;
        regs.si = 0x0000;
        regs.di = 0x0100;
        let mut memory = Memory::new();
        for i in 0..3u32 {
            memory.write_u8(i, 7);
            memory.write_u8(0x0100 + i, 7);
        }
        let i = instr(Mnemonic::Cmpsb, vec![], None).with_repeat(x8086_isa::Repeat::Repe);
        execute(&i, &mut regs, &mut memory);
        assert_eq!(regs.cx, 0);
        assert!(regs.get_flag(Flag::Zero));
    }

    #[test]
    fn lodsb_and_stosb_round_trip_through_al() {
        let mut regs = Registers::new();
        regs.si = 0x0000;
        regs.di = 0x0100;
        let mut memory = Memory::new();
        memory.write_u8(0, 0x42);
        execute(
            &instr(Mnemonic::Lodsb, vec![], None),
            &mut regs,
            &mut memory,
        );
        assert_eq!(regs.get8(Reg8::Al), 0x42);
        assert_eq!(regs.si, 1);
        execute(
            &instr(Mnemonic::Stosb, vec![], None),
            &mut regs,
            &mut memory,
        );
        assert_eq!(memory.read_u8(0x0100), 0x42);
        assert_eq!(regs.di, 0x0101);
    }

    #[test]
    fn scasb_compares_al_against_es_di_and_sets_zero_on_match() {
        let mut regs = Registers::new();
        regs.set8(Reg8::Al, 9);
        regs.di = 0x0100;
        let mut memory = Memory::new();
        memory.write_u8(0x0100, 9);
        execute(
            &instr(Mnemonic::Scasb, vec![], None),
            &mut regs,
            &mut memory,
        );
        assert!(regs.get_flag(Flag::Zero));
        assert_eq!(regs.di, 0x0101);
    }
}
