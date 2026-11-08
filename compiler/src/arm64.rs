//! ARM64 instruction encoders. Every function returns one 32-bit instruction.
//! `tools/check_encoder.py` compares `golden()` against llvm-mc.

pub type Reg = u32;

pub const X0: Reg = 0;
pub const X1: Reg = 1;
pub const X2: Reg = 2;
pub const X9: Reg = 9;
pub const X10: Reg = 10;
pub const X11: Reg = 11;
pub const X12: Reg = 12;
pub const X13: Reg = 13;
pub const X16: Reg = 16;
pub const FP: Reg = 29;
pub const LR: Reg = 30;
/// Register number 31 is SP or XZR depending on the instruction.
pub const SP: Reg = 31;
pub const XZR: Reg = 31;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cond {
    Eq = 0,
    Ne = 1,
    Hs = 2,
    Vs = 6,
    Vc = 7,
    Ge = 10,
    Lt = 11,
    Gt = 12,
    Le = 13,
}

fn rrr(base: u32, rd: Reg, rn: Reg, rm: Reg) -> u32 {
    base | (rm << 16) | (rn << 5) | rd
}

pub fn movz(rd: Reg, imm: u32, hw: u32) -> u32 {
    0xD280_0000 | (hw << 21) | (imm << 5) | rd
}
pub fn movk(rd: Reg, imm: u32, hw: u32) -> u32 {
    0xF280_0000 | (hw << 21) | (imm << 5) | rd
}
pub fn movn(rd: Reg, imm: u32, hw: u32) -> u32 {
    0x9280_0000 | (hw << 21) | (imm << 5) | rd
}
pub fn mov(rd: Reg, rm: Reg) -> u32 {
    rrr(0xAA00_03E0, rd, 0, rm)
}
/// `mov` that can read or write SP.
pub fn mov_sp(rd: Reg, rn: Reg) -> u32 {
    add_imm(rd, rn, 0)
}

pub fn add_imm(rd: Reg, rn: Reg, imm: u32) -> u32 {
    0x9100_0000 | (imm << 10) | (rn << 5) | rd
}
pub fn sub_imm(rd: Reg, rn: Reg, imm: u32) -> u32 {
    0xD100_0000 | (imm << 10) | (rn << 5) | rd
}
pub fn sub_imm_lsl12(rd: Reg, rn: Reg, imm: u32) -> u32 {
    0xD140_0000 | (imm << 10) | (rn << 5) | rd
}
pub fn cmp_imm(rn: Reg, imm: u32) -> u32 {
    0xF100_0000 | (imm << 10) | (rn << 5) | XZR
}

pub fn cmn_imm(rn: Reg, imm: u32) -> u32 {
    0xB100_0000 | (imm << 10) | (rn << 5) | XZR
}

pub fn adds(rd: Reg, rn: Reg, rm: Reg) -> u32 {
    rrr(0xAB00_0000, rd, rn, rm)
}
pub fn subs(rd: Reg, rn: Reg, rm: Reg) -> u32 {
    rrr(0xEB00_0000, rd, rn, rm)
}
pub fn sub(rd: Reg, rn: Reg, rm: Reg) -> u32 {
    rrr(0xCB00_0000, rd, rn, rm)
}
pub fn cmp(rn: Reg, rm: Reg) -> u32 {
    subs(XZR, rn, rm)
}
/// `cmp rn, rm, asr #shift`
pub fn cmp_asr(rn: Reg, rm: Reg, shift: u32) -> u32 {
    0xEB80_0000 | (rm << 16) | (shift << 10) | (rn << 5) | XZR
}
pub fn neg(rd: Reg, rm: Reg) -> u32 {
    sub(rd, XZR, rm)
}
pub fn negs(rd: Reg, rm: Reg) -> u32 {
    subs(rd, XZR, rm)
}

pub fn mul(rd: Reg, rn: Reg, rm: Reg) -> u32 {
    rrr(0x9B00_7C00, rd, rn, rm)
}
pub fn smulh(rd: Reg, rn: Reg, rm: Reg) -> u32 {
    rrr(0x9B40_7C00, rd, rn, rm)
}
/// `rd = ra - rn * rm`
pub fn msub(rd: Reg, rn: Reg, rm: Reg, ra: Reg) -> u32 {
    0x9B00_8000 | (rm << 16) | (ra << 10) | (rn << 5) | rd
}
pub fn sdiv(rd: Reg, rn: Reg, rm: Reg) -> u32 {
    rrr(0x9AC0_0C00, rd, rn, rm)
}
pub fn udiv(rd: Reg, rn: Reg, rm: Reg) -> u32 {
    rrr(0x9AC0_0800, rd, rn, rm)
}

