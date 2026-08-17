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

use std::collections::BTreeMap;

use crate::ast::{DataItem, ParsedExpr, ParsedOperand, Statement, StatementKind};
use crate::encoder::encode_one;
use crate::parse_error::ParseError;
use crate::statement_parser::parse_program;
use crate::{AssembleResult, Diagnostic, Severity, SymbolTableEntry};
use x8086_isa::{Instruction, Mnemonic, Operand, Width};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SymbolKind {
    /// Defined via `EQU` - a compile-time constant, never a memory
    /// reference even when used bare.
    Constant,
    /// A code label (`name:`) or `PROC` name - used as a branch target,
    /// or as a raw address value if used bare in a non-branch context.
    Label,
    /// A `DB`/`DW` variable name - a bare reference to it means "the
    /// value stored at this address", matching MASM/emu8086 convention.
    Data,
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

    let (symbols, lengths) = pass_one(&statements, &mut diagnostics);
    let (machine_code, line_to_address, entry_point) =
        pass_two(&statements, &symbols, &lengths, &mut diagnostics);

    let mut symbol_entries: Vec<SymbolTableEntry> = symbols
        .into_iter()
        .map(|(name, entry)| SymbolTableEntry {
            name,
            value: entry.value,
        })
        .collect();
    symbol_entries.sort_by(|a, b| a.name.cmp(&b.name));

    AssembleResult {
        machine_code,
        entry_point,
        line_to_address,
        diagnostics,
        symbols: symbol_entries,
    }
}

// --- pass 1: symbol table + lengths -----------------------------------------

