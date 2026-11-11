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
pub const X14: Reg = 14;
pub const X16: Reg = 16;
pub const X19: Reg = 19;
pub const X20: Reg = 20;
pub const X21: Reg = 21;
pub const D0: Reg = 0;
pub const D1: Reg = 1;
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
    Lo = 3,
    Mi = 4,
    Vs = 6,
    Vc = 7,
    Hi = 8,
    Ls = 9,
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
pub fn adds_imm(rd: Reg, rn: Reg, imm: u32) -> u32 {
    0xB100_0000 | (imm << 10) | (rn << 5) | rd
}
pub fn subs_imm(rd: Reg, rn: Reg, imm: u32) -> u32 {
    0xF100_0000 | (imm << 10) | (rn << 5) | rd
}
/// `add rd, rn, rm, lsl #shift`
pub fn add_lsl(rd: Reg, rn: Reg, rm: Reg, shift: u32) -> u32 {
    0x8B00_0000 | (rm << 16) | (shift << 10) | (rn << 5) | rd
}
/// `ldr rt, [rn, rm, lsl #3]`
pub fn ldr_idx(rt: Reg, rn: Reg, rm: Reg) -> u32 {
    0xF860_7800 | (rm << 16) | (rn << 5) | rt
}
/// `str rt, [rn, rm, lsl #3]`
pub fn str_idx(rt: Reg, rn: Reg, rm: Reg) -> u32 {
    0xF820_7800 | (rm << 16) | (rn << 5) | rt
}
/// `ldrb wt, [rn, rm]`
pub fn ldrb_idx(rt: Reg, rn: Reg, rm: Reg) -> u32 {
    0x3860_6800 | (rm << 16) | (rn << 5) | rt
}
/// `strb wt, [rn, rm]`
pub fn strb_idx(rt: Reg, rn: Reg, rm: Reg) -> u32 {
    0x3820_6800 | (rm << 16) | (rn << 5) | rt
}
/// `ldr dt, [rn, rm, lsl #3]`
pub fn ldr_d_idx(rt: Reg, rn: Reg, rm: Reg) -> u32 {
    0xFC60_7800 | (rm << 16) | (rn << 5) | rt
}
/// `str dt, [rn, rm, lsl #3]`
pub fn str_d_idx(rt: Reg, rn: Reg, rm: Reg) -> u32 {
    0xFC20_7800 | (rm << 16) | (rn << 5) | rt
}
pub fn adcs(rd: Reg, rn: Reg, rm: Reg) -> u32 {
    rrr(0xBA00_0000, rd, rn, rm)
}
pub fn sbcs(rd: Reg, rn: Reg, rm: Reg) -> u32 {
    rrr(0xFA00_0000, rd, rn, rm)
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
pub fn umulh(rd: Reg, rn: Reg, rm: Reg) -> u32 {
    rrr(0x9BC0_7C00, rd, rn, rm)
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
/// The 13-bit pattern `N:immr:imms` that writes `v` as a logical immediate, if it can.
pub fn logical_imm(v: u64) -> Option<u32> {
    if v == 0 || v == u64::MAX {
        return None;
    }
    // The smallest repeating element: 2, 4, 8, 16, 32 or 64 bits.
    let mut size = 64u32;
    while size > 2 {
        let half = size / 2;
        let mask = (1u64 << half) - 1;
        if v & mask != (v >> half) & mask {
            break;
        }
        size = half;
    }
    let mask = if size == 64 { u64::MAX } else { (1u64 << size) - 1 };
    let e = v & mask;
    // The element must be one run of ones, possibly rotated.
    let ones = e.count_ones();
    let rot = (0..size).find(|r| {
        let x = if *r == 0 { e } else { ((e >> r) | (e << (size - r))) & mask };
        x == (1u64 << ones) - 1 || (ones == size)
    })?;
    if ones == size {
        return None;
    }
    let immr = (size - rot) & (size - 1);
    let n = (size == 64) as u32;
    let imms = ((!(size - 1) << 1) | (ones - 1)) & 0x3F;
    Some((n << 12) | (immr << 6) | imms)
}
pub fn and_imm(rd: Reg, rn: Reg, pattern: u32) -> u32 {
    0x9200_0000 | (pattern << 10) | (rn << 5) | rd
}
pub fn orr_imm(rd: Reg, rn: Reg, pattern: u32) -> u32 {
    0xB200_0000 | (pattern << 10) | (rn << 5) | rd
}
pub fn eor_imm(rd: Reg, rn: Reg, pattern: u32) -> u32 {
    0xD200_0000 | (pattern << 10) | (rn << 5) | rd
}
/// `lsl rd, rn, #k`
pub fn lsl_imm(rd: Reg, rn: Reg, k: u32) -> u32 {
    0xD340_0000 | (((64 - k) & 63) << 16) | ((63 - k) << 10) | (rn << 5) | rd
}
/// `lsr rd, rn, #k`
pub fn lsr_imm(rd: Reg, rn: Reg, k: u32) -> u32 {
    0xD340_FC00 | (k << 16) | (rn << 5) | rd
}
/// `asr rd, rn, #k`
pub fn asr_imm(rd: Reg, rn: Reg, k: u32) -> u32 {
    0x9340_FC00 | (k << 16) | (rn << 5) | rd
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

pub fn add(rd: Reg, rn: Reg, rm: Reg) -> u32 {
    rrr(0x8B00_0000, rd, rn, rm)
}
/// `add rd, rn, rm` where rn may be SP.
pub fn add_ext(rd: Reg, rn: Reg, rm: Reg) -> u32 {
    rrr(0x8B20_6000, rd, rn, rm)
}
pub fn lsr(rd: Reg, rn: Reg, rm: Reg) -> u32 {
    rrr(0x9AC0_2400, rd, rn, rm)
}
/// `sbfx rd, rn, #0, #bits`: sign-extend the low bits.
pub fn sbfx(rd: Reg, rn: Reg, bits: u32) -> u32 {
    0x9340_0000 | ((bits - 1) << 10) | (rn << 5) | rd
}
/// `ubfx rd, rn, #0, #bits`: zero-extend the low bits.
pub fn ubfx(rd: Reg, rn: Reg, bits: u32) -> u32 {
    0xD340_0000 | ((bits - 1) << 10) | (rn << 5) | rd
}

fn ls(base: u32, rt: Reg, rn: Reg, scaled: u32) -> u32 {
    base | (scaled << 10) | (rn << 5) | rt
}
pub fn ldrb(rt: Reg, rn: Reg, off: u32) -> u32 {
    ls(0x3940_0000, rt, rn, off)
}
pub fn ldrsb(rt: Reg, rn: Reg) -> u32 {
    ls(0x3980_0000, rt, rn, 0)
}
pub fn ldrh(rt: Reg, rn: Reg) -> u32 {
    ls(0x7940_0000, rt, rn, 0)
}
pub fn ldrsh(rt: Reg, rn: Reg) -> u32 {
    ls(0x7980_0000, rt, rn, 0)
}
pub fn ldr_w(rt: Reg, rn: Reg) -> u32 {
    ls(0xB940_0000, rt, rn, 0)
}
pub fn ldrsw(rt: Reg, rn: Reg) -> u32 {
    ls(0xB980_0000, rt, rn, 0)
}
pub fn strh(rt: Reg, rn: Reg) -> u32 {
    ls(0x7900_0000, rt, rn, 0)
}
pub fn str_w(rt: Reg, rn: Reg) -> u32 {
    ls(0xB900_0000, rt, rn, 0)
}
pub fn strb_reg(rt: Reg, rn: Reg, rm: Reg) -> u32 {
    rrr(0x3820_6800, rt, rn, rm)
}
/// `stp rt, rt2, [rn, #off]`
pub fn stp(rt: Reg, rt2: Reg, rn: Reg, off: u32) -> u32 {
    0xA900_0000 | ((off / 8) << 15) | (rt2 << 10) | (rn << 5) | rt
}
/// `ldp rt, rt2, [rn, #off]`
pub fn ldp(rt: Reg, rt2: Reg, rn: Reg, off: u32) -> u32 {
    0xA940_0000 | ((off / 8) << 15) | (rt2 << 10) | (rn << 5) | rt
}
pub fn stp_d(rt: Reg, rt2: Reg, rn: Reg, off: u32) -> u32 {
    0x6D00_0000 | ((off / 8) << 15) | (rt2 << 10) | (rn << 5) | rt
}
pub fn ldp_d(rt: Reg, rt2: Reg, rn: Reg, off: u32) -> u32 {
    0x6D40_0000 | ((off / 8) << 15) | (rt2 << 10) | (rn << 5) | rt
}

pub fn ldr_d(rt: Reg, rn: Reg, off: u32) -> u32 {
    ls(0xFD40_0000, rt, rn, off / 8)
}
pub fn str_d(rt: Reg, rn: Reg, off: u32) -> u32 {
    ls(0xFD00_0000, rt, rn, off / 8)
}
pub fn ldr_s(rt: Reg, rn: Reg, off: u32) -> u32 {
    ls(0xBD40_0000, rt, rn, off / 4)
}
pub fn str_s(rt: Reg, rn: Reg, off: u32) -> u32 {
    ls(0xBD00_0000, rt, rn, off / 4)
}

/// The float size bit: `ftype` 01 for double, 00 for single.
fn ft(bits: u8) -> u32 {
    if bits == 64 {
        0x0040_0000
    } else {
        0
    }
}
pub fn fadd(rd: Reg, rn: Reg, rm: Reg, bits: u8) -> u32 {
    rrr(0x1E20_2800 | ft(bits), rd, rn, rm)
}
pub fn fsub(rd: Reg, rn: Reg, rm: Reg, bits: u8) -> u32 {
    rrr(0x1E20_3800 | ft(bits), rd, rn, rm)
}
pub fn fmul(rd: Reg, rn: Reg, rm: Reg, bits: u8) -> u32 {
    rrr(0x1E20_0800 | ft(bits), rd, rn, rm)
}
pub fn fdiv(rd: Reg, rn: Reg, rm: Reg, bits: u8) -> u32 {
    rrr(0x1E20_1800 | ft(bits), rd, rn, rm)
}
pub fn fneg(rd: Reg, rn: Reg, bits: u8) -> u32 {
    0x1E21_4000 | ft(bits) | (rn << 5) | rd
}
pub fn fcmp(rn: Reg, rm: Reg, bits: u8) -> u32 {
    0x1E20_2000 | ft(bits) | (rm << 16) | (rn << 5)
}
pub fn fcvt_s_to_d(rd: Reg, rn: Reg) -> u32 {
    0x1E22_C000 | (rn << 5) | rd
}
pub fn fcvt_d_to_s(rd: Reg, rn: Reg) -> u32 {
    0x1E62_4000 | (rn << 5) | rd
}
pub fn scvtf(rd: Reg, rn: Reg, bits: u8) -> u32 {
    0x9E22_0000 | ft(bits) | (rn << 5) | rd
}
pub fn ucvtf(rd: Reg, rn: Reg, bits: u8) -> u32 {
    0x9E23_0000 | ft(bits) | (rn << 5) | rd
}
pub fn fcvtzs(rd: Reg, rn: Reg) -> u32 {
    0x9E78_0000 | (rn << 5) | rd
}
pub fn fcvtzu(rd: Reg, rn: Reg) -> u32 {
    0x9E79_0000 | (rn << 5) | rd
}
/// `fmov xd, dn`
pub fn fmov_x_d(rd: Reg, rn: Reg) -> u32 {
    0x9E66_0000 | (rn << 5) | rd
}
/// `fmov dd, dn`
pub fn fmov_d_d(rd: Reg, rn: Reg) -> u32 {
    0x1E60_4000 | (rn << 5) | rd
}
pub fn fmov_d_x(rd: Reg, rn: Reg) -> u32 {
    0x9E67_0000 | (rn << 5) | rd
}

// Branch encoders take the displacement in instructions, not bytes.
pub const B: u32 = 0x1400_0000;
pub const BL: u32 = 0x9400_0000;

/// Call the address in `rn`.
pub fn blr(rn: Reg) -> u32 {
    0xD63F_0000 | ((rn as u32) << 5)
}
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
        s(fmov_x_d(X9, 3), "fmov x9, d3"),
        s(fmov_d_d(3, 17), "fmov d3, d17"),
        s(adds_imm(X11, X9, 17), "adds x11, x9, #17"),
        s(subs_imm(X11, X9, 4095), "subs x11, x9, #4095"),
        s(add_lsl(X11, X9, X10, 3), "add x11, x9, x10, lsl #3"),
        s(ldr_idx(X11, X9, X10), "ldr x11, [x9, x10, lsl #3]"),
        s(str_idx(X11, X9, X10), "str x11, [x9, x10, lsl #3]"),
        s(ldrb_idx(X11, X9, X10), "ldrb w11, [x9, x10]"),
        s(strb_idx(X11, X9, X10), "strb w11, [x9, x10]"),
        s(ldr_d_idx(3, X9, X10), "ldr d3, [x9, x10, lsl #3]"),
        s(str_d_idx(3, X9, X10), "str d3, [x9, x10, lsl #3]"),
        s(adds(X11, X9, X10), "adds x11, x9, x10"),
        s(adcs(X14, X10, X12), "adcs x14, x10, x12"),
        s(sbcs(X14, X10, X12), "sbcs x14, x10, x12"),
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
        s(blr(X16), "blr x16"),
        s(ret(), "ret"),
        s(brk(1), "brk #1"),
        s(umulh(X12, X9, X10), "umulh x12, x9, x10"),
        s(add(X9, X9, X16), "add x9, x9, x16"),
        s(add_ext(X9, SP, X16), "add x9, sp, x16"),
        s(lsr(X11, X9, X10), "lsr x11, x9, x10"),
        s(sbfx(X12, X11, 8), "sbfx x12, x11, #0, #8"),
        s(sbfx(X12, X11, 32), "sbfx x12, x11, #0, #32"),
        s(ubfx(X11, X11, 16), "ubfx x11, x11, #0, #16"),
        s(ldrb(X10, X9, 0), "ldrb w10, [x9]"),
        s(ldrb(X10, X9, 63), "ldrb w10, [x9, #63]"),
        s(ldrsb(X10, X9), "ldrsb x10, [x9]"),
        s(ldrh(X10, X9), "ldrh w10, [x9]"),
        s(ldrsh(X10, X9), "ldrsh x10, [x9]"),
        s(ldr_w(X10, X9), "ldr w10, [x9]"),
        s(ldrsw(X10, X9), "ldrsw x10, [x9]"),
        s(strh(X10, X9), "strh w10, [x9]"),
        s(str_w(X10, X9), "str w10, [x9]"),
        s(strb_reg(X13, X12, X10), "strb w13, [x12, x10]"),
        s(stp(X19, X20, SP, 16), "stp x19, x20, [sp, #16]"),
        s(ldp(X19, X20, SP, 16), "ldp x19, x20, [sp, #16]"),
        s(stp_d(8, 9, SP, 24), "stp d8, d9, [sp, #24]"),
        s(ldp_d(10, 11, SP, 504), "ldp d10, d11, [sp, #504]"),
        s(ldr_d(D0, SP, 8), "ldr d0, [sp, #8]"),
        s(str_d(D0, SP, 32760), "str d0, [sp, #32760]"),
        s(ldr_s(D1, SP, 4), "ldr s1, [sp, #4]"),
        s(str_s(D0, X16, 0), "str s0, [x16]"),
        s(fadd(D0, D0, D1, 64), "fadd d0, d0, d1"),
        s(fsub(D0, D0, D1, 64), "fsub d0, d0, d1"),
        s(fmul(D0, D0, D1, 64), "fmul d0, d0, d1"),
        s(fdiv(D0, D0, D1, 64), "fdiv d0, d0, d1"),
        s(fadd(D0, D0, D1, 32), "fadd s0, s0, s1"),
        s(fsub(D0, D0, D1, 32), "fsub s0, s0, s1"),
        s(fmul(D0, D0, D1, 32), "fmul s0, s0, s1"),
        s(fdiv(D0, D0, D1, 32), "fdiv s0, s0, s1"),
        s(fneg(D0, D0, 64), "fneg d0, d0"),
        s(fneg(D0, D0, 32), "fneg s0, s0"),
        s(fcmp(D0, D1, 64), "fcmp d0, d1"),
        s(fcmp(D0, D1, 32), "fcmp s0, s1"),
        s(fcvt_s_to_d(D0, D0), "fcvt d0, s0"),
        s(fcvt_d_to_s(D0, D0), "fcvt s0, d0"),
        s(scvtf(D0, X9, 64), "scvtf d0, x9"),
        s(scvtf(D0, X9, 32), "scvtf s0, x9"),
        s(ucvtf(D0, X9, 64), "ucvtf d0, x9"),
        s(ucvtf(D0, X9, 32), "ucvtf s0, x9"),
        s(fcvtzs(X11, D0), "fcvtzs x11, d0"),
        s(fcvtzu(X11, D0), "fcvtzu x11, d0"),
        s(fmov_d_x(D1, X16), "fmov d1, x16"),
        s(b_cond(Cond::Lo, 4), "b.lo #16"),
        s(b_cond(Cond::Mi, 4), "b.mi #16"),
        s(b_cond(Cond::Hi, 4), "b.hi #16"),
        s(b_cond(Cond::Ls, 4), "b.ls #16"),
        s(cset(X11, Cond::Lo as u32), "cset x11, lo"),
        s(cset(X11, Cond::Ls as u32), "cset x11, ls"),
        s(cset(X11, Cond::Hi as u32), "cset x11, hi"),
        s(cset(X11, Cond::Hs as u32), "cset x11, hs"),
        s(cset(X11, Cond::Mi as u32), "cset x11, mi"),
        s(lsl_imm(X11, X9, 1), "lsl x11, x9, #1"),
        s(lsl_imm(X11, X9, 63), "lsl x11, x9, #63"),
        s(lsr_imm(X11, X9, 3), "lsr x11, x9, #3"),
        s(asr_imm(X11, X9, 1), "asr x11, x9, #1"),
        s(asr_imm(X11, X9, 63), "asr x11, x9, #63"),
    ]
    .into_iter()
    .chain(
        [1u64, 3, 0xFF, 0xFFFF_FFFF, 0x8000_0000_0000_0000, 0x7FFF_FFFF_FFFF_FFFF, 0xF0F0_F0F0_F0F0_F0F0, 0x5555_5555_5555_5555, 0x00FF_00FF_00FF_00FF, 6, 0xFFFF_FFFF_FFFF_FFFE, 0x0000_FFFF_0000_0000, 12, 0x0101_0101_0101_0101]
            .into_iter()
            .flat_map(|v| {
                let p = logical_imm(v).expect("encodable");
                vec![
                    (and_imm(X11, X9, p), format!("and x11, x9, #{v}")),
                    (orr_imm(X11, X9, p), format!("orr x11, x9, #{v}")),
                    (eor_imm(X11, X9, p), format!("eor x11, x9, #{v}")),
                ]
            }),
    )
    .collect()
}
