//! Stateful facade: owns the CPU, memory, decoder, debugger, and
//! interrupt-simulation layer, and exposes the single API that
//! `x8086-ffi` wraps for Swift.
//!
//! This crate intentionally has zero uniffi/Swift awareness, so the
//! entire core is testable with plain `cargo test` and no Swift
//! toolchain. The background-thread/command-channel design described in
//! the architecture plan (so `run()` never blocks the FFI-calling
//! thread) is still ahead of this facade - `step()` is synchronous
//! today. What *is* here is the behavior that design exists to support:
//! a blocking keyboard read (INT 16h/21h) suspends mid-program via
//! `StepOutcome::WaitingForKeyboard` rather than either blocking the
//! caller or silently completing with garbage input, and `feed_key`
//! resumes it. That poll-and-resume contract is exactly what a
//! background thread would eventually sit behind; adding the thread
//! later changes how callers *wait* for it, not this state machine.

use x8086_cpu::{ExecutionEffect, Registers};
use x8086_debugger::{Breakpoints, History, HistoryEntry};
use x8086_decoder::{decode_one, DecodeError};
use x8086_interrupts::{handle_interrupt, InterruptOutcome, IoSink};
use x8086_memory::Memory;

/// Longest possible 8086 instruction encoding is 6 bytes (full ModRM +
/// displacement + immediate); the decoder never needs to look further.
const MAX_INSTRUCTION_LEN: usize = 6;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepOutcome {
    Continued,
    Halted,
    /// A blocking keyboard read (INT 16h/21h) is in progress. No new
    /// instruction will be fetched until `feed_key` supplies input and
    /// `step` is called again.
    WaitingForKeyboard,
}

/// In-memory `IoSink`: accumulates console output as plain text and
/// holds at most one pending keystroke. This is what a real UI (or a
/// test) drives directly - `console_write`/`console_clear` model what
/// the eventual FFI callback interface forwards to Swift, and
/// `read_key` is exactly what `Emulator::feed_key` populates.
#[derive(Debug, Default)]
struct ConsoleSink {
    output: String,
    pending_key: Option<(u8, u8)>,
}

impl IoSink for ConsoleSink {
    fn console_write(&mut self, text: &str) {
        self.output.push_str(text);
    }
    fn console_clear(&mut self) {
        self.output.clear();
    }
    fn read_key(&mut self) -> Option<(u8, u8)> {
        self.pending_key.take()
    }
}

pub struct Emulator {
    pub registers: Registers,
    pub memory: Memory,
    pub breakpoints: Breakpoints,
    history: History,
    pub halted: bool,
    console: ConsoleSink,
    /// Set when a step ended on a keyboard-read interrupt that couldn't
    /// complete yet; `step` retries this same vector (rather than
    /// fetching a new instruction) until it succeeds.
    pending_interrupt: Option<u8>,
}

impl Emulator {
    pub fn new() -> Self {
        Self {
            registers: Registers::new(),
            memory: Memory::new(),
            breakpoints: Breakpoints::new(),
            history: History::new(10_000),
            halted: false,
            console: ConsoleSink::default(),
            pending_interrupt: None,
        }
    }

    pub fn reset(&mut self) {
        *self = Self::new();
    }

    /// Load raw machine code starting at CS:IP = 0000:0000. Real segment
    /// layout (matching emu8086's default `.MODEL SMALL` placement)
    /// arrives once multi-segment layout matters to the emulator; today
    /// everything lives in one flat image, matching the assembler's own
    /// scope.
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

    /// Everything written by simulated console-output interrupts so
    /// far. `step_back` deliberately does not unwind this - like real
    /// hardware, undoing a CPU state doesn't un-print what already
    /// reached the screen.
    pub fn console_output(&self) -> &str {
        &self.console.output
    }

    /// Supplies a keystroke to satisfy a pending `StepOutcome::
    /// WaitingForKeyboard`. Safe to call even when nothing is waiting -
    /// the key is simply buffered for the next blocking read.
    pub fn feed_key(&mut self, scancode: u8, ascii: u8) {
        self.console.pending_key = Some((scancode, ascii));
    }

