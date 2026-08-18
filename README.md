# OpCode

A native macOS 8086/80186 assembler, emulator, and debugger - an emu8086-style teaching tool for x86 assembly language, built from scratch for the Mac.

OpCode assembles real 8086/80186 assembly (both MASM/emu8086-style and NASM-style syntax), runs it against a from-scratch CPU emulation with simulated BIOS/DOS interrupts, and gives you a full source-level debugger: breakpoints, step into/over, a distinctive step *back*, live register/flags/memory/stack views, and watch expressions.

The core (CPU, decoder, assembler, debugger) is a pure Rust workspace with no UI dependency, bridged to a native SwiftUI shell over [uniffi](https://mozilla.github.io/uniffi-rs/). Support for additional instruction set architectures (starting with ARM64) is planned.

## Installing on macOS

Download the latest `.dmg` from the [Releases](../../releases) page, open it, and drag **OpCode** into **Applications**.

This build is not yet notarized by Apple (that requires an active Apple Developer Program membership, which this project doesn't have yet), so **macOS will refuse to open it on the first launch** with a message like *"Apple could not verify 'OpCode' is free of malware"* or *"OpCode is damaged and can't be opened."*
This is expected for any app distributed outside the App Store without notarization - it does not mean the app is actually damaged.
To open it anyway:

1. In Finder, **Control-click** (or right-click) `OpCode.app` and choose **Open** from the menu, then confirm **Open** in the dialog that appears.
   If that option isn't offered, or the app still won't launch:
2. Open **System Settings > Privacy & Security**, scroll down to the Security section, and click **Open Anyway** next to the message about OpCode.

You only need to do this once - after the first successful launch, OpCode opens normally like any other app.

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

Or build an installable `.dmg` directly from the command line:

```bash
./scripts/build-dmg-unsigned.sh
```

produces `build/release-unsigned/OpCode-<version>-unsigned.dmg` (unsigned, ad-hoc - see "Installing on macOS" above for what that means for whoever opens it).

Once Developer ID enrollment happens, `./scripts/release-dmg.sh` replaces the above with a fully signed, notarized, and stapled build - see that script's header comment for what it needs.

## Testing

```bash
cargo test --workspace        # Rust core: unit + golden-program integration tests
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --check
```

```bash
cd x8086-mac
xcodebuild -project x8086.xcodeproj -scheme x8086 -destination 'platform=macOS' test -only-testing:x8086Tests     # Swift unit tests
xcodebuild -project x8086.xcodeproj -scheme x8086 -destination 'platform=macOS' test -only-testing:x8086UITests   # End-to-end UI tests
```
