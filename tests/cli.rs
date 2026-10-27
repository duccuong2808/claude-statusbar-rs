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

/// `csr hook` (a PostToolUse hook) brings the git cache up to date before the next render, and
/// stays silent and successful whatever it is fed.
#[test]
fn hook_refreshes_the_git_cache_for_the_session_cwd() {
    let d = tempfile::tempdir().unwrap();
    let home = d.path().join("home");
    let repo = d.path().join("repo");
    std::fs::create_dir_all(repo.join("sub")).unwrap();
    std::fs::create_dir_all(&home).unwrap();
    std::fs::write(repo.join("f.txt"), "1\n").unwrap();
    let git = |args: &[&str]| {
        assert!(Command::new("git")
            .args(["-c", "user.email=t@t", "-c", "user.name=t"])
            .args(args)
            .current_dir(&repo)
            .output()
            .unwrap()
            .status
            .success());
    };
    git(&["init", "-q"]);
    git(&["add", "."]);
    git(&["commit", "-qm", "init"]);
    let top = repo.canonicalize().unwrap();
    let cache_dir = csr::cachedir::resolve_with(&home, home.to_str());
    let entry = csr::gitcache::cache_path(&cache_dir, &top.to_string_lossy());

    let hook = |stdin: &str| {
        let mut child = Command::new(env!("CARGO_BIN_EXE_csr"))
            .arg("hook")
            .env_clear()
            .env("HOME", &home)
            .env("XDG_RUNTIME_DIR", &home)
            .env("PATH", "/usr/bin:/bin:/usr/local/bin:/opt/homebrew/bin")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(stdin.as_bytes())
            .unwrap();
        let out = child.wait_with_output().unwrap();
        assert!(out.status.success(), "{out:?}");
        assert!(out.stdout.is_empty() && out.stderr.is_empty(), "{out:?}");
    };
    // cwd in a subdirectory, as after `cd sub` in the session
    let payload = serde_json::json!({
        "hook_event_name": "PostToolUse",
        "tool_name": "Edit",
        "cwd": repo.join("sub"),
    })
    .to_string();

    hook(&payload);
    let e = csr::gitcache::read_entry(&entry).expect("hook wrote the entry");
    assert_eq!(e.dirty, Some(false));

    std::fs::write(repo.join("f.txt"), "2\n").unwrap();
    hook(&payload);
    assert_eq!(csr::gitcache::read_entry(&entry).unwrap().dirty, Some(true));

    // Outside a repo, without cwd, or with junk: nothing to do, still exit 0 and say nothing.
    let elsewhere = serde_json::json!({ "cwd": d.path() }).to_string();
    for junk in ["", "{not json", "{}", elsewhere.as_str()] {
        hook(junk);
    }
}

/// `csr themes` previews every built-in theme; `csr theme [name]` shows or sets the config's.
#[test]
fn theme_commands_list_show_and_set_the_configured_theme() {
    let d = tempfile::tempdir().unwrap();
    let cfg = d.path().join("cfg.json");
    std::fs::write(&cfg, r#"{"show_cost":true}"#).unwrap();
    let csr = |args: &[&str]| {
        let out = Command::new(env!("CARGO_BIN_EXE_csr"))
            .args(args)
            .env_clear()
            .env("HOME", d.path())
            .env("CSR_CONFIG", &cfg)
            .stdin(Stdio::null())
            .output()
            .unwrap();
        (
            out.status.code(),
            String::from_utf8(out.stdout).unwrap(),
            String::from_utf8(out.stderr).unwrap(),
        )
    };

    assert_eq!(
        csr(&["theme"]),
        (Some(0), "graphite\n".into(), String::new())
    );
    assert_eq!(csr(&["theme", "Nord"]).0, Some(0));
    assert_eq!(csr(&["theme"]).1, "nord\n");
    let v: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&cfg).unwrap()).unwrap();
    assert_eq!(
        (v["theme"].as_str(), v["show_cost"].as_bool()),
        (Some("nord"), Some(true))
    );

    let (code, out, err) = csr(&["theme", "bogus"]);
    assert_eq!((code, out.as_str()), (Some(2), ""));
    assert!(
        err.contains("unknown theme") && err.contains("dracula"),
        "{err}"
    );
    assert_eq!(
        csr(&["theme"]).1,
        "nord\n",
        "a failed set leaves the config alone"
    );

    let (code, out, _) = csr(&["themes"]);
    assert_eq!(code, Some(0));
    for t in csr::theme::THEMES {
        assert!(out.contains(t.name), "{} missing from:\n{out}", t.name);
    }
    assert!(out.contains("nord (current)"), "{out}");
    assert!(out.contains("\x1b[38;2;"), "previews are coloured");
}
