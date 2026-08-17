//! The parsed (pre-symbol-resolution) statement model. `x8086_isa`
//! operands carry fully-resolved values; these `Parsed*` types exist
//! because an operand can reference a symbol (`myLabel`, `myVar`) whose
//! numeric value isn't known until the two-pass resolver has walked the
//! whole program.

use x8086_isa::{Mnemonic, Reg16, Reg8, Width};

#[derive(Debug, Clone, PartialEq)]
pub enum ParsedExpr {
    Number(i64),
    Symbol(String),
    /// `a+b` (also used to represent `a-b`, by folding the negation into
    /// a `Number` term before combining) - lets `[myArray+SI+2]`-style
    /// chains of symbol/number terms resolve without a full expression
    /// grammar.
    Sum(Box<ParsedExpr>, Box<ParsedExpr>),
}

impl ParsedExpr {
    fn add(self, other: ParsedExpr) -> ParsedExpr {
        ParsedExpr::Sum(Box::new(self), Box::new(other))
    }

    /// Fold `new` into `existing` (`None` on the first term), building up
    /// a left-associative `Sum` chain term by term.
    pub fn accumulate(existing: Option<ParsedExpr>, new: ParsedExpr) -> ParsedExpr {
        match existing {
            None => new,
            Some(expr) => expr.add(new),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum ParsedOperand {
    Reg16(Reg16),
    Reg8(Reg8),
    /// An immediate value, or (for branch mnemonics) a jump/call target.
    Immediate(ParsedExpr),
    Memory {
        size_override: Option<Width>,
        segment_override: Option<Reg16>,
        base: Option<Reg16>,
        index: Option<Reg16>,
        displacement: Option<ParsedExpr>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum DataItem {
    Value(ParsedExpr),
    /// `DB "text"` - each character becomes one byte.
    Str(String),
    /// `?` - reserves space without a defined initial value (we still
    /// zero-fill it, since our memory model has no concept of
    /// "uninitialized").
    Uninitialized,
    Dup {
        count: ParsedExpr,
        item: Box<DataItem>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum StatementKind {
    Label(String),
    Org(ParsedExpr),
    Equ {
        name: String,
        value: ParsedExpr,
    },
    Db(Vec<DataItem>),
    Dw(Vec<DataItem>),
    Instruction {
        mnemonic: Mnemonic,
        operands: Vec<ParsedOperand>,
        /// Set when the source explicitly wrote `JMP SHORT target`,
        /// requesting the 2-byte rel8 encoding instead of the default
        /// 3-byte near form. Meaningless for any mnemonic other than
        /// `JMP` (which is the only one with both a short and a near
        /// direct-jump encoding).
        short_jump: bool,
    },
    /// `END [label]` - marks the logical end of the program and
    /// optionally names the entry point.
    End(Option<String>),
    /// Structural directives kept only for source compatibility with
    /// real emu8086/MASM-style programs (`.MODEL`, `.STACK`, `.DATA`,
    /// `.CODE`, `SEGMENT`, `ENDS`, `ASSUME`, the `NEAR`/`FAR` qualifier
    /// on `PROC`). Our CPU/memory model is flat and single-segment, so
    /// these carry no runtime effect - they're recognized rather than
    /// rejected purely so realistic source files parse. (`PROC` itself
    /// becomes a `Label`, since it also defines a callable symbol.)
    NoOp,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Statement {
    pub kind: StatementKind,
    pub line: u32,
}
