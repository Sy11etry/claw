//! `claw`: the Claw compiler (stage 0, in Rust, no dependencies).

mod arm64;
mod ast;
mod cir;
mod codegen;
mod codegen_x64;
mod ctfe;
mod diag;
mod dwarf;
mod elf;
mod dump;
mod effects;
mod fmt;
mod fuzz;
mod info;
mod interp;
mod lexer;
mod lower;
mod lsp;
mod macho;
mod modules;
mod opt;
mod parser;
mod ranges;
mod regalloc;
mod sema;
mod sha256;
mod ssa;
mod types;
mod why;
mod x64;

use diag::Diag;
use std::path::{Path, PathBuf};
use std::process::{exit, Command};

const VERSION: &str = env!("CARGO_PKG_VERSION");

const USAGE: &str = "claw run <file.claw>            compile with all checks and run
claw test <file.claw>           run the `test` blocks, then try every function without `+io` on generated inputs
    --blocks                    only the `test` blocks
claw why <file.claw>            run again and say which lines led to the failure
    --test <name>               of that `test` block instead of `main`
claw fix <file.claw>            apply the compiler's safe patches
claw build <file.claw> [-o out] optimized executable; contracts with calls are left out
    --fast                      remove every run-time check: speed like C, risk like C
    --stats                     say how many checks were proved unnecessary
    -g                          debug info so lldb can stop on source lines (<out>.dSYM on macOS, inside the ELF on Linux)
claw check <file.claw>          report errors only
claw card                       the language on one page
claw sig <file.claw> | std      only the signatures, with the effects worked out
    --pub                       only what the module exports
claw ctx <file.claw> <fn>       one function and what it takes to change it
claw audit <file.claw>          every place the compiler has to trust
claw fmt <file.claw> [--check]  put a file into the one standard format
claw lsp                        language server on stdin and stdout, for editors
claw --version
flags: --json (errors as JSON), --emit-cir (print the IR)
       --target arm64|x86_64|aarch64-linux|aarch64-linux-musl (default arm64; Linux targets build only)
";

struct Options {
    file: String,
    out: Option<String>,
    json: bool,
    emit_cir: bool,
    check_only: bool,
    alloc_stats: bool,
    /// `--fast`: no run-time checks at all.
    fast: bool,
    stats: bool,
    /// `claw test --blocks`: no generated inputs, no explanations.
    blocks: bool,
    /// `claw build -g`: write `name.dSYM` with line tables for lldb.
    debug: bool,
    /// `claw build --no-opt`: what the compiler written in Claw produces, for comparing.
    no_opt: bool,
    /// `--target aarch64-linux`: an ELF for Linux instead of a Mach-O for macOS.
    linux: Option<elf::Libc>,
    /// `claw sig --pub`.
    public: bool,
    /// `claw ctx file name`: the function asked about.
    name: Option<String>,
    /// `claw why --test name`.
    test_name: Option<String>,
    /// `--target x86_64`: write an x86-64 executable instead of an arm64 one.
    x86_64: bool,
}

fn parse_options(args: &[String]) -> Options {
    options(args, false)
}

