//! The two-pass driver: turns parsed `Statement`s into a final
//! `AssembleResult` (machine code, line map, diagnostics, symbol table).
//!
//! Pass 1 walks the statements once, building the symbol table (labels/
//! `EQU`) and each instruction's *encoded length* - which never depends
//! on a symbol's actual resolved value (branch instructions use a fixed
//! length per their syntactic form - see `branch_fixed_length` - and
//! immediate sizes are fixed by operand width), only on its syntactic
//! shape. That means pass 1 can resolve operands leniently (forward
//! references default to a `0` placeholder) and still get correct
//! lengths, which is what makes a second, fully-resolved pass possible
//! at all. Any error a lenient pass 1 resolution hits is *not* reported
//! yet, to avoid duplicate diagnostics - pass 2 (strict, with the
//! now-complete symbol table) is the one that reports real errors.
//!
//! Pass 2 re-walks the statements with the address of each already
//! known (from pass 1's lengths), resolving every operand strictly and
//! encoding for real.
//!
//! ## Segments
//!
//! Every statement has a `SegmentRole` (`Code` or `Data`), assigned by
//! `prescan_segment_roles` from `.DATA`/`.CODE` switches and defaulting
//! to `Code` throughout for a program that never uses them - which
//! reproduces exactly the old single flat region, so this is purely
//! additive. Pass 1 tracks two independent location counters
//! (`code_location`, `data_location`), so `SymbolTableEntry.value` for a
//! `Data`-kind symbol is an offset *relative to the data segment*, not a
//! flat address - matching what a `MOV`/`LEA` direct-address operand
//! actually encodes on real 8086 hardware (just the offset; the segment
//! is implicit via whatever's in DS at runtime). The data segment's real
//! placement (paragraph-aligned, right after the code segment) is only
//! computed after pass 1, once the code segment's total size is known.
//!
//! `line_to_address` is the one exception to "relative": it stays a true
//! flat address for every line, code or data, so existing consumers
//! (breakpoints, disassembly, current-line highlighting) need no changes.

use std::collections::{BTreeMap, BTreeSet};

use crate::ast::{DataItem, ParsedExpr, ParsedOperand, SegmentRole, Statement, StatementKind};
use crate::encoder::{condition_index, encode_one};
use crate::parse_error::ParseError;
use crate::statement_parser::parse_program;
use crate::{AssembleResult, Diagnostic, Severity, SymbolTableEntry};
use x8086_isa::{Condition, Instruction, Mnemonic, Operand, Repeat, Width};

/// Segment bases must be paragraph-aligned (a multiple of 16 bytes),
/// matching real 8086 segment:offset addressing (`physical = segment*16
/// + offset`).
const PARAGRAPH_SIZE: u32 = 16;

fn round_up_to_paragraph(n: u32) -> u32 {
    (n + PARAGRAPH_SIZE - 1) & !(PARAGRAPH_SIZE - 1)
}

/// A resolved symbol's role, also surfaced on `SymbolTableEntry` for a
/// Variables/Watch panel: `DataByte`/`DataWord` tell the panel how many
/// bytes to read back from memory at the symbol's address, while
/// `Constant`/`Label` are plain numeric values with no memory behind them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SymbolKind {
    /// Defined via `EQU` - a compile-time constant, never a memory
    /// reference even when used bare.
    Constant,
    /// A code label (`name:`) or `PROC` name - used as a branch target,
    /// or as a raw address value if used bare in a non-branch context.
    Label,
    /// A `DB` variable name - a bare reference means "the byte stored at
    /// this address", matching MASM/emu8086 convention.
    DataByte,
    /// A `DW` variable name - same convention as `DataByte`, but the
    /// bare reference is a 2-byte value.
    DataWord,
}

impl SymbolKind {
    fn is_data(self) -> bool {
        matches!(self, SymbolKind::DataByte | SymbolKind::DataWord)
    }
}

#[derive(Debug, Clone, Copy)]
struct SymbolEntry {
    value: i64,
    kind: SymbolKind,
}

type SymbolTable = BTreeMap<String, SymbolEntry>;

