//! Simulated BIOS/DOS interrupt services, emu8086-style.
//!
//! Real emu8086 never runs under actual DOS: it intercepts these
//! interrupt numbers itself and simulates their effect (console output,
//! keyboard input) directly. `IoSink` is that simulation boundary - the
//! emulator facade supplies an implementation that forwards to Swift via
//! the FFI callback interface.
//!
//! Covered so far: INT 21h AH=01h/02h/09h (console I/O), AH=0Ah
//! (buffered line input), and AH=4Ch (terminate), INT 10h AH=0Eh (BIOS
//! teletype output) and AH=13h (write string), INT 16h AH=00h (blocking
//! keystroke read), and INT 20h (terminate). Keyboard reads are the one
//! place this crate
//! can't just "complete" synchronously - see
//! `InterruptOutcome::NeedsKeyboardInput` below for how that's handled
//! without this crate needing to know anything about threads.

mod cp437;

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
/// report results in registers); `memory` is mutable since AH=0Ah writes
/// the characters it reads back into the caller's buffer. `first_attempt`
/// is `true` only on the initial dispatch of this specific interrupt
/// instruction, `false` on every retry after a `NeedsKeyboardInput` -
/// AH=0Ah is the only service that consults it, to know when to reset its
/// in-buffer character count rather than continuing to accumulate into
/// whatever was already typed.
pub fn handle_interrupt(
    number: u8,
    regs: &mut Registers,
    memory: &mut Memory,
    io: &mut dyn IoSink,
    first_attempt: bool,
) -> InterruptOutcome {
    match number {
        0x10 => handle_video_service(regs, memory, io),
        0x16 => handle_keyboard_service(regs, io),
        0x20 => InterruptOutcome::Terminate { exit_code: 0 },
        0x21 => handle_dos_service(regs, memory, io, first_attempt),
        _ => InterruptOutcome::Continue,
    }
}

fn handle_video_service(
    regs: &mut Registers,
    memory: &Memory,
    io: &mut dyn IoSink,
) -> InterruptOutcome {
    let ah = (regs.ax >> 8) as u8;
    match ah {
        // AH=0Eh: teletype output - print AL, advance the cursor.
        0x0E => {
            let al = regs.ax as u8;
            io.console_write(&cp437::to_char(al).to_string());
            InterruptOutcome::Continue
        }
        // AH=13h: write a string of CX characters from ES:BP.
        0x13 => {
            write_string(regs, memory, io);
            InterruptOutcome::Continue
        }
        _ => InterruptOutcome::Continue,
    }
}

/// INT 10h AH=13h. AL selects the layout: even modes (0/2) leave the
/// cursor where it was and odd ones (1/3) advance it, while modes 2 and 3
/// interleave an attribute byte after each character instead of taking a
/// single attribute from BL.
///
/// Only the characters are reproduced. The console is an append-only text
/// transcript with no cursor addressing or color (see `ConsoleSink` in
/// x8086-emulator), so the DH/DL start position, the BH page, and every
/// attribute byte have nowhere to go - consistent with how AH=0Eh and the
/// DOS output services already behave here.
fn write_string(regs: &mut Registers, memory: &Memory, io: &mut dyn IoSink) {
    let attributes_interleaved = matches!(regs.ax as u8, 2 | 3);
    let stride = if attributes_interleaved { 2 } else { 1 };
    let base = Memory::resolve(regs.es, regs.bp);

    let mut text = String::new();
    for index in 0..regs.cx {
        let offset = (index as u32).wrapping_mul(stride);
        text.push(cp437::to_char(memory.read_u8(base.wrapping_add(offset))));
    }
    io.console_write(&text);
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
    memory: &mut Memory,
    io: &mut dyn IoSink,
    first_attempt: bool,
) -> InterruptOutcome {
    let ah = (regs.ax >> 8) as u8;
    match ah {
        // AH=01h: read a character with echo; AL=ASCII.
        0x01 => match io.read_key() {
            Some((_, ascii)) => {
                regs.ax = (regs.ax & 0xFF00) | ascii as u16;
                io.console_write(&cp437::to_char(ascii).to_string());
                InterruptOutcome::Continue
            }
            None => InterruptOutcome::NeedsKeyboardInput,
        },
        // AH=02h: print the character in DL.
        0x02 => {
            let dl = regs.dx as u8;
            io.console_write(&cp437::to_char(dl).to_string());
            InterruptOutcome::Continue
        }
        // AH=09h: print the '$'-terminated string at DS:DX.
        0x09 => {
            let mut addr = Memory::resolve(regs.ds, regs.dx);
            let mut text = String::new();
            loop {
                let byte = memory.read_u8(addr);
                if byte == b'$' {
                    break;
                }
                text.push(cp437::to_char(byte));
                addr = addr.wrapping_add(1);
            }
            io.console_write(&text);
            InterruptOutcome::Continue
        }
        // AH=0Ah: buffered ("cooked") line input into the classic DOS
        // structure at DS:DX - byte 0 is the caller-supplied max length,
        // byte 1 becomes the actual character count, and the characters
        // themselves (no terminating CR) follow starting at byte 2.
        0x0A => handle_buffered_input(regs, memory, io, first_attempt),
        // AH=4Ch: terminate with exit code in AL.
        0x4C => InterruptOutcome::Terminate {
            exit_code: regs.ax as u8,
        },
        _ => InterruptOutcome::Continue,
    }
}

