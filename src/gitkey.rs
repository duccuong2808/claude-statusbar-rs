//! A cheap fingerprint of everything `git status --porcelain=v1 --branch` reports, computed from
//! the index and a few stats without spawning git (port of the old statusline's `changes_key`).
//! While it is unchanged the cached dirty / ahead / behind facts are still right, so a render
//! never has to ask git again.
//!
//! What goes in:
//! - HEAD and the commit it resolves to, plus the upstream's commit (ahead / behind).
//! - `.git/index`'s stat, and the stat of every tracked file whose stat no longer matches its
//!   index entry or is racy (mtime not older than the index), so each further edit changes it too.
//! - The mtime of every directory that holds a tracked file: creating, deleting or renaming an
//!   entry bumps its parent's mtime, which is how untracked files show up.
//! - `.git/config`, `info/exclude` and the user's global git config / ignore file.
//!
//! None (the caller falls back to a plain time-to-live) for what the index alone cannot answer:
//! submodules, split or sparse index, SHA-256 repos, `core.worktree`, and very large indexes,
//! where the stat walk would cost more than the render.

use std::collections::hash_map::DefaultHasher;
use std::collections::HashSet;
use std::ffi::OsStr;
use std::fs;
use std::hash::{Hash, Hasher};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

use crate::identity::resolve_gitdir;

/// Above this many index entries the walk is no longer cheap; use the time-to-live instead.
pub const MAX_ENTRIES: u32 = 20_000;

/// The fingerprint of `toplevel`'s worktree, or None when it cannot be judged without git.
pub fn key(toplevel: &Path) -> Option<u64> {
    let gd = resolve_gitdir(toplevel)?;
    let cd = common_dir(&gd);
    let config = fs::read_to_string(cd.join("config")).unwrap_or_default();
    let flat = config.to_ascii_lowercase().replace([' ', '\t'], "");
    if flat.contains("objectformat") || flat.contains("worktree=") {
        return None; // SHA-256 entries are laid out differently; core.worktree moves the root
    }

    let mut h = DefaultHasher::new();
    config.hash(&mut h);
    let head = fs::read_to_string(gd.join("HEAD")).unwrap_or_default();
    let head = head.trim();
    head.hash(&mut h);
    match head.strip_prefix("ref:").map(str::trim) {
        Some(r) => {
            resolve_ref(&gd, &cd, r).hash(&mut h);
            let branch = r.strip_prefix("refs/heads/").unwrap_or(r);
            if let Some(up) = upstream_ref(&config, branch) {
                (&up, resolve_ref(&gd, &cd, &up)).hash(&mut h);
            }
        }
        None => "detached".hash(&mut h),
    }
    for f in ignore_sources(&cd) {
        stat_of(&f).hash(&mut h);
    }

    index_part(&gd.join("index"), toplevel, &mut h)?;
    Some(h.finish())
}

/// Hashes the index's stat, the stat of every changed or racy tracked file and the mtime of
/// every directory holding one. None for an index layout this walk cannot interpret.
fn index_part(index_path: &Path, work: &Path, h: &mut DefaultHasher) -> Option<()> {
    let im = fs::metadata(index_path).ok()?;
    let data = fs::read(index_path).ok()?;
    (im.mtime(), im.mtime_nsec(), im.size(), im.ino()).hash(h);
    let be32 = |p: usize| -> Option<u32> {
        Some(u32::from_be_bytes(data.get(p..p + 4)?.try_into().ok()?))
    };
    let be16 = |p: usize| -> Option<u16> {
        Some(u16::from_be_bytes(data.get(p..p + 2)?.try_into().ok()?))
    };
    if data.get(0..4)? != b"DIRC" {
        return None;
    }
    let version = be32(4)?;
    let count = be32(8)?;
    if !(2..=4).contains(&version) || count > MAX_ENTRIES {
        return None;
    }
    let racy_at = (im.mtime(), im.mtime_nsec());
    let mut dirs: HashSet<Vec<u8>> = HashSet::new();
    dirs.insert(Vec::new()); // the worktree root
    let mut path: Vec<u8> = Vec::new();
    let mut pos = 12;
    for _ in 0..count {
        let (m_s, m_ns) = (i64::from(be32(pos + 8)?), i64::from(be32(pos + 12)?));
        let ino = be32(pos + 20)?;
        let mode = be32(pos + 24)?;
        let size = be32(pos + 36)?;
        let flags = be16(pos + 60)?;
        let (fixed, ext) = if flags & 0x4000 != 0 {
            (64, be16(pos + 62)?)
        } else {
            (62, 0)
        };
        let name = pos + fixed;
        if version == 4 {
            // Prefix-compressed: a varint of bytes to drop from the previous path, then the
            // NUL-terminated suffix.
            let mut p = name;
            let mut c = *data.get(p)?;
            p += 1;
            let mut strip = usize::from(c & 127);
            while c & 128 != 0 {
                c = *data.get(p)?;
                p += 1;
                strip = ((strip + 1) << 7) | usize::from(c & 127);
            }
            let nul = p + data.get(p..)?.iter().position(|&b| b == 0)?;
            path.truncate(path.len().checked_sub(strip)?);
            path.extend_from_slice(&data[p..nul]);
            pos = nul + 1;
        } else {
            let nul = name + data.get(name..)?.iter().position(|&b| b == 0)?;
            path.clear();
            path.extend_from_slice(&data[name..nul]);
            pos += (fixed + (nul - name) + 8) & !7;
        }
        if mode & 0o170000 == 0o160000 {
            return None; // submodule: its own worktree decides whether it is dirty
        }
        // Every parent directory, longest first; once one is known, so are all shorter ones.
        let mut end = path.len();
        while let Some(i) = path[..end].iter().rposition(|&b| b == b'/') {
            if !dirs.insert(path[..i].to_vec()) {
                break;
            }
            end = i;
        }
        let stage = (flags >> 12) & 3;
        let skip = flags & 0x8000 != 0 || ext & 0x4000 != 0; // assume-valid, skip-worktree
        if stage != 0 || skip {
            path.hash(h); // conflicted / not checked out: status depends only on the index
            continue;
        }
        let file = work.join(OsStr::from_bytes(&path));
        match fs::symlink_metadata(&file) {
            Ok(m) => {
                let same = m.mtime() == m_s
                    && (m_ns == 0 || m.mtime_nsec() == m_ns)
                    && m.size() as u32 == size
                    && m.ino() as u32 == ino;
                let racy = (m.mtime(), m.mtime_nsec()) >= racy_at;
                if !same || racy {
                    (&path, m.mtime(), m.mtime_nsec(), m.size(), m.ino()).hash(h);
                }
            }
            Err(_) => (&path, "missing").hash(h),
        }
    }
    // Split index ("link") or sparse index ("sdir"): the entries above are not the whole story.
    let trailer = 20;
    while pos + 8 <= data.len().saturating_sub(trailer) {
        let sig = &data[pos..pos + 4];
        if sig == b"link" || sig == b"sdir" {
            return None;
        }
        pos += 8 + be32(pos + 4)? as usize;
    }
    let mut dirs: Vec<Vec<u8>> = dirs.into_iter().collect();
    dirs.sort_unstable(); // HashSet order is random per process; the key must not be
    for d in dirs {
        let m = fs::metadata(work.join(OsStr::from_bytes(&d)));
        (&d, m.ok().map(|m| (m.mtime(), m.mtime_nsec(), m.ino()))).hash(h);
    }
    Some(())
}

