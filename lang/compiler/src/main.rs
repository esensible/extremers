//! langc: check or build a program against its interface.
//!
//!   langc check PROGRAM.expr --interface IFACE.json [--samples RUNS.json] [--limit BYTES] [--code-in-ram] [--json]
//!   langc build PROGRAM.expr --interface IFACE.json -o OUT.bin [--limit BYTES] [--code-in-ram]
//!   langc dump PROGRAM.expr --interface IFACE.json
//!
//! `check` reports diagnostics and the program's memory, then evaluates any
//! sample runs on the VM: `{"runs": [{"inputs": {"name": value, …}, "chain"?: {"name": "output.path"}}, …]}`
//! (spec §5.3). `dump` prints the emitted code, one instruction per line.

use compiler::samples::run_samples;
use compiler::{Options, compile};
use serde_json::{Value as Json, json};
use std::process::ExitCode;

fn main() -> ExitCode {
    match real_main() {
        Ok(code) => code,
        Err(e) => {
            eprintln!("langc: {e}");
            ExitCode::from(2)
        }
    }
}

fn real_main() -> Result<ExitCode, String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cmd = args.first().ok_or(USAGE)?.as_str();
    let program = args.get(1).ok_or(USAGE)?;
    let flag = |name: &str| args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned();
    let iface_path = flag("--interface").ok_or("--interface is required")?;
    let as_json = args.iter().any(|a| a == "--json");
    let opts = Options {
        limit_bytes: flag("--limit").map(|s| s.parse().map_err(|_| "--limit takes a number of bytes")).transpose()?,
        code_in_ram: args.iter().any(|a| a == "--code-in-ram"),
    };
    let src = std::fs::read_to_string(program).map_err(|e| format!("{program}: {e}"))?;
    let iface: Json = read_json(&iface_path)?;

    let compiled = match compile(&src, &iface, &opts) {
        Ok(c) => c,
        Err(diags) => {
            if as_json {
                let out: Vec<Json> = diags.iter().map(|d| d.to_json(&src)).collect();
                println!("{}", json!({"ok": false, "diagnostics": out}));
            } else {
                for d in &diags {
                    eprintln!("{}", d.render(program, &src));
                }
            }
            return Ok(ExitCode::from(1));
        }
    };

    match cmd {
        "dump" => {
            print!("{}", compiler::disassemble(&compiled.bytes)?);
            Ok(ExitCode::SUCCESS)
        }
        "build" => {
            let out = flag("-o").ok_or("build needs -o OUT.bin")?;
            std::fs::write(&out, &compiled.bytes).map_err(|e| format!("{out}: {e}"))?;
            eprintln!("{out}: {} bytes; needs {} bytes of RAM", compiled.bytes.len(), compiled.memory.total_bytes);
            Ok(ExitCode::SUCCESS)
        }
        "check" => {
            let mut runs_out = Vec::new();
            let mut failed = false;
            if let Some(path) = flag("--samples") {
                let samples = read_json(&path)?;
                // langc is generic: it has no host builtins, so a program
                // that calls any can't be sample-run here (the host's own
                // tooling runs those).
                let report = run_samples(&compiled.bytes, &iface, &samples, &[]);
                if let Some(e) = report.get("invalid") {
                    return Err(e.as_str().unwrap_or("invalid samples").to_string());
                }
                for r in report["runs"].as_array().into_iter().flatten() {
                    let i = &r["run"];
                    if let Some(e) = r.get("invalid") {
                        return Err(format!("run {i}: {}", e.as_str().unwrap_or("invalid")));
                    }
                    if r["ok"] == true {
                        let stack_high = r["stack_high"].as_u64().unwrap_or(0);
                        if stack_high > compiled.memory.stack_slots as u64 {
                            return Err(format!("run {i}: the VM used {stack_high} stack slots, more than the {} the compiler worked out (a compiler bug)", compiled.memory.stack_slots));
                        }
                        runs_out.push(json!({"run": i, "ok": r["tagged"], "stack_high": stack_high, "steps": r["steps"]}));
                    } else {
                        failed = true;
                        let at = r["span"]["start"].as_u64().map(|s| {
                            let (l, c) = compiler::diag::line_col(&src, s as u32);
                            format!("{program}:{l}:{c}")
                        });
                        runs_out.push(json!({"run": i, "error": r["error"], "at": at, "help": r["help"]}));
                    }
                }
            }
            if as_json {
                println!("{}", json!({"ok": !failed, "memory": compiled.memory.to_json(), "runs": runs_out}));
            } else {
                let m = compiled.memory;
                println!(
                    "ok: needs {} bytes (stack {} × {b}, locals {} × {b}, output {} × {b}); program {} bytes",
                    m.total_bytes,
                    m.stack_slots,
                    m.locals,
                    m.output_slots,
                    m.program_bytes,
                    b = m.slot_bytes
                );
                for r in &runs_out {
                    match r.get("error") {
                        Some(code) => println!(
                            "{}: run {} failed: {}\n  help: {}",
                            r["at"].as_str().unwrap_or(program),
                            r["run"],
                            code.as_str().unwrap_or("?"),
                            r["help"].as_str().unwrap_or("")
                        ),
                        None => println!("{r}"),
                    }
                }
            }
            Ok(if failed { ExitCode::from(1) } else { ExitCode::SUCCESS })
        }
        other => Err(format!("unknown command `{other}`\n{USAGE}")),
    }
}

const USAGE: &str = "usage: langc check|build|dump PROGRAM.expr --interface IFACE.json [--samples RUNS.json] [-o OUT.bin] [--limit BYTES] [--code-in-ram] [--json]";

fn read_json(path: &str) -> Result<Json, String> {
    let s = std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))?;
    serde_json::from_str(&s).map_err(|e| format!("{path}: {e}"))
}
