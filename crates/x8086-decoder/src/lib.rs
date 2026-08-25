//! Pure decode: bytes -> `x8086_isa::Instruction`.
//!
//! No execution semantics live here, only recognizing byte sequences and
//! producing the shared instruction model. Coverage so far: MOV/PUSH/POP/
//! XCHG/LEA, the ADD/OR/ADC/SBB/AND/SUB/XOR/CMP group (register and
//! immediate forms) plus TEST, INC/DEC, unconditional/conditional/short
//! jumps, the LOOP family, near CALL/RET, INT/INT3/IRET, the
//! processor-control instructions (HLT, NOP, flag-control), the shift/
//! rotate group (D0-D3, and the 80186 C0/C1 immediate-count form), the
//! F6/F7 unary group (TEST/NOT/NEG/MUL/IMUL/DIV/IDIV), the string
//! instructions (MOVS/CMPS/STOS/LODS/SCAS) and the REP/REPE/REPNE
//! prefixes that repeat them. Segment-override/LOCK prefixes, far JMP/
//! CALL, and indirect JMP/CALL through memory are not decoded yet.

mod groups;
mod modrm;
mod text;

pub use text::format_instruction;

use modrm::decode_modrm;
use x8086_isa::{Condition, Instruction, Mnemonic, Operand, Reg16, Reg8, Repeat, Width};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecodeError {
    /// Ran out of bytes while decoding an instruction that needed more.
    UnexpectedEndOfInput,
    /// The opcode byte (or an opcode-extension field within it) doesn't
    /// map to any known 8086/80186 instruction.
    InvalidOpcode(u8),
}

/// Reads a raw little-endian byte/word starting at `offset` and stores it
/// via a sign-extending cast. This is a deliberate convention, not a bug:
/// every consumer recovers the original bit pattern with a *truncating*
/// cast back to `u8`/`u16` (e.g. `imm as u16`), and two's-complement
/// truncation is bit-identical whether the intermediate value was
/// produced by a sign-extending or zero-extending read. It lets one pair
/// of helpers serve both genuinely-signed displacements and raw
/// bit-pattern fields (addresses, interrupt numbers, pop counts) without
/// two parallel sets of read functions.
pub(crate) fn read_i8(bytes: &[u8], offset: usize) -> Result<i8, DecodeError> {
    bytes
        .get(offset)
        .map(|&b| b as i8)
        .ok_or(DecodeError::UnexpectedEndOfInput)
}

pub(crate) fn read_i16(bytes: &[u8], offset: usize) -> Result<i16, DecodeError> {
    let low = *bytes.get(offset).ok_or(DecodeError::UnexpectedEndOfInput)?;
    let high = *bytes
        .get(offset + 1)
        .ok_or(DecodeError::UnexpectedEndOfInput)?;
    Ok(i16::from_le_bytes([low, high]))
}

#[derive(Debug, Clone, Copy)]
enum Direction {
    /// The ModRM `reg` field is the destination; `r/m` is the source.
    ToReg,
    /// The ModRM `r/m` field is the destination; `reg` is the source.
    ToRm,
}

#[derive(Debug, Clone, Copy)]
enum ImmSize {
    Imm8,
    Imm16,
    /// An imm8 that gets sign-extended to 16 bits at execution time - the
    /// `as i32` truncating-cast convention (see `read_i8`) means no extra
    /// work is needed here versus plain `Imm8`.
    Imm8SignExtended,
}

fn reg_operand_from_field(field: u8, width: Width) -> Operand {
    match width {
        Width::Byte => Operand::Reg8(Reg8::from_index(field)),
        Width::Word => Operand::Reg16(Reg16::from_index(field)),
    }
}

fn condition_from_index(index: u8) -> Condition {
    match index & 0b1111 {
        0x0 => Condition::Overflow,
        0x1 => Condition::NotOverflow,
        0x2 => Condition::Below,
        0x3 => Condition::AboveOrEqual,
        0x4 => Condition::Equal,
        0x5 => Condition::NotEqual,
        0x6 => Condition::BelowOrEqual,
        0x7 => Condition::Above,
        0x8 => Condition::Sign,
        0x9 => Condition::NotSign,
        0xA => Condition::Parity,
        0xB => Condition::NotParity,
        0xC => Condition::Less,
        0xD => Condition::GreaterOrEqual,
        0xE => Condition::LessOrEqual,
        0xF => Condition::Greater,
        _ => unreachable!("index & 0b1111 is always in 0..=15"),
    }
}

/// `ADD=0x00, OR=0x08, ADC=0x10, SBB=0x18, AND=0x20, SUB=0x28, XOR=0x30,
/// CMP=0x38`, each with 6 forms (`+0` r/m8,r8 ... `+5` AX,imm16). Opcodes
/// `+6`/`+7` within each base are segment PUSH/POP or override prefixes,
/// not part of this pattern, so the check excludes them.
fn is_arithmetic_group_opcode(opcode: u8) -> bool {
    opcode < 0x40 && (opcode & 0b111) <= 5
}

fn decode_modrm_two_operand(
    mnemonic: Mnemonic,
    bytes: &[u8],
    width: Width,
    direction: Direction,
) -> Result<(Instruction, usize), DecodeError> {
    let modrm = decode_modrm(&bytes[1..], width)?;
    let reg_operand = reg_operand_from_field(modrm.reg_field, width);
    let (dst, src) = match direction {
        Direction::ToReg => (reg_operand, modrm.rm_operand),
        Direction::ToRm => (modrm.rm_operand, reg_operand),
    };
    let len = 1 + modrm.consumed;
    Ok((
        Instruction::new(mnemonic, vec![dst, src], Some(width), len as u8),
        len,
    ))
}

fn decode_acc_imm(
    mnemonic: Mnemonic,
    bytes: &[u8],
    width: Width,
) -> Result<(Instruction, usize), DecodeError> {
    match width {
        Width::Byte => {
            let imm = read_i8(bytes, 1)? as i32;
            let acc = Operand::Reg8(Reg8::Al);
            Ok((
                Instruction::new(mnemonic, vec![acc, Operand::Immediate(imm)], Some(width), 2),
                2,
            ))
        }
        Width::Word => {
            let imm = read_i16(bytes, 1)? as i32;
            let acc = Operand::Reg16(Reg16::Ax);
            Ok((
                Instruction::new(mnemonic, vec![acc, Operand::Immediate(imm)], Some(width), 3),
                3,
            ))
        }
    }
}

