<p align="center">
<img width="889" height="500" alt="Screenshot 2026-08-19 at 4 36 55 AM" src="https://github.com/user-attachments/assets/094eade2-ebc6-4ddf-8aec-7b6e84b16648" />
</p>   

# OpCode

A native macOS 8086/80186 assembler, emulator, and debugger - an emu8086-style teaching tool for x86 assembly language, built from scratch for the Mac.

OpCode assembles real 8086/80186 assembly and runs it against a from-scratch CPU emulation with simulated BIOS/DOS interrupts, backed by a full source-level debugger.

## Features

- **Assembler** supporting both MASM/emu8086-style and NASM-style syntax, with line-numbered error diagnostics and click-to-jump.
- **CPU emulation**: real-mode 8086/80186, segmented memory, full flags, simulated BIOS/DOS console and keyboard interrupts.
- **Debugger**: breakpoints, Step In, Step Over, and a distinctive Undo Step that steps *backward*, plus Run to Here.
- **Live inspection**: registers, flags, memory, and stack views that update as you step, plus watch expressions.
- **Syntax-highlighted editor** with a breakpoint gutter and inline diagnostics.
- **Utilities**: a bundled sample-program library, a number base converter, an ASCII table reference, and listing export/print.

## Installing on macOS

Download the latest `.dmg` from the [Releases](../../releases) page, open it, and drag **OpCode** into **Applications**.

This build isn't code-signed, so macOS will show a security warning the first time you open it (something like *"Apple could not verify 'OpCode' is free of malware"*).
To open it anyway:

1. In Finder, **Control-click** (or right-click) `OpCode.app` and choose **Open**, then confirm **Open** in the dialog that appears.
   If that option isn't offered, or the app still won't launch:
2. Open **System Settings > Privacy & Security**, scroll down to the Security section, and click **Open Anyway** next to the message about OpCode.

You only need to do this once - after the first successful launch, OpCode opens normally like any other app.

## Usage

1. **Write or open code**: `File > New`, `File > Open…`, or pick one of the bundled samples under `File > Open Sample`.
2. **Run**: click **Run** to assemble and execute from the top. Errors show up as a line-numbered list below the editor - click one to jump to it.
3. **Set a breakpoint** by clicking the gutter next to a line, then **Run** - execution stops there.
4. **Step through**: **Step In** executes one instruction (following into a `CALL`); **Step Over** runs a whole subroutine call at once; **Undo Step** reverses the last instruction, restoring registers, flags, and memory exactly as they were.
5. **Inspect state** any time you're paused: the Registers, Flags, Memory, and Stack panels update live, and you can add expressions to **Watches** to track specific values.
6. **Select a line in the Disassembly panel** and use **Run to Here** to execute straight to it, like a one-off breakpoint.

## Limitations

- Real-mode 8086/80186 only - no protected mode or later x86 extensions.
- Simulated BIOS/DOS interrupts cover the common console/keyboard/terminate services, not the full DOS API surface.
- macOS only, direct distribution (not on the Mac App Store).

## Building from source

Requirements: Xcode (full app, not just Command Line Tools), [Rust](https://rustup.rs), and [XcodeGen](https://github.com/yonaskolb/XcodeGen) (`brew install xcodegen`).

```bash
git clone <this repo>
cd x8086
./scripts/build-universal.sh   # builds the Rust core as a universal XCFramework
cd x8086-mac
xcodegen generate
open x8086.xcodeproj
```
