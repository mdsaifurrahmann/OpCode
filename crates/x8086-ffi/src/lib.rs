//! Thin uniffi bridge to the emulator core. This crate contains no
//! business logic - only marshaling - so regenerating Swift bindings
//! never risks touching real behavior. FFI-facing types are deliberately
//! flatter than their Rust counterparts (e.g. `Diagnostic.is_error: bool`
//! instead of an enum, `AssembleResult` omitting the raw machine-code
//! bytes Swift doesn't need to see directly) since uniffi's `Record`/
//! `Enum` derive works best on simple, self-contained shapes.

use std::sync::Mutex;

uniffi::setup_scaffolding!();

/// A deliberately trivial round-trip used to prove the whole pipeline
/// (Rust -> staticlib -> XCFramework -> Swift bindings -> SwiftUI call)
/// before any real emulator surface existed.
#[uniffi::export]
pub fn ping() -> String {
    "pong".to_string()
}

#[derive(uniffi::Record, Clone, Debug, PartialEq, Eq)]
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

#[derive(uniffi::Enum, Clone, Debug, PartialEq, Eq)]
pub enum StepResult {
    Continued,
    Halted,
    WaitingForKeyboard,
    /// The byte at IP didn't decode to a known instruction. This is a
    /// real, user-facing outcome (a malformed/unsupported program), not
    /// an FFI failure, so it's a `StepResult` variant rather than a
    /// thrown error.
    DecodeError {
        message: String,
    },
}

/// The reason a `run`/`run_to_cursor` call stopped.
#[derive(uniffi::Enum, Clone, Debug, PartialEq, Eq)]
pub enum RunResult {
    Halted,
    WaitingForKeyboard,
    BreakpointHit,
    DecodeError,
    /// Hit the step ceiling without otherwise stopping - a runaway
    /// program, not a real stopping condition. The caller may call `run`
    /// again to keep going.
    StepLimitReached,
}

fn convert_run_outcome(outcome: x8086_emulator::RunOutcome) -> RunResult {
    match outcome {
        x8086_emulator::RunOutcome::Halted => RunResult::Halted,
        x8086_emulator::RunOutcome::WaitingForKeyboard => RunResult::WaitingForKeyboard,
        x8086_emulator::RunOutcome::BreakpointHit => RunResult::BreakpointHit,
        x8086_emulator::RunOutcome::DecodeError => RunResult::DecodeError,
        x8086_emulator::RunOutcome::StepLimitReached => RunResult::StepLimitReached,
    }
}

#[derive(uniffi::Record, Clone, Debug, PartialEq, Eq)]
pub struct Diagnostic {
    pub line: u32,
    pub col: u32,
    pub is_error: bool,
    pub message: String,
}

/// One entry of the assembler's source-line <-> address map. A flat
/// `Vec` rather than a `HashMap`, since uniffi's map support is
/// string-keyed only and Swift builds its own `Dictionary` from this in
/// the one place (the editor) that needs O(1) lookups.
#[derive(uniffi::Record, Clone, Debug, PartialEq, Eq)]
pub struct LineAddress {
    pub line: u32,
    pub address: u32,
}

#[derive(uniffi::Record, Clone, Debug, PartialEq, Eq)]
pub struct AssembleResult {
    pub machine_code_len: u32,
    pub entry_point: u32,
    pub diagnostics: Vec<Diagnostic>,
    pub line_to_address: Vec<LineAddress>,
}

fn convert_diagnostic(d: x8086_assembler::Diagnostic) -> Diagnostic {
    Diagnostic {
        line: d.line,
        col: d.col,
        is_error: matches!(d.severity, x8086_assembler::Severity::Error),
        message: d.message,
    }
}

/// One row of the Watch panel. `value` is `None` when the expression no
/// longer resolves (most commonly, a variable-name watch surviving past
/// a reassemble that removed that symbol).
#[derive(uniffi::Record, Clone, Debug, PartialEq, Eq)]
pub struct WatchValue {
    pub expression: String,
    pub value: Option<u16>,
}

