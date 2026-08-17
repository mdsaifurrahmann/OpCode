//! Stateful facade: owns the CPU, memory, decoder, debugger, and (once
//! wired in a later phase) the interrupt-simulation layer, and exposes
//! the single API that `x8086-ffi` wraps for Swift.
//!
//! This crate intentionally has zero uniffi/Swift awareness, so the
//! entire core is testable with plain `cargo test` and no Swift
//! toolchain. The background-thread/command-channel design described in
//! the architecture plan (so `run()` never blocks the FFI-calling
//! thread) lands with the interrupts-integration phase, once there is
//! actually a blocking keyboard-read to design around; today `step()` is
//! synchronous.

use x8086_cpu::{ExecutionEffect, Registers};
use x8086_debugger::{Breakpoints, History, HistoryEntry};
use x8086_decoder::{decode_one, DecodeError};
use x8086_memory::Memory;

/// Longest possible 8086 instruction encoding is 6 bytes (full ModRM +
/// displacement + immediate); the decoder never needs to look further.
const MAX_INSTRUCTION_LEN: usize = 6;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepOutcome {
    Continued,
    Halted,
}

pub struct Emulator {
    pub registers: Registers,
    pub memory: Memory,
    pub breakpoints: Breakpoints,
    history: History,
    pub halted: bool,
}

impl Emulator {
    pub fn new() -> Self {
        Self {
            registers: Registers::new(),
            memory: Memory::new(),
            breakpoints: Breakpoints::new(),
            history: History::new(10_000),
            halted: false,
        }
    }

    pub fn reset(&mut self) {
        *self = Self::new();
    }

    /// Load raw machine code starting at CS:IP = 0000:0000. Real segment
    /// layout (matching emu8086's default `.MODEL SMALL` placement)
    /// arrives once the assembler is wired in for real; today
    /// `x8086-assembler::assemble` always returns an empty program, so
    /// `assemble_and_load` is a real code path with nothing to exercise
    /// yet.
    pub fn load_program(&mut self, machine_code: &[u8]) {
        self.reset();
        for (offset, byte) in machine_code.iter().enumerate() {
            self.memory.write_u8(offset as u32, *byte);
        }
    }

    pub fn assemble_and_load(&mut self, source: &str) -> x8086_assembler::AssembleResult {
        let result = x8086_assembler::assemble(source);
        self.load_program(&result.machine_code);
        self.registers.ip = result.entry_point as u16;
        result
    }

    /// Decode and execute a single instruction at the current CS:IP.
    pub fn step(&mut self) -> Result<StepOutcome, DecodeError> {
        if self.halted {
            return Ok(StepOutcome::Halted);
        }

        let ip = self.registers.ip as u32;
        let mut window = [0u8; MAX_INSTRUCTION_LEN];
        for (offset, slot) in window.iter_mut().enumerate() {
            *slot = self.memory.read_u8(ip.wrapping_add(offset as u32));
        }
        let (instruction, len) = decode_one(&window)?;

        let registers_before = self.registers;
        // x8086_cpu::execute expects IP to already point past the
        // instruction being run (see its module docs) - relative jumps
        // measure their displacement from here.
        self.registers.ip = self.registers.ip.wrapping_add(len as u16);

        // Simulated interrupts (INT 21h/10h/etc.) are dispatched through
        // x8086-interrupts once that's wired into this facade in a later
        // phase; for now `ExecutionEffect::Interrupt` is a no-op beyond
        // the IP advance already applied above.
        match x8086_cpu::execute(&instruction, &mut self.registers, &mut self.memory) {
            ExecutionEffect::Halted => self.halted = true,
            ExecutionEffect::Continue | ExecutionEffect::Interrupt(_) => {}
        }

        self.history.push(HistoryEntry {
            registers_before,
            memory_diffs: vec![],
        });

        Ok(if self.halted {
            StepOutcome::Halted
        } else {
            StepOutcome::Continued
        })
    }

    /// Undo the most recent step. Returns false if there is no history
    /// to undo, in which case this is a no-op rather than an error.
    pub fn step_back(&mut self) -> bool {
        match self.history.pop() {
            Some(entry) => {
                for (address, old_value) in entry.memory_diffs.into_iter().rev() {
                    self.memory.write_u8(address, old_value);
                }
                self.registers = entry.registers_before;
                self.halted = false;
                true
            }
            None => false,
        }
    }
}

impl Default for Emulator {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stepping_over_hlt_halts_the_emulator() {
        let mut emulator = Emulator::new();
        emulator.load_program(&[0xF4]); // HLT
        assert_eq!(emulator.step().unwrap(), StepOutcome::Halted);
        assert!(emulator.halted);
        assert_eq!(emulator.registers.ip, 1);
    }

    #[test]
    fn stepping_while_halted_is_idempotent() {
        let mut emulator = Emulator::new();
        emulator.load_program(&[0xF4]);
        emulator.step().unwrap();
        assert_eq!(emulator.step().unwrap(), StepOutcome::Halted);
        assert_eq!(emulator.registers.ip, 1); // did not advance a second time
    }

    #[test]
    fn stepping_an_unknown_opcode_is_an_error() {
        let mut emulator = Emulator::new();
        emulator.load_program(&[0x0F]); // reserved on 8086/80186, deliberately undecoded
        assert!(emulator.step().is_err());
    }

    #[test]
    fn step_back_undoes_the_halt_and_restores_ip() {
        let mut emulator = Emulator::new();
        emulator.load_program(&[0xF4]);
        emulator.step().unwrap();
        assert!(emulator.halted);

        assert!(emulator.step_back());
        assert!(!emulator.halted);
        assert_eq!(emulator.registers.ip, 0);
    }

    #[test]
    fn step_back_on_fresh_emulator_is_a_no_op() {
        let mut emulator = Emulator::new();
        assert!(!emulator.step_back());
    }

    #[test]
    fn reset_clears_registers_memory_and_halted_state() {
        let mut emulator = Emulator::new();
        emulator.load_program(&[0xF4]);
        emulator.step().unwrap();
        emulator.reset();
        assert!(!emulator.halted);
        assert_eq!(emulator.registers.ip, 0);
        assert_eq!(emulator.memory.read_u8(0), 0);
    }
}
