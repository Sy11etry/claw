//! Writes a signed arm64 Mach-O executable directly: no assembler, no linker.
//! The layout mirrors what clang + ld produce on macOS (checked with otool).

use crate::codegen::Object;
use crate::codegen_x64::ObjectX64;
use crate::sha256::sha256;

const BASE: u64 = 0x1_0000_0000;
const PAGE: u64 = 0x4000;
const SIG_PAGE: usize = 4096;

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
    fn name16(&mut self, s: &str) {
        let mut b = [0u8; 16];
        b[..s.len()].copy_from_slice(s.as_bytes());
        self.0.extend_from_slice(&b);
    }
    fn pad_to(&mut self, len: usize) {
        self.0.resize(len, 0);
    }
}

struct Seg<'a> {
    name: &'a str,
    vmaddr: u64,
    vmsize: u64,
    fileoff: u64,
    filesize: u64,
    maxprot: u32,
    initprot: u32,
    flags: u32,
    nsects: u32,
}

fn segment(w: &mut W, s: &Seg) {
    w.u32(0x19);
    w.u32(72 + 80 * s.nsects);
    w.name16(s.name);
    w.u64(s.vmaddr);
    w.u64(s.vmsize);
    w.u64(s.fileoff);
    w.u64(s.filesize);
    w.u32(s.maxprot);
    w.u32(s.initprot);
    w.u32(s.nsects);
    w.u32(s.flags);
}

fn section(w: &mut W, sect: &str, seg: &str, addr: u64, size: u64, off: u32, al: u32, flags: u32) {
    w.name16(sect);
    w.name16(seg);
    w.u64(addr);
    w.u64(size);
    w.u32(off);
    w.u32(al);
    w.u32(0);
    w.u32(0);
    w.u32(flags);
    w.u32(0);
    w.u32(0);
    w.u32(0);
}

fn path_cmd(w: &mut W, cmd: u32, path: &str, extra: &[u32]) {
    let header = 8 + 4 + 4 * extra.len();
    let size = align((header + path.len() + 1) as u64, 8) as u32;
    let start = w.0.len();
    w.u32(cmd);
    w.u32(size);
    w.u32(header as u32);
    for e in extra {
        w.u32(*e);
    }
    w.0.extend_from_slice(path.as_bytes());
    w.pad_to(start + size as usize);
}

fn linkedit_cmd(w: &mut W, cmd: u32, off: u64, size: u64) {
    w.u32(cmd);
    w.u32(16);
    w.u32(off as u32);
    w.u32(size as u32);
}

/// Chained fixups: every GOT slot binds to a libSystem symbol.
fn chained_fixups(imports: &[String], got_seg_offset: u64, page: u64) -> Vec<u8> {
    const SEG_COUNT: u32 = 5;
    const GOT_SEG_INDEX: usize = 2;
    let starts_offset = 32u32;
    let image_size = 4 + 4 * SEG_COUNT;
    let seg_info_offset = align(image_size as u64, 8) as u32;
    let seg_size = 24u32;
    let imports_offset = starts_offset + seg_info_offset + seg_size;
    let symbols_offset = imports_offset + 4 * imports.len() as u32;

    let mut w = W(Vec::new());
    w.u32(0);
    w.u32(starts_offset);
    w.u32(imports_offset);
    w.u32(symbols_offset);
    w.u32(imports.len() as u32);
    w.u32(1); // DYLD_CHAINED_IMPORT
    w.u32(0); // uncompressed symbol names
    w.pad_to(starts_offset as usize);

    w.u32(SEG_COUNT);
    for i in 0..SEG_COUNT as usize {
        w.u32(if i == GOT_SEG_INDEX { seg_info_offset } else { 0 });
    }
    w.pad_to((starts_offset + seg_info_offset) as usize);

    w.u32(seg_size);
    w.u16(page as u16);
    w.u16(6); // DYLD_CHAINED_PTR_64_OFFSET
    w.u64(got_seg_offset);
    w.u32(0);
    w.u16(1);
    w.u16(0);

    let mut names = vec![0u8];
    for name in imports {
        w.u32(1 | ((names.len() as u32) << 9)); // library ordinal 1 = libSystem
        names.extend_from_slice(name.as_bytes());
        names.push(0);
    }
    w.0.extend_from_slice(&names);
    let n = align(w.0.len() as u64, 8) as usize;
    w.pad_to(n);
    w.0
}

