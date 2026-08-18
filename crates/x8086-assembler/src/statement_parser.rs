//! Turns one source line's tokens into zero or more `Statement`s. A line
//! can hold a label and content together (`loop_start: ADD AX, BX`), so
//! this emits a `Statement::Label` followed by whatever else the line
//! contains, all sharing that line's number.

use crate::ast::{DataItem, ParsedOperand, SegmentRole, Statement, StatementKind};
use crate::expr_parser::parse_expr_chain;
use crate::mnemonics::{is_noop_directive_keyword, lookup_mnemonic, lookup_repeat_prefix};
use crate::operand_parser::parse_operand;
use crate::parse_error::ParseError;
use crate::{tokenize, Token, TokenKind};
use x8086_isa::Mnemonic;

/// Tokenizes and parses a whole program. Parse errors are collected
/// (not fatal): a bad line is skipped, recovering at the next line, so
/// one typo doesn't hide every other diagnostic in the file.
pub fn parse_program(source: &str) -> (Vec<Statement>, Vec<ParseError>) {
    let tokens = tokenize(source);
    let mut statements = Vec::new();
    let mut errors = Vec::new();

    for line_tokens in split_lines(&tokens) {
        if line_tokens.is_empty() {
            continue;
        }
        match parse_line(&line_tokens) {
            Ok(mut line_statements) => statements.append(&mut line_statements),
            Err(error) => errors.push(error),
        }
    }

    (statements, errors)
}

fn split_lines(tokens: &[Token]) -> Vec<Vec<Token>> {
    let mut lines = Vec::new();
    let mut current = Vec::new();
    for tok in tokens {
        match tok.kind {
            TokenKind::Newline => lines.push(std::mem::take(&mut current)),
            TokenKind::Comment => {}
            _ => current.push(tok.clone()),
        }
    }
    if !current.is_empty() {
        lines.push(current);
    }
    lines
}

fn is_punct(tok: &Token, text: &str) -> bool {
    tok.kind == TokenKind::Punctuation && tok.text == text
}

fn is_keyword(tok: &Token, keyword: &str) -> bool {
    tok.kind == TokenKind::Identifier && tok.text.eq_ignore_ascii_case(keyword)
}

/// Errors if any tokens remain unconsumed at the end of a line - catches
/// trailing garbage (`MOV AX, 5 extra`) instead of silently ignoring it.
fn ensure_line_consumed(tokens: &[Token], pos: usize) -> Result<(), ParseError> {
    match tokens.get(pos) {
        None => Ok(()),
        Some(tok) => Err(ParseError::new(
            tok.span,
            format!("unexpected trailing token '{}'", tok.text),
        )),
    }
}

