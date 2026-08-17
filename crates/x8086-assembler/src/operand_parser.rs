//! Parses one instruction operand (register, immediate/symbol, or a
//! `[base+index+disp]` memory reference) from a token stream.

use crate::ast::{ParsedExpr, ParsedOperand};
use crate::expr_parser::parse_expr_chain;
use crate::numbers::parse_number;
use crate::parse_error::ParseError;
use crate::{Token, TokenKind};
use x8086_isa::{Reg16, Reg8, Width};

fn register_operand_from_name(name: &str) -> Option<ParsedOperand> {
    match name.to_ascii_uppercase().as_str() {
        "AX" => Some(ParsedOperand::Reg16(Reg16::Ax)),
        "BX" => Some(ParsedOperand::Reg16(Reg16::Bx)),
        "CX" => Some(ParsedOperand::Reg16(Reg16::Cx)),
        "DX" => Some(ParsedOperand::Reg16(Reg16::Dx)),
        "SP" => Some(ParsedOperand::Reg16(Reg16::Sp)),
        "BP" => Some(ParsedOperand::Reg16(Reg16::Bp)),
        "SI" => Some(ParsedOperand::Reg16(Reg16::Si)),
        "DI" => Some(ParsedOperand::Reg16(Reg16::Di)),
        "CS" => Some(ParsedOperand::Reg16(Reg16::Cs)),
        "DS" => Some(ParsedOperand::Reg16(Reg16::Ds)),
        "ES" => Some(ParsedOperand::Reg16(Reg16::Es)),
        "SS" => Some(ParsedOperand::Reg16(Reg16::Ss)),
        "AL" => Some(ParsedOperand::Reg8(Reg8::Al)),
        "AH" => Some(ParsedOperand::Reg8(Reg8::Ah)),
        "BL" => Some(ParsedOperand::Reg8(Reg8::Bl)),
        "BH" => Some(ParsedOperand::Reg8(Reg8::Bh)),
        "CL" => Some(ParsedOperand::Reg8(Reg8::Cl)),
        "CH" => Some(ParsedOperand::Reg8(Reg8::Ch)),
        "DL" => Some(ParsedOperand::Reg8(Reg8::Dl)),
        "DH" => Some(ParsedOperand::Reg8(Reg8::Dh)),
        _ => None,
    }
}

/// The 8086 only allows BX/BP as a memory operand's base register and
/// SI/DI as its index - not the other general-purpose or segment
/// registers.
fn addressing_reg16_from_name(name: &str) -> Option<Reg16> {
    match name.to_ascii_uppercase().as_str() {
        "BX" => Some(Reg16::Bx),
        "BP" => Some(Reg16::Bp),
        "SI" => Some(Reg16::Si),
        "DI" => Some(Reg16::Di),
        _ => None,
    }
}

fn peek(tokens: &[Token], pos: usize) -> Option<&Token> {
    tokens.get(pos)
}

fn is_punct(tok: &Token, text: &str) -> bool {
    tok.kind == TokenKind::Punctuation && tok.text == text
}

fn is_keyword(tok: &Token, keyword: &str) -> bool {
    tok.kind == TokenKind::Identifier && tok.text.eq_ignore_ascii_case(keyword)
}

pub fn parse_operand(tokens: &[Token], pos: &mut usize) -> Result<ParsedOperand, ParseError> {
    let mut size_override = None;
    if let Some(tok) = peek(tokens, *pos) {
        if is_keyword(tok, "byte") || is_keyword(tok, "word") {
            let ptr_pos = *pos + 1;
            if peek(tokens, ptr_pos).is_some_and(|t| is_keyword(t, "ptr")) {
                size_override = Some(if is_keyword(tok, "byte") {
                    Width::Byte
                } else {
                    Width::Word
                });
                *pos = ptr_pos + 1;
            }
        }
    }

    let mut segment_override = None;
    if let (Some(seg_tok), Some(colon_tok)) = (peek(tokens, *pos), peek(tokens, *pos + 1)) {
        if seg_tok.kind == TokenKind::Register && is_punct(colon_tok, ":") {
            if let Some(ParsedOperand::Reg16(reg)) = register_operand_from_name(&seg_tok.text) {
                if reg.is_segment() {
                    segment_override = Some(reg);
                    *pos += 2;
                }
            }
        }
    }

    let tok = peek(tokens, *pos)
        .ok_or_else(|| ParseError::new(Default::default(), "expected an operand"))?
        .clone();

    if is_punct(&tok, "[") {
        *pos += 1;
        let (base, index, displacement) = parse_memory_expr(tokens, pos)?;
        let close = peek(tokens, *pos)
            .ok_or_else(|| ParseError::new(tok.span, "unterminated memory operand: missing ']'"))?;
        if !is_punct(close, "]") {
            return Err(ParseError::new(
                close.span,
                format!("expected ']', found '{}'", close.text),
            ));
        }
        *pos += 1;
        return Ok(ParsedOperand::Memory {
            size_override,
            segment_override,
            base,
            index,
            displacement,
        });
    }

    if segment_override.is_some() {
        return Err(ParseError::new(
            tok.span,
            "a segment override (e.g. ES:) must be followed by a memory operand in [...]",
        ));
    }

    if tok.kind == TokenKind::Register {
        if size_override.is_some() {
            return Err(ParseError::new(
                tok.span,
                "BYTE PTR/WORD PTR cannot precede a register operand",
            ));
        }
        let operand = register_operand_from_name(&tok.text)
            .ok_or_else(|| ParseError::new(tok.span, format!("unknown register '{}'", tok.text)))?;
        *pos += 1;
        return Ok(operand);
    }

    if matches!(tok.kind, TokenKind::Number | TokenKind::Identifier) {
        let expr = parse_expr_chain(tokens, pos)?;
        return Ok(ParsedOperand::Immediate(expr));
    }

    Err(ParseError::new(
        tok.span,
        format!("unexpected token '{}' in operand", tok.text),
    ))
}

