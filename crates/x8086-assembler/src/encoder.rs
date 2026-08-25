//! `Instruction` -> bytes: the encoder half of the same
//! `x8086_isa::Instruction` model `x8086-decoder` decodes bytes into.
//! Deliberately not byte-optimal (e.g. word immediates always emit the
//! full 16-bit form rather than picking the shortest sign-extendable
//! encoding) - correctness and simplicity over code-size optimization,
//! which nothing in this project needs yet.

use x8086_isa::{Condition, Instruction, Mnemonic, Operand, Reg16, Reg8, Repeat, Width};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EncodeError(pub String);

impl EncodeError {
    fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl std::fmt::Display for EncodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

pub fn encode_one(instr: &Instruction) -> Result<Vec<u8>, EncodeError> {
    match instr.mnemonic {
        Mnemonic::Mov => encode_mov(instr),
        Mnemonic::Push => encode_push(instr),
        Mnemonic::Pop => encode_pop(instr),
        Mnemonic::Xchg => encode_xchg(instr),
        Mnemonic::Lea => encode_lea(instr),
        Mnemonic::Add
        | Mnemonic::Adc
        | Mnemonic::Sub
        | Mnemonic::Sbb
        | Mnemonic::And
        | Mnemonic::Or
        | Mnemonic::Xor
        | Mnemonic::Cmp => encode_arithmetic_group(instr),
        Mnemonic::Inc | Mnemonic::Dec => encode_inc_dec(instr),
        Mnemonic::Test => encode_test(instr),
        Mnemonic::Jmp => encode_jmp(instr),
        Mnemonic::Jcc(condition) => encode_jcc(condition, instr),
        Mnemonic::Loop => encode_rel8_branch(0xE2, instr),
        Mnemonic::Loope => encode_rel8_branch(0xE1, instr),
        Mnemonic::Loopne => encode_rel8_branch(0xE0, instr),
        Mnemonic::Jcxz => encode_rel8_branch(0xE3, instr),
        Mnemonic::Call => encode_call(instr),
        Mnemonic::Ret => encode_ret(instr),
        Mnemonic::Int => encode_int(instr),
        Mnemonic::Int3 => Ok(vec![0xCC]),
        Mnemonic::Iret => Ok(vec![0xCF]),
        Mnemonic::Hlt => Ok(vec![0xF4]),
        Mnemonic::Nop => Ok(vec![0x90]),
        Mnemonic::Clc => Ok(vec![0xF8]),
        Mnemonic::Stc => Ok(vec![0xF9]),
        Mnemonic::Cmc => Ok(vec![0xF5]),
        Mnemonic::Cld => Ok(vec![0xFC]),
        Mnemonic::Std => Ok(vec![0xFD]),
        Mnemonic::Cli => Ok(vec![0xFA]),
        Mnemonic::Sti => Ok(vec![0xFB]),
        Mnemonic::Pushf => Ok(vec![0x9C]),
        Mnemonic::Popf => Ok(vec![0x9D]),
        Mnemonic::Sahf => Ok(vec![0x9E]),
        Mnemonic::Lahf => Ok(vec![0x9F]),
        Mnemonic::Xlat => Ok(vec![0xD7]),
        Mnemonic::Shl
        | Mnemonic::Shr
        | Mnemonic::Sar
        | Mnemonic::Rol
        | Mnemonic::Ror
        | Mnemonic::Rcl
        | Mnemonic::Rcr => encode_shift_rotate(instr),
        Mnemonic::Mul
        | Mnemonic::Imul
        | Mnemonic::Div
        | Mnemonic::Idiv
        | Mnemonic::Not
        | Mnemonic::Neg => encode_unary(instr),
        Mnemonic::Movsb
        | Mnemonic::Movsw
        | Mnemonic::Cmpsb
        | Mnemonic::Cmpsw
        | Mnemonic::Stosb
        | Mnemonic::Stosw
        | Mnemonic::Lodsb
        | Mnemonic::Lodsw
        | Mnemonic::Scasb
        | Mnemonic::Scasw => encode_string(instr),
        Mnemonic::Unknown => Err(EncodeError::new("cannot encode Mnemonic::Unknown")),
    }
}

// --- shift/rotate group (D0-D3, C0-C1) ---------------------------------

fn shift_rotate_reg_field(mnemonic: Mnemonic) -> u8 {
    match mnemonic {
        Mnemonic::Rol => 0,
        Mnemonic::Ror => 1,
        Mnemonic::Rcl => 2,
        Mnemonic::Rcr => 3,
        Mnemonic::Shl => 4,
        Mnemonic::Shr => 5,
        Mnemonic::Sar => 7,
        other => unreachable!(
            "encode_shift_rotate only dispatches for the shift/rotate group, got {other:?}"
        ),
    }
}

fn encode_shift_rotate(instr: &Instruction) -> Result<Vec<u8>, EncodeError> {
    let width = instr
        .width
        .ok_or_else(|| EncodeError::new("shift/rotate instruction is missing a width"))?;
    let dst = &instr.operands[0];
    let reg_field = shift_rotate_reg_field(instr.mnemonic);
    let count = instr
        .operands
        .get(1)
        .ok_or_else(|| EncodeError::new("shift/rotate instruction requires a count operand"))?;
    match count {
        Operand::Immediate(1) => {
            let opcode = match width {
                Width::Byte => 0xD0,
                Width::Word => 0xD1,
            };
            let mut bytes = vec![opcode];
            bytes.extend(encode_rm(reg_field, dst, width)?);
            Ok(bytes)
        }
        Operand::Reg8(Reg8::Cl) => {
            let opcode = match width {
                Width::Byte => 0xD2,
                Width::Word => 0xD3,
            };
            let mut bytes = vec![opcode];
            bytes.extend(encode_rm(reg_field, dst, width)?);
            Ok(bytes)
        }
        Operand::Immediate(n) => {
            // The 80186 immediate-count form - real 8086 only has the
            // implicit-1 and CL forms above.
            let opcode = match width {
                Width::Byte => 0xC0,
                Width::Word => 0xC1,
            };
            let mut bytes = vec![opcode];
            bytes.extend(encode_rm(reg_field, dst, width)?);
            with_immediate(bytes, *n, Width::Byte)
        }
        other => Err(EncodeError::new(format!(
            "invalid shift/rotate count operand {other:?} - must be 1, CL, or an immediate"
        ))),
    }
}

// --- F6/F7 unary group (MUL/IMUL/DIV/IDIV/NEG/NOT) ----------------------

fn unary_group_reg_field(mnemonic: Mnemonic) -> u8 {
    match mnemonic {
        Mnemonic::Not => 2,
        Mnemonic::Neg => 3,
        Mnemonic::Mul => 4,
        Mnemonic::Imul => 5,
        Mnemonic::Div => 6,
        Mnemonic::Idiv => 7,
        other => unreachable!("encode_unary only dispatches for the F6/F7 group, got {other:?}"),
    }
}

fn encode_unary(instr: &Instruction) -> Result<Vec<u8>, EncodeError> {
    let width = instr
        .width
        .ok_or_else(|| EncodeError::new("instruction is missing a width"))?;
    let opcode = match width {
        Width::Byte => 0xF6,
        Width::Word => 0xF7,
    };
    let reg_field = unary_group_reg_field(instr.mnemonic);
    let mut bytes = vec![opcode];
    bytes.extend(encode_rm(reg_field, &instr.operands[0], width)?);
    Ok(bytes)
}

// --- string instructions + REP/REPE/REPNE prefix ------------------------

fn string_opcode(mnemonic: Mnemonic) -> u8 {
    match mnemonic {
        Mnemonic::Movsb => 0xA4,
        Mnemonic::Movsw => 0xA5,
        Mnemonic::Cmpsb => 0xA6,
        Mnemonic::Cmpsw => 0xA7,
        Mnemonic::Stosb => 0xAA,
        Mnemonic::Stosw => 0xAB,
        Mnemonic::Lodsb => 0xAC,
        Mnemonic::Lodsw => 0xAD,
        Mnemonic::Scasb => 0xAE,
        Mnemonic::Scasw => 0xAF,
        other => unreachable!("encode_string only dispatches for string mnemonics, got {other:?}"),
    }
}

fn encode_string(instr: &Instruction) -> Result<Vec<u8>, EncodeError> {
    let mut bytes = Vec::new();
    match instr.repeat {
        Some(Repeat::Rep) | Some(Repeat::Repe) => bytes.push(0xF3),
        Some(Repeat::Repne) => bytes.push(0xF2),
        None => {}
    }
    bytes.push(string_opcode(instr.mnemonic));
    Ok(bytes)
}

// --- shared low-level helpers -----------------------------------------------

fn width_bit(width: Width) -> u8 {
    match width {
        Width::Byte => 0,
        Width::Word => 1,
    }
}

fn is_accumulator(operand: &Operand, width: Width) -> bool {
    matches!(
        (operand, width),
        (Operand::Reg8(x8086_isa::Reg8::Al), Width::Byte)
            | (Operand::Reg16(Reg16::Ax), Width::Word)
    )
}

fn validate_fits_width(value: i32, width: Width) -> Result<(), EncodeError> {
    let (lo, hi) = match width {
        Width::Byte => (-128, 255),
        Width::Word => (-32768, 65535),
    };
    if value < lo || value > hi {
        return Err(EncodeError::new(format!(
            "value {value} does not fit in a {width:?} immediate"
        )));
    }
    Ok(())
}

fn immediate_bytes(value: i32, width: Width) -> Result<Vec<u8>, EncodeError> {
    validate_fits_width(value, width)?;
    Ok(match width {
        Width::Byte => vec![value as u8],
        Width::Word => (value as i16).to_le_bytes().to_vec(),
    })
}

fn with_immediate(mut bytes: Vec<u8>, value: i32, width: Width) -> Result<Vec<u8>, EncodeError> {
    bytes.extend(immediate_bytes(value, width)?);
    Ok(bytes)
}

fn reg_field_of(operand: &Operand) -> Result<u8, EncodeError> {
    match operand {
        Operand::Reg8(r) => Ok(r.to_index()),
        Operand::Reg16(r) => r
            .to_index()
            .ok_or_else(|| EncodeError::new(format!("{r:?} has no plain register-field encoding"))),
        other => Err(EncodeError::new(format!(
            "expected a register operand, found {other:?}"
        ))),
    }
}

/// Which of two operands plays the ModRM `reg` role vs the `r/m` role,
/// for instructions (TEST, XCHG) whose encoding doesn't distinguish
/// "destination" from "source". Whichever operand is a memory reference
/// must be `r/m` (a register can never be the memory side); when both
/// are registers, `b` becomes `reg` and `a` becomes `r/m` so that
/// decoding the result (a "ToRm"-style opcode always reports
/// `[rm, reg]`) reconstructs the same `[a, b]` order the caller passed
/// in, rather than silently swapping it.
fn split_reg_and_rm<'a>(
    a: &'a Operand,
    b: &'a Operand,
) -> Result<(&'a Operand, &'a Operand), EncodeError> {
    match (a, b) {
        (Operand::Memory { .. }, Operand::Memory { .. }) => Err(EncodeError::new(
            "both operands cannot be memory references",
        )),
        (Operand::Memory { .. }, _) => Ok((b, a)),
        (_, Operand::Memory { .. }) => Ok((a, b)),
        _ => Ok((b, a)),
    }
}