fn decode_arithmetic_group_form(
    opcode: u8,
    bytes: &[u8],
) -> Result<(Instruction, usize), DecodeError> {
    let mnemonic = groups::ARITHMETIC_GROUP[((opcode >> 3) & 0b111) as usize];
    match opcode & 0b111 {
        0 => decode_modrm_two_operand(mnemonic, bytes, Width::Byte, Direction::ToRm),
        1 => decode_modrm_two_operand(mnemonic, bytes, Width::Word, Direction::ToRm),
        2 => decode_modrm_two_operand(mnemonic, bytes, Width::Byte, Direction::ToReg),
        3 => decode_modrm_two_operand(mnemonic, bytes, Width::Word, Direction::ToReg),
        4 => decode_acc_imm(mnemonic, bytes, Width::Byte),
        5 => decode_acc_imm(mnemonic, bytes, Width::Word),
        _ => unreachable!("is_arithmetic_group_opcode guarantees the low 3 bits are <= 5"),
    }
}

fn decode_immediate_group(
    bytes: &[u8],
    width: Width,
    imm_size: ImmSize,
) -> Result<(Instruction, usize), DecodeError> {
    let modrm = decode_modrm(&bytes[1..], width)?;
    let mnemonic = groups::arithmetic_group_from_reg_field(modrm.reg_field);
    let imm_offset = 1 + modrm.consumed;
    let (imm, imm_len) = match imm_size {
        ImmSize::Imm8 | ImmSize::Imm8SignExtended => (read_i8(bytes, imm_offset)? as i32, 1),
        ImmSize::Imm16 => (read_i16(bytes, imm_offset)? as i32, 2),
    };
    let len = imm_offset + imm_len;
    Ok((
        Instruction::new(
            mnemonic,
            vec![modrm.rm_operand, Operand::Immediate(imm)],
            Some(width),
            len as u8,
        ),
        len,
    ))
}

fn decode_mov_sreg(
    bytes: &[u8],
    opcode: u8,
    direction: Direction,
) -> Result<(Instruction, usize), DecodeError> {
    let modrm = decode_modrm(&bytes[1..], Width::Word)?;
    let segment =
        Reg16::segment_from_index(modrm.reg_field).ok_or(DecodeError::InvalidOpcode(opcode))?;
    let seg_operand = Operand::Reg16(segment);
    let (dst, src) = match direction {
        Direction::ToReg => (seg_operand, modrm.rm_operand),
        Direction::ToRm => (modrm.rm_operand, seg_operand),
    };
    let len = 1 + modrm.consumed;
    Ok((
        Instruction::new(Mnemonic::Mov, vec![dst, src], Some(Width::Word), len as u8),
        len,
    ))
}

fn decode_lea(bytes: &[u8]) -> Result<(Instruction, usize), DecodeError> {
    let modrm = decode_modrm(&bytes[1..], Width::Word)?;
    let dst = Operand::Reg16(Reg16::from_index(modrm.reg_field));
    let len = 1 + modrm.consumed;
    Ok((
        Instruction::new(
            Mnemonic::Lea,
            vec![dst, modrm.rm_operand],
            Some(Width::Word),
            len as u8,
        ),
        len,
    ))
}

fn decode_pop_rm(bytes: &[u8], opcode: u8) -> Result<(Instruction, usize), DecodeError> {
    let modrm = decode_modrm(&bytes[1..], Width::Word)?;
    if modrm.reg_field != 0 {
        return Err(DecodeError::InvalidOpcode(opcode));
    }
    let len = 1 + modrm.consumed;
    Ok((
        Instruction::new(
            Mnemonic::Pop,
            vec![modrm.rm_operand],
            Some(Width::Word),
            len as u8,
        ),
        len,
    ))
}

fn decode_mov_acc_mem(
    bytes: &[u8],
    width: Width,
    direction: Direction,
) -> Result<(Instruction, usize), DecodeError> {
    let addr = read_i16(bytes, 1)? as i32;
    let mem_operand = Operand::mem_direct(addr);
    let acc = match width {
        Width::Byte => Operand::Reg8(Reg8::Al),
        Width::Word => Operand::Reg16(Reg16::Ax),
    };
    let (dst, src) = match direction {
        Direction::ToReg => (acc, mem_operand),
        Direction::ToRm => (mem_operand, acc),
    };
    Ok((
        Instruction::new(Mnemonic::Mov, vec![dst, src], Some(width), 3),
        3,
    ))
}

fn decode_mov_reg_imm(
    opcode: u8,
    bytes: &[u8],
    width: Width,
) -> Result<(Instruction, usize), DecodeError> {
    match width {
        Width::Byte => {
            let reg = Reg8::from_index(opcode - 0xB0);
            let imm = read_i8(bytes, 1)? as i32;
            Ok((
                Instruction::new(
                    Mnemonic::Mov,
                    vec![Operand::Reg8(reg), Operand::Immediate(imm)],
                    Some(width),
                    2,
                ),
                2,
            ))
        }
        Width::Word => {
            let reg = Reg16::from_index(opcode - 0xB8);
            let imm = read_i16(bytes, 1)? as i32;
            Ok((
                Instruction::new(
                    Mnemonic::Mov,
                    vec![Operand::Reg16(reg), Operand::Immediate(imm)],
                    Some(width),
                    3,
                ),
                3,
            ))
        }
    }
}

fn decode_mov_rm_imm(
    bytes: &[u8],
    width: Width,
    opcode: u8,
) -> Result<(Instruction, usize), DecodeError> {
    let modrm = decode_modrm(&bytes[1..], width)?;
    if modrm.reg_field != 0 {
        return Err(DecodeError::InvalidOpcode(opcode));
    }
    let imm_offset = 1 + modrm.consumed;
    let (imm, imm_len) = match width {
        Width::Byte => (read_i8(bytes, imm_offset)? as i32, 1),
        Width::Word => (read_i16(bytes, imm_offset)? as i32, 2),
    };
    let len = imm_offset + imm_len;
    Ok((
        Instruction::new(
            Mnemonic::Mov,
            vec![modrm.rm_operand, Operand::Immediate(imm)],
            Some(width),
            len as u8,
        ),
        len,
    ))
}

