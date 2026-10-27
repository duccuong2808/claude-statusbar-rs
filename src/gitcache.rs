//! Cache of `git status` facts (port of git_cache.py + _git_refresh.py), validated by
//! `gitkey::key`: while the worktree's fingerprint matches the one stored with the entry, the
//! entry is reused without running git (up to `MAX_AGE_S`); a different fingerprint makes it stale
//! at once. Repos without a fingerprint fall back to a 30-second time-to-live.
//! The render path only reads the cache; a stale cache spawns a detached `csr git-refresh`.

use serde::{Deserialize, Serialize};
use std::fs;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use crate::atomic_file;
use crate::clock::unix_now;
use crate::gitkey;

pub const TTL_S: f64 = 30.0;
/// A fingerprint-matched entry is still re-checked with git this often, as a backstop for what
/// the fingerprint cannot see (files emptied out of an untracked directory, a custom
/// core.excludesFile edited in place).
pub const MAX_AGE_S: f64 = 600.0;
pub const INFLIGHT_MAX_AGE_S: f64 = 30.0;
const LOCK_MAX_AGE_S: u64 = 60;
/// `git status` gets this long before it is killed (a hung git must never pile up children).
pub const GIT_TIMEOUT_S: f64 = 5.0;
/// Exit polling interval. A typical git run is ~10 ms, so a coarse poll would mostly be waiting;
/// a 1 ms try_wait costs microseconds, even over the full timeout of a hung git.
const POLL: Duration = Duration::from_millis(1);
/// How long the hook waits for another refresh's lock: past GIT_TIMEOUT_S, so by then that
/// refresh's git has finished or been killed.
const LOCK_WAIT: Duration = Duration::from_secs(6);
const LOCK_POLL: Duration = Duration::from_millis(5);

/// Who asked for a refresh; decides what happens when another refresh holds the lock.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Trigger {
    /// The detached child a render spawned. Gives up at once on a held lock (its holder clears
    /// the render's inflight marker when done).
    Render,
    /// The PostToolUse hook. Its tool call may have changed the tree after the running refresh
    /// took its fingerprint, so it waits for the lock and checks again.
    Hook,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
pub struct Entry {
    pub toplevel: String,
    pub dirty: Option<bool>,
    pub ahead: Option<i64>,
    pub behind: Option<i64>,
    pub ts: f64,
    /// `gitkey::key` taken just before the git run that produced this entry.
    #[serde(default)]
    pub key: Option<u64>,
}

pub fn fnv1a64(s: &str) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in s.bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

pub fn cache_path(cache_dir: &Path, toplevel: &str) -> PathBuf {
    cache_dir
        .join("git")
        .join(format!("{:016x}.json", fnv1a64(toplevel)))
}

pub fn read_entry(path: &Path) -> Option<Entry> {
    serde_json::from_str(&fs::read_to_string(path).ok()?).ok()
}

pub fn is_fresh(e: &Entry, now: f64) -> bool {
    let age = now - e.ts;
    (0.0..TTL_S).contains(&age)
}

/// Whether `e` still describes the worktree whose fingerprint is now `key`.
pub fn is_current(e: &Entry, key: Option<u64>, now: f64) -> bool {
    match (e.key, key) {
        (Some(a), Some(b)) if a == b => (0.0..MAX_AGE_S).contains(&(now - e.ts)),
        (Some(_), Some(_)) => false,
        // No fingerprint now, or none stored (failed git run, older cache file): time-to-live.
        _ => is_fresh(e, now),
    }
}

/// None when the file is missing. An mtime in the future (clock stepped back, or a network /
/// drvfs filesystem whose clock runs ahead) counts as fresh while it is within the lock lifetime
/// and as very old beyond it, so a stranded marker is still cleared eventually but a live one is
/// never mistaken for stale.
fn file_age_secs(path: &Path) -> Option<f64> {
    let modified = fs::metadata(path).ok()?.modified().ok()?;
    Some(
        match std::time::SystemTime::now().duration_since(modified) {
            Ok(age) => age.as_secs_f64(),
            Err(e) if e.duration().as_secs_f64() > LOCK_MAX_AGE_S as f64 => f64::MAX,
            Err(_) => 0.0,
        },
    )
}

