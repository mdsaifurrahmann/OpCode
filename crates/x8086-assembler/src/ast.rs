//! The parsed (pre-symbol-resolution) statement model. `x8086_isa`
//! operands carry fully-resolved values; these `Parsed*` types exist
//! because an operand can reference a symbol (`myLabel`, `myVar`) whose
//! numeric value isn't known until the two-pass resolver has walked the
//! whole program.

use x8086_isa::{Mnemonic, Reg16, Reg8, Repeat, Width};

#[derive(Debug, Clone, PartialEq)]
pub enum ParsedExpr {
    Number(i64),
    /// A named symbol, or the reserved pseudo-symbol `"$"` (NASM's
    /// "address of this line," resolved by `codegen::eval_expr` against
    /// the current location counter rather than a real symbol-table
    /// entry - the same trick `"@data"` uses).
    Symbol(String),
    /// `a+b` - subtracting a *number* folds the negation into a `Number`
    /// term instead of using `Diff` (so `x-5` keeps the same compact
    /// shape it always has), but that trick only works when the value
    /// being subtracted is already known at parse time.
    Sum(Box<ParsedExpr>, Box<ParsedExpr>),
    /// `a-b` where `b` isn't a plain number (e.g. `$-msg`, subtracting
    /// one symbol's address from another to compute a length) - real
    /// subtraction, since a symbol's value isn't known until resolution
    /// and so can't be pre-negated the way `Sum` folds a literal number.
    Diff(Box<ParsedExpr>, Box<ParsedExpr>),
    /// `OFFSET expr` - explicitly requests *the address* of whatever
    /// `expr` names, unconditionally, regardless of what a bare
    /// reference to it would otherwise mean. This has to be a distinct
    /// node (not just parsed and discarded): a bare `DB`/`DW` variable
    /// reference means different things in different dialects (see
    /// `codegen::resolve_operand`) - MASM/emu8086 dereferences it,
    /// pre-`SECTION`-detection NASM mode treats it as an address - but
    /// `OFFSET` must always mean "the address" in every dialect, so it
    /// needs to survive as its own marker all the way to
    /// `resolve_operand`, which special-cases it ahead of that
    /// dialect-dependent branching.
    Offset(Box<ParsedExpr>),
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

/// Which segment subsequent statements belong to, as switched by `.DATA`/
/// `.CODE`. Every statement implicitly has a role (see
/// `codegen::prescan_segment_roles`) - a program that never uses `.DATA`/
/// `.CODE` at all has every statement in `Code`, which is exactly today's
/// flat, single-region layout (code segment base is always 0), so this
/// is purely additive: it changes nothing for programs that don't opt in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SegmentRole {
    Code,
    Data,
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
        /// Set when the source prefixed a string instruction with
        /// `REP`/`REPE`/`REPZ`/`REPNE`/`REPNZ`. `None` for every other
        /// mnemonic and for a non-repeated string instruction.
        repeat: Option<Repeat>,
    },
    /// `END [label]` - marks the logical end of the program and
    /// optionally names the entry point.
    End(Option<String>),
    /// `.STACK <size>` - the one structural directive that carries real
    /// data: how large a stack segment to reserve. See `codegen` for how
    /// this becomes SS:SP at load time.
    Stack(ParsedExpr),
    /// `.DATA` / `.CODE` - switches which segment subsequent statements
    /// belong to.
    SegmentSwitch(SegmentRole),
    /// Structural directives kept only for source compatibility with
    /// real emu8086/MASM-style programs (`.MODEL`, `SEGMENT`, `ENDS`,
    /// `ASSUME`, the `NEAR`/`FAR` qualifier on `PROC`). Genuine
    /// multi-segment `SEGMENT`/`ENDS` layout (as opposed to the
    /// simplified `.STACK`/`.DATA`/`.CODE` directives above, which do
    /// get real effect) is not modeled yet - these are recognized rather
    /// than rejected purely so realistic source files parse. (`PROC`
    /// itself becomes a `Label`, since it also defines a callable
    /// symbol.)
    NoOp,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Statement {
    pub kind: StatementKind,
    pub line: u32,
}