fn options(args: &[String], ctx: bool) -> Options {
    let mut o = Options { file: String::new(), out: None, json: false, emit_cir: false, check_only: false, alloc_stats: false, fast: false, stats: false, blocks: false, public: false, name: None, no_opt: false, debug: false, test_name: None, linux: None, x86_64: false };
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--json" => o.json = true,
            "--emit-cir" => o.emit_cir = true,
            "--check" => o.check_only = true,
            "--alloc-stats" => o.alloc_stats = true,
            "--fast" => o.fast = true,
            "--stats" => o.stats = true,
            "--blocks" => o.blocks = true,
            "--no-opt" => o.no_opt = true,
            "--target" => {
                i += 1;
                o.x86_64 = false;
                o.linux = None;
                match args.get(i).map(|s| s.as_str()) {
                    Some("arm64" | "aarch64" | "arm64-macos") => {}
                    Some("x86_64" | "x86-64" | "x64" | "amd64") => o.x86_64 = true,
                    Some("aarch64-linux" | "aarch64-linux-gnu" | "arm64-linux") => o.linux = Some(elf::Libc::Gnu),
                    Some("aarch64-linux-musl") => o.linux = Some(elf::Libc::Musl),
                    _ => die("`--target` is `arm64`, `x86_64`, `aarch64-linux` (glibc) or `aarch64-linux-musl`"),
                }
            }
            "-g" => o.debug = true,
            "--pub" => o.public = true,
            "--test" => {
                i += 1;
                match args.get(i) {
                    Some(p) => o.test_name = Some(p.clone()),
                    None => die("`--test` needs a name"),
                }
            }
            "-o" => {
                i += 1;
                match args.get(i) {
                    Some(p) => o.out = Some(p.clone()),
                    None => die("`-o` needs a path"),
                }
            }
            a if a.starts_with('-') => die(&format!("unknown flag `{a}`")),
            a if o.file.is_empty() => o.file = a.to_string(),
            a if ctx && o.name.is_none() => o.name = Some(a.to_string()),
            a => die(&format!("unexpected argument `{a}`")),
        }
        i += 1;
    }
    if o.file.is_empty() {
        die("no input file");
    }
    lower::LINUX.store(o.linux.is_some(), std::sync::atomic::Ordering::Relaxed);
    o
}

fn die(msg: &str) -> ! {
    eprint!("claw: {msg}\n{USAGE}");
    exit(2);
}

fn report(o: &Options, mut diags: Vec<Diag>, files: &[String]) -> ! {
    diag::sort(&mut diags);
    if o.json {
        print!("{}", diag::render_json(files, &diags));
    } else {
        eprint!("{}", diag::render_text(files, &diags));
    }
    exit(1);
}

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Build,
    Run,
    Test,
}

/// A program that has passed every check, and what those checks found out.
struct Front {
    loaded: modules::Loaded,
    checked: sema::Checked,
    effects: effects::Report,
    consts: Vec<lower::ConstData>,
}

/// Runs the front end; never returns on a source error.
fn front(o: &Options, memory_checker: bool, tests_only: bool) -> Front {
    let loaded = match modules::load(&o.file, memory_checker) {
        Ok(l) => l,
        Err((diags, files)) => report(o, diags, &files),
    };
    let mut diags = Vec::new();
    let checked = sema::check(&loaded.units, loaded.core, tests_only, &mut diags);
    let effects = effects::check(&loaded.units, &checked, &mut diags);
    if !diags.is_empty() {
        report(o, diags, &loaded.files);
    }
    let consts = ctfe::evaluate(&loaded.units, &checked, &loaded.base_names, &mut diags);
    if !diags.is_empty() {
        report(o, diags, &loaded.files);
    }
    Front { loaded, checked, effects, consts }
}

/// The program as the interpreter runs it: every contract, and the source lines.
fn for_interp(f: &Front, test: bool) -> cir::Module {
    let opts = lower::Opts { check: true, test, fast: false, interp: true, debug: false, consts: &f.consts };
    lower::lower(&f.loaded.units, &f.checked, &f.loaded.base_names, opts)
}

fn compile_to_cir(o: &Options, mode: Mode) -> (cir::Module, Vec<String>, Vec<String>) {
    let f = front(o, mode != Mode::Build, mode == Mode::Test);
    let opts = lower::Opts { check: mode != Mode::Build, test: mode == Mode::Test, fast: o.fast && mode == Mode::Build, interp: false, debug: o.debug && mode == Mode::Build, consts: &f.consts };
    if mode == Mode::Build && f.effects.slow > 0 {
        eprintln!("claw: note: {} checks that call functions are left out of this build; `claw run` and `claw test` do them", f.effects.slow);
    }
    let mut module = lower::lower(&f.loaded.units, &f.checked, &f.loaded.base_names, opts);
    if mode == Mode::Build && !o.no_opt {
        let stats = opt::optimize(&mut module, o.fast);
        if o.stats {
            let gone = stats.checks_before.saturating_sub(stats.checks_after);
            println!("checks: {} of {} removed, {} calls inlined", gone, stats.checks_before, stats.inlined);
        }
    }
    let names = f.loaded.base_names.clone();
    (module, names, f.loaded.files.clone())
}

