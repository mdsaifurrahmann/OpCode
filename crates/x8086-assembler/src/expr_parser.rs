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

/// Consumes any run of leading `+`/`-` signs, reporting whether the net
/// effect is a negation. This is the sign *of* a term (`-10`, `+5`), a
/// different thing from the binary `+`/`-` between two terms that
/// `parse_expr_chain` handles - the two can legitimately sit next to
/// each other (`10 - -3`), so a term has to be able to carry its own
/// sign rather than relying on the separator alone.
fn consume_sign(tokens: &[Token], pos: &mut usize) -> bool {
    let mut negate = false;
    while let Some(tok) = tokens.get(*pos) {
        if is_punct(tok, "-") {
            negate = !negate;
        } else if !is_punct(tok, "+") {
            break;
        }
        *pos += 1;
    }
    negate
}

/// One optionally-signed `Number` or `Symbol` term (including the `$`
/// pseudo-symbol). Past the sign it consumes a single token - the caller
/// (`parse_expr_chain`) is what requires an explicit `+`/`-` *between*
/// terms, which matters: without that requirement, something like
/// `5 DUP` would misparse "DUP" as a symbol term to add.
fn parse_term(tokens: &[Token], pos: &mut usize) -> Result<ParsedExpr, ParseError> {
    let negate = consume_sign(tokens, pos);
    let tok = tokens
        .get(*pos)
        .ok_or_else(|| ParseError::new(Default::default(), "expected a number or symbol"))?
        .clone();
    // A symbol's value isn't known until resolution, so it can't be
    // pre-negated the way a literal can - `0 - sym` expresses the
    // negation with nodes that already exist rather than adding a
    // unary-negate variant the evaluator would have to learn.
    let negated = |expr: ParsedExpr| -> ParsedExpr {
        if negate {
            ParsedExpr::Diff(Box::new(ParsedExpr::Number(0)), Box::new(expr))
        } else {
            expr
        }
    };
    match tok.kind {
        TokenKind::Number => {
            let value = parse_number(&tok.text).ok_or_else(|| {
                ParseError::new(tok.span, format!("invalid numeric literal '{}'", tok.text))
            })?;
            *pos += 1;
            Ok(ParsedExpr::Number(if negate { -value } else { value }))
        }
        TokenKind::Identifier => {
            *pos += 1;
            Ok(negated(ParsedExpr::Symbol(tok.text)))
        }
        // NASM's `$` - "the address of this line" - resolved the same way
        // `@DATA` is: as a reserved pseudo-symbol name, evaluated by
        // `codegen::eval_expr` against the current location counter
        // rather than a real symbol-table entry.
        TokenKind::Punctuation if tok.text == "$" => {
            *pos += 1;
            Ok(negated(ParsedExpr::Symbol("$".to_string())))
        }
        _ => Err(ParseError::new(
            tok.span,
            format!("expected a number or symbol, found '{}'", tok.text),
        )),
    }
}

/// Parses one term, then as many `('+' | '-') term` pairs as follow it.
/// Subtracting a plain number folds the negation into a compact
/// `Number(-n)` term (via `Sum`); subtracting anything else (a symbol,
/// `$`) produces a real `Diff` node, since its value isn't known until
/// resolution and so can't be pre-negated the way a literal can.
pub fn parse_expr_chain(tokens: &[Token], pos: &mut usize) -> Result<ParsedExpr, ParseError> {
    let mut expr = parse_term(tokens, pos)?;
    loop {
        match tokens.get(*pos) {
            Some(tok) if is_punct(tok, "+") => {
                *pos += 1;
                let term = parse_term(tokens, pos)?;
                expr = ParsedExpr::accumulate(Some(expr), term);
            }
            Some(tok) if is_punct(tok, "-") => {
                *pos += 1;
                let term = parse_term(tokens, pos)?;
                expr = match term {
                    ParsedExpr::Number(n) => {
                        ParsedExpr::accumulate(Some(expr), ParsedExpr::Number(-n))
                    }
                    other => ParsedExpr::Diff(Box::new(expr), Box::new(other)),
                };
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
    fn subtracting_a_symbol_produces_a_diff_node() {
        // Unlike subtracting a literal number, this can't be folded into
        // a negated `Number` term - the symbol's value isn't known until
        // resolution - so it needs real `Diff` representation.
        assert_eq!(
            expr_of("10-myVar"),
            ParsedExpr::Diff(
                Box::new(ParsedExpr::Number(10)),
                Box::new(ParsedExpr::Symbol("myVar".to_string()))
            )
        );
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
    fn dollar_parses_as_a_pseudo_symbol() {
        assert_eq!(expr_of("$"), ParsedExpr::Symbol("$".to_string()));
        // The classic NASM "string length" idiom: `$ - label - 1`.
        // `$ - msg` isn't foldable (msg's value is unknown until
        // resolution) so it becomes a `Diff`; the trailing `- 1` *is* a
        // literal, so it folds into a negated `Number` via `Sum`.
        assert_eq!(
            expr_of("$-msg-1"),
            ParsedExpr::Sum(
                Box::new(ParsedExpr::Diff(
                    Box::new(ParsedExpr::Symbol("$".to_string())),
                    Box::new(ParsedExpr::Symbol("msg".to_string())),
                )),
                Box::new(ParsedExpr::Number(-1)),
            )
        );
    }

    #[test]
    fn subtracting_dollar_also_produces_a_diff_node() {
        assert_eq!(
            expr_of("10-$"),
            ParsedExpr::Diff(
                Box::new(ParsedExpr::Number(10)),
                Box::new(ParsedExpr::Symbol("$".to_string()))
            )
        );
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

    #[test]
    fn a_leading_minus_negates_the_literal() {
        // `MOV BX, -10` - previously rejected outright ("unexpected
        // token '-' in operand"), since only the binary minus *between*
        // two terms was recognized.
        assert_eq!(expr_of("-10"), ParsedExpr::Number(-10));
    }

    #[test]
    fn a_leading_plus_is_accepted_and_changes_nothing() {
        assert_eq!(expr_of("+5"), ParsedExpr::Number(5));
    }

    #[test]
    fn repeated_signs_cancel() {
        assert_eq!(expr_of("--7"), ParsedExpr::Number(7));
        assert_eq!(expr_of("-+-7"), ParsedExpr::Number(7));
        assert_eq!(expr_of("---7"), ParsedExpr::Number(-7));
    }

    #[test]
    fn a_binary_minus_can_be_followed_by_a_signed_term() {
        // `10 - -3` = 13. The separator and the term's own sign are
        // different things, and both have to be allowed at once.
        assert_eq!(
            expr_of("10 - -3"),
            ParsedExpr::Sum(
                Box::new(ParsedExpr::Number(10)),
                Box::new(ParsedExpr::Number(3))
            )
        );
    }

    #[test]
    fn negating_a_symbol_becomes_a_subtraction_from_zero() {
        // A symbol's value isn't known until resolution, so unlike a
        // literal it can't be pre-negated - `0 - sym` expresses it with
        // nodes the evaluator already understands.
        assert_eq!(
            expr_of("-myVar"),
            ParsedExpr::Diff(
                Box::new(ParsedExpr::Number(0)),
                Box::new(ParsedExpr::Symbol("myVar".to_string()))
            )
        );
    }

    #[test]
    fn a_sign_with_nothing_after_it_is_still_an_error() {
        let tokens = tokenize("-");
        let mut pos = 0;
        assert!(parse_expr_chain(&tokens, &mut pos).is_err());
    }
}