pub fn assemble(source: &str) -> AssembleResult {
    let (statements, parse_errors) = parse_program(source);
    let mut diagnostics: Vec<Diagnostic> =
        parse_errors.iter().map(parse_error_to_diagnostic).collect();

    // MASM/emu8086 and NASM disagree, flatly, on what a *bare* reference
    // to a `DB`/`DW` variable means: MASM/emu8086 dereferences it ("the
    // value stored there"), NASM treats it as that variable's address
    // (same as a bare label - you write `[var]` to dereference). Since
    // the same source text means opposite things depending on dialect,
    // one file has to pick one - `SECTION` is NASM's own, unambiguous
    // signal for which (real MASM/emu8086 source never uses it), so its
    // presence anywhere in the file switches `resolve_operand` over to
    // NASM's convention for the whole file.
    let is_nasm_dialect = crate::tokenize(source)
        .iter()
        .any(|t| t.kind == crate::TokenKind::Identifier && t.text.eq_ignore_ascii_case("section"));

    // A symbol's *kind* (Data vs. Label/Constant) only depends on where
    // it's declared in the source, never on an address - so it can be
    // determined in one pure syntax pass, independent of pass 1's
    // address bookkeeping. This matters: pass 1 needs it for *forward*
    // references (e.g. `LEA DX, msg` appearing before `msg DB ...`), to
    // resolve them with the right operand *shape* (Memory vs Immediate)
    // even though the real address isn't known yet - getting the shape
    // wrong would make the encoded length wrong too (LEA requires a
    // Memory operand; an Immediate placeholder makes it fail to encode
    // at all), corrupting every address that follows.
    let symbol_kinds = prescan_symbol_kinds(&statements);
    let roles = prescan_segment_roles(&statements);
    let long_jcc = resolve_long_jcc_statements(&statements, &roles, &symbol_kinds, is_nasm_dialect);

    let (symbols, code_segment_size, data_segment_size, _) = pass_one(
        &statements,
        &roles,
        &symbol_kinds,
        &long_jcc,
        is_nasm_dialect,
        &mut diagnostics,
    );

    let data_segment_base = round_up_to_paragraph(code_segment_size);
    let stack_segment_base = round_up_to_paragraph(data_segment_base + data_segment_size);
    let has_data_segment = data_segment_size > 0
        || statements
            .iter()
            .any(|s| matches!(s.kind, StatementKind::SegmentSwitch(SegmentRole::Data)));

    let (machine_code, line_to_address, entry_point, stack_size) = pass_two(
        &statements,
        &symbols,
        &symbol_kinds,
        &roles,
        data_segment_base,
        &long_jcc,
        is_nasm_dialect,
        &mut diagnostics,
    );

    let mut symbol_entries: Vec<SymbolTableEntry> = symbols
        .into_iter()
        .map(|(name, entry)| SymbolTableEntry {
            name,
            value: entry.value,
            kind: entry.kind,
        })
        .collect();
    symbol_entries.sort_by(|a, b| a.name.cmp(&b.name));

    AssembleResult {
        machine_code,
        entry_point,
        line_to_address,
        diagnostics,
        symbols: symbol_entries,
        data_segment_base: has_data_segment.then_some(data_segment_base),
        stack_segment: stack_size.map(|size| (stack_segment_base, size)),
    }
}

/// One pure syntax pass to learn every symbol's kind ahead of pass 1 -
/// see the comment in `assemble` for why this needs to be separate from
/// (and run before) address computation.
fn prescan_symbol_kinds(statements: &[Statement]) -> BTreeMap<String, SymbolKind> {
    let mut kinds = BTreeMap::new();
    for (i, stmt) in statements.iter().enumerate() {
        if let StatementKind::Label(name) = &stmt.kind {
            let kind = match statements.get(i + 1).map(|s| &s.kind) {
                Some(StatementKind::Db(_)) => SymbolKind::DataByte,
                Some(StatementKind::Dw(_)) => SymbolKind::DataWord,
                _ => SymbolKind::Label,
            };
            kinds.insert(name.clone(), kind);
        }
    }
    kinds
}

/// Which segment (`Code` or `Data`) each statement belongs to, driven by
/// `.DATA`/`.CODE` switches. Defaults to (and for a program that never
/// uses these directives, stays) `Code` for every statement, exactly
/// reproducing the old single flat region.
fn prescan_segment_roles(statements: &[Statement]) -> Vec<SegmentRole> {
    let mut roles = Vec::with_capacity(statements.len());
    let mut current = SegmentRole::Code;
    for stmt in statements {
        if let StatementKind::SegmentSwitch(role) = &stmt.kind {
            current = *role;
        }
        roles.push(current);
    }
    roles
}

// --- pass 1: symbol table + segment sizes -----------------------------------