fn format_file(o: &Options) -> ! {
    let src = match std::fs::read_to_string(&o.file) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("claw: cannot read {}: {e}", o.file);
            exit(2);
        }
    };
    let out = match fmt::format(&src) {
        Ok(s) => s,
        Err(diags) => report(o, diags, std::slice::from_ref(&o.file)),
    };
    if out == src {
        exit(0);
    }
    if o.check_only {
        eprintln!("{}: not formatted; run `claw fmt`", o.file);
        exit(1);
    }
    if let Err(e) = std::fs::write(&o.file, out) {
        eprintln!("claw: cannot write {}: {e}", o.file);
        exit(2);
    }
    exit(0);
}

fn compile(o: &Options, out: &Path, mode: Mode) {
    let (module, files, paths) = compile_to_cir(o, mode);
    if o.emit_cir {
        print!("{}", cir::dump(&module));
    }
    emit(o, &module, &files, &paths, out);
}

fn emit(o: &Options, module: &cir::Module, files: &[String], paths: &[String], out: &Path) {
    if let Some(libc) = o.linux {
        match codegen::generate_linux(module, files, o.alloc_stats) {
            Ok(obj) => write_executable(out, &elf::write_elf(&obj, libc, if o.debug { Some(paths) } else { None })),
            Err(e) => {
                eprintln!("{}:1:1 {e}", o.file);
                exit(1);
            }
        }
        return;
    }
    let ident = out.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "a.out".into());
    let debug_info: Box<dyn Fn(u64, u64) -> Vec<(&'static str, Vec<u8>)>>;
    let bytes = if o.x86_64 {
        match codegen_x64::generate(module, files, o.alloc_stats) {
            Ok(obj) => {
                let bytes = macho::write_macho_x64(&obj, &ident);
                let files = paths.to_vec();
                debug_info = Box::new(move |addr, size| dwarf::sections_of(&obj.symbols, &obj.lines, obj.code.len(), 1, addr, size, &files));
                bytes
            }
            Err(e) => {
                eprintln!("{}:1:1 {e}", o.file);
                exit(1);
            }
        }
    } else {
        match codegen::generate(module, files, o.alloc_stats) {
            Ok(obj) => {
                let bytes = macho::write_macho(&obj, &ident);
                let files = paths.to_vec();
                debug_info = Box::new(move |addr, size| dwarf::sections(&obj, addr, size, &files));
                bytes
            }
            Err(e) => {
                eprintln!("{}:1:1 {e}", o.file);
                exit(1);
            }
        }
    };
    write_executable(out, &bytes);
    if o.debug {
        if let Err(e) = dwarf::write_dsym(out, &bytes, o.x86_64, &debug_info) {
            eprintln!("claw: {e}");
            exit(2);
        }
    }
}

/// Writes through a temp file and renames: replacing a binary that already
/// ran in place can get the new one killed by the kernel's signature cache.
fn write_executable(out: &Path, bytes: &[u8]) {
    use std::os::unix::fs::PermissionsExt;
    let tmp = out.with_extension(format!("tmp{}", std::process::id()));
    let result = std::fs::write(&tmp, bytes)
        .and_then(|_| std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755)))
        .and_then(|_| std::fs::rename(&tmp, out));
    if let Err(e) = result {
        let _ = std::fs::remove_file(&tmp);
        eprintln!("claw: cannot write {}: {e}", out.display());
        exit(2);
    }
}

