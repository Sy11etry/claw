//! x86-64 instruction encoders. Every function appends one instruction to a byte vector.
//! `tools/check_encoder_x64.py` compares `golden()` against llvm-mc.
//!
//! Branches and calls leave a zero rel32 behind: the caller patches the last four bytes
//! (`o.len() - 4` right after the call is where they sit).

pub type Reg = u8;
pub type Xmm = u8;

pub const RAX: Reg = 0;
pub const RCX: Reg = 1;
pub const RDX: Reg = 2;
pub const RBX: Reg = 3;
pub const RSP: Reg = 4;
pub const RBP: Reg = 5;
pub const RSI: Reg = 6;
pub const RDI: Reg = 7;
pub const R8: Reg = 8;
pub const R9: Reg = 9;
pub const R10: Reg = 10;
pub const R11: Reg = 11;
pub const R12: Reg = 12;
pub const R13: Reg = 13;
pub const R14: Reg = 14;
pub const R15: Reg = 15;

/// The condition of `jcc`, `setcc` and `cmovcc`; the number is the opcode's low nibble.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cond {
    O = 0,
    No = 1,
    B = 2,
    Ae = 3,
    E = 4,
    Ne = 5,
    Be = 6,
    A = 7,
    S = 8,
    Ns = 9,
    P = 10,
    Np = 11,
    L = 12,
    Ge = 13,
    Le = 14,
    G = 15,
}

impl Cond {
    /// The exact opposite condition (flipping the low bit of the code).
    pub fn not(self) -> Cond {
        const ALL: [Cond; 16] = [Cond::O, Cond::No, Cond::B, Cond::Ae, Cond::E, Cond::Ne, Cond::Be, Cond::A, Cond::S, Cond::Ns, Cond::P, Cond::Np, Cond::L, Cond::Ge, Cond::Le, Cond::G];
        ALL[(self as usize) ^ 1]
    }
}

/// A register or memory operand, the "r/m" of an instruction.
#[derive(Clone, Copy, Debug)]
pub enum Op {
    R(Reg),
    /// `[base + index * scale + disp]`
    M { base: Reg, index: Option<(Reg, u8)>, disp: i32 },
    /// `[rip + disp32]`, with the 32 bits left zero for the caller to patch.
    Rip,
}

pub fn reg(r: Reg) -> Op {
    Op::R(r)
}

pub fn at(base: Reg, disp: i32) -> Op {
    Op::M { base, index: None, disp }
}

pub fn at_idx(base: Reg, index: Reg, scale: u8, disp: i32) -> Op {
    debug_assert!(index != RSP && matches!(scale, 1 | 2 | 4 | 8));
    Op::M { base, index: Some((index, scale)), disp }
}

/// Legacy prefix, REX, opcode bytes, ModRM, SIB and displacement.
/// `byte` says which operands are 8-bit registers: bit 0 the reg field, bit 1 the r/m field
/// (`spl`, `bpl`, `sil` and `dil` need an empty REX to be reachable).
fn emit(o: &mut Vec<u8>, prefix: u8, w: bool, opc: &[u8], reg: u8, rm: Op, byte: u8) {
    let (b, x) = match rm {
        Op::R(r) => (r >> 3, 0),
        Op::M { base, index, .. } => (base >> 3, index.map_or(0, |(i, _)| i >> 3)),
        Op::Rip => (0, 0),
    };
    let rex = 0x40 | ((w as u8) << 3) | (((reg >> 3) & 1) << 2) | (x << 1) | b;
    let low = |r: u8| (4..8).contains(&r);
    let need = rex != 0x40 || (byte & 1 != 0 && low(reg)) || (byte & 2 != 0 && matches!(rm, Op::R(r) if low(r)));
    if prefix != 0 {
        o.push(prefix);
    }
    if need {
        o.push(rex);
    }
    o.extend_from_slice(opc);
    let r = (reg & 7) << 3;
    match rm {
        Op::R(x) => o.push(0xC0 | r | (x & 7)),
        Op::Rip => {
            o.push(r | 5);
            o.extend_from_slice(&[0; 4]);
        }
        Op::M { base, index, disp } => {
            let sib = index.is_some() || base & 7 == 4;
            let md = if disp == 0 && base & 7 != 5 {
                0
            } else if (-128..128).contains(&disp) {
                1
            } else {
                2
            };
            o.push((md << 6) | r | if sib { 4 } else { base & 7 });
            if sib {
                let (i, s) = index.map_or((4, 0), |(i, s)| (i & 7, s.trailing_zeros() as u8));
                o.push((s << 6) | (i << 3) | (base & 7));
            }
            match md {
                1 => o.push(disp as i8 as u8),
                2 => o.extend_from_slice(&disp.to_le_bytes()),
                _ => {}
            }
        }
    }
}

// ---- moves ----

