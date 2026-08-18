//! emu8086-syntax lexer, parser, and two-pass assembler.
//!
//! `tokenize()` is deliberately its own standalone function (not buried
//! inside the parser): the SwiftUI editor calls it directly for syntax
//! highlighting, so the editor's notion of "this is a register / number /
//! string" can never drift from what the assembler itself considers valid.
//! Full directive parsing and two-pass codegen (`assemble()`) land during
//! the assembler build phase; this scaffold wires up the lexer plus the
//! result/diagnostic shapes the rest of the system will depend on.

pub mod ast;
mod codegen;
pub use codegen::SymbolKind;
mod encoder;
mod expr_parser;
mod mnemonics;
mod numbers;
mod operand_parser;
mod parse_error;
mod statement_parser;

use std::collections::BTreeMap;

const REGISTER_NAMES: &[&str] = &[
    "ax", "bx", "cx", "dx", "sp", "bp", "si", "di", "cs", "ds", "es", "ss", "ip", "al", "ah", "bl",
    "bh", "cl", "ch", "dl", "dh",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Span {
    pub line: u32,
    pub col: u32,
    pub byte_offset: u32,
    pub len: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenKind {
    Identifier,
    Register,
    Number,
    StringLiteral,
    Comment,
    Punctuation,
    Newline,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Token {
    pub kind: TokenKind,
    pub text: String,
    pub span: Span,
}

/// Tokenize emu8086-syntax source into a flat token stream. Never fails:
/// unrecognized characters simply become single-character `Punctuation`
/// tokens, since a highlighter must handle every keystroke, including
/// invalid ones mid-edit.
pub fn tokenize(source: &str) -> Vec<Token> {
    let mut tokens = Vec::new();
    let bytes = source.as_bytes();
    let mut i = 0usize;
    let mut line = 1u32;
    let mut col = 1u32;

    let push = |tokens: &mut Vec<Token>,
                kind: TokenKind,
                text: String,
                line: u32,
                col: u32,
                byte_offset: u32| {
        let len = text.len() as u32;
        tokens.push(Token {
            kind,
            text,
            span: Span {
                line,
                col,
                byte_offset,
                len,
            },
        });
    };

    while i < bytes.len() {
        let c = bytes[i] as char;
        let start_col = col;
        let start_offset = i as u32;

        if c == '\n' {
            push(
                &mut tokens,
                TokenKind::Newline,
                "\n".to_string(),
                line,
                start_col,
                start_offset,
            );
            i += 1;
            line += 1;
            col = 1;
            continue;
        }
        if c.is_whitespace() {
            i += 1;
            col += 1;
            continue;
        }
        if c == ';' {
            let start = i;
            while i < bytes.len() && bytes[i] as char != '\n' {
                i += 1;
            }
            let text = source[start..i].to_string();
            col += text.chars().count() as u32;
            push(
                &mut tokens,
                TokenKind::Comment,
                text,
                line,
                start_col,
                start_offset,
            );
            continue;
        }
        if c == '"' || c == '\'' {
            let quote = c;
            let start = i;
            i += 1;
            while i < bytes.len() && bytes[i] as char != quote {
                i += 1;
            }
            if i < bytes.len() {
                i += 1; // consume closing quote
            }
            let text = source[start..i].to_string();
            col += text.chars().count() as u32;
            push(
                &mut tokens,
                TokenKind::StringLiteral,
                text,
                line,
                start_col,
                start_offset,
            );
            continue;
        }
        if c.is_ascii_digit() {
            let start = i;
            while i < bytes.len() && (bytes[i] as char).is_ascii_alphanumeric() {
                i += 1;
            }
            let text = source[start..i].to_string();
            col += text.chars().count() as u32;
            push(
                &mut tokens,
                TokenKind::Number,
                text,
                line,
                start_col,
                start_offset,
            );
            continue;
        }
        if c.is_ascii_alphabetic() || c == '_' || c == '.' || c == '@' {
            let start = i;
            // Unconditionally consume the starting character: '.' and
            // '@' are valid identifier starts but not valid continuation
            // characters, so a while-loop condition alone would never
            // advance past them, hanging forever on input like ".MODEL".
            i += 1;
            while i < bytes.len() && {
                let ch = bytes[i] as char;
                ch.is_ascii_alphanumeric() || ch == '_'
            } {
                i += 1;
            }
            let text = source[start..i].to_string();
            col += text.chars().count() as u32;
            let kind = if REGISTER_NAMES.contains(&text.to_ascii_lowercase().as_str()) {
                TokenKind::Register
            } else {
                TokenKind::Identifier
            };
            push(&mut tokens, kind, text, line, start_col, start_offset);
            continue;
        }

        // Single-character punctuation (`,`, `:`, `[`, `]`, `+`, `-`, etc.)
        push(
            &mut tokens,
            TokenKind::Punctuation,
            c.to_string(),
            line,
            start_col,
            start_offset,
        );
        i += 1;
        col += 1;
    }

    tokens
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    pub line: u32,
    pub col: u32,
    pub severity: Severity,
    pub message: String,
}

/// A resolved symbol (label, `EQU` constant, or data variable), exposed
/// for the Variables/Watch panel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SymbolTableEntry {
    pub name: String,
    pub value: i64,
    pub kind: SymbolKind,
}

/// The result of assembling a source file: the encoded program, a
/// source-line <-> address map (for breakpoints and step highlighting),
/// and any diagnostics collected across both passes.
#[derive(Debug, Clone, Default)]
pub struct AssembleResult {
    pub machine_code: Vec<u8>,
    pub entry_point: u32,
    pub line_to_address: BTreeMap<u32, u32>,
    pub diagnostics: Vec<Diagnostic>,
    pub symbols: Vec<SymbolTableEntry>,
    /// The data segment's paragraph-aligned base address, if `.DATA` was
    /// used anywhere in the source. `None` means the program never
    /// declared one, in which case a caller (see `x8086-emulator`'s
    /// loader) must leave DS untouched rather than pointing it somewhere
    /// meaningless.
    pub data_segment_base: Option<u32>,
    /// The stack segment's paragraph-aligned base address and requested
    /// size in bytes, if `.STACK <size>` was used. `None` means no
    /// stack was declared, in which case a caller must leave SS/SP
    /// untouched.
    pub stack_segment: Option<(u32, u32)>,
}

/// Two-pass assemble of emu8086-syntax source: lex -> parse -> resolve
/// symbols/addresses -> encode. See `codegen` for the pass structure.
pub fn assemble(source: &str) -> AssembleResult {
    codegen::assemble(source)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokenizes_a_mov_instruction() {
        let tokens = tokenize("MOV AX, 5");
        let kinds: Vec<_> = tokens.iter().map(|t| t.kind).collect();
        assert_eq!(
            kinds,
            vec![
                TokenKind::Identifier,  // "MOV" - mnemonic classification is the parser's job
                TokenKind::Register,    // "AX"
                TokenKind::Punctuation, // ","
                TokenKind::Number,      // "5"
            ]
        );
        assert_eq!(tokens[0].text, "MOV".to_string());
    }

    #[test]
    fn identifiers_starting_with_dot_or_at_terminate_and_tokenize_correctly() {
        // Regression test: '.' and '@' are valid identifier *starts*
        // (".MODEL", "@@local") but not valid continuation characters,
        // so the identifier scanner must unconditionally consume the
        // start character before checking the continuation predicate -
        // otherwise it never advances and tokenize() loops forever.
        let tokens = tokenize(".MODEL SMALL");
        assert_eq!(tokens[0].kind, TokenKind::Identifier);
        assert_eq!(tokens[0].text, ".MODEL");
        assert_eq!(tokens[1].text, "SMALL");
    }

    #[test]
    fn register_names_are_case_insensitive() {
        // tokens: "mov"(0) "ax"(1) ","(2) "bx"(3)
        let tokens = tokenize("mov ax, bx");
        assert_eq!(tokens[1].kind, TokenKind::Register);
        assert_eq!(tokens[3].kind, TokenKind::Register);
    }

    #[test]
    fn non_register_words_are_plain_identifiers() {
        // The lexer only classifies register names; mnemonics, labels, and
        // variable names are all `Identifier` until the parser (a later
        // phase) resolves what role each one plays.
        let tokens = tokenize("MOV counter, 5");
        assert_eq!(tokens[0].kind, TokenKind::Identifier); // "MOV"
        assert_eq!(tokens[1].kind, TokenKind::Identifier); // "counter"
    }

    #[test]
    fn comment_runs_to_end_of_line() {
        let tokens = tokenize("MOV AX, 5 ; load count\nHLT");
        let comment = tokens
            .iter()
            .find(|t| t.kind == TokenKind::Comment)
            .unwrap();
        assert_eq!(comment.text, "; load count");
    }

    #[test]
    fn string_literal_captures_quotes() {
        let tokens = tokenize("DB \"hi$\"");
        let s = tokens
            .iter()
            .find(|t| t.kind == TokenKind::StringLiteral)
            .unwrap();
        assert_eq!(s.text, "\"hi$\"");
    }

    #[test]
    fn tracks_line_and_column_across_newlines() {
        let tokens = tokenize("MOV\nHLT");
        let hlt = tokens.iter().find(|t| t.text == "HLT").unwrap();
        assert_eq!(hlt.span.line, 2);
        assert_eq!(hlt.span.col, 1);
    }

    #[test]
    fn assemble_encodes_a_simple_instruction() {
        let result = assemble("MOV AX, 5");
        assert!(result.diagnostics.is_empty());
        assert_eq!(result.machine_code, vec![0xB8, 0x05, 0x00]);
    }
}
