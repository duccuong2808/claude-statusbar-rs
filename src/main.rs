//! `csr` entry point: renders the status line from the JSON Claude Code sends on stdin.

use std::io::{Read, Write};
use std::path::PathBuf;
use std::process;

use csr::config::{config_path, Config};
use csr::payload::Payload;
use csr::render::{render, Env};
use csr::width::clip_lines;

/// `$HOME`, else the passwd entry (`std::env::home_dir`); `/` only when neither exists.
fn home() -> PathBuf {
    std::env::home_dir().unwrap_or_else(|| PathBuf::from("/"))
}

/// `CSR_NOW=<unix seconds>` freezes the clock for the golden tests (see README).
fn now() -> f64 {
    if let Some(v) = std::env::var("CSR_NOW")
        .ok()
        .and_then(|s| s.trim().parse::<f64>().ok())
    {
        return v;
    }
    csr::clock::unix_now()
}

fn columns(p: &Payload) -> Option<usize> {
    p.cs_env
        .as_ref()
        .and_then(|m| m.get("COLUMNS").cloned())
        .or_else(|| std::env::var("COLUMNS").ok())
        .and_then(|s| s.trim().parse::<usize>().ok())
        .filter(|c| *c > 0)
}

const USAGE: &str = "usage: csr            render the status line from Claude Code's stdin JSON
       csr git-refresh <toplevel> [cache_dir]   (internal) refresh the dirty cache";

fn main() {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        None => render_stdin(),
        Some("git-refresh") => {
            let Some(toplevel) = args.get(2) else {
                eprintln!("csr: git-refresh needs <toplevel>\n{USAGE}");
                process::exit(2);
            };
            let cache_dir = args
                .get(3)
                .map_or_else(|| csr::cachedir::resolve(&home()), PathBuf::from);
            csr::gitcache::refresh(&cache_dir, toplevel);
        }
        Some("--version" | "-V") => println!("csr {}", env!("CARGO_PKG_VERSION")),
        Some("--help" | "-h") => println!("{USAGE}"),
        Some(other) => {
            eprintln!("csr: unknown argument {other:?}\n{USAGE}");
            process::exit(2);
        }
    }
}

/// The statusLine path. Always prints a line and exits 0: a failing command blanks the status
/// line, so unreadable or invalid stdin shows the Python tool's warning instead.
fn render_stdin() {
    let mut raw = String::new();
    // A read error (or non-UTF-8 input) leaves `raw` empty, which takes the warning branch below.
    let _ = std::io::stdin().read_to_string(&mut raw);
    let home = home();
    let cfg = Config::load(&config_path(&home));
    let out = match serde_json::from_str::<Payload>(&raw) {
        Ok(p) if !raw.trim().is_empty() => {
            let env = Env {
                now: now(),
                cache_dir: csr::cachedir::resolve(&home),
                columns: columns(&p),
            };
            render(&p, &cfg, &env)
        }
        // core.py no-stdin branch: model = display_name ("Unknown") → model_id ("unknown").
        _ => clip_lines(
            "⚠ Run inside Claude Code statusLine for rate-limit data | unknown",
            columns(&Payload::default()),
        ),
    };
    let mut lock = std::io::stdout().lock();
    let _ = lock.write_all(out.as_bytes());
    let _ = lock.write_all(b"\n");
    let _ = lock.flush();
}