/// One register/number/symbol term inside `[...]`. Only consumes a
/// single token - `parse_memory_expr` is what requires an explicit
/// `+`/`-` between terms (so e.g. a stray missing `+` is a clear parse
/// error rather than silently accepted).
enum MemoryTerm {
    Reg(Reg16),
    Value(ParsedExpr),
}

fn parse_memory_term(
    tokens: &[Token],
    pos: &mut usize,
    negate: bool,
) -> Result<MemoryTerm, ParseError> {
    let tok = peek(tokens, *pos)
        .ok_or_else(|| ParseError::new(Default::default(), "unterminated memory operand"))?
        .clone();
    match tok.kind {
        TokenKind::Register => {
            if negate {
                return Err(ParseError::new(tok.span, "cannot negate a register"));
            }
            let reg = addressing_reg16_from_name(&tok.text).ok_or_else(|| {
                ParseError::new(
                    tok.span,
                    format!(
                        "'{}' cannot be used as a base/index register inside [...]",
                        tok.text
                    ),
                )
            })?;
            *pos += 1;
            Ok(MemoryTerm::Reg(reg))
        }
        TokenKind::Number => {
            let value = parse_number(&tok.text).ok_or_else(|| {
                ParseError::new(tok.span, format!("invalid numeric literal '{}'", tok.text))
            })?;
            *pos += 1;
            Ok(MemoryTerm::Value(ParsedExpr::Number(if negate {
                -value
            } else {
                value
            })))
        }
        TokenKind::Identifier => {
            if negate {
                return Err(ParseError::new(tok.span, "cannot negate a symbol"));
            }
            *pos += 1;
            Ok(MemoryTerm::Value(ParsedExpr::Symbol(tok.text)))
        }
        _ => Err(ParseError::new(
            tok.span,
            format!("unexpected token '{}' in memory operand", tok.text),
        )),
    }
}

/// A memory operand's addressing components: base register, index
/// register, and displacement expression (all optional).
type MemoryAddressing = (Option<Reg16>, Option<Reg16>, Option<ParsedExpr>);