fn rm_field_for(base: Option<Reg16>, index: Option<Reg16>) -> Result<u8, EncodeError> {
    match (base, index) {
        (Some(Reg16::Bx), Some(Reg16::Si)) => Ok(0b000),
        (Some(Reg16::Bx), Some(Reg16::Di)) => Ok(0b001),
        (Some(Reg16::Bp), Some(Reg16::Si)) => Ok(0b010),
        (Some(Reg16::Bp), Some(Reg16::Di)) => Ok(0b011),
        (Some(Reg16::Si), None) => Ok(0b100),
        (Some(Reg16::Di), None) => Ok(0b101),
        (Some(Reg16::Bp), None) => Ok(0b110),
        (Some(Reg16::Bx), None) => Ok(0b111),
        (None, None) => Ok(0b110),
        _ => Err(EncodeError::new(format!(
            "{base:?}+{index:?} is not a valid 8086 addressing-mode combination"
        ))),
    }
}

/// Encodes the ModRM byte (and any displacement) for `operand` as the
/// `r/m` half, with `reg_field` supplying the other 3 bits (either a
/// real register or an opcode-extension selector, depending on caller).
fn encode_rm(reg_field: u8, operand: &Operand, width: Width) -> Result<Vec<u8>, EncodeError> {
    match operand {
        Operand::Reg8(r) => Ok(vec![0b11_000_000 | (reg_field << 3) | r.to_index()]),
        Operand::Reg16(r) => {
            let index = r.to_index().ok_or_else(|| {
                EncodeError::new(format!("{r:?} cannot be used as an r/m register operand"))
            })?;
            Ok(vec![0b11_000_000 | (reg_field << 3) | index])
        }
        Operand::Memory {
            segment_override,
            base,
            index,
            displacement,
        } => {
            if segment_override.is_some() {
                return Err(EncodeError::new(
                    "segment override prefixes are not yet supported by the assembler",
                ));
            }
            let _ = width; // width affects the caller's opcode choice, not the ModRM/displacement bytes
            encode_memory_operand(reg_field, *base, *index, *displacement)
        }
        Operand::Immediate(_) => Err(EncodeError::new(
            "an immediate cannot be used as an r/m operand",
        )),
    }
}

