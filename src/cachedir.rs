//! Where the small shared caches live. On Linux they go to a RAM-backed directory
//! (`$XDG_RUNTIME_DIR`, else `/dev/shm`) so the 30 s git refresh and quota updates never write to
//! the SSD; macOS has no tmpfs, so it (and any unusable candidate) uses `$HOME/.cache/csr`.
//! Losing the cache on reboot is harmless: every entry is rebuilt within one refresh.

use std::fs;
use std::os::unix::fs::{DirBuilderExt, MetadataExt};
use std::path::{Path, PathBuf};

/// Whether the host has a RAM-backed runtime directory worth preferring.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Os {
    Linux,
    Other,
}

/// Candidate directories, best first; the last one (`$HOME/.cache/csr`) is the unconditional fallback.
pub fn candidates(home: &Path, xdg_runtime_dir: Option<&str>, uid: u32, os: Os) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if os == Os::Linux {
        if let Some(x) = xdg_runtime_dir.filter(|x| x.starts_with('/')) {
            out.push(Path::new(x).join("csr"));
        }
        out.push(PathBuf::from(format!("/dev/shm/csr-{uid}")));
    }
    out.push(home.join(".cache").join("csr"));
    out
}

/// A private (0700), existing directory we own that we can write into.
fn usable(dir: &Path, uid: u32) -> bool {
    let made = fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)
        .is_ok();
    // /dev/shm is shared: refuse a directory someone else pre-created under our name.
    let ours = fs::metadata(dir).is_ok_and(|m| m.is_dir() && m.uid() == uid);
    made && ours && fs::File::create(dir.join(".probe")).is_ok()
}

pub fn pick(cands: &[PathBuf], uid: u32) -> PathBuf {
    let (last, rest) = cands
        .split_last()
        .expect("candidates always end with the fallback");
    rest.iter().find(|d| usable(d, uid)).unwrap_or(last).clone()
}

pub fn resolve(home: &Path) -> PathBuf {
    let xdg = std::env::var("XDG_RUNTIME_DIR").ok();
    resolve_with(home, xdg.as_deref())
}

/// `resolve` with `$XDG_RUNTIME_DIR` passed in, so a test can predict where a child process
/// started with that environment will look, on whichever host OS it runs.
pub fn resolve_with(home: &Path, xdg_runtime_dir: Option<&str>) -> PathBuf {
    let uid = fs::metadata(home).map_or(0, |m| m.uid());
    pick(
        &candidates(
            home,
            xdg_runtime_dir,
            uid,
            if cfg!(target_os = "linux") {
                Os::Linux
            } else {
                Os::Other
            },
        ),
        uid,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linux_prefers_xdg_then_shm_then_home() {
        let c = candidates(Path::new("/h"), Some("/run/user/1000"), 1000, Os::Linux);
        assert_eq!(
            c,
            vec![
                PathBuf::from("/run/user/1000/csr"),
                PathBuf::from("/dev/shm/csr-1000"),
                PathBuf::from("/h/.cache/csr"),
            ]
        );
    }

    #[test]
    fn relative_or_missing_xdg_is_skipped() {
        for x in [None, Some(""), Some("rel/dir")] {
            let c = candidates(Path::new("/h"), x, 7, Os::Linux);
            assert_eq!(
                c,
                vec![
                    PathBuf::from("/dev/shm/csr-7"),
                    PathBuf::from("/h/.cache/csr")
                ]
            );
        }
    }

    #[test]
    fn macos_uses_home_cache_only() {
        let c = candidates(Path::new("/h"), Some("/run/user/1"), 1, Os::Other);
        assert_eq!(c, vec![PathBuf::from("/h/.cache/csr")]);
    }

    #[test]
    fn pick_takes_first_usable_and_falls_back() {
        let d = tempfile::tempdir().unwrap();
        let uid = fs::metadata(d.path()).unwrap().uid();
        let fallback = d.path().join("home-cache");
        let good = d.path().join("good");
        // a plain file blocks the first candidate
        let blocked = d.path().join("blocked");
        fs::write(&blocked, b"x").unwrap();
        let picked = pick(&[blocked.join("csr"), good.clone(), fallback.clone()], uid);
        assert_eq!(picked, good);
        assert!(good.is_dir());
        assert_eq!(
            pick(&[blocked.join("csr"), fallback.clone()], uid),
            fallback
        );
    }

    #[test]
    fn pick_rejects_a_directory_owned_by_someone_else() {
        let d = tempfile::tempdir().unwrap();
        let uid = fs::metadata(d.path()).unwrap().uid();
        let fallback = d.path().join("fb");
        let theirs = d.path().join("theirs");
        assert_eq!(pick(&[theirs.clone(), fallback.clone()], uid + 1), fallback);
    }
}