/// Cached dirty flag (None = unknown) plus the whole entry. Kicks off a detached refresh when the
/// cache is stale or missing. Never blocks on git.
pub fn dirty_with_async_refresh(
    cache_dir: &Path,
    toplevel: &str,
    now: f64,
) -> (Option<bool>, Option<Entry>) {
    let path = cache_path(cache_dir, toplevel);
    let entry = read_entry(&path);
    if let Some(e) = &entry {
        if is_current(e, gitkey::key(Path::new(toplevel)), now) {
            return (e.dirty, entry);
        }
    }
    let inflight = path.with_extension("inflight");
    let inflight_recent = file_age_secs(&inflight).is_some_and(|a| a < INFLIGHT_MAX_AGE_S);
    if !inflight_recent {
        start_refresh(&inflight, || {
            std::env::current_exe()
                .ok()
                .and_then(|exe| {
                    // All three stdio to null: an inherited stdout would hold Claude Code's pipe open
                    // until `git status` finishes (seconds on a big repo).
                    Command::new(exe)
                        .arg("git-refresh")
                        .arg(toplevel)
                        .arg(cache_dir)
                        .stdin(Stdio::null())
                        .stdout(Stdio::null())
                        .stderr(Stdio::null())
                        .process_group(0)
                        .spawn()
                        .ok()
                })
                .is_some()
        });
    }
    (entry.as_ref().and_then(|e| e.dirty), entry)
}

/// Write the inflight marker, then spawn. No marker (unwritable cache dir) → no child at all:
/// without the marker every render would spawn another one. A failed spawn removes the marker.
fn start_refresh(inflight: &Path, spawn: impl FnOnce() -> bool) {
    if let Some(dir) = inflight.parent() {
        let _ = fs::create_dir_all(dir);
    }
    if fs::write(inflight, b"").is_err() {
        return;
    }
    if !spawn() {
        let _ = fs::remove_file(inflight);
    }
}

/// The facts one `git status --porcelain=v1 --branch` run yields.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Status {
    pub dirty: bool,
    /// None without an upstream (`...` absent from the `## ` header), 0 when in sync.
    pub ahead: Option<i64>,
    pub behind: Option<i64>,
}

/// Parse `git status --porcelain=v1 --branch` output.
pub fn parse_status(stdout: &str) -> Status {
    let mut header = "";
    let mut dirty = false;
    for ln in stdout.lines() {
        if let Some(h) = ln.strip_prefix("## ") {
            header = h;
        } else if !ln.trim().is_empty() {
            dirty = true;
        }
    }
    let (mut ahead, mut behind) = (None, None);
    if header.contains("...") {
        ahead = Some(0);
        behind = Some(0);
        if let Some(i) = header.find("ahead ") {
            ahead = Some(leading_int(&header[i + 6..]));
        }
        if let Some(i) = header.find("behind ") {
            behind = Some(leading_int(&header[i + 7..]));
        }
    }
    Status {
        dirty,
        ahead,
        behind,
    }
}

fn leading_int(s: &str) -> i64 {
    s.bytes()
        .take_while(u8::is_ascii_digit)
        .fold(0i64, |acc, b| {
            acc.saturating_mul(10).saturating_add(i64::from(b - b'0'))
        })
}

fn write_entry(path: &Path, e: &Entry) {
    if let Ok(bytes) = serde_json::to_vec(e) {
        let _ = atomic_file::write(path, &bytes);
    }
}

/// Run `git status` with stdout going to `out` (a file, so a huge porcelain output can never
/// fill a pipe), polling for exit until `timeout`; on expiry the child is killed and reaped.
/// Returns the output text only for a successful exit.
fn run_git(git: &Path, toplevel: &str, out: &Path, timeout: Duration) -> Option<String> {
    let file = fs::File::create(out).ok()?;
    let mut child = Command::new(git)
        .args([
            "-C",
            toplevel,
            "--no-optional-locks",
            "status",
            "--porcelain=v1",
            "--branch",
        ])
        .stdin(Stdio::null())
        .stdout(file)
        .stderr(Stdio::null())
        // Own process group, so a timeout can take down whatever git started (fsmonitor hook,
        // submodule status, a hook stuck on NFS) instead of leaving it reparented to init.
        .process_group(0)
        .spawn()
        .ok()?;
    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(st)) => break Some(st),
            Ok(None) if Instant::now() < deadline => std::thread::sleep(POLL),
            _ => {
                let _ = Command::new("kill")
                    .args(["-KILL", "--", &format!("-{}", child.id())])
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status();
                let _ = child.kill(); // fallback when `kill` itself could not run
                let _ = child.wait();
                break None;
            }
        }
    };
    if !status?.success() {
        return None;
    }
    fs::read(out)
        .ok()
        .map(|b| String::from_utf8_lossy(&b).into_owned())
}