fn encode_memory_operand(
    reg_field: u8,
    base: Option<Reg16>,
    index: Option<Reg16>,
    displacement: i32,
) -> Result<Vec<u8>, EncodeError> {
    let rm = rm_field_for(base, index)?;

    if base.is_none() && index.is_none() {
        let modrm = (reg_field << 3) | rm; // mod=00, rm=110: direct address
        let mut bytes = vec![modrm];
        bytes.extend_from_slice(&(displacement as i16).to_le_bytes());
        return Ok(bytes);
    }

    // [BP] alone has no mod=00 encoding (that bit pattern means "direct
    // address" instead) - it must be written as mod=01 with an explicit
    // zero displacement.
    let bare_bp_zero = base == Some(Reg16::Bp) && index.is_none() && displacement == 0;

    if displacement == 0 && !bare_bp_zero {
        Ok(vec![(reg_field << 3) | rm])
    } else if bare_bp_zero {
        Ok(vec![(0b01 << 6) | (reg_field << 3) | rm, 0x00])
    } else {
        let mut bytes = vec![(0b10 << 6) | (reg_field << 3) | rm];
        bytes.extend_from_slice(&(displacement as i16).to_le_bytes());
        Ok(bytes)
    }
}

/// The `rm,reg`/`reg,rm` register-or-memory pattern shared by MOV and
/// the arithmetic/logic group (opcodes `base+0`..`base+3`).
fn encode_rm_reg_form(
    base: u8,
    dst: &Operand,
    src: &Operand,
    width: Width,
) -> Result<Vec<u8>, EncodeError> {
    let w = width_bit(width);
    match (dst, src) {
        (Operand::Memory { .. }, Operand::Memory { .. }) => Err(EncodeError::new(
            "both operands cannot be memory references",
        )),
        (Operand::Memory { .. }, _) => {
            let reg_field = reg_field_of(src)?;
            let mut bytes = vec![base + w];
            bytes.extend(encode_rm(reg_field, dst, width)?);
            Ok(bytes)
        }
        (_, Operand::Memory { .. }) => {
            let reg_field = reg_field_of(dst)?;
            let mut bytes = vec![base + 2 + w];
            bytes.extend(encode_rm(reg_field, src, width)?);
            Ok(bytes)
        }
        _ => {
            // Both registers: either form is valid; ToRm (dst=rm,
            // src=reg) is our deterministic convention.
            let reg_field = reg_field_of(src)?;
            let mut bytes = vec![base + w];
            bytes.extend(encode_rm(reg_field, dst, width)?);
            Ok(bytes)
        }
    }
}

fn immediate_operand_value(operand: &Operand) -> Result<i32, EncodeError> {
    match operand {
        Operand::Immediate(value) => Ok(*value),
        other => Err(EncodeError::new(format!(
            "expected an immediate/relative operand, found {other:?}"
        ))),
    }
}

pub(crate) fn condition_index(condition: Condition) -> u8 {
    match condition {
        Condition::Overflow => 0x0,
        Condition::NotOverflow => 0x1,
        Condition::Below => 0x2,
        Condition::AboveOrEqual => 0x3,
        Condition::Equal => 0x4,
        Condition::NotEqual => 0x5,
        Condition::BelowOrEqual => 0x6,
        Condition::Above => 0x7,
        Condition::Sign => 0x8,
        Condition::NotSign => 0x9,
        Condition::Parity => 0xA,
        Condition::NotParity => 0xB,
        Condition::Less => 0xC,
        Condition::GreaterOrEqual => 0xD,
        Condition::LessOrEqual => 0xE,
        Condition::Greater => 0xF,
    }
}

// --- MOV ---------------------------------------------------------------

