//! Writes an ARM64 Linux executable (ELF, linked against the system libc) directly: no assembler, no linker.
//! Position independent; every libc function is reached through a GOT slot that the dynamic loader fills in.

use crate::arm64::*;
use crate::codegen::Object;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Libc {
    Gnu,
    Musl,
}

/// 64 KB: right for kernels with 4 KB, 16 KB and 64 KB pages alike.
const PAGE: u64 = 0x10000;

fn align(v: u64, a: u64) -> u64 {
    (v + a - 1) & !(a - 1)
}

struct W(Vec<u8>);

impl W {
    fn u8(&mut self, v: u8) {
        self.0.push(v);
    }
    fn u16(&mut self, v: u16) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    fn u32(&mut self, v: u32) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    fn u64(&mut self, v: u64) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    fn pad_to(&mut self, len: usize) {
        assert!(self.0.len() <= len, "layout overlap");
        self.0.resize(len, 0);
    }
}

fn page_patch(word: u32, pc: u64, target: u64) -> u32 {
    let pages = ((target & !0xFFF) as i64 - (pc & !0xFFF) as i64) >> 12;
    let p = pages as u32;
    word | ((p & 3) << 29) | (((p >> 2) & 0x7FFFF) << 5)
}

/// A C name as the code generator spells it (`_write`) becomes the name libc exports.
fn libc_name(import: &str) -> String {
    let n = import.strip_prefix('_').unwrap_or(import);
    match n {
        // macOS's way to reach `errno`; glibc and musl call it differently.
        "__error" => "__errno_location".to_string(),
        _ => n.to_string(),
    }
}

/// A string table that remembers the offset of each name.
struct Strings {
    bytes: Vec<u8>,
}

impl Strings {
    fn new() -> Self {
        Strings { bytes: vec![0] }
    }
    fn add(&mut self, s: &str) -> u32 {
        let at = self.bytes.len() as u32;
        self.bytes.extend_from_slice(s.as_bytes());
        self.bytes.push(0);
        at
    }
}

const PT_LOAD: u32 = 1;
const PT_DYNAMIC: u32 = 2;
const PT_INTERP: u32 = 3;
const PT_PHDR: u32 = 6;
const PT_GNU_STACK: u32 = 0x6474_e551;
const PF_X: u32 = 1;
const PF_W: u32 = 2;
const PF_R: u32 = 4;

fn phdr(w: &mut W, kind: u32, flags: u32, off: u64, vaddr: u64, filesz: u64, memsz: u64, al: u64) {
    w.u32(kind);
    w.u32(flags);
    w.u64(off);
    w.u64(vaddr);
    w.u64(vaddr);
    w.u64(filesz);
    w.u64(memsz);
    w.u64(al);
}

#[allow(clippy::too_many_arguments)]
fn shdr(w: &mut W, name: u32, kind: u32, flags: u64, addr: u64, off: u64, size: u64, link: u32, info: u32, al: u64, entsize: u64) {
    w.u32(name);
    w.u32(kind);
    w.u64(flags);
    w.u64(addr);
    w.u64(off);
    w.u64(size);
    w.u32(link);
    w.u32(info);
    w.u64(al);
    w.u64(entsize);
}

