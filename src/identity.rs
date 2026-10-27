//! Project + branch identity from the stdin payload and the local filesystem (port of identity.py).
//! Pure file reads; never runs git.

use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use crate::payload::Payload;

#[derive(Debug, Default, Clone, PartialEq)]
pub struct Identity {
    pub project_name: String,
    pub in_git: bool,
    pub branch: Option<String>,
    pub detached: bool,
    pub worktree_name: Option<String>,
    pub toplevel: Option<PathBuf>,
    pub is_worktree: bool,
    /// Live linked worktrees of the repo (this one included). 0 when none / unknown.
    pub worktree_count: usize,
    pub web_url: Option<String>,
}

/// What `HEAD` points at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Head {
    /// Branch name, or the 7-char sha when detached.
    pub name: String,
    pub detached: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Worktree {
    pub name: String,
    pub repo_name: String,
    /// The main repo's `.git/worktrees/` dir.
    pub registry: PathBuf,
}

/// `start` (canonicalized when it exists) followed by every parent, root last.
fn ancestors(start: &Path) -> Vec<PathBuf> {
    let cur = if start.exists() {
        start.canonicalize().unwrap_or_else(|_| start.to_path_buf())
    } else {
        start.to_path_buf()
    };
    let mut v = vec![cur.clone()];
    let mut p = cur.as_path();
    while let Some(parent) = p.parent() {
        v.push(parent.to_path_buf());
        p = parent;
    }
    v
}

fn read_gitdir_pointer(dotgit_file: &Path) -> Option<String> {
    let text = fs::read_to_string(dotgit_file).ok()?;
    Some(text.trim().strip_prefix("gitdir:")?.trim().to_string())
}

/// The directory that holds HEAD: `.git/` itself, or the target of a `.git` file.
pub fn resolve_gitdir(start: &Path) -> Option<PathBuf> {
    for c in ancestors(start) {
        let dotgit = c.join(".git");
        if dotgit.is_dir() {
            return Some(dotgit);
        }
        if dotgit.is_file() {
            let raw = read_gitdir_pointer(&dotgit)?;
            let p = PathBuf::from(&raw);
            let p = if p.is_absolute() {
                p
            } else {
                c.join(p).canonicalize().ok()?
            };
            return if p.exists() { Some(p) } else { None };
        }
    }
    None
}

/// The checked-out branch (or detached sha); None when not in a git repo.
pub fn read_head(start: &Path) -> Option<Head> {
    let gitdir = resolve_gitdir(start)?;
    let text = fs::read_to_string(gitdir.join("HEAD")).ok()?;
    let text = text.trim();
    if let Some(r) = text.strip_prefix("ref:") {
        let r = r.trim();
        let name = match r.strip_prefix("refs/heads/") {
            Some(n) => n.to_string(),
            None => r.rsplit('/').next().unwrap_or(r).to_string(),
        };
        return Some(Head {
            name,
            detached: false,
        });
    }
    if text.len() == 40
        && text
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Some(Head {
            name: text[..7].to_string(),
            detached: true,
        });
    }
    None
}

/// First ancestor (including `start`) that contains a `.git` file or dir.
pub fn resolve_toplevel(start: &Path) -> Option<PathBuf> {
    ancestors(start)
        .into_iter()
        .find(|c| c.join(".git").exists())
}

/// Where a linked worktree's `.git` file points: `<root>/.git/worktrees/<name>`.
const WORKTREES_MARK: &str = "/.git/worktrees/";

/// Some(..) when `start` sits inside a *linked* worktree (`.git` file → `<root>/.git/worktrees/<name>`).
pub fn parse_worktree(start: &Path) -> Option<Worktree> {
    for c in ancestors(start) {
        let dotgit = c.join(".git");
        if dotgit.is_dir() {
            return None;
        }
        if dotgit.is_file() {
            let raw = read_gitdir_pointer(&dotgit)?;
            let norm = raw.replace('\\', "/");
            let norm = norm.trim_end_matches('/');
            let idx = norm.rfind(WORKTREES_MARK)?;
            let root = norm[..idx].trim_end_matches('/');
            let name = &norm[idx + WORKTREES_MARK.len()..];
            if root.is_empty() || name.is_empty() || name.contains('/') {
                return None;
            }
            let repo_name = root
                .rsplit('/')
                .next()
                .filter(|s| !s.is_empty())
                .unwrap_or(root)
                .to_string();
            return Some(Worktree {
                name: name.to_string(),
                repo_name,
                registry: Path::new(&raw).parent()?.to_path_buf(),
            });
        }
    }
    None
}

