//! Parses a chain of number/symbol terms joined by `+`/`-` (no
//! registers) into a `ParsedExpr` - used for plain immediate operands,
//! `ORG`/`EQU` values, `DUP` counts, and numeric `DB`/`DW` items.

use crate::ast::ParsedExpr;
use crate::numbers::parse_number;
use crate::parse_error::ParseError;
use crate::{Token, TokenKind};

fn is_punct(tok: &Token, text: &str) -> bool {
    tok.kind == TokenKind::Punctuation && tok.text == text
}

/// One `Number` or `Symbol` term. Only consumes a single token - the
/// caller (`parse_expr_chain`) is what requires an explicit `+`/`-`
/// between terms, which matters: without that requirement, something
/// like `5 DUP` would misparse "DUP" as a symbol term to add.
fn parse_term(tokens: &[Token], pos: &mut usize, negate: bool) -> Result<ParsedExpr, ParseError> {
    let tok = tokens
        .get(*pos)
        .ok_or_else(|| ParseError::new(Default::default(), "expected a number or symbol"))?
        .clone();
    match tok.kind {
        TokenKind::Number => {
            let value = parse_number(&tok.text).ok_or_else(|| {
                ParseError::new(tok.span, format!("invalid numeric literal '{}'", tok.text))
            })?;
            *pos += 1;
            Ok(ParsedExpr::Number(if negate { -value } else { value }))
        }
        TokenKind::Identifier => {
            if negate {
                return Err(ParseError::new(tok.span, "cannot negate a symbol"));
            }
            *pos += 1;
            Ok(ParsedExpr::Symbol(tok.text))
        }
        _ => Err(ParseError::new(
            tok.span,
            format!("expected a number or symbol, found '{}'", tok.text),
        )),
    }
}

/// Parses one term, then as many `('+' | '-') term` pairs as follow it.
pub fn parse_expr_chain(tokens: &[Token], pos: &mut usize) -> Result<ParsedExpr, ParseError> {
    let mut expr = parse_term(tokens, pos, false)?;
    loop {
        match tokens.get(*pos) {
            Some(tok) if is_punct(tok, "+") => {
                *pos += 1;
                let term = parse_term(tokens, pos, false)?;
                expr = ParsedExpr::accumulate(Some(expr), term);
            }
            Some(tok) if is_punct(tok, "-") => {
                *pos += 1;
                let term = parse_term(tokens, pos, true)?;
                expr = ParsedExpr::accumulate(Some(expr), term);
            }
            _ => break,
        }
    }
    Ok(expr)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tokenize;

    fn expr_of(source: &str) -> ParsedExpr {
        let tokens = tokenize(source);
        let mut pos = 0;
        parse_expr_chain(&tokens, &mut pos).unwrap()
    }

    #[test]
    fn single_number() {
        assert_eq!(expr_of("42"), ParsedExpr::Number(42));
    }

    #[test]
    fn single_symbol() {
        assert_eq!(expr_of("myVar"), ParsedExpr::Symbol("myVar".to_string()));
    }

    #[test]
    fn symbol_plus_number_chain() {
        assert_eq!(
            expr_of("myVar+2"),
            ParsedExpr::Sum(
                Box::new(ParsedExpr::Symbol("myVar".to_string())),
                Box::new(ParsedExpr::Number(2))
            )
        );
    }

    #[test]
    fn minus_negates_the_following_number() {
        assert_eq!(
            expr_of("10-3"),
            ParsedExpr::Sum(
                Box::new(ParsedExpr::Number(10)),
                Box::new(ParsedExpr::Number(-3))
            )
        );
    }

    #[test]
    fn negating_a_symbol_is_an_error() {
        let tokens = tokenize("10-myVar");
        let mut pos = 0;
        assert!(parse_expr_chain(&tokens, &mut pos).is_err());
    }

    #[test]
    fn stops_before_an_unrelated_trailing_token() {
        let tokens = tokenize("42, 5");
        let mut pos = 0;
        let expr = parse_expr_chain(&tokens, &mut pos).unwrap();
        assert_eq!(expr, ParsedExpr::Number(42));
        assert_eq!(tokens[pos].text, ",");
    }

    #[test]
    fn does_not_swallow_a_following_identifier_without_an_operator() {
        // "5 DUP" must parse as just the number 5, leaving "DUP" for the
        // caller (DB/DW item parsing) to recognize - not "5 + DUP".
        let tokens = tokenize("5 dup");
        let mut pos = 0;
        let expr = parse_expr_chain(&tokens, &mut pos).unwrap();
        assert_eq!(expr, ParsedExpr::Number(5));
        assert_eq!(tokens[pos].text, "dup");
    }
}