fn encode_mov(instr: &Instruction) -> Result<Vec<u8>, EncodeError> {
    let width = instr
        .width
        .ok_or_else(|| EncodeError::new("MOV is missing a width"))?;
    let dst = &instr.operands[0];
    let src = &instr.operands[1];

    if let Operand::Reg16(r) = dst {
        if r.is_segment() {
            let seg_index = r
                .to_segment_index()
                .expect("is_segment() implies a valid segment index");
            let mut bytes = vec![0x8E];
            bytes.extend(encode_rm(seg_index, src, Width::Word)?);
            return Ok(bytes);
        }
    }
    if let Operand::Reg16(r) = src {
        if r.is_segment() {
            let seg_index = r
                .to_segment_index()
                .expect("is_segment() implies a valid segment index");
            let mut bytes = vec![0x8C];
            bytes.extend(encode_rm(seg_index, dst, Width::Word)?);
            return Ok(bytes);
        }
    }

    if let Operand::Immediate(value) = src {
        return match dst {
            Operand::Reg8(r) => with_immediate(vec![0xB0 + r.to_index()], *value, Width::Byte),
            Operand::Reg16(r) => {
                let index = r.to_index().ok_or_else(|| {
                    EncodeError::new(format!("{r:?} cannot receive an immediate MOV"))
                })?;
                with_immediate(vec![0xB8 + index], *value, Width::Word)
            }
            Operand::Memory { .. } => {
                let opcode = match width {
                    Width::Byte => 0xC6,
                    Width::Word => 0xC7,
                };
                let mut bytes = vec![opcode];
                bytes.extend(encode_rm(0, dst, width)?);
                with_immediate(bytes, *value, width)
            }
            Operand::Immediate(_) => Err(EncodeError::new("cannot MOV into an immediate")),
        };
    }

    encode_rm_reg_form(0x88, dst, src, width)
}

// --- PUSH / POP ----------------------------------------------------------

fn encode_push(instr: &Instruction) -> Result<Vec<u8>, EncodeError> {
    match &instr.operands[0] {
        Operand::Reg16(r) if r.is_segment() => Ok(vec![match r {
            Reg16::Es => 0x06,
            Reg16::Cs => 0x0E,
            Reg16::Ss => 0x16,
            Reg16::Ds => 0x1E,
            _ => unreachable!("is_segment() covers exactly ES/CS/SS/DS"),
        }]),
        Operand::Reg16(r) => {
            let index = r
                .to_index()
                .ok_or_else(|| EncodeError::new(format!("{r:?} cannot be PUSHed")))?;
            Ok(vec![0x50 + index])
        }
        mem @ Operand::Memory { .. } => {
            let mut bytes = vec![0xFF];
            bytes.extend(encode_rm(6, mem, Width::Word)?);
            Ok(bytes)
        }
        other => Err(EncodeError::new(format!("cannot PUSH {other:?}"))),
    }
}

fn encode_pop(instr: &Instruction) -> Result<Vec<u8>, EncodeError> {
    match &instr.operands[0] {
        Operand::Reg16(Reg16::Cs) => {
            Err(EncodeError::new("POP CS is not a valid 8086 instruction"))
        }
        Operand::Reg16(r) if r.is_segment() => Ok(vec![match r {
            Reg16::Es => 0x07,
            Reg16::Ss => 0x17,
            Reg16::Ds => 0x1F,
            _ => unreachable!("CS was handled above; only ES/SS/DS remain"),
        }]),
        Operand::Reg16(r) => {
            let index = r
                .to_index()
                .ok_or_else(|| EncodeError::new(format!("{r:?} cannot be POPped")))?;
            Ok(vec![0x58 + index])
        }
        mem @ Operand::Memory { .. } => {
            let mut bytes = vec![0x8F];
            bytes.extend(encode_rm(0, mem, Width::Word)?);
            Ok(bytes)
        }
        other => Err(EncodeError::new(format!("cannot POP {other:?}"))),
    }
}

// --- XCHG / LEA ------------------------------------------------------------

fn encode_xchg(instr: &Instruction) -> Result<Vec<u8>, EncodeError> {
    let width = instr
        .width
        .ok_or_else(|| EncodeError::new("XCHG is missing a width"))?;
    let a = &instr.operands[0];
    let b = &instr.operands[1];

    if width == Width::Word {
        let ax_and_other = match (a, b) {
            (Operand::Reg16(Reg16::Ax), Operand::Reg16(r)) => Some(*r),
            (Operand::Reg16(r), Operand::Reg16(Reg16::Ax)) => Some(*r),
            _ => None,
        };
        if let Some(r) = ax_and_other {
            let index = r
                .to_index()
                .ok_or_else(|| EncodeError::new(format!("{r:?} cannot be XCHGed")))?;
            return Ok(vec![0x90 + index]);
        }
    }

    let base = match width {
        Width::Byte => 0x86,
        Width::Word => 0x87,
    };
    let (reg_operand, rm_operand) = split_reg_and_rm(a, b)?;
    let reg_field = reg_field_of(reg_operand)?;
    let mut bytes = vec![base];
    bytes.extend(encode_rm(reg_field, rm_operand, width)?);
    Ok(bytes)
}

fn encode_lea(instr: &Instruction) -> Result<Vec<u8>, EncodeError> {
    let dst = &instr.operands[0];
    let src = &instr.operands[1];
    if !matches!(src, Operand::Memory { .. }) {
        return Err(EncodeError::new(
            "LEA's source operand must be a memory reference",
        ));
    }
    let reg_field = reg_field_of(dst)?;
    let mut bytes = vec![0x8D];
    bytes.extend(encode_rm(reg_field, src, Width::Word)?);
    Ok(bytes)
}

// --- arithmetic/logic group + INC/DEC/TEST ----------------------------------

fn arithmetic_group_index(mnemonic: Mnemonic) -> u8 {
    match mnemonic {
        Mnemonic::Add => 0,
        Mnemonic::Or => 1,
        Mnemonic::Adc => 2,
        Mnemonic::Sbb => 3,
        Mnemonic::And => 4,
        Mnemonic::Sub => 5,
        Mnemonic::Xor => 6,
        Mnemonic::Cmp => 7,
        other => unreachable!(
            "encode_arithmetic_group only dispatches for the 8-op group, got {other:?}"
        ),
    }
}