fn parse_memory_expr(tokens: &[Token], pos: &mut usize) -> Result<MemoryAddressing, ParseError> {
    let mut base = None;
    let mut index = None;
    let mut displacement: Option<ParsedExpr> = None;

    let apply = |term: MemoryTerm,
                 base: &mut Option<Reg16>,
                 index: &mut Option<Reg16>,
                 displacement: &mut Option<ParsedExpr>,
                 span| match term {
        MemoryTerm::Reg(reg) => {
            if base.is_none() {
                *base = Some(reg);
                Ok(())
            } else if index.is_none() {
                *index = Some(reg);
                Ok(())
            } else {
                Err(ParseError::new(
                    span,
                    "a memory operand allows at most two registers (base + index)",
                ))
            }
        }
        MemoryTerm::Value(expr) => {
            *displacement = Some(ParsedExpr::accumulate(displacement.take(), expr));
            Ok(())
        }
    };

    let first_span = peek(tokens, *pos).map(|t| t.span).unwrap_or_default();
    let first = parse_memory_term(tokens, pos, false)?;
    apply(first, &mut base, &mut index, &mut displacement, first_span)?;

    loop {
        let Some(tok) = peek(tokens, *pos) else {
            return Err(ParseError::new(
                Default::default(),
                "unterminated memory operand",
            ));
        };
        if is_punct(tok, "]") {
            break;
        }
        let negate = if is_punct(tok, "+") {
            *pos += 1;
            false
        } else if is_punct(tok, "-") {
            *pos += 1;
            true
        } else {
            return Err(ParseError::new(
                tok.span,
                format!(
                    "expected '+', '-', or ']' in memory operand, found '{}'",
                    tok.text
                ),
            ));
        };
        let span = peek(tokens, *pos).map(|t| t.span).unwrap_or_default();
        let term = parse_memory_term(tokens, pos, negate)?;
        apply(term, &mut base, &mut index, &mut displacement, span)?;
    }

    Ok((base, index, displacement))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tokenize;

    fn operand_of(source: &str) -> ParsedOperand {
        let tokens = tokenize(source);
        let mut pos = 0;
        parse_operand(&tokens, &mut pos).unwrap()
    }

    #[test]
    fn parses_16bit_register() {
        assert_eq!(operand_of("AX"), ParsedOperand::Reg16(Reg16::Ax));
    }

    #[test]
    fn parses_8bit_register() {
        assert_eq!(operand_of("AL"), ParsedOperand::Reg8(Reg8::Al));
    }

    #[test]
    fn parses_immediate_number() {
        assert_eq!(
            operand_of("5"),
            ParsedOperand::Immediate(ParsedExpr::Number(5))
        );
        assert_eq!(
            operand_of("0FFh"),
            ParsedOperand::Immediate(ParsedExpr::Number(0xFF))
        );
    }

    #[test]
    fn parses_bare_symbol_as_immediate() {
        assert_eq!(
            operand_of("myLabel"),
            ParsedOperand::Immediate(ParsedExpr::Symbol("myLabel".to_string()))
        );
    }

    #[test]
    fn parses_simple_memory_operand() {
        assert_eq!(
            operand_of("[BX]"),
            ParsedOperand::Memory {
                size_override: None,
                segment_override: None,
                base: Some(Reg16::Bx),
                index: None,
                displacement: None
            }
        );
    }

    #[test]
    fn parses_base_plus_index_plus_displacement() {
        let op = operand_of("[BX+SI+4]");
        assert_eq!(
            op,
            ParsedOperand::Memory {
                size_override: None,
                segment_override: None,
                base: Some(Reg16::Bx),
                index: Some(Reg16::Si),
                displacement: Some(ParsedExpr::Number(4)),
            }
        );
    }

    #[test]
    fn parses_direct_address_memory_operand() {
        let op = operand_of("[1234h]");
        assert_eq!(
            op,
            ParsedOperand::Memory {
                size_override: None,
                segment_override: None,
                base: None,
                index: None,
                displacement: Some(ParsedExpr::Number(0x1234))
            }
        );
    }

    #[test]
    fn parses_symbol_memory_operand() {
        let op = operand_of("[myVar]");
        assert_eq!(
            op,
            ParsedOperand::Memory {
                size_override: None,
                segment_override: None,
                base: None,
                index: None,
                displacement: Some(ParsedExpr::Symbol("myVar".to_string())),
            }
        );
    }

    #[test]
    fn parses_byte_ptr_size_override() {
        let op = operand_of("BYTE PTR [BX]");
        assert_eq!(
            op,
            ParsedOperand::Memory {
                size_override: Some(Width::Byte),
                segment_override: None,
                base: Some(Reg16::Bx),
                index: None,
                displacement: None
            }
        );
    }

    #[test]
    fn parses_segment_override() {
        let op = operand_of("ES:[BX]");
        assert_eq!(
            op,
            ParsedOperand::Memory {
                size_override: None,
                segment_override: Some(Reg16::Es),
                base: Some(Reg16::Bx),
                index: None,
                displacement: None
            }
        );
    }

    #[test]
    fn rejects_sp_as_a_memory_base_register() {
        let tokens = tokenize("[SP]");
        let mut pos = 0;
        assert!(
            parse_operand(&tokens, &mut pos).is_err(),
            "SP is not a valid 8086 addressing-mode register"
        );
    }

    #[test]
    fn rejects_a_third_register_in_a_memory_operand() {
        let tokens = tokenize("[BX+SI+DI]");
        let mut pos = 0;
        assert!(parse_operand(&tokens, &mut pos).is_err());
    }

    #[test]
    fn rejects_unterminated_memory_operand() {
        let tokens = tokenize("[BX");
        let mut pos = 0;
        assert!(parse_operand(&tokens, &mut pos).is_err());
    }

    #[test]
    fn requires_an_explicit_operator_between_memory_terms() {
        let tokens = tokenize("[BX SI]");
        let mut pos = 0;
        assert!(
            parse_operand(&tokens, &mut pos).is_err(),
            "'[BX SI]' without a '+' must not silently parse as [BX+SI]"
        );
    }
}