/// Returns the symbol table (values relative to their own segment - see
/// the module docs), the final size of each segment, and each
/// statement's own start address (parallel to `statements` - used by
/// `resolve_long_jcc_statements` to check real branch distances once a
/// full symbol table exists).
#[allow(clippy::too_many_arguments)]
fn pass_one(
    statements: &[Statement],
    roles: &[SegmentRole],
    symbol_kinds: &BTreeMap<String, SymbolKind>,
    long_jcc: &BTreeSet<usize>,
    is_nasm_dialect: bool,
    diagnostics: &mut Vec<Diagnostic>,
) -> (SymbolTable, u32, u32, Vec<u32>) {
    let mut symbols = SymbolTable::new();
    let mut code_location: u32 = 0;
    let mut data_location: u32 = 0;
    let mut statement_addresses: Vec<u32> = Vec::with_capacity(statements.len());

    for (i, stmt) in statements.iter().enumerate() {
        let role = roles[i];
        let location_counter = match role {
            SegmentRole::Code => code_location,
            SegmentRole::Data => data_location,
        };
        statement_addresses.push(location_counter);
        match &stmt.kind {
            StatementKind::Label(name) => {
                let kind = match statements.get(i + 1).map(|s| &s.kind) {
                    Some(StatementKind::Db(_)) => SymbolKind::DataByte,
                    Some(StatementKind::Dw(_)) => SymbolKind::DataWord,
                    _ => SymbolKind::Label,
                };
                symbols.insert(
                    name.clone(),
                    SymbolEntry {
                        value: location_counter as i64,
                        kind,
                    },
                );
            }
            StatementKind::Org(expr) => {
                match eval_expr(expr, &symbols, 0, location_counter as i64) {
                    Ok(value) => match role {
                        SegmentRole::Code => code_location = value as u32,
                        SegmentRole::Data => data_location = value as u32,
                    },
                    Err(msg) => diagnostics.push(diag_error(stmt.line, msg)),
                }
            }
            StatementKind::Equ { name, value } => {
                match eval_expr(value, &symbols, 0, location_counter as i64) {
                    Ok(v) => {
                        symbols.insert(
                            name.clone(),
                            SymbolEntry {
                                value: v,
                                kind: SymbolKind::Constant,
                            },
                        );
                    }
                    Err(msg) => diagnostics.push(diag_error(stmt.line, msg)),
                }
            }
            StatementKind::Db(items) => {
                let len = data_items_len(items, 1, &symbols);
                match role {
                    SegmentRole::Code => code_location += len,
                    SegmentRole::Data => data_location += len,
                }
            }
            StatementKind::Dw(items) => {
                let len = data_items_len(items, 2, &symbols);
                match role {
                    SegmentRole::Code => code_location += len,
                    SegmentRole::Data => data_location += len,
                }
            }
            StatementKind::End(_)
            | StatementKind::NoOp
            | StatementKind::Stack(_)
            | StatementKind::SegmentSwitch(_) => {}
            StatementKind::Instruction {
                mnemonic,
                operands,
                short_jump,
                repeat,
            } => {
                let len = if is_branch_mnemonic(*mnemonic) {
                    // Every branch's length is fixed purely by its
                    // syntactic form (see `branch_fixed_length`) - it
                    // must never be derived by actually trying to encode
                    // a lenient/placeholder target, since a *forward*
                    // reference's placeholder value (0) can easily land
                    // outside a short Jcc/LOOP's rel8 range even when
                    // the real, final target won't - silently swallowing
                    // that failure (as the generic path below does) would
                    // contribute zero bytes for the statement and corrupt
                    // every address after it. Whether a `Jcc` needs the
                    // 5-byte invert+JMP expansion was already decided,
                    // correctly, by `resolve_long_jcc_statements` using
                    // the real (not placeholder) final distance.
                    if matches!(mnemonic, Mnemonic::Jcc(_)) && long_jcc.contains(&i) {
                        5
                    } else {
                        branch_fixed_length(*mnemonic, *short_jump) as u32
                    }
                } else if let Ok(instr) = resolve_instruction(
                    *mnemonic,
                    operands,
                    *short_jump,
                    *repeat,
                    &symbols,
                    symbol_kinds,
                    location_counter,
                    false,
                    0,
                    is_nasm_dialect,
                ) {
                    encode_one(&instr).map(|b| b.len() as u32).unwrap_or(0)
                } else {
                    0
                };
                match role {
                    SegmentRole::Code => code_location += len,
                    SegmentRole::Data => data_location += len,
                }
                // A non-branch resolution/encoding failure above is
                // deliberately swallowed - pass 2 re-resolves strictly
                // and reports the real diagnostic.
            }
        }
    }

    (symbols, code_location, data_location, statement_addresses)
}

/// Real 8086 `Jcc` only has an 8-bit relative form; a target farther
/// than that needs the classic assembler trick of inverting the
/// condition and falling through to a near `JMP` (see
/// `encode_long_jcc`). Which `Jcc` statements need this can only be
/// known once every label's real address is known, which is exactly
/// what a `pass_one` run produces - so this is a small fixed-point
/// relaxation: assume every `Jcc` is short, run pass 1, check every
/// still-short `Jcc`'s real distance against the resulting symbol table,
/// promote any that don't fit, and repeat. Promoting a branch only ever
/// grows the code that follows it, so `long_jcc` only ever grows too -
/// this always terminates, bounded by the number of `Jcc` statements in
/// the program.
fn resolve_long_jcc_statements(
    statements: &[Statement],
    roles: &[SegmentRole],
    symbol_kinds: &BTreeMap<String, SymbolKind>,
    is_nasm_dialect: bool,
) -> BTreeSet<usize> {
    let mut long_jcc: BTreeSet<usize> = BTreeSet::new();
    loop {
        let mut probe_diagnostics = Vec::new();
        let (symbols, _, _, statement_addresses) = pass_one(
            statements,
            roles,
            symbol_kinds,
            &long_jcc,
            is_nasm_dialect,
            &mut probe_diagnostics,
        );
        let newly_long =
            newly_out_of_range_jcc(statements, &symbols, &statement_addresses, &long_jcc);
        if newly_long.is_empty() {
            return long_jcc;
        }
        long_jcc.extend(newly_long);
    }
}