fn encode_arithmetic_group(instr: &Instruction) -> Result<Vec<u8>, EncodeError> {
    let width = instr
        .width
        .ok_or_else(|| EncodeError::new("arithmetic/logic instruction is missing a width"))?;
    let index = arithmetic_group_index(instr.mnemonic);
    let base = index * 8;
    let dst = &instr.operands[0];
    let src = &instr.operands[1];

    if let Operand::Immediate(value) = src {
        if is_accumulator(dst, width) {
            let opcode = base + 4 + width_bit(width);
            return with_immediate(vec![opcode], *value, width);
        }
        let opcode = match width {
            Width::Byte => 0x80,
            Width::Word => 0x81,
        };
        let mut bytes = vec![opcode];
        bytes.extend(encode_rm(index, dst, width)?);
        return with_immediate(bytes, *value, width);
    }

    encode_rm_reg_form(base, dst, src, width)
}

fn encode_inc_dec(instr: &Instruction) -> Result<Vec<u8>, EncodeError> {
    let width = instr
        .width
        .ok_or_else(|| EncodeError::new("INC/DEC is missing a width"))?;
    let operand = &instr.operands[0];
    let is_inc = instr.mnemonic == Mnemonic::Inc;

    if let Operand::Reg16(r) = operand {
        let index = r
            .to_index()
            .ok_or_else(|| EncodeError::new(format!("{r:?} cannot be INC/DEC'd")))?;
        let base = if is_inc { 0x40 } else { 0x48 };
        return Ok(vec![base + index]);
    }

    let opcode = match width {
        Width::Byte => 0xFE,
        Width::Word => 0xFF,
    };
    let reg_field = if is_inc { 0 } else { 1 };
    let mut bytes = vec![opcode];
    bytes.extend(encode_rm(reg_field, operand, width)?);
    Ok(bytes)
}

fn encode_test(instr: &Instruction) -> Result<Vec<u8>, EncodeError> {
    let width = instr
        .width
        .ok_or_else(|| EncodeError::new("TEST is missing a width"))?;
    let dst = &instr.operands[0];
    let src = &instr.operands[1];

    if let Operand::Immediate(value) = src {
        if is_accumulator(dst, width) {
            let opcode = match width {
                Width::Byte => 0xA8,
                Width::Word => 0xA9,
            };
            return with_immediate(vec![opcode], *value, width);
        }
        return Err(EncodeError::new(
            "TEST reg/mem, immediate is only supported against AL/AX (the general F6/F7 opcode group isn't implemented yet)",
        ));
    }

    let base = match width {
        Width::Byte => 0x84,
        Width::Word => 0x85,
    };
    let (reg_operand, rm_operand) = split_reg_and_rm(dst, src)?;
    let reg_field = reg_field_of(reg_operand)?;
    let mut bytes = vec![base];
    bytes.extend(encode_rm(reg_field, rm_operand, width)?);
    Ok(bytes)
}

// --- control transfer --------------------------------------------------

fn encode_jmp(instr: &Instruction) -> Result<Vec<u8>, EncodeError> {
    let rel = immediate_operand_value(&instr.operands[0])?;
    if instr.byte_len == 2 {
        let rel8 = i8::try_from(rel).map_err(|_| {
            EncodeError::new(format!("JMP SHORT target is out of rel8 range ({rel})"))
        })?;
        Ok(vec![0xEB, rel8 as u8])
    } else {
        let rel16 = i16::try_from(rel)
            .map_err(|_| EncodeError::new(format!("JMP target is out of rel16 range ({rel})")))?;
        let mut bytes = vec![0xE9];
        bytes.extend_from_slice(&rel16.to_le_bytes());
        Ok(bytes)
    }
}

fn encode_jcc(condition: Condition, instr: &Instruction) -> Result<Vec<u8>, EncodeError> {
    let rel = immediate_operand_value(&instr.operands[0])?;
    let rel8 =
        i8::try_from(rel).map_err(|_| EncodeError::new(format!("conditional jump target is out of range ({rel}); 8086 Jcc only has an 8-bit relative form")))?;
    Ok(vec![0x70 + condition_index(condition), rel8 as u8])
}

fn encode_rel8_branch(opcode: u8, instr: &Instruction) -> Result<Vec<u8>, EncodeError> {
    let rel = immediate_operand_value(&instr.operands[0])?;
    let rel8 = i8::try_from(rel)
        .map_err(|_| EncodeError::new(format!("branch target is out of rel8 range ({rel})")))?;
    Ok(vec![opcode, rel8 as u8])
}

fn encode_call(instr: &Instruction) -> Result<Vec<u8>, EncodeError> {
    let rel = immediate_operand_value(&instr.operands[0])?;
    let rel16 = i16::try_from(rel)
        .map_err(|_| EncodeError::new(format!("CALL target is out of rel16 range ({rel})")))?;
    let mut bytes = vec![0xE8];
    bytes.extend_from_slice(&rel16.to_le_bytes());
    Ok(bytes)
}

fn encode_ret(instr: &Instruction) -> Result<Vec<u8>, EncodeError> {
    match instr.operands.first() {
        None => Ok(vec![0xC3]),
        Some(Operand::Immediate(value)) => {
            if !(0..=0xFFFF).contains(value) {
                return Err(EncodeError::new(format!(
                    "RET pop count out of range: {value}"
                )));
            }
            let mut bytes = vec![0xC2];
            bytes.extend_from_slice(&(*value as i16).to_le_bytes());
            Ok(bytes)
        }
        Some(other) => Err(EncodeError::new(format!(
            "RET's operand must be an immediate pop count, found {other:?}"
        ))),
    }
}

fn encode_int(instr: &Instruction) -> Result<Vec<u8>, EncodeError> {
    let value = immediate_operand_value(&instr.operands[0])?;
    if !(0..=255).contains(&value) {
        return Err(EncodeError::new(format!(
            "INT vector out of range: {value}"
        )));
    }
    Ok(vec![0xCD, value as u8])
}

#[cfg(test)]
mod tests {
    use super::*;
    use x8086_isa::{Reg16, Reg8};

