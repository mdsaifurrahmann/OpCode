//! Simulated BIOS/DOS interrupt services, emu8086-style.
//!
//! Real emu8086 never runs under actual DOS: it intercepts these
//! interrupt numbers itself and simulates their effect (console output,
//! keyboard input) directly. `IoSink` is that simulation boundary - the
//! emulator facade supplies an implementation that forwards to Swift via
//! the FFI callback interface.
//!
//! This scaffold covers the console-output side (INT 21h AH=02h/09h,
//! INT 20h and INT 21h AH=4Ch termination). Blocking keyboard input
//! (INT 16h) needs the condvar-based suspend/resume mechanism owned by
//! `x8086-emulator`, so it lands with that facade in a later phase.

use x8086_cpu::Registers;
use x8086_memory::Memory;

pub trait IoSink {
    fn console_write(&mut self, text: &str);
    fn console_clear(&mut self);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InterruptOutcome {
    Continue,
    Terminate { exit_code: u8 },
}

/// Dispatch a simulated interrupt. `regs` may be mutated (some DOS
/// services report results in registers); `memory` is read-only here
/// since console-output services only ever read from it.
pub fn handle_interrupt(
    number: u8,
    regs: &mut Registers,
    memory: &Memory,
    io: &mut dyn IoSink,
) -> InterruptOutcome {
    match number {
        0x20 => InterruptOutcome::Terminate { exit_code: 0 },
        0x21 => handle_dos_service(regs, memory, io),
        _ => InterruptOutcome::Continue,
    }
}

fn handle_dos_service(
    regs: &mut Registers,
    memory: &Memory,
    io: &mut dyn IoSink,
) -> InterruptOutcome {
    let ah = (regs.ax >> 8) as u8;
    match ah {
        // AH=02h: print the character in DL.
        0x02 => {
            let dl = regs.dx as u8;
            io.console_write(&(dl as char).to_string());
            InterruptOutcome::Continue
        }
        // AH=09h: print the '$'-terminated string at DS:DX.
        0x09 => {
            let mut addr = Memory::resolve(regs.ds, regs.dx);
            let mut text = String::new();
            loop {
                let byte = memory.read_u8(addr);
                if byte as char == '$' {
                    break;
                }
                text.push(byte as char);
                addr = addr.wrapping_add(1);
            }
            io.console_write(&text);
            InterruptOutcome::Continue
        }
        // AH=4Ch: terminate with exit code in AL.
        0x4C => InterruptOutcome::Terminate {
            exit_code: regs.ax as u8,
        },
        _ => InterruptOutcome::Continue,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct RecordingSink {
        output: String,
        cleared: bool,
    }
    impl IoSink for RecordingSink {
        fn console_write(&mut self, text: &str) {
            self.output.push_str(text);
        }
        fn console_clear(&mut self) {
            self.cleared = true;
        }
    }

    #[test]
    fn int20h_terminates_with_zero_exit_code() {
        let mut regs = Registers::new();
        let memory = Memory::new();
        let mut sink = RecordingSink::default();
        let outcome = handle_interrupt(0x20, &mut regs, &memory, &mut sink);
        assert_eq!(outcome, InterruptOutcome::Terminate { exit_code: 0 });
    }

    #[test]
    fn int21h_ah4c_terminates_with_al_exit_code() {
        let mut regs = Registers::new();
        regs.ax = 0x4C07; // AH=4Ch, AL=07h
        let memory = Memory::new();
        let mut sink = RecordingSink::default();
        let outcome = handle_interrupt(0x21, &mut regs, &memory, &mut sink);
        assert_eq!(outcome, InterruptOutcome::Terminate { exit_code: 0x07 });
    }

    #[test]
    fn int21h_ah02_prints_character_in_dl() {
        let mut regs = Registers::new();
        regs.ax = 0x0200; // AH=02h
        regs.dx = b'X' as u16;
        let memory = Memory::new();
        let mut sink = RecordingSink::default();
        let outcome = handle_interrupt(0x21, &mut regs, &memory, &mut sink);
        assert_eq!(outcome, InterruptOutcome::Continue);
        assert_eq!(sink.output, "X");
    }

    #[test]
    fn int21h_ah09_prints_dollar_terminated_string() {
        let mut regs = Registers::new();
        regs.ax = 0x0900; // AH=09h
        regs.ds = 0x0000;
        regs.dx = 0x0100;
        let mut memory = Memory::new();
        for (offset, byte) in b"Hi!$".iter().enumerate() {
            memory.write_u8(0x0100 + offset as u32, *byte);
        }
        let mut sink = RecordingSink::default();
        let outcome = handle_interrupt(0x21, &mut regs, &memory, &mut sink);
        assert_eq!(outcome, InterruptOutcome::Continue);
        assert_eq!(sink.output, "Hi!");
    }

    #[test]
    fn unknown_interrupt_number_is_a_no_op() {
        let mut regs = Registers::new();
        let memory = Memory::new();
        let mut sink = RecordingSink::default();
        let outcome = handle_interrupt(0xFF, &mut regs, &memory, &mut sink);
        assert_eq!(outcome, InterruptOutcome::Continue);
        assert_eq!(sink.output, "");
    }
}
