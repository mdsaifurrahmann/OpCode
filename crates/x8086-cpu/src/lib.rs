//! 8086/80186 register file, FLAGS, and (eventually) instruction execution.
//!
//! Full opcode execution semantics land during the CPU-core build phase.
//! This scaffold establishes the register file and FLAGS bit layout,
//! since those are exercised directly by `x8086-interrupts` and
//! `x8086-debugger` regardless of how much of the opcode table exists yet.

mod execute;
mod flags;
mod operand;

pub use execute::{execute, ExecutionEffect};

use x8086_isa::{Flag, Reg16, Reg8};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Registers {
    pub ax: u16,
    pub bx: u16,
    pub cx: u16,
    pub dx: u16,
    pub sp: u16,
    pub bp: u16,
    pub si: u16,
    pub di: u16,
    pub cs: u16,
    pub ds: u16,
    pub es: u16,
    pub ss: u16,
    pub ip: u16,
    pub flags: u16,
}

/// Bit position of each FLAGS bit in the 8086 FLAGS register.
fn flag_bit(flag: Flag) -> u16 {
    match flag {
        Flag::Carry => 0,
        Flag::Parity => 2,
        Flag::AuxCarry => 4,
        Flag::Zero => 6,
        Flag::Sign => 7,
        Flag::Trap => 8,
        Flag::Interrupt => 9,
        Flag::Direction => 10,
        Flag::Overflow => 11,
    }
}

impl Registers {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn get16(&self, reg: Reg16) -> u16 {
        match reg {
            Reg16::Ax => self.ax,
            Reg16::Bx => self.bx,
            Reg16::Cx => self.cx,
            Reg16::Dx => self.dx,
            Reg16::Sp => self.sp,
            Reg16::Bp => self.bp,
            Reg16::Si => self.si,
            Reg16::Di => self.di,
            Reg16::Cs => self.cs,
            Reg16::Ds => self.ds,
            Reg16::Es => self.es,
            Reg16::Ss => self.ss,
            Reg16::Ip => self.ip,
        }
    }

    pub fn set16(&mut self, reg: Reg16, value: u16) {
        match reg {
            Reg16::Ax => self.ax = value,
            Reg16::Bx => self.bx = value,
            Reg16::Cx => self.cx = value,
            Reg16::Dx => self.dx = value,
            Reg16::Sp => self.sp = value,
            Reg16::Bp => self.bp = value,
            Reg16::Si => self.si = value,
            Reg16::Di => self.di = value,
            Reg16::Cs => self.cs = value,
            Reg16::Ds => self.ds = value,
            Reg16::Es => self.es = value,
            Reg16::Ss => self.ss = value,
            Reg16::Ip => self.ip = value,
        }
    }

    pub fn get8(&self, reg: Reg8) -> u8 {
        match reg {
            Reg8::Al => self.ax as u8,
            Reg8::Ah => (self.ax >> 8) as u8,
            Reg8::Bl => self.bx as u8,
            Reg8::Bh => (self.bx >> 8) as u8,
            Reg8::Cl => self.cx as u8,
            Reg8::Ch => (self.cx >> 8) as u8,
            Reg8::Dl => self.dx as u8,
            Reg8::Dh => (self.dx >> 8) as u8,
        }
    }

    pub fn set8(&mut self, reg: Reg8, value: u8) {
        let merge = |word: u16, high: bool| -> u16 {
            if high {
                (word & 0x00FF) | ((value as u16) << 8)
            } else {
                (word & 0xFF00) | (value as u16)
            }
        };
        match reg {
            Reg8::Al => self.ax = merge(self.ax, false),
            Reg8::Ah => self.ax = merge(self.ax, true),
            Reg8::Bl => self.bx = merge(self.bx, false),
            Reg8::Bh => self.bx = merge(self.bx, true),
            Reg8::Cl => self.cx = merge(self.cx, false),
            Reg8::Ch => self.cx = merge(self.cx, true),
            Reg8::Dl => self.dx = merge(self.dx, false),
            Reg8::Dh => self.dx = merge(self.dx, true),
        }
    }

    pub fn get_flag(&self, flag: Flag) -> bool {
        (self.flags >> flag_bit(flag)) & 1 == 1
    }

    pub fn set_flag(&mut self, flag: Flag, value: bool) {
        let bit = flag_bit(flag);
        if value {
            self.flags |= 1 << bit;
        } else {
            self.flags &= !(1 << bit);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registers_default_to_zero() {
        let regs = Registers::new();
        assert_eq!(regs.ax, 0);
        assert_eq!(regs.flags, 0);
    }

    #[test]
    fn get16_set16_round_trip_for_every_register() {
        let all = [
            Reg16::Ax,
            Reg16::Bx,
            Reg16::Cx,
            Reg16::Dx,
            Reg16::Sp,
            Reg16::Bp,
            Reg16::Si,
            Reg16::Di,
            Reg16::Cs,
            Reg16::Ds,
            Reg16::Es,
            Reg16::Ss,
            Reg16::Ip,
        ];
        for reg in all {
            let mut regs = Registers::new();
            regs.set16(reg, 0xBEEF);
            assert_eq!(regs.get16(reg), 0xBEEF);
        }
    }

    #[test]
    fn set8_only_touches_its_own_half_of_the_word() {
        let mut regs = Registers::new();
        regs.ax = 0x1234;
        regs.set8(Reg8::Al, 0xFF);
        assert_eq!(regs.ax, 0x12FF);
        regs.set8(Reg8::Ah, 0xAB);
        assert_eq!(regs.ax, 0xABFF);
    }

    #[test]
    fn get8_reads_the_correct_half_of_the_word() {
        let mut regs = Registers::new();
        regs.dx = 0xCAFE;
        assert_eq!(regs.get8(Reg8::Dh), 0xCA);
        assert_eq!(regs.get8(Reg8::Dl), 0xFE);
    }

    #[test]
    fn flag_get_set_round_trip_and_do_not_disturb_other_bits() {
        let all = [
            Flag::Carry,
            Flag::Parity,
            Flag::AuxCarry,
            Flag::Zero,
            Flag::Sign,
            Flag::Trap,
            Flag::Interrupt,
            Flag::Direction,
            Flag::Overflow,
        ];
        let mut regs = Registers::new();
        for flag in all {
            regs.set_flag(flag, true);
            assert!(regs.get_flag(flag), "{flag:?} should be set");
        }
        for flag in all {
            assert!(regs.get_flag(flag), "{flag:?} should still be set");
            regs.set_flag(flag, false);
            assert!(!regs.get_flag(flag), "{flag:?} should be cleared");
        }
        assert_eq!(regs.flags, 0);
    }
}