    /// Encodes `instr`, then decodes the result back with the real
    /// decoder and asserts it reconstructs an equivalent instruction -
    /// the property that actually matters: whatever we emit must be
    /// exactly what a program reading it back would execute.
    fn assert_round_trips(instr: Instruction) {
        let bytes =
            encode_one(&instr).unwrap_or_else(|e| panic!("failed to encode {instr:?}: {e}"));
        let (decoded, len) = x8086_decoder::decode_one(&bytes).unwrap_or_else(|e| {
            panic!("failed to decode our own output {bytes:02x?} for {instr:?}: {e:?}")
        });
        assert_eq!(
            len,
            bytes.len(),
            "decoded length must match encoded length for {instr:?}"
        );
        assert_eq!(
            decoded.mnemonic, instr.mnemonic,
            "mnemonic mismatch after round-trip for {instr:?}, got bytes {bytes:02x?}"
        );
        assert_eq!(
            decoded.operands, instr.operands,
            "operand mismatch after round-trip for {instr:?}, got bytes {bytes:02x?}"
        );
        assert_eq!(
            decoded.width, instr.width,
            "width mismatch after round-trip for {instr:?}, got bytes {bytes:02x?}"
        );
        assert_eq!(
            decoded.repeat, instr.repeat,
            "repeat-prefix mismatch after round-trip for {instr:?}, got bytes {bytes:02x?}"
        );
    }

    fn instr(mnemonic: Mnemonic, operands: Vec<Operand>, width: Option<Width>) -> Instruction {
        Instruction::new(mnemonic, operands, width, 0)
    }

    #[test]
    fn mov_reg_to_reg_encodes_and_round_trips() {
        let i = instr(
            Mnemonic::Mov,
            vec![Operand::Reg16(Reg16::Cx), Operand::Reg16(Reg16::Ax)],
            Some(Width::Word),
        );
        assert_eq!(encode_one(&i).unwrap(), vec![0x89, 0b11_000_001]);
        assert_round_trips(i);
    }

    #[test]
    fn mov_reg_imm16_prefers_the_compact_form() {
        let i = instr(
            Mnemonic::Mov,
            vec![Operand::Reg16(Reg16::Ax), Operand::Immediate(0x1234)],
            Some(Width::Word),
        );
        assert_eq!(encode_one(&i).unwrap(), vec![0xB8, 0x34, 0x12]);
        assert_round_trips(i);
    }

    #[test]
    fn mov_mem_imm16_uses_the_general_form() {
        let i = instr(
            Mnemonic::Mov,
            vec![
                Operand::mem(Some(Reg16::Bx), None, 0),
                Operand::Immediate(5),
            ],
            Some(Width::Word),
        );
        assert_eq!(
            encode_one(&i).unwrap(),
            vec![0xC7, 0b00_000_111, 0x05, 0x00]
        );
        assert_round_trips(i);
    }

    #[test]
    fn mov_segment_register_forms_round_trip() {
        let to_ds = instr(
            Mnemonic::Mov,
            vec![Operand::Reg16(Reg16::Ds), Operand::Reg16(Reg16::Ax)],
            Some(Width::Word),
        );
        assert_round_trips(to_ds);
        let from_ds = instr(
            Mnemonic::Mov,
            vec![Operand::Reg16(Reg16::Ax), Operand::Reg16(Reg16::Ds)],
            Some(Width::Word),
        );
        assert_round_trips(from_ds);
    }

    #[test]
    fn lea_round_trips() {
        let i = instr(
            Mnemonic::Lea,
            vec![
                Operand::Reg16(Reg16::Ax),
                Operand::mem(Some(Reg16::Bx), Some(Reg16::Si), 4),
            ],
            Some(Width::Word),
        );
        assert_round_trips(i);
    }

    #[test]
    fn lea_rejects_a_register_source() {
        let i = instr(
            Mnemonic::Lea,
            vec![Operand::Reg16(Reg16::Ax), Operand::Reg16(Reg16::Bx)],
            Some(Width::Word),
        );
        assert!(encode_one(&i).is_err());
    }

    #[test]
    fn arithmetic_group_all_eight_ops_round_trip_register_form() {
        for mnemonic in [
            Mnemonic::Add,
            Mnemonic::Or,
            Mnemonic::Adc,
            Mnemonic::Sbb,
            Mnemonic::And,
            Mnemonic::Sub,
            Mnemonic::Xor,
            Mnemonic::Cmp,
        ] {
            let i = instr(
                mnemonic,
                vec![Operand::Reg8(Reg8::Al), Operand::Reg8(Reg8::Cl)],
                Some(Width::Byte),
            );
            assert_round_trips(i);
        }
    }

    #[test]
    fn add_accumulator_immediate_uses_the_compact_form() {
        let i = instr(
            Mnemonic::Add,
            vec![Operand::Reg8(Reg8::Al), Operand::Immediate(5)],
            Some(Width::Byte),
        );
        assert_eq!(encode_one(&i).unwrap(), vec![0x04, 0x05]);
        assert_round_trips(i);
    }

    #[test]
    fn cmp_immediate_group_word_round_trips() {
        let i = instr(
            Mnemonic::Cmp,
            vec![Operand::Reg16(Reg16::Cx), Operand::Immediate(-1)],
            Some(Width::Word),
        );
        assert_round_trips(i);
    }

    #[test]
    fn dst_memory_src_register_round_trips() {
        let i = instr(
            Mnemonic::Add,
            vec![
                Operand::mem(Some(Reg16::Bx), None, 0),
                Operand::Reg16(Reg16::Ax),
            ],
            Some(Width::Word),
        );
        assert_round_trips(i);
    }

    #[test]
    fn dst_register_src_memory_round_trips() {
        let i = instr(
            Mnemonic::Add,
            vec![
                Operand::Reg16(Reg16::Ax),
                Operand::mem(Some(Reg16::Bx), None, 0),
            ],
            Some(Width::Word),
        );
        assert_round_trips(i);
    }

    #[test]
    fn mem_mem_is_rejected() {
        let i = instr(
            Mnemonic::Add,
            vec![
                Operand::mem(Some(Reg16::Bx), None, 0),
                Operand::mem(Some(Reg16::Si), None, 0),
            ],
            Some(Width::Word),
        );
        assert!(encode_one(&i).is_err());
    }

