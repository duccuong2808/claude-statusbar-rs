//! Runs the built `csr` binary on tests/fixtures/stdin-orbit.json at the instant recorded in
//! golden-orbit.now and compares to what Python claude-statusbar 3.52.2 rendered. The only
//! masked part is the `→NN%` projection chip (csr uses the simple average-pace formula).
//!
//! The golden was captured once (2026-10-27) from Python claude-statusbar 3.52.2 against the
//! orbit repo. This test replays it against a synthetic repo in a temp dir with the same git
//! facts (branch `development`, origin git@github.com:exampleco/orbit.git, exactly one live
//! linked worktree, dirty tree), so it no longer depends on any checkout on this machine.

use std::fs;
use std::io::Write;
use std::process::{Command, Stdio};

fn mask_projection(s: &str) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(i) = rest.find('→') {
        out.push_str(&rest[..i]);
        out.push_str("→XX");
        let after = &rest[i + '→'.len_utf8()..];
        let b = after.as_bytes();
        let mut j = 0;
        while j < b.len() && (b[j].is_ascii_digit() || b[j] == b'-') {
            j += 1;
        }
        if j < b.len() && b[j] == b'%' {
            j += 1;
        }
        if after[j..].starts_with('·') {
            j += '·'.len_utf8();
            while j < b.len() && b[j] != b' ' && b[j] != 0x1b {
                j += 1;
            }
        }
        rest = &after[j..];
    }
    out.push_str(rest);
    out
}

#[test]
fn renders_like_python_for_the_orbit_fixture() {
    let root = env!("CARGO_MANIFEST_DIR");
    let fixture = fs::read_to_string(format!("{root}/tests/fixtures/stdin-orbit.json")).unwrap();
    let golden = fs::read_to_string(format!("{root}/tests/fixtures/golden-orbit.raw")).unwrap();
    let now = fs::read_to_string(format!("{root}/tests/fixtures/golden-orbit.now"))
        .unwrap()
        .trim()
        .to_string();

    // Synthetic repo with the facts the golden encodes. Line 2 of the golden holds only the
    // branch, the origin URL, the project name (stdin `workspace.repo.name`), the lines text and
    // `worktree (🌳1)`; none of them depend on the path.
    let tmp = tempfile::tempdir().unwrap();
    let repo_dir = tmp.path().join("repo");
    let wt_dir = tmp.path().join("wt-trial");
    fs::create_dir_all(repo_dir.join(".git/worktrees/wt-trial")).unwrap();
    fs::create_dir_all(&wt_dir).unwrap();
    // identity.rs canonicalizes the start path and the dirty cache key is the canonical toplevel.
    let repo = repo_dir.canonicalize().unwrap();
    let repo_str = repo.to_str().unwrap();
    fs::write(repo.join(".git/HEAD"), "ref: refs/heads/development\n").unwrap();
    fs::write(
        repo.join(".git/config"),
        "[core]\n\trepositoryformatversion = 0\n[remote \"origin\"]\n\turl = git@github.com:exampleco/orbit.git\n\tfetch = +refs/heads/*:refs/remotes/origin/*\n",
    )
    .unwrap();
    // One live linked worktree: identity.rs only checks that the gitdir target exists.
    fs::write(
        repo.join(".git/worktrees/wt-trial/gitdir"),
        format!("{}/.git\n", wt_dir.display()),
    )
    .unwrap();
    fs::write(
        wt_dir.join(".git"),
        format!("gitdir: {repo_str}/.git/worktrees/wt-trial\n"),
    )
    .unwrap();

    // Point the captured stdin at the synthetic repo: every occurrence of the captured checkout
    // path (`cwd`, `workspace.current_dir`, `workspace.project_dir`) becomes the temp repo path.
    // The transcript_path uses the dashed form and stays as is.
    let captured: serde_json::Value = serde_json::from_str(&fixture).unwrap();
    let captured_dir = captured["cwd"].as_str().unwrap().to_string();
    let fixture = fixture.replace(&captured_dir, repo_str);
    assert!(
        !fixture.contains(&format!("\"{captured_dir}\"")),
        "captured checkout path still in stdin"
    );

    // Isolated HOME: only the config below, and a pre-seeded fresh dirty cache so no refresh is spawned
    // and the ● renders (Python's golden had its cache filled by a prior run).
    let home = tempfile::tempdir().unwrap();
    // The effort level is csr's own addition; Python 3.52.2 never draws it.
    fs::create_dir_all(home.path().join(".claude")).unwrap();
    fs::write(
        home.path().join(".claude/claude-statusbar.json"),
        r#"{"show_effort": false}"#,
    )
    .unwrap();
    // Seed where the child will look: <home>/csr on Linux (XDG_RUNTIME_DIR = home),
    // <home>/.cache/csr on macOS.
    let cache_dir = csr::cachedir::resolve_with(home.path(), home.path().to_str());
    let cache = csr::gitcache::cache_path(&cache_dir, repo_str);
    fs::create_dir_all(cache.parent().unwrap()).unwrap();
    fs::write(
        &cache,
        format!(r#"{{"toplevel":"{repo_str}","dirty":true,"ahead":0,"behind":3,"ts":{now}}}"#),
    )
    .unwrap();

    let mut child = Command::new(env!("CARGO_BIN_EXE_csr"))
        .env_clear()
        .env("HOME", home.path())
        .env("XDG_RUNTIME_DIR", home.path())
        .env("PATH", "/usr/bin:/bin")
        .env("CSR_NOW", &now)
        .env("COLUMNS", "182")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(fixture.as_bytes())
        .unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(out.status.success());
    let got = String::from_utf8(out.stdout).unwrap();

    let got_lines: Vec<String> = got
        .trim_end_matches('\n')
        .lines()
        .map(mask_projection)
        .collect();
    let want_lines: Vec<String> = golden
        .trim_end_matches('\n')
        .lines()
        .map(mask_projection)
        .collect();
    assert_eq!(got_lines.len(), 2, "got:\n{got:?}");
    for (i, (g, w)) in got_lines.iter().zip(want_lines.iter()).enumerate() {
        assert_eq!(g, w, "line {} differs\n got: {:?}\nwant: {:?}", i + 1, g, w);
    }
    assert!(
        !cache.with_extension("inflight").exists(),
        "fresh cache must not spawn a refresh"
    );
}