fn default_out(file: &str) -> PathBuf {
    let p = Path::new(file);
    let stem = p.file_stem().map(|s| s.to_os_string()).unwrap_or_else(|| "a.out".into());
    PathBuf::from(stem)
}

fn run(o: &Options) -> ! {
    if o.linux.is_some() {
        die("`claw run` starts the program here, and this machine is not Linux; use `claw build --target aarch64-linux`");
    }
    let dir = std::env::temp_dir().join(format!("claw-run-{}", std::process::id()));
    if let Err(e) = std::fs::create_dir_all(&dir) {
        eprintln!("claw: cannot create {}: {e}", dir.display());
        exit(2);
    }
    let exe = dir.join(default_out(&o.file));
    compile(o, &exe, Mode::Run);
    let status = Command::new(&exe).status();
    let _ = std::fs::remove_dir_all(&dir);
    match status {
        Ok(s) => {
            use std::io::IsTerminal;
            use std::os::unix::process::ExitStatusExt;
            if s.code() == Some(101) && std::io::stderr().is_terminal() {
                eprintln!("claw: `claw why {}` finds the cause", o.file);
            }
            exit(s.code().unwrap_or_else(|| 128 + s.signal().unwrap_or(0)));
        }
        Err(e) => {
            eprintln!("claw: cannot run the program: {e}");
            exit(2);
        }
    }
}

/// Longest a single test may run.
const TEST_LIMIT: std::time::Duration = std::time::Duration::from_secs(10);

/// Each test is its own process, so a trap in one cannot take the others down.
fn run_one_test(exe: &Path, dir: &Path) -> Result<(), String> {
    use std::os::unix::process::ExitStatusExt;
    use std::process::Stdio;
    let (out_path, err_path) = (dir.join("out"), dir.join("err"));
    let (out, err) = match (std::fs::File::create(&out_path), std::fs::File::create(&err_path)) {
        (Ok(a), Ok(b)) => (a, b),
        _ => return Err("cannot capture the test's output".to_string()),
    };
    let mut child = Command::new(exe).stdout(Stdio::from(out)).stderr(Stdio::from(err)).spawn().map_err(|e| format!("cannot start: {e}"))?;
    let start = std::time::Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(s)) => break s,
            Ok(None) if start.elapsed() > TEST_LIMIT => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("still running after {} seconds", TEST_LIMIT.as_secs()));
            }
            Ok(None) => std::thread::sleep(std::time::Duration::from_micros(200)),
            Err(e) => return Err(format!("cannot wait for it: {e}")),
        }
    };
    if status.success() {
        return Ok(());
    }
    let mut text = std::fs::read_to_string(&err_path).unwrap_or_default();
    if text.is_empty() {
        text = format!("stopped with exit code {}\n", status.code().unwrap_or_else(|| 128 + status.signal().unwrap_or(0)));
    }
    let printed = std::fs::read_to_string(&out_path).unwrap_or_default();
    if !printed.is_empty() {
        text.push_str("printed:\n");
        text.push_str(&printed);
    }
    Err(text)
}

/// Limits for replaying a whole program in the interpreter.
fn replay_config(trace: bool, stdout: bool) -> interp::Config {
    interp::Config { steps: 3_000_000_000, memory: 1 << 34, depth: 1_000_000, io: true, stdout, trace, cover: false }
}

/// Explains a `test` block that failed, by running it again in the interpreter.
fn explain_block(f: &Front, m: &cir::Module, src: &why::Sources, id: cir::FuncId) -> Vec<String> {
    let files = &f.loaded.base_names;
    let mut mc = interp::Machine::new(m, files, replay_config(true, false));
    match mc.call(id, &[]) {
        interp::Stop::Fail(x) => why::report(src, &mc, &x, None),
        _ => Vec::new(),
    }
}