/// A trie holding one export, `__mh_execute_header` at offset 0.
fn exports_trie() -> Vec<u8> {
    let name = b"__mh_execute_header\0";
    let mut t = vec![0u8, 1];
    t.extend_from_slice(name);
    let node = (t.len() + 1) as u8;
    t.push(node);
    t.extend_from_slice(&[2, 0, 0, 0]); // terminal size 2, flags 0, address 0, no children
    let n = align(t.len() as u64, 8) as usize;
    t.resize(n, 0);
    t
}

fn page_patch(word: u32, pc: u64, target: u64) -> u32 {
    let pages = ((target & !0xFFF) as i64 - (pc & !0xFFF) as i64) >> 12;
    let p = pages as u32;
    word | ((p & 3) << 29) | (((p >> 2) & 0x7FFFF) << 5)
}

fn be32(b: &mut Vec<u8>, v: u32) {
    b.extend_from_slice(&v.to_be_bytes());
}

fn be64(b: &mut Vec<u8>, v: u64) {
    b.extend_from_slice(&v.to_be_bytes());
}

fn signature_size(code_limit: usize, ident: &str) -> usize {
    let slots = code_limit.div_ceil(SIG_PAGE);
    20 + 88 + ident.len() + 1 + slots * 32
}

/// Ad-hoc signature in the same shape ld writes: SuperBlob + CodeDirectory v20400.
fn sign(file: &[u8], ident: &str, text_filesize: u64) -> Vec<u8> {
    let slots = file.len().div_ceil(SIG_PAGE);
    let ident_off = 88u32;
    let hash_off = ident_off + ident.len() as u32 + 1;
    let cd_len = hash_off + slots as u32 * 32;

    let mut b = Vec::with_capacity(20 + cd_len as usize);
    be32(&mut b, 0xfade_0cc0); // SuperBlob
    be32(&mut b, 20 + cd_len);
    be32(&mut b, 1);
    be32(&mut b, 0); // slot: CodeDirectory
    be32(&mut b, 20);

    be32(&mut b, 0xfade_0c02);
    be32(&mut b, cd_len);
    be32(&mut b, 0x20400);
    be32(&mut b, 0x20002); // adhoc | linker-signed
    be32(&mut b, hash_off);
    be32(&mut b, ident_off);
    be32(&mut b, 0); // special slots
    be32(&mut b, slots as u32);
    be32(&mut b, file.len() as u32);
    b.extend_from_slice(&[32, 2, 0, 12]); // SHA-256, 4K pages
    be32(&mut b, 0);
    be32(&mut b, 0); // scatter
    be32(&mut b, 0); // team
    be32(&mut b, 0);
    be64(&mut b, 0); // codeLimit64
    be64(&mut b, 0); // execSegBase
    be64(&mut b, text_filesize);
    be64(&mut b, 1); // CS_EXECSEG_MAIN_BINARY

    b.extend_from_slice(ident.as_bytes());
    b.push(0);
    for page in file.chunks(SIG_PAGE) {
        b.extend_from_slice(&sha256(page));
    }
    b
}

