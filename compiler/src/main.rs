//! `claw`: the Claw compiler (stage 0, in Rust, no dependencies).

mod arm64;
mod ast;
mod cir;
mod codegen;
mod diag;
mod lexer;
mod lower;
mod macho;
mod parser;
mod sema;
mod sha256;

use diag::Diag;
use std::path::{Path, PathBuf};
use std::process::{exit, Command};

const VERSION: &str = env!("CARGO_PKG_VERSION");

const USAGE: &str = "claw run <file.claw>            compile with all checks and run
claw build <file.claw> [-o out] compile to an executable
claw check <file.claw>          report errors only
claw --version
flags: --json (errors as JSON), --emit-cir (print the IR)
";

struct Options {
    file: String,
    out: Option<String>,
    json: bool,
    emit_cir: bool,
}

fn parse_options(args: &[String]) -> Options {
    let mut o = Options { file: String::new(), out: None, json: false, emit_cir: false };
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--json" => o.json = true,
            "--emit-cir" => o.emit_cir = true,
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

fn report(o: &Options, mut diags: Vec<Diag>) -> ! {
    diag::sort(&mut diags);
    if o.json {
        print!("{}", diag::render_json(&o.file, &diags));
    } else {
        eprint!("{}", diag::render_text(&o.file, &diags));
    }
    exit(1);
}

/// Runs the front end; never returns on a source error.
fn compile_to_cir(o: &Options) -> cir::Module {
    let src = match std::fs::read_to_string(&o.file) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("claw: cannot read {}: {e}", o.file);
            exit(2);
        }
    };
    let mut diags = Vec::new();
    let toks = lexer::lex(&src, &mut diags);
    if !diags.is_empty() {
        // A broken token only produces noise in the parser.
        report(o, diags);
    }
    let program = parser::Parser::new(toks, &mut diags).parse_program();
    if !diags.is_empty() {
        report(o, diags);
    }
    sema::check(&program, &mut diags);
    if !diags.is_empty() {
        report(o, diags);
    }
    lower::lower(&program)
}

fn compile(o: &Options, out: &Path) {
    let module = compile_to_cir(o);
    if o.emit_cir {
        print!("{}", cir::dump(&module));
    }
    let base = Path::new(&o.file).file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let object = match codegen::generate(&module, &base) {
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
    compile(o, &exe);
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
            compile(&o, &out);
        }
        Some("check") => {
            let o = parse_options(&args[1..]);
            compile_to_cir(&o);
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