    #[test]
    fn inc_dec_register_and_memory_forms_round_trip() {
        assert_round_trips(instr(
            Mnemonic::Inc,
            vec![Operand::Reg16(Reg16::Cx)],
            Some(Width::Word),
        ));
        assert_round_trips(instr(
            Mnemonic::Dec,
            vec![Operand::Reg16(Reg16::Cx)],
            Some(Width::Word),
        ));
        assert_round_trips(instr(
            Mnemonic::Inc,
            vec![Operand::mem(Some(Reg16::Bx), None, 0)],
            Some(Width::Byte),
        ));
        assert_round_trips(instr(
            Mnemonic::Dec,
            vec![Operand::mem(Some(Reg16::Bx), None, 0)],
            Some(Width::Word),
        ));
    }

    #[test]
    fn push_pop_register_segment_and_memory_forms_round_trip() {
        assert_round_trips(instr(
            Mnemonic::Push,
            vec![Operand::Reg16(Reg16::Bx)],
            Some(Width::Word),
        ));
        assert_round_trips(instr(
            Mnemonic::Pop,
            vec![Operand::Reg16(Reg16::Bx)],
            Some(Width::Word),
        ));
        assert_round_trips(instr(
            Mnemonic::Push,
            vec![Operand::Reg16(Reg16::Es)],
            Some(Width::Word),
        ));
        assert_round_trips(instr(
            Mnemonic::Pop,
            vec![Operand::Reg16(Reg16::Ds)],
            Some(Width::Word),
        ));
        assert_round_trips(instr(
            Mnemonic::Push,
            vec![Operand::mem(Some(Reg16::Bx), None, 0)],
            Some(Width::Word),
        ));
    }

    #[test]
    fn pop_cs_is_rejected() {
        let i = instr(
            Mnemonic::Pop,
            vec![Operand::Reg16(Reg16::Cs)],
            Some(Width::Word),
        );
        assert!(encode_one(&i).is_err());
    }

    #[test]
    fn xchg_ax_form_and_general_form_round_trip() {
        assert_round_trips(instr(
            Mnemonic::Xchg,
            vec![Operand::Reg16(Reg16::Ax), Operand::Reg16(Reg16::Bx)],
            Some(Width::Word),
        ));
        assert_round_trips(instr(
            Mnemonic::Xchg,
            vec![Operand::Reg8(Reg8::Al), Operand::Reg8(Reg8::Cl)],
            Some(Width::Byte),
        ));
    }

    #[test]
    fn xchg_ax_ax_matches_nop() {
        let i = instr(
            Mnemonic::Xchg,
            vec![Operand::Reg16(Reg16::Ax), Operand::Reg16(Reg16::Ax)],
            Some(Width::Word),
        );
        assert_eq!(encode_one(&i).unwrap(), vec![0x90]);
    }

    #[test]
    fn test_register_and_accumulator_forms_round_trip() {
        assert_round_trips(instr(
            Mnemonic::Test,
            vec![Operand::Reg16(Reg16::Ax), Operand::Reg16(Reg16::Bx)],
            Some(Width::Word),
        ));
        assert_round_trips(instr(
            Mnemonic::Test,
            vec![Operand::Reg8(Reg8::Al), Operand::Immediate(0x0F)],
            Some(Width::Byte),
        ));
    }

    #[test]
    fn test_non_accumulator_immediate_is_rejected() {
        let i = instr(
            Mnemonic::Test,
            vec![Operand::Reg16(Reg16::Bx), Operand::Immediate(5)],
            Some(Width::Word),
        );
        assert!(encode_one(&i).is_err(), "the F6/F7 group isn't implemented, so this must be a clear error, not silently wrong bytes");
    }

    #[test]
    fn conditional_jump_and_loop_family_round_trip() {
        assert_round_trips(instr(
            Mnemonic::Jcc(Condition::Equal),
            vec![Operand::Immediate(-4)],
            None,
        ));
        assert_round_trips(instr(Mnemonic::Loop, vec![Operand::Immediate(-10)], None));
        assert_round_trips(instr(Mnemonic::Jcxz, vec![Operand::Immediate(2)], None));
    }

    #[test]
    fn conditional_jump_out_of_range_is_an_error() {
        let i = instr(
            Mnemonic::Jcc(Condition::Equal),
            vec![Operand::Immediate(200)],
            None,
        );
        assert!(encode_one(&i).is_err());
    }

    #[test]
    fn jmp_short_and_near_both_round_trip_based_on_byte_len() {
        let short = Instruction::new(Mnemonic::Jmp, vec![Operand::Immediate(-2)], None, 2);
        let bytes = encode_one(&short).unwrap();
        assert_eq!(bytes, vec![0xEB, 0xFE]);
        assert_round_trips(short);

        let near = Instruction::new(Mnemonic::Jmp, vec![Operand::Immediate(0x0100)], None, 3);
        let bytes = encode_one(&near).unwrap();
        assert_eq!(bytes[0], 0xE9);
        assert_round_trips(near);
    }

    #[test]
    fn call_and_ret_round_trip() {
        assert_round_trips(instr(
            Mnemonic::Call,
            vec![Operand::Immediate(0x0010)],
            None,
        ));
        assert_round_trips(instr(Mnemonic::Ret, vec![], None));
        assert_round_trips(instr(Mnemonic::Ret, vec![Operand::Immediate(4)], None));
    }

    #[test]
    fn int_and_processor_control_round_trip() {
        assert_round_trips(instr(Mnemonic::Int, vec![Operand::Immediate(0x21)], None));
        assert_round_trips(instr(Mnemonic::Int3, vec![], None));
        assert_round_trips(instr(Mnemonic::Iret, vec![], None));
        assert_round_trips(instr(Mnemonic::Hlt, vec![], None));
        assert_round_trips(instr(Mnemonic::Clc, vec![], None));
        assert_round_trips(instr(Mnemonic::Pushf, vec![], None));
        assert_round_trips(instr(Mnemonic::Popf, vec![], None));
        assert_round_trips(instr(Mnemonic::Lahf, vec![], None));
        assert_round_trips(instr(Mnemonic::Sahf, vec![], None));
        assert_round_trips(instr(Mnemonic::Xlat, vec![], None));
    }

