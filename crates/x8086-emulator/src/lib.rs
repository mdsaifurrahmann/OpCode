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

use std::sync::{Arc, Mutex};

use x8086_cpu::{ExecutionEffect, Registers};
use x8086_debugger::{Breakpoints, History, HistoryEntry};
use x8086_decoder::{decode_one, format_instruction, DecodeError};
use x8086_interrupts::{handle_interrupt, InterruptOutcome, IoSink};
use x8086_memory::{Memory, WriteObserver};

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

/// The reason a multi-step `run`/`run_to_cursor` call stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunOutcome {
    Halted,
    WaitingForKeyboard,
    BreakpointHit,
    DecodeError,
    /// Reached the caller's step ceiling without stopping for any other
    /// reason - a runaway program (or a decoder/CPU bug), not a real
    /// stopping condition. The caller may call `run` again to keep going.
    StepLimitReached,
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

/// Notified on every byte write via `Memory::set_observer`, so `step` can
/// drain exactly the diffs one step made into that step's `HistoryEntry`.
/// Shared via `Arc<Mutex<_>>` rather than handed back and forth, since
/// `Memory` takes ownership of its observer as a `Box<dyn WriteObserver>`
/// and has no way to return it - the mutex is uncontended in practice
/// because the emulator is single-threaded internally.
#[derive(Clone, Default)]
struct DiffRecorder(Arc<Mutex<Vec<(u32, u8, u8)>>>);

impl WriteObserver for DiffRecorder {
    fn on_write(&mut self, address: u32, old_value: u8, new_value: u8) {
        self.0.lock().unwrap().push((address, old_value, new_value));
    }
}

/// A watch expression's target, resolved fresh on every read rather than
/// cached at add-time - see `Emulator::resolve_watch` for why (a cached
/// address would go stale across a reassemble that relocates a variable).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ResolvedWatch {
    Live(x8086_debugger::watch::WatchTarget),
    /// A `Label` or `EQU` `Constant` symbol - a plain number, not a
    /// memory location to read through.
    Constant(u16),
}

/// One row of the live Variables panel: every `DB`/`DW` symbol from the
/// last assemble, with its current value read back from memory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VariableValue {
    pub name: String,
    pub address: u32,
    pub value: u16,
    pub is_word: bool,
}

/// One row of the Watch panel: the expression as typed, and its current
/// value - `None` if it no longer resolves (e.g. a variable-name watch
/// whose symbol vanished from the most recent assemble).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WatchValue {
    pub expression: String,
    pub value: Option<u16>,
}

/// One disassembled instruction, as produced by `Emulator::disassemble`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DisassembledLine {
    pub address: u32,
    pub text: String,
    pub byte_len: u32,
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
    diff_recorder: DiffRecorder,
    /// The symbol table from the most recent `assemble_and_load`, used to
    /// resolve named-variable watches and to drive the Variables panel.
    /// Empty until the first assemble.
    last_symbols: Vec<x8086_assembler::SymbolTableEntry>,
    /// The data segment's flat base address from the most recent
    /// assemble, needed because `last_symbols` entries for `Data`-kind
    /// symbols are offsets *relative to that segment* (see
    /// `x8086-assembler`'s codegen docs), not flat addresses - `0` when
    /// the program never declared a `.DATA` section, which is exactly
    /// correct since data-kind symbols can't exist without one.
    data_segment_base: u32,
    /// Watch expressions as typed, in add order. Deliberately just
    /// strings, not pre-resolved targets - see `ResolvedWatch`. Cleared
    /// on `reset`/`assemble_and_load` like everything else in `Emulator`;
    /// callers that want watches to survive a re-run (as breakpoints do)
    /// own that list themselves and re-add after each assemble, the same
    /// pattern the UI already uses for breakpoints.
    watch_expressions: Vec<String>,
}

impl Emulator {
    pub fn new() -> Self {
        let mut memory = Memory::new();
        let diff_recorder = DiffRecorder::default();
        memory.set_observer(Box::new(diff_recorder.clone()));
        Self {
            registers: Registers::new(),
            memory,
            breakpoints: Breakpoints::new(),
            history: History::new(10_000),
            halted: false,
            console: ConsoleSink::default(),
            pending_interrupt: None,
            diff_recorder,
            last_symbols: Vec::new(),
            data_segment_base: 0,
            watch_expressions: Vec::new(),
        }
    }

