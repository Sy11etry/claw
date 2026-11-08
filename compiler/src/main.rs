//! `claw`: the Claw compiler (stage 0, in Rust, no dependencies).

mod arm64;
mod ast;
mod cir;
mod codegen;
mod diag;
mod effects;
mod fmt;
mod lexer;
mod lower;
mod macho;
mod modules;
mod parser;
mod sema;
mod sha256;
mod types;

use diag::Diag;
use std::path::{Path, PathBuf};
use std::process::{exit, Command};

const VERSION: &str = env!("CARGO_PKG_VERSION");

const USAGE: &str = "claw run <file.claw>            compile with all checks and run
claw test <file.claw>           run the file's `test` blocks with all checks
claw build <file.claw> [-o out] compile to an executable; contracts with calls are left out
claw check <file.claw>          report errors only
claw fmt <file.claw> [--check]  put a file into the one standard format
claw --version
flags: --json (errors as JSON), --emit-cir (print the IR)
";

struct Options {
    file: String,
    out: Option<String>,
    json: bool,
    emit_cir: bool,
    check_only: bool,
    alloc_stats: bool,
}

fn parse_options(args: &[String]) -> Options {
    let mut o = Options { file: String::new(), out: None, json: false, emit_cir: false, check_only: false, alloc_stats: false };
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--json" => o.json = true,
            "--emit-cir" => o.emit_cir = true,
            "--check" => o.check_only = true,
            "--alloc-stats" => o.alloc_stats = true,
            "-o" => {
                i += 1;
                match args.get(i) {
                    Some(p) => o.out = Some(p.clone()),
                    None => die("`-o` needs a path"),
                }
            }
            a if a.starts_with('-') => die(&format!("unknown flag `{a}`")),
            a if o.file.is_empty() => o.file = a.to_string(),
            a => die(&format!("unexpected argument `{a}`")),
        }
        i += 1;
    }
    if o.file.is_empty() {
        die("no input file");
    }
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

/// Runs the front end; never returns on a source error.
fn compile_to_cir(o: &Options, mode: Mode) -> (cir::Module, Vec<String>) {
    let opts = lower::Opts { check: mode != Mode::Build, test: mode == Mode::Test };
    let loaded = match modules::load(&o.file, opts.check) {
        Ok(l) => l,
        Err((diags, files)) => report(o, diags, &files),
    };
    let mut diags = Vec::new();
    let checked = sema::check(&loaded.units, loaded.core, mode == Mode::Test, &mut diags);
    let slow = effects::check(&loaded.units, &checked, &mut diags);
    if !diags.is_empty() {
        report(o, diags, &loaded.files);
    }
    if mode == Mode::Build && slow > 0 {
        eprintln!("claw: note: {slow} checks that call functions are left out of this build; `claw run` and `claw test` do them");
    }
    let module = lower::lower(&loaded.units, &checked, &loaded.base_names, opts);
    (module, loaded.base_names.clone())
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
    let (module, files) = compile_to_cir(o, mode);
    if o.emit_cir {
        print!("{}", cir::dump(&module));
    }
    emit(o, &module, &files, out);
}

fn emit(o: &Options, module: &cir::Module, files: &[String], out: &Path) {
    let object = match codegen::generate(module, files, o.alloc_stats) {
        Ok(obj) => obj,
        Err(e) => {
            eprintln!("{}:1:1 {e}", o.file);
            exit(1);
        }
    };
    let ident = out.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "a.out".into());
    let bytes = macho::write_macho(&object, &ident);
    write_executable(out, &bytes);
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
            use std::os::unix::process::ExitStatusExt;
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

fn test_all(o: &Options) -> ! {
    let (mut module, files) = compile_to_cir(o, Mode::Test);
    if module.tests.is_empty() {
        eprintln!("claw: no `test` blocks in {}", o.file);
        exit(1);
    }
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
        emit(o, &module, &files, &exe);
        if let Err(why) = run_one_test(&exe, &dir) {
            failed += 1;
            println!("FAIL {name}");
            for line in why.lines() {
                println!("  {line}");
            }
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
    println!("{} passed, {failed} failed", tests.len() - failed);
    exit((failed > 0) as i32);
}

fn main() {
    // The front end recurses on expression depth, so give it plenty of stack.
    let worker = std::thread::Builder::new().stack_size(1 << 28).spawn(real_main);
    match worker.map(|t| t.join()) {
        Ok(Ok(())) => {}
        _ => exit(70),
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
        Some("fmt") => format_file(&parse_options(&args[1..])),
        Some("check") => {
            let o = parse_options(&args[1..]);
            compile_to_cir(&o, Mode::Build);
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