fn parse_line(tokens: &[Token]) -> Result<Vec<Statement>, ParseError> {
    let line = tokens[0].span.line;
    let mut pos = 0;
    let mut statements = Vec::new();

    // `label:`
    if tokens[0].kind == TokenKind::Identifier && tokens.get(1).is_some_and(|t| is_punct(t, ":")) {
        statements.push(Statement {
            kind: StatementKind::Label(tokens[0].text.clone()),
            line,
        });
        pos = 2;
        if pos >= tokens.len() {
            return Ok(statements);
        }
    }

    let first = tokens[pos].clone();

    if first.kind == TokenKind::Identifier {
        if is_keyword(&first, "org") {
            pos += 1;
            let expr = parse_expr_chain(tokens, &mut pos)?;
            ensure_line_consumed(tokens, pos)?;
            statements.push(Statement {
                kind: StatementKind::Org(expr),
                line,
            });
            return Ok(statements);
        }

        if is_keyword(&first, "end") {
            pos += 1;
            let label = match tokens.get(pos) {
                Some(tok) if tok.kind == TokenKind::Identifier => {
                    pos += 1;
                    Some(tok.text.clone())
                }
                _ => None,
            };
            ensure_line_consumed(tokens, pos)?;
            statements.push(Statement {
                kind: StatementKind::End(label),
                line,
            });
            return Ok(statements);
        }

        if is_keyword(&first, ".stack") {
            pos += 1;
            let size = parse_expr_chain(tokens, &mut pos)?;
            ensure_line_consumed(tokens, pos)?;
            statements.push(Statement {
                kind: StatementKind::Stack(size),
                line,
            });
            return Ok(statements);
        }

        if is_keyword(&first, ".data") || is_keyword(&first, ".code") {
            let role = if is_keyword(&first, ".data") {
                SegmentRole::Data
            } else {
                SegmentRole::Code
            };
            pos += 1;
            ensure_line_consumed(tokens, pos)?;
            statements.push(Statement {
                kind: StatementKind::SegmentSwitch(role),
                line,
            });
            return Ok(statements);
        }

        // NASM `SECTION .text`/`SECTION .data` (also accepts the bare
        // `.bss`/`text`/`data`/`code` spellings some NASM programs use) -
        // maps onto the same `SegmentSwitch` mechanism as `.CODE`/`.DATA`.
        if is_keyword(&first, "section") {
            pos += 1;
            let name_tok = tokens.get(pos).ok_or_else(|| {
                ParseError::new(first.span, "expected a section name after SECTION")
            })?;
            let role = if [".data", "data", ".bss", "bss"]
                .iter()
                .any(|kw| is_keyword(name_tok, kw))
            {
                SegmentRole::Data
            } else if [".text", "text", ".code", "code"]
                .iter()
                .any(|kw| is_keyword(name_tok, kw))
            {
                SegmentRole::Code
            } else {
                return Err(ParseError::new(
                    name_tok.span,
                    format!("unrecognized SECTION name '{}'", name_tok.text),
                ));
            };
            pos += 1;
            ensure_line_consumed(tokens, pos)?;
            statements.push(Statement {
                kind: StatementKind::SegmentSwitch(role),
                line,
            });
            return Ok(statements);
        }

        // `REP`/`REPE`/`REPZ`/`REPNE`/`REPNZ` string-instruction prefix.
        if let Some(repeat) = lookup_repeat_prefix(&first.text) {
            pos += 1;
            let mnemonic_tok = tokens.get(pos).ok_or_else(|| {
                ParseError::new(
                    first.span,
                    format!("expected a string instruction after '{}'", first.text),
                )
            })?;
            let mnemonic = lookup_mnemonic(&mnemonic_tok.text).ok_or_else(|| {
                ParseError::new(
                    mnemonic_tok.span,
                    format!(
                        "unrecognized statement starting with '{}'",
                        mnemonic_tok.text
                    ),
                )
            })?;
            if !is_repeatable_mnemonic(mnemonic) {
                return Err(ParseError::new(
                    mnemonic_tok.span,
                    format!(
                        "'{}' cannot be prefixed with {} - only string instructions can",
                        mnemonic_tok.text, first.text
                    ),
                ));
            }
            pos += 1;
            let operands = parse_operand_list(tokens, &mut pos)?;
            ensure_line_consumed(tokens, pos)?;
            statements.push(Statement {
                kind: StatementKind::Instruction {
                    mnemonic,
                    operands,
                    short_jump: false,
                    repeat: Some(repeat),
                },
                line,
            });
            return Ok(statements);
        }

        // `NAME EQU expr` / `NAME DB ...` / `NAME DW ...` / `NAME PROC ...`
        // / `NAME ENDP` / `NAME TIMES count DB/DW item`
        if let Some(second) = tokens.get(pos + 1) {
            if second.kind == TokenKind::Identifier {
                if is_keyword(second, "equ") {
                    let name = first.text.clone();
                    pos += 2;
                    let value = parse_expr_chain(tokens, &mut pos)?;
                    ensure_line_consumed(tokens, pos)?;
                    statements.push(Statement {
                        kind: StatementKind::Equ { name, value },
                        line,
                    });
                    return Ok(statements);
                }
                if is_keyword(second, "db") || is_keyword(second, "dw") {
                    let is_byte = is_keyword(second, "db");
                    statements.push(Statement {
                        kind: StatementKind::Label(first.text.clone()),
                        line,
                    });
                    pos += 2;
                    let items = parse_data_items(tokens, &mut pos)?;
                    ensure_line_consumed(tokens, pos)?;
                    let kind = if is_byte {
                        StatementKind::Db(items)
                    } else {
                        StatementKind::Dw(items)
                    };
                    statements.push(Statement { kind, line });
                    return Ok(statements);
                }
                if is_keyword(second, "proc") {
                    // `NAME PROC [NEAR|FAR]` - the name is a callable
                    // label; the rest of the line carries no runtime
                    // effect in our flat memory model.
                    statements.push(Statement {
                        kind: StatementKind::Label(first.text.clone()),
                        line,
                    });
                    return Ok(statements);
                }
                if is_keyword(second, "endp") {
                    // `NAME ENDP` closes a PROC - a pure structural
                    // marker. Unlike PROC, it defines no symbol, so
                    // (unlike bare `ENDP`, which is dispatched below via
                    // `is_noop_directive_keyword`) there's nothing to do
                    // beyond recognizing and discarding the line.
                    return Ok(statements);
                }
                if is_keyword(second, "times") {
                    let name = first.text.clone();
                    statements.push(Statement {
                        kind: StatementKind::Label(name),
                        line,
                    });
                    pos += 2;
                    statements.push(parse_times(tokens, &mut pos, line)?);
                    return Ok(statements);
                }
            }
        }
    }

    // Bare `DB`/`DW` (no leading name), e.g. unnamed padding bytes.
    if is_keyword(&first, "db") || is_keyword(&first, "dw") {
        let is_byte = is_keyword(&first, "db");
        pos += 1;
        let items = parse_data_items(tokens, &mut pos)?;
        ensure_line_consumed(tokens, pos)?;
        let kind = if is_byte {
            StatementKind::Db(items)
        } else {
            StatementKind::Dw(items)
        };
        statements.push(Statement { kind, line });
        return Ok(statements);
    }

    // Bare `TIMES count DB/DW item` (no leading name), NASM's repeat
    // directive - most commonly used for unnamed padding.
    if is_keyword(&first, "times") {
        pos += 1;
        statements.push(parse_times(tokens, &mut pos, line)?);
        return Ok(statements);
    }

    if first.kind == TokenKind::Identifier && is_noop_directive_keyword(&first.text) {
        return Ok(statements); // whole line is a recognized-but-inert directive
    }

    if first.kind == TokenKind::Identifier {
        if let Some(mnemonic) = lookup_mnemonic(&first.text) {
            pos += 1;
            let short_jump = matches!(tokens.get(pos), Some(tok) if is_keyword(tok, "short"));
            if short_jump {
                pos += 1;
            }
            let operands = parse_operand_list(tokens, &mut pos)?;
            ensure_line_consumed(tokens, pos)?;
            statements.push(Statement {
                kind: StatementKind::Instruction {
                    mnemonic,
                    operands,
                    short_jump,
                    repeat: None,
                },
                line,
            });
            return Ok(statements);
        }
    }

    Err(ParseError::new(
        first.span,
        format!("unrecognized statement starting with '{}'", first.text),
    ))
}