/// Where objects, refs and config live: the gitdir itself, or a linked worktree's `commondir`.
fn common_dir(gd: &Path) -> PathBuf {
    match fs::read_to_string(gd.join("commondir")) {
        Ok(s) => gd.join(s.trim()),
        Err(_) => gd.to_path_buf(),
    }
}

/// The oid a ref names: loose in the gitdir, loose in the common dir, else from packed-refs.
fn resolve_ref(gd: &Path, cd: &Path, r: &str) -> String {
    for base in [gd, cd] {
        if let Ok(oid) = fs::read_to_string(base.join(r)) {
            return oid.trim().to_string();
        }
    }
    let packed = fs::read_to_string(cd.join("packed-refs")).unwrap_or_default();
    packed
        .lines()
        .find_map(|l| l.strip_suffix(r).and_then(|p| p.strip_suffix(' ')))
        .map_or_else(|| format!("unresolved {r}"), str::to_string)
}

/// `branch.<name>.remote` + `branch.<name>.merge` → the remote-tracking ref status compares to.
fn upstream_ref(config: &str, branch: &str) -> Option<String> {
    let section = format!("[branch \"{branch}\"]");
    let (mut in_section, mut remote, mut merge) = (false, None, None);
    for line in config.lines().map(str::trim) {
        if line.starts_with('[') {
            in_section = line == section;
        } else if let (true, Some((k, v))) = (in_section, line.split_once('=')) {
            match k.trim().to_ascii_lowercase().as_str() {
                "remote" => remote = Some(v.trim()),
                "merge" => merge = Some(v.trim()),
                _ => {}
            }
        }
    }
    let (remote, merge) = (remote?, merge?);
    if remote == "." {
        return Some(merge.to_string()); // tracks a local branch
    }
    Some(format!(
        "refs/remotes/{remote}/{}",
        merge.strip_prefix("refs/heads/").unwrap_or(merge)
    ))
}

/// Files whose edits can change what counts as untracked: the repo's exclude file and the
/// user's global config (core.excludesFile, status.showUntrackedFiles) and ignore file.
fn ignore_sources(cd: &Path) -> Vec<PathBuf> {
    let mut v = vec![cd.join("info").join("exclude")];
    if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
        v.push(home.join(".gitconfig"));
        let xdg = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .unwrap_or_else(|| home.join(".config"));
        v.push(xdg.join("git").join("config"));
        v.push(xdg.join("git").join("ignore"));
    }
    v
}

fn stat_of(p: &Path) -> Option<(i64, i64, u64, u64)> {
    let m = fs::metadata(p).ok()?;
    Some((m.mtime(), m.mtime_nsec(), m.size(), m.ino()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upstream_from_branch_section() {
        let cfg = "[core]\n\tbare = false\n[branch \"main\"]\n\tremote = origin\n\tmerge = refs/heads/main\n[branch \"x\"]\n\tremote = .\n\tmerge = refs/heads/main\n";
        assert_eq!(
            upstream_ref(cfg, "main").as_deref(),
            Some("refs/remotes/origin/main")
        );
        assert_eq!(upstream_ref(cfg, "x").as_deref(), Some("refs/heads/main"));
        assert_eq!(upstream_ref(cfg, "none"), None);
    }

    #[test]
    fn packed_ref_resolves() {
        let d = tempfile::tempdir().unwrap();
        fs::write(
            d.path().join("packed-refs"),
            "# pack-refs with: peeled\nabc123 refs/remotes/origin/main\n",
        )
        .unwrap();
        assert_eq!(
            resolve_ref(d.path(), d.path(), "refs/remotes/origin/main"),
            "abc123"
        );
        assert_eq!(
            resolve_ref(d.path(), d.path(), "refs/heads/gone"),
            "unresolved refs/heads/gone"
        );
    }
}