fn convert_watch_value(w: x8086_emulator::WatchValue) -> WatchValue {
    WatchValue {
        expression: w.expression,
        value: w.value,
    }
}

/// One row of the Variables panel: a `DB`/`DW` symbol with its live
/// value read back from memory.
#[derive(uniffi::Record, Clone, Debug, PartialEq, Eq)]
pub struct VariableValue {
    pub name: String,
    pub address: u32,
    pub value: u16,
    pub is_word: bool,
}

fn convert_variable_value(v: x8086_emulator::VariableValue) -> VariableValue {
    VariableValue {
        name: v.name,
        address: v.address,
        value: v.value,
        is_word: v.is_word,
    }
}

/// One instruction from the Disassembly panel.
#[derive(uniffi::Record, Clone, Debug, PartialEq, Eq)]
pub struct DisassembledLine {
    pub address: u32,
    pub text: String,
    pub byte_len: u32,
}

fn convert_disassembled_line(l: x8086_emulator::DisassembledLine) -> DisassembledLine {
    DisassembledLine {
        address: l.address,
        text: l.text,
        byte_len: l.byte_len,
    }
}

#[derive(uniffi::Enum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum TokenKind {
    Identifier,
    Register,
    Number,
    StringLiteral,
    Comment,
    Punctuation,
    Newline,
}

#[derive(uniffi::Record, Clone, Debug, PartialEq, Eq)]
pub struct Token {
    pub kind: TokenKind,
    pub text: String,
    pub line: u32,
    pub col: u32,
    pub len: u32,
    /// UTF-8 byte offset from the start of the source. Assembly source
    /// is overwhelmingly ASCII, where a byte offset and a UTF-16 offset
    /// coincide - which is exactly what Swift's `NSRange`/`NSAttributedString`
    /// need, so the editor can use this directly without re-deriving it
    /// by walking the string itself.
    pub byte_offset: u32,
}

fn convert_token_kind(kind: x8086_assembler::TokenKind) -> TokenKind {
    match kind {
        x8086_assembler::TokenKind::Identifier => TokenKind::Identifier,
        x8086_assembler::TokenKind::Register => TokenKind::Register,
        x8086_assembler::TokenKind::Number => TokenKind::Number,
        x8086_assembler::TokenKind::StringLiteral => TokenKind::StringLiteral,
        x8086_assembler::TokenKind::Comment => TokenKind::Comment,
        x8086_assembler::TokenKind::Punctuation => TokenKind::Punctuation,
        x8086_assembler::TokenKind::Newline => TokenKind::Newline,
    }
}

/// Tokenizes `source` the exact same way the assembler itself does, so
/// the editor's syntax highlighting can never drift from what actually
/// assembles - reused directly rather than a separate Swift-side lexer.
#[uniffi::export]
pub fn tokenize_source(source: String) -> Vec<Token> {
    x8086_assembler::tokenize(&source)
        .into_iter()
        .map(|t| Token {
            kind: convert_token_kind(t.kind),
            text: t.text,
            line: t.span.line,
            col: t.span.col,
            len: t.span.len,
            byte_offset: t.span.byte_offset,
        })
        .collect()
}

fn convert_registers(regs: x8086_cpu::Registers) -> Registers {
    Registers {
        ax: regs.ax,
        bx: regs.bx,
        cx: regs.cx,
        dx: regs.dx,
        sp: regs.sp,
        bp: regs.bp,
        si: regs.si,
        di: regs.di,
        cs: regs.cs,
        ds: regs.ds,
        es: regs.es,
        ss: regs.ss,
        ip: regs.ip,
        flags: regs.flags,
    }
}

fn convert_registers_to_core(regs: Registers) -> x8086_cpu::Registers {
    x8086_cpu::Registers {
        ax: regs.ax,
        bx: regs.bx,
        cx: regs.cx,
        dx: regs.dx,
        sp: regs.sp,
        bp: regs.bp,
        si: regs.si,
        di: regs.di,
        cs: regs.cs,
        ds: regs.ds,
        es: regs.es,
        ss: regs.ss,
        ip: regs.ip,
        flags: regs.flags,
    }
}