/// The body of `csr git-refresh <toplevel>` and `csr hook`. Runs git (no optional locks, so a
/// killed run can never strand `.git/index.lock`), writes the cache, clears markers. Silent.
pub fn refresh(cache_dir: &Path, toplevel: &str, trigger: Trigger) {
    refresh_with(
        cache_dir,
        toplevel,
        Path::new("git"),
        Duration::from_secs_f64(GIT_TIMEOUT_S),
        trigger,
    );
}

/// `refresh` with the git executable and the timeout injectable (tests use a fake git).
pub fn refresh_with(
    cache_dir: &Path,
    toplevel: &str,
    git: &Path,
    timeout: Duration,
    trigger: Trigger,
) {
    let path = cache_path(cache_dir, toplevel);
    let inflight = path.with_extension("inflight");
    // Taken before git runs: an edit made while it runs then leaves a mismatch, not a stale match.
    let key = gitkey::key(Path::new(toplevel));
    let current = || read_entry(&path).is_some_and(|e| is_current(&e, key, unix_now()));
    if current() {
        // Nothing written (the usual hook case, e.g. after a Bash `ls`); a render's child still
        // clears its marker, which would otherwise hold off the next spawn for 30 s.
        if trigger == Trigger::Render {
            let _ = fs::remove_file(&inflight);
        }
        return;
    }
    let Some(lock) = acquire_lock(&path, trigger) else {
        return; // the refresh holding the lock clears the markers when it is done
    };
    // A refresh that finished while this one waited or was being spawned already did the work.
    if !current() {
        let out = path.with_extension(format!("out{}", std::process::id()));
        let result = run_git(git, toplevel, &out, timeout).map(|text| parse_status(&text));
        let _ = fs::remove_file(&out);
        let entry = if let Some(st) = result {
            Entry {
                toplevel: toplevel.to_string(),
                dirty: Some(st.dirty),
                ahead: st.ahead,
                behind: st.behind,
                ts: unix_now(),
                key,
            }
        } else {
            let mut e = read_entry(&path).unwrap_or_default();
            e.toplevel = toplevel.to_string();
            e.ts = unix_now();
            e.key = None; // retry after the time-to-live, not after MAX_AGE_S
            e
        };
        write_entry(&path, &entry);
    }
    let _ = fs::remove_file(&inflight);
    let _ = fs::remove_file(&lock);
}