/// `debug`: the source file names, when `-g` asks for DWARF inside the file.
pub fn write_elf(obj: &Object, libc: Libc, debug: Option<&[String]>) -> Vec<u8> {
    // The imports of the program, then the one the start-up code needs.
    let mut imports: Vec<String> = obj.imports.iter().map(|n| libc_name(n)).collect();
    let start_import = imports.len();
    imports.push("__libc_start_main".to_string());
    let nimports = imports.len();

    let interp: &[u8] = match libc {
        Libc::Gnu => b"/lib/ld-linux-aarch64.so.1\0",
        Libc::Musl => b"/lib/ld-musl-aarch64.so.1\0",
    };
    let mut needed: Vec<&str> = match libc {
        Libc::Gnu => vec!["libc.so.6", "libm.so.6"],
        Libc::Musl => vec!["libc.musl-aarch64.so.1"],
    };
    if libc == Libc::Gnu && imports.iter().any(|n| n.starts_with("pthread_")) {
        needed.push("libpthread.so.0");
    }

    // ---- dynamic symbols and their names
    let mut dynstr = Strings::new();
    let needed_names: Vec<u32> = needed.iter().map(|n| dynstr.add(n)).collect();
    let mut dynsym = W(vec![0u8; 24]);
    for n in &imports {
        dynsym.u32(dynstr.add(n));
        dynsym.u8(0x12); // global function
        dynsym.u8(0);
        dynsym.u16(0); // undefined
        dynsym.u64(0);
        dynsym.u64(0);
    }
    let nsyms = nimports + 1;

    // SysV hash table with one bucket: nothing is exported, but some loaders insist on having one.
    let mut hash = W(Vec::new());
    hash.u32(1);
    hash.u32(nsyms as u32);
    hash.u32(0);
    for _ in 0..nsyms {
        hash.u32(0);
    }

    // ---- code and the start-up stub
    let mut code = obj.code.clone();
    let start = code.len();
    let main_at = start + 6; // the adrp of `main` below
    let got_at = start + 10; // the adrp of `__libc_start_main`
    code.extend_from_slice(&[
        movz(FP, 0, 0),
        movz(LR, 0, 0),
        mov(5, X0),           // the loader's exit hook
        ldr(X1, SP, 0),       // argc
        add_imm(X2, SP, 8),   // argv
        mov_sp(6, SP),       // end of the stack
        adrp(X0, 0),          // &main
        add_imm(X0, X0, 0),
        movz(3, 0, 0),       // no init and fini: the loader runs them
        movz(4, 0, 0),
        adrp(X16, 0),
        ldr(X16, X16, 0),
        blr(X16),
        brk(1),
    ]);
    debug_assert_eq!(start + 10, got_at);

    // ---- layout. Everything before the RW segment is mapped read-only and executable.
    let ph_count = 6u64;
    let ph_off = 64u64;
    let interp_off = ph_off + ph_count * 56;
    let hash_off = align(interp_off + interp.len() as u64, 8);
    let dynsym_off = align(hash_off + hash.0.len() as u64, 8);
    let dynstr_off = dynsym_off + dynsym.0.len() as u64;
    let rela_off = align(dynstr_off + dynstr.bytes.len() as u64, 8);
    let rela_size = nimports as u64 * 24;
    let text_off = align(rela_off + rela_size, 16);
    let text_size = code.len() as u64 * 4;
    let rodata_off = align(text_off + text_size, 16);
    let seg1_end = rodata_off + obj.rodata.len() as u64;

    let seg2_off = align(seg1_end, PAGE);
    // dynamic: needed ones, hash, strtab, symtab, strsz, syment, rela, relasz, relaent, flags, flags_1, null
    let dyn_entries = needed.len() as u64 + 11;
    let dyn_size = dyn_entries * 16;
    let got_off = seg2_off + dyn_size;
    let got_size = nimports as u64 * 8;
    let seg2_filesz = dyn_size + got_size;
    let bss_addr = align(seg2_off + seg2_filesz, 64);
    let seg2_memsz = bss_addr - seg2_off + obj.bss_size;

    // ---- patch address references
    let code_addr = text_off;
    let rodata_addr = rodata_off;
    let got_addr = got_off;
    let patch = |code: &mut Vec<u32>, at: usize, target: u64, scale: u64| {
        let pc = code_addr + at as u64 * 4;
        code[at] = page_patch(code[at], pc, target);
        code[at + 1] |= (((target & 0xFFF) / scale) as u32) << 10;
    };
    for (at, off) in &obj.data_refs {
        patch(&mut code, *at, rodata_addr + *off as u64, 1);
    }
    for (at, pos) in &obj.code_refs {
        patch(&mut code, *at, code_addr + *pos as u64 * 4, 1);
    }
    for at in &obj.bss_refs {
        patch(&mut code, *at, bss_addr, 1);
    }
    for (at, slot) in &obj.got_refs {
        patch(&mut code, *at, got_addr + *slot as u64 * 8, 8);
    }
    patch(&mut code, main_at, code_addr + obj.entry as u64 * 4, 1);
    patch(&mut code, got_at, got_addr + start_import as u64 * 8, 8);

    // ---- symbols for debuggers, in sections that the loader never maps
    let mut strtab = Strings::new();
    let mut symtab = W(vec![0u8; 24]);
    for (name, pos) in &obj.symbols {
        let n = name.strip_prefix('_').unwrap_or(name);
        symtab.u32(strtab.add(n));
        symtab.u8(0x02); // local function
        symtab.u8(0);
        symtab.u16(1); // .text is section 1
        symtab.u64(code_addr + *pos as u64 * 4);
        symtab.u64(0);
    }
    symtab.u32(strtab.add("_start"));
    symtab.u8(0x12);
    symtab.u8(0);
    symtab.u16(1);
    symtab.u64(code_addr + start as u64 * 4);
    symtab.u64(0);

    // DWARF goes into sections the loader never maps, after the string tables.
    let dwarf: Vec<(String, Vec<u8>)> = match debug {
        Some(files) => crate::dwarf::sections(obj, code_addr, text_size, files).into_iter().map(|(n, b)| (format!(".{n}"), b)).collect(),
        None => Vec::new(),
    };
    let mut shstr = Strings::new();
    let names: Vec<u32> = [".text", ".rodata", ".dynamic", ".got", ".bss", ".symtab", ".strtab", ".shstrtab"].iter().map(|n| shstr.add(n)).collect();
    let dwarf_names: Vec<u32> = dwarf.iter().map(|(n, _)| shstr.add(n)).collect();

    let symtab_off = align(got_off + got_size, 8);
    let strtab_off = symtab_off + symtab.0.len() as u64;
    let shstr_off = strtab_off + strtab.bytes.len() as u64;
    let mut dwarf_offs = Vec::new();
    let mut cursor = shstr_off + shstr.bytes.len() as u64;
    for (_, bytes) in &dwarf {
        dwarf_offs.push(cursor);
        cursor += bytes.len() as u64;
    }
    let sh_off = align(cursor, 8);

    // ---- write
    let mut w = W(Vec::new());
    w.0.extend_from_slice(&[0x7f, b'E', b'L', b'F', 2, 1, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    w.u16(3); // ET_DYN: position independent
    w.u16(183); // EM_AARCH64
    w.u32(1);
    w.u64(code_addr + start as u64 * 4); // entry
    w.u64(ph_off);
    w.u64(sh_off);
    w.u32(0);
    w.u16(64);
    w.u16(56);
    w.u16(ph_count as u16);
    w.u16(64);
    w.u16(9 + dwarf.len() as u16); // null + 8 sections + DWARF
    w.u16(8); // .shstrtab

    phdr(&mut w, PT_PHDR, PF_R, ph_off, ph_off, ph_count * 56, ph_count * 56, 8);
    phdr(&mut w, PT_INTERP, PF_R, interp_off, interp_off, interp.len() as u64, interp.len() as u64, 1);
    phdr(&mut w, PT_LOAD, PF_R | PF_X, 0, 0, seg1_end, seg1_end, PAGE);
    phdr(&mut w, PT_LOAD, PF_R | PF_W, seg2_off, seg2_off, seg2_filesz, seg2_memsz, PAGE);
    phdr(&mut w, PT_DYNAMIC, PF_R | PF_W, seg2_off, seg2_off, dyn_size, dyn_size, 8);
    phdr(&mut w, PT_GNU_STACK, PF_R | PF_W, 0, 0, 0, 0, 16);

    w.pad_to(interp_off as usize);
    w.0.extend_from_slice(interp);
    w.pad_to(hash_off as usize);
    w.0.extend_from_slice(&hash.0);
    w.pad_to(dynsym_off as usize);
    w.0.extend_from_slice(&dynsym.0);
    w.pad_to(dynstr_off as usize);
    w.0.extend_from_slice(&dynstr.bytes);
    w.pad_to(rela_off as usize);
    for i in 0..nimports as u64 {
        w.u64(got_addr + i * 8);
        w.u64(((i + 1) << 32) | 1025); // R_AARCH64_GLOB_DAT
        w.u64(0);
    }
    w.pad_to(text_off as usize);
    for c in &code {
        w.u32(*c);
    }
    w.pad_to(rodata_off as usize);
    w.0.extend_from_slice(&obj.rodata);
    w.pad_to(seg2_off as usize);

    // .dynamic
    for n in &needed_names {
        w.u64(1); // DT_NEEDED
        w.u64(*n as u64);
    }
    for (tag, val) in [
        (4u64, hash_off),                    // DT_HASH
        (5, dynstr_off),                     // DT_STRTAB
        (6, dynsym_off),                     // DT_SYMTAB
        (10, dynstr.bytes.len() as u64),     // DT_STRSZ
        (11, 24),                            // DT_SYMENT
        (7, rela_off),                       // DT_RELA
        (8, rela_size),                      // DT_RELASZ
        (9, 24),                             // DT_RELAENT
        (30, 8),                             // DT_FLAGS: BIND_NOW
        (0x6fff_fffb, 1 | 0x0800_0000),      // DT_FLAGS_1: NOW | PIE
        (0, 0),
    ] {
        w.u64(tag);
        w.u64(val);
    }
    // .got: filled by the loader
    w.pad_to((got_off + got_size) as usize);
    w.pad_to(symtab_off as usize);
    w.0.extend_from_slice(&symtab.0);
    w.0.extend_from_slice(&strtab.bytes);
    w.0.extend_from_slice(&shstr.bytes);
    for (_, bytes) in &dwarf {
        w.0.extend_from_slice(bytes);
    }
    w.pad_to(sh_off as usize);

    shdr(&mut w, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0);
    shdr(&mut w, names[0], 1, 6, code_addr, text_off, text_size, 0, 0, 16, 0); // PROGBITS, AX
    shdr(&mut w, names[1], 1, 2, rodata_addr, rodata_off, obj.rodata.len() as u64, 0, 0, 16, 0);
    shdr(&mut w, names[2], 6, 3, seg2_off, seg2_off, dyn_size, 0, 0, 8, 16); // DYNAMIC, WA
    shdr(&mut w, names[3], 1, 3, got_off, got_off, got_size, 0, 0, 8, 8);
    shdr(&mut w, names[4], 8, 3, bss_addr, got_off + got_size, obj.bss_size, 0, 0, 64, 0); // NOBITS
    shdr(&mut w, names[5], 2, 0, 0, symtab_off, symtab.0.len() as u64, 7, obj.symbols.len() as u32 + 1, 8, 24); // SYMTAB, links to .strtab
    shdr(&mut w, names[6], 3, 0, 0, strtab_off, strtab.bytes.len() as u64, 0, 0, 1, 0);
    shdr(&mut w, names[7], 3, 0, 0, shstr_off, shstr.bytes.len() as u64, 0, 0, 1, 0);
    for (((_, bytes), name), off) in dwarf.iter().zip(&dwarf_names).zip(&dwarf_offs) {
        shdr(&mut w, *name, 1, 0, 0, *off, bytes.len() as u64, 0, 0, 1, 0); // PROGBITS, not allocated
    }
    w.0
}