fn decode_inc_dec_rm_byte(bytes: &[u8], opcode: u8) -> Result<(Instruction, usize), DecodeError> {
    let modrm = decode_modrm(&bytes[1..], Width::Byte)?;
    let mnemonic = match modrm.reg_field {
        0 => Mnemonic::Inc,
        1 => Mnemonic::Dec,
        _ => return Err(DecodeError::InvalidOpcode(opcode)),
    };
    let len = 1 + modrm.consumed;
    Ok((
        Instruction::new(
            mnemonic,
            vec![modrm.rm_operand],
            Some(Width::Byte),
            len as u8,
        ),
        len,
    ))
}

/// The `FFh` group also covers indirect/far CALL and JMP (`/2`-`/5`),
/// which aren't decoded yet - only `/0` INC, `/1` DEC, and `/6` PUSH are.
fn decode_group_ff(bytes: &[u8], opcode: u8) -> Result<(Instruction, usize), DecodeError> {
    let modrm = decode_modrm(&bytes[1..], Width::Word)?;
    let len = 1 + modrm.consumed;
    let mnemonic = match modrm.reg_field {
        0 => Mnemonic::Inc,
        1 => Mnemonic::Dec,
        6 => Mnemonic::Push,
        _ => return Err(DecodeError::InvalidOpcode(opcode)),
    };
    Ok((
        Instruction::new(
            mnemonic,
            vec![modrm.rm_operand],
            Some(Width::Word),
            len as u8,
        ),
        len,
    ))
}

fn decode_rel8_branch(
    mnemonic: Mnemonic,
    bytes: &[u8],
) -> Result<(Instruction, usize), DecodeError> {
    let rel = read_i8(bytes, 1)? as i32;
    Ok((
        Instruction::new(mnemonic, vec![Operand::Immediate(rel)], None, 2),
        2,
    ))
}

fn decode_rel16_branch(
    mnemonic: Mnemonic,
    bytes: &[u8],
) -> Result<(Instruction, usize), DecodeError> {
    let rel = read_i16(bytes, 1)? as i32;
    Ok((
        Instruction::new(mnemonic, vec![Operand::Immediate(rel)], None, 3),
        3,
    ))
}

fn decode_conditional_jump(opcode: u8, bytes: &[u8]) -> Result<(Instruction, usize), DecodeError> {
    let condition = condition_from_index(opcode);
    decode_rel8_branch(Mnemonic::Jcc(condition), bytes)
}

fn decode_ret_imm(bytes: &[u8]) -> Result<(Instruction, usize), DecodeError> {
    let imm = read_i16(bytes, 1)? as i32;
    Ok((
        Instruction::new(Mnemonic::Ret, vec![Operand::Immediate(imm)], None, 3),
        3,
    ))
}

fn decode_int(bytes: &[u8]) -> Result<(Instruction, usize), DecodeError> {
    let number = read_i8(bytes, 1)? as i32;
    Ok((
        Instruction::new(Mnemonic::Int, vec![Operand::Immediate(number)], None, 2),
        2,
    ))
}

fn simple(mnemonic: Mnemonic) -> Result<(Instruction, usize), DecodeError> {
    Ok((Instruction::new(mnemonic, vec![], None, 1), 1))
}

/// Where a shift/rotate instruction's count operand comes from: an
/// implicit `1` (`D0`/`D1`), the `CL` register (`D2`/`D3`), or an 80186
/// immediate byte (`C0`/`C1`).
#[derive(Debug, Clone, Copy)]
enum ShiftCountSource {
    One,
    Cl,
    Imm8,
}

fn decode_shift_rotate(
    bytes: &[u8],
    width: Width,
    count_source: ShiftCountSource,
) -> Result<(Instruction, usize), DecodeError> {
    let modrm = decode_modrm(&bytes[1..], width)?;
    let mnemonic = groups::shift_rotate_group_from_reg_field(modrm.reg_field);
    let modrm_len = 1 + modrm.consumed;
    let (count_operand, len) = match count_source {
        ShiftCountSource::One => (Operand::Immediate(1), modrm_len),
        ShiftCountSource::Cl => (Operand::Reg8(Reg8::Cl), modrm_len),
        ShiftCountSource::Imm8 => {
            let imm = read_i8(bytes, modrm_len)? as i32;
            (Operand::Immediate(imm), modrm_len + 1)
        }
    };
    Ok((
        Instruction::new(
            mnemonic,
            vec![modrm.rm_operand, count_operand],
            Some(width),
            len as u8,
        ),
        len,
    ))
}

/// The `F6`/`F7` unary group: `TEST r/m, imm` (reg field 0/1), or the
/// single-operand `NOT`/`NEG`/`MUL`/`IMUL`/`DIV`/`IDIV` (reg fields 2-7).
fn decode_unary_group(bytes: &[u8], width: Width) -> Result<(Instruction, usize), DecodeError> {
    let modrm = decode_modrm(&bytes[1..], width)?;
    let mnemonic = groups::unary_group_from_reg_field(modrm.reg_field);
    let modrm_len = 1 + modrm.consumed;
    if mnemonic == Mnemonic::Test {
        let (imm, imm_len) = match width {
            Width::Byte => (read_i8(bytes, modrm_len)? as i32, 1),
            Width::Word => (read_i16(bytes, modrm_len)? as i32, 2),
        };
        let len = modrm_len + imm_len;
        return Ok((
            Instruction::new(
                mnemonic,
                vec![modrm.rm_operand, Operand::Immediate(imm)],
                Some(width),
                len as u8,
            ),
            len,
        ));
    }
    Ok((
        Instruction::new(
            mnemonic,
            vec![modrm.rm_operand],
            Some(width),
            modrm_len as u8,
        ),
        modrm_len,
    ))
}