/// One iteration's worth of `resolve_long_jcc_statements`' relaxation:
/// which currently-short `Jcc` statements don't actually fit an 8-bit
/// relative displacement, given `symbols`/`statement_addresses` from a
/// `pass_one` run against the current `long_jcc` set.
fn newly_out_of_range_jcc(
    statements: &[Statement],
    symbols: &SymbolTable,
    statement_addresses: &[u32],
    long_jcc: &BTreeSet<usize>,
) -> BTreeSet<usize> {
    let mut newly_long = BTreeSet::new();
    for (i, stmt) in statements.iter().enumerate() {
        if long_jcc.contains(&i) {
            continue;
        }
        if let StatementKind::Instruction {
            mnemonic: Mnemonic::Jcc(_),
            operands,
            ..
        } = &stmt.kind
        {
            if let Some(ParsedOperand::Immediate(expr)) = operands.first() {
                let address = statement_addresses[i];
                let address_after = address + 2; // a short Jcc is always 2 bytes
                if let Ok(target) = eval_expr(expr, symbols, 0, address as i64) {
                    let rel = target - address_after as i64;
                    if rel < i8::MIN as i64 || rel > i8::MAX as i64 {
                        newly_long.insert(i);
                    }
                }
            }
        }
    }
    newly_long
}

/// A far conditional jump's expansion: `J!cc +3` (skip the following
/// `JMP` when the original condition does *not* hold) then `JMP target`
/// with a full-range rel16. This is genuinely two back-to-back real
/// instructions, not one, so it deliberately bypasses the single-
/// mnemonic `Instruction`/`encode_one` pipeline - which also means a
/// disassembler reads it back as exactly what it is: two ordinary
/// instructions, correctly.
fn encode_long_jcc(condition: Condition, target: i64, address: u32) -> Result<Vec<u8>, String> {
    let address_after = address + 5;
    let rel = target - address_after as i64;
    let rel16 =
        i16::try_from(rel).map_err(|_| format!("branch target is out of rel16 range ({rel})"))?;
    // Every 8086 condition/negated-condition pair is adjacent in the
    // opcode's 4-bit condition field (JE=4/JNE=5, JB=2/JAE=3, ...), so
    // flipping the low bit always yields the logical negation.
    let inverted_index = condition_index(condition) ^ 1;
    let mut bytes = vec![0x70 + inverted_index, 0x03, 0xE9];
    bytes.extend_from_slice(&rel16.to_le_bytes());
    Ok(bytes)
}

// --- pass 2: final resolution + encoding ------------------------------------