    /// Drains every byte write recorded since the last drain, as
    /// `(address, old_value)` undo pairs in write order.
    fn drain_memory_diffs(&self) -> Vec<(u32, u8)> {
        self.diff_recorder
            .0
            .lock()
            .unwrap()
            .drain(..)
            .map(|(address, old_value, _new_value)| (address, old_value))
            .collect()
    }

    /// Resets CPU/memory/breakpoint/history state for a fresh program
    /// load. Watch expressions are the one thing that survives - like a
    /// real debugger, the user's watch list is a property of the
    /// debugging *session*, not of any one loaded program, and should
    /// still be there (re-resolved against whatever the next assemble
    /// produces) after clicking Run again. Breakpoints don't get the same
    /// treatment: they're stored as raw addresses, which can silently
    /// drift to mean a different instruction after a reassemble, so
    /// keeping them line-number-keyed and re-resolved is the caller's
    /// job, not this facade's - see `EmulatorController` on the Swift
    /// side.
    pub fn reset(&mut self) {
        let watch_expressions = std::mem::take(&mut self.watch_expressions);
        *self = Self::new();
        self.watch_expressions = watch_expressions;
    }

    /// Load raw machine code starting at CS:IP = 0000:0000. Callers that
    /// need real `.MODEL SMALL`-style segment setup (DS/SS/SP pointed at
    /// a separate data/stack segment) should use `assemble_and_load`
    /// instead, which does that on top of this - this method itself only
    /// ever loads a single flat image starting at physical address 0.
    pub fn load_program(&mut self, machine_code: &[u8]) {
        self.reset();
        for (offset, byte) in machine_code.iter().enumerate() {
            self.memory.write_u8(offset as u32, *byte);
        }
        // These writes are the program load, not an instruction step -
        // draining (rather than letting the first real `step` inherit
        // them) keeps step_back from "undoing" the load itself.
        self.drain_memory_diffs();
    }

