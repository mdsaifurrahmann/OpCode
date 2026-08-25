//! Parses one instruction operand (register, immediate/symbol, or a
//! `[base+index+disp]` memory reference) from a token stream.

use crate::ast::{ParsedExpr, ParsedOperand};
use crate::expr_parser::parse_expr_chain;
use crate::numbers::parse_number;
use crate::parse_error::ParseError;
use crate::{Span, Token, TokenKind};
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
    // `OFFSET expr` always means "the address," full stop - regardless
    // of what a *bare* reference to the same expression would otherwise
    // mean (see `ParsedExpr::Offset`'s docs for why that's not always
    // the same thing). Parsed as its own self-contained operand form
    // rather than falling through to the rest of this function.
    if let Some(tok) = peek(tokens, *pos) {
        if is_keyword(tok, "offset") {
            *pos += 1;
            let expr = parse_expr_chain(tokens, pos)?;
            return Ok(ParsedOperand::Immediate(ParsedExpr::Offset(Box::new(expr))));
        }
    }

    let mut size_override = None;
    if let Some(tok) = peek(tokens, *pos) {
        if is_keyword(tok, "byte") || is_keyword(tok, "word") {
            let width = if is_keyword(tok, "byte") {
                Width::Byte
            } else {
                Width::Word
            };
            // MASM/emu8086 spell this `BYTE PTR [...]`/`WORD PTR [...]`;
            // NASM drops `PTR` entirely (`WORD [...]`) - accept both by
            // treating `PTR` as optional. MASM also allows the size in
            // front of an *unbracketed* variable (`MOV AL, BYTE PTR v`),
            // where the bare name is already a memory reference in this
            // dialect, so a following symbol/number counts too.
            //
            // Committing only when something that can actually start a
            // memory operand follows is what keeps a real symbol merely
            // spelled "byte"/"word" - unlikely, but possible - from being
            // misread as a size prefix: on its own, or before an
            // operator, it stays an ordinary symbol.
            let mut after_pos = *pos + 1;
            if peek(tokens, after_pos).is_some_and(|t| is_keyword(t, "ptr")) {
                after_pos += 1;
            }
            let starts_memory_operand = peek(tokens, after_pos).is_some_and(|t| {
                is_punct(t, "[")
                    || matches!(t.kind, TokenKind::Identifier | TokenKind::Number)
                    || is_punct(t, "$")
            });
            if starts_memory_operand {
                size_override = Some(width);
                *pos = after_pos;
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
        let (base, index, displacement) = parse_bracket_addressing(tokens, pos)?;
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

        // MASM/emu8086's `reg[expr]` indexing operator: sugar for
        // `[reg+expr]` (`bx[2]`, `si[di]`) - see the identical handling
        // below for the symbol/number case, which this mirrors.
        if peek(tokens, *pos).is_some_and(|t| is_punct(t, "[")) {
            let reg = addressing_reg16_from_name(&tok.text).ok_or_else(|| {
                ParseError::new(
                    tok.span,
                    format!(
                        "'{}' cannot be used as a base/index register inside [...]",
                        tok.text
                    ),
                )
            })?;
            let bracket_span = tokens[*pos].span;
            let (bracket_base, bracket_index, displacement) =
                parse_bracket_addressing(tokens, pos)?;
            let mut base = Some(reg);
            let mut index = None;
            if let Some(b) = bracket_base {
                assign_addressing_reg(&mut base, &mut index, b, bracket_span)?;
            }
            if let Some(ix) = bracket_index {
                assign_addressing_reg(&mut base, &mut index, ix, bracket_span)?;
            }
            return Ok(ParsedOperand::Memory {
                size_override,
                segment_override,
                base,
                index,
                displacement,
            });
        }

        return Ok(operand);
    }

    // A leading `-`/`+` starts a signed immediate (`MOV BX, -10`) - it
    // has to be routed into the expression parser here, since otherwise
    // the sign isn't one of the token kinds that looks like the start of
    // an operand and falls through to the catch-all error below.
    if matches!(tok.kind, TokenKind::Number | TokenKind::Identifier)
        || is_punct(&tok, "$")
        || is_punct(&tok, "-")
        || is_punct(&tok, "+")
    {
        let expr = parse_expr_chain(tokens, pos)?;

        // MASM/emu8086's `symbol[expr]` indexing operator: pure sugar for
        // `[symbol+expr]` (`buff1[1]`, `table[SI]`, `array[BX+2]`). Only
        // recognized here, not inside expr_parser, since it produces a
        // memory operand shape, not an arithmetic value.
        if peek(tokens, *pos).is_some_and(|t| is_punct(t, "[")) {
            let (base, index, bracket_displacement) = parse_bracket_addressing(tokens, pos)?;
            let displacement = Some(match bracket_displacement {
                Some(d) => ParsedExpr::accumulate(Some(expr), d),
                None => expr,
            });
            return Ok(ParsedOperand::Memory {
                size_override,
                segment_override,
                base,
                index,
                displacement,
            });
        }

        // An explicit `BYTE`/`WORD` in front makes this a memory
        // reference of that size (`MOV AL, BYTE PTR v`), not an
        // immediate - the size only means anything applied to memory, so
        // dropping it here would silently reinterpret the operand as the
        // symbol's *address*. Without the prefix a bare name stays an
        // `Immediate`, which `codegen::resolve_operand` then resolves
        // per-dialect (MASM dereferences it, NASM treats it as an
        // address), and that dialect choice is exactly what writing the
        // size explicitly overrides.
        if size_override.is_some() {
            return Ok(ParsedOperand::Memory {
                size_override,
                segment_override,
                base: None,
                index: None,
                displacement: Some(expr),
            });
        }

        return Ok(ParsedOperand::Immediate(expr));
    }

    // A single-quoted character literal (`'0'`, `'A'`) used as an
    // immediate - the classic emu8086 "convert a digit to ASCII" idiom
    // (`ADD AL, '0'`). Only single-character literals are accepted here;
    // multi-character packed-word literals aren't a pattern real emu8086
    // programs rely on, so it's better to reject them clearly than to
    // guess at a packing convention nobody asked for.
    if tok.kind == TokenKind::StringLiteral && tok.text.starts_with('\'') {
        let inner = strip_quotes(&tok.text);
        if inner.len() == 1 {
            *pos += 1;
            return Ok(ParsedOperand::Immediate(ParsedExpr::Number(
                inner.as_bytes()[0] as i64,
            )));
        }
        return Err(ParseError::new(
            tok.span,
            format!(
                "character-literal operand '{}' must be exactly one character",
                tok.text
            ),
        ));
    }

    Err(ParseError::new(
        tok.span,
        format!("unexpected token '{}' in operand", tok.text),
    ))
}

fn strip_quotes(text: &str) -> String {
    let bytes = text.as_bytes();
    if bytes.len() >= 2 {
        text[1..text.len() - 1].to_string()
    } else {
        String::new()
    }
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

/// Parses a `[...]` group and returns its addressing components. Assumes
/// the caller already confirmed `tokens[*pos]` is `[` (both call sites -
/// the plain `[...]` operand and the `reg[...]`/`symbol[...]` postfix
/// forms - check that before calling, so a missing `[` here would be a
/// caller bug, not a user-facing parse error).
fn parse_bracket_addressing(
    tokens: &[Token],
    pos: &mut usize,
) -> Result<MemoryAddressing, ParseError> {
    let open_span = tokens[*pos].span;
    *pos += 1;
    let (base, index, displacement) = parse_memory_expr(tokens, pos)?;
    let close = peek(tokens, *pos)
        .ok_or_else(|| ParseError::new(open_span, "unterminated memory operand: missing ']'"))?;
    if !is_punct(close, "]") {
        return Err(ParseError::new(
            close.span,
            format!("expected ']', found '{}'", close.text),
        ));
    }
    *pos += 1;
    Ok((base, index, displacement))
}

/// Fills `reg` into `base` if it's free, else `index`, else errors - the
/// same "at most two registers" rule `parse_memory_expr`'s own `apply`
/// closure enforces within a single `[...]`, reused here to merge a
/// register that came from *outside* the brackets (the `reg` in
/// `reg[...]`) with whatever the brackets themselves contributed.
fn assign_addressing_reg(
    base: &mut Option<Reg16>,
    index: &mut Option<Reg16>,
    reg: Reg16,
    span: Span,
) -> Result<(), ParseError> {
    if base.is_none() {
        *base = Some(reg);
    } else if index.is_none() {
        *index = Some(reg);
    } else {
        return Err(ParseError::new(
            span,
            "a memory operand allows at most two registers (base + index)",
        ));
    }
    Ok(())
}

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

    // The first term can carry its own sign (`[-10]`); every later term
    // gets its sign from the `+`/`-` separator the loop below requires.
    let mut negate_first = false;
    while let Some(tok) = peek(tokens, *pos) {
        if is_punct(tok, "-") {
            negate_first = !negate_first;
        } else if !is_punct(tok, "+") {
            break;
        }
        *pos += 1;
    }
    let first_span = peek(tokens, *pos).map(|t| t.span).unwrap_or_default();
    let first = parse_memory_term(tokens, pos, negate_first)?;
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
    fn parses_dollar_as_an_immediate_operand() {
        assert_eq!(
            operand_of("$"),
            ParsedOperand::Immediate(ParsedExpr::Symbol("$".to_string()))
        );
    }

    #[test]
    fn offset_keyword_wraps_the_expression_in_an_offset_node() {
        // OFFSET must stay distinguishable from a bare reference - they
        // aren't always the same thing (see `ParsedExpr::Offset`'s docs:
        // MASM/emu8086 dereferences a bare `DB`/`DW` reference, but
        // `OFFSET` always means "the address," in every dialect).
        assert_eq!(
            operand_of("OFFSET myLabel"),
            ParsedOperand::Immediate(ParsedExpr::Offset(Box::new(ParsedExpr::Symbol(
                "myLabel".to_string()
            ))))
        );
    }

    #[test]
    fn parses_a_single_character_literal_as_its_ascii_code() {
        assert_eq!(
            operand_of("'0'"),
            ParsedOperand::Immediate(ParsedExpr::Number(b'0' as i64))
        );
        assert_eq!(
            operand_of("'A'"),
            ParsedOperand::Immediate(ParsedExpr::Number(b'A' as i64))
        );
    }

    #[test]
    fn rejects_a_multi_character_literal_operand() {
        let tokens = tokenize("'AB'");
        let mut pos = 0;
        assert!(parse_operand(&tokens, &mut pos).is_err());
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
    fn parses_nasm_style_word_size_override_without_ptr() {
        // NASM drops "PTR": `WORD [BX]` instead of `WORD PTR [BX]`.
        assert_eq!(operand_of("WORD [BX]"), operand_of("WORD PTR [BX]"));
        let op = operand_of("word [bp-2]");
        assert_eq!(
            op,
            ParsedOperand::Memory {
                size_override: Some(Width::Word),
                segment_override: None,
                base: Some(Reg16::Bp),
                index: None,
                displacement: Some(ParsedExpr::Number(-2)),
            }
        );
    }

    #[test]
    fn a_symbol_named_word_without_a_following_bracket_is_not_a_size_override() {
        // "word" alone (no "[" after it) must still parse as a plain
        // symbol reference, not silently swallowed as a size prefix.
        assert_eq!(
            operand_of("word"),
            ParsedOperand::Immediate(ParsedExpr::Symbol("word".to_string()))
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

    #[test]
    fn symbol_bracket_indexing_is_sugar_for_symbol_plus_displacement() {
        // The exact shape that used to fail with "unexpected token '['
        // after operand" - MASM/emu8086's `buff1[1]` reads as `[buff1+1]`.
        assert_eq!(operand_of("buff1[1]"), operand_of("[buff1+1]"));
    }

    #[test]
    fn symbol_bracket_indexing_with_a_register_inside() {
        // `table[SI]` == `[table+SI]` - the classic array-indexing idiom.
        assert_eq!(operand_of("table[SI]"), operand_of("[table+SI]"));
    }

    #[test]
    fn register_bracket_indexing_is_sugar_for_register_plus_displacement() {
        assert_eq!(operand_of("BX[2]"), operand_of("[BX+2]"));
    }

    #[test]
    fn register_bracket_indexing_with_a_second_register_inside() {
        assert_eq!(operand_of("BX[SI]"), operand_of("[BX+SI]"));
    }

    #[test]
    fn rejects_bracket_indexing_off_a_non_addressing_register() {
        // AX can't be a base/index register, with or without the [...]
        // sugar - the same restriction `[AX]` already has.
        let tokens = tokenize("AX[2]");
        let mut pos = 0;
        assert!(parse_operand(&tokens, &mut pos).is_err());
    }

    #[test]
    fn bracket_indexing_still_rejects_a_third_register() {
        let tokens = tokenize("BX[SI+DI]");
        let mut pos = 0;
        assert!(parse_operand(&tokens, &mut pos).is_err());
    }
}
