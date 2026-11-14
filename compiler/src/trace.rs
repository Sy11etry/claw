//! Reads a `.clawtrace` file, the flight recorder's output (see `std/trace.claw`): how a program ended and what it
//! was given from outside the language until then.
//!
//! Text with lengths, so any bytes can travel in it: a header line, then the exact number of bytes it announces,
//! then a newline.
//!
//! ```text
//! clawtrace 1
//! trap <n>            the message the runtime printed, n bytes
//! argv <count>        then count times: <n> and n bytes
//! env <name n> <value n or -1>      name, then the value unless -1
//! file <path n> <data n or -1> <errno>     path, then the data unless -1 (the read failed with errno)
//! truncated           the program read more than the recorder could keep
//! end
//! ```

use crate::interp::Replay;
use std::collections::VecDeque;

pub struct Trace {
    /// The runtime's own message: `trap: index out of bounds at f.claw:2:9`.
    pub trap: String,
    pub replay: Replay,
    pub truncated: bool,
}

struct Reader<'a> {
    b: &'a [u8],
    at: usize,
}

impl Reader<'_> {
    fn line(&mut self) -> Result<String, String> {
        let rest = self.b.get(self.at..).ok_or("the trace ends too early")?;
        let n = rest.iter().position(|c| *c == b'\n').ok_or("the trace ends too early")?;
        self.at += n + 1;
        Ok(String::from_utf8_lossy(&rest[..n]).into_owned())
    }

    /// `n` bytes and the newline after them.
    fn bytes(&mut self, n: i64) -> Result<Vec<u8>, String> {
        let n = usize::try_from(n).map_err(|_| "a negative length in the trace".to_string())?;
        let end = self.at.checked_add(n).filter(|e| *e < self.b.len()).ok_or("the trace ends too early")?;
        let out = self.b[self.at..end].to_vec();
        if self.b[end] != b'\n' {
            return Err("the trace is damaged: a block does not end where it says".to_string());
        }
        self.at = end + 1;
        Ok(out)
    }
}

fn numbers(line: &str, word: &str, count: usize) -> Result<Vec<i64>, String> {
    let rest = line.strip_prefix(word).and_then(|r| r.strip_prefix(' ')).ok_or_else(|| format!("expected `{word}` in the trace, found `{line}`"))?;
    let out: Vec<i64> = rest.split(' ').filter_map(|p| p.parse().ok()).collect();
    if out.len() != count {
        return Err(format!("the trace line `{line}` should have {count} numbers"));
    }
    Ok(out)
}

pub fn parse(bytes: &[u8]) -> Result<Trace, String> {
    let mut r = Reader { b: bytes, at: 0 };
    if r.line()? != "clawtrace 1" {
        return Err("this is not a clawtrace file (version 1)".to_string());
    }
    let n = numbers(&r.line()?, "trap", 1)?[0];
    let trap = String::from_utf8_lossy(&r.bytes(n)?).trim_end().to_string();
    let argc = numbers(&r.line()?, "argv", 1)?[0];
    let mut replay = Replay::default();
    for _ in 0..argc {
        let n = r.line()?.parse::<i64>().map_err(|_| "a bad argument length in the trace")?;
        replay.args.push(r.bytes(n)?);
    }
    let mut truncated = false;
    loop {
        let line = r.line()?;
        match line.split(' ').next().unwrap_or("") {
            "end" => break,
            "truncated" => truncated = true,
            "env" => {
                let n = numbers(&line, "env", 2)?;
                let name = r.bytes(n[0])?;
                let value = if n[1] >= 0 { Some(r.bytes(n[1])?) } else { None };
                replay.env.insert(name, value);
            }
            "file" => {
                let n = numbers(&line, "file", 3)?;
                let path = r.bytes(n[0])?;
                let got = if n[1] >= 0 { Ok(r.bytes(n[1])?) } else { Err(n[2] as i32) };
                replay.files.entry(path).or_insert_with(VecDeque::new).push_back(got);
            }
            other => return Err(format!("an unknown record `{other}` in the trace")),
        }
    }
    Ok(Trace { trap, replay, truncated })
}