    /// Assembles `source` and loads it, then - unlike `load_program`
    /// alone - initializes DS/SS/SP from any `.DATA`/`.STACK` segments
    /// the source declared. DS is set to the data segment's paragraph
    /// value even though real DOS never does this for you (that's what
    /// the traditional `MOV AX,@DATA` / `MOV DS,AX` boilerplate is for) -
    /// a deliberate compatibility choice: it makes programs that already
    /// include that boilerplate work (they just redundantly re-set DS to
    /// what it already was), *and* programs that omit it (common in
    /// simple/tutorial code) work too, which a strictly DOS-accurate
    /// "leave DS alone" behavior would not. A program with no `.DATA`/
    /// `.STACK` at all sees no change here - DS/SS/SP all stay 0, exactly
    /// as before this existed.
    pub fn assemble_and_load(&mut self, source: &str) -> x8086_assembler::AssembleResult {
        let result = x8086_assembler::assemble(source);
        self.load_program(&result.machine_code);
        self.registers.ip = result.entry_point as u16;
        self.last_symbols = result.symbols.clone();
        self.data_segment_base = result.data_segment_base.unwrap_or(0);
        if let Some(data_base) = result.data_segment_base {
            self.registers.ds = (data_base / 16) as u16;
        }
        if let Some((stack_base, stack_size)) = result.stack_segment {
            self.registers.ss = (stack_base / 16) as u16;
            self.registers.sp = stack_size as u16;
        }
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
            let registers_before = self.registers;
            let outcome = self.try_complete_interrupt(vector);
            let memory_diffs = self.drain_memory_diffs();
            // Still waiting means nothing actually happened yet - no step
            // to undo, and pushing a no-op entry would let step_back
            // "rewind" to an identical state, wasting history capacity.
            if outcome != StepOutcome::WaitingForKeyboard {
                self.history.push(HistoryEntry {
                    registers_before,
                    memory_diffs,
                });
            }
            return Ok(outcome);
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

        let outcome = match x8086_cpu::execute(&instruction, &mut self.registers, &mut self.memory)
        {
            ExecutionEffect::Halted => {
                self.halted = true;
                StepOutcome::Halted
            }
            ExecutionEffect::Continue => StepOutcome::Continued,
            ExecutionEffect::Interrupt(vector) => self.try_complete_interrupt(vector),
        };
        let memory_diffs = self.drain_memory_diffs();
        if outcome != StepOutcome::WaitingForKeyboard {
            self.history.push(HistoryEntry {
                registers_before,
                memory_diffs,
            });
        }

        Ok(outcome)
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

    /// Whether `step_back` has anything to undo.
    pub fn can_step_back(&self) -> bool {
        !self.history.is_empty()
    }

    /// Step repeatedly until the program halts, blocks on keyboard input,
    /// hits a set breakpoint, hits a decode error, or `max_steps` is
    /// reached (a runaway-program ceiling, not a real stopping condition).
    ///
    /// A breakpoint at the *starting* IP does not stop this call
    /// immediately - clicking Run while already sitting on a breakpoint
    /// line must execute that line and continue, matching emu8086 and
    /// every other source debugger's convention. It only stops once a
    /// step lands the IP back on a set breakpoint.
    pub fn run(&mut self, max_steps: u32) -> RunOutcome {
        for _ in 0..max_steps {
            match self.step() {
                Ok(StepOutcome::Halted) => return RunOutcome::Halted,
                Ok(StepOutcome::WaitingForKeyboard) => return RunOutcome::WaitingForKeyboard,
                Ok(StepOutcome::Continued) => {
                    if self.breakpoints.is_set(self.registers.ip as u32) {
                        return RunOutcome::BreakpointHit;
                    }
                }
                Err(_) => return RunOutcome::DecodeError,
            }
        }
        RunOutcome::StepLimitReached
    }

    /// Run until `address` is reached, a real breakpoint is hit along the
    /// way, or the program halts/blocks/errors first. `address` acts as a
    /// one-shot breakpoint: if it wasn't already a permanent breakpoint,
    /// it is removed again before returning, regardless of outcome.
    pub fn run_to_cursor(&mut self, address: u32, max_steps: u32) -> RunOutcome {
        let already_set = self.breakpoints.is_set(address);
        if !already_set {
            self.breakpoints.toggle(address);
        }
        let outcome = self.run(max_steps);
        if !already_set {
            self.breakpoints.toggle(address);
        }
        outcome
    }

    /// Overwrites the whole register file at once - the live-edit path
    /// for the Registers panel. Not part of step-back history: like a
    /// real debugger, poking a register while paused is a direct state
    /// edit, not a CPU step.
    pub fn set_registers(&mut self, registers: Registers) {
        self.registers = registers;
    }

    /// Reads up to `len` bytes starting at `start`, for the Memory/Stack
    /// panels' on-demand, visible-range-only reads. `len` is clamped to
    /// the size of the address space, since nothing meaningful could ever
    /// be read past it in one call.
    pub fn read_memory(&self, start: u32, len: u32) -> Vec<u8> {
        let len = len.min(x8086_memory::MEMORY_SIZE as u32);
        (0..len)
            .map(|offset| self.memory.read_u8(start.wrapping_add(offset)))
            .collect()
    }

    /// Live-edits one memory cell - the Memory panel's write path. Not
    /// part of step-back history, for the same reason as `set_registers`.
    pub fn write_memory_byte(&mut self, address: u32, value: u8) {
        self.memory.write_u8(address, value);
        self.drain_memory_diffs();
    }

    /// Every `DB`/`DW` symbol from the last assemble, with its current
    /// value read back from memory - the Variables panel's data source.
    pub fn variables(&self) -> Vec<VariableValue> {
        self.last_symbols
            .iter()
            .filter_map(|symbol| {
                // `symbol.value` is an offset relative to the data
                // segment (see x8086-assembler's codegen docs) - add its
                // base to get the real flat address to read.
                let address = symbol.value as u32 + self.data_segment_base;
                match symbol.kind {
                    x8086_assembler::SymbolKind::DataByte => Some(VariableValue {
                        name: symbol.name.clone(),
                        address,
                        value: self.memory.read_u8(address) as u16,
                        is_word: false,
                    }),
                    x8086_assembler::SymbolKind::DataWord => Some(VariableValue {
                        name: symbol.name.clone(),
                        address,
                        value: self.memory.read_u16(address),
                        is_word: true,
                    }),
                    x8086_assembler::SymbolKind::Label | x8086_assembler::SymbolKind::Constant => {
                        None
                    }
                }
            })
            .collect()
    }

    /// Resolves a watch expression against the fixed register/flag/raw-
    /// address vocabulary first, then (for anything that doesn't match)
    /// against the last-assembled symbol table by name. Resolved fresh
    /// on every call rather than cached, so a variable-name watch always
    /// reflects the *current* assemble - caching the address at add-time
    /// would leave it pointing at a stale location after the user edits
    /// and re-runs a program that relocates the variable.
    fn resolve_watch(&self, expression: &str) -> Option<ResolvedWatch> {
        if let Some(target) = x8086_debugger::watch::parse_register_or_raw_address(expression) {
            return Some(ResolvedWatch::Live(target));
        }
        let symbol = self
            .last_symbols
            .iter()
            .find(|s| s.name == expression)
            .or_else(|| {
                self.last_symbols
                    .iter()
                    .find(|s| s.name.eq_ignore_ascii_case(expression))
            })?;
        Some(match symbol.kind {
            x8086_assembler::SymbolKind::DataByte => {
                ResolvedWatch::Live(x8086_debugger::watch::WatchTarget::Memory {
                    address: symbol.value as u32 + self.data_segment_base,
                    size: x8086_debugger::watch::WatchSize::Byte,
                })
            }
            x8086_assembler::SymbolKind::DataWord => {
                ResolvedWatch::Live(x8086_debugger::watch::WatchTarget::Memory {
                    address: symbol.value as u32 + self.data_segment_base,
                    size: x8086_debugger::watch::WatchSize::Word,
                })
            }
            x8086_assembler::SymbolKind::Label | x8086_assembler::SymbolKind::Constant => {
                ResolvedWatch::Constant(symbol.value as u16)
            }
        })
    }

    /// Adds a watch expression, rejecting it immediately if it doesn't
    /// currently resolve to anything (an unknown register/flag/variable
    /// name, or malformed `byte`/`word [addr]` syntax) rather than
    /// silently accepting text that will never show a value.
    pub fn add_watch(&mut self, expression: &str) -> Result<(), String> {
        let expression = expression.trim();
        if expression.is_empty() {
            return Err("watch expression must not be empty".to_string());
        }
        if self.resolve_watch(expression).is_none() {
            return Err(format!("unrecognized watch expression '{expression}'"));
        }
        self.watch_expressions.push(expression.to_string());
        Ok(())
    }

    /// Removes the watch at `index`, if any - out of range is a no-op
    /// rather than an error, since a UI list index racing a concurrent
    /// removal is a normal, harmless occurrence, not a bug.
    pub fn remove_watch(&mut self, index: usize) {
        if index < self.watch_expressions.len() {
            self.watch_expressions.remove(index);
        }
    }

    pub fn clear_watches(&mut self) {
        self.watch_expressions.clear();
    }

    /// Every current watch's live value, in add order. `value` is `None`
    /// when the expression no longer resolves - most commonly, a
    /// variable-name watch surviving past a reassemble that removed or
    /// renamed that symbol.
    pub fn watch_values(&self) -> Vec<WatchValue> {
        self.watch_expressions
            .iter()
            .map(|expression| {
                let value = self
                    .resolve_watch(expression)
                    .map(|resolved| match resolved {
                        ResolvedWatch::Live(target) => {
                            x8086_debugger::watch::evaluate(target, &self.registers, &self.memory)
                        }
                        ResolvedWatch::Constant(value) => value,
                    });
                WatchValue {
                    expression: expression.clone(),
                    value,
                }
            })
            .collect()
    }

    /// Disassembles `count` instructions forward from `start`, for the
    /// Disassembly panel. Forward-only and never backward: x86's
    /// variable-length encoding makes walking backward from an arbitrary
    /// address fundamentally ambiguous (you can't know where the
    /// "previous" instruction started without already knowing the
    /// boundaries), so every real disassembler has this same limitation -
    /// the panel always anchors its window at the current IP rather than
    /// trying to show instructions before it.
    ///
    /// A byte that doesn't decode is shown as a one-byte `DB` and
    /// disassembly resumes at the next byte, so one bad byte (stray data
    /// mixed into a code region, or landing mid-instruction) doesn't
    /// blank out the rest of the requested window.
    pub fn disassemble(&self, start: u32, count: u32) -> Vec<DisassembledLine> {
        let mut lines = Vec::with_capacity(count as usize);
        let mut address = start;
        for _ in 0..count {
            let mut window = [0u8; MAX_INSTRUCTION_LEN];
            for (offset, slot) in window.iter_mut().enumerate() {
                *slot = self.memory.read_u8(address.wrapping_add(offset as u32));
            }
            match decode_one(&window) {
                Ok((instruction, len)) => {
                    lines.push(DisassembledLine {
                        address,
                        text: format_instruction(&instruction, address),
                        byte_len: len as u32,
                    });
                    address = address.wrapping_add(len as u32);
                }
                Err(_) => {
                    let byte = self.memory.read_u8(address);
                    lines.push(DisassembledLine {
                        address,
                        text: format!("DB {byte:02X}h"),
                        byte_len: 1,
                    });
                    address = address.wrapping_add(1);
                }
            }
        }
        lines
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
    fn step_back_restores_a_memory_write() {
        let mut emulator = Emulator::new();
        // MOV AL, 0x42; MOV [0x0010], AL; HLT
        let program: &[u8] = &[0xB0, 0x42, 0xA2, 0x10, 0x00, 0xF4];
        emulator.load_program(program);
        assert_eq!(emulator.step().unwrap(), StepOutcome::Continued); // MOV AL, 0x42
        assert_eq!(emulator.memory.read_u8(0x10), 0x00);
        assert_eq!(emulator.step().unwrap(), StepOutcome::Continued); // MOV [0x10], AL
        assert_eq!(emulator.memory.read_u8(0x10), 0x42);

        assert!(emulator.step_back());
        assert_eq!(
            emulator.memory.read_u8(0x10),
            0x00,
            "step_back must undo the memory write the step made, not just registers"
        );
        assert_eq!(
            emulator.registers.ip, 2,
            "IP must roll back to before MOV [0x10], AL"
        );
    }

    fn full_memory_snapshot(emulator: &Emulator) -> Vec<u8> {
        (0..x8086_memory::MEMORY_SIZE as u32)
            .map(|address| emulator.memory.read_u8(address))
            .collect()
    }

    /// Validates the diff-based step-back storage optimization: `History`
    /// records only `(address, old_value)` pairs per step rather than a
    /// full memory snapshot, on the assumption that replaying those diffs
    /// in reverse is exactly equivalent to a naive "restore a full copy"
    /// approach. This runs a program that writes memory in every way the
    /// emulator currently supports - a direct byte write, a direct word
    /// write, and a stack push/pop (word writes at an SP-relative
    /// address) - and proves that property end to end rather than
    /// spot-checking one write in isolation.
    #[test]
    fn step_back_diff_based_restore_matches_a_naive_full_memory_snapshot() {
        let mut emulator = Emulator::new();
        let source = "\
MOV AX, 1234h
MOV [0010h], AX
MOV BL, 42h
MOV [0020h], BL
PUSH AX
MOV CX, 5678h
MOV [0030h], CX
POP DX
HLT
";
        let result = emulator.assemble_and_load(source);
        assert!(
            result.diagnostics.is_empty(),
            "diagnostics: {:?}",
            result.diagnostics
        );

        // Record a full reference snapshot immediately before each step
        // actually runs - the ground truth a "naive full-copy" step-back
        // implementation would have captured.
        let mut snapshots_before_each_step = Vec::new();
        loop {
            snapshots_before_each_step.push((emulator.registers, full_memory_snapshot(&emulator)));
            match emulator.step().unwrap() {
                StepOutcome::Halted => break,
                StepOutcome::Continued => {}
                StepOutcome::WaitingForKeyboard => panic!("unexpected keyboard wait"),
            }
        }
        assert!(
            snapshots_before_each_step.len() > 5,
            "the program should take a nontrivial number of steps to be a meaningful test"
        );

        // Step back through the entire history, and after each one,
        // confirm current state exactly matches the reference snapshot
        // recorded before that step originally ran.
        while let Some((expected_registers, expected_memory)) = snapshots_before_each_step.pop() {
            assert!(emulator.step_back());
            assert_eq!(emulator.registers, expected_registers);
            assert_eq!(full_memory_snapshot(&emulator), expected_memory);
        }
        assert!(!emulator.step_back(), "history must be fully exhausted");
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

    #[test]
    fn can_step_back_reflects_whether_history_has_anything_to_undo() {
        let mut emulator = Emulator::new();
        emulator.load_program(&[0xF4]); // HLT
        assert!(!emulator.can_step_back());
        emulator.step().unwrap();
        assert!(emulator.can_step_back());
        emulator.step_back();
        assert!(!emulator.can_step_back());
    }

    // MOV AX,1 (3); MOV BX,2 (3, at addr 3); MOV CX,3 (3, at addr 6); HLT (at addr 9).
    const THREE_MOVS_THEN_HLT: &[u8] =
        &[0xB8, 0x01, 0x00, 0xBB, 0x02, 0x00, 0xB9, 0x03, 0x00, 0xF4];

    #[test]
    fn run_executes_to_completion_when_no_breakpoints_are_set() {
        let mut emulator = Emulator::new();
        emulator.load_program(THREE_MOVS_THEN_HLT);
        assert_eq!(emulator.run(1_000), RunOutcome::Halted);
        assert_eq!(emulator.registers.ax, 1);
        assert_eq!(emulator.registers.bx, 2);
        assert_eq!(emulator.registers.cx, 3);
    }

    #[test]
    fn run_stops_at_a_breakpoint_without_executing_past_it() {
        let mut emulator = Emulator::new();
        emulator.load_program(THREE_MOVS_THEN_HLT);
        emulator.breakpoints.toggle(6); // address of MOV CX, 3
        assert_eq!(emulator.run(1_000), RunOutcome::BreakpointHit);
        assert_eq!(emulator.registers.ip, 6);
        assert_eq!(emulator.registers.ax, 1);
        assert_eq!(emulator.registers.bx, 2);
        assert_eq!(emulator.registers.cx, 0, "must stop before MOV CX,3 runs");
        assert!(!emulator.halted);
    }

    #[test]
    fn run_started_on_a_breakpoint_line_executes_it_and_continues() {
        let mut emulator = Emulator::new();
        emulator.load_program(THREE_MOVS_THEN_HLT);
        emulator.breakpoints.toggle(0); // the very first instruction
        assert_eq!(
            emulator.run(1_000),
            RunOutcome::Halted,
            "must not stop immediately on the line it started on"
        );
    }

    #[test]
    fn run_reports_step_limit_reached_on_a_runaway_loop() {
        let mut emulator = Emulator::new();
        emulator.load_program(&[0xEB, 0xFE]); // JMP $ (short jump to self)
        assert_eq!(emulator.run(50), RunOutcome::StepLimitReached);
    }

    #[test]
    fn run_to_cursor_stops_at_the_target_without_leaving_a_permanent_breakpoint() {
        let mut emulator = Emulator::new();
        emulator.load_program(THREE_MOVS_THEN_HLT);
        assert_eq!(emulator.run_to_cursor(9, 1_000), RunOutcome::BreakpointHit);
        assert_eq!(emulator.registers.ip, 9);
        assert!(!emulator.halted, "must stop before HLT executes");
        assert!(
            !emulator.breakpoints.is_set(9),
            "temporary breakpoint must not persist"
        );
    }

    #[test]
    fn run_to_cursor_leaves_a_pre_existing_real_breakpoint_at_the_target_set() {
        let mut emulator = Emulator::new();
        emulator.load_program(THREE_MOVS_THEN_HLT);
        emulator.breakpoints.toggle(9);
        assert_eq!(emulator.run_to_cursor(9, 1_000), RunOutcome::BreakpointHit);
        assert!(
            emulator.breakpoints.is_set(9),
            "a real breakpoint must survive run_to_cursor"
        );
    }

    #[test]
    fn run_to_cursor_still_stops_at_a_real_breakpoint_hit_before_the_target() {
        let mut emulator = Emulator::new();
        emulator.load_program(THREE_MOVS_THEN_HLT);
        emulator.breakpoints.toggle(3); // MOV BX, 2 - before the cursor target
        assert_eq!(emulator.run_to_cursor(9, 1_000), RunOutcome::BreakpointHit);
        assert_eq!(emulator.registers.ip, 3);
    }

    #[test]
    fn set_registers_overwrites_the_whole_register_file() {
        let mut emulator = Emulator::new();
        let mut regs = emulator.registers;
        regs.ax = 0x1234;
        regs.bx = 0x5678;
        emulator.set_registers(regs);
        assert_eq!(emulator.registers.ax, 0x1234);
        assert_eq!(emulator.registers.bx, 0x5678);
    }

    #[test]
    fn read_memory_returns_the_requested_bytes_in_order() {
        let mut emulator = Emulator::new();
        emulator.load_program(&[0x11, 0x22, 0x33, 0x44]);
        assert_eq!(emulator.read_memory(1, 3), vec![0x22, 0x33, 0x44]);
    }

    #[test]
    fn write_memory_byte_edits_memory_without_being_undoable_via_step_back() {
        let mut emulator = Emulator::new();
        emulator.load_program(&[0xF4]); // HLT
        emulator.step().unwrap(); // creates one history entry to pop
        emulator.write_memory_byte(0x10, 0x99);
        assert_eq!(emulator.memory.read_u8(0x10), 0x99);

        assert!(emulator.step_back());
        assert_eq!(
            emulator.memory.read_u8(0x10),
            0x99,
            "a manual live-edit is not a CPU step and must not be touched by step_back"
        );
    }

    #[test]
    fn variables_lists_db_and_dw_symbols_with_live_values_and_skips_labels_and_constants() {
        let mut emulator = Emulator::new();
        let result =
            emulator.assemble_and_load("SIZE EQU 1\nmsg DB 'H'\ncount DW 42\nstart: HLT\n");
        assert!(
            result.diagnostics.is_empty(),
            "diagnostics: {:?}",
            result.diagnostics
        );

        let vars = emulator.variables();
        assert_eq!(vars.len(), 2, "only DB/DW symbols should appear: {vars:?}");
        let msg = vars.iter().find(|v| v.name == "msg").expect("msg variable");
        assert!(!msg.is_word);
        assert_eq!(msg.value, b'H' as u16);
        let count = vars
            .iter()
            .find(|v| v.name == "count")
            .expect("count variable");
        assert!(count.is_word);
        assert_eq!(count.value, 42);
    }

    #[test]
    fn add_watch_accepts_registers_flags_raw_addresses_and_variable_names_and_rejects_junk() {
        let mut emulator = Emulator::new();
        let result = emulator.assemble_and_load("count DW 42\nHLT\n");
        assert!(result.diagnostics.is_empty());

        assert!(emulator.add_watch("AX").is_ok());
        assert!(
            emulator.add_watch(" zf ").is_ok(),
            "whitespace must be trimmed"
        );
        assert!(emulator.add_watch("word [0x0000]").is_ok());
        assert!(emulator.add_watch("count").is_ok());
        assert!(
            emulator.add_watch("COUNT").is_ok(),
            "variable lookup is case-insensitive"
        );
        assert_eq!(
            emulator.add_watch("nonexistent"),
            Err("unrecognized watch expression 'nonexistent'".to_string())
        );
        assert_eq!(
            emulator.add_watch("   "),
            Err("watch expression must not be empty".to_string())
        );
    }

    #[test]
    fn watch_values_are_evaluated_fresh_not_cached_at_add_time() {
        let mut emulator = Emulator::new();
        emulator.assemble_and_load("count DW 0\nHLT\n"); // count lives at address 0
        emulator.add_watch("count").unwrap();
        assert_eq!(emulator.watch_values()[0].value, Some(0));

        emulator.write_memory_byte(0, 0x2A);
        assert_eq!(
            emulator.watch_values()[0].value,
            Some(0x2A),
            "must reflect the live value, not the value from when the watch was added"
        );
    }

    #[test]
    fn watches_survive_reset_and_re_resolve_against_the_next_assemble() {
        let mut emulator = Emulator::new();
        emulator.assemble_and_load("count DW 7\nHLT\n");
        emulator.add_watch("count").unwrap();
        emulator.add_watch("AX").unwrap();
        assert_eq!(emulator.watch_values()[0].value, Some(7));

        // Reassemble with `count` moved to a different address by a
        // leading NOP, and a different value - the watch must follow it.
        emulator.assemble_and_load("NOP\ncount DW 99\nHLT\n");
        let values = emulator.watch_values();
        assert_eq!(values.len(), 2, "the watch list itself must survive reset");
        assert_eq!(
            values
                .iter()
                .find(|w| w.expression == "count")
                .unwrap()
                .value,
            Some(99)
        );
    }

    #[test]
    fn watch_value_is_none_once_its_symbol_no_longer_exists_after_a_reassemble() {
        let mut emulator = Emulator::new();
        emulator.assemble_and_load("count DW 7\nHLT\n");
        emulator.add_watch("count").unwrap();

        emulator.assemble_and_load("HLT\n"); // no more `count`
        assert_eq!(emulator.watch_values()[0].value, None);
    }

    #[test]
    fn remove_watch_and_clear_watches_shrink_the_list() {
        let mut emulator = Emulator::new();
        emulator.add_watch("AX").unwrap();
        emulator.add_watch("BX").unwrap();
        emulator.remove_watch(0);
        assert_eq!(emulator.watch_values().len(), 1);
        assert_eq!(emulator.watch_values()[0].expression, "BX");

        emulator.remove_watch(50); // out of range: a no-op, not a panic
        assert_eq!(emulator.watch_values().len(), 1);

        emulator.clear_watches();
        assert!(emulator.watch_values().is_empty());
    }

    #[test]
    fn disassemble_walks_forward_producing_readable_text_and_correct_addresses() {
        let mut emulator = Emulator::new();
        // MOV AX,1 (3 bytes, addr 0); MOV BX,2 (3 bytes, addr 3); HLT (addr 6).
        emulator.load_program(&[0xB8, 0x01, 0x00, 0xBB, 0x02, 0x00, 0xF4]);

        let lines = emulator.disassemble(0, 3);
        assert_eq!(lines.len(), 3);
        assert_eq!(
            lines[0],
            DisassembledLine {
                address: 0,
                text: "MOV AX, 1".to_string(),
                byte_len: 3
            }
        );
        assert_eq!(
            lines[1],
            DisassembledLine {
                address: 3,
                text: "MOV BX, 2".to_string(),
                byte_len: 3
            }
        );
        assert_eq!(
            lines[2],
            DisassembledLine {
                address: 6,
                text: "HLT".to_string(),
                byte_len: 1
            }
        );
    }

    #[test]
    fn disassemble_recovers_from_an_undecodable_byte_instead_of_stopping() {
        let mut emulator = Emulator::new();
        // 0x0F is reserved/undecoded; HLT follows right after it.
        emulator.load_program(&[0x0F, 0xF4]);

        let lines = emulator.disassemble(0, 2);
        assert_eq!(
            lines.len(),
            2,
            "one bad byte must not swallow the rest of the window"
        );
        assert_eq!(
            lines[0],
            DisassembledLine {
                address: 0,
                text: "DB 0Fh".to_string(),
                byte_len: 1
            }
        );
        assert_eq!(
            lines[1],
            DisassembledLine {
                address: 1,
                text: "HLT".to_string(),
                byte_len: 1
            }
        );
    }

    #[test]
    fn model_small_program_with_boilerplate_initializes_segments_and_runs_correctly() {
        let mut emulator = Emulator::new();
        let source = "\
.MODEL SMALL
.STACK 100h
.DATA
msg DB \"Hi$\"
.CODE
start:
MOV AX, @DATA
MOV DS, AX
LEA DX, msg
MOV AH, 9
INT 21h
MOV AH, 4Ch
INT 21h
END start
";
        let result = emulator.assemble_and_load(source);
        assert!(
            result.diagnostics.is_empty(),
            "diagnostics: {:?}",
            result.diagnostics
        );

        assert_ne!(
            emulator.registers.ds, 0,
            "DS must point at the data segment"
        );
        assert_ne!(
            emulator.registers.ss, 0,
            "SS must point at the stack segment"
        );
        assert_eq!(
            emulator.registers.sp, 0x100,
            "SP must start at the declared stack size"
        );

        loop {
            match emulator.step().unwrap() {
                // INT 21h AH=4Ch (terminate) reports Halted too, exactly
                // like a real HLT - it's the program's normal exit here.
                StepOutcome::Halted => break,
                StepOutcome::Continued => {}
                StepOutcome::WaitingForKeyboard => panic!("unexpected keyboard wait"),
            }
        }
        assert_eq!(emulator.console_output(), "Hi");
    }

    #[test]
    fn model_small_program_without_the_boilerplate_still_works() {
        // Real DOS requires MOV AX,@DATA/MOV DS,AX, but this emulator
        // auto-initializes DS to the data segment specifically so
        // programs that skip that boilerplate (common in simple/tutorial
        // emu8086 code) still resolve variables correctly.
        let mut emulator = Emulator::new();
        let source = "\
.MODEL SMALL
.DATA
msg DB \"Hi$\"
.CODE
start:
LEA DX, msg
MOV AH, 9
INT 21h
MOV AH, 4Ch
INT 21h
END start
";
        let result = emulator.assemble_and_load(source);
        assert!(
            result.diagnostics.is_empty(),
            "diagnostics: {:?}",
            result.diagnostics
        );

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
    fn flat_style_program_without_directives_still_starts_with_all_segments_zero() {
        // Backward-compatibility guard: a program that never uses
        // .MODEL/.STACK/.DATA/.CODE must see zero change in segment
        // register initialization.
        let mut emulator = Emulator::new();
        emulator.assemble_and_load("MOV AX, 1\nHLT\n");
        assert_eq!(emulator.registers.ds, 0);
        assert_eq!(emulator.registers.ss, 0);
        assert_eq!(emulator.registers.sp, 0);
    }

    #[test]
    fn variables_reads_the_correct_flat_address_once_a_real_data_segment_exists() {
        let mut emulator = Emulator::new();
        let result = emulator.assemble_and_load(".CODE\nMOV AX, 1\nHLT\n.DATA\nvalue DB 42\n");
        assert!(
            result.diagnostics.is_empty(),
            "diagnostics: {:?}",
            result.diagnostics
        );

        let data_base = result.data_segment_base.expect("a .DATA section was used");
        let vars = emulator.variables();
        let value = vars
            .iter()
            .find(|v| v.name == "value")
            .expect("value variable");
        assert_eq!(
            value.address, data_base,
            "must be the true flat address, not the segment-relative offset"
        );
        assert_eq!(value.value, 42);
    }
}