/// `mov dst, src` (64-bit); `dst` may be memory.
pub fn mov(o: &mut Vec<u8>, dst: Op, src: Reg) {
    emit(o, 0, true, &[0x89], src, dst, 0);
}
pub fn mov_load(o: &mut Vec<u8>, dst: Reg, src: Op) {
    emit(o, 0, true, &[0x8B], dst, src, 0);
}
/// 32-bit move: zero-extends into the full register.
pub fn mov32(o: &mut Vec<u8>, dst: Op, src: Reg) {
    emit(o, 0, false, &[0x89], src, dst, 0);
}
pub fn mov32_load(o: &mut Vec<u8>, dst: Reg, src: Op) {
    emit(o, 0, false, &[0x8B], dst, src, 0);
}
pub fn mov16(o: &mut Vec<u8>, dst: Op, src: Reg) {
    emit(o, 0x66, false, &[0x89], src, dst, 0);
}
pub fn mov8(o: &mut Vec<u8>, dst: Op, src: Reg) {
    emit(o, 0, false, &[0x88], src, dst, 1);
}
/// `mov r32, imm32`: zero-extends.
pub fn mov_ri32(o: &mut Vec<u8>, dst: Reg, imm: u32) {
    if dst >= 8 {
        o.push(0x41);
    }
    o.push(0xB8 + (dst & 7));
    o.extend_from_slice(&imm.to_le_bytes());
}
/// `mov r64, simm32`: sign-extends.
pub fn mov_ri_sext(o: &mut Vec<u8>, dst: Reg, imm: i32) {
    emit(o, 0, true, &[0xC7], 0, Op::R(dst), 0);
    o.extend_from_slice(&imm.to_le_bytes());
}
/// `movabs r64, imm64`
pub fn mov_ri64(o: &mut Vec<u8>, dst: Reg, imm: i64) {
    o.push(0x48 | (dst >> 3));
    o.push(0xB8 + (dst & 7));
    o.extend_from_slice(&imm.to_le_bytes());
}
/// `mov qword ptr [m], simm32`
pub fn mov_mi(o: &mut Vec<u8>, dst: Op, imm: i32) {
    emit(o, 0, true, &[0xC7], 0, dst, 0);
    o.extend_from_slice(&imm.to_le_bytes());
}
pub fn lea(o: &mut Vec<u8>, dst: Reg, src: Op) {
    emit(o, 0, true, &[0x8D], dst, src, 0);
}

/// Zero-extending loads and register forms: the result is the full 64-bit register.
pub fn movzx8(o: &mut Vec<u8>, dst: Reg, src: Op) {
    emit(o, 0, false, &[0x0F, 0xB6], dst, src, 2);
}
pub fn movzx16(o: &mut Vec<u8>, dst: Reg, src: Op) {
    emit(o, 0, false, &[0x0F, 0xB7], dst, src, 0);
}
pub fn movsx8(o: &mut Vec<u8>, dst: Reg, src: Op) {
    emit(o, 0, true, &[0x0F, 0xBE], dst, src, 2);
}
pub fn movsx16(o: &mut Vec<u8>, dst: Reg, src: Op) {
    emit(o, 0, true, &[0x0F, 0xBF], dst, src, 0);
}
pub fn movsx32(o: &mut Vec<u8>, dst: Reg, src: Op) {
    emit(o, 0, true, &[0x63], dst, src, 0);
}

pub fn push(o: &mut Vec<u8>, r: Reg) {
    if r >= 8 {
        o.push(0x41);
    }
    o.push(0x50 + (r & 7));
}
pub fn pop(o: &mut Vec<u8>, r: Reg) {
    if r >= 8 {
        o.push(0x41);
    }
    o.push(0x58 + (r & 7));
}

// ---- integer arithmetic ----

/// The eight classic ALU operations, numbered as in the opcode map.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Alu {
    Add = 0,
    Or = 1,
    Adc = 2,
    Sbb = 3,
    And = 4,
    Sub = 5,
    Xor = 6,
    Cmp = 7,
}

