//! Renders a decoded `Instruction` back to assembly text, for the
//! debugger's Disassembly panel.
//!
//! This is a text *sink*, not a second decoder - it never needs to be
//! symmetric with `x8086-assembler`'s parser the way the byte-level
//! decode/encode pair does, so minor formatting choices (decimal vs.
//! hex displacements, canonical condition mnemonics rather than
//! whichever alias the source used) are free to prioritize readability.

use x8086_isa::{Condition, Instruction, Mnemonic, Operand, Reg16, Reg8, Width};

/// Formats `instr` as assembly text. `address` is the instruction's own
/// address (not the address after it) - needed because a relative
/// branch's `Operand::Immediate` stores a *displacement*, not an
/// absolute target, and resolving that to the address a human actually
/// wants to see (e.g. "JMP 0106h") requires knowing where the
/// instruction itself sits in memory.
pub fn format_instruction(instr: &Instruction, address: u32) -> String {
    let mnemonic = mnemonic_text(instr.mnemonic);

    if is_relative_branch(instr.mnemonic) {
        if let [Operand::Immediate(displacement)] = instr.operands.as_slice() {
            let target = (address
                .wrapping_add(instr.byte_len as u32)
                .wrapping_add(*displacement as u32))
                & 0xFFFF;
            return format!("{mnemonic} {target:04X}h");
        }
    }

    if instr.operands.is_empty() {
        return mnemonic.to_string();
    }

    let size_prefix = size_prefix_if_needed(instr);
    let operand_text: Vec<String> = instr
        .operands
        .iter()
        .map(|operand| format_operand(operand, size_prefix))
        .collect();
    format!("{mnemonic} {}", operand_text.join(", "))
}

fn is_relative_branch(mnemonic: Mnemonic) -> bool {
    matches!(
        mnemonic,
        Mnemonic::Jmp
            | Mnemonic::Jcc(_)
            | Mnemonic::Loop
            | Mnemonic::Loope
            | Mnemonic::Loopne
            | Mnemonic::Jcxz
            | Mnemonic::Call
    )
}

/// A memory operand's size (byte vs. word) is otherwise ambiguous when
/// nothing else in the instruction pins it down - real assemblers and
/// disassemblers alike need an explicit `BYTE`/`WORD PTR` in exactly
/// this case (see the assembler's own `ambiguous_memory_operand_size`
/// diagnostic, which exists for the same reason on the parsing side).
fn size_prefix_if_needed(instr: &Instruction) -> Option<&'static str> {
    let width = instr.width?;
    let has_memory_operand = instr
        .operands
        .iter()
        .any(|op| matches!(op, Operand::Memory { .. }));
    let has_register_operand = instr
        .operands
        .iter()
        .any(|op| matches!(op, Operand::Reg8(_) | Operand::Reg16(_)));
    if has_memory_operand && !has_register_operand {
        Some(match width {
            Width::Byte => "BYTE PTR ",
            Width::Word => "WORD PTR ",
        })
    } else {
        None
    }
}

fn format_operand(operand: &Operand, size_prefix: Option<&'static str>) -> String {
    match operand {
        Operand::Reg16(reg) => reg16_text(*reg).to_string(),
        Operand::Reg8(reg) => reg8_text(*reg).to_string(),
        Operand::Immediate(value) => format_immediate(*value),
        Operand::Memory {
            segment_override,
            base,
            index,
            displacement,
        } => {
            let prefix = size_prefix.unwrap_or("");
            let segment = segment_override
                .map(|s| format!("{}:", reg16_text(s)))
                .unwrap_or_default();
            if base.is_none() && index.is_none() {
                // Direct addressing: the displacement *is* the address.
                format!("{prefix}{segment}[{:04X}h]", *displacement as u16)
            } else {
                let mut inner = String::new();
                if let Some(base) = base {
                    inner.push_str(reg16_text(*base));
                }
                if let Some(index) = index {
                    if !inner.is_empty() {
                        inner.push('+');
                    }
                    inner.push_str(reg16_text(*index));
                }
                if *displacement != 0 {
                    if *displacement > 0 {
                        inner.push('+');
                        inner.push_str(&displacement.to_string());
                    } else {
                        inner.push_str(&displacement.to_string());
                    }
                }
                format!("{prefix}{segment}[{inner}]")
            }
        }
    }
}

