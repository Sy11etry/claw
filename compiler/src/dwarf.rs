//! `claw build -g`: the debugger's view of a program.
//!
//! The executable stays as it is. Next to it goes `name.dSYM`, a small Mach-O file with the same
//! UUID that holds DWARF 4: one compile unit, one entry per function, and the line table. That is
//! enough for lldb to stop on `file.claw:12`, show the source, and print a backtrace with lines.

use crate::cir::Loc;
use crate::codegen::Object;
use std::path::{Path, PathBuf};

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
    fn uleb(&mut self, mut v: u64) {
        loop {
            let b = (v & 0x7F) as u8;
            v >>= 7;
            if v == 0 {
                self.u8(b);
                break;
            }
            self.u8(b | 0x80);
        }
    }
    fn sleb(&mut self, mut v: i64) {
        loop {
            let b = (v & 0x7F) as u8;
            v >>= 7;
            let done = (v == 0 && b & 0x40 == 0) || (v == -1 && b & 0x40 != 0);
            if done {
                self.u8(b);
                break;
            }
            self.u8(b | 0x80);
        }
    }
    fn cstr(&mut self, s: &str) {
        self.0.extend_from_slice(s.as_bytes());
        self.0.push(0);
    }
    fn name16(&mut self, s: &str) {
        let mut b = [0u8; 16];
        b[..s.len()].copy_from_slice(s.as_bytes());
        self.0.extend_from_slice(&b);
    }
}

/// What the dSYM needs to know about the executable it describes.
struct Image {
    uuid: [u8; 16],
    text_addr: u64,
    text_size: u64,
    seg_addr: u64,
    seg_size: u64,
    end: u64,
}

fn u32_at(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(b[at..at + 4].try_into().unwrap())
}
fn u64_at(b: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(b[at..at + 8].try_into().unwrap())
}

/// Reads the UUID and the place of `__text` out of the finished executable.
fn read_image(b: &[u8]) -> Option<Image> {
    let ncmds = u32_at(b, 16);
    let mut at = 32;
    let (mut uuid, mut text) = (None, None);
    let mut end = 0;
    for _ in 0..ncmds {
        let cmd = u32_at(b, at);
        let size = u32_at(b, at + 4) as usize;
        match cmd {
            0x1b => uuid = Some(<[u8; 16]>::try_from(&b[at + 8..at + 24]).ok()?),
            0x19 => {
                let name = &b[at + 8..at + 24];
                let (addr, vmsize) = (u64_at(b, at + 24), u64_at(b, at + 32));
                end = end.max(addr + vmsize);
                if name.starts_with(b"__TEXT\0") {
                    let nsects = u32_at(b, at + 64) as usize;
                    for k in 0..nsects {
                        let s = at + 72 + k * 80;
                        if b[s..s + 16].starts_with(b"__text\0") {
                            text = Some((addr, vmsize, u64_at(b, s + 32), u64_at(b, s + 40)));
                        }
                    }
                }
            }
            _ => {}
        }
        at += size;
    }
    let (seg_addr, seg_size, text_addr, text_size) = text?;
    Some(Image { uuid: uuid?, text_addr, text_size, seg_addr, seg_size, end })
}

fn absolute(name: &str) -> String {
    // Files of the standard library are not on disk under their short names.
    Path::new(name).canonicalize().map_or_else(|_| name.to_string(), |c| c.to_string_lossy().into_owned())
}

struct Row {
    addr: u64,
    loc: Loc,
}

struct Func {
    name: String,
    start: u64,
    end: u64,
    loc: Loc,
}

fn line_table(rows: &[Row], files: &[String], text_addr: u64, text_size: u64) -> Vec<u8> {
    let mut hdr = W(Vec::new());
    hdr.u8(1); // minimum instruction length
    hdr.u8(1); // maximum operations per instruction
    hdr.u8(1); // default is_stmt
    hdr.u8(-5i8 as u8); // line base
    hdr.u8(14); // line range
    hdr.u8(13); // opcode base
    for n in [0u8, 1, 1, 1, 1, 0, 0, 0, 1, 0, 0, 1] {
        hdr.u8(n);
    }
    hdr.u8(0); // no include directories: names are absolute
    for f in files {
        hdr.cstr(&absolute(f));
        hdr.uleb(0);
        hdr.uleb(0);
        hdr.uleb(0);
    }
    hdr.u8(0);

    let mut prog = W(Vec::new());
    prog.u8(0);
    prog.uleb(9);
    prog.u8(2); // DW_LNE_set_address
    prog.u64(text_addr);
    let (mut addr, mut line, mut file, mut col) = (text_addr, 1i64, 1u64, 0u64);
    for r in rows {
        let want_file = r.loc.file as u64 + 1;
        if want_file != file {
            prog.u8(4);
            prog.uleb(want_file);
            file = want_file;
        }
        if r.loc.col as u64 != col {
            prog.u8(5);
            prog.uleb(r.loc.col as u64);
            col = r.loc.col as u64;
        }
        if r.addr != addr {
            prog.u8(2);
            prog.uleb(r.addr - addr);
            addr = r.addr;
        }
        if r.loc.line as i64 != line {
            prog.u8(3);
            prog.sleb(r.loc.line as i64 - line);
            line = r.loc.line as i64;
        }
        prog.u8(1); // DW_LNS_copy
    }
    let end = text_addr + text_size;
    if end != addr {
        prog.u8(2);
        prog.uleb(end - addr);
    }
    prog.u8(0);
    prog.uleb(1);
    prog.u8(1); // DW_LNE_end_sequence

    let mut out = W(Vec::new());
    out.u32((2 + 4 + hdr.0.len() + prog.0.len()) as u32);
    out.u16(4);
    out.u32(hdr.0.len() as u32);
    out.0.extend_from_slice(&hdr.0);
    out.0.extend_from_slice(&prog.0);
    out.0
}

