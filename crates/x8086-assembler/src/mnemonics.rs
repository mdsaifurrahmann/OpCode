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
        "PUSHF" => Mnemonic::Pushf,
        "POPF" => Mnemonic::Popf,
        "LAHF" => Mnemonic::Lahf,
        "SAHF" => Mnemonic::Sahf,
        // NASM spells the no-operand form XLATB; both assemble to D7.
        "XLAT" | "XLATB" => Mnemonic::Xlat,

        "SHL" | "SAL" => Mnemonic::Shl,
        "SHR" => Mnemonic::Shr,
        "SAR" => Mnemonic::Sar,
        "ROL" => Mnemonic::Rol,
        "ROR" => Mnemonic::Ror,
        "RCL" => Mnemonic::Rcl,
        "RCR" => Mnemonic::Rcr,

        "MUL" => Mnemonic::Mul,
        "IMUL" => Mnemonic::Imul,
        "DIV" => Mnemonic::Div,
        "IDIV" => Mnemonic::Idiv,
        "NEG" => Mnemonic::Neg,
        "NOT" => Mnemonic::Not,

        "MOVSB" => Mnemonic::Movsb,
        "MOVSW" => Mnemonic::Movsw,
        "CMPSB" => Mnemonic::Cmpsb,
        "CMPSW" => Mnemonic::Cmpsw,
        "STOSB" => Mnemonic::Stosb,
        "STOSW" => Mnemonic::Stosw,
        "LODSB" => Mnemonic::Lodsb,
        "LODSW" => Mnemonic::Lodsw,
        "SCASB" => Mnemonic::Scasb,
        "SCASW" => Mnemonic::Scasw,

        _ => return None,
    })
}

/// The string-instruction-repeat prefixes: `REP` (unconditional on MOVS/
/// STOS/LODS), `REPE`/`REPZ` and `REPNE`/`REPNZ` (conditional on ZF for
/// CMPS/SCAS). Returns the `x8086_isa::Repeat` the prefix keyword maps to.
pub fn lookup_repeat_prefix(text: &str) -> Option<x8086_isa::Repeat> {
    match text.to_ascii_uppercase().as_str() {
        "REP" => Some(x8086_isa::Repeat::Rep),
        "REPE" | "REPZ" => Some(x8086_isa::Repeat::Repe),
        "REPNE" | "REPNZ" => Some(x8086_isa::Repeat::Repne),
        _ => None,
    }
}

/// Structural/no-op directive keywords recognized only for source
/// compatibility with real emu8086/MASM-style programs (see
/// `StatementKind::NoOp`'s docs). `.STACK`/`.DATA`/`.CODE` are handled
/// separately in `statement_parser` since they get real effect now, not
/// listed here.
pub fn is_noop_directive_keyword(text: &str) -> bool {
    matches!(
        text.to_ascii_uppercase().as_str(),
        ".MODEL" | "SEGMENT" | "ENDS" | "ASSUME" | "ENDP"
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

    #[test]
    fn sal_is_an_alias_for_shl() {
        assert_eq!(lookup_mnemonic("SAL"), Some(Mnemonic::Shl));
        assert_eq!(lookup_mnemonic("SHL"), Some(Mnemonic::Shl));
    }

    #[test]
    fn resolves_shift_rotate_and_unary_group_mnemonics() {
        for (text, expected) in [
            ("SHR", Mnemonic::Shr),
            ("SAR", Mnemonic::Sar),
            ("ROL", Mnemonic::Rol),
            ("ROR", Mnemonic::Ror),
            ("RCL", Mnemonic::Rcl),
            ("RCR", Mnemonic::Rcr),
            ("MUL", Mnemonic::Mul),
            ("IMUL", Mnemonic::Imul),
            ("DIV", Mnemonic::Div),
            ("IDIV", Mnemonic::Idiv),
            ("NEG", Mnemonic::Neg),
            ("NOT", Mnemonic::Not),
        ] {
            assert_eq!(lookup_mnemonic(text), Some(expected), "for {text}");
        }
    }

    #[test]
    fn resolves_string_instruction_mnemonics() {
        for (text, expected) in [
            ("MOVSB", Mnemonic::Movsb),
            ("MOVSW", Mnemonic::Movsw),
            ("CMPSB", Mnemonic::Cmpsb),
            ("CMPSW", Mnemonic::Cmpsw),
            ("STOSB", Mnemonic::Stosb),
            ("STOSW", Mnemonic::Stosw),
            ("LODSB", Mnemonic::Lodsb),
            ("LODSW", Mnemonic::Lodsw),
            ("SCASB", Mnemonic::Scasb),
            ("SCASW", Mnemonic::Scasw),
        ] {
            assert_eq!(lookup_mnemonic(text), Some(expected), "for {text}");
        }
    }

    #[test]
    fn resolves_repeat_prefix_keywords() {
        assert_eq!(lookup_repeat_prefix("REP"), Some(x8086_isa::Repeat::Rep));
        assert_eq!(lookup_repeat_prefix("repe"), Some(x8086_isa::Repeat::Repe));
        assert_eq!(lookup_repeat_prefix("REPZ"), Some(x8086_isa::Repeat::Repe));
        assert_eq!(
            lookup_repeat_prefix("REPNE"),
            Some(x8086_isa::Repeat::Repne)
        );
        assert_eq!(
            lookup_repeat_prefix("REPNZ"),
            Some(x8086_isa::Repeat::Repne)
        );
        assert_eq!(lookup_repeat_prefix("MOV"), None);
    }
}