/// AH=0Ah reads a whole line, one keystroke per poll, echoing each
/// character as it arrives and stopping on Enter - built entirely on top
/// of the same single-keystroke `read_key` poll AH=00h/01h already use,
/// retried via `NeedsKeyboardInput` once per character rather than once
/// per line. The character count lives in the buffer itself (byte 1), so
/// no extra state is needed to resume mid-line on the next poll; it's
/// reset to 0 only on `first_attempt`, since every later poll for the
/// same line is a retry of the very same still-pending interrupt.
fn handle_buffered_input(
    regs: &mut Registers,
    memory: &mut Memory,
    io: &mut dyn IoSink,
    first_attempt: bool,
) -> InterruptOutcome {
    let buffer_addr = Memory::resolve(regs.ds, regs.dx);
    let max_len = memory.read_u8(buffer_addr) as usize;
    if first_attempt {
        memory.write_u8(buffer_addr.wrapping_add(1), 0);
    }
    let count = memory.read_u8(buffer_addr.wrapping_add(1)) as usize;

    let Some((_, ascii)) = io.read_key() else {
        return InterruptOutcome::NeedsKeyboardInput;
    };

    match ascii {
        b'\r' => {
            io.console_write("\r\n");
            InterruptOutcome::Continue
        }
        // Backspace: only the stored count moves back - the console is a
        // plain append-only transcript (see `ConsoleSink`), not a real
        // terminal with cursor control, so there's no way to visually
        // erase the previous character without risking exactly the kind
        // of garbled output a past bug already had to fix here.
        0x08 if count > 0 => {
            memory.write_u8(buffer_addr.wrapping_add(1), (count - 1) as u8);
            InterruptOutcome::NeedsKeyboardInput
        }
        0x08 => InterruptOutcome::NeedsKeyboardInput,
        _ if ascii != 0 && count < max_len => {
            memory.write_u8(buffer_addr.wrapping_add(2 + count as u32), ascii);
            memory.write_u8(buffer_addr.wrapping_add(1), (count + 1) as u8);
            io.console_write(&cp437::to_char(ascii).to_string());
            InterruptOutcome::NeedsKeyboardInput
        }
        // Either a non-ASCII key (scancode with no ASCII meaning) or the
        // buffer is already full - real DOS ignores further characters
        // once the max is reached rather than erroring.
        _ => InterruptOutcome::NeedsKeyboardInput,
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
        let mut memory = Memory::new();
        let mut sink = RecordingSink::default();
        let outcome = handle_interrupt(0x20, &mut regs, &mut memory, &mut sink, true);
        assert_eq!(outcome, InterruptOutcome::Terminate { exit_code: 0 });
    }

    #[test]
    fn int21h_ah4c_terminates_with_al_exit_code() {
        let mut regs = Registers::new();
        regs.ax = 0x4C07; // AH=4Ch, AL=07h
        let mut memory = Memory::new();
        let mut sink = RecordingSink::default();
        let outcome = handle_interrupt(0x21, &mut regs, &mut memory, &mut sink, true);
        assert_eq!(outcome, InterruptOutcome::Terminate { exit_code: 0x07 });
    }

    #[test]
    fn int21h_ah02_prints_character_in_dl() {
        let mut regs = Registers::new();
        regs.ax = 0x0200; // AH=02h
        regs.dx = b'X' as u16;
        let mut memory = Memory::new();
        let mut sink = RecordingSink::default();
        let outcome = handle_interrupt(0x21, &mut regs, &mut memory, &mut sink, true);
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
        let outcome = handle_interrupt(0x21, &mut regs, &mut memory, &mut sink, true);
        assert_eq!(outcome, InterruptOutcome::Continue);
        assert_eq!(sink.output, "Hi!");
    }

    #[test]
    fn int21h_ah01_reads_and_echoes_a_key() {
        let mut regs = Registers::new();
        regs.ax = 0x0100; // AH=01h
        let mut memory = Memory::new();
        let mut sink = RecordingSink {
            keys: vec![(0x1E, b'a')],
            ..Default::default()
        };
        let outcome = handle_interrupt(0x21, &mut regs, &mut memory, &mut sink, true);
        assert_eq!(outcome, InterruptOutcome::Continue);
        assert_eq!(regs.ax as u8, b'a');
        assert_eq!(sink.output, "a");
    }

    #[test]
    fn int21h_ah01_reports_needs_keyboard_input_when_no_key_is_available() {
        let mut regs = Registers::new();
        regs.ax = 0x0100;
        let mut memory = Memory::new();
        let mut sink = RecordingSink::default();
        let outcome = handle_interrupt(0x21, &mut regs, &mut memory, &mut sink, true);
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
        let mut memory = Memory::new();
        let mut sink = RecordingSink {
            keys: vec![(0x1E, b'a')],
            ..Default::default()
        };
        let outcome = handle_interrupt(0x16, &mut regs, &mut memory, &mut sink, true);
        assert_eq!(outcome, InterruptOutcome::Continue);
        assert_eq!(regs.ax, 0x1E61); // AH=scancode, AL='a'=0x61
        assert_eq!(sink.output, "", "INT 16h/00h does not echo");
    }

    #[test]
    fn int16h_ah00_reports_needs_keyboard_input_when_no_key_is_available() {
        let mut regs = Registers::new();
        regs.ax = 0x0000;
        let mut memory = Memory::new();
        let mut sink = RecordingSink::default();
        let outcome = handle_interrupt(0x16, &mut regs, &mut memory, &mut sink, true);
        assert_eq!(outcome, InterruptOutcome::NeedsKeyboardInput);
    }

    #[test]
    fn int10h_ah0e_writes_teletype_output() {
        let mut regs = Registers::new();
        regs.ax = 0x0E41; // AH=0Eh, AL='A'
        let mut memory = Memory::new();
        let mut sink = RecordingSink::default();
        let outcome = handle_interrupt(0x10, &mut regs, &mut memory, &mut sink, true);
        assert_eq!(outcome, InterruptOutcome::Continue);
        assert_eq!(sink.output, "A");
    }

    /// Drives AH=0Ah to completion the way `Emulator::step` really would:
    /// one `handle_interrupt` call per available key, `first_attempt` set
    /// only on the very first call, looping on `NeedsKeyboardInput` until
    /// the line is done (or `max_polls` is hit, so a bug that never
    /// completes fails the test instead of hanging it).
    fn drive_buffered_input(
        regs: &mut Registers,
        memory: &mut Memory,
        sink: &mut RecordingSink,
        max_polls: usize,
    ) -> InterruptOutcome {
        let mut first_attempt = true;
        for _ in 0..max_polls {
            let outcome = handle_interrupt(0x21, regs, memory, sink, first_attempt);
            first_attempt = false;
            if outcome != InterruptOutcome::NeedsKeyboardInput {
                return outcome;
            }
        }
        panic!("buffered input never completed within {max_polls} polls");
    }

    /// `buff DB 7,0,7 DUP('$')` - byte 0 is the max length, byte 1 is the
    /// (initially garbage) count, and the rest is scratch space for typed
    /// characters. The exact structure real emu8086/MASM programs use.
    fn make_input_buffer(memory: &mut Memory, addr: u32, max_len: u8, garbage_count: u8) {
        memory.write_u8(addr, max_len);
        memory.write_u8(addr.wrapping_add(1), garbage_count);
    }

    #[test]
    fn int21h_ah0a_reads_a_line_echoing_each_character_and_stopping_at_enter() {
        let mut regs = Registers::new();
        regs.ax = 0x0A00; // AH=0Ah
        regs.ds = 0x0000;
        regs.dx = 0x0200;
        let mut memory = Memory::new();
        make_input_buffer(&mut memory, 0x0200, 10, 0);
        let mut sink = RecordingSink {
            keys: vec![(0, b'4'), (0, b'2'), (0, b'\r')],
            ..Default::default()
        };

        let outcome = drive_buffered_input(&mut regs, &mut memory, &mut sink, 10);

        assert_eq!(outcome, InterruptOutcome::Continue);
        assert_eq!(
            sink.output, "42\r\n",
            "each typed digit echoes, Enter ends the line"
        );
        assert_eq!(
            memory.read_u8(0x0201),
            2,
            "byte 1 becomes the character count"
        );
        assert_eq!(memory.read_u8(0x0202), b'4');
        assert_eq!(memory.read_u8(0x0203), b'2');
    }

    #[test]
    fn int21h_ah0a_reports_needs_keyboard_input_when_no_key_is_available_yet() {
        let mut regs = Registers::new();
        regs.ax = 0x0A00;
        regs.ds = 0x0000;
        regs.dx = 0x0200;
        let mut memory = Memory::new();
        make_input_buffer(&mut memory, 0x0200, 10, 0);
        let mut sink = RecordingSink::default();

        let outcome = handle_interrupt(0x21, &mut regs, &mut memory, &mut sink, true);

        assert_eq!(outcome, InterruptOutcome::NeedsKeyboardInput);
        assert_eq!(sink.output, "");
    }

    #[test]
    fn int21h_ah0a_stops_accepting_characters_once_max_length_is_reached() {
        let mut regs = Registers::new();
        regs.ax = 0x0A00;
        regs.ds = 0x0000;
        regs.dx = 0x0200;
        let mut memory = Memory::new();
        make_input_buffer(&mut memory, 0x0200, 2, 0);
        let mut sink = RecordingSink {
            // Three digits typed against a 2-character buffer, then Enter.
            keys: vec![(0, b'1'), (0, b'2'), (0, b'3'), (0, b'\r')],
            ..Default::default()
        };

        let outcome = drive_buffered_input(&mut regs, &mut memory, &mut sink, 10);

        assert_eq!(outcome, InterruptOutcome::Continue);
        assert_eq!(
            memory.read_u8(0x0201),
            2,
            "the third digit is silently dropped"
        );
        assert_eq!(memory.read_u8(0x0202), b'1');
        assert_eq!(memory.read_u8(0x0203), b'2');
        assert_eq!(
            sink.output, "12\r\n",
            "the dropped digit isn't echoed either"
        );
    }

    #[test]
    fn int21h_ah0a_backspace_decrements_the_count_without_going_negative() {
        let mut regs = Registers::new();
        regs.ax = 0x0A00;
        regs.ds = 0x0000;
        regs.dx = 0x0200;
        let mut memory = Memory::new();
        make_input_buffer(&mut memory, 0x0200, 10, 0);
        let mut sink = RecordingSink {
            // 'a', backspace (erases it), backspace again (nothing left
            // to erase - must not underflow), 'b', Enter.
            keys: vec![(0, b'a'), (0, 0x08), (0, 0x08), (0, b'b'), (0, b'\r')],
            ..Default::default()
        };

        let outcome = drive_buffered_input(&mut regs, &mut memory, &mut sink, 10);

        assert_eq!(outcome, InterruptOutcome::Continue);
        assert_eq!(memory.read_u8(0x0201), 1, "only 'b' should remain");
        assert_eq!(memory.read_u8(0x0202), b'b');
    }

    #[test]
    fn int21h_ah0a_resets_a_stale_count_left_over_from_a_previous_read() {
        // A program that calls AH=0Ah twice into the same buffer leaves
        // the first read's count sitting in byte 1 - first_attempt must
        // reset it, or the second read would start appending after
        // characters that were never actually typed this time.
        let mut regs = Registers::new();
        regs.ax = 0x0A00;
        regs.ds = 0x0000;
        regs.dx = 0x0200;
        let mut memory = Memory::new();
        make_input_buffer(&mut memory, 0x0200, 10, 5); // stale count of 5
        let mut sink = RecordingSink {
            keys: vec![(0, b'9'), (0, b'\r')],
            ..Default::default()
        };

        let outcome = drive_buffered_input(&mut regs, &mut memory, &mut sink, 10);

        assert_eq!(outcome, InterruptOutcome::Continue);
        assert_eq!(
            memory.read_u8(0x0201),
            1,
            "count reset before counting the new input"
        );
        assert_eq!(memory.read_u8(0x0202), b'9');
    }

    #[test]
    fn int10h_ah13_writes_a_counted_string_from_es_bp() {
        let mut regs = Registers::new();
        regs.ax = 0x1301; // AH=13h, AL=01 (chars only, advance cursor)
        regs.es = 0x0000;
        regs.bp = 0x0300;
        regs.cx = 5;
        let mut memory = Memory::new();
        for (offset, byte) in b"Hello".iter().enumerate() {
            memory.write_u8(0x0300 + offset as u32, *byte);
        }
        let mut sink = RecordingSink::default();

        let outcome = handle_interrupt(0x10, &mut regs, &mut memory, &mut sink, true);

        assert_eq!(outcome, InterruptOutcome::Continue);
        assert_eq!(sink.output, "Hello");
    }

    #[test]
    fn int10h_ah13_skips_interleaved_attribute_bytes_in_modes_2_and_3() {
        let mut regs = Registers::new();
        regs.ax = 0x1302; // AL=02: each character is followed by an attribute
        regs.es = 0x0000;
        regs.bp = 0x0300;
        regs.cx = 3;
        let mut memory = Memory::new();
        for (offset, byte) in b"H\x07i\x07!\x07".iter().enumerate() {
            memory.write_u8(0x0300 + offset as u32, *byte);
        }
        let mut sink = RecordingSink::default();

        handle_interrupt(0x10, &mut regs, &mut memory, &mut sink, true);

        assert_eq!(
            sink.output, "Hi!",
            "CX counts characters, not bytes - the attribute bytes are stepped over"
        );
    }

    #[test]
    fn int10h_ah13_with_a_zero_length_writes_nothing() {
        let mut regs = Registers::new();
        regs.ax = 0x1301;
        regs.cx = 0;
        let mut memory = Memory::new();
        let mut sink = RecordingSink::default();

        handle_interrupt(0x10, &mut regs, &mut memory, &mut sink, true);

        assert_eq!(sink.output, "");
    }

    #[test]
    fn unknown_interrupt_number_is_a_no_op() {
        let mut regs = Registers::new();
        let mut memory = Memory::new();
        let mut sink = RecordingSink::default();
        let outcome = handle_interrupt(0xFF, &mut regs, &mut memory, &mut sink, true);
        assert_eq!(outcome, InterruptOutcome::Continue);
        assert_eq!(sink.output, "");
    }
}