/// Which addresses belong to the one compile unit, so an address finds its line table.
fn aranges(text_addr: u64, text_size: u64) -> Vec<u8> {
    let mut a = W(Vec::new());
    a.u16(2);
    a.u32(0);
    a.u8(8);
    a.u8(0);
    a.u32(0); // pad the 12 header bytes to a multiple of the 16-byte tuple
    a.u64(text_addr);
    a.u64(text_size);
    a.u64(0);
    a.u64(0);
    let mut out = W(Vec::new());
    out.u32(a.0.len() as u32);
    out.0.extend_from_slice(&a.0);
    out.0
}

fn abbrev() -> Vec<u8> {
    let mut a = W(Vec::new());
    // 1: the compile unit
    a.uleb(1);
    a.uleb(0x11);
    a.u8(1);
    for (at, form) in [(0x25u64, 0x08u64), (0x13, 0x05), (0x03, 0x08), (0x1b, 0x08), (0x11, 0x01), (0x12, 0x07), (0x10, 0x17)] {
        a.uleb(at);
        a.uleb(form);
    }
    a.uleb(0);
    a.uleb(0);
    // 2: a function
    a.uleb(2);
    a.uleb(0x2e);
    a.u8(0);
    for (at, form) in [(0x03u64, 0x08u64), (0x11, 0x01), (0x12, 0x07), (0x3a, 0x0b), (0x3b, 0x06), (0x3f, 0x19)] {
        a.uleb(at);
        a.uleb(form);
    }
    a.uleb(0);
    a.uleb(0);
    a.u8(0);
    a.0
}

fn info(funcs: &[Func], files: &[String], text_addr: u64, text_size: u64) -> Vec<u8> {
    let mut d = W(Vec::new());
    d.u16(4);
    d.u32(0);
    d.u8(8);
    d.uleb(1);
    d.cstr(concat!("claw ", env!("CARGO_PKG_VERSION")));
    d.u16(0x000c); // DW_LANG_C99: the debugger prints values the way C does
    d.cstr(&absolute(files.first().map(|s| s.as_str()).unwrap_or("a.claw")));
    d.cstr(&std::env::current_dir().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default());
    d.u64(text_addr);
    d.u64(text_size);
    d.u32(0);
    for f in funcs {
        d.uleb(2);
        d.cstr(f.name.strip_prefix('_').unwrap_or(&f.name));
        d.u64(f.start);
        d.u64(f.end - f.start);
        d.u8((f.loc.file + 1).min(255) as u8);
        d.u32(f.loc.line);
    }
    d.u8(0);
    let mut out = W(Vec::new());
    out.u32(d.0.len() as u32);
    out.0.extend_from_slice(&d.0);
    out.0
}