    #[test]
    fn bare_bp_memory_operand_round_trips() {
        // The irregular mod=01,disp8=0 encoding for a bare [BP].
        let i = instr(
            Mnemonic::Mov,
            vec![
                Operand::Reg16(Reg16::Ax),
                Operand::mem(Some(Reg16::Bp), None, 0),
            ],
            Some(Width::Word),
        );
        let bytes = encode_one(&i).unwrap();
        assert_eq!(bytes.len(), 3); // opcode + modrm + disp8, not just opcode + modrm
        assert_round_trips(i);
    }

    #[test]
    fn direct_address_memory_operand_round_trips() {
        let i = instr(
            Mnemonic::Mov,
            vec![Operand::Reg16(Reg16::Ax), Operand::mem_direct(0x1234)],
            Some(Width::Word),
        );
        assert_round_trips(i);
    }

    #[test]
    fn value_out_of_width_range_is_rejected_not_silently_truncated() {
        let i = instr(
            Mnemonic::Mov,
            vec![Operand::Reg8(Reg8::Al), Operand::Immediate(300)],
            Some(Width::Byte),
        );
        assert!(encode_one(&i).is_err());
    }

    // --- shift/rotate group -------------------------------------------

    #[test]
    fn shift_by_one_encodes_as_d0_d1_and_round_trips() {
        assert_round_trips(instr(
            Mnemonic::Shl,
            vec![Operand::Reg8(Reg8::Al), Operand::Immediate(1)],
            Some(Width::Byte),
        ));
        let i = instr(
            Mnemonic::Sar,
            vec![Operand::Reg16(Reg16::Dx), Operand::Immediate(1)],
            Some(Width::Word),
        );
        assert_eq!(encode_one(&i).unwrap()[0], 0xD1);
        assert_round_trips(i);
    }

    #[test]
    fn shift_by_cl_encodes_as_d2_d3_and_round_trips() {
        let i = instr(
            Mnemonic::Rol,
            vec![Operand::Reg8(Reg8::Al), Operand::Reg8(Reg8::Cl)],
            Some(Width::Byte),
        );
        assert_eq!(encode_one(&i).unwrap()[0], 0xD2);
        assert_round_trips(i);
    }

    #[test]
    fn shift_by_immediate_encodes_as_the_80186_c0_c1_form_and_round_trips() {
        let i = instr(
            Mnemonic::Rcr,
            vec![Operand::Reg16(Reg16::Bx), Operand::Immediate(5)],
            Some(Width::Word),
        );
        let bytes = encode_one(&i).unwrap();
        assert_eq!(bytes[0], 0xC1);
        assert_round_trips(i);
    }

    #[test]
    fn all_eight_shift_rotate_reg_field_forms_round_trip() {
        for mnemonic in [
            Mnemonic::Rol,
            Mnemonic::Ror,
            Mnemonic::Rcl,
            Mnemonic::Rcr,
            Mnemonic::Shl,
            Mnemonic::Shr,
            Mnemonic::Sar,
        ] {
            assert_round_trips(instr(
                mnemonic,
                vec![Operand::Reg16(Reg16::Ax), Operand::Immediate(1)],
                Some(Width::Word),
            ));
        }
    }

    // --- F6/F7 unary group ----------------------------------------------

    #[test]
    fn mul_imul_div_idiv_neg_not_round_trip() {
        for mnemonic in [
            Mnemonic::Mul,
            Mnemonic::Imul,
            Mnemonic::Div,
            Mnemonic::Idiv,
            Mnemonic::Neg,
            Mnemonic::Not,
        ] {
            assert_round_trips(instr(
                mnemonic,
                vec![Operand::Reg16(Reg16::Bx)],
                Some(Width::Word),
            ));
            assert_round_trips(instr(
                mnemonic,
                vec![Operand::Reg8(Reg8::Bl)],
                Some(Width::Byte),
            ));
        }
    }

    #[test]
    fn div_memory_operand_round_trips() {
        assert_round_trips(instr(
            Mnemonic::Div,
            vec![Operand::mem(Some(Reg16::Bx), None, 0)],
            Some(Width::Word),
        ));
    }

    // --- string instructions + REP prefix --------------------------------

    #[test]
    fn all_ten_string_instructions_round_trip() {
        for mnemonic in [
            Mnemonic::Movsb,
            Mnemonic::Movsw,
            Mnemonic::Cmpsb,
            Mnemonic::Cmpsw,
            Mnemonic::Stosb,
            Mnemonic::Stosw,
            Mnemonic::Lodsb,
            Mnemonic::Lodsw,
            Mnemonic::Scasb,
            Mnemonic::Scasw,
        ] {
            assert_round_trips(instr(mnemonic, vec![], None));
        }
    }

    #[test]
    fn rep_movsb_encodes_with_the_f3_prefix_byte_and_round_trips() {
        let i = instr(Mnemonic::Movsb, vec![], None).with_repeat(Repeat::Rep);
        let bytes = encode_one(&i).unwrap();
        assert_eq!(bytes, vec![0xF3, 0xA4]);
        assert_round_trips(i);
    }

    #[test]
    fn repe_cmpsb_encodes_with_the_f3_prefix_byte_and_round_trips_as_repe() {
        let i = instr(Mnemonic::Cmpsb, vec![], None).with_repeat(Repeat::Repe);
        let bytes = encode_one(&i).unwrap();
        assert_eq!(bytes, vec![0xF3, 0xA6]);
        assert_round_trips(i);
    }

    #[test]
    fn repne_scasb_encodes_with_the_f2_prefix_byte_and_round_trips() {
        let i = instr(Mnemonic::Scasb, vec![], None).with_repeat(Repeat::Repne);
        let bytes = encode_one(&i).unwrap();
        assert_eq!(bytes, vec![0xF2, 0xAE]);
        assert_round_trips(i);
    }
}