fn is_string_mnemonic(mnemonic: Mnemonic) -> bool {
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

/// `REPNE`/`REPNZ` is always `0xF2`. `0xF3` is `REP` on MOVS/STOS/LODS but
/// means `REPE`/`REPZ` on CMPS/SCAS - the meaning genuinely depends on the
/// trailing opcode, not the prefix byte, matching real 8086 semantics.
fn decode_repeat_prefix(prefix: u8, bytes: &[u8]) -> Result<(Instruction, usize), DecodeError> {
    let (inner, inner_len) = decode_one(&bytes[1..])?;
    if !is_string_mnemonic(inner.mnemonic) {
        return Err(DecodeError::InvalidOpcode(prefix));
    }
    let repeat = if prefix == 0xF2 {
        Repeat::Repne
    } else if matches!(
        inner.mnemonic,
        Mnemonic::Cmpsb | Mnemonic::Cmpsw | Mnemonic::Scasb | Mnemonic::Scasw
    ) {
        Repeat::Repe
    } else {
        Repeat::Rep
    };
    let len = 1 + inner_len;
    let mut instr = inner.with_repeat(repeat);
    instr.byte_len = len as u8;
    Ok((instr, len))
}

/// Decode a single instruction starting at the front of `bytes`.
/// Returns the instruction and how many bytes it consumed.
pub fn decode_one(bytes: &[u8]) -> Result<(Instruction, usize), DecodeError> {
    let opcode = *bytes.first().ok_or(DecodeError::UnexpectedEndOfInput)?;

    if opcode == 0xF2 || opcode == 0xF3 {
        return decode_repeat_prefix(opcode, bytes);
    }

    if is_arithmetic_group_opcode(opcode) {
        return decode_arithmetic_group_form(opcode, bytes);
    }

    match opcode {
        0x06 => simple_reg16(Mnemonic::Push, Reg16::Es),
        0x07 => simple_reg16(Mnemonic::Pop, Reg16::Es),
        0x0E => simple_reg16(Mnemonic::Push, Reg16::Cs),
        0x16 => simple_reg16(Mnemonic::Push, Reg16::Ss),
        0x17 => simple_reg16(Mnemonic::Pop, Reg16::Ss),
        0x1E => simple_reg16(Mnemonic::Push, Reg16::Ds),
        0x1F => simple_reg16(Mnemonic::Pop, Reg16::Ds),

        0x40..=0x47 => Ok((
            Instruction::new(
                Mnemonic::Inc,
                vec![Operand::Reg16(Reg16::from_index(opcode - 0x40))],
                Some(Width::Word),
                1,
            ),
            1,
        )),
        0x48..=0x4F => Ok((
            Instruction::new(
                Mnemonic::Dec,
                vec![Operand::Reg16(Reg16::from_index(opcode - 0x48))],
                Some(Width::Word),
                1,
            ),
            1,
        )),
        0x50..=0x57 => Ok((
            Instruction::new(
                Mnemonic::Push,
                vec![Operand::Reg16(Reg16::from_index(opcode - 0x50))],
                Some(Width::Word),
                1,
            ),
            1,
        )),
        0x58..=0x5F => Ok((
            Instruction::new(
                Mnemonic::Pop,
                vec![Operand::Reg16(Reg16::from_index(opcode - 0x58))],
                Some(Width::Word),
                1,
            ),
            1,
        )),

        0x70..=0x7F => decode_conditional_jump(opcode, bytes),

        0x80 => decode_immediate_group(bytes, Width::Byte, ImmSize::Imm8),
        0x81 => decode_immediate_group(bytes, Width::Word, ImmSize::Imm16),
        0x82 => decode_immediate_group(bytes, Width::Byte, ImmSize::Imm8),
        0x83 => decode_immediate_group(bytes, Width::Word, ImmSize::Imm8SignExtended),

        0x84 => decode_modrm_two_operand(Mnemonic::Test, bytes, Width::Byte, Direction::ToRm),
        0x85 => decode_modrm_two_operand(Mnemonic::Test, bytes, Width::Word, Direction::ToRm),
        0x86 => decode_modrm_two_operand(Mnemonic::Xchg, bytes, Width::Byte, Direction::ToRm),
        0x87 => decode_modrm_two_operand(Mnemonic::Xchg, bytes, Width::Word, Direction::ToRm),
        0x88 => decode_modrm_two_operand(Mnemonic::Mov, bytes, Width::Byte, Direction::ToRm),
        0x89 => decode_modrm_two_operand(Mnemonic::Mov, bytes, Width::Word, Direction::ToRm),
        0x8A => decode_modrm_two_operand(Mnemonic::Mov, bytes, Width::Byte, Direction::ToReg),
        0x8B => decode_modrm_two_operand(Mnemonic::Mov, bytes, Width::Word, Direction::ToReg),
        0x8C => decode_mov_sreg(bytes, opcode, Direction::ToRm),
        0x8D => decode_lea(bytes),
        0x8E => decode_mov_sreg(bytes, opcode, Direction::ToReg),
        0x8F => decode_pop_rm(bytes, opcode),

        0x90 => simple(Mnemonic::Nop),
        0x91..=0x97 => Ok((
            Instruction::new(
                Mnemonic::Xchg,
                vec![
                    Operand::Reg16(Reg16::Ax),
                    Operand::Reg16(Reg16::from_index(opcode - 0x90)),
                ],
                Some(Width::Word),
                1,
            ),
            1,
        )),

        0x9C => simple(Mnemonic::Pushf),
        0x9D => simple(Mnemonic::Popf),
        0x9E => simple(Mnemonic::Sahf),
        0x9F => simple(Mnemonic::Lahf),

        0xA0 => decode_mov_acc_mem(bytes, Width::Byte, Direction::ToReg),
        0xA1 => decode_mov_acc_mem(bytes, Width::Word, Direction::ToReg),
        0xA2 => decode_mov_acc_mem(bytes, Width::Byte, Direction::ToRm),
        0xA3 => decode_mov_acc_mem(bytes, Width::Word, Direction::ToRm),
        0xA4 => simple(Mnemonic::Movsb),
        0xA5 => simple(Mnemonic::Movsw),
        0xA6 => simple(Mnemonic::Cmpsb),
        0xA7 => simple(Mnemonic::Cmpsw),
        0xA8 => decode_acc_imm(Mnemonic::Test, bytes, Width::Byte),
        0xA9 => decode_acc_imm(Mnemonic::Test, bytes, Width::Word),
        0xAA => simple(Mnemonic::Stosb),
        0xAB => simple(Mnemonic::Stosw),
        0xAC => simple(Mnemonic::Lodsb),
        0xAD => simple(Mnemonic::Lodsw),
        0xAE => simple(Mnemonic::Scasb),
        0xAF => simple(Mnemonic::Scasw),

        0xB0..=0xB7 => decode_mov_reg_imm(opcode, bytes, Width::Byte),
        0xB8..=0xBF => decode_mov_reg_imm(opcode, bytes, Width::Word),

        0xC0 => decode_shift_rotate(bytes, Width::Byte, ShiftCountSource::Imm8),
        0xC1 => decode_shift_rotate(bytes, Width::Word, ShiftCountSource::Imm8),
        0xC2 => decode_ret_imm(bytes),
        0xC3 => simple(Mnemonic::Ret),
        0xC6 => decode_mov_rm_imm(bytes, Width::Byte, opcode),
        0xC7 => decode_mov_rm_imm(bytes, Width::Word, opcode),
        0xCC => simple(Mnemonic::Int3),
        0xCD => decode_int(bytes),
        0xCF => simple(Mnemonic::Iret),

        0xD0 => decode_shift_rotate(bytes, Width::Byte, ShiftCountSource::One),
        0xD1 => decode_shift_rotate(bytes, Width::Word, ShiftCountSource::One),
        0xD2 => decode_shift_rotate(bytes, Width::Byte, ShiftCountSource::Cl),
        0xD3 => decode_shift_rotate(bytes, Width::Word, ShiftCountSource::Cl),
        0xD7 => simple(Mnemonic::Xlat),

        0xE0 => decode_rel8_branch(Mnemonic::Loopne, bytes),
        0xE1 => decode_rel8_branch(Mnemonic::Loope, bytes),
        0xE2 => decode_rel8_branch(Mnemonic::Loop, bytes),
        0xE3 => decode_rel8_branch(Mnemonic::Jcxz, bytes),
        0xE8 => decode_rel16_branch(Mnemonic::Call, bytes),
        0xE9 => decode_rel16_branch(Mnemonic::Jmp, bytes),
        0xEB => decode_rel8_branch(Mnemonic::Jmp, bytes),

        0xF4 => simple(Mnemonic::Hlt),
        0xF5 => simple(Mnemonic::Cmc),
        0xF6 => decode_unary_group(bytes, Width::Byte),
        0xF7 => decode_unary_group(bytes, Width::Word),
        0xF8 => simple(Mnemonic::Clc),
        0xF9 => simple(Mnemonic::Stc),
        0xFA => simple(Mnemonic::Cli),
        0xFB => simple(Mnemonic::Sti),
        0xFC => simple(Mnemonic::Cld),
        0xFD => simple(Mnemonic::Std),
        0xFE => decode_inc_dec_rm_byte(bytes, opcode),
        0xFF => decode_group_ff(bytes, opcode),

        other => Err(DecodeError::InvalidOpcode(other)),
    }
}

fn simple_reg16(mnemonic: Mnemonic, reg: Reg16) -> Result<(Instruction, usize), DecodeError> {
    Ok((
        Instruction::new(mnemonic, vec![Operand::Reg16(reg)], Some(Width::Word), 1),
        1,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_input_is_an_error() {
        assert_eq!(decode_one(&[]), Err(DecodeError::UnexpectedEndOfInput));
    }

    #[test]
    fn unknown_opcode_is_reported() {
        // 0x0F: reserved on 8086/80186, deliberately undecoded.
        assert_eq!(
            decode_one(&[0x0F]).unwrap_err(),
            DecodeError::InvalidOpcode(0x0F)
        );
    }

    #[test]
    fn decodes_hlt_and_processor_control_instructions() {
        for (byte, mnemonic) in [
            (0xF4, Mnemonic::Hlt),
            (0x90, Mnemonic::Nop),
            (0xF8, Mnemonic::Clc),
            (0xF9, Mnemonic::Stc),
            (0xF5, Mnemonic::Cmc),
            (0xFC, Mnemonic::Cld),
            (0xFD, Mnemonic::Std),
            (0xFA, Mnemonic::Cli),
            (0xFB, Mnemonic::Sti),
            (0x9C, Mnemonic::Pushf),
            (0x9D, Mnemonic::Popf),
            (0x9E, Mnemonic::Sahf),
            (0x9F, Mnemonic::Lahf),
            (0xD7, Mnemonic::Xlat),
            (0xC3, Mnemonic::Ret),
            (0xCC, Mnemonic::Int3),
            (0xCF, Mnemonic::Iret),
        ] {
            let (instr, len) = decode_one(&[byte]).unwrap();
            assert_eq!(instr.mnemonic, mnemonic, "opcode {byte:#04x}");
            assert_eq!(len, 1);
            assert!(instr.operands.is_empty());
        }
    }

    #[test]
    fn decodes_mov_reg_to_reg() {
        // MOV CX, AX -> 89 C1 (mod=11 reg=000(AX) rm=001(CX), direction to rm)
        let (instr, len) = decode_one(&[0x89, 0b11_000_001]).unwrap();
        assert_eq!(instr.mnemonic, Mnemonic::Mov);
        assert_eq!(
            instr.operands,
            vec![Operand::Reg16(Reg16::Cx), Operand::Reg16(Reg16::Ax)]
        );
        assert_eq!(instr.width, Some(Width::Word));
        assert_eq!(len, 2);
    }

    #[test]
    fn decodes_mov_reg_imm16() {
        // MOV AX, 0x1234 -> B8 34 12
        let (instr, len) = decode_one(&[0xB8, 0x34, 0x12]).unwrap();
        assert_eq!(instr.mnemonic, Mnemonic::Mov);
        assert_eq!(
            instr.operands,
            vec![Operand::Reg16(Reg16::Ax), Operand::Immediate(0x1234)]
        );
        assert_eq!(len, 3);
    }

    #[test]
    fn decodes_mov_reg_imm8() {
        // MOV AL, 0x42 -> B0 42
        let (instr, len) = decode_one(&[0xB0, 0x42]).unwrap();
        assert_eq!(instr.mnemonic, Mnemonic::Mov);
        assert_eq!(
            instr.operands,
            vec![Operand::Reg8(Reg8::Al), Operand::Immediate(0x42)]
        );
        assert_eq!(len, 2);
    }

    #[test]
    fn decodes_mov_memory_to_accumulator() {
        // MOV AX, [0x0100] -> A1 00 01
        let (instr, len) = decode_one(&[0xA1, 0x00, 0x01]).unwrap();
        assert_eq!(instr.mnemonic, Mnemonic::Mov);
        assert_eq!(
            instr.operands,
            vec![Operand::Reg16(Reg16::Ax), Operand::mem_direct(0x0100)]
        );
        assert_eq!(len, 3);
    }

    #[test]
    fn decodes_mov_rm_immediate() {
        // MOV WORD [BX], 0x0005 -> C7 07 05 00
        let (instr, len) = decode_one(&[0xC7, 0b00_000_111, 0x05, 0x00]).unwrap();
        assert_eq!(instr.mnemonic, Mnemonic::Mov);
        assert_eq!(
            instr.operands,
            vec![
                Operand::mem(Some(Reg16::Bx), None, 0),
                Operand::Immediate(5)
            ]
        );
        assert_eq!(len, 4);
    }

    #[test]
    fn decodes_mov_segment_register() {
        // MOV DS, AX -> 8E D8 (reg field 011 = DS)
        let (instr, len) = decode_one(&[0x8E, 0b11_011_000]).unwrap();
        assert_eq!(instr.mnemonic, Mnemonic::Mov);
        assert_eq!(
            instr.operands,
            vec![Operand::Reg16(Reg16::Ds), Operand::Reg16(Reg16::Ax)]
        );
        assert_eq!(len, 2);
    }

    #[test]
    fn rejects_reserved_segment_register_encoding() {
        // reg field 100 (4) is not a valid segment register.
        assert_eq!(
            decode_one(&[0x8E, 0b11_100_000]).unwrap_err(),
            DecodeError::InvalidOpcode(0x8E)
        );
    }

    #[test]
    fn decodes_lea() {
        // LEA AX, [BX+SI] -> 8D 00
        let (instr, len) = decode_one(&[0x8D, 0b00_000_000]).unwrap();
        assert_eq!(instr.mnemonic, Mnemonic::Lea);
        assert_eq!(
            instr.operands,
            vec![
                Operand::Reg16(Reg16::Ax),
                Operand::mem(Some(Reg16::Bx), Some(Reg16::Si), 0)
            ]
        );
        assert_eq!(len, 2);
    }

    #[test]
    fn decodes_xchg_accumulator_form() {
        // XCHG AX, BX -> 93
        let (instr, len) = decode_one(&[0x93]).unwrap();
        assert_eq!(instr.mnemonic, Mnemonic::Xchg);
        assert_eq!(
            instr.operands,
            vec![Operand::Reg16(Reg16::Ax), Operand::Reg16(Reg16::Bx)]
        );
        assert_eq!(len, 1);
    }

    #[test]
    fn decodes_all_eight_arithmetic_group_register_forms() {
        // opcode base -> mnemonic, using the "+2" (r8,r/m8 -> to reg) form
        // with mod=11 reg=000(AL) rm=001(CL) so it round-trips cleanly.
        let cases = [
            (0x02, Mnemonic::Add),
            (0x0A, Mnemonic::Or),
            (0x12, Mnemonic::Adc),
            (0x1A, Mnemonic::Sbb),
            (0x22, Mnemonic::And),
            (0x2A, Mnemonic::Sub),
            (0x32, Mnemonic::Xor),
            (0x3A, Mnemonic::Cmp),
        ];
        for (opcode, mnemonic) in cases {
            let (instr, len) = decode_one(&[opcode, 0b11_000_001]).unwrap();
            assert_eq!(instr.mnemonic, mnemonic, "opcode {opcode:#04x}");
            assert_eq!(
                instr.operands,
                vec![Operand::Reg8(Reg8::Al), Operand::Reg8(Reg8::Cl)]
            );
            assert_eq!(len, 2);
        }
    }

    #[test]
    fn decodes_add_al_imm8() {
        // ADD AL, 5 -> 04 05
        let (instr, len) = decode_one(&[0x04, 0x05]).unwrap();
        assert_eq!(instr.mnemonic, Mnemonic::Add);
        assert_eq!(
            instr.operands,
            vec![Operand::Reg8(Reg8::Al), Operand::Immediate(5)]
        );
        assert_eq!(len, 2);
    }

    #[test]
    fn decodes_sub_ax_imm16() {
        // SUB AX, 0x0100 -> 2D 00 01
        let (instr, len) = decode_one(&[0x2D, 0x00, 0x01]).unwrap();
        assert_eq!(instr.mnemonic, Mnemonic::Sub);
        assert_eq!(
            instr.operands,
            vec![Operand::Reg16(Reg16::Ax), Operand::Immediate(0x0100)]
        );
        assert_eq!(len, 3);
    }

    #[test]
    fn decodes_immediate_group_16bit_form() {
        // ADD WORD [BX], 0x0010 -> 81 07 10 00 (reg field 000 = ADD)
        let (instr, len) = decode_one(&[0x81, 0b00_000_111, 0x10, 0x00]).unwrap();
        assert_eq!(instr.mnemonic, Mnemonic::Add);
        assert_eq!(
            instr.operands,
            vec![
                Operand::mem(Some(Reg16::Bx), None, 0),
                Operand::Immediate(0x10)
            ]
        );
        assert_eq!(len, 4);
    }

    #[test]
    fn decodes_immediate_group_sign_extended_form() {
        // CMP CX, -1 -> 83 F9 FF (reg field 111 = CMP, rm=001=CX)
        let (instr, len) = decode_one(&[0x83, 0b11_111_001, 0xFF]).unwrap();
        assert_eq!(instr.mnemonic, Mnemonic::Cmp);
        assert_eq!(
            instr.operands,
            vec![Operand::Reg16(Reg16::Cx), Operand::Immediate(-1)]
        );
        assert_eq!(len, 3);
    }

    #[test]
    fn decodes_test_forms() {
        // TEST AL, 0x0F -> A8 0F
        let (instr, len) = decode_one(&[0xA8, 0x0F]).unwrap();
        assert_eq!(instr.mnemonic, Mnemonic::Test);
        assert_eq!(
            instr.operands,
            vec![Operand::Reg8(Reg8::Al), Operand::Immediate(0x0F)]
        );
        assert_eq!(len, 2);
    }

    #[test]
    fn decodes_inc_dec_register_forms() {
        // INC CX -> 41, DEC CX -> 49
        let (inc, inc_len) = decode_one(&[0x41]).unwrap();
        assert_eq!(inc.mnemonic, Mnemonic::Inc);
        assert_eq!(inc.operands, vec![Operand::Reg16(Reg16::Cx)]);
        assert_eq!(inc_len, 1);

        let (dec, dec_len) = decode_one(&[0x49]).unwrap();
        assert_eq!(dec.mnemonic, Mnemonic::Dec);
        assert_eq!(dec.operands, vec![Operand::Reg16(Reg16::Cx)]);
        assert_eq!(dec_len, 1);
    }

    #[test]
    fn decodes_inc_dec_memory_byte_form() {
        // INC BYTE [BX] -> FE 07 (reg field 000 = INC)
        let (instr, len) = decode_one(&[0xFE, 0b00_000_111]).unwrap();
        assert_eq!(instr.mnemonic, Mnemonic::Inc);
        assert_eq!(instr.width, Some(Width::Byte));
        assert_eq!(len, 2);
    }

    #[test]
    fn decodes_group_ff_push_form() {
        // PUSH WORD [BX] -> FF 37 (reg field 110 = PUSH)
        let (instr, len) = decode_one(&[0xFF, 0b00_110_111]).unwrap();
        assert_eq!(instr.mnemonic, Mnemonic::Push);
        assert_eq!(len, 2);
    }

    #[test]
    fn group_ff_indirect_call_form_is_not_yet_supported() {
        // reg field 010 = CALL r/m16 (indirect) - deliberately deferred.
        assert_eq!(
            decode_one(&[0xFF, 0b00_010_111]).unwrap_err(),
            DecodeError::InvalidOpcode(0xFF)
        );
    }

    #[test]
    fn decodes_push_pop_register_forms() {
        let (push, push_len) = decode_one(&[0x53]).unwrap(); // PUSH BX
        assert_eq!(push.mnemonic, Mnemonic::Push);
        assert_eq!(push.operands, vec![Operand::Reg16(Reg16::Bx)]);
        assert_eq!(push_len, 1);

        let (pop, pop_len) = decode_one(&[0x5B]).unwrap(); // POP BX
        assert_eq!(pop.mnemonic, Mnemonic::Pop);
        assert_eq!(pop.operands, vec![Operand::Reg16(Reg16::Bx)]);
        assert_eq!(pop_len, 1);
    }

    #[test]
    fn decodes_push_pop_segment_registers() {
        for (byte, mnemonic, reg) in [
            (0x06, Mnemonic::Push, Reg16::Es),
            (0x07, Mnemonic::Pop, Reg16::Es),
            (0x0E, Mnemonic::Push, Reg16::Cs),
            (0x16, Mnemonic::Push, Reg16::Ss),
            (0x17, Mnemonic::Pop, Reg16::Ss),
            (0x1E, Mnemonic::Push, Reg16::Ds),
            (0x1F, Mnemonic::Pop, Reg16::Ds),
        ] {
            let (instr, len) = decode_one(&[byte]).unwrap();
            assert_eq!(instr.mnemonic, mnemonic, "opcode {byte:#04x}");
            assert_eq!(instr.operands, vec![Operand::Reg16(reg)]);
            assert_eq!(len, 1);
        }
    }

    #[test]
    fn decodes_pop_rm_memory_form() {
        // POP WORD [BX] -> 8F 07
        let (instr, len) = decode_one(&[0x8F, 0b00_000_111]).unwrap();
        assert_eq!(instr.mnemonic, Mnemonic::Pop);
        assert_eq!(len, 2);
    }

    #[test]
    fn decodes_all_sixteen_conditional_jumps() {
        let expected = [
            Condition::Overflow,
            Condition::NotOverflow,
            Condition::Below,
            Condition::AboveOrEqual,
            Condition::Equal,
            Condition::NotEqual,
            Condition::BelowOrEqual,
            Condition::Above,
            Condition::Sign,
            Condition::NotSign,
            Condition::Parity,
            Condition::NotParity,
            Condition::Less,
            Condition::GreaterOrEqual,
            Condition::LessOrEqual,
            Condition::Greater,
        ];
        for (index, condition) in expected.into_iter().enumerate() {
            let opcode = 0x70 + index as u8;
            let (instr, len) = decode_one(&[opcode, 0x02]).unwrap();
            assert_eq!(
                instr.mnemonic,
                Mnemonic::Jcc(condition),
                "opcode {opcode:#04x}"
            );
            assert_eq!(instr.operands, vec![Operand::Immediate(2)]);
            assert_eq!(len, 2);
        }
    }

    #[test]
    fn decodes_negative_relative_jump() {
        // JMP short -2 -> EB FE
        let (instr, len) = decode_one(&[0xEB, 0xFE]).unwrap();
        assert_eq!(instr.mnemonic, Mnemonic::Jmp);
        assert_eq!(instr.operands, vec![Operand::Immediate(-2)]);
        assert_eq!(len, 2);
    }

    #[test]
    fn decodes_near_jmp_and_call() {
        let (jmp, jmp_len) = decode_one(&[0xE9, 0x00, 0x01]).unwrap(); // JMP +0x0100
        assert_eq!(jmp.mnemonic, Mnemonic::Jmp);
        assert_eq!(jmp.operands, vec![Operand::Immediate(0x0100)]);
        assert_eq!(jmp_len, 3);

        let (call, call_len) = decode_one(&[0xE8, 0x00, 0x01]).unwrap(); // CALL +0x0100
        assert_eq!(call.mnemonic, Mnemonic::Call);
        assert_eq!(call.operands, vec![Operand::Immediate(0x0100)]);
        assert_eq!(call_len, 3);
    }

    #[test]
    fn decodes_loop_family() {
        for (byte, mnemonic) in [
            (0xE0, Mnemonic::Loopne),
            (0xE1, Mnemonic::Loope),
            (0xE2, Mnemonic::Loop),
            (0xE3, Mnemonic::Jcxz),
        ] {
            let (instr, len) = decode_one(&[byte, 0xFC]).unwrap(); // rel8 = -4
            assert_eq!(instr.mnemonic, mnemonic, "opcode {byte:#04x}");
            assert_eq!(instr.operands, vec![Operand::Immediate(-4)]);
            assert_eq!(len, 2);
        }
    }

    #[test]
    fn decodes_ret_with_immediate_pop_count() {
        // RET 4 -> C2 04 00
        let (instr, len) = decode_one(&[0xC2, 0x04, 0x00]).unwrap();
        assert_eq!(instr.mnemonic, Mnemonic::Ret);
        assert_eq!(instr.operands, vec![Operand::Immediate(4)]);
        assert_eq!(len, 3);
    }

    #[test]
    fn decodes_int_with_vector_number() {
        // INT 0x21 -> CD 21
        let (instr, len) = decode_one(&[0xCD, 0x21]).unwrap();
        assert_eq!(instr.mnemonic, Mnemonic::Int);
        assert_eq!(instr.operands, vec![Operand::Immediate(0x21)]);
        assert_eq!(len, 2);
    }

    #[test]
    fn decodes_shift_by_one_forms() {
        // SHL AL, 1 -> D0 E0 (reg field 100 = SHL, rm=000=AL)
        let (instr, len) = decode_one(&[0xD0, 0b11_100_000]).unwrap();
        assert_eq!(instr.mnemonic, Mnemonic::Shl);
        assert_eq!(
            instr.operands,
            vec![Operand::Reg8(Reg8::Al), Operand::Immediate(1)]
        );
        assert_eq!(instr.width, Some(Width::Byte));
        assert_eq!(len, 2);
    }

    #[test]
    fn decodes_shift_by_cl_forms() {
        // SAR DX, CL -> D3 FA (reg field 111 = SAR, rm=010=DX)
        let (instr, len) = decode_one(&[0xD3, 0b11_111_010]).unwrap();
        assert_eq!(instr.mnemonic, Mnemonic::Sar);
        assert_eq!(
            instr.operands,
            vec![Operand::Reg16(Reg16::Dx), Operand::Reg8(Reg8::Cl)]
        );
        assert_eq!(len, 2);
    }

    #[test]
    fn decodes_shift_by_immediate_80186_form() {
        // ROL AX, 4 -> C1 C0 04 (reg field 000 = ROL, rm=000=AX)
        let (instr, len) = decode_one(&[0xC1, 0b11_000_000, 0x04]).unwrap();
        assert_eq!(instr.mnemonic, Mnemonic::Rol);
        assert_eq!(
            instr.operands,
            vec![Operand::Reg16(Reg16::Ax), Operand::Immediate(4)]
        );
        assert_eq!(len, 3);
    }

    #[test]
    fn decodes_all_eight_shift_rotate_reg_field_forms() {
        let cases = [
            (0b000, Mnemonic::Rol),
            (0b001, Mnemonic::Ror),
            (0b010, Mnemonic::Rcl),
            (0b011, Mnemonic::Rcr),
            (0b100, Mnemonic::Shl),
            (0b101, Mnemonic::Shr),
            (0b111, Mnemonic::Sar),
        ];
        for (reg_field, mnemonic) in cases {
            let modrm = 0b11_000_000 | (reg_field << 3);
            let (instr, _) = decode_one(&[0xD0, modrm]).unwrap();
            assert_eq!(instr.mnemonic, mnemonic, "reg field {reg_field:#05b}");
        }
    }

    #[test]
    fn decodes_unary_group_mul_imul_div_idiv_neg_not() {
        let cases = [
            (0b010, Mnemonic::Not),
            (0b011, Mnemonic::Neg),
            (0b100, Mnemonic::Mul),
            (0b101, Mnemonic::Imul),
            (0b110, Mnemonic::Div),
            (0b111, Mnemonic::Idiv),
        ];
        for (reg_field, mnemonic) in cases {
            let modrm = 0b11_000_001 | (reg_field << 3); // rm = CX
            let (instr, len) = decode_one(&[0xF7, modrm]).unwrap();
            assert_eq!(instr.mnemonic, mnemonic, "reg field {reg_field:#05b}");
            assert_eq!(instr.operands, vec![Operand::Reg16(Reg16::Cx)]);
            assert_eq!(len, 2);
        }
    }

    #[test]
    fn decodes_unary_group_test_with_immediate() {
        // TEST WORD [BX], 0x00FF -> F7 07 FF 00 (reg field 000 = TEST)
        let (instr, len) = decode_one(&[0xF7, 0b00_000_111, 0xFF, 0x00]).unwrap();
        assert_eq!(instr.mnemonic, Mnemonic::Test);
        assert_eq!(
            instr.operands,
            vec![
                Operand::mem(Some(Reg16::Bx), None, 0),
                Operand::Immediate(0xFF)
            ]
        );
        assert_eq!(len, 4);
    }

    #[test]
    fn decodes_string_instructions_as_zero_operand_forms() {
        for (byte, mnemonic) in [
            (0xA4, Mnemonic::Movsb),
            (0xA5, Mnemonic::Movsw),
            (0xA6, Mnemonic::Cmpsb),
            (0xA7, Mnemonic::Cmpsw),
            (0xAA, Mnemonic::Stosb),
            (0xAB, Mnemonic::Stosw),
            (0xAC, Mnemonic::Lodsb),
            (0xAD, Mnemonic::Lodsw),
            (0xAE, Mnemonic::Scasb),
            (0xAF, Mnemonic::Scasw),
        ] {
            let (instr, len) = decode_one(&[byte]).unwrap();
            assert_eq!(instr.mnemonic, mnemonic, "opcode {byte:#04x}");
            assert!(instr.operands.is_empty());
            assert_eq!(len, 1);
        }
    }

    #[test]
    fn decodes_rep_prefix_on_movsb() {
        // REP MOVSB -> F3 A4
        let (instr, len) = decode_one(&[0xF3, 0xA4]).unwrap();
        assert_eq!(instr.mnemonic, Mnemonic::Movsb);
        assert_eq!(instr.repeat, Some(x8086_isa::Repeat::Rep));
        assert_eq!(len, 2);
    }

    #[test]
    fn f3_prefix_on_cmps_means_repe_not_rep() {
        // REPE CMPSB -> F3 A6 (F3 means REPE, not unconditional REP, on CMPS)
        let (instr, len) = decode_one(&[0xF3, 0xA6]).unwrap();
        assert_eq!(instr.mnemonic, Mnemonic::Cmpsb);
        assert_eq!(instr.repeat, Some(x8086_isa::Repeat::Repe));
        assert_eq!(len, 2);
    }

    #[test]
    fn decodes_repne_prefix_on_scasb() {
        // REPNE SCASB -> F2 AE
        let (instr, len) = decode_one(&[0xF2, 0xAE]).unwrap();
        assert_eq!(instr.mnemonic, Mnemonic::Scasb);
        assert_eq!(instr.repeat, Some(x8086_isa::Repeat::Repne));
        assert_eq!(len, 2);
    }

    #[test]
    fn repeat_prefix_on_a_non_string_instruction_is_rejected() {
        // F3 90 would be a REP NOP - not a real repeatable instruction.
        assert_eq!(
            decode_one(&[0xF3, 0x90]).unwrap_err(),
            DecodeError::InvalidOpcode(0xF3)
        );
    }

    #[test]
    fn truncated_instruction_reports_unexpected_end_of_input() {
        // MOV AX, imm16 with only one immediate byte present.
        assert_eq!(
            decode_one(&[0xB8, 0x34]).unwrap_err(),
            DecodeError::UnexpectedEndOfInput
        );
    }
}