fn is_repeatable_mnemonic(mnemonic: Mnemonic) -> bool {
    matches!(
        mnemonic,
        Mnemonic::Movsb
            | Mnemonic::Movsw
            | Mnemonic::Cmpsb
            | Mnemonic::Cmpsw
            | Mnemonic::Stosb
            | Mnemonic::Stosw
            | Mnemonic::Lodsb
            | Mnemonic::Lodsw
            | Mnemonic::Scasb
            | Mnemonic::Scasw
    )
}

/// `TIMES count DB/DW item` (the `TIMES` keyword itself already consumed
/// by the caller, which also owns emitting any leading label) - NASM
/// sugar for repeating one data item, equivalent to `count DUP(item)`.
fn parse_times(tokens: &[Token], pos: &mut usize, line: u32) -> Result<Statement, ParseError> {
    let count = parse_expr_chain(tokens, pos)?;
    let size_tok = tokens.get(*pos).ok_or_else(|| {
        ParseError::new(Default::default(), "expected DB or DW after TIMES count")
    })?;
    let is_byte = is_keyword(size_tok, "db");
    let is_word = is_keyword(size_tok, "dw");
    if !is_byte && !is_word {
        return Err(ParseError::new(
            size_tok.span,
            format!(
                "expected DB or DW after TIMES count, found '{}'",
                size_tok.text
            ),
        ));
    }
    *pos += 1;
    let item = parse_data_item(tokens, pos)?;
    ensure_line_consumed(tokens, *pos)?;
    let dup = DataItem::Dup {
        count,
        item: Box::new(item),
    };
    let kind = if is_byte {
        StatementKind::Db(vec![dup])
    } else {
        StatementKind::Dw(vec![dup])
    };
    Ok(Statement { kind, line })
}