pub fn and(rd: Reg, rn: Reg, rm: Reg) -> u32 {
    rrr(0x8A00_0000, rd, rn, rm)
}
pub fn orr(rd: Reg, rn: Reg, rm: Reg) -> u32 {
    rrr(0xAA00_0000, rd, rn, rm)
}
pub fn eor(rd: Reg, rn: Reg, rm: Reg) -> u32 {
    rrr(0xCA00_0000, rd, rn, rm)
}
pub fn mvn(rd: Reg, rm: Reg) -> u32 {
    rrr(0xAA20_03E0, rd, 0, rm)
}
/// `eor rd, rn, #1`
pub fn eor_1(rd: Reg, rn: Reg) -> u32 {
    0xD240_0000 | (rn << 5) | rd
}
pub fn lsl(rd: Reg, rn: Reg, rm: Reg) -> u32 {
    rrr(0x9AC0_2000, rd, rn, rm)
}
pub fn asr(rd: Reg, rn: Reg, rm: Reg) -> u32 {
    rrr(0x9AC0_2800, rd, rn, rm)
}

/// `cset rd, cond`
pub fn cset(rd: Reg, cond: u32) -> u32 {
    0x9A9F_07E0 | ((cond ^ 1) << 12) | rd
}

pub fn ldr(rt: Reg, rn: Reg, off: u32) -> u32 {
    0xF940_0000 | ((off / 8) << 10) | (rn << 5) | rt
}
pub fn str(rt: Reg, rn: Reg, off: u32) -> u32 {
    0xF900_0000 | ((off / 8) << 10) | (rn << 5) | rt
}
pub fn ldr_reg(rt: Reg, rn: Reg, rm: Reg) -> u32 {
    rrr(0xF860_6800, rt, rn, rm)
}
pub fn str_reg(rt: Reg, rn: Reg, rm: Reg) -> u32 {
    rrr(0xF820_6800, rt, rn, rm)
}
pub fn strb(rt: Reg, rn: Reg, off: u32) -> u32 {
    0x3900_0000 | (off << 10) | (rn << 5) | rt
}
/// `stp rt, rt2, [rn, #off]!`
pub fn stp_pre(rt: Reg, rt2: Reg, rn: Reg, off: i32) -> u32 {
    0xA980_0000 | ((((off / 8) as u32) & 0x7F) << 15) | (rt2 << 10) | (rn << 5) | rt
}
/// `ldp rt, rt2, [rn], #off`
pub fn ldp_post(rt: Reg, rt2: Reg, rn: Reg, off: i32) -> u32 {
    0xA8C0_0000 | (((off / 8) as u32 & 0x7F) << 15) | (rt2 << 10) | (rn << 5) | rt
}

// Branch encoders take the displacement in instructions, not bytes.
pub const B: u32 = 0x1400_0000;
pub const BL: u32 = 0x9400_0000;
pub fn b_at(base: u32, disp: i32) -> u32 {
    base | (disp as u32 & 0x03FF_FFFF)
}
pub fn b_cond(cond: Cond, disp: i32) -> u32 {
    0x5400_0000 | ((disp as u32 & 0x7FFFF) << 5) | cond as u32
}
pub fn cbz(rt: Reg, disp: i32) -> u32 {
    0xB400_0000 | ((disp as u32 & 0x7FFFF) << 5) | rt
}
pub fn cbnz(rt: Reg, disp: i32) -> u32 {
    0xB500_0000 | ((disp as u32 & 0x7FFFF) << 5) | rt
}
pub fn br(rn: Reg) -> u32 {
    0xD61F_0000 | (rn << 5)
}
pub fn ret() -> u32 {
    0xD65F_03C0
}
pub fn brk(imm: u32) -> u32 {
    0xD420_0000 | (imm << 5)
}

/// `adrp rd, target` with the page distance in pages.
pub fn adrp(rd: Reg, pages: i64) -> u32 {
    let p = pages as u32;
    0x9000_0000 | ((p & 3) << 29) | (((p >> 2) & 0x7FFFF) << 5) | rd
}

