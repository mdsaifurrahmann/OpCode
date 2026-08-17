//! Simulated BIOS/DOS interrupt services, emu8086-style.
//!
//! Real emu8086 never runs under actual DOS: it intercepts these
//! interrupt numbers itself and simulates their effect (console output,
//! keyboard input) directly. `IoSink` is that simulation boundary - the
//! emulator facade supplies an implementation that forwards to Swift via
//! the FFI callback interface.
//!
//! Covered so far: INT 21h AH=01h/02h/09h (console I/O) and AH=4Ch
//! (terminate), INT 10h AH=0Eh (BIOS teletype output), INT 16h AH=00h
//! (blocking keystroke read), and INT 20h (terminate). Keyboard reads
//! are the one place this crate can't just "complete" synchronously -
//! see `InterruptOutcome::NeedsKeyboardInput` below for how that's
//! handled without this crate needing to know anything about threads.

use x8086_cpu::Registers;
use x8086_memory::Memory;

pub trait IoSink {
    fn console_write(&mut self, text: &str);
    fn console_clear(&mut self);
    /// The next available keystroke as `(scancode, ascii)`, or `None` if
    /// the user hasn't provided one yet. A poll, not a block: it's the
    /// caller's job (see `InterruptOutcome::NeedsKeyboardInput`) to
    /// retry until this returns `Some`.
    fn read_key(&mut self) -> Option<(u8, u8)>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InterruptOutcome {
    Continue,
    Terminate {
        exit_code: u8,
    },
    /// A keyboard-read service was invoked but no key is available yet.
    /// Registers are left untouched - the caller must re-invoke this
    /// same interrupt vector later (once a key has been supplied)
    /// rather than treating this as having completed.
    NeedsKeyboardInput,
}

/// Dispatch a simulated interrupt. `regs` may be mutated (some services
/// report results in registers); `memory` is read-only here since only
/// console-output services read from it.
pub fn handle_interrupt(
    number: u8,
    regs: &mut Registers,
    memory: &Memory,
    io: &mut dyn IoSink,
) -> InterruptOutcome {
    match number {
        0x10 => handle_video_service(regs, io),
        0x16 => handle_keyboard_service(regs, io),
        0x20 => InterruptOutcome::Terminate { exit_code: 0 },
        0x21 => handle_dos_service(regs, memory, io),
        _ => InterruptOutcome::Continue,
    }
}

fn handle_video_service(regs: &mut Registers, io: &mut dyn IoSink) -> InterruptOutcome {
    let ah = (regs.ax >> 8) as u8;
    match ah {
        // AH=0Eh: teletype output - print AL, advance the cursor.
        0x0E => {
            let al = regs.ax as u8;
            io.console_write(&(al as char).to_string());
            InterruptOutcome::Continue
        }
        _ => InterruptOutcome::Continue,
    }
}

fn handle_keyboard_service(regs: &mut Registers, io: &mut dyn IoSink) -> InterruptOutcome {
    let ah = (regs.ax >> 8) as u8;
    match ah {
        // AH=00h: block until a key is pressed; AH=scancode, AL=ASCII.
        0x00 => match io.read_key() {
            Some((scancode, ascii)) => {
                regs.ax = ((scancode as u16) << 8) | ascii as u16;
                InterruptOutcome::Continue
            }
            None => InterruptOutcome::NeedsKeyboardInput,
        },
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
        // AH=01h: read a character with echo; AL=ASCII.
        0x01 => match io.read_key() {
            Some((_, ascii)) => {
                regs.ax = (regs.ax & 0xFF00) | ascii as u16;
                io.console_write(&(ascii as char).to_string());
                InterruptOutcome::Continue
            }
            None => InterruptOutcome::NeedsKeyboardInput,
        },
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
        keys: Vec<(u8, u8)>,
    }
    impl IoSink for RecordingSink {
        fn console_write(&mut self, text: &str) {
            self.output.push_str(text);
        }
        fn console_clear(&mut self) {
            self.cleared = true;
        }
        fn read_key(&mut self) -> Option<(u8, u8)> {
            if self.keys.is_empty() {
                None
            } else {
                Some(self.keys.remove(0))
            }
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
    fn int21h_ah01_reads_and_echoes_a_key() {
        let mut regs = Registers::new();
        regs.ax = 0x0100; // AH=01h
        let memory = Memory::new();
        let mut sink = RecordingSink {
            keys: vec![(0x1E, b'a')],
            ..Default::default()
        };
        let outcome = handle_interrupt(0x21, &mut regs, &memory, &mut sink);
        assert_eq!(outcome, InterruptOutcome::Continue);
        assert_eq!(regs.ax as u8, b'a');
        assert_eq!(sink.output, "a");
    }

    #[test]
    fn int21h_ah01_reports_needs_keyboard_input_when_no_key_is_available() {
        let mut regs = Registers::new();
        regs.ax = 0x0100;
        let memory = Memory::new();
        let mut sink = RecordingSink::default();
        let outcome = handle_interrupt(0x21, &mut regs, &memory, &mut sink);
        assert_eq!(outcome, InterruptOutcome::NeedsKeyboardInput);
        assert_eq!(
            sink.output, "",
            "must not echo anything until a key is actually available"
        );
    }

    #[test]
    fn int16h_ah00_returns_scancode_and_ascii_without_echoing() {
        let mut regs = Registers::new();
        regs.ax = 0x0000; // AH=00h
        let memory = Memory::new();
        let mut sink = RecordingSink {
            keys: vec![(0x1E, b'a')],
            ..Default::default()
        };
        let outcome = handle_interrupt(0x16, &mut regs, &memory, &mut sink);
        assert_eq!(outcome, InterruptOutcome::Continue);
        assert_eq!(regs.ax, 0x1E61); // AH=scancode, AL='a'=0x61
        assert_eq!(sink.output, "", "INT 16h/00h does not echo");
    }

    #[test]
    fn int16h_ah00_reports_needs_keyboard_input_when_no_key_is_available() {
        let mut regs = Registers::new();
        regs.ax = 0x0000;
        let memory = Memory::new();
        let mut sink = RecordingSink::default();
        let outcome = handle_interrupt(0x16, &mut regs, &memory, &mut sink);
        assert_eq!(outcome, InterruptOutcome::NeedsKeyboardInput);
    }

    #[test]
    fn int10h_ah0e_writes_teletype_output() {
        let mut regs = Registers::new();
        regs.ax = 0x0E41; // AH=0Eh, AL='A'
        let memory = Memory::new();
        let mut sink = RecordingSink::default();
        let outcome = handle_interrupt(0x10, &mut regs, &memory, &mut sink);
        assert_eq!(outcome, InterruptOutcome::Continue);
        assert_eq!(sink.output, "A");
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
