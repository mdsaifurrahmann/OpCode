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

#[derive(uniffi::Record, Clone, Debug, PartialEq, Eq)]
pub struct Diagnostic {
    pub line: u32,
    pub col: u32,
    pub is_error: bool,
    pub message: String,
}

#[derive(uniffi::Record, Clone, Debug, PartialEq, Eq)]
pub struct AssembleResult {
    pub machine_code_len: u32,
    pub entry_point: u32,
    pub diagnostics: Vec<Diagnostic>,
}

fn convert_diagnostic(d: x8086_assembler::Diagnostic) -> Diagnostic {
    Diagnostic {
        line: d.line,
        col: d.col,
        is_error: matches!(d.severity, x8086_assembler::Severity::Error),
        message: d.message,
    }
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
}
