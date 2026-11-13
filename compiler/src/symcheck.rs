//! Before a program that calls C is written out: is every C function it calls really in one of the libraries it
//! loads? Without this check a missing symbol shows up only when the program starts (`dyld: Symbol not found`).
//!
//! A library file on disk is read, not loaded: its export trie lists the names it has, and no code of it runs.
//! System libraries have no file (they live in the dyld shared cache), so those are opened with `dlopen`; that runs
//! only Apple's own code. When some library cannot be read, nothing is reported: the check must never be wrong.

use std::ffi::{c_char, c_int, c_void, CString};

unsafe extern "C" {
    fn dlopen(path: *const c_char, mode: c_int) -> *mut c_void;
    fn dlsym(handle: *mut c_void, name: *const c_char) -> *mut c_void;
}

const RTLD_LAZY: c_int = 0x1;
const RTLD_LOCAL: c_int = 0x4;

/// What a library offers, as far as it can be told.
enum Lib {
    /// The exported names, with their leading `_`.
    Names(Vec<String>),
    /// Opened by the dynamic loader.
    Handle(*mut c_void),
}

impl Lib {
    fn has(&self, sym: &str) -> bool {
        match self {
            Lib::Names(n) => n.iter().any(|x| x.strip_prefix('_') == Some(sym)),
            Lib::Handle(h) => {
                let Ok(c) = CString::new(sym) else { return false };
                // Safety: a handle from dlopen and a NUL-terminated name.
                !unsafe { dlsym(*h, c.as_ptr()) }.is_null()
            }
        }
    }
}

fn uleb(b: &[u8], at: &mut usize) -> Option<u64> {
    let mut v = 0u64;
    let mut shift = 0;
    loop {
        let x = *b.get(*at)?;
        *at += 1;
        v |= ((x & 0x7f) as u64) << shift;
        if x & 0x80 == 0 {
            return Some(v);
        }
        shift += 7;
        if shift > 63 {
            return None;
        }
    }
}

/// Every name in a Mach-O export trie.
fn trie_names(t: &[u8]) -> Option<Vec<String>> {
    let mut out = Vec::new();
    let mut work = vec![(0usize, Vec::<u8>::new())];
    let mut visited = 0;
    while let Some((node, prefix)) = work.pop() {
        visited += 1;
        if visited > 2_000_000 {
            return None;
        }
        let mut at = node;
        let term = uleb(t, &mut at)? as usize;
        if term > 0 {
            out.push(String::from_utf8_lossy(&prefix).into_owned());
        }
        at = at.checked_add(term)?;
        let kids = *t.get(at)?;
        at += 1;
        for _ in 0..kids {
            let end = at + t.get(at..)?.iter().position(|&c| c == 0)?;
            let mut name = prefix.clone();
            name.extend_from_slice(&t[at..end]);
            at = end + 1;
            let child = uleb(t, &mut at)? as usize;
            if child >= t.len() {
                return None;
            }
            work.push((child, name));
        }
    }
    Some(out)
}

fn u32le(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

fn u32be(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

/// The exported names of one 64-bit Mach-O image.
fn macho_names(b: &[u8]) -> Option<Vec<String>> {
    if u32le(b, 0)? != 0xfeed_facf {
        return None;
    }
    let ncmds = u32le(b, 16)?;
    let mut at = 32;
    for _ in 0..ncmds {
        let cmd = u32le(b, at)?;
        let size = u32le(b, at + 4)? as usize;
        // A library that passes on another one's names whole: its own list is not the full story.
        if cmd == 0x8000_001f {
            return None;
        }
        let (off, len) = match cmd {
            // LC_DYLD_EXPORTS_TRIE: dataoff, datasize.
            0x8000_0033 => (u32le(b, at + 8)? as usize, u32le(b, at + 12)? as usize),
            // LC_DYLD_INFO(_ONLY): export_off, export_size are its last pair.
            0x22 | 0x8000_0022 => (u32le(b, at + 40)? as usize, u32le(b, at + 44)? as usize),
            _ => {
                at += size;
                continue;
            }
        };
        return trie_names(b.get(off..off + len)?);
    }
    Some(Vec::new())
}

/// A library file: a plain Mach-O, or a universal one (each slice exports the same names; the first readable wins).
fn file_names(b: &[u8]) -> Option<Vec<String>> {
    if u32be(b, 0)? == 0xcafe_babe {
        let n = u32be(b, 4)? as usize;
        for i in 0..n.min(16) {
            let e = 8 + i * 20;
            let (off, size) = (u32be(b, e + 8)? as usize, u32be(b, e + 12)? as usize);
            if let Some(names) = b.get(off..off + size).and_then(macho_names) {
                return Some(names);
            }
        }
        return None;
    }
    macho_names(b)
}

fn open(path: &str) -> Option<Lib> {
    if let Ok(bytes) = std::fs::read(path) {
        return file_names(&bytes).map(Lib::Names);
    }
    // Not on disk: a system library in the shared cache.
    if !path.starts_with("/usr/lib/") && !path.starts_with("/System/") {
        return None;
    }
    let c = CString::new(path).ok()?;
    // Safety: a NUL-terminated path; the flags ask for nothing to be bound or exported further.
    let h = unsafe { dlopen(c.as_ptr(), RTLD_LAZY | RTLD_LOCAL) };
    (!h.is_null()).then_some(Lib::Handle(h))
}

/// The symbols (without `_`) that no library has. Empty when every one was found or when it cannot be told.
pub fn missing(syms: &[String], libs: &[String]) -> Vec<String> {
    let mut opened = Vec::new();
    for l in std::iter::once("/usr/lib/libSystem.B.dylib").chain(libs.iter().map(|s| s.as_str())) {
        match open(l) {
            Some(x) => opened.push(x),
            None => return Vec::new(),
        }
    }
    syms.iter().filter(|s| !opened.iter().any(|l| l.has(s))).cloned().collect()
}