/// Small values print in decimal (readable for loop counters, `INT`
/// vectors, and the like); anything larger prints in hex, matching the
/// assembler's own accepted `h`-suffixed numeric-literal syntax.
fn format_immediate(value: i32) -> String {
    if (0..16).contains(&value) {
        value.to_string()
    } else {
        format!("{value:X}h")
    }
}

fn reg16_text(reg: Reg16) -> &'static str {
    match reg {
        Reg16::Ax => "AX",
        Reg16::Bx => "BX",
        Reg16::Cx => "CX",
        Reg16::Dx => "DX",
        Reg16::Sp => "SP",
        Reg16::Bp => "BP",
        Reg16::Si => "SI",
        Reg16::Di => "DI",
        Reg16::Cs => "CS",
        Reg16::Ds => "DS",
        Reg16::Es => "ES",
        Reg16::Ss => "SS",
        Reg16::Ip => "IP",
    }
}

fn reg8_text(reg: Reg8) -> &'static str {
    match reg {
        Reg8::Al => "AL",
        Reg8::Ah => "AH",
        Reg8::Bl => "BL",
        Reg8::Bh => "BH",
        Reg8::Cl => "CL",
        Reg8::Ch => "CH",
        Reg8::Dl => "DL",
        Reg8::Dh => "DH",
    }
}

fn condition_text(condition: Condition) -> &'static str {
    match condition {
        Condition::Overflow => "JO",
        Condition::NotOverflow => "JNO",
        Condition::Below => "JB",
        Condition::AboveOrEqual => "JAE",
        Condition::Equal => "JE",
        Condition::NotEqual => "JNE",
        Condition::BelowOrEqual => "JBE",
        Condition::Above => "JA",
        Condition::Sign => "JS",
        Condition::NotSign => "JNS",
        Condition::Parity => "JP",
        Condition::NotParity => "JNP",
        Condition::Less => "JL",
        Condition::GreaterOrEqual => "JGE",
        Condition::LessOrEqual => "JLE",
        Condition::Greater => "JG",
    }
}