/// `op dst, src` on 64-bit registers (or `dst` in memory).
pub fn alu(o: &mut Vec<u8>, op: Alu, dst: Op, src: Reg) {
    emit(o, 0, true, &[(op as u8) * 8 + 1], src, dst, 0);
}
/// `op dst, [src]`
pub fn alu_load(o: &mut Vec<u8>, op: Alu, dst: Reg, src: Op) {
    emit(o, 0, true, &[(op as u8) * 8 + 3], dst, src, 0);
}
/// `op dst, imm`: the 8-bit immediate form when it fits.
pub fn alu_i(o: &mut Vec<u8>, op: Alu, dst: Op, imm: i32) {
    if (-128..128).contains(&imm) {
        emit(o, 0, true, &[0x83], op as u8, dst, 0);
        o.push(imm as i8 as u8);
    } else {
        emit(o, 0, true, &[0x81], op as u8, dst, 0);
        o.extend_from_slice(&imm.to_le_bytes());
    }
}
pub fn test(o: &mut Vec<u8>, a: Reg, b: Reg) {
    emit(o, 0, true, &[0x85], b, Op::R(a), 0);
}
/// `imul dst, src`
pub fn imul(o: &mut Vec<u8>, dst: Reg, src: Op) {
    emit(o, 0, true, &[0x0F, 0xAF], dst, src, 0);
}
/// One-operand forms working on rdx:rax.
pub fn mul1(o: &mut Vec<u8>, src: Op) {
    emit(o, 0, true, &[0xF7], 4, src, 0);
}
pub fn imul1(o: &mut Vec<u8>, src: Op) {
    emit(o, 0, true, &[0xF7], 5, src, 0);
}
pub fn div(o: &mut Vec<u8>, src: Op) {
    emit(o, 0, true, &[0xF7], 6, src, 0);
}
pub fn idiv(o: &mut Vec<u8>, src: Op) {
    emit(o, 0, true, &[0xF7], 7, src, 0);
}
pub fn neg(o: &mut Vec<u8>, dst: Reg) {
    emit(o, 0, true, &[0xF7], 3, Op::R(dst), 0);
}
pub fn not(o: &mut Vec<u8>, dst: Reg) {
    emit(o, 0, true, &[0xF7], 2, Op::R(dst), 0);
}
pub fn cqo(o: &mut Vec<u8>) {
    o.extend_from_slice(&[0x48, 0x99]);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shift {
    Shl = 4,
    Shr = 5,
    Sar = 7,
}

/// Shift by `cl`.
pub fn shift_cl(o: &mut Vec<u8>, op: Shift, dst: Reg) {
    emit(o, 0, true, &[0xD3], op as u8, Op::R(dst), 0);
}
pub fn shift_i(o: &mut Vec<u8>, op: Shift, dst: Reg, imm: u8) {
    emit(o, 0, true, &[0xC1], op as u8, Op::R(dst), 0);
    o.push(imm);
}

pub fn setcc(o: &mut Vec<u8>, cc: Cond, dst: Reg) {
    emit(o, 0, false, &[0x0F, 0x90 + cc as u8], 0, Op::R(dst), 2);
}
pub fn cmov(o: &mut Vec<u8>, cc: Cond, dst: Reg, src: Reg) {
    emit(o, 0, true, &[0x0F, 0x40 + cc as u8], dst, Op::R(src), 0);
}

// ---- control flow ----

fn rel32(o: &mut Vec<u8>, opc: &[u8]) {
    o.extend_from_slice(opc);
    o.extend_from_slice(&[0; 4]);
}
pub fn jmp(o: &mut Vec<u8>) {
    rel32(o, &[0xE9]);
}
pub fn jcc(o: &mut Vec<u8>, cc: Cond) {
    rel32(o, &[0x0F, 0x80 + cc as u8]);
}
pub fn call(o: &mut Vec<u8>) {
    rel32(o, &[0xE8]);
}
pub fn call_r(o: &mut Vec<u8>, r: Reg) {
    emit(o, 0, false, &[0xFF], 2, Op::R(r), 0);
}
/// `jmp qword ptr [rip + disp32]`: an import stub.
pub fn jmp_rip(o: &mut Vec<u8>) {
    emit(o, 0, false, &[0xFF], 4, Op::Rip, 0);
}
pub fn ret(o: &mut Vec<u8>) {
    o.push(0xC3);
}
pub fn leave(o: &mut Vec<u8>) {
    o.push(0xC9);
}
pub fn ud2(o: &mut Vec<u8>) {
    o.extend_from_slice(&[0x0F, 0x0B]);
}
pub fn rep_movsb(o: &mut Vec<u8>) {
    o.extend_from_slice(&[0xF3, 0xA4]);
}

/// Writes `disp` into the rel32 that ends at `end` (a position in `o`).
pub fn patch_rel(o: &mut [u8], at: usize, target: usize) {
    let disp = target as i64 - (at as i64 + 4);
    o[at..at + 4].copy_from_slice(&(disp as i32).to_le_bytes());
}

// ---- SSE scalar floating point ----

pub fn movq_xr(o: &mut Vec<u8>, x: Xmm, r: Reg) {
    emit(o, 0x66, true, &[0x0F, 0x6E], x, Op::R(r), 0);
}
pub fn movq_rx(o: &mut Vec<u8>, r: Reg, x: Xmm) {
    emit(o, 0x66, true, &[0x0F, 0x7E], x, Op::R(r), 0);
}
pub fn movd_xr(o: &mut Vec<u8>, x: Xmm, r: Reg) {
    emit(o, 0x66, false, &[0x0F, 0x6E], x, Op::R(r), 0);
}
pub fn movd_rx(o: &mut Vec<u8>, r: Reg, x: Xmm) {
    emit(o, 0x66, false, &[0x0F, 0x7E], x, Op::R(r), 0);
}
/// `movsd`/`movss` between an xmm register and memory.
pub fn movs_load(o: &mut Vec<u8>, bits: u8, x: Xmm, m: Op) {
    emit(o, if bits == 64 { 0xF2 } else { 0xF3 }, false, &[0x0F, 0x10], x, m, 0);
}
pub fn movs_store(o: &mut Vec<u8>, bits: u8, m: Op, x: Xmm) {
    emit(o, if bits == 64 { 0xF2 } else { 0xF3 }, false, &[0x0F, 0x11], x, m, 0);
}
pub fn movaps(o: &mut Vec<u8>, d: Xmm, s: Xmm) {
    emit(o, 0, false, &[0x0F, 0x28], d, Op::R(s), 0);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sse {
    Add = 0x58,
    Mul = 0x59,
    Sub = 0x5C,
    Div = 0x5E,
}

pub fn sse(o: &mut Vec<u8>, op: Sse, bits: u8, d: Xmm, s: Op) {
    emit(o, if bits == 64 { 0xF2 } else { 0xF3 }, false, &[0x0F, op as u8], d, s, 0);
}
pub fn ucomis(o: &mut Vec<u8>, bits: u8, a: Xmm, b: Op) {
    emit(o, if bits == 64 { 0x66 } else { 0 }, false, &[0x0F, 0x2E], a, b, 0);
}
/// Signed 64-bit integer to float.
pub fn cvtsi2s(o: &mut Vec<u8>, bits: u8, d: Xmm, s: Reg) {
    emit(o, if bits == 64 { 0xF2 } else { 0xF3 }, true, &[0x0F, 0x2A], d, Op::R(s), 0);
}
/// Truncating float to signed 64-bit integer; out of range gives 0x8000000000000000.
pub fn cvtts2si(o: &mut Vec<u8>, bits: u8, d: Reg, s: Xmm) {
    emit(o, if bits == 64 { 0xF2 } else { 0xF3 }, true, &[0x0F, 0x2C], d, Op::R(s), 0);
}
pub fn cvtss2sd(o: &mut Vec<u8>, d: Xmm, s: Xmm) {
    emit(o, 0xF3, false, &[0x0F, 0x5A], d, Op::R(s), 0);
}
pub fn cvtsd2ss(o: &mut Vec<u8>, d: Xmm, s: Xmm) {
    emit(o, 0xF2, false, &[0x0F, 0x5A], d, Op::R(s), 0);
}


// ---- SSE packed (128-bit vectors) ----

/// `movups xmm, m128`
pub fn movups_load(o: &mut Vec<u8>, x: Xmm, m: Op) {
    emit(o, 0, false, &[0x0F, 0x10], x, m, 0);
}
/// `movups m128, xmm`
pub fn movups_store(o: &mut Vec<u8>, m: Op, x: Xmm) {
    emit(o, 0, false, &[0x0F, 0x11], x, m, 0);
}
/// `movlps xmm, m64` / `movhps xmm, m64`: the low or high half, the other half stays.
pub fn movlps_load(o: &mut Vec<u8>, x: Xmm, m: Op) {
    emit(o, 0, false, &[0x0F, 0x12], x, m, 0);
}
pub fn movhps_load(o: &mut Vec<u8>, x: Xmm, m: Op) {
    emit(o, 0, false, &[0x0F, 0x16], x, m, 0);
}
pub fn movlps_store(o: &mut Vec<u8>, m: Op, x: Xmm) {
    emit(o, 0, false, &[0x0F, 0x13], x, m, 0);
}
pub fn movhps_store(o: &mut Vec<u8>, m: Op, x: Xmm) {
    emit(o, 0, false, &[0x0F, 0x17], x, m, 0);
}
/// `addpd`, `addps` and so on: `double` picks two f64 lanes, else four f32.
pub fn psse(o: &mut Vec<u8>, op: Sse, double: bool, d: Xmm, s: Op) {
    emit(o, if double { 0x66 } else { 0 }, false, &[0x0F, op as u8], d, s, 0);
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PInt {
    /// `paddd`
    Add = 0xFE,
    /// `psubd`
    Sub = 0xFA,
    /// `pxor`
    Xor = 0xEF,
}
pub fn pint(o: &mut Vec<u8>, op: PInt, d: Xmm, s: Op) {
    emit(o, 0x66, false, &[0x0F, op as u8], d, s, 0);
}
/// `pmulld`: four 32-bit products, low halves (SSE4.1).
pub fn pmulld(o: &mut Vec<u8>, d: Xmm, s: Op) {
    emit(o, 0x66, false, &[0x0F, 0x38, 0x40], d, s, 0);
}
/// `xorps`
pub fn xorps(o: &mut Vec<u8>, d: Xmm, s: Xmm) {
    emit(o, 0, false, &[0x0F, 0x57], d, Op::R(s), 0);
}
/// `unpcklpd d, s`: both lanes become the low lane of `d`/`s`.
pub fn unpcklpd(o: &mut Vec<u8>, d: Xmm, s: Xmm) {
    emit(o, 0x66, false, &[0x0F, 0x14], d, Op::R(s), 0);
}
/// `punpcklqdq d, s`
pub fn punpcklqdq(o: &mut Vec<u8>, d: Xmm, s: Xmm) {
    emit(o, 0x66, false, &[0x0F, 0x6C], d, Op::R(s), 0);
}
/// `shufps d, s, imm`
pub fn shufps(o: &mut Vec<u8>, d: Xmm, s: Xmm, imm: u8) {
    emit(o, 0, false, &[0x0F, 0xC6], d, Op::R(s), 0);
    o.push(imm);
}
/// `pshufd d, s, imm`
pub fn pshufd(o: &mut Vec<u8>, d: Xmm, s: Xmm, imm: u8) {
    emit(o, 0x66, false, &[0x0F, 0x70], d, Op::R(s), 0);
    o.push(imm);
}
/// `vfmadd231pd` / `vfmadd231ps` on xmm0..xmm7: `d += a * b`, one rounding (FMA3).
pub fn vfmadd231(o: &mut Vec<u8>, double: bool, d: Xmm, a: Xmm, b: Xmm) {
    debug_assert!(d < 8 && a < 8 && b < 8);
    o.extend_from_slice(&[0xC4, 0xE2, ((double as u8) << 7) | ((!a & 15) << 3) | 1, 0xB8, 0xC0 | (d << 3) | b]);
}

/// Encodings checked against llvm-mc by `tools/check_encoder_x64.py`.
/// Entries whose text ends in ` L` are branches to a label 200 bytes ahead.
pub fn golden() -> Vec<(Vec<u8>, String)> {
    let mut out: Vec<(Vec<u8>, String)> = Vec::new();
    macro_rules! g {
        ($text:expr, $f:expr) => {{
            let mut o: Vec<u8> = Vec::new();
            #[allow(clippy::redundant_closure_call)]
            ($f)(&mut o);
            out.push((o, $text.to_string()));
        }};
    }
    g!("mov rax, rcx", |o: &mut Vec<u8>| mov(o, reg(RAX), RCX));
    g!("mov r9, r12", |o: &mut Vec<u8>| mov(o, reg(R9), R12));
    g!("mov rsp, rbp", |o: &mut Vec<u8>| mov(o, reg(RSP), RBP));
    g!("mov eax, ecx", |o: &mut Vec<u8>| mov32(o, reg(RAX), RCX));
    g!("mov r8d, r10d", |o: &mut Vec<u8>| mov32(o, reg(R8), R10));
    g!("mov rax, qword ptr [rbp - 8]", |o: &mut Vec<u8>| mov_load(o, RAX, at(RBP, -8)));
    g!("mov rdx, qword ptr [rbp - 4096]", |o: &mut Vec<u8>| mov_load(o, RDX, at(RBP, -4096)));
    g!("mov rax, qword ptr [rsp]", |o: &mut Vec<u8>| mov_load(o, RAX, at(RSP, 0)));
    g!("mov rcx, qword ptr [rsp + 24]", |o: &mut Vec<u8>| mov_load(o, RCX, at(RSP, 24)));
    g!("mov rax, qword ptr [r12]", |o: &mut Vec<u8>| mov_load(o, RAX, at(R12, 0)));
    g!("mov r11, qword ptr [r13]", |o: &mut Vec<u8>| mov_load(o, R11, at(R13, 0)));
    g!("mov rax, qword ptr [rbp + 16]", |o: &mut Vec<u8>| mov_load(o, RAX, at(RBP, 16)));
    g!("mov rax, qword ptr [rax + rcx*8 + 16]", |o: &mut Vec<u8>| mov_load(o, RAX, at_idx(RAX, RCX, 8, 16)));
    g!("mov r10, qword ptr [r9 + r13*4]", |o: &mut Vec<u8>| mov_load(o, R10, at_idx(R9, R13, 4, 0)));
    g!("mov r10, qword ptr [rbp + rax*1 - 1000]", |o: &mut Vec<u8>| mov_load(o, R10, at_idx(RBP, RAX, 1, -1000)));
    g!("mov qword ptr [rbp - 16], rax", |o: &mut Vec<u8>| mov(o, at(RBP, -16), RAX));
    g!("mov qword ptr [rsp + 8], r15", |o: &mut Vec<u8>| mov(o, at(RSP, 8), R15));
    g!("mov qword ptr [rdi + rcx*2], rdx", |o: &mut Vec<u8>| mov(o, at_idx(RDI, RCX, 2, 0), RDX));
    g!("mov eax, dword ptr [rcx + 4]", |o: &mut Vec<u8>| mov32_load(o, RAX, at(RCX, 4)));
    g!("mov dword ptr [rcx + 4], r9d", |o: &mut Vec<u8>| mov32(o, at(RCX, 4), R9));
    g!("mov word ptr [rcx + 2], dx", |o: &mut Vec<u8>| mov16(o, at(RCX, 2), RDX));
    g!("mov byte ptr [rax], sil", |o: &mut Vec<u8>| mov8(o, at(RAX, 0), RSI));
    g!("mov byte ptr [rax], cl", |o: &mut Vec<u8>| mov8(o, at(RAX, 0), RCX));
    g!("mov byte ptr [r9 + rdx], r10b", |o: &mut Vec<u8>| mov8(o, at_idx(R9, RDX, 1, 0), R10));
    g!("mov eax, 5", |o: &mut Vec<u8>| mov_ri32(o, RAX, 5));
    g!("mov r10d, 4000000000", |o: &mut Vec<u8>| mov_ri32(o, R10, 4_000_000_000));
    g!("mov rcx, -5", |o: &mut Vec<u8>| mov_ri_sext(o, RCX, -5));
    g!("mov r9, 70000", |o: &mut Vec<u8>| mov_ri_sext(o, R9, 70000));
    g!("movabs rax, 0x123456789a", |o: &mut Vec<u8>| mov_ri64(o, RAX, 0x12_3456_789A));
    g!("movabs r11, -2", |o: &mut Vec<u8>| mov_ri64(o, R11, -2));
    g!("mov qword ptr [rbp - 24], 7", |o: &mut Vec<u8>| mov_mi(o, at(RBP, -24), 7));
    g!("mov qword ptr [rbp - 24], -100000", |o: &mut Vec<u8>| mov_mi(o, at(RBP, -24), -100_000));
    g!("lea rax, [rbp - 64]", |o: &mut Vec<u8>| lea(o, RAX, at(RBP, -64)));
    g!("lea rdi, [rax + rcx*8]", |o: &mut Vec<u8>| lea(o, RDI, at_idx(RAX, RCX, 8, 0)));
    g!("lea r8, [r9 + r10*2 + 12]", |o: &mut Vec<u8>| lea(o, R8, at_idx(R9, R10, 2, 12)));
    g!("lea rax, [rip + 0]", |o: &mut Vec<u8>| lea(o, RAX, Op::Rip));
    g!("lea r13, [rip + 0]", |o: &mut Vec<u8>| lea(o, R13, Op::Rip));
    g!("movzx eax, byte ptr [rcx]", |o: &mut Vec<u8>| movzx8(o, RAX, at(RCX, 0)));
    g!("movzx r9d, byte ptr [rbp - 1]", |o: &mut Vec<u8>| movzx8(o, R9, at(RBP, -1)));
    g!("movzx eax, al", |o: &mut Vec<u8>| movzx8(o, RAX, reg(RAX)));
    g!("movzx eax, sil", |o: &mut Vec<u8>| movzx8(o, RAX, reg(RSI)));
    g!("movzx ecx, r10b", |o: &mut Vec<u8>| movzx8(o, RCX, reg(R10)));
    g!("movzx eax, word ptr [rcx]", |o: &mut Vec<u8>| movzx16(o, RAX, at(RCX, 0)));
    g!("movzx eax, ax", |o: &mut Vec<u8>| movzx16(o, RAX, reg(RAX)));
    g!("movsx rax, byte ptr [rcx]", |o: &mut Vec<u8>| movsx8(o, RAX, at(RCX, 0)));
    g!("movsx r12, al", |o: &mut Vec<u8>| movsx8(o, R12, reg(RAX)));
    g!("movsx rax, dil", |o: &mut Vec<u8>| movsx8(o, RAX, reg(RDI)));
    g!("movsx rax, word ptr [rcx + 2]", |o: &mut Vec<u8>| movsx16(o, RAX, at(RCX, 2)));
    g!("movsx r12, ax", |o: &mut Vec<u8>| movsx16(o, R12, reg(RAX)));
    g!("movsxd rax, dword ptr [rcx]", |o: &mut Vec<u8>| movsx32(o, RAX, at(RCX, 0)));
    g!("movsxd r12, eax", |o: &mut Vec<u8>| movsx32(o, R12, reg(RAX)));
    g!("push rbp", |o: &mut Vec<u8>| push(o, RBP));
    g!("push r12", |o: &mut Vec<u8>| push(o, R12));
    g!("pop rbx", |o: &mut Vec<u8>| pop(o, RBX));
    g!("pop r15", |o: &mut Vec<u8>| pop(o, R15));
    for (name, op) in [("add", Alu::Add), ("or", Alu::Or), ("adc", Alu::Adc), ("sbb", Alu::Sbb), ("and", Alu::And), ("sub", Alu::Sub), ("xor", Alu::Xor), ("cmp", Alu::Cmp)] {
        g!(format!("{name} rax, rcx"), |o: &mut Vec<u8>| alu(o, op, reg(RAX), RCX));
        g!(format!("{name} r9, r14"), |o: &mut Vec<u8>| alu(o, op, reg(R9), R14));
        g!(format!("{name} qword ptr [rbp - 8], rdx"), |o: &mut Vec<u8>| alu(o, op, at(RBP, -8), RDX));
        g!(format!("{name} rcx, qword ptr [rbp - 8]"), |o: &mut Vec<u8>| alu_load(o, op, RCX, at(RBP, -8)));
        g!(format!("{name} rcx, 7"), |o: &mut Vec<u8>| alu_i(o, op, reg(RCX), 7));
        g!(format!("{name} r10, -1"), |o: &mut Vec<u8>| alu_i(o, op, reg(R10), -1));
        g!(format!("{name} rcx, 1000"), |o: &mut Vec<u8>| alu_i(o, op, reg(RCX), 1000));
        g!(format!("{name} qword ptr [rax], 3"), |o: &mut Vec<u8>| alu_i(o, op, at(RAX, 0), 3));
    }
    g!("cmp rcx, -2147483648", |o: &mut Vec<u8>| alu_i(o, Alu::Cmp, reg(RCX), i32::MIN));
    g!("test rax, rax", |o: &mut Vec<u8>| test(o, RAX, RAX));
    g!("test r9, rdx", |o: &mut Vec<u8>| test(o, R9, RDX));
    g!("imul rax, rcx", |o: &mut Vec<u8>| imul(o, RAX, reg(RCX)));
    g!("imul r9, qword ptr [rbp - 8]", |o: &mut Vec<u8>| imul(o, R9, at(RBP, -8)));
    g!("mul rcx", |o: &mut Vec<u8>| mul1(o, reg(RCX)));
    g!("imul r10", |o: &mut Vec<u8>| imul1(o, reg(R10)));
    g!("div rcx", |o: &mut Vec<u8>| div(o, reg(RCX)));
    g!("idiv rcx", |o: &mut Vec<u8>| idiv(o, reg(RCX)));
    g!("idiv qword ptr [rbp - 8]", |o: &mut Vec<u8>| idiv(o, at(RBP, -8)));
    g!("neg rax", |o: &mut Vec<u8>| neg(o, RAX));
    g!("neg r11", |o: &mut Vec<u8>| neg(o, R11));
    g!("not rax", |o: &mut Vec<u8>| not(o, RAX));
    g!("cqo", cqo);
    g!("shl rax, cl", |o: &mut Vec<u8>| shift_cl(o, Shift::Shl, RAX));
    g!("shr r9, cl", |o: &mut Vec<u8>| shift_cl(o, Shift::Shr, R9));
    g!("sar rax, cl", |o: &mut Vec<u8>| shift_cl(o, Shift::Sar, RAX));
    g!("shl rax, 3", |o: &mut Vec<u8>| shift_i(o, Shift::Shl, RAX, 3));
    g!("shr rcx, 2", |o: &mut Vec<u8>| shift_i(o, Shift::Shr, RCX, 2));
    g!("sar r12, 63", |o: &mut Vec<u8>| shift_i(o, Shift::Sar, R12, 63));
    for (name, cc) in [("o", Cond::O), ("no", Cond::No), ("b", Cond::B), ("ae", Cond::Ae), ("e", Cond::E), ("ne", Cond::Ne), ("be", Cond::Be), ("a", Cond::A), ("s", Cond::S), ("ns", Cond::Ns), ("p", Cond::P), ("np", Cond::Np), ("l", Cond::L), ("ge", Cond::Ge), ("le", Cond::Le), ("g", Cond::G)] {
        g!(format!("set{name} al"), |o: &mut Vec<u8>| setcc(o, cc, RAX));
        g!(format!("cmov{name} rax, rcx"), |o: &mut Vec<u8>| cmov(o, cc, RAX, RCX));
        g!(format!("j{name} L"), |o: &mut Vec<u8>| {
            jcc(o, cc);
            patch_rel(o, 2, 6 + 200);
        });
    }
    g!("setl sil", |o: &mut Vec<u8>| setcc(o, Cond::L, RSI));
    g!("setne r9b", |o: &mut Vec<u8>| setcc(o, Cond::Ne, R9));
    g!("jmp L", |o: &mut Vec<u8>| {
        jmp(o);
        patch_rel(o, 1, 5 + 200);
    });
    g!("call L", |o: &mut Vec<u8>| {
        call(o);
        patch_rel(o, 1, 5 + 200);
    });
    g!("call rax", |o: &mut Vec<u8>| call_r(o, RAX));
    g!("call r11", |o: &mut Vec<u8>| call_r(o, R11));
    g!("jmp qword ptr [rip + 0]", jmp_rip);
    g!("ret", ret);
    g!("leave", leave);
    g!("ud2", ud2);
    g!("rep movsb", rep_movsb);
    g!("movq xmm0, rax", |o: &mut Vec<u8>| movq_xr(o, 0, RAX));
    g!("movq xmm9, r10", |o: &mut Vec<u8>| movq_xr(o, 9, R10));
    g!("movq rax, xmm1", |o: &mut Vec<u8>| movq_rx(o, RAX, 1));
    g!("movq r11, xmm12", |o: &mut Vec<u8>| movq_rx(o, R11, 12));
    g!("movd xmm1, eax", |o: &mut Vec<u8>| movd_xr(o, 1, RAX));
    g!("movd eax, xmm2", |o: &mut Vec<u8>| movd_rx(o, RAX, 2));
    g!("movd r9d, xmm0", |o: &mut Vec<u8>| movd_rx(o, R9, 0));
    g!("movsd xmm0, qword ptr [rbp - 8]", |o: &mut Vec<u8>| movs_load(o, 64, 0, at(RBP, -8)));
    g!("movsd qword ptr [rbp - 8], xmm1", |o: &mut Vec<u8>| movs_store(o, 64, at(RBP, -8), 1));
    g!("movss xmm0, dword ptr [rbp - 8]", |o: &mut Vec<u8>| movs_load(o, 32, 0, at(RBP, -8)));
    g!("movss dword ptr [rcx + 4], xmm3", |o: &mut Vec<u8>| movs_store(o, 32, at(RCX, 4), 3));
    g!("movsd xmm8, qword ptr [r9 + rax*8]", |o: &mut Vec<u8>| movs_load(o, 64, 8, at_idx(R9, RAX, 8, 0)));
    g!("movaps xmm1, xmm0", |o: &mut Vec<u8>| movaps(o, 1, 0));
    for (name, op) in [("add", Sse::Add), ("mul", Sse::Mul), ("sub", Sse::Sub), ("div", Sse::Div)] {
        g!(format!("{name}sd xmm0, xmm1"), |o: &mut Vec<u8>| sse(o, op, 64, 0, reg(1)));
        g!(format!("{name}ss xmm2, xmm9"), |o: &mut Vec<u8>| sse(o, op, 32, 2, reg(9)));
        g!(format!("{name}sd xmm0, qword ptr [rbp - 8]"), |o: &mut Vec<u8>| sse(o, op, 64, 0, at(RBP, -8)));
    }
    g!("ucomisd xmm0, xmm1", |o: &mut Vec<u8>| ucomis(o, 64, 0, reg(1)));
    g!("ucomiss xmm0, xmm1", |o: &mut Vec<u8>| ucomis(o, 32, 0, reg(1)));
    g!("ucomisd xmm10, xmm0", |o: &mut Vec<u8>| ucomis(o, 64, 10, reg(0)));
    g!("ucomisd xmm0, qword ptr [rip + 0]", |o: &mut Vec<u8>| ucomis(o, 64, 0, Op::Rip));
    g!("cvtsi2sd xmm0, rax", |o: &mut Vec<u8>| cvtsi2s(o, 64, 0, RAX));
    g!("cvtsi2ss xmm1, r9", |o: &mut Vec<u8>| cvtsi2s(o, 32, 1, R9));
    g!("cvttsd2si rax, xmm0", |o: &mut Vec<u8>| cvtts2si(o, 64, RAX, 0));
    g!("cvttss2si r11, xmm1", |o: &mut Vec<u8>| cvtts2si(o, 32, R11, 1));
    g!("cvtss2sd xmm0, xmm1", |o: &mut Vec<u8>| cvtss2sd(o, 0, 1));
    g!("cvtsd2ss xmm0, xmm1", |o: &mut Vec<u8>| cvtsd2ss(o, 0, 1));
    g!("movups xmm0, xmmword ptr [rax + 16]", |o: &mut Vec<u8>| movups_load(o, 0, at(RAX, 16)));
    g!("movups xmmword ptr [rcx], xmm2", |o: &mut Vec<u8>| movups_store(o, at(RCX, 0), 2));
    g!("movlps xmm0, qword ptr [rbp - 8]", |o: &mut Vec<u8>| movlps_load(o, 0, at(RBP, -8)));
    g!("movhps xmm1, qword ptr [rbp - 1000]", |o: &mut Vec<u8>| movhps_load(o, 1, at(RBP, -1000)));
    g!("movlps qword ptr [rbp - 8], xmm0", |o: &mut Vec<u8>| movlps_store(o, at(RBP, -8), 0));
    g!("movhps qword ptr [rbp - 24], xmm2", |o: &mut Vec<u8>| movhps_store(o, at(RBP, -24), 2));
    for (name, op) in [("add", Sse::Add), ("mul", Sse::Mul), ("sub", Sse::Sub), ("div", Sse::Div)] {
        g!(format!("{name}pd xmm0, xmm1"), |o: &mut Vec<u8>| psse(o, op, true, 0, reg(1)));
        g!(format!("{name}ps xmm0, xmm1"), |o: &mut Vec<u8>| psse(o, op, false, 0, reg(1)));
    }
    g!("paddd xmm0, xmm1", |o: &mut Vec<u8>| pint(o, PInt::Add, 0, reg(1)));
    g!("psubd xmm1, xmm0", |o: &mut Vec<u8>| pint(o, PInt::Sub, 1, reg(0)));
    g!("pxor xmm1, xmm1", |o: &mut Vec<u8>| pint(o, PInt::Xor, 1, reg(1)));
    g!("pmulld xmm0, xmm1", |o: &mut Vec<u8>| pmulld(o, 0, reg(1)));
    g!("xorps xmm0, xmm1", |o: &mut Vec<u8>| xorps(o, 0, 1));
    g!("unpcklpd xmm0, xmm0", |o: &mut Vec<u8>| unpcklpd(o, 0, 0));
    g!("punpcklqdq xmm1, xmm1", |o: &mut Vec<u8>| punpcklqdq(o, 1, 1));
    g!("shufps xmm0, xmm0, 0", |o: &mut Vec<u8>| shufps(o, 0, 0, 0));
    g!("pshufd xmm1, xmm1, 0", |o: &mut Vec<u8>| pshufd(o, 1, 1, 0));
    g!("vfmadd231pd xmm0, xmm1, xmm2", |o: &mut Vec<u8>| vfmadd231(o, true, 0, 1, 2));
    g!("vfmadd231ps xmm0, xmm1, xmm2", |o: &mut Vec<u8>| vfmadd231(o, false, 0, 1, 2));
    g!("vfmadd231pd xmm3, xmm5, xmm7", |o: &mut Vec<u8>| vfmadd231(o, true, 3, 5, 7));
    out
}
