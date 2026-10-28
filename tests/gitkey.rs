//! `csr::gitkey::key` against real repos: it must change whenever `git status --porcelain
//! --branch` could answer differently, and stay put otherwise.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use csr::gitkey::key;

fn git(repo: &Path, args: &[&str]) {
    let out = Command::new("git")
        .args(["-c", "user.email=t@t", "-c", "user.name=t"])
        .args(args)
        .current_dir(repo)
        .output()
        .unwrap();
    assert!(out.status.success(), "git {args:?}: {out:?}");
}

/// A committed repo: `a.txt` and `sub/b.txt`. No `-b` on init: Ubuntu 20.04's git 2.25 lacks it.
fn repo() -> (tempfile::TempDir, PathBuf) {
    let d = tempfile::tempdir().unwrap();
    let r = d.path().join("repo");
    fs::create_dir_all(r.join("sub")).unwrap();
    fs::write(r.join("a.txt"), "a\n").unwrap();
    fs::write(r.join("sub/b.txt"), "b\n").unwrap();
    git(&r, &["init", "-q"]);
    git(&r, &["add", "."]);
    git(&r, &["commit", "-qm", "init"]);
    let r = r.canonicalize().unwrap();
    (d, r)
}

/// Linux stamps files from a coarse clock (a tick is 1–10 ms), so creating and deleting an entry
/// in the same directory within one tick leaves the directory's mtime, and the key, unchanged.
/// Real edits are never that close to a render and its `git status`; the tests wait a tick.
fn tick() {
    std::thread::sleep(std::time::Duration::from_millis(25));
}

fn k(r: &Path) -> u64 {
    key(r).expect("supported repo")
}

#[test]
fn stable_while_nothing_changes() {
    let (_d, r) = repo();
    let k0 = k(&r);
    assert_eq!(k(&r), k0);
    // reading files and running git status (no optional locks) changes nothing
    fs::read(r.join("a.txt")).unwrap();
    git(&r, &["--no-optional-locks", "status", "--porcelain"]);
    assert_eq!(k(&r), k0);
}

#[test]
fn tracked_edit_changes_it_and_each_further_edit_too() {
    let (_d, r) = repo();
    let k0 = k(&r);
    fs::write(r.join("sub/b.txt"), "b2\n").unwrap();
    let k1 = k(&r);
    assert_ne!(k1, k0);
    fs::write(r.join("sub/b.txt"), "b33\n").unwrap();
    assert_ne!(k(&r), k1);
}

#[test]
fn untracked_files_and_deletions_change_it() {
    let (_d, r) = repo();
    let k0 = k(&r);
    tick();
    fs::write(r.join("new.txt"), "x").unwrap();
    let k1 = k(&r);
    assert_ne!(k1, k0);
    tick();
    fs::remove_file(r.join("new.txt")).unwrap();
    let k2 = k(&r);
    assert_ne!(k2, k1);
    tick();
    fs::write(r.join("sub/new.txt"), "x").unwrap();
    let k3 = k(&r);
    assert_ne!(k3, k2);
    tick();
    fs::remove_file(r.join("a.txt")).unwrap();
    assert_ne!(k(&r), k3);
}

#[test]
fn commit_and_upstream_moves_change_it() {
    let (_d, r) = repo();
    fs::write(r.join("a.txt"), "a2\n").unwrap();
    git(&r, &["commit", "-qam", "two"]);
    let k0 = k(&r);
    // Fake an upstream: origin/<branch> at HEAD~1, then "fetch" it forward to HEAD.
    let branch = String::from_utf8(
        Command::new("git")
            .args(["symbolic-ref", "--short", "HEAD"])
            .current_dir(&r)
            .output()
            .unwrap()
            .stdout,
    )
    .unwrap()
    .trim()
    .to_string();
    git(
        &r,
        &[
            "update-ref",
            &format!("refs/remotes/origin/{branch}"),
            "HEAD~1",
        ],
    );
    git(
        &r,
        &["config", &format!("branch.{branch}.remote"), "origin"],
    );
    git(
        &r,
        &[
            "config",
            &format!("branch.{branch}.merge"),
            &format!("refs/heads/{branch}"),
        ],
    );
    let k1 = k(&r);
    assert_ne!(k1, k0);
    git(
        &r,
        &[
            "update-ref",
            &format!("refs/remotes/origin/{branch}"),
            "HEAD",
        ],
    );
    let k2 = k(&r);
    assert_ne!(k2, k1);
    git(&r, &["commit", "-qm", "empty", "--allow-empty"]);
    assert_ne!(k(&r), k2);
}

#[test]
fn ignore_rules_change_it() {
    let (_d, r) = repo();
    let k0 = k(&r);
    let exclude = r.join(".git/info/exclude");
    fs::create_dir_all(exclude.parent().unwrap()).unwrap();
    fs::write(&exclude, "*.log\n").unwrap();
    assert_ne!(k(&r), k0);
}

#[test]
fn linked_worktree_has_its_own_key() {
    let (d, r) = repo();
    let wt = d.path().join("wt");
    git(
        &r,
        &["worktree", "add", "-q", "-b", "side", wt.to_str().unwrap()],
    );
    let wt = wt.canonicalize().unwrap();
    let k0 = k(&wt);
    assert_eq!(k(&wt), k0);
    fs::write(wt.join("a.txt"), "w\n").unwrap();
    assert_ne!(k(&wt), k0);
}

#[test]
fn unsupported_layouts_have_no_key() {
    // no index yet
    let d = tempfile::tempdir().unwrap();
    git(d.path(), &["init", "-q"]);
    assert_eq!(key(d.path()), None);
    // not a repo at all
    let e = tempfile::tempdir().unwrap();
    assert_eq!(key(e.path()), None);
    // a gitlink (submodule) entry: its dirtiness cannot be judged from the superproject's stat
    let (_d, r) = repo();
    let oid = String::from_utf8(
        Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(&r)
            .output()
            .unwrap()
            .stdout,
    )
    .unwrap();
    git(
        &r,
        &[
            "update-index",
            "--add",
            "--cacheinfo",
            &format!("160000,{},mod", oid.trim()),
        ],
    );
    assert_eq!(key(&r), None);
}