/// Takes `<entry>.lock`, first clearing one stranded past LOCK_MAX_AGE_S. A render's child gives
/// up at once when it is held; the hook polls for up to LOCK_WAIT.
fn acquire_lock(path: &Path, trigger: Trigger) -> Option<PathBuf> {
    if let Some(dir) = path.parent() {
        let _ = fs::create_dir_all(dir);
    }
    let lock = path.with_extension("lock");
    let wait = match trigger {
        Trigger::Render => Duration::ZERO,
        Trigger::Hook => LOCK_WAIT,
    };
    let deadline = Instant::now() + wait;
    loop {
        if file_age_secs(&lock).is_some_and(|age| age >= LOCK_MAX_AGE_S as f64) {
            let _ = fs::remove_file(&lock);
        }
        let taken = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&lock)
            .is_ok();
        if taken {
            return Some(lock);
        }
        if Instant::now() >= deadline {
            return None;
        }
        std::thread::sleep(LOCK_POLL);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    fn status(dirty: bool, ahead: Option<i64>, behind: Option<i64>) -> Status {
        Status {
            dirty,
            ahead,
            behind,
        }
    }

    #[test]
    fn parses_porcelain_branch_header() {
        // 25 digits: saturates instead of overflowing
        assert_eq!(
            parse_status("## a...o/a [ahead 9999999999999999999999999]\n").ahead,
            Some(i64::MAX)
        );
        assert_eq!(
            parse_status("## main...origin/main [ahead 2, behind 1]\n M a.rs\n?? b\n"),
            status(true, Some(2), Some(1))
        );
        assert_eq!(
            parse_status("## main...origin/main\n"),
            status(false, Some(0), Some(0))
        );
        assert_eq!(parse_status("## main\n"), status(false, None, None));
        assert_eq!(parse_status("## main\n\n"), status(false, None, None));
        assert_eq!(
            parse_status("## HEAD (no branch)\n M x\n"),
            status(true, None, None)
        );
    }

    #[test]
    fn cache_path_is_stable_and_freshness_window_is_30s() {
        let home = Path::new("/h");
        let p = cache_path(home, "/repo");
        assert_eq!(p.parent().unwrap(), Path::new("/h/git"));
        assert_eq!(p, cache_path(home, "/repo"));
        assert_ne!(p, cache_path(home, "/repo2"));
        let e = Entry {
            toplevel: "/repo".into(),
            dirty: Some(true),
            ahead: None,
            behind: None,
            ts: 1000.0,
            key: None,
        };
        assert!(is_fresh(&e, 1000.0));
        assert!(is_fresh(&e, 1029.9));
        assert!(!is_fresh(&e, 1030.0));
        assert!(!is_fresh(&e, 999.0));
    }

    #[test]
    fn matching_key_outlives_the_ttl_and_a_new_key_does_not_wait_for_it() {
        let keyed = Entry {
            ts: 1000.0,
            key: Some(7),
            ..Entry::default()
        };
        assert!(is_current(&keyed, Some(7), 1000.0 + TTL_S + 1.0));
        assert!(is_current(&keyed, Some(7), 1000.0 + MAX_AGE_S - 0.1));
        assert!(!is_current(&keyed, Some(7), 1000.0 + MAX_AGE_S));
        assert!(!is_current(&keyed, Some(8), 1000.1));
        // no fingerprint on either side: plain time-to-live
        assert!(is_current(&keyed, None, 1000.0 + TTL_S - 0.1));
        assert!(!is_current(&keyed, None, 1000.0 + TTL_S));
        let unkeyed = Entry {
            ts: 1000.0,
            ..Entry::default()
        };
        assert!(is_current(&unkeyed, Some(7), 1000.0 + TTL_S - 0.1));
        assert!(!is_current(&unkeyed, Some(7), 1000.0 + TTL_S));
    }

    /// `<dir>/repo` with `f.txt` committed (so it has an index and a fingerprint); returns the
    /// repo path and its canonical toplevel string, the cache key.
    fn committed_repo(dir: &Path) -> (PathBuf, String) {
        let repo = dir.join("repo");
        fs::create_dir_all(&repo).unwrap();
        fs::write(repo.join("f.txt"), "1\n").unwrap();
        for args in [
            &["init", "-q"][..],
            &["add", "."],
            &["commit", "-qm", "init"],
        ] {
            let out = Command::new("git")
                .args(["-c", "user.email=t@t", "-c", "user.name=t"])
                .args(args)
                .current_dir(&repo)
                .output()
                .unwrap();
            assert!(out.status.success(), "git {args:?}: {out:?}");
        }
        let top = repo.canonicalize().unwrap().to_string_lossy().into_owned();
        (repo, top)
    }

    #[test]
    fn keyed_refresh_reruns_git_after_an_edit_and_is_reused_while_nothing_changes() {
        let d = tempfile::tempdir().unwrap();
        let home = d.path().join("home");
        let (repo, top) = committed_repo(d.path());
        let path = cache_path(&home, &top);

        refresh(&home, &top, Trigger::Render);
        let e = read_entry(&path).unwrap();
        assert_eq!(e.dirty, Some(false));
        assert!(e.key.is_some());
        // Unchanged worktree: still served well past the 30 s time-to-live, nothing spawned.
        let (dirty, _) = dirty_with_async_refresh(&home, &top, e.ts + 300.0);
        assert_eq!(dirty, Some(false));
        assert!(!path.with_extension("inflight").exists());

        // An edit within the 30 s window is picked up by the very next refresh.
        std::fs::write(repo.join("f.txt"), "2\n").unwrap();
        refresh(&home, &top, Trigger::Render);
        assert_eq!(read_entry(&path).unwrap().dirty, Some(true));
    }

    #[test]
    fn hook_on_an_unchanged_tree_runs_no_git_and_writes_nothing() {
        let d = tempfile::tempdir().unwrap();
        let home = d.path().join("home");
        let (_repo, top) = committed_repo(d.path());
        let path = cache_path(&home, &top);
        refresh(&home, &top, Trigger::Hook);
        let before = read_entry(&path).expect("first hook wrote the entry");
        // A render's marker must survive: the hook did not take over that render's child.
        fs::write(path.with_extension("inflight"), b"").unwrap();

        let marker = d.path().join("ran");
        let git = fake_git(d.path(), &format!("touch '{}'", marker.display()));
        refresh_with(&home, &top, &git, Duration::from_secs(5), Trigger::Hook);
        assert!(!marker.exists(), "git ran although nothing changed");
        assert_eq!(read_entry(&path).unwrap(), before);
        let inflight = path.with_extension("inflight");
        let inflight = inflight.file_name().unwrap().to_string_lossy();
        assert_eq!(leftovers(&path), vec![inflight.into_owned()]);
    }

    #[test]
    fn hook_waits_for_a_held_lock_and_then_sees_its_own_edit() {
        let d = tempfile::tempdir().unwrap();
        let home = d.path().join("home");
        let (repo, top) = committed_repo(d.path());
        let path = cache_path(&home, &top);
        refresh(&home, &top, Trigger::Render);
        // Another refresh holds the lock, its fingerprint taken before this edit.
        fs::write(repo.join("f.txt"), "2\n").unwrap();
        let lock = path.with_extension("lock");
        fs::write(&lock, b"").unwrap();

        // A render's child gives up and leaves the entry alone ...
        refresh(&home, &top, Trigger::Render);
        assert_eq!(read_entry(&path).unwrap().dirty, Some(false));

        // ... the hook waits for the holder to finish, then refreshes.
        let release = std::thread::spawn({
            let lock = lock.clone();
            move || {
                std::thread::sleep(Duration::from_millis(150));
                fs::remove_file(lock).unwrap();
            }
        });
        let t0 = Instant::now();
        refresh(&home, &top, Trigger::Hook);
        release.join().unwrap();
        assert!(t0.elapsed() >= Duration::from_millis(100), "did not wait");
        assert_eq!(read_entry(&path).unwrap().dirty, Some(true));
        assert!(!lock.exists());
    }

    #[test]
    fn refresh_writes_entry_for_a_real_repo() {
        let d = tempfile::tempdir().unwrap();
        let home = d.path().join("home");
        let repo = d.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        let git = |args: &[&str]| {
            assert!(Command::new("git")
                .args(args)
                .current_dir(&repo)
                .output()
                .unwrap()
                .status
                .success());
        };
        git(&["init", "-q"]); // no `-b`: Ubuntu 20.04 ships git 2.25, which lacks it
        git(&[
            "-c",
            "user.email=t@t",
            "-c",
            "user.name=t",
            "commit",
            "-q",
            "--allow-empty",
            "-m",
            "init",
        ]);
        let top = repo.canonicalize().unwrap().to_string_lossy().into_owned();
        refresh(&home, &top, Trigger::Render);
        let e = read_entry(&cache_path(&home, &top)).expect("entry written");
        assert_eq!(e.dirty, Some(false));
        assert_eq!((e.ahead, e.behind), (None, None));
        std::fs::write(repo.join("new.txt"), "x").unwrap();
        std::fs::remove_file(cache_path(&home, &top)).unwrap();
        refresh(&home, &top, Trigger::Render);
        assert_eq!(
            read_entry(&cache_path(&home, &top)).unwrap().dirty,
            Some(true)
        );
        assert!(!cache_path(&home, &top).with_extension("lock").exists());
        assert!(!cache_path(&home, &top).with_extension("inflight").exists());
    }

    #[test]
    fn fresh_cache_is_returned_without_spawning() {
        let d = tempfile::tempdir().unwrap();
        let home = d.path();
        let path = cache_path(home, "/repo");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(
            &path,
            r#"{"toplevel":"/repo","dirty":true,"ahead":0,"behind":3,"ts":5000.0}"#,
        )
        .unwrap();
        let (dirty, entry) = dirty_with_async_refresh(home, "/repo", 5010.0);
        assert_eq!(dirty, Some(true));
        assert_eq!(entry.unwrap().behind, Some(3));
        assert!(!path.with_extension("inflight").exists());
    }

    /// Create an executable fake `git` without ever holding a write fd to it in this process
    /// (a forked sibling test inheriting that fd would make exec fail with ETXTBSY).
    fn fake_git(dir: &Path, body: &str) -> PathBuf {
        let path = dir.join("git");
        let mut child = Command::new("sh")
            .args(["-c", "cat > \"$1\" && chmod 755 \"$1\"", "sh"])
            .arg(&path)
            .stdin(Stdio::piped())
            .spawn()
            .unwrap();
        use std::io::Write;
        child
            .stdin
            .take()
            .unwrap()
            .write_all(format!("#!/bin/sh\n{body}\n").as_bytes())
            .unwrap();
        assert!(child.wait().unwrap().success());
        path
    }

    fn leftovers(path: &Path) -> Vec<String> {
        let mut v: Vec<String> = fs::read_dir(path.parent().unwrap())
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| !n.ends_with(".json"))
            .collect();
        v.sort();
        v
    }

    fn seed(path: &Path, dirty: bool, ts: f64) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(
            path,
            format!(
                r#"{{"toplevel":"/repo","dirty":{dirty},"ahead":null,"behind":null,"ts":{ts}}}"#
            ),
        )
        .unwrap();
    }

    #[test]
    fn hung_git_is_killed_at_the_deadline_and_keeps_the_old_entry() {
        let d = tempfile::tempdir().unwrap();
        let home = d.path().join("home");
        let git = fake_git(d.path(), "exec sleep 30");
        let path = cache_path(&home, "/repo");
        seed(&path, true, 1000.0);
        fs::write(path.with_extension("inflight"), b"").unwrap();
        let t0 = std::time::Instant::now();
        // short deadline keeps the suite fast; the binary uses GIT_TIMEOUT_S through `refresh`
        refresh_with(
            &home,
            "/repo",
            &git,
            std::time::Duration::from_millis(300),
            Trigger::Render,
        );
        let took = t0.elapsed().as_secs_f64();
        assert!(took < 1.5, "refresh took {took}s");
        let e = read_entry(&path).unwrap();
        assert_eq!(e.dirty, Some(true));
        assert!(e.ts > unix_now() - 60.0, "ts not refreshed: {}", e.ts);
        assert_eq!(leftovers(&path), Vec::<String>::new());
    }

    #[test]
    fn porcelain_is_read_back_from_the_temp_file() {
        let d = tempfile::tempdir().unwrap();
        let home = d.path().join("home");
        let git = fake_git(
            d.path(),
            "printf '## main...origin/main [ahead 4, behind 2]\\n M src/x.rs\\n'",
        );
        refresh_with(
            &home,
            "/repo",
            &git,
            std::time::Duration::from_secs(5),
            Trigger::Render,
        );
        let path = cache_path(&home, "/repo");
        let e = read_entry(&path).unwrap();
        assert_eq!((e.dirty, e.ahead, e.behind), (Some(true), Some(4), Some(2)));
        assert_eq!(leftovers(&path), Vec::<String>::new());
    }

    #[test]
    fn fresh_entry_after_taking_the_lock_skips_git() {
        let d = tempfile::tempdir().unwrap();
        let home = d.path().join("home");
        let marker = d.path().join("ran");
        let git = fake_git(d.path(), &format!("touch '{}'", marker.display()));
        let path = cache_path(&home, "/repo");
        let ts = unix_now();
        seed(&path, false, ts);
        // compare with the parsed value: serde_json's default float parsing may be 1 ULP off
        let seeded = read_entry(&path).unwrap().ts;
        fs::write(path.with_extension("inflight"), b"").unwrap();
        refresh_with(
            &home,
            "/repo",
            &git,
            std::time::Duration::from_secs(5),
            Trigger::Render,
        );
        assert!(!marker.exists(), "git ran although the entry was fresh");
        assert_eq!(read_entry(&path).unwrap().ts, seeded);
        assert_eq!(leftovers(&path), Vec::<String>::new());
    }

    #[test]
    fn lock_far_in_the_future_counts_as_stale() {
        let d = tempfile::tempdir().unwrap();
        let home = d.path().join("home");
        let git = fake_git(d.path(), "printf '## main\\n M x\\n'");
        let path = cache_path(&home, "/repo");
        seed(&path, false, 1000.0);
        let lock = fs::File::create(path.with_extension("lock")).unwrap();
        lock.set_modified(std::time::SystemTime::now() + std::time::Duration::from_secs(2 * 3600))
            .unwrap();
        drop(lock);
        refresh_with(
            &home,
            "/repo",
            &git,
            std::time::Duration::from_secs(5),
            Trigger::Render,
        );
        let e = read_entry(&path).unwrap();
        assert_eq!(e.dirty, Some(true));
        assert!(e.ts > 1000.0);
        assert_eq!(leftovers(&path), Vec::<String>::new());
    }

    #[test]
    fn lock_slightly_in_the_future_counts_as_live() {
        // a filesystem clock running a little ahead (NFS / SMB / drvfs) must not make a live
        // refresh's lock look stale
        let d = tempfile::tempdir().unwrap();
        let home = d.path().join("home");
        let git = fake_git(d.path(), "printf '## main\\n M x\\n'");
        let path = cache_path(&home, "/repo");
        seed(&path, false, 1000.0);
        let lock = fs::File::create(path.with_extension("lock")).unwrap();
        lock.set_modified(std::time::SystemTime::now() + std::time::Duration::from_secs(30))
            .unwrap();
        drop(lock);
        refresh_with(
            &home,
            "/repo",
            &git,
            std::time::Duration::from_secs(5),
            Trigger::Render,
        );
        let e = read_entry(&path).unwrap();
        assert_eq!(
            (e.dirty, e.ts),
            (Some(false), 1000.0),
            "entry rewritten although the lock is live"
        );
        assert!(path.with_extension("lock").exists());
    }

    #[test]
    fn no_inflight_marker_means_no_child() {
        let d = tempfile::tempdir().unwrap();
        fs::write(d.path().join("git"), b"not a dir").unwrap(); // cache dir cannot be created
        let inflight = cache_path(d.path(), "/repo").with_extension("inflight");
        let mut called = false;
        start_refresh(&inflight, || {
            called = true;
            true
        });
        assert!(
            !called,
            "spawned although the inflight marker could not be written"
        );
        // writable: marker written, spawn called; a failed spawn removes the marker again
        let home = d.path().join("ok");
        let inflight = cache_path(&home, "/repo").with_extension("inflight");
        let mut called = false;
        start_refresh(&inflight, || {
            called = true;
            true
        });
        assert!(called && inflight.exists());
        fs::remove_file(&inflight).unwrap();
        start_refresh(&inflight, || false);
        assert!(!inflight.exists());
    }

    fn pids_matching(pattern: &str) -> Vec<String> {
        let out = Command::new("pgrep")
            .args(["-f", pattern])
            .output()
            .unwrap();
        String::from_utf8_lossy(&out.stdout)
            .split_whitespace()
            .map(str::to_string)
            .collect()
    }

    /// Kills whatever still matches the pattern when the test ends, pass or fail.
    struct KillOnDrop(String);
    impl Drop for KillOnDrop {
        fn drop(&mut self) {
            for pid in pids_matching(&self.0) {
                let _ = Command::new("kill").args(["-KILL", &pid]).status();
            }
        }
    }

    #[test]
    fn timeout_kills_everything_git_started() {
        let d = tempfile::tempdir().unwrap();
        let home = d.path().join("home");
        // unique per run so pgrep only ever sees this test's grandchild
        let pattern = format!("sleep 297.{}", std::process::id());
        let _guard = KillOnDrop(pattern.clone());
        let git = fake_git(d.path(), &format!("{pattern} &\nwait"));
        refresh_with(
            &home,
            "/repo",
            &git,
            std::time::Duration::from_millis(300),
            Trigger::Render,
        );
        assert_eq!(
            pids_matching(&pattern),
            Vec::<String>::new(),
            "git's child survived the timeout"
        );
    }
}