#[allow(clippy::too_many_arguments)]
fn pass_two(
    statements: &[Statement],
    symbols: &SymbolTable,
    symbol_kinds: &BTreeMap<String, SymbolKind>,
    roles: &[SegmentRole],
    data_segment_base: u32,
    long_jcc: &BTreeSet<usize>,
    is_nasm_dialect: bool,
    diagnostics: &mut Vec<Diagnostic>,
) -> (Vec<u8>, BTreeMap<u32, u32>, u32, Option<u32>) {
    let data_segment_paragraph = (data_segment_base / PARAGRAPH_SIZE) as i64;
    let mut machine_code: Vec<u8> = Vec::new();
    let mut line_to_address: BTreeMap<u32, u32> = BTreeMap::new();
    let mut code_address: u32 = 0;
    let mut data_address: u32 = data_segment_base;
    let mut entry_point_symbol: Option<String> = None;
    let mut stack_size: Option<u32> = None;

    for (i, stmt) in statements.iter().enumerate() {
        let role = roles[i];
        match &stmt.kind {
            StatementKind::Label(_) | StatementKind::Equ { .. } | StatementKind::NoOp => {}
            StatementKind::SegmentSwitch(_) => {}
            StatementKind::Stack(expr) => {
                match eval_expr(expr, symbols, data_segment_paragraph, code_address as i64) {
                    Ok(value) => stack_size = Some(value.max(0) as u32),
                    Err(msg) => diagnostics.push(diag_error(stmt.line, msg)),
                }
            }
            StatementKind::Org(expr) => {
                let current_location = match role {
                    SegmentRole::Code => code_address,
                    SegmentRole::Data => data_address,
                };
                if let Ok(value) = eval_expr(
                    expr,
                    symbols,
                    data_segment_paragraph,
                    current_location as i64,
                ) {
                    match role {
                        SegmentRole::Code => code_address = value as u32,
                        // `value` is an offset *within* the data segment,
                        // matching pass 1's relative bookkeeping - the
                        // physical address needs the segment's base added.
                        SegmentRole::Data => data_address = data_segment_base + value as u32,
                    }
                }
            }
            StatementKind::End(label) => {
                if let Some(name) = label {
                    if symbols.contains_key(name) {
                        entry_point_symbol = Some(name.clone());
                    } else {
                        diagnostics.push(diag_error(
                            stmt.line,
                            format!("undefined entry point symbol '{name}'"),
                        ));
                    }
                }
            }
            StatementKind::Db(items) => {
                let current_location = match role {
                    SegmentRole::Code => code_address,
                    SegmentRole::Data => data_address,
                };
                match encode_data_items(
                    items,
                    1,
                    symbols,
                    data_segment_paragraph,
                    current_location as i64,
                ) {
                    Ok(bytes) => {
                        place_bytes(
                            role,
                            &bytes,
                            &mut code_address,
                            &mut data_address,
                            &mut machine_code,
                            &mut line_to_address,
                            stmt.line,
                        );
                    }
                    Err(msg) => diagnostics.push(diag_error(stmt.line, msg)),
                }
            }
            StatementKind::Dw(items) => {
                let current_location = match role {
                    SegmentRole::Code => code_address,
                    SegmentRole::Data => data_address,
                };
                match encode_data_items(
                    items,
                    2,
                    symbols,
                    data_segment_paragraph,
                    current_location as i64,
                ) {
                    Ok(bytes) => {
                        place_bytes(
                            role,
                            &bytes,
                            &mut code_address,
                            &mut data_address,
                            &mut machine_code,
                            &mut line_to_address,
                            stmt.line,
                        );
                    }
                    Err(msg) => diagnostics.push(diag_error(stmt.line, msg)),
                }
            }
            StatementKind::Instruction {
                mnemonic,
                operands,
                short_jump,
                repeat,
            } => {
                let address = match role {
                    SegmentRole::Code => code_address,
                    SegmentRole::Data => data_address,
                };
                let long_jcc_condition = match mnemonic {
                    Mnemonic::Jcc(condition) if long_jcc.contains(&i) => Some(*condition),
                    _ => None,
                };
                if let Some(condition) = long_jcc_condition {
                    let target_expr = match operands.first() {
                        Some(ParsedOperand::Immediate(expr)) => Some(expr),
                        _ => None,
                    };
                    match target_expr {
                        Some(expr) => {
                            match eval_expr(expr, symbols, data_segment_paragraph, address as i64) {
                                Ok(target) => match encode_long_jcc(condition, target, address) {
                                    Ok(bytes) => {
                                        place_bytes(
                                            role,
                                            &bytes,
                                            &mut code_address,
                                            &mut data_address,
                                            &mut machine_code,
                                            &mut line_to_address,
                                            stmt.line,
                                        );
                                    }
                                    Err(msg) => diagnostics.push(diag_error(stmt.line, msg)),
                                },
                                Err(msg) => diagnostics.push(diag_error(stmt.line, msg)),
                            }
                        }
                        None => diagnostics.push(diag_error(
                            stmt.line,
                            "branch instruction requires a target operand".to_string(),
                        )),
                    }
                } else {
                    match resolve_instruction(
                        *mnemonic,
                        operands,
                        *short_jump,
                        *repeat,
                        symbols,
                        symbol_kinds,
                        address,
                        true,
                        data_segment_paragraph,
                        is_nasm_dialect,
                    ) {
                        Ok(instr) => match encode_one(&instr) {
                            Ok(bytes) => {
                                place_bytes(
                                    role,
                                    &bytes,
                                    &mut code_address,
                                    &mut data_address,
                                    &mut machine_code,
                                    &mut line_to_address,
                                    stmt.line,
                                );
                            }
                            Err(e) => diagnostics.push(diag_error(stmt.line, e.0)),
                        },
                        Err(msg) => diagnostics.push(diag_error(stmt.line, msg)),
                    }
                }
            }
        }
    }

    let entry_point = entry_point_symbol
        .and_then(|name| symbols.get(&name))
        .map(|e| e.value as u32)
        .unwrap_or(0);

    (machine_code, line_to_address, entry_point, stack_size)
}

/// Writes `bytes` at whichever running address matches `role`, records a
/// true flat `line_to_address` entry, and advances that same counter -
/// the shared tail end of `Db`/`Dw`/`Instruction` handling in `pass_two`.
#[allow(clippy::too_many_arguments)]
fn place_bytes(
    role: SegmentRole,
    bytes: &[u8],
    code_address: &mut u32,
    data_address: &mut u32,
    machine_code: &mut Vec<u8>,
    line_to_address: &mut BTreeMap<u32, u32>,
    line: u32,
) {
    let address = match role {
        SegmentRole::Code => *code_address,
        SegmentRole::Data => *data_address,
    };
    line_to_address.insert(line, address);
    write_at(machine_code, address, bytes);
    match role {
        SegmentRole::Code => *code_address += bytes.len() as u32,
        SegmentRole::Data => *data_address += bytes.len() as u32,
    }
}

fn write_at(buf: &mut Vec<u8>, address: u32, bytes: &[u8]) {
    let end = address as usize + bytes.len();
    if buf.len() < end {
        buf.resize(end, 0);
    }
    buf[address as usize..end].copy_from_slice(bytes);
}

// --- expression evaluation ---------------------------------------------