/// Writes `<out>.dSYM` for the executable `exe` that `obj` became.
pub fn write_dsym(out: &Path, obj: &Object, exe: &[u8], files: &[String]) -> Result<PathBuf, String> {
    let img = read_image(exe).ok_or("cannot read the executable's layout")?;
    let at = |i: usize| img.text_addr + i as u64 * 4;

    // Functions run from one symbol to the next; the runtime's own helpers have no source.
    let mut syms: Vec<&(String, usize)> = obj.symbols.iter().collect();
    syms.sort_by_key(|s| s.1);
    let mut rows: Vec<Row> = obj.lines.iter().map(|(i, l)| Row { addr: at(*i), loc: *l }).collect();
    rows.sort_by_key(|r| r.addr);
    let mut funcs = Vec::new();
    for (k, (name, pos)) in syms.iter().enumerate() {
        let end = syms.get(k + 1).map_or(obj.code.len(), |s| s.1);
        let (start, stop) = (at(*pos), at(end));
        if let Some(first) = rows.iter().find(|r| r.addr >= start && r.addr < stop) {
            funcs.push(Func { name: name.clone(), start, end: stop, loc: first.loc });
        }
    }
    // A row at each function's first instruction lets the debugger skip the prologue by line.
    for f in &funcs {
        if !rows.iter().any(|r| r.addr == f.start) {
            rows.push(Row { addr: f.start, loc: f.loc });
        }
    }
    rows.sort_by_key(|r| r.addr);

    let sections: Vec<(&str, Vec<u8>)> = vec![
        ("__debug_line", line_table(&rows, files, img.text_addr, img.text_size)),
        ("__debug_info", info(&funcs, files, img.text_addr, img.text_size)),
        ("__debug_abbrev", abbrev()),
        ("__debug_aranges", aranges(img.text_addr, img.text_size)),
    ];

    // Header, then the commands, then the section bytes one after another.
    let seg_cmd = |nsects: u32| 72 + 80 * nsects;
    let sizeofcmds = 24 + 24 + seg_cmd(1) + seg_cmd(sections.len() as u32) + 24;
    let data_start = (32 + sizeofcmds as u64 + 15) & !15;
    let mut offsets = Vec::new();
    let mut cursor = data_start;
    for (_, bytes) in &sections {
        offsets.push(cursor);
        cursor += bytes.len() as u64;
    }
    let dwarf_size = cursor - data_start;
    let dwarf_addr = (img.end + 0x3FFF) & !0x3FFF;

    let mut w = W(Vec::new());
    w.u32(0xfeed_facf);
    w.u32(0x0100_000c);
    w.u32(0);
    w.u32(0xA); // MH_DSYM
    w.u32(5);
    w.u32(sizeofcmds);
    w.u32(0);
    w.u32(0);

    w.u32(0x1b);
    w.u32(24);
    w.0.extend_from_slice(&img.uuid);

    w.u32(0x32); // LC_BUILD_VERSION: macOS, no tools
    w.u32(24);
    w.u32(1);
    w.u32(0x000e_0000);
    w.u32(0x000e_0000);
    w.u32(0);

    // __TEXT with no bytes of its own: it only says where the code sits.
    w.u32(0x19);
    w.u32(seg_cmd(1));
    w.name16("__TEXT");
    w.u64(img.seg_addr);
    w.u64(img.seg_size);
    w.u64(0);
    w.u64(0);
    w.u32(5);
    w.u32(5);
    w.u32(1);
    w.u32(0);
    w.name16("__text");
    w.name16("__TEXT");
    w.u64(img.text_addr);
    w.u64(img.text_size);
    w.u32(0);
    w.u32(2);
    w.u32(0);
    w.u32(0);
    w.u32(0x8000_0400);
    w.u32(0);
    w.u32(0);
    w.u32(0);

    w.u32(0x19);
    w.u32(seg_cmd(sections.len() as u32));
    w.name16("__DWARF");
    w.u64(dwarf_addr);
    w.u64((dwarf_size + 0x3FFF) & !0x3FFF);
    w.u64(data_start);
    w.u64(dwarf_size);
    w.u32(7);
    w.u32(3);
    w.u32(sections.len() as u32);
    w.u32(0);
    let mut addr = dwarf_addr;
    for ((name, bytes), off) in sections.iter().zip(&offsets) {
        w.name16(name);
        w.name16("__DWARF");
        w.u64(addr);
        w.u64(bytes.len() as u64);
        w.u32(*off as u32);
        w.u32(0);
        w.u32(0);
        w.u32(0);
        w.u32(0x0200_0000); // S_ATTR_DEBUG
        w.u32(0);
        w.u32(0);
        w.u32(0);
        addr += bytes.len() as u64;
    }

    w.u32(0x2); // LC_SYMTAB with nothing in it: the names live in the DWARF
    w.u32(24);
    w.u32(cursor as u32);
    w.u32(0);
    w.u32(cursor as u32);
    w.u32(0);

    w.0.resize(data_start as usize, 0);
    for (_, bytes) in &sections {
        w.0.extend_from_slice(bytes);
    }

    let name = out.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "a.out".into());
    let bundle = PathBuf::from(format!("{}.dSYM", out.display()));
    let dir = bundle.join("Contents/Resources/DWARF");
    std::fs::create_dir_all(&dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    std::fs::write(dir.join(&name), &w.0).map_err(|e| format!("cannot write the debug file: {e}"))?;
    let plist = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\">\n<dict>\n<key>CFBundleIdentifier</key><string>com.apple.xcode.dsym.{name}</string>\n<key>CFBundlePackageType</key><string>dSYM</string>\n<key>CFBundleSignature</key><string>????</string>\n<key>CFBundleShortVersionString</key><string>1.0</string>\n<key>CFBundleVersion</key><string>1</string>\n</dict>\n</plist>\n"
    );
    std::fs::write(bundle.join("Contents/Info.plist"), plist).map_err(|e| format!("cannot write the debug file: {e}"))?;
    Ok(bundle)
}