/// Entries of `.git/worktrees/` whose `gitdir` target still exists (pruned ghosts don't count).
pub fn count_live_worktrees(registry: &Path) -> usize {
    let Ok(rd) = fs::read_dir(registry) else {
        return 0;
    };
    rd.flatten()
        .filter(|e| {
            fs::read_to_string(e.path().join("gitdir")).is_ok_and(|g| {
                let g = g.trim();
                !g.is_empty() && Path::new(g).exists()
            })
        })
        .count()
}

/// `("repo-wt-x", "repo") -> "wt-x"` for separators `-`, `_`, `.`; never empties the name.
pub fn strip_repo_prefix(name: &str, anchor: &str) -> String {
    if name.is_empty() || anchor.is_empty() || name == anchor {
        return name.to_string();
    }
    for sep in ["-", "_", "."] {
        let prefix = format!("{anchor}{sep}");
        if name.starts_with(&prefix) && name.len() > prefix.len() {
            return name[prefix.len()..].to_string();
        }
    }
    name.to_string()
}

fn word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '.' || c == '-'
}

/// `git@host:o/r.git`, `ssh://git@host:22/o/r`, `https://user:tok@host/o/r.git` → `https://host/o/r`.
pub fn remote_web_url(remote: &str) -> Option<String> {
    let r = remote.trim();
    let scheme_rest = ["ssh://", "git+ssh://", "https://", "http://"]
        .iter()
        .find_map(|s| r.strip_prefix(s));
    let (host, path) = if let Some(rest) = scheme_rest {
        let slash = rest.find('/')?;
        let authority = &rest[..slash];
        let hostport = match authority.rfind('@') {
            Some(at) => &authority[at + 1..],
            None => authority,
        };
        let (host, port) = match hostport.split_once(':') {
            Some((h, p)) => (h, Some(p)),
            None => (hostport, None),
        };
        if host.is_empty() || !host.chars().all(word_char) {
            return None;
        }
        if let Some(p) = port {
            if p.is_empty() || !p.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
        }
        (host, &rest[slash + 1..])
    } else {
        let at = r.find('@')?;
        let user = &r[..at];
        if user.is_empty() || !user.chars().all(word_char) {
            return None;
        }
        let rest = &r[at + 1..];
        let colon = rest.find(':')?;
        let host = &rest[..colon];
        if host.is_empty() || !host.chars().all(word_char) {
            return None;
        }
        let path = &rest[colon + 1..];
        let digits = path.bytes().take_while(u8::is_ascii_digit).count();
        if digits > 0 && path[digits..].starts_with('/') {
            return None; // the `(?!\d+/)` lookahead: that is a port, not a path
        }
        (host, path)
    };
    let path = path.trim_end_matches('/');
    let path = path.strip_suffix(".git").unwrap_or(path);
    if path.is_empty() || path.contains("..") || !path.chars().all(|c| word_char(c) || c == '/') {
        return None;
    }
    Some(format!("https://{host}/{path}"))
}

/// Python `urllib.parse.quote(branch, safe="/")`.
fn percent_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'_' | b'.' | b'-' | b'~' | b'/' => {
                out.push(b as char);
            }
            _ => {
                let _ = write!(out, "%{b:02X}");
            }
        }
    }
    out
}

/// Branch page on the forge: GitLab `/-/tree/`, Bitbucket `/src/`, else `/tree/`.
pub fn branch_web_url(repo_url: Option<&str>, branch: Option<&str>) -> Option<String> {
    let (url, b) = (repo_url?, branch?);
    if url.is_empty() || b.is_empty() {
        return None;
    }
    let host = url.split('/').nth(2).unwrap_or("");
    let seg = if host.contains("gitlab") {
        "/-/tree/"
    } else if host.contains("bitbucket") {
        "/src/"
    } else {
        "/tree/"
    };
    Some(format!("{url}{seg}{}", percent_encode(b)))
}