/// `data_segment_paragraph` is what the MASM/emu8086 builtin `@DATA`
/// resolves to (the data segment's paragraph value); `current_location`
/// is what NASM's `$` resolves to (the address of whatever statement is
/// currently being resolved). Both are threaded through as plain
/// parameters rather than injected into the symbol table because they're
/// pseudo-symbols needing case-insensitive (`@DATA`) or reserved-name
/// (`$`) matching that a plain `BTreeMap` lookup can't give them.
fn eval_expr(
    expr: &ParsedExpr,
    symbols: &SymbolTable,
    data_segment_paragraph: i64,
    current_location: i64,
) -> Result<i64, String> {
    match expr {
        ParsedExpr::Number(n) => Ok(*n),
        ParsedExpr::Symbol(name) if name == "$" => Ok(current_location),
        ParsedExpr::Symbol(name) if name.eq_ignore_ascii_case("@data") => {
            Ok(data_segment_paragraph)
        }
        ParsedExpr::Symbol(name) => symbols
            .get(name)
            .map(|e| e.value)
            .ok_or_else(|| format!("undefined symbol '{name}'")),
        ParsedExpr::Sum(a, b) => {
            Ok(
                eval_expr(a, symbols, data_segment_paragraph, current_location)?
                    + eval_expr(b, symbols, data_segment_paragraph, current_location)?,
            )
        }
        ParsedExpr::Diff(a, b) => {
            Ok(
                eval_expr(a, symbols, data_segment_paragraph, current_location)?
                    - eval_expr(b, symbols, data_segment_paragraph, current_location)?,
            )
        }
        // A symbol's stored value is already its address/offset (never
        // a dereferenced value - eval_expr never touches memory), so
        // `OFFSET x` evaluates to the exact same number as bare `x`;
        // only the *operand shape* differs, which `resolve_operand`
        // special-cases separately.
        ParsedExpr::Offset(inner) => {
            eval_expr(inner, symbols, data_segment_paragraph, current_location)
        }
    }
}

/// Pass 1's lenient counterpart: forward references (or any other
/// evaluation failure) resolve to `0` rather than erroring, since pass 1
/// only needs a *length*, which never depends on the actual value.
fn eval_expr_lenient(
    expr: &ParsedExpr,
    symbols: &SymbolTable,
    data_segment_paragraph: i64,
    current_location: i64,
) -> i64 {
    eval_expr(expr, symbols, data_segment_paragraph, current_location).unwrap_or(0)
}

// --- DB/DW data items --------------------------------------------------

fn data_items_len(items: &[DataItem], unit_size: u32, symbols: &SymbolTable) -> u32 {
    items
        .iter()
        .map(|item| data_item_len(item, unit_size, symbols))
        .sum()
}

fn data_item_len(item: &DataItem, unit_size: u32, symbols: &SymbolTable) -> u32 {
    match item {
        DataItem::Value(_) | DataItem::Uninitialized => unit_size,
        DataItem::Str(s) => s.len() as u32,
        DataItem::Dup { count, item } => {
            // A DUP count referencing `@DATA`/`$` is a degenerate case
            // pass 1 doesn't need to get right (only a length is needed
            // here, and pass 2 re-evaluates it strictly) - 0 is a fine
            // placeholder for both.
            let count_value = eval_expr_lenient(count, symbols, 0, 0).max(0) as u32;
            count_value * data_item_len(item, unit_size, symbols)
        }
    }
}

fn encode_data_items(
    items: &[DataItem],
    unit_size: u32,
    symbols: &SymbolTable,
    data_segment_paragraph: i64,
    current_location: i64,
) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    for item in items {
        encode_data_item(
            item,
            unit_size,
            symbols,
            data_segment_paragraph,
            current_location,
            &mut bytes,
        )?;
    }
    Ok(bytes)
}

fn encode_data_item(
    item: &DataItem,
    unit_size: u32,
    symbols: &SymbolTable,
    data_segment_paragraph: i64,
    current_location: i64,
    out: &mut Vec<u8>,
) -> Result<(), String> {
    match item {
        DataItem::Value(expr) => {
            let value = eval_expr(expr, symbols, data_segment_paragraph, current_location)?;
            push_unit(out, value, unit_size)
        }
        DataItem::Str(s) => {
            out.extend(s.as_bytes());
            Ok(())
        }
        DataItem::Uninitialized => {
            out.extend(std::iter::repeat_n(0u8, unit_size as usize));
            Ok(())
        }
        DataItem::Dup { count, item } => {
            let count_value = eval_expr(count, symbols, data_segment_paragraph, current_location)?;
            if count_value < 0 {
                return Err(format!("DUP count cannot be negative, got {count_value}"));
            }
            for _ in 0..count_value {
                encode_data_item(
                    item,
                    unit_size,
                    symbols,
                    data_segment_paragraph,
                    current_location,
                    out,
                )?;
            }
            Ok(())
        }
    }
}

fn push_unit(out: &mut Vec<u8>, value: i64, unit_size: u32) -> Result<(), String> {
    if unit_size == 1 {
        if !(-128..=255).contains(&value) {
            return Err(format!("value {value} does not fit in a DB byte"));
        }
        out.push(value as u8);
    } else {
        if !(-32768..=65535).contains(&value) {
            return Err(format!("value {value} does not fit in a DW word"));
        }
        out.extend_from_slice(&(value as i16).to_le_bytes());
    }
    Ok(())
}

// --- instruction operand/width resolution -----------------------------

