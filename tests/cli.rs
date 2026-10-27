//! The `csr` binary itself: no stdin / bad JSON fallback line, argument handling.

use std::io::Write;
use std::process::{Command, Stdio};

fn run(stdin: &str, columns: Option<&str>) -> String {
    let home = tempfile::tempdir().unwrap();
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_csr"));
    cmd.env_clear()
        .env("HOME", home.path())
        .env("PATH", "/usr/bin:/bin");
    if let Some(c) = columns {
        cmd.env("COLUMNS", c);
    }
    let mut child = cmd
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(stdin.as_bytes())
        .unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(out.status.success());
    String::from_utf8(out.stdout).unwrap()
}

#[test]
fn no_stdin_and_bad_json_print_the_python_warning() {
    // core.py: no stdin → model = display_name ("Unknown") replaced by model_id ("unknown")
    let want = "⚠ Run inside Claude Code statusLine for rate-limit data | unknown\n";
    assert_eq!(run("", None), want);
    assert_eq!(run("{not json", None), want);
    assert_eq!(run("", Some("20")), "⚠ Run inside Claude…\n");
}

#[test]
fn unknown_argument_and_bare_git_refresh_fail_with_usage() {
    for args in [&["--bogus"][..], &["git-refresh"][..]] {
        let out = Command::new(env!("CARGO_BIN_EXE_csr"))
            .args(args)
            .stdin(Stdio::null())
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(2), "args {args:?}");
        assert!(out.stdout.is_empty(), "args {args:?}");
        let err = String::from_utf8(out.stderr).unwrap();
        assert!(err.contains("usage: csr"), "args {args:?}: {err}");
    }
}