/// origin's URL (else the first other remote's) from the common `.git/config`, no git subprocess.
pub fn read_web_url(start: &Path) -> Option<String> {
    let gitdir = resolve_gitdir(start)?;
    let common = match fs::read_to_string(gitdir.join("commondir")) {
        Ok(rel) => gitdir
            .join(rel.trim())
            .canonicalize()
            .unwrap_or_else(|_| gitdir.clone()),
        Err(_) => gitdir.clone(),
    };
    let text = fs::read_to_string(common.join("config")).ok()?;
    let mut origin: Option<String> = None;
    let mut first_other: Option<String> = None;
    let mut section = String::new();
    for line in text.lines() {
        let l = line.trim();
        if l.starts_with('[') && l.ends_with(']') {
            section = l[1..l.len() - 1].to_string();
            continue;
        }
        if !section.starts_with("remote \"") {
            continue;
        }
        if let Some((k, v)) = l.split_once('=') {
            if k.trim() == "url" {
                let v = v.trim().to_string();
                if section == "remote \"origin\"" {
                    origin.get_or_insert(v);
                } else {
                    first_other.get_or_insert(v);
                }
            }
        }
    }
    origin
        .as_deref()
        .and_then(remote_web_url)
        .or_else(|| first_other.as_deref().and_then(remote_web_url))
}

fn non_empty(s: Option<&str>) -> Option<&str> {
    s.filter(|x| !x.is_empty())
}

fn basename(s: &str) -> String {
    let t = s.trim_end_matches('/');
    match t.rsplit('/').next() {
        Some(b) if !b.is_empty() => b.to_string(),
        _ => s.to_string(),
    }
}