#[allow(clippy::too_many_arguments)]
fn resolve_instruction(
    mnemonic: Mnemonic,
    operands: &[ParsedOperand],
    short_jump: bool,
    repeat: Option<Repeat>,
    symbols: &SymbolTable,
    symbol_kinds: &BTreeMap<String, SymbolKind>,
    location_counter: u32,
    strict: bool,
    data_segment_paragraph: i64,
    is_nasm_dialect: bool,
) -> Result<Instruction, String> {
    if is_branch_mnemonic(mnemonic) {
        return resolve_branch_instruction(
            mnemonic,
            operands,
            short_jump,
            symbols,
            location_counter,
            strict,
            data_segment_paragraph,
        );
    }

    let width = determine_width(mnemonic, operands)?;
    let mut resolved = Vec::with_capacity(operands.len());
    for op in operands {
        resolved.push(resolve_operand(
            op,
            symbols,
            symbol_kinds,
            strict,
            data_segment_paragraph,
            location_counter as i64,
            is_nasm_dialect,
        )?);
    }
    let mut instr = Instruction::new(mnemonic, resolved, width, 0);
    if let Some(repeat) = repeat {
        instr = instr.with_repeat(repeat);
    }
    Ok(instr)
}

fn is_branch_mnemonic(mnemonic: Mnemonic) -> bool {
    matches!(
        mnemonic,
        Mnemonic::Jmp
            | Mnemonic::Jcc(_)
            | Mnemonic::Loop
            | Mnemonic::Loope
            | Mnemonic::Loopne
            | Mnemonic::Jcxz
            | Mnemonic::Call
    )
}

/// The 8086 gives every branch form a *fixed* length purely from its
/// syntax (`JMP` defaults near unless `SHORT` is explicit; `Jcc`/`LOOP`-
/// family are always rel8; `CALL` is always near) - so, unlike x86-64's
/// variable-length encodings, no relaxation solver is needed to know an
/// instruction's length before its target is resolved.
fn branch_fixed_length(mnemonic: Mnemonic, short_jump: bool) -> u8 {
    match mnemonic {
        Mnemonic::Jmp => {
            if short_jump {
                2
            } else {
                3
            }
        }
        Mnemonic::Call => 3,
        _ => 2, // Jcc, Loop, Loope, Loopne, Jcxz
    }
}

fn resolve_branch_instruction(
    mnemonic: Mnemonic,
    operands: &[ParsedOperand],
    short_jump: bool,
    symbols: &SymbolTable,
    location_counter: u32,
    strict: bool,
    data_segment_paragraph: i64,
) -> Result<Instruction, String> {
    let target_expr = match operands.first() {
        Some(ParsedOperand::Immediate(expr)) => expr,
        Some(other) => {
            return Err(format!(
                "branch target must be a label or address, found {other:?}"
            ))
        }
        None => return Err("branch instruction requires a target operand".to_string()),
    };

    let fixed_len = branch_fixed_length(mnemonic, short_jump);
    let address_after = location_counter + fixed_len as u32;
    // `$` inside a branch target means the address of the branch
    // instruction *itself* (its start, not the address after it) -
    // that's what makes the classic `JMP $` infinite-loop idiom work.
    let current_location = location_counter as i64;

    let target = if strict {
        eval_expr(
            target_expr,
            symbols,
            data_segment_paragraph,
            current_location,
        )?
    } else {
        eval_expr_lenient(
            target_expr,
            symbols,
            data_segment_paragraph,
            current_location,
        )
    };
    let rel = target - address_after as i64;

    Ok(Instruction::new(
        mnemonic,
        vec![Operand::Immediate(rel as i32)],
        None,
        fixed_len,
    ))
}