#[derive(uniffi::Object)]
pub struct Emulator {
    inner: Mutex<x8086_emulator::Emulator>,
}

#[uniffi::export]
impl Emulator {
    #[uniffi::constructor]
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(x8086_emulator::Emulator::new()),
        }
    }

    pub fn reset(&self) {
        self.inner.lock().unwrap().reset();
    }

    pub fn assemble_and_load(&self, source: String) -> AssembleResult {
        let mut emulator = self.inner.lock().unwrap();
        let result = emulator.assemble_and_load(&source);
        AssembleResult {
            machine_code_len: result.machine_code.len() as u32,
            entry_point: result.entry_point,
            diagnostics: result
                .diagnostics
                .into_iter()
                .map(convert_diagnostic)
                .collect(),
            line_to_address: result
                .line_to_address
                .into_iter()
                .map(|(line, address)| LineAddress { line, address })
                .collect(),
        }
    }

    pub fn step(&self) -> StepResult {
        let mut emulator = self.inner.lock().unwrap();
        match emulator.step() {
            Ok(x8086_emulator::StepOutcome::Continued) => StepResult::Continued,
            Ok(x8086_emulator::StepOutcome::Halted) => StepResult::Halted,
            Ok(x8086_emulator::StepOutcome::WaitingForKeyboard) => StepResult::WaitingForKeyboard,
            Err(e) => StepResult::DecodeError {
                message: format!("{e:?}"),
            },
        }
    }

    pub fn feed_key(&self, scancode: u8, ascii: u8) {
        self.inner.lock().unwrap().feed_key(scancode, ascii);
    }

    pub fn console_output(&self) -> String {
        self.inner.lock().unwrap().console_output().to_string()
    }

    pub fn registers(&self) -> Registers {
        convert_registers(self.inner.lock().unwrap().registers)
    }

    pub fn halted(&self) -> bool {
        self.inner.lock().unwrap().halted
    }

    /// Overwrites the whole register file - the Registers panel's
    /// live-edit path.
    pub fn set_registers(&self, registers: Registers) {
        self.inner
            .lock()
            .unwrap()
            .set_registers(convert_registers_to_core(registers));
    }

    /// Undo the most recent step. Returns false (a no-op, not an error)
    /// if there is no history left to undo.
    pub fn step_back(&self) -> bool {
        self.inner.lock().unwrap().step_back()
    }

    pub fn can_step_back(&self) -> bool {
        self.inner.lock().unwrap().can_step_back()
    }

    /// Steps repeatedly until the program halts, blocks on keyboard
    /// input, hits a set breakpoint, hits a decode error, or `max_steps`
    /// is reached.
    pub fn run(&self, max_steps: u32) -> RunResult {
        convert_run_outcome(self.inner.lock().unwrap().run(max_steps))
    }

    /// Runs until `address` is reached, a real breakpoint is hit first,
    /// or the program halts/blocks/errors - the Run-to-cursor command.
    pub fn run_to_cursor(&self, address: u32, max_steps: u32) -> RunResult {
        convert_run_outcome(self.inner.lock().unwrap().run_to_cursor(address, max_steps))
    }

    /// Toggles a breakpoint at `address`, returning whether it is now
    /// set (was previously absent).
    pub fn toggle_breakpoint(&self, address: u32) -> bool {
        self.inner.lock().unwrap().breakpoints.toggle(address)
    }

    pub fn is_breakpoint_set(&self, address: u32) -> bool {
        self.inner.lock().unwrap().breakpoints.is_set(address)
    }

    pub fn clear_breakpoints(&self) {
        self.inner.lock().unwrap().breakpoints.clear_all();
    }

    /// Reads up to `len` bytes starting at `address` - the Memory/Stack
    /// panels' on-demand, visible-range-only read path.
    pub fn read_memory(&self, address: u32, len: u32) -> Vec<u8> {
        self.inner.lock().unwrap().read_memory(address, len)
    }

    /// Live-edits one memory cell - the Memory panel's write path.
    pub fn write_memory_byte(&self, address: u32, value: u8) {
        self.inner.lock().unwrap().write_memory_byte(address, value);
    }

    /// Every `DB`/`DW` symbol from the last assemble, with its current
    /// value - the Variables panel's data source.
    pub fn variables(&self) -> Vec<VariableValue> {
        self.inner
            .lock()
            .unwrap()
            .variables()
            .into_iter()
            .map(convert_variable_value)
            .collect()
    }

    /// Adds a watch expression (a register/flag name, `byte`/`word
    /// [addr]`, or a variable name from the last assemble). Returns an
    /// error message on failure rather than throwing, matching this
    /// crate's existing convention of surfacing user-facing failures as
    /// data (see `StepResult::DecodeError`) rather than exceptions.
    pub fn add_watch(&self, expression: String) -> Option<String> {
        self.inner.lock().unwrap().add_watch(&expression).err()
    }

    pub fn remove_watch(&self, index: u32) {
        self.inner.lock().unwrap().remove_watch(index as usize);
    }

    pub fn clear_watches(&self) {
        self.inner.lock().unwrap().clear_watches();
    }

    pub fn watch_values(&self) -> Vec<WatchValue> {
        self.inner
            .lock()
            .unwrap()
            .watch_values()
            .into_iter()
            .map(convert_watch_value)
            .collect()
    }

    /// Disassembles `count` instructions forward from `address` - the
    /// Disassembly panel's data source.
    pub fn disassemble(&self, address: u32, count: u32) -> Vec<DisassembledLine> {
        self.inner
            .lock()
            .unwrap()
            .disassemble(address, count)
            .into_iter()
            .map(convert_disassembled_line)
            .collect()
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
    fn ping_returns_pong() {
        assert_eq!(ping(), "pong");
    }

    #[test]
    fn emulator_runs_a_hello_world_program_through_the_ffi_surface() {
        let emulator = Emulator::new();
        let result = emulator.assemble_and_load(
            // LEA, not MOV: a bare data-symbol operand dereferences the
            // variable (MASM/emu8086 convention), so loading its
            // *address* needs LEA - matching real emu8086 sample code's
            // `lea dx, message` idiom.
            "LEA DX, msg\nMOV AH, 9\nINT 21h\nHLT\nmsg DB \"Hi$\"\n".to_string(),
        );
        assert!(
            result.diagnostics.is_empty(),
            "diagnostics: {:?}",
            result.diagnostics
        );

        loop {
            match emulator.step() {
                StepResult::Halted => break,
                StepResult::Continued => {}
                other => panic!("unexpected step result: {other:?}"),
            }
        }
        assert_eq!(emulator.console_output(), "Hi");
    }

    #[test]
    fn emulator_reports_waiting_for_keyboard_and_resumes_after_feed_key() {
        let emulator = Emulator::new();
        let result = emulator.assemble_and_load("MOV AH, 0\nINT 16h\nHLT\n".to_string());
        assert!(
            result.diagnostics.is_empty(),
            "diagnostics: {:?}",
            result.diagnostics
        );

        assert_eq!(emulator.step(), StepResult::Continued); // MOV AH, 0
        assert_eq!(emulator.step(), StepResult::WaitingForKeyboard);
        emulator.feed_key(0x1E, b'a');
        assert_eq!(emulator.step(), StepResult::Continued);
        assert_eq!(emulator.registers().ax, 0x1E61);
    }

    #[test]
    fn tokenize_source_classifies_registers_and_numbers() {
        let tokens = tokenize_source("MOV AX, 5".to_string());
        let kinds: Vec<TokenKind> = tokens.iter().map(|t| t.kind).collect();
        assert_eq!(
            kinds,
            vec![
                TokenKind::Identifier,
                TokenKind::Register,
                TokenKind::Punctuation,
                TokenKind::Number
            ]
        );
        assert_eq!(tokens[1].text, "AX");
    }

    #[test]
    fn assemble_and_load_exposes_the_line_to_address_map() {
        let emulator = Emulator::new();
        let result = emulator.assemble_and_load("MOV AX, 5\nHLT\n".to_string());
        assert!(
            result.diagnostics.is_empty(),
            "diagnostics: {:?}",
            result.diagnostics
        );
        assert!(result.line_to_address.contains(&LineAddress {
            line: 1,
            address: 0
        }));
        assert!(result.line_to_address.contains(&LineAddress {
            line: 2,
            address: 3
        })); // MOV AX,5 is 3 bytes
    }

    #[test]
    fn run_executes_to_completion_and_step_back_undoes_the_last_step() {
        let emulator = Emulator::new();
        let result = emulator.assemble_and_load("MOV AX, 1\nMOV BX, 2\nHLT\n".to_string());
        assert!(
            result.diagnostics.is_empty(),
            "diagnostics: {:?}",
            result.diagnostics
        );

        assert_eq!(emulator.run(1_000), RunResult::Halted);
        assert_eq!(emulator.registers().bx, 2);

        assert!(emulator.can_step_back());
        assert!(emulator.step_back());
        assert!(!emulator.halted(), "stepping back over HLT must un-halt");
    }

    #[test]
    fn run_stops_at_a_breakpoint_and_run_to_cursor_reaches_the_target() {
        let emulator = Emulator::new();
        let result = emulator.assemble_and_load("MOV AX, 1\nMOV BX, 2\nHLT\n".to_string());
        assert!(result.diagnostics.is_empty());
        // MOV BX, 2 sits at address 3 (MOV AX,1 is 3 bytes).
        assert!(emulator.toggle_breakpoint(3));
        assert!(emulator.is_breakpoint_set(3));

        assert_eq!(emulator.run(1_000), RunResult::BreakpointHit);
        assert_eq!(emulator.registers().ip, 3);
        assert_eq!(emulator.registers().bx, 0, "must stop before MOV BX,2 runs");

        emulator.clear_breakpoints();
        assert!(!emulator.is_breakpoint_set(3));
        assert_eq!(emulator.run_to_cursor(6, 1_000), RunResult::BreakpointHit);
        assert_eq!(emulator.registers().ip, 6);
    }

    #[test]
    fn set_registers_and_memory_read_write_round_trip_through_the_ffi_surface() {
        let emulator = Emulator::new();
        let mut regs = emulator.registers();
        regs.ax = 0xBEEF;
        emulator.set_registers(regs);
        assert_eq!(emulator.registers().ax, 0xBEEF);

        emulator.write_memory_byte(0x10, 0x42);
        assert_eq!(emulator.read_memory(0x10, 2), vec![0x42, 0x00]);
    }

    #[test]
    fn watches_and_variables_are_exposed_over_ffi() {
        let emulator = Emulator::new();
        let result = emulator.assemble_and_load("count DW 42\nHLT\n".to_string());
        assert!(result.diagnostics.is_empty());

        let vars = emulator.variables();
        assert_eq!(
            vars,
            vec![VariableValue {
                name: "count".to_string(),
                address: 0,
                value: 42,
                is_word: true
            }]
        );

        assert_eq!(emulator.add_watch("count".to_string()), None);
        assert_eq!(
            emulator.add_watch("nonexistent".to_string()),
            Some("unrecognized watch expression 'nonexistent'".to_string())
        );
        assert_eq!(
            emulator.watch_values(),
            vec![WatchValue {
                expression: "count".to_string(),
                value: Some(42)
            }]
        );

        emulator.remove_watch(0);
        assert!(emulator.watch_values().is_empty());
    }

    #[test]
    fn disassemble_is_exposed_over_ffi() {
        let emulator = Emulator::new();
        let result = emulator.assemble_and_load("MOV AX, 1\nHLT\n".to_string());
        assert!(result.diagnostics.is_empty());

        let lines = emulator.disassemble(0, 2);
        assert_eq!(
            lines,
            vec![
                DisassembledLine {
                    address: 0,
                    text: "MOV AX, 1".to_string(),
                    byte_len: 3
                },
                DisassembledLine {
                    address: 3,
                    text: "HLT".to_string(),
                    byte_len: 1
                },
            ]
        );
    }
}