pub fn resolve_identity(p: &Payload) -> Identity {
    let repo_name = p
        .workspace
        .repo
        .as_ref()
        .and_then(|r| non_empty(r.name.as_deref()));
    let project_dir = non_empty(p.workspace.project_dir.as_deref());
    let current_dir =
        non_empty(p.workspace.current_dir.as_deref()).or_else(|| non_empty(Some(&p.cwd)));
    let cwd_env = std::env::current_dir()
        .map(|d| d.to_string_lossy().into_owned())
        .unwrap_or_default();
    let start = PathBuf::from(current_dir.or(project_dir).unwrap_or(&cwd_env));

    let head = read_head(&start);
    let toplevel = if head.is_some() {
        resolve_toplevel(&start)
    } else {
        None
    };
    let wt = if head.is_some() {
        parse_worktree(&start)
    } else {
        None
    };
    // Filesystem first, the stdin hint for the rare case the cwd is not on disk (identity.py).
    let stdin_worktree = non_empty(p.workspace.git_worktree.as_deref());
    let is_worktree = wt.is_some() || stdin_worktree.is_some();
    let worktree_name = stdin_worktree.map(str::to_string).or_else(|| {
        wt.as_ref()
            .map(|w| strip_repo_prefix(&w.name, &w.repo_name))
    });
    let worktree_count = match (&wt, &toplevel) {
        (Some(w), _) => count_live_worktrees(&w.registry),
        (None, Some(t)) => count_live_worktrees(&t.join(".git").join("worktrees")),
        _ => 0,
    };
    let project_name = repo_name
        .map(str::to_string)
        .or_else(|| wt.map(|w| w.repo_name))
        .or_else(|| project_dir.map(basename))
        .or_else(|| current_dir.map(basename))
        .unwrap_or_else(|| {
            let b = basename(&cwd_env);
            if b.is_empty() {
                "?".to_string()
            } else {
                b
            }
        });
    let web_url = if head.is_some() {
        read_web_url(&start)
    } else {
        None
    };

    let in_git = head.is_some();
    let detached = head.as_ref().is_some_and(|h| h.detached);
    Identity {
        project_name,
        in_git,
        branch: head.map(|h| h.name),
        detached,
        worktree_name,
        toplevel,
        is_worktree,
        worktree_count,
        web_url,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn head(name: &str, detached: bool) -> Option<Head> {
        Some(Head {
            name: name.to_string(),
            detached,
        })
    }

    fn write(p: &Path, s: &str) {
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, s).unwrap();
    }

    #[test]
    fn head_branch_and_detached() {
        let d = tempfile::tempdir().unwrap();
        let repo = d.path().join("repo");
        write(&repo.join(".git/HEAD"), "ref: refs/heads/feat/x\n");
        fs::create_dir_all(repo.join("sub")).unwrap();
        assert_eq!(read_head(&repo.join("sub/dir")), head("feat/x", false));
        assert_eq!(
            resolve_toplevel(&repo.join("sub")),
            Some(repo.canonicalize().unwrap())
        );
        write(
            &repo.join(".git/HEAD"),
            "0123456789abcdef0123456789abcdef01234567\n",
        );
        assert_eq!(read_head(&repo), head("0123456", true));
        write(&repo.join(".git/HEAD"), "ref: refs/remotes/origin/HEAD\n");
        assert_eq!(read_head(&repo), head("HEAD", false));
        assert_eq!(read_head(d.path()), None);
    }

    #[test]
    fn linked_worktree_detection_and_count() {
        let d = tempfile::tempdir().unwrap();
        let repo = d.path().join("myrepo");
        let wt = d.path().join("myrepo-wt1");
        let ghost = repo.join(".git/worktrees/ghost");
        write(&repo.join(".git/HEAD"), "ref: refs/heads/main\n");
        write(
            &repo.join(".git/worktrees/wt1/HEAD"),
            "ref: refs/heads/topic\n",
        );
        write(
            &repo.join(".git/worktrees/wt1/gitdir"),
            &format!("{}/.git\n", wt.display()),
        );
        write(&repo.join(".git/worktrees/wt1/commondir"), "../..\n");
        write(&ghost.join("gitdir"), "/nonexistent/.git\n");
        write(
            &wt.join(".git"),
            &format!("gitdir: {}/.git/worktrees/wt1\n", repo.display()),
        );

        let w = parse_worktree(&wt.join("deep")).expect("linked worktree");
        assert_eq!(w.name, "wt1");
        assert_eq!(w.repo_name, "myrepo");
        assert_eq!(w.registry, repo.join(".git/worktrees"));
        assert_eq!(count_live_worktrees(&w.registry), 1); // ghost not counted
        assert_eq!(read_head(&wt), head("topic", false));
        assert!(parse_worktree(&repo).is_none());
        assert_eq!(strip_repo_prefix("myrepo-wt1", "myrepo"), "wt1");
        assert_eq!(strip_repo_prefix("myrepo", "myrepo"), "myrepo");
        assert_eq!(strip_repo_prefix("other", "myrepo"), "other");
    }

    #[test]
    fn remote_urls() {
        assert_eq!(
            remote_web_url("git@github.com:exampleco/orbit.git"),
            Some("https://github.com/exampleco/orbit".into())
        );
        assert_eq!(
            remote_web_url("https://user:tok@github.com/o/r.git"),
            Some("https://github.com/o/r".into())
        );
        assert_eq!(
            remote_web_url("ssh://git@gitlab.com:22/grp/sub/r"),
            Some("https://gitlab.com/grp/sub/r".into())
        );
        assert_eq!(
            remote_web_url("https://github.com/o/r/"),
            Some("https://github.com/o/r".into())
        );
        assert_eq!(remote_web_url("git@host:1234/r"), None); // scp form with a leading port-like path
        assert_eq!(remote_web_url("https://h/o/../r"), None);
        assert_eq!(remote_web_url("file:///x"), None);
        assert_eq!(
            branch_web_url(Some("https://github.com/o/r"), Some("feat/a b")),
            Some("https://github.com/o/r/tree/feat/a%20b".into())
        );
        assert_eq!(
            branch_web_url(Some("https://gitlab.com/o/r"), Some("main")),
            Some("https://gitlab.com/o/r/-/tree/main".into())
        );
        assert_eq!(
            branch_web_url(Some("https://bitbucket.org/o/r"), Some("main")),
            Some("https://bitbucket.org/o/r/src/main".into())
        );
        assert_eq!(branch_web_url(None, Some("main")), None);
    }

    #[test]
    fn web_url_from_config_prefers_origin_and_follows_commondir() {
        let d = tempfile::tempdir().unwrap();
        let repo = d.path().join("r");
        write(&repo.join(".git/HEAD"), "ref: refs/heads/main\n");
        write(&repo.join(".git/config"), "[core]\n\tbare = false\n[remote \"up\"]\n\turl = git@h.com:a/b.git\n[remote \"origin\"]\n\turl = git@github.com:x/y.git\n\tfetch = +refs/heads/*:refs/remotes/origin/*\n");
        assert_eq!(read_web_url(&repo), Some("https://github.com/x/y".into()));
        let wt = d.path().join("r-wt");
        write(&repo.join(".git/worktrees/wt/HEAD"), "ref: refs/heads/t\n");
        write(&repo.join(".git/worktrees/wt/commondir"), "../..\n");
        write(
            &wt.join(".git"),
            &format!("gitdir: {}/.git/worktrees/wt\n", repo.display()),
        );
        assert_eq!(read_web_url(&wt), Some("https://github.com/x/y".into()));
    }

    #[test]
    fn resolve_identity_from_payload() {
        let d = tempfile::tempdir().unwrap();
        let repo = d.path().join("proj");
        write(&repo.join(".git/HEAD"), "ref: refs/heads/dev\n");
        fs::create_dir_all(repo.join("src")).unwrap();
        let mut p = crate::payload::Payload::default();
        p.workspace.current_dir = Some(repo.join("src").to_string_lossy().into_owned());
        p.workspace.project_dir = Some(repo.to_string_lossy().into_owned());
        let id = resolve_identity(&p);
        assert_eq!(id.project_name, "proj");
        assert!(id.in_git && !id.is_worktree && !id.detached);
        assert_eq!(id.branch.as_deref(), Some("dev"));
        assert_eq!(id.worktree_count, 0);
        assert_eq!(id.toplevel, Some(repo.canonicalize().unwrap()));
        p.workspace.repo = Some(crate::payload::Repo {
            name: Some("orbit".into()),
        });
        assert_eq!(resolve_identity(&p).project_name, "orbit");
        p.workspace.current_dir = Some(d.path().join("nogit").to_string_lossy().into_owned());
        p.workspace.project_dir = None;
        p.workspace.repo = None;
        let id = resolve_identity(&p);
        assert!(!id.in_git);
        assert_eq!(id.project_name, "nogit");
    }

    #[test]
    fn stdin_git_worktree_is_honoured() {
        let d = tempfile::tempdir().unwrap();
        let repo = d.path().join("proj");
        write(&repo.join(".git/HEAD"), "ref: refs/heads/dev\n");
        let mut p = crate::payload::Payload::default();
        p.workspace.current_dir = Some(repo.to_string_lossy().into_owned());
        p.workspace.git_worktree = Some("feature-x".into());
        let id = resolve_identity(&p);
        assert!(id.is_worktree);
        assert_eq!(id.worktree_name.as_deref(), Some("feature-x"));
        // empty stdin value → filesystem decides
        p.workspace.git_worktree = Some(String::new());
        let id = resolve_identity(&p);
        assert!(!id.is_worktree);
        assert_eq!(id.worktree_name, None);
        // linked worktree on disk: a non-empty stdin value wins over the derived name
        let wt = d.path().join("proj-wt1");
        write(
            &repo.join(".git/worktrees/wt1/HEAD"),
            "ref: refs/heads/topic\n",
        );
        write(
            &repo.join(".git/worktrees/wt1/gitdir"),
            &format!("{}/.git\n", wt.display()),
        );
        write(
            &wt.join(".git"),
            &format!("gitdir: {}/.git/worktrees/wt1\n", repo.display()),
        );
        p.workspace.current_dir = Some(wt.to_string_lossy().into_owned());
        p.workspace.git_worktree = None;
        assert_eq!(resolve_identity(&p).worktree_name.as_deref(), Some("wt1"));
        p.workspace.git_worktree = Some("from-stdin".into());
        assert_eq!(
            resolve_identity(&p).worktree_name.as_deref(),
            Some("from-stdin")
        );
    }
}