#[allow(clippy::too_many_arguments)]
fn resolve_operand(
    operand: &ParsedOperand,
    symbols: &SymbolTable,
    symbol_kinds: &BTreeMap<String, SymbolKind>,
    strict: bool,
    data_segment_paragraph: i64,
    current_location: i64,
    is_nasm_dialect: bool,
) -> Result<Operand, String> {
    match operand {
        ParsedOperand::Reg16(r) => Ok(Operand::Reg16(*r)),
        ParsedOperand::Reg8(r) => Ok(Operand::Reg8(*r)),
        // `OFFSET expr` always means "the address" - unconditionally,
        // regardless of dialect or the referenced symbol's kind (unlike
        // the bare-symbol case just below, which is exactly where those
        // start to matter). Handled first, and completely bypasses that
        // branching, on purpose.
        ParsedOperand::Immediate(ParsedExpr::Offset(inner)) => {
            let value = if strict {
                eval_expr(inner, symbols, data_segment_paragraph, current_location)?
            } else {
                eval_expr_lenient(inner, symbols, data_segment_paragraph, current_location)
            };
            Ok(Operand::Immediate(value as i32))
        }
        ParsedOperand::Immediate(ParsedExpr::Symbol(name)) if name == "$" => {
            Ok(Operand::Immediate(current_location as i32))
        }
        ParsedOperand::Immediate(ParsedExpr::Symbol(name))
            if name.eq_ignore_ascii_case("@data") =>
        {
            Ok(Operand::Immediate(data_segment_paragraph as i32))
        }
        // A bare `DB`/`DW` variable reference is where MASM/emu8086 and
        // NASM flatly disagree: MASM/emu8086 dereferences it ("the value
        // stored there"), NASM treats it as that variable's address
        // (same as a bare label). `is_nasm_dialect` (see `assemble`)
        // picks which convention this whole file uses.
        ParsedOperand::Immediate(ParsedExpr::Symbol(name)) => match symbols.get(name) {
            Some(entry) if entry.kind.is_data() && !is_nasm_dialect => {
                Ok(Operand::mem_direct(entry.value as i32))
            }
            Some(entry) => Ok(Operand::Immediate(entry.value as i32)),
            None if strict => Err(format!("undefined symbol '{name}'")),
            // Forward reference during pass 1: the value is genuinely
            // unknown (0 placeholder), but the operand *shape* must
            // still match what the symbol will turn out to be - the
            // pre-scanned kind (see `prescan_symbol_kinds`) is what
            // makes that possible before the symbol is actually defined.
            None => match symbol_kinds.get(name) {
                Some(kind) if kind.is_data() && !is_nasm_dialect => Ok(Operand::mem_direct(0)),
                _ => Ok(Operand::Immediate(0)),
            },
        },
        ParsedOperand::Immediate(expr) => {
            let value = if strict {
                eval_expr(expr, symbols, data_segment_paragraph, current_location)?
            } else {
                eval_expr_lenient(expr, symbols, data_segment_paragraph, current_location)
            };
            Ok(Operand::Immediate(value as i32))
        }
        ParsedOperand::Memory {
            segment_override,
            base,
            index,
            displacement,
            ..
        } => {
            let disp = match displacement {
                None => 0,
                Some(expr) => {
                    if strict {
                        eval_expr(expr, symbols, data_segment_paragraph, current_location)? as i32
                    } else {
                        eval_expr_lenient(expr, symbols, data_segment_paragraph, current_location)
                            as i32
                    }
                }
            };
            Ok(Operand::Memory {
                segment_override: *segment_override,
                base: *base,
                index: *index,
                displacement: disp,
            })
        }
    }
}

fn mnemonic_needs_width(mnemonic: Mnemonic) -> bool {
    matches!(
        mnemonic,
        Mnemonic::Mov
            | Mnemonic::Xchg
            | Mnemonic::Add
            | Mnemonic::Adc
            | Mnemonic::Sub
            | Mnemonic::Sbb
            | Mnemonic::Cmp
            | Mnemonic::Inc
            | Mnemonic::Dec
            | Mnemonic::And
            | Mnemonic::Or
            | Mnemonic::Xor
            | Mnemonic::Test
            | Mnemonic::Mul
            | Mnemonic::Imul
            | Mnemonic::Div
            | Mnemonic::Idiv
            | Mnemonic::Neg
            | Mnemonic::Not
    )
}

fn is_shift_rotate_mnemonic(mnemonic: Mnemonic) -> bool {
    matches!(
        mnemonic,
        Mnemonic::Shl
            | Mnemonic::Shr
            | Mnemonic::Sar
            | Mnemonic::Rol
            | Mnemonic::Ror
            | Mnemonic::Rcl
            | Mnemonic::Rcr
    )
}

fn determine_width(
    mnemonic: Mnemonic,
    operands: &[ParsedOperand],
) -> Result<Option<Width>, String> {
    if matches!(mnemonic, Mnemonic::Push | Mnemonic::Pop | Mnemonic::Lea) {
        return Ok(Some(Width::Word));
    }
    if is_shift_rotate_mnemonic(mnemonic) {
        // Only the destination (operands[0]) determines width - the count
        // operand (an immediate, or the CL register) must never be
        // considered: CL is always an 8-bit register regardless of
        // whether the destination is byte- or word-sized, so scanning
        // *all* operands (as the generic loop below does) would wrongly
        // infer Byte for e.g. `SHL WORD PTR [BX], CL`.
        return match operands.first() {
            Some(ParsedOperand::Reg8(_)) => Ok(Some(Width::Byte)),
            Some(ParsedOperand::Reg16(_)) => Ok(Some(Width::Word)),
            Some(ParsedOperand::Memory {
                size_override: Some(w),
                ..
            }) => Ok(Some(*w)),
            _ => Err("ambiguous operand size: add BYTE PTR or WORD PTR".to_string()),
        };
    }
    if !mnemonic_needs_width(mnemonic) {
        return Ok(None);
    }
    for op in operands {
        match op {
            ParsedOperand::Reg8(_) => return Ok(Some(Width::Byte)),
            ParsedOperand::Reg16(_) => return Ok(Some(Width::Word)),
            _ => {}
        }
    }
    for op in operands {
        if let ParsedOperand::Memory {
            size_override: Some(w),
            ..
        } = op
        {
            return Ok(Some(*w));
        }
    }
    Err("ambiguous operand size: add BYTE PTR or WORD PTR".to_string())
}

// --- diagnostics ---------------------------------------------------------

fn parse_error_to_diagnostic(e: &ParseError) -> Diagnostic {
    Diagnostic {
        line: e.span.line,
        col: e.span.col,
        severity: Severity::Error,
        message: e.message.clone(),
    }
}

fn diag_error(line: u32, message: String) -> Diagnostic {
    Diagnostic {
        line,
        col: 1,
        severity: Severity::Error,
        message,
    }
}