fn mnemonic_text(mnemonic: Mnemonic) -> &'static str {
    match mnemonic {
        Mnemonic::Mov => "MOV",
        Mnemonic::Push => "PUSH",
        Mnemonic::Pop => "POP",
        Mnemonic::Xchg => "XCHG",
        Mnemonic::Lea => "LEA",
        Mnemonic::Add => "ADD",
        Mnemonic::Adc => "ADC",
        Mnemonic::Sub => "SUB",
        Mnemonic::Sbb => "SBB",
        Mnemonic::Cmp => "CMP",
        Mnemonic::Inc => "INC",
        Mnemonic::Dec => "DEC",
        Mnemonic::And => "AND",
        Mnemonic::Or => "OR",
        Mnemonic::Xor => "XOR",
        Mnemonic::Test => "TEST",
        Mnemonic::Jmp => "JMP",
        Mnemonic::Jcc(condition) => condition_text(condition),
        Mnemonic::Loop => "LOOP",
        Mnemonic::Loope => "LOOPE",
        Mnemonic::Loopne => "LOOPNE",
        Mnemonic::Jcxz => "JCXZ",
        Mnemonic::Call => "CALL",
        Mnemonic::Ret => "RET",
        Mnemonic::Int => "INT",
        Mnemonic::Int3 => "INT3",
        Mnemonic::Iret => "IRET",
        Mnemonic::Hlt => "HLT",
        Mnemonic::Nop => "NOP",
        Mnemonic::Clc => "CLC",
        Mnemonic::Stc => "STC",
        Mnemonic::Cmc => "CMC",
        Mnemonic::Cld => "CLD",
        Mnemonic::Std => "STD",
        Mnemonic::Cli => "CLI",
        Mnemonic::Sti => "STI",
        Mnemonic::Unknown => "???",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_register_to_register_mov() {
        let instr = Instruction::new(
            Mnemonic::Mov,
            vec![Operand::Reg16(Reg16::Ax), Operand::Reg16(Reg16::Bx)],
            Some(Width::Word),
            2,
        );
        assert_eq!(format_instruction(&instr, 0), "MOV AX, BX");
    }

    #[test]
    fn formats_small_immediates_in_decimal_and_large_ones_in_hex() {
        let small = Instruction::new(
            Mnemonic::Mov,
            vec![Operand::Reg16(Reg16::Cx), Operand::Immediate(5)],
            Some(Width::Word),
            3,
        );
        assert_eq!(format_instruction(&small, 0), "MOV CX, 5");

        let large = Instruction::new(
            Mnemonic::Mov,
            vec![Operand::Reg16(Reg16::Cx), Operand::Immediate(0x1234)],
            Some(Width::Word),
            3,
        );
        assert_eq!(format_instruction(&large, 0), "MOV CX, 1234h");
    }

    #[test]
    fn formats_base_index_displacement_memory_operands() {
        let instr = Instruction::new(
            Mnemonic::Mov,
            vec![
                Operand::Reg8(Reg8::Al),
                Operand::mem(Some(Reg16::Bx), Some(Reg16::Si), -2),
            ],
            Some(Width::Byte),
            2,
        );
        assert_eq!(format_instruction(&instr, 0), "MOV AL, [BX+SI-2]");
    }

    #[test]
    fn formats_direct_addressing_in_hex_with_segment_override() {
        let instr = Instruction::new(
            Mnemonic::Mov,
            vec![
                Operand::Reg8(Reg8::Al),
                Operand::mem_direct(0x10).with_segment_override(Reg16::Es),
            ],
            Some(Width::Byte),
            4,
        );
        assert_eq!(format_instruction(&instr, 0), "MOV AL, ES:[0010h]");
    }

    #[test]
    fn adds_a_size_prefix_only_when_the_memory_operand_is_otherwise_ambiguous() {
        let ambiguous = Instruction::new(
            Mnemonic::Mov,
            vec![Operand::mem_direct(0x10), Operand::Immediate(5)],
            Some(Width::Byte),
            4,
        );
        assert_eq!(format_instruction(&ambiguous, 0), "MOV BYTE PTR [0010h], 5");

        let unambiguous = Instruction::new(
            Mnemonic::Mov,
            vec![Operand::Reg8(Reg8::Al), Operand::mem_direct(0x10)],
            Some(Width::Byte),
            3,
        );
        assert_eq!(format_instruction(&unambiguous, 0), "MOV AL, [0010h]");
    }

    #[test]
    fn resolves_a_relative_jump_to_its_absolute_target_address() {
        // JMP SHORT +2, encoded at address 0x0100 as a 2-byte instruction:
        // target = 0x0100 + 2 (byte_len) + 2 (displacement) = 0x0104.
        let instr = Instruction::new(Mnemonic::Jmp, vec![Operand::Immediate(2)], None, 2);
        assert_eq!(format_instruction(&instr, 0x0100), "JMP 0104h");
    }

    #[test]
    fn resolves_a_backward_conditional_jump() {
        // JE -5 at address 0x0010, 2 bytes long: target = 0x0010+2-5 = 0x000D.
        let instr = Instruction::new(
            Mnemonic::Jcc(Condition::Equal),
            vec![Operand::Immediate(-5)],
            None,
            2,
        );
        assert_eq!(format_instruction(&instr, 0x0010), "JE 000Dh");
    }

    #[test]
    fn formats_operand_less_instructions_as_bare_mnemonics() {
        let instr = Instruction::new(Mnemonic::Hlt, vec![], None, 1);
        assert_eq!(format_instruction(&instr, 0), "HLT");
    }

    #[test]
    fn formats_int_with_a_byte_vector() {
        let instr = Instruction::new(Mnemonic::Int, vec![Operand::Immediate(0x21)], None, 2);
        assert_eq!(format_instruction(&instr, 0), "INT 21h");
    }
}
