//! Runs the `const`s that need a function call, while compiling.

use crate::cir::FuncId;
use crate::diag::Diag;
use crate::interp::{Config, Machine, Stop};
use crate::lower::{self, ConstData, Opts};
use crate::modules::Unit;
use crate::sema::Checked;
use crate::types::STR;

/// What a compile may spend on one constant.
const STEPS: u64 = 500_000_000;

pub fn evaluate(units: &[Unit], c: &Checked, files: &[String], diags: &mut Vec<Diag>) -> Vec<ConstData> {
    let n = c.computed.len();
    if n == 0 {
        return Vec::new();
    }
    let module = lower::lower(units, c, files, Opts { check: true, test: false, fast: false, interp: true, consts: &[] });
    let cfg = Config { steps: STEPS, memory: 1 << 30, depth: 100_000, io: false, stdout: false, trace: false, cover: false };
    let mut mc = Machine::new(&module, files, cfg);
    mc.consts = vec![None; n];
    let mut failed = vec![false; n];
    for start in 0..n {
        let mut stack = vec![start];
        while let Some(&id) = stack.last() {
            if mc.consts[id].is_some() || failed[id] {
                stack.pop();
                continue;
            }
            if let Some(dep) = run_one(&mut mc, c, module.const_funcs[id], id, diags, &mut failed) {
                if stack.contains(&dep) {
                    diags.push(Diag::new("E0175", c.computed[id].span, format!("`{}` needs `{}`, which needs `{}` back", c.computed[id].name, c.computed[dep].name, c.computed[id].name)));
                    failed[id] = true;
                } else if failed[dep] {
                    failed[id] = true;
                } else {
                    stack.push(dep);
                }
            }
        }
    }
    mc.consts.into_iter().map(|v| v.unwrap_or(ConstData::Scalar(0))).collect()
}

/// Returns the constant this one is waiting for, if any.
fn run_one(mc: &mut Machine, c: &Checked, func: FuncId, id: usize, diags: &mut Vec<Diag>, failed: &mut [bool]) -> Option<usize> {
    let x = &c.computed[id];
    mc.reset();
    let aggregate = c.types.is_aggregate(x.ty);
    let size = c.types.size(x.ty);
    let slot = if aggregate { mc.input_block(size.max(1)) } else { 0 };
    let stop = if aggregate { mc.call(func, &[slot]) } else { mc.call(func, &[]) };
    let why = match stop {
        Stop::Done(v) => {
            mc.consts[id] = Some(if !aggregate {
                ConstData::Scalar(v)
            } else if x.ty == STR {
                let header = mc.block_bytes(slot, 24).expect("the result slot is 24 bytes");
                let ptr = u64::from_le_bytes(header[0..8].try_into().expect("8 bytes"));
                let len = u64::from_le_bytes(header[8..16].try_into().expect("8 bytes"));
                ConstData::Str(if len == 0 { Vec::new() } else { mc.block_bytes(ptr, len as usize).expect("a valid str").to_vec() })
            } else {
                ConstData::Bytes(mc.block_bytes(slot, size as usize).expect("the result slot").to_vec())
            });
            return None;
        }
        Stop::NeedConst(dep) => return Some(dep),
        Stop::Fail(f) => f.msg.lines().next().unwrap_or("").trim_start_matches("trap: ").to_string(),
        Stop::Limit(what) => format!("it ran out of {what} while compiling"),
        Stop::Unsupported(sym) => format!("it needs the C function `{sym}`, which cannot run while compiling"),
        Stop::Exit(_) => "it stopped the compiler's interpreter".to_string(),
    };
    diags.push(Diag::new("E0174", x.span, format!("cannot compute `{}`: {why}", x.name)));
    failed[id] = true;
    None
}