    /// Decode and execute a single instruction at the current CS:IP, or
    /// (if a keyboard read is pending) retry completing it.
    pub fn step(&mut self) -> Result<StepOutcome, DecodeError> {
        if self.halted {
            return Ok(StepOutcome::Halted);
        }

        if let Some(vector) = self.pending_interrupt {
            return Ok(self.try_complete_interrupt(vector));
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
        self.history.push(HistoryEntry {
            registers_before,
            memory_diffs: vec![],
        });

        Ok(
            match x8086_cpu::execute(&instruction, &mut self.registers, &mut self.memory) {
                ExecutionEffect::Halted => {
                    self.halted = true;
                    StepOutcome::Halted
                }
                ExecutionEffect::Continue => StepOutcome::Continued,
                ExecutionEffect::Interrupt(vector) => self.try_complete_interrupt(vector),
            },
        )
    }

    fn try_complete_interrupt(&mut self, vector: u8) -> StepOutcome {
        match handle_interrupt(vector, &mut self.registers, &self.memory, &mut self.console) {
            InterruptOutcome::Continue => {
                self.pending_interrupt = None;
                StepOutcome::Continued
            }
            InterruptOutcome::Terminate { .. } => {
                self.pending_interrupt = None;
                self.halted = true;
                StepOutcome::Halted
            }
            InterruptOutcome::NeedsKeyboardInput => {
                self.pending_interrupt = Some(vector);
                StepOutcome::WaitingForKeyboard
            }
        }
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
                self.pending_interrupt = None;
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

    #[test]
    fn int21h_09h_prints_a_dollar_terminated_string_to_the_console() {
        let mut emulator = Emulator::new();
        // MOV DX, 0x0008 (address of the string below); MOV AH, 9;
        // INT 21h; HLT; DB "Hi$"
        let program: &[u8] = &[
            0xBA, 0x08, 0x00, 0xB4, 0x09, 0xCD, 0x21, 0xF4, b'H', b'i', b'$',
        ];
        emulator.load_program(program);
        loop {
            match emulator.step().unwrap() {
                StepOutcome::Halted => break,
                StepOutcome::Continued => {}
                StepOutcome::WaitingForKeyboard => panic!("unexpected keyboard wait"),
            }
        }
        assert_eq!(emulator.console_output(), "Hi");
    }

    #[test]
    fn int16h_blocks_until_fed_a_key_then_resumes() {
        let mut emulator = Emulator::new();
        // MOV AH, 0; INT 16h; MOV DX, AX; HLT
        let program: &[u8] = &[0xB4, 0x00, 0xCD, 0x16, 0x89, 0xC2, 0xF4];
        emulator.load_program(program);

        // Step through MOV AH,0.
        assert_eq!(emulator.step().unwrap(), StepOutcome::Continued);
        // INT 16h: no key yet, must report waiting rather than hang or
        // fabricate a result.
        assert_eq!(emulator.step().unwrap(), StepOutcome::WaitingForKeyboard);
        assert_eq!(
            emulator.step().unwrap(),
            StepOutcome::WaitingForKeyboard,
            "must keep waiting, not silently give up"
        );

        emulator.feed_key(0x1E, b'a');
        assert_eq!(emulator.step().unwrap(), StepOutcome::Continued);
        assert_eq!(emulator.registers.ax, 0x1E61);

        assert_eq!(emulator.step().unwrap(), StepOutcome::Continued); // MOV DX, AX
        assert_eq!(emulator.registers.dx, 0x1E61);
        assert_eq!(emulator.step().unwrap(), StepOutcome::Halted);
    }

    #[test]
    fn feed_key_before_any_wait_is_harmless_and_buffers_for_later() {
        let mut emulator = Emulator::new();
        // MOV AH, 0; INT 16h; HLT
        emulator.load_program(&[0xB4, 0x00, 0xCD, 0x16, 0xF4]);
        // Fed proactively, before the program has even asked for it -
        // load_program (via reset) must run first, since resetting
        // after feeding a key would silently discard it.
        emulator.feed_key(0x1E, b'a');
        assert_eq!(emulator.step().unwrap(), StepOutcome::Continued); // MOV AH,0
        assert_eq!(emulator.step().unwrap(), StepOutcome::Continued); // INT 16h completes immediately
        assert_eq!(emulator.registers.ax, 0x1E61);
    }
}
