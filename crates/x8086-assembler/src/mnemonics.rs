//! Mnemonic-text -> `x8086_isa::Mnemonic` lookup, including every
//! standard alias for the conditional jumps (`JZ`/`JE` are the same
//! condition, etc.) - matching what real MASM/TASM/emu8086 accept.

use x8086_isa::{Condition, Mnemonic};

pub fn lookup_mnemonic(text: &str) -> Option<Mnemonic> {
    let upper = text.to_ascii_uppercase();
    Some(match upper.as_str() {
        "MOV" => Mnemonic::Mov,
        "PUSH" => Mnemonic::Push,
        "POP" => Mnemonic::Pop,
        "XCHG" => Mnemonic::Xchg,
        "LEA" => Mnemonic::Lea,
        "ADD" => Mnemonic::Add,
        "ADC" => Mnemonic::Adc,
        "SUB" => Mnemonic::Sub,
        "SBB" => Mnemonic::Sbb,
        "CMP" => Mnemonic::Cmp,
        "INC" => Mnemonic::Inc,
        "DEC" => Mnemonic::Dec,
        "AND" => Mnemonic::And,
        "OR" => Mnemonic::Or,
        "XOR" => Mnemonic::Xor,
        "TEST" => Mnemonic::Test,
        "JMP" => Mnemonic::Jmp,

        "JO" => Mnemonic::Jcc(Condition::Overflow),
        "JNO" => Mnemonic::Jcc(Condition::NotOverflow),
        "JB" | "JNAE" | "JC" => Mnemonic::Jcc(Condition::Below),
        "JNB" | "JAE" | "JNC" => Mnemonic::Jcc(Condition::AboveOrEqual),
        "JE" | "JZ" => Mnemonic::Jcc(Condition::Equal),
        "JNE" | "JNZ" => Mnemonic::Jcc(Condition::NotEqual),
        "JBE" | "JNA" => Mnemonic::Jcc(Condition::BelowOrEqual),
        "JA" | "JNBE" => Mnemonic::Jcc(Condition::Above),
        "JS" => Mnemonic::Jcc(Condition::Sign),
        "JNS" => Mnemonic::Jcc(Condition::NotSign),
        "JP" | "JPE" => Mnemonic::Jcc(Condition::Parity),
        "JNP" | "JPO" => Mnemonic::Jcc(Condition::NotParity),
        "JL" | "JNGE" => Mnemonic::Jcc(Condition::Less),
        "JGE" | "JNL" => Mnemonic::Jcc(Condition::GreaterOrEqual),
        "JLE" | "JNG" => Mnemonic::Jcc(Condition::LessOrEqual),
        "JG" | "JNLE" => Mnemonic::Jcc(Condition::Greater),

        "LOOP" => Mnemonic::Loop,
        "LOOPE" | "LOOPZ" => Mnemonic::Loope,
        "LOOPNE" | "LOOPNZ" => Mnemonic::Loopne,
        "JCXZ" => Mnemonic::Jcxz,

        "CALL" => Mnemonic::Call,
        "RET" | "RETN" => Mnemonic::Ret,
        "INT" => Mnemonic::Int,
        "INT3" => Mnemonic::Int3,
        "IRET" | "IRETW" => Mnemonic::Iret,

        "HLT" => Mnemonic::Hlt,
        "NOP" => Mnemonic::Nop,
        "CLC" => Mnemonic::Clc,
        "STC" => Mnemonic::Stc,
        "CMC" => Mnemonic::Cmc,
        "CLD" => Mnemonic::Cld,
        "STD" => Mnemonic::Std,
        "CLI" => Mnemonic::Cli,
        "STI" => Mnemonic::Sti,

        _ => return None,
    })
}

/// Structural/no-op directive keywords recognized only for source
/// compatibility with real emu8086/MASM-style programs (see
/// `StatementKind::NoOp`'s docs).
pub fn is_noop_directive_keyword(text: &str) -> bool {
    matches!(
        text.to_ascii_uppercase().as_str(),
        ".MODEL" | ".STACK" | ".DATA" | ".CODE" | "SEGMENT" | "ENDS" | "ASSUME" | "ENDP"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_basic_mnemonics_case_insensitively() {
        assert_eq!(lookup_mnemonic("mov"), Some(Mnemonic::Mov));
        assert_eq!(lookup_mnemonic("MOV"), Some(Mnemonic::Mov));
        assert_eq!(lookup_mnemonic("Mov"), Some(Mnemonic::Mov));
    }

    #[test]
    fn conditional_jump_aliases_map_to_the_same_condition() {
        assert_eq!(lookup_mnemonic("JE"), lookup_mnemonic("JZ"));
        assert_eq!(lookup_mnemonic("JB"), lookup_mnemonic("JNAE"));
        assert_eq!(lookup_mnemonic("JB"), lookup_mnemonic("JC"));
        assert_eq!(lookup_mnemonic("JG"), lookup_mnemonic("JNLE"));
    }

    #[test]
    fn unknown_text_is_none() {
        assert_eq!(lookup_mnemonic("FROBNICATE"), None);
    }
}