fn pass_one(
    statements: &[Statement],
    diagnostics: &mut Vec<Diagnostic>,
) -> (SymbolTable, Vec<u32>) {
    let mut symbols = SymbolTable::new();
    let mut lengths = vec![0u32; statements.len()];
    let mut location_counter: u32 = 0;

    for (i, stmt) in statements.iter().enumerate() {
        match &stmt.kind {
            StatementKind::Label(name) => {
                let kind = match statements.get(i + 1).map(|s| &s.kind) {
                    Some(StatementKind::Db(_)) | Some(StatementKind::Dw(_)) => SymbolKind::Data,
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
            StatementKind::Org(expr) => match eval_expr(expr, &symbols) {
                Ok(value) => location_counter = value as u32,
                Err(msg) => diagnostics.push(diag_error(stmt.line, msg)),
            },
            StatementKind::Equ { name, value } => match eval_expr(value, &symbols) {
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
            },
            StatementKind::Db(items) => {
                let len = data_items_len(items, 1, &symbols);
                lengths[i] = len;
                location_counter += len;
            }
            StatementKind::Dw(items) => {
                let len = data_items_len(items, 2, &symbols);
                lengths[i] = len;
                location_counter += len;
            }
            StatementKind::End(_) | StatementKind::NoOp => {}
            StatementKind::Instruction {
                mnemonic,
                operands,
                short_jump,
            } => {
                if let Ok(instr) = resolve_instruction(
                    *mnemonic,
                    operands,
                    *short_jump,
                    &symbols,
                    location_counter,
                    false,
                ) {
                    if let Ok(bytes) = encode_one(&instr) {
                        lengths[i] = bytes.len() as u32;
                        location_counter += bytes.len() as u32;
                    }
                }
                // Any failure here is deliberately swallowed - pass 2
                // re-resolves strictly and reports the real diagnostic.
            }
        }
    }

    (symbols, lengths)
}

// --- pass 2: final resolution + encoding ------------------------------------

fn pass_two(
    statements: &[Statement],
    symbols: &SymbolTable,
    lengths: &[u32],
    diagnostics: &mut Vec<Diagnostic>,
) -> (Vec<u8>, BTreeMap<u32, u32>, u32) {
    let mut machine_code: Vec<u8> = Vec::new();
    let mut line_to_address: BTreeMap<u32, u32> = BTreeMap::new();
    let mut address: u32 = 0;
    let mut entry_point_symbol: Option<String> = None;

    for stmt in statements {
        match &stmt.kind {
            StatementKind::Label(_) | StatementKind::Equ { .. } | StatementKind::NoOp => {}
            StatementKind::Org(expr) => {
                if let Ok(value) = eval_expr(expr, symbols) {
                    address = value as u32;
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
            StatementKind::Db(items) => match encode_data_items(items, 1, symbols) {
                Ok(bytes) => {
                    line_to_address.insert(stmt.line, address);
                    write_at(&mut machine_code, address, &bytes);
                    address += bytes.len() as u32;
                }
                Err(msg) => diagnostics.push(diag_error(stmt.line, msg)),
            },
            StatementKind::Dw(items) => match encode_data_items(items, 2, symbols) {
                Ok(bytes) => {
                    line_to_address.insert(stmt.line, address);
                    write_at(&mut machine_code, address, &bytes);
                    address += bytes.len() as u32;
                }
                Err(msg) => diagnostics.push(diag_error(stmt.line, msg)),
            },
            StatementKind::Instruction {
                mnemonic,
                operands,
                short_jump,
            } => {
                match resolve_instruction(*mnemonic, operands, *short_jump, symbols, address, true)
                {
                    Ok(instr) => match encode_one(&instr) {
                        Ok(bytes) => {
                            line_to_address.insert(stmt.line, address);
                            write_at(&mut machine_code, address, &bytes);
                            address += bytes.len() as u32;
                        }
                        Err(e) => diagnostics.push(diag_error(stmt.line, e.0)),
                    },
                    Err(msg) => diagnostics.push(diag_error(stmt.line, msg)),
                }
            }
        }
    }

    let _ = lengths; // pass 1's lengths only need to be correct in aggregate (via location_counter), not consulted directly here
    let entry_point = entry_point_symbol
        .and_then(|name| symbols.get(&name))
        .map(|e| e.value as u32)
        .unwrap_or(0);

    (machine_code, line_to_address, entry_point)
}

fn write_at(buf: &mut Vec<u8>, address: u32, bytes: &[u8]) {
    let end = address as usize + bytes.len();
    if buf.len() < end {
        buf.resize(end, 0);
    }
    buf[address as usize..end].copy_from_slice(bytes);
}

// --- expression evaluation ---------------------------------------------

fn eval_expr(expr: &ParsedExpr, symbols: &SymbolTable) -> Result<i64, String> {
    match expr {
        ParsedExpr::Number(n) => Ok(*n),
        ParsedExpr::Symbol(name) => symbols
            .get(name)
            .map(|e| e.value)
            .ok_or_else(|| format!("undefined symbol '{name}'")),
        ParsedExpr::Sum(a, b) => Ok(eval_expr(a, symbols)? + eval_expr(b, symbols)?),
    }
}

/// Pass 1's lenient counterpart: forward references (or any other
/// evaluation failure) resolve to `0` rather than erroring, since pass 1
/// only needs a *length*, which never depends on the actual value.
fn eval_expr_lenient(expr: &ParsedExpr, symbols: &SymbolTable) -> i64 {
    eval_expr(expr, symbols).unwrap_or(0)
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
            let count_value = eval_expr_lenient(count, symbols).max(0) as u32;
            count_value * data_item_len(item, unit_size, symbols)
        }
    }
}

fn encode_data_items(
    items: &[DataItem],
    unit_size: u32,
    symbols: &SymbolTable,
) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    for item in items {
        encode_data_item(item, unit_size, symbols, &mut bytes)?;
    }
    Ok(bytes)
}

fn encode_data_item(
    item: &DataItem,
    unit_size: u32,
    symbols: &SymbolTable,
    out: &mut Vec<u8>,
) -> Result<(), String> {
    match item {
        DataItem::Value(expr) => {
            let value = eval_expr(expr, symbols)?;
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
            let count_value = eval_expr(count, symbols)?;
            if count_value < 0 {
                return Err(format!("DUP count cannot be negative, got {count_value}"));
            }
            for _ in 0..count_value {
                encode_data_item(item, unit_size, symbols, out)?;
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

fn resolve_instruction(
    mnemonic: Mnemonic,
    operands: &[ParsedOperand],
    short_jump: bool,
    symbols: &SymbolTable,
    location_counter: u32,
    strict: bool,
) -> Result<Instruction, String> {
    if is_branch_mnemonic(mnemonic) {
        return resolve_branch_instruction(
            mnemonic,
            operands,
            short_jump,
            symbols,
            location_counter,
            strict,
        );
    }

    let width = determine_width(mnemonic, operands)?;
    let mut resolved = Vec::with_capacity(operands.len());
    for op in operands {
        resolved.push(resolve_operand(op, symbols, strict)?);
    }
    Ok(Instruction::new(mnemonic, resolved, width, 0))
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

    let target = if strict {
        eval_expr(target_expr, symbols)?
    } else {
        eval_expr_lenient(target_expr, symbols)
    };
    let rel = target - address_after as i64;

    Ok(Instruction::new(
        mnemonic,
        vec![Operand::Immediate(rel as i32)],
        None,
        fixed_len,
    ))
}

fn resolve_operand(
    operand: &ParsedOperand,
    symbols: &SymbolTable,
    strict: bool,
) -> Result<Operand, String> {
    match operand {
        ParsedOperand::Reg16(r) => Ok(Operand::Reg16(*r)),
        ParsedOperand::Reg8(r) => Ok(Operand::Reg8(*r)),
        ParsedOperand::Immediate(ParsedExpr::Symbol(name)) => match symbols.get(name) {
            Some(entry) => match entry.kind {
                SymbolKind::Data => Ok(Operand::mem_direct(entry.value as i32)),
                SymbolKind::Constant | SymbolKind::Label => {
                    Ok(Operand::Immediate(entry.value as i32))
                }
            },
            None if strict => Err(format!("undefined symbol '{name}'")),
            None => Ok(Operand::Immediate(0)),
        },
        ParsedOperand::Immediate(expr) => {
            let value = if strict {
                eval_expr(expr, symbols)?
            } else {
                eval_expr_lenient(expr, symbols)
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
                        eval_expr(expr, symbols)? as i32
                    } else {
                        eval_expr_lenient(expr, symbols) as i32
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
    )
}

fn determine_width(
    mnemonic: Mnemonic,
    operands: &[ParsedOperand],
) -> Result<Option<Width>, String> {
    if matches!(mnemonic, Mnemonic::Push | Mnemonic::Pop | Mnemonic::Lea) {
        return Ok(Some(Width::Word));
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