/// Sample encodings with their assembly, checked against llvm-mc.
pub fn golden() -> Vec<(u32, String)> {
    let s = |w: u32, t: &str| (w, t.to_string());
    vec![
        s(movz(X0, 42, 0), "movz x0, #42"),
        s(movz(X9, 0xBEEF, 1), "movz x9, #48879, lsl #16"),
        s(movk(X9, 0x1234, 3), "movk x9, #4660, lsl #48"),
        s(movn(X10, 7, 0), "movn x10, #7"),
        s(mov(X9, X11), "mov x9, x11"),
        s(mov_sp(FP, SP), "mov x29, sp"),
        s(add_imm(X9, SP, 47), "add x9, sp, #47"),
        s(sub_imm(SP, SP, 32), "sub sp, sp, #32"),
        s(sub_imm_lsl12(SP, SP, 3), "sub sp, sp, #3, lsl #12"),
        s(cmp_imm(X0, 0), "cmp x0, #0"),
        s(cmp_imm(X10, 64), "cmp x10, #64"),
        s(cmn_imm(X10, 1), "cmn x10, #1"),
        s(adds(X11, X9, X10), "adds x11, x9, x10"),
        s(subs(X11, X9, X10), "subs x11, x9, x10"),
        s(sub(X2, X2, X9), "sub x2, x2, x9"),
        s(cmp(X9, X10), "cmp x9, x10"),
        s(cmp_asr(X12, X11, 63), "cmp x12, x11, asr #63"),
        s(neg(X11, X0), "neg x11, x0"),
        s(negs(X11, X9), "negs x11, x9"),
        s(mul(X11, X9, X10), "mul x11, x9, x10"),
        s(smulh(X12, X9, X10), "smulh x12, x9, x10"),
        s(msub(X13, X12, X10, X11), "msub x13, x12, x10, x11"),
        s(sdiv(X11, X9, X10), "sdiv x11, x9, x10"),
        s(udiv(X12, X11, X10), "udiv x12, x11, x10"),
        s(and(X11, X9, X10), "and x11, x9, x10"),
        s(orr(X11, X9, X10), "orr x11, x9, x10"),
        s(eor(X11, X9, X10), "eor x11, x9, x10"),
        s(mvn(X11, X9), "mvn x11, x9"),
        s(eor_1(X11, X9), "eor x11, x9, #1"),
        s(lsl(X11, X9, X10), "lsl x11, x9, x10"),
        s(asr(X11, X9, X10), "asr x11, x9, x10"),
        s(cset(X11, Cond::Eq as u32), "cset x11, eq"),
        s(cset(X11, Cond::Ne as u32), "cset x11, ne"),
        s(cset(X11, Cond::Lt as u32), "cset x11, lt"),
        s(cset(X11, Cond::Le as u32), "cset x11, le"),
        s(cset(X11, Cond::Gt as u32), "cset x11, gt"),
        s(cset(X11, Cond::Ge as u32), "cset x11, ge"),
        s(ldr(X9, SP, 8), "ldr x9, [sp, #8]"),
        s(ldr(X16, X16, 4088), "ldr x16, [x16, #4088]"),
        s(str(X11, SP, 32760), "str x11, [sp, #32760]"),
        s(ldr_reg(X9, SP, X16), "ldr x9, [sp, x16]"),
        s(str_reg(X11, SP, X16), "str x11, [sp, x16]"),
        s(strb(X13, X9, 0), "strb w13, [x9]"),
        s(stp_pre(FP, LR, SP, -16), "stp x29, x30, [sp, #-16]!"),
        s(stp_pre(FP, LR, SP, -48), "stp x29, x30, [sp, #-48]!"),
        s(ldp_post(FP, LR, SP, 16), "ldp x29, x30, [sp], #16"),
        s(ldp_post(FP, LR, SP, 48), "ldp x29, x30, [sp], #48"),
        s(b_at(B, 8), "b #32"),
        s(b_at(B, -8), "b #-32"),
        s(b_at(BL, 16), "bl #64"),
        s(b_cond(Cond::Vs, 4), "b.vs #16"),
        s(b_cond(Cond::Ne, -4), "b.ne #-16"),
        s(b_cond(Cond::Hs, 4), "b.hs #16"),
        s(b_cond(Cond::Vc, 4), "b.vc #16"),
        s(cbz(X9, 3), "cbz x9, #12"),
        s(cbnz(X9, -3), "cbnz x9, #-12"),
        s(br(X16), "br x16"),
        s(ret(), "ret"),
        s(brk(1), "brk #1"),
    ]
}