pub fn write_macho(obj: &Object, ident: &str) -> Vec<u8> {
    let sizeofcmds: u64 = 72 + (72 + 160) + (72 + 80) + (72 + 80) + 72 // segments
        + 16 + 16 + 24 + 80 // fixups, trie, symtab, dysymtab
        + 32 + 24 + 24 + 24 + 56 + 16; // dylinker, uuid, build, main, dylib, signature
    const NCMDS: u32 = 15;

    let text_off = align(32 + sizeofcmds, 16);
    let code_size = obj.code.len() as u64 * 4;
    let rodata_off = align(text_off + code_size, 8);
    let text_end = rodata_off + obj.rodata.len() as u64;
    let text_filesize = align(text_end.max(1), PAGE);

    let got_off = text_filesize;
    let got_addr = BASE + got_off;
    let got_size = obj.imports.len() as u64 * 8;
    let data_size = align(got_size.max(1), PAGE);
    let linkedit_off = got_off + data_size;
    // Zero-filled writable memory: takes address space, not file space.
    let bss_addr = got_addr + data_size;
    let bss_vmsize = align(obj.bss_size.max(1), PAGE);

    let fixups = chained_fixups(&obj.imports, got_off, PAGE);
    let trie = exports_trie();

    let mut strtab = vec![0u8];
    let mut nlist = W(Vec::new());
    for (name, pos) in &obj.symbols {
        nlist.u32(strtab.len() as u32);
        nlist.u8(0x0e); // N_SECT, local
        nlist.u8(1);
        nlist.u16(0);
        nlist.u64(BASE + text_off + *pos as u64 * 4);
        strtab.extend_from_slice(name.as_bytes());
        strtab.push(0);
    }
    strtab.resize(align(strtab.len() as u64, 8) as usize, 0);

    let fixups_off = linkedit_off;
    let trie_off = fixups_off + fixups.len() as u64;
    let sym_off = trie_off + trie.len() as u64;
    let str_off = sym_off + nlist.0.len() as u64;
    let sig_off = align(str_off + strtab.len() as u64, 16);
    let sig_size = signature_size(sig_off as usize, ident) as u64;
    let linkedit_filesize = sig_off + sig_size - linkedit_off;

    // Patch address references now that the layout is known.
    let mut code = obj.code.clone();
    let code_addr = BASE + text_off;
    let rodata_addr = BASE + rodata_off;
    for (at, off) in &obj.data_refs {
        let target = rodata_addr + *off as u64;
        let pc = code_addr + *at as u64 * 4;
        code[*at] = page_patch(code[*at], pc, target);
        code[*at + 1] |= ((target & 0xFFF) as u32) << 10;
    }
    for (at, pos) in &obj.code_refs {
        let target = code_addr + *pos as u64 * 4;
        let pc = code_addr + *at as u64 * 4;
        code[*at] = page_patch(code[*at], pc, target);
        code[*at + 1] |= ((target & 0xFFF) as u32) << 10;
    }
    for at in &obj.bss_refs {
        let pc = code_addr + *at as u64 * 4;
        code[*at] = page_patch(code[*at], pc, bss_addr);
        code[*at + 1] |= ((bss_addr & 0xFFF) as u32) << 10;
    }
    for (at, slot) in &obj.got_refs {
        let target = got_addr + *slot as u64 * 8;
        let pc = code_addr + *at as u64 * 4;
        code[*at] = page_patch(code[*at], pc, target);
        code[*at + 1] |= (((target & 0xFFF) / 8) as u32) << 10;
    }

    let mut w = W(Vec::with_capacity((sig_off + sig_size) as usize));
    w.u32(0xfeed_facf);
    w.u32(0x0100_000c); // arm64
    w.u32(0);
    w.u32(2); // MH_EXECUTE
    w.u32(NCMDS);
    w.u32(sizeofcmds as u32);
    w.u32(0x0020_0085); // NOUNDEFS | DYLDLINK | TWOLEVEL | PIE
    w.u32(0);

    segment(&mut w, &Seg { name: "__PAGEZERO", vmaddr: 0, vmsize: BASE, fileoff: 0, filesize: 0, maxprot: 0, initprot: 0, flags: 0, nsects: 0 });
    segment(&mut w, &Seg { name: "__TEXT", vmaddr: BASE, vmsize: text_filesize, fileoff: 0, filesize: text_filesize, maxprot: 5, initprot: 5, flags: 0, nsects: 2 });
    section(&mut w, "__text", "__TEXT", code_addr, code_size, text_off as u32, 2, 0x8000_0400);
    section(&mut w, "__const", "__TEXT", rodata_addr, obj.rodata.len() as u64, rodata_off as u32, 3, 0);
    segment(&mut w, &Seg { name: "__DATA_CONST", vmaddr: got_addr, vmsize: data_size, fileoff: got_off, filesize: data_size, maxprot: 3, initprot: 3, flags: 0x10, nsects: 1 });
    section(&mut w, "__got", "__DATA_CONST", got_addr, got_size, got_off as u32, 3, 0);
    segment(&mut w, &Seg { name: "__DATA", vmaddr: bss_addr, vmsize: bss_vmsize, fileoff: linkedit_off, filesize: 0, maxprot: 3, initprot: 3, flags: 0, nsects: 1 });
    section(&mut w, "__bss", "__DATA", bss_addr, obj.bss_size, 0, 3, 1);
    segment(&mut w, &Seg {
        name: "__LINKEDIT",
        vmaddr: bss_addr + bss_vmsize,
        vmsize: align(linkedit_filesize, PAGE),
        fileoff: linkedit_off,
        filesize: linkedit_filesize,
        maxprot: 1,
        initprot: 1,
        flags: 0,
        nsects: 0,
    });

    linkedit_cmd(&mut w, 0x8000_0034, fixups_off, fixups.len() as u64);
    linkedit_cmd(&mut w, 0x8000_0033, trie_off, trie.len() as u64);

    let nsyms = obj.symbols.len() as u32;
    w.u32(2); // LC_SYMTAB
    w.u32(24);
    w.u32(sym_off as u32);
    w.u32(nsyms);
    w.u32(str_off as u32);
    w.u32(strtab.len() as u32);

    w.u32(0x0b); // LC_DYSYMTAB
    w.u32(80);
    for v in [0, nsyms, nsyms, 0, nsyms, 0] {
        w.u32(v);
    }
    for _ in 0..12 {
        w.u32(0);
    }

    path_cmd(&mut w, 0x0e, "/usr/lib/dyld", &[]);

    // Deterministic UUID derived from the program itself.
    let mut body: Vec<u8> = code.iter().flat_map(|c| c.to_le_bytes()).collect();
    body.extend_from_slice(&obj.rodata);
    let digest = sha256(&body);
    let mut uuid = [0u8; 16];
    uuid.copy_from_slice(&digest[..16]);
    uuid[6] = (uuid[6] & 0x0F) | 0x40;
    uuid[8] = (uuid[8] & 0x3F) | 0x80;
    w.u32(0x1b);
    w.u32(24);
    w.0.extend_from_slice(&uuid);

    w.u32(0x32); // LC_BUILD_VERSION
    w.u32(24);
    w.u32(1); // macOS
    w.u32(0x000b_0000); // minos 11.0
    w.u32(0x000f_0000); // sdk 15.0
    w.u32(0);

    w.u32(0x8000_0028); // LC_MAIN
    w.u32(24);
    w.u64(text_off + obj.entry as u64 * 4);
    w.u64(0);

    path_cmd(&mut w, 0x0c, "/usr/lib/libSystem.B.dylib", &[2, 0x0001_0000, 0x0001_0000]);

    linkedit_cmd(&mut w, 0x1d, sig_off, sig_size);

    debug_assert_eq!(w.0.len() as u64, 32 + sizeofcmds);

    w.pad_to(text_off as usize);
    for c in &code {
        w.u32(*c);
    }
    w.pad_to(rodata_off as usize);
    w.0.extend_from_slice(&obj.rodata);
    w.pad_to(got_off as usize);

    // GOT: a chain of binds, each pointing 8 bytes ahead to the next.
    let n = obj.imports.len();
    for i in 0..n {
        let next = if i + 1 < n { 2u64 } else { 0 };
        w.u64(i as u64 | (next << 51) | (1u64 << 63));
    }
    w.pad_to(linkedit_off as usize);

    w.0.extend_from_slice(&fixups);
    w.0.extend_from_slice(&trie);
    w.0.extend_from_slice(&nlist.0);
    w.0.extend_from_slice(&strtab);
    w.pad_to(sig_off as usize);

    let sig = sign(&w.0, ident, text_filesize);
    debug_assert_eq!(sig.len() as u64, sig_size);
    w.0.extend_from_slice(&sig);
    w.0
}