fn test_all(o: &Options) -> ! {
    let f = front(o, true, true);
    let files = f.loaded.base_names.clone();
    let opts = lower::Opts { check: true, test: true, fast: false, interp: false, debug: false, consts: &f.consts };
    let mut module = lower::lower(&f.loaded.units, &f.checked, &files, opts);
    let interpreted = for_interp(&f, true);
    let src = why::Sources::new(&f.loaded.units, &f.loaded.files, &files, &f.checked.types);
    let dir = std::env::temp_dir().join(format!("claw-test-{}", std::process::id()));
    if let Err(e) = std::fs::create_dir_all(&dir) {
        eprintln!("claw: cannot create {}: {e}", dir.display());
        exit(2);
    }
    let tests = module.tests.clone();
    let mut failed = 0;
    for (i, (name, id)) in tests.iter().enumerate() {
        module.main = *id;
        let exe = dir.join(format!("t{i}"));
        emit(o, &module, &files, &[], &exe);
        if let Err(why) = run_one_test(&exe, &dir) {
            failed += 1;
            println!("FAIL {name}");
            for line in why.lines() {
                println!("  {line}");
            }
            if !o.blocks {
                for line in explain_block(&f, &interpreted, &src, interpreted.tests[i].1) {
                    println!("  {line}");
                }
            }
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
    let auto = if o.blocks { None } else { Some(fuzz::run(&interpreted, &files, &src, &f.checked.types, &f.effects)) };
    let ran_auto = auto.as_ref().is_some_and(|a| a.functions > 0 || a.skipped > 0);
    if tests.is_empty() && !ran_auto {
        eprintln!("claw: nothing to test in {}: no `test` blocks and no functions without `+io`", o.file);
        exit(1);
    }
    if let Some(a) = &auto {
        for line in &a.lines {
            println!("{line}");
        }
    }
    if !tests.is_empty() {
        println!("{} passed, {failed} failed", tests.len() - failed);
    }
    if let Some(a) = auto.filter(|_| ran_auto) {
        let skipped = if a.skipped > 0 { format!(", {} skipped", a.skipped) } else { String::new() };
        println!("auto: {} functions, {} inputs, {} failed{skipped}", a.functions, a.inputs, a.failed);
        failed += a.failed;
    }
    exit((failed > 0) as i32);
}

/// `claw why`: the program runs again, this time remembering where every value came from.
fn why_file(o: &Options) -> ! {
    let f = front(o, false, o.test_name.is_some());
    let m = for_interp(&f, o.test_name.is_some());
    let files = &f.loaded.base_names;
    let start = match &o.test_name {
        Some(name) => match m.tests.iter().find(|(n, _)| n == name) {
            Some((_, id)) => *id,
            None => {
                eprintln!("claw: no `test` named \"{name}\" in {}", o.file);
                exit(2);
            }
        },
        None => m.main,
    };
    let mut mc = interp::Machine::new(&m, files, replay_config(true, false));
    let failure = match mc.call(start, &[]) {
        interp::Stop::Done(_) => mc.check_leaks(),
        interp::Stop::Exit(0) => None,
        interp::Stop::Exit(code) => {
            println!("the program stopped itself with exit code {code}");
            exit(1);
        }
        interp::Stop::Fail(x) => Some(*x),
        interp::Stop::Limit(what) => {
            eprintln!("claw: cannot replay: the program uses too much {what} for the interpreter");
            exit(2);
        }
        interp::Stop::Unsupported(sym) => {
            eprintln!("claw: cannot replay: the interpreter does not have the C function `{sym}`");
            exit(2);
        }
        interp::Stop::NeedConst(_) => unreachable!("constants were computed in the front end"),
    };
    let Some(x) = failure else {
        println!("no failure: the program ran to the end");
        exit(0);
    };
    let src = why::Sources::new(&f.loaded.units, &f.loaded.files, files, &f.checked.types);
    println!("FAIL {}", x.msg.lines().next().unwrap_or("").trim_start_matches("trap: "));
    for line in why::report(&src, &mc, &x, None) {
        println!("{line}");
    }
    exit(1);
}

/// `claw __interp`: runs `main` in the interpreter; the test suite compares it with real code.
fn interpret(o: &Options) -> ! {
    use std::io::Write;
    let f = front(o, false, false);
    let m = for_interp(&f, false);
    let mut mc = interp::Machine::new(&m, &f.loaded.base_names, replay_config(false, true));
    let stop = mc.call(m.main, &[]);
    let _ = std::io::stderr().write_all(&mc.err);
    match stop {
        interp::Stop::Done(_) => match mc.check_leaks() {
            Some(x) => {
                eprint!("{}", x.msg);
                exit(x.exit);
            }
            None => exit(0),
        },
        interp::Stop::Exit(c) => exit(c),
        interp::Stop::Fail(x) => exit(x.exit),
        interp::Stop::Limit(what) => {
            eprintln!("claw: interpreter limit: {what}");
            exit(90);
        }
        interp::Stop::Unsupported(sym) => {
            eprintln!("claw: interpreter lacks `{sym}`");
            exit(91);
        }
        interp::Stop::NeedConst(_) => unreachable!("constants were computed in the front end"),
    }
}

/// Every error of the file, without stopping at the first bad phase.
fn diagnose(o: &Options) -> (Vec<Diag>, Vec<String>) {
    let loaded = match modules::load(&o.file, false) {
        Ok(l) => l,
        Err(x) => return x,
    };
    let mut diags = Vec::new();
    let checked = sema::check(&loaded.units, loaded.core, true, &mut diags);
    effects::check(&loaded.units, &checked, &mut diags);
    (diags, loaded.files)
}

/// `claw fix`: applies safe patches until none are left, then reports what still needs a person.
fn fix_file(o: &Options) -> ! {
    use std::collections::BTreeMap;
    let mut applied = 0;
    for _ in 0..50 {
        let (mut diags, files) = diagnose(o);
        diag::sort(&mut diags);
        let mut by_file: BTreeMap<u32, Vec<diag::Fix>> = BTreeMap::new();
        for fix in diags.iter().filter_map(|d| d.fix.clone()).filter(|f| f.safe) {
            by_file.entry(fix.span.file).or_default().push(fix);
        }
        by_file.retain(|file, _| !files[*file as usize].contains("/std/"));
        if by_file.is_empty() {
            break;
        }
        for (file, mut fixes) in by_file {
            let path = &files[file as usize];
            let mut text = match std::fs::read_to_string(path) {
                Ok(t) => t,
                Err(e) => {
                    eprintln!("claw: cannot read {path}: {e}");
                    exit(2);
                }
            };
            fixes.sort_by_key(|f| std::cmp::Reverse(f.span.start));
            let mut floor = usize::MAX;
            for f in fixes {
                let (a, b) = (f.span.start as usize, f.span.end as usize);
                if b > floor || b > text.len() {
                    continue;
                }
                text.replace_range(a..b, &f.text);
                floor = a;
                applied += 1;
            }
            if let Err(e) = std::fs::write(path, text) {
                eprintln!("claw: cannot write {path}: {e}");
                exit(2);
            }
        }
    }
    let (diags, files) = diagnose(o);
    println!("fixed {applied}");
    if diags.is_empty() {
        exit(0);
    }
    report(o, diags, &files);
}

fn main() {
    // The front end recurses on expression depth, so give it plenty of stack.
    let worker = std::thread::Builder::new().stack_size(1 << 28).spawn(real_main);
    match worker.map(|t| t.join()) {
        Ok(Ok(())) => {}
        _ => exit(70),
    }
}

/// Every token of a file, to compare with the lexer written in Claw.
fn dump_tokens(path: &str) {
    let src = std::fs::read_to_string(path).unwrap_or_default();
    let mut diags = Vec::new();
    for t in lexer::lex(&src, 0, &mut diags) {
        let (kind, text, value, big) = match &t.tok {
            lexer::Tok::Int(v) => (0, String::new(), if *v > u64::MAX as i128 { 0 } else { *v as u64 }, *v > u64::MAX as i128),
            lexer::Tok::Float(_) => (1, String::new(), 0, false),
            lexer::Tok::Char(c) => (2, String::new(), *c as u64, false),
            lexer::Tok::Str(x) => (3, x.clone(), 0, false),
            lexer::Tok::Ident(x) => (4, x.clone(), 0, false),
            lexer::Tok::Kw(x) => (5, x.to_string(), 0, false),
            lexer::Tok::Op(x) => (6, x.to_string(), 0, false),
            lexer::Tok::Newline => (7, String::new(), 0, false),
            lexer::Tok::Eof => (8, String::new(), 0, false),
        };
        println!("{kind} {text} {value} {big} {} {} {} {}", t.span.start, t.span.end, t.span.line, t.span.col);
    }
    for d in diags {
        println!("{} {}:{}", d.code, d.span.line, d.span.col);
    }
}

fn real_main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(|s| s.as_str()) {
        Some("--version") | Some("-V") => println!("claw {VERSION}"),
        Some("run") => run(&parse_options(&args[1..])),
        Some("build") => {
            let o = parse_options(&args[1..]);
            let out = o.out.as_ref().map(PathBuf::from).unwrap_or_else(|| default_out(&o.file));
            compile(&o, &out, Mode::Build);
        }
        Some("test") => test_all(&parse_options(&args[1..])),
        Some("why") => why_file(&parse_options(&args[1..])),
        Some("fix") => fix_file(&parse_options(&args[1..])),
        Some("card") => print!("{}", info::CARD),
        Some("sig") if args.get(1).is_some_and(|a| a == "std") || args.get(2).is_some_and(|a| a == "std") => print!("{}", info::sig_std()),
        Some("sig") => {
            let o = parse_options(&args[1..]);
            let f = front(&o, false, false);
            print!("{}", info::sig(&f.loaded.units, &f.effects, o.public));
        }
        Some("ctx") => {
            let o = options(&args[1..], true);
            let Some(name) = o.name.clone() else { die("`claw ctx` needs a function name") };
            let f = front(&o, false, false);
            match info::ctx(&f.loaded.units, &f.checked, &f.effects, &f.loaded.files, &f.loaded.base_names, &name) {
                Ok(text) => print!("{text}"),
                Err(msg) => {
                    eprintln!("claw: {msg}");
                    exit(2);
                }
            }
        }
        Some("audit") => {
            let o = parse_options(&args[1..]);
            let f = front(&o, false, false);
            print!("{}", info::audit(&f.loaded.units, &f.checked, &f.loaded.base_names));
        }
        Some("__tokens") => dump_tokens(&args[1]),
        Some("__ast") => dump::run(&args[1..]),
        Some("__sema") => dump::sema_dump(&args[1..]),
        Some("__cir") => dump::cir_dump(&args[1..]),
        Some("__interp") => interpret(&parse_options(&args[1..])),
        Some("fmt") => format_file(&parse_options(&args[1..])),
        Some("check") => {
            let o = parse_options(&args[1..]);
            compile_to_cir(&o, Mode::Build);
        }
        Some("lsp") => lsp::serve(),
        Some("__encoder-golden-x64") => {
            for (bytes, text) in x64::golden() {
                let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
                println!("{hex}\t{text}");
            }
        }
        Some("__encoder-golden") => {
            for (word, text) in arm64::golden() {
                println!("{word:08x}\t{text}");
            }
        }
        Some(c) if !c.starts_with('-') => die(&format!("unknown command `{c}`")),
        _ => die("expected a command"),
    }
}