fn parse_operand_list(tokens: &[Token], pos: &mut usize) -> Result<Vec<ParsedOperand>, ParseError> {
    let mut operands = Vec::new();
    if *pos >= tokens.len() {
        return Ok(operands);
    }
    loop {
        operands.push(parse_operand(tokens, pos)?);
        match tokens.get(*pos) {
            Some(tok) if is_punct(tok, ",") => *pos += 1,
            Some(tok) => {
                return Err(ParseError::new(
                    tok.span,
                    format!("unexpected token '{}' after operand", tok.text),
                ))
            }
            None => break,
        }
    }
    Ok(operands)
}

fn parse_data_items(tokens: &[Token], pos: &mut usize) -> Result<Vec<DataItem>, ParseError> {
    let mut items = Vec::new();
    loop {
        items.push(parse_data_item(tokens, pos)?);
        match tokens.get(*pos) {
            Some(tok) if is_punct(tok, ",") => *pos += 1,
            _ => break,
        }
    }
    Ok(items)
}

fn parse_data_item(tokens: &[Token], pos: &mut usize) -> Result<DataItem, ParseError> {
    let tok = tokens
        .get(*pos)
        .cloned()
        .ok_or_else(|| ParseError::new(Default::default(), "expected a data item"))?;

    if tok.kind == TokenKind::StringLiteral {
        *pos += 1;
        return Ok(DataItem::Str(strip_quotes(&tok.text)));
    }
    if is_punct(&tok, "?") {
        *pos += 1;
        return Ok(DataItem::Uninitialized);
    }

    // `count DUP (item)` - only reachable when a number/symbol term is
    // immediately (no operator) followed by "DUP"; parse_expr_chain
    // itself never crosses that boundary (see its own tests), so we
    // just need to check for it once the count expression is parsed.
    let count_start = *pos;
    let count = parse_expr_chain(tokens, pos)?;
    if let Some(dup_tok) = tokens.get(*pos) {
        if is_keyword(dup_tok, "dup") {
            *pos += 1;
            let open = tokens
                .get(*pos)
                .ok_or_else(|| ParseError::new(dup_tok.span, "expected '(' after DUP"))?;
            if !is_punct(open, "(") {
                return Err(ParseError::new(
                    open.span,
                    format!("expected '(' after DUP, found '{}'", open.text),
                ));
            }
            *pos += 1;
            let inner = parse_data_item(tokens, pos)?;
            let close = tokens
                .get(*pos)
                .ok_or_else(|| ParseError::new(open.span, "expected ')' to close DUP"))?;
            if !is_punct(close, ")") {
                return Err(ParseError::new(
                    close.span,
                    format!("expected ')' to close DUP, found '{}'", close.text),
                ));
            }
            *pos += 1;
            return Ok(DataItem::Dup {
                count,
                item: Box::new(inner),
            });
        }
    }
    let _ = count_start;
    Ok(DataItem::Value(count))
}