/// x86-64 pages are 4 KB.
const PAGE_X64: u64 = 0x1000;

/// Patches the rip-relative `disp32` at `pos` so that it reaches `target`.
fn rip_patch(code: &mut [u8], pos: usize, code_addr: u64, target: u64) {
    let disp = target as i64 - (code_addr + pos as u64 + 4) as i64;
    code[pos..pos + 4].copy_from_slice(&(disp as i32).to_le_bytes());
}

/// The x86-64 twin of `write_macho`: same commands, 4 KB pages, byte-addressed code.
/// Compared with `clang -arch x86_64` (otool -l, dyld_info, codesign -dvvv).
pub fn write_macho_x64(obj: &ObjectX64, ident: &str) -> Vec<u8> {
    let sizeofcmds: u64 = 72 + (72 + 160) + (72 + 80) + (72 + 80) + 72 // segments
        + 16 + 16 + 24 + 80 // fixups, trie, symtab, dysymtab
        + 32 + 24 + 24 + 24 + 56 + 16; // dylinker, uuid, build, main, dylib, signature
    const NCMDS: u32 = 15;

    let text_off = align(32 + sizeofcmds, 16);
    let code_size = obj.code.len() as u64;
    let rodata_off = align(text_off + code_size, 8);
    let text_end = rodata_off + obj.rodata.len() as u64;
    let text_filesize = align(text_end.max(1), PAGE_X64);

    let got_off = text_filesize;
    let got_addr = BASE + got_off;
    let got_size = obj.imports.len() as u64 * 8;
    let data_size = align(got_size.max(1), PAGE_X64);
    let linkedit_off = got_off + data_size;
    let bss_addr = got_addr + data_size;
    let bss_vmsize = align(obj.bss_size.max(1), PAGE_X64);

    let fixups = chained_fixups(&obj.imports, got_off, PAGE_X64);
    let trie = exports_trie();

    let mut strtab = vec![0u8];
    let mut nlist = W(Vec::new());
    for (name, pos) in &obj.symbols {
        nlist.u32(strtab.len() as u32);
        nlist.u8(0x0e); // N_SECT, local
        nlist.u8(1);
        nlist.u16(0);
        nlist.u64(BASE + text_off + *pos as u64);
        strtab.extend_from_slice(name.as_bytes());
        strtab.push(0);
    }
    strtab.resize(align(strtab.len() as u64, 8) as usize, 0);

    let fixups_off = linkedit_off;
    let trie_off = fixups_off + fixups.len() as u64;
    let sym_off = trie_off + trie.len() as u64;
    let str_off = sym_off + nlist.0.len() as u64;
    let sig_off = align(str_off + strtab.len() as u64, 16);
    let sig_size = signature_size(sig_off as usize, ident) as u64;
    let linkedit_filesize = sig_off + sig_size - linkedit_off;

    let mut code = obj.code.clone();
    let code_addr = BASE + text_off;
    let rodata_addr = BASE + rodata_off;
    for (pos, off) in &obj.data_refs {
        rip_patch(&mut code, *pos, code_addr, rodata_addr + *off as u64);
    }
    for (pos, off) in &obj.bss_refs {
        rip_patch(&mut code, *pos, code_addr, bss_addr + *off);
    }
    for (pos, slot) in &obj.got_refs {
        rip_patch(&mut code, *pos, code_addr, got_addr + *slot as u64 * 8);
    }

    let mut w = W(Vec::with_capacity((sig_off + sig_size) as usize));
    w.u32(0xfeed_facf);
    w.u32(0x0100_0007); // x86_64
    w.u32(3); // CPU_SUBTYPE_X86_64_ALL
    w.u32(2); // MH_EXECUTE
    w.u32(NCMDS);
    w.u32(sizeofcmds as u32);
    w.u32(0x0020_0085); // NOUNDEFS | DYLDLINK | TWOLEVEL | PIE
    w.u32(0);

    segment(&mut w, &Seg { name: "__PAGEZERO", vmaddr: 0, vmsize: BASE, fileoff: 0, filesize: 0, maxprot: 0, initprot: 0, flags: 0, nsects: 0 });
    segment(&mut w, &Seg { name: "__TEXT", vmaddr: BASE, vmsize: text_filesize, fileoff: 0, filesize: text_filesize, maxprot: 5, initprot: 5, flags: 0, nsects: 2 });
    section(&mut w, "__text", "__TEXT", code_addr, code_size, text_off as u32, 4, 0x8000_0400);
    section(&mut w, "__const", "__TEXT", rodata_addr, obj.rodata.len() as u64, rodata_off as u32, 3, 0);
    segment(&mut w, &Seg { name: "__DATA_CONST", vmaddr: got_addr, vmsize: data_size, fileoff: got_off, filesize: data_size, maxprot: 3, initprot: 3, flags: 0x10, nsects: 1 });
    section(&mut w, "__got", "__DATA_CONST", got_addr, got_size, got_off as u32, 3, 0);
    segment(&mut w, &Seg { name: "__DATA", vmaddr: bss_addr, vmsize: bss_vmsize, fileoff: linkedit_off, filesize: 0, maxprot: 3, initprot: 3, flags: 0, nsects: 1 });
    section(&mut w, "__bss", "__DATA", bss_addr, obj.bss_size, 0, 4, 1);
    segment(&mut w, &Seg {
        name: "__LINKEDIT",
        vmaddr: bss_addr + bss_vmsize,
        vmsize: align(linkedit_filesize, PAGE_X64),
        fileoff: linkedit_off,
        filesize: linkedit_filesize,
        maxprot: 1,
        initprot: 1,
        flags: 0,
        nsects: 0,
    });

    linkedit_cmd(&mut w, 0x8000_0034, fixups_off, fixups.len() as u64);
    linkedit_cmd(&mut w, 0x8000_0033, trie_off, trie.len() as u64);

    let nsyms = obj.symbols.len() as u32;
    w.u32(2); // LC_SYMTAB
    w.u32(24);
    w.u32(sym_off as u32);
    w.u32(nsyms);
    w.u32(str_off as u32);
    w.u32(strtab.len() as u32);

    w.u32(0x0b); // LC_DYSYMTAB
    w.u32(80);
    for v in [0, nsyms, nsyms, 0, nsyms, 0] {
        w.u32(v);
    }
    for _ in 0..12 {
        w.u32(0);
    }

    path_cmd(&mut w, 0x0e, "/usr/lib/dyld", &[]);

    let mut body = code.clone();
    body.extend_from_slice(&obj.rodata);
    let digest = sha256(&body);
    let mut uuid = [0u8; 16];
    uuid.copy_from_slice(&digest[..16]);
    uuid[6] = (uuid[6] & 0x0F) | 0x40;
    uuid[8] = (uuid[8] & 0x3F) | 0x80;
    w.u32(0x1b);
    w.u32(24);
    w.0.extend_from_slice(&uuid);

    w.u32(0x32); // LC_BUILD_VERSION
    w.u32(24);
    w.u32(1); // macOS
    w.u32(0x000c_0000); // minos 12.0: the first release where x86_64 binaries use chained fixups
    w.u32(0x000f_0000); // sdk 15.0
    w.u32(0);

    w.u32(0x8000_0028); // LC_MAIN
    w.u32(24);
    w.u64(text_off + obj.entry as u64);
    w.u64(0);

    path_cmd(&mut w, 0x0c, "/usr/lib/libSystem.B.dylib", &[2, 0x0001_0000, 0x0001_0000]);

    linkedit_cmd(&mut w, 0x1d, sig_off, sig_size);

    debug_assert_eq!(w.0.len() as u64, 32 + sizeofcmds);

    w.pad_to(text_off as usize);
    w.0.extend_from_slice(&code);
    w.pad_to(rodata_off as usize);
    w.0.extend_from_slice(&obj.rodata);
    w.pad_to(got_off as usize);

    let n = obj.imports.len();
    for i in 0..n {
        let next = if i + 1 < n { 2u64 } else { 0 };
        w.u64(i as u64 | (next << 51) | (1u64 << 63));
    }
    w.pad_to(linkedit_off as usize);

    w.0.extend_from_slice(&fixups);
    w.0.extend_from_slice(&trie);
    w.0.extend_from_slice(&nlist.0);
    w.0.extend_from_slice(&strtab);
    w.pad_to(sig_off as usize);

    let sig = sign(&w.0, ident, text_filesize);
    debug_assert_eq!(sig.len() as u64, sig_size);
    w.0.extend_from_slice(&sig);
    w.0
}