fn strip_quotes(text: &str) -> String {
    let bytes = text.as_bytes();
    if bytes.len() >= 2 {
        text[1..text.len() - 1].to_string()
    } else {
        String::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use x8086_isa::{Condition, Mnemonic, Reg16};

    #[test]
    fn parses_label_only_line() {
        let (stmts, errors) = parse_program("start:");
        assert!(errors.is_empty());
        assert_eq!(
            stmts,
            vec![Statement {
                kind: StatementKind::Label("start".to_string()),
                line: 1
            }]
        );
    }

    #[test]
    fn parses_label_and_instruction_on_one_line() {
        let (stmts, errors) = parse_program("start: HLT");
        assert!(errors.is_empty());
        assert_eq!(stmts.len(), 2);
        assert_eq!(stmts[0].kind, StatementKind::Label("start".to_string()));
        assert_eq!(
            stmts[1].kind,
            StatementKind::Instruction {
                mnemonic: Mnemonic::Hlt,
                operands: vec![],
                short_jump: false,
                repeat: None,
            }
        );
    }

    #[test]
    fn parses_instruction_with_two_operands() {
        let (stmts, errors) = parse_program("MOV AX, 5");
        assert!(errors.is_empty());
        assert_eq!(stmts.len(), 1);
        assert_eq!(
            stmts[0].kind,
            StatementKind::Instruction {
                mnemonic: Mnemonic::Mov,
                operands: vec![
                    ParsedOperand::Reg16(Reg16::Ax),
                    ParsedOperand::Immediate(crate::ast::ParsedExpr::Number(5))
                ],
                short_jump: false,
                repeat: None,
            }
        );
    }

    #[test]
    fn parses_conditional_jump_alias() {
        let (stmts, errors) = parse_program("JZ done");
        assert!(errors.is_empty());
        match &stmts[0].kind {
            StatementKind::Instruction { mnemonic, .. } => {
                assert_eq!(*mnemonic, Mnemonic::Jcc(Condition::Equal))
            }
            other => panic!("expected an instruction, got {other:?}"),
        }
    }

    #[test]
    fn parses_jmp_short() {
        let (stmts, errors) = parse_program("JMP SHORT done");
        assert!(errors.is_empty());
        match &stmts[0].kind {
            StatementKind::Instruction { short_jump, .. } => assert!(*short_jump),
            other => panic!("expected an instruction, got {other:?}"),
        }
    }

    #[test]
    fn parses_org_directive() {
        let (stmts, errors) = parse_program("ORG 100h");
        assert!(errors.is_empty());
        assert_eq!(
            stmts[0].kind,
            StatementKind::Org(crate::ast::ParsedExpr::Number(0x100))
        );
    }

    #[test]
    fn parses_equ_directive() {
        let (stmts, errors) = parse_program("COUNT EQU 10");
        assert!(errors.is_empty());
        assert_eq!(
            stmts[0].kind,
            StatementKind::Equ {
                name: "COUNT".to_string(),
                value: crate::ast::ParsedExpr::Number(10)
            }
        );
    }

    #[test]
    fn parses_named_db_with_string_and_number_items() {
        let (stmts, errors) = parse_program("msg DB \"hi\", 0");
        assert!(errors.is_empty());
        assert_eq!(stmts.len(), 2);
        assert_eq!(stmts[0].kind, StatementKind::Label("msg".to_string()));
        assert_eq!(
            stmts[1].kind,
            StatementKind::Db(vec![
                DataItem::Str("hi".to_string()),
                DataItem::Value(crate::ast::ParsedExpr::Number(0))
            ])
        );
    }

    #[test]
    fn parses_dup_directive() {
        let (stmts, errors) = parse_program("buf DB 10 DUP(?)");
        assert!(errors.is_empty());
        assert_eq!(
            stmts[1].kind,
            StatementKind::Db(vec![DataItem::Dup {
                count: crate::ast::ParsedExpr::Number(10),
                item: Box::new(DataItem::Uninitialized)
            }])
        );
    }

    #[test]
    fn parses_dw_with_zero_initializer() {
        let (stmts, errors) = parse_program("counter DW 0");
        assert!(errors.is_empty());
        assert_eq!(stmts[0].kind, StatementKind::Label("counter".to_string()));
        assert_eq!(
            stmts[1].kind,
            StatementKind::Dw(vec![DataItem::Value(crate::ast::ParsedExpr::Number(0))])
        );
    }

    #[test]
    fn parses_end_with_entry_label() {
        let (stmts, errors) = parse_program("END start");
        assert!(errors.is_empty());
        assert_eq!(stmts[0].kind, StatementKind::End(Some("start".to_string())));
    }

    #[test]
    fn noop_directives_produce_no_statements_but_no_error() {
        let (stmts, errors) =
            parse_program(".MODEL SMALL\nSEGMENT CODE\nENDS\nASSUME CS:CODE\nENDP\nHLT");
        assert!(errors.is_empty());
        assert_eq!(stmts.len(), 1); // only the HLT
        assert_eq!(
            stmts[0].kind,
            StatementKind::Instruction {
                mnemonic: Mnemonic::Hlt,
                operands: vec![],
                short_jump: false,
                repeat: None,
            }
        );
    }

    #[test]
    fn stack_data_code_directives_produce_real_statements() {
        let (stmts, errors) = parse_program(".STACK 100h\n.DATA\n.CODE\nHLT");
        assert!(errors.is_empty(), "errors: {errors:?}");
        assert_eq!(stmts.len(), 4);
        assert_eq!(
            stmts[0].kind,
            StatementKind::Stack(crate::ast::ParsedExpr::Number(0x100))
        );
        assert_eq!(
            stmts[1].kind,
            StatementKind::SegmentSwitch(SegmentRole::Data)
        );
        assert_eq!(
            stmts[2].kind,
            StatementKind::SegmentSwitch(SegmentRole::Code)
        );
        assert_eq!(
            stmts[3].kind,
            StatementKind::Instruction {
                mnemonic: Mnemonic::Hlt,
                operands: vec![],
                short_jump: false,
                repeat: None,
            }
        );
    }

    #[test]
    fn blank_lines_and_comments_are_skipped() {
        let (stmts, errors) = parse_program("; a comment\n\nHLT\n; trailing comment\n");
        assert!(errors.is_empty());
        assert_eq!(stmts.len(), 1);
    }

    #[test]
    fn unrecognized_statement_is_a_recoverable_error() {
        let (stmts, errors) = parse_program("FROBNICATE AX\nHLT");
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].span.line, 1);
        // recovery: the next line still parses.
        assert_eq!(stmts.len(), 1);
        assert_eq!(
            stmts[0].kind,
            StatementKind::Instruction {
                mnemonic: Mnemonic::Hlt,
                operands: vec![],
                short_jump: false,
                repeat: None,
            }
        );
    }

    #[test]
    fn trailing_garbage_after_a_complete_statement_is_an_error() {
        let (_, errors) = parse_program("MOV AX, 5 6");
        assert_eq!(errors.len(), 1);
    }

    #[test]
    fn line_numbers_are_tracked_across_multiple_lines() {
        let (stmts, _) = parse_program("HLT\nHLT\nHLT");
        assert_eq!(stmts[0].line, 1);
        assert_eq!(stmts[1].line, 2);
        assert_eq!(stmts[2].line, 3);
    }

    #[test]
    fn name_endp_closes_a_proc_without_erroring() {
        let (stmts, errors) = parse_program("MAIN PROC\nHLT\nMAIN ENDP\nEND MAIN");
        assert!(errors.is_empty(), "errors: {errors:?}");
        // MAIN (label), HLT, END - the ENDP line produces nothing.
        assert_eq!(stmts.len(), 3);
        assert_eq!(stmts[0].kind, StatementKind::Label("MAIN".to_string()));
    }

    #[test]
    fn section_text_and_data_map_onto_segment_switch() {
        let (stmts, errors) = parse_program("SECTION .text\nHLT\nSECTION .data\nHLT");
        assert!(errors.is_empty(), "errors: {errors:?}");
        assert_eq!(
            stmts[0].kind,
            StatementKind::SegmentSwitch(SegmentRole::Code)
        );
        assert_eq!(
            stmts[2].kind,
            StatementKind::SegmentSwitch(SegmentRole::Data)
        );
    }

    #[test]
    fn section_with_an_unknown_name_is_an_error() {
        let (_, errors) = parse_program("SECTION .rodata");
        assert_eq!(errors.len(), 1);
    }

    #[test]
    fn named_times_directive_desugars_to_dup() {
        let (stmts, errors) = parse_program("dst times 32 db 0");
        assert!(errors.is_empty(), "errors: {errors:?}");
        assert_eq!(stmts.len(), 2);
        assert_eq!(stmts[0].kind, StatementKind::Label("dst".to_string()));
        assert_eq!(
            stmts[1].kind,
            StatementKind::Db(vec![DataItem::Dup {
                count: crate::ast::ParsedExpr::Number(32),
                item: Box::new(DataItem::Value(crate::ast::ParsedExpr::Number(0))),
            }])
        );
    }

    #[test]
    fn bare_times_directive_without_a_label_works() {
        let (stmts, errors) = parse_program("TIMES 4 DW 0");
        assert!(errors.is_empty(), "errors: {errors:?}");
        assert_eq!(stmts.len(), 1);
        assert_eq!(
            stmts[0].kind,
            StatementKind::Dw(vec![DataItem::Dup {
                count: crate::ast::ParsedExpr::Number(4),
                item: Box::new(DataItem::Value(crate::ast::ParsedExpr::Number(0))),
            }])
        );
    }

    #[test]
    fn rep_prefix_attaches_to_a_string_instruction() {
        let (stmts, errors) = parse_program("REP MOVSB");
        assert!(errors.is_empty(), "errors: {errors:?}");
        assert_eq!(
            stmts[0].kind,
            StatementKind::Instruction {
                mnemonic: Mnemonic::Movsb,
                operands: vec![],
                short_jump: false,
                repeat: Some(x8086_isa::Repeat::Rep),
            }
        );
    }

    #[test]
    fn repe_and_repne_prefixes_resolve_to_the_right_repeat_variant() {
        let (stmts, errors) = parse_program("REPE CMPSB\nREPNE SCASB");
        assert!(errors.is_empty(), "errors: {errors:?}");
        match &stmts[0].kind {
            StatementKind::Instruction { repeat, .. } => {
                assert_eq!(*repeat, Some(x8086_isa::Repeat::Repe))
            }
            other => panic!("expected an instruction, got {other:?}"),
        }
        match &stmts[1].kind {
            StatementKind::Instruction { repeat, .. } => {
                assert_eq!(*repeat, Some(x8086_isa::Repeat::Repne))
            }
            other => panic!("expected an instruction, got {other:?}"),
        }
    }

    #[test]
    fn rep_prefix_on_a_non_string_instruction_is_a_diagnostic() {
        let (_, errors) = parse_program("REP MOV AX, BX");
        assert_eq!(errors.len(), 1);
    }
}
