//! Aurora identity line: `⎇ main ●↓1 #42 ✓  ·  project  ·  +244 −3  ·  ⏱ 4m  ·  worktree (🌳1)`
//! (port of styles.render_identity_line_aurora; the PR badge is csr's own). Muted palette:
//! line 1 carries the color.

use std::fmt::Write as _;

use crate::config::Config;
use crate::identity::{branch_web_url, strip_repo_prefix, Identity};
use crate::payload::Pr;
use crate::theme::{blend, fg, Rgb, Theme, ITAL, RESET};

/// Everything line 2 shows: project, branch, git state, activity.
#[derive(Debug, Clone, Copy)]
pub struct Line2Input<'a> {
    pub info: &'a Identity,
    pub dirty: Option<bool>,
    pub ahead: Option<i64>,
    pub behind: Option<i64>,
    pub lines_text: &'a str,
    pub duration_text: &'a str,
    /// The branch's open PR, drawn after the branch; `None` → not shown.
    pub pr: Option<&'a Pr>,
}

const QUIET_SEP: &str = "  ·  ";

/// OSC 8 hyperlink, BEL-terminated (what Claude Code's statusline docs show). Only plain
/// printable http(s) URLs are linked; anything else leaves the text bare.
pub fn hyperlink(text: &str, url: Option<&str>) -> String {
    match url {
        Some(u) if is_safe_url(u) => format!("\x1b]8;;{u}\x07{text}\x1b]8;;\x07"),
        _ => text.to_string(),
    }
}

fn is_safe_url(u: &str) -> bool {
    let rest = u
        .strip_prefix("https://")
        .or_else(|| u.strip_prefix("http://"));
    matches!(rest, Some(r) if !r.is_empty() && u.bytes().all(|b| (0x21..=0x7e).contains(&b)))
}

fn ahead_behind_glyphs(ahead: Option<i64>, behind: Option<i64>) -> String {
    let mut s = String::new();
    if let Some(a) = ahead.filter(|a| *a != 0) {
        let _ = write!(s, "↑{a}");
    }
    if let Some(b) = behind.filter(|b| *b != 0) {
        let _ = write!(s, "↓{b}");
    }
    s
}

/// `#42` (`!42` for a GitLab merge request) linked to the PR, then its review state: `✓`
/// approved, `✗` changes requested, `○` pending, `draft`. `None` without a number.
pub fn pr_badge(t: &Theme, pr: &Pr) -> Option<String> {
    let n = pr.number?;
    let sigil = if pr.kind.as_deref() == Some("mr") {
        '!'
    } else {
        '#'
    };
    let number = format!("{}{sigil}{n}{RESET}", fg(blend(t.pill_ink, t.mute, 0.5)));
    let soft = |c: Rgb| fg(blend(c, t.mute, 0.45));
    let state = match pr.review_state.as_deref() {
        Some("approved") => format!(" {}✓{RESET}", soft(t.s_ok)),
        Some("changes_requested") => format!(" {}✗{RESET}", soft(t.s_hot)),
        Some("pending") => format!(" {}○{RESET}", fg(t.mute)),
        Some("draft") => format!(" {}{ITAL}draft{RESET}", fg(t.mute)),
        _ => String::new(),
    };
    Some(hyperlink(&number, pr.url.as_deref()) + &state)
}

/// Display name of a linked worktree: its name minus a redundant repo prefix, collapsing to
/// `worktree` when unknown or when it would only repeat the repo / branch; `(🌳N)` appended.
pub fn worktree_label(info: &Identity, glyph: &str) -> String {
    let raw = info.worktree_name.as_deref().unwrap_or("").trim();
    let project = info.project_name.trim();
    let mut name = if raw.is_empty() {
        "worktree".to_string()
    } else {
        strip_repo_prefix(raw, project)
    };
    if name == project || Some(name.as_str()) == info.branch.as_deref() {
        name = "worktree".to_string();
    }
    if info.worktree_count > 0 {
        format!("{name} ({glyph}{})", info.worktree_count)
    } else {
        name
    }
}

pub fn render_line2(cfg: &Config, inp: &Line2Input) -> String {
    let t = &cfg.theme;
    let info = inp.info;
    let base = fg(blend(t.pill_ink, t.mute, 0.5));
    let dim = fg(t.mute);
    let edge = fg(t.edge);
    let soft = |c: Rgb| fg(blend(c, t.mute, 0.45));
    let c = |color: &str, text: &str| format!("{color}{text}{RESET}");
    let repo_url = info.web_url.as_deref();

    let mut segs: Vec<String> = Vec::with_capacity(6);
    if info.is_worktree {
        segs.push(c(&soft(t.wt), &worktree_label(info, &cfg.worktree_glyph)));
    }
    if info.in_git {
        let branch = info
            .branch
            .as_deref()
            .filter(|b| !b.is_empty())
            .unwrap_or("?");
        let (styled, url) = if info.detached {
            (c(&format!("{dim}{ITAL}"), branch), None)
        } else {
            (c(&base, branch), branch_web_url(repo_url, Some(branch)))
        };
        let mut head = c(&dim, "⎇ ") + &hyperlink(&styled, url.as_deref());
        let mut flags = String::new();
        if inp.dirty == Some(true) {
            flags.push('●');
        }
        flags.push_str(&ahead_behind_glyphs(inp.ahead, inp.behind));
        if !flags.is_empty() {
            head.push(' ');
            head.push_str(&c(&soft(t.s_warn), &flags));
        }
        if let Some(badge) = inp.pr.and_then(|pr| pr_badge(t, pr)) {
            head.push(' ');
            head.push_str(&badge);
        }
        segs.push(head);
    }
    segs.push(hyperlink(&c(&base, &info.project_name), repo_url));
    if !info.in_git {
        segs.push(c(&format!("{dim}{ITAL}"), "no git"));
    }
    if !inp.lines_text.is_empty() {
        let toks: Vec<String> = inp
            .lines_text
            .split_whitespace()
            .map(|tok| {
                if tok.starts_with('+') {
                    c(&soft(t.s_ok), tok)
                } else if let Some(n) = tok.strip_prefix('-') {
                    c(&soft(t.s_hot), &format!("−{n}"))
                } else {
                    c(&dim, tok)
                }
            })
            .collect();
        segs.push(toks.join(" "));
    }
    if !inp.duration_text.is_empty() {
        segs.push(c(&dim, &format!("⏱ {}", inp.duration_text)));
    }
    if info.in_git && !info.is_worktree && info.worktree_count > 0 {
        segs.push(c(
            &dim,
            &format!("worktree ({}{})", cfg.worktree_glyph, info.worktree_count),
        ));
    }
    segs.join(&c(&edge, QUIET_SEP))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::identity::Identity;

    fn orbit() -> Identity {
        Identity {
            project_name: "orbit".into(),
            in_git: true,
            branch: Some("development".into()),
            detached: false,
            worktree_name: None,
            toplevel: Some("/home/someone/dev/example/orbit".into()),
            is_worktree: false,
            worktree_count: 1,
            web_url: Some("https://github.com/exampleco/orbit".into()),
        }
    }

    #[test]
    fn matches_golden_second_line() {
        let golden = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/golden-orbit.raw"
        ))
        .unwrap();
        let second = golden.lines().nth(1).unwrap();
        let info = orbit();
        let inp = Line2Input {
            info: &info,
            dirty: Some(true),
            ahead: None,
            behind: None,
            lines_text: "+965 -253",
            duration_text: "",
            pr: None,
        };
        assert_eq!(render_line2(&Config::default(), &inp), second);
    }

    #[test]
    fn variants() {
        let c = Config::default();
        let mut info = orbit();
        info.worktree_count = 0;
        info.web_url = None;
        let plain = |s: &str| {
            crate::width::segments(s)
                .iter()
                .filter_map(|seg| match seg {
                    crate::width::Segment::Text(t) => Some(*t),
                    _ => None,
                })
                .collect::<String>()
        };
        let inp = Line2Input {
            info: &info,
            dirty: Some(false),
            ahead: Some(2),
            behind: Some(1),
            lines_text: "",
            duration_text: "5m",
            pr: None,
        };
        assert_eq!(
            plain(&render_line2(&c, &inp)),
            "⎇ development ↑2↓1  ·  orbit  ·  ⏱ 5m"
        );
        // dirty unknown → no dot; detached → italic sha, no link
        info.detached = true;
        info.branch = Some("abc1234".into());
        info.web_url = Some("https://github.com/o/r".into());
        let inp = Line2Input {
            info: &info,
            dirty: None,
            ahead: None,
            behind: None,
            lines_text: "",
            duration_text: "",
            pr: None,
        };
        let out = render_line2(&c, &inp);
        assert_eq!(plain(&out), "⎇ abc1234  ·  orbit");
        assert!(out.contains("\x1b[38;2;120;125;132m\x1b[3mabc1234\x1b[0m"));
        assert!(!out.contains("/tree/"));
        assert!(out.contains("\x1b]8;;https://github.com/o/r\x07"));
        // linked worktree leads the line; main-checkout marker absent
        let wt = Identity {
            is_worktree: true,
            worktree_name: Some("feature-x".into()),
            worktree_count: 2,
            detached: false,
            branch: Some("feature-x".into()),
            ..orbit()
        };
        let inp = Line2Input {
            info: &wt,
            dirty: Some(true),
            ahead: None,
            behind: None,
            lines_text: "",
            duration_text: "",
            pr: None,
        };
        assert_eq!(
            plain(&render_line2(&c, &inp)),
            "worktree (🌳2)  ·  ⎇ feature-x ●  ·  orbit"
        );
        let wt2 = Identity {
            worktree_name: Some("orbit-fix".into()),
            ..wt.clone()
        };
        assert_eq!(worktree_label(&wt2, "🌳"), "fix (🌳2)");
        // no git
        let ng = Identity {
            project_name: "notes".into(),
            ..Identity::default()
        };
        let inp = Line2Input {
            info: &ng,
            dirty: None,
            ahead: None,
            behind: None,
            lines_text: "",
            duration_text: "",
            pr: None,
        };
        assert_eq!(plain(&render_line2(&c, &inp)), "notes  ·  no git");
    }

    #[test]
    fn empty_branch_name_prints_question_mark() {
        let c = Config::default();
        let info = Identity {
            branch: Some(String::new()),
            worktree_count: 0,
            web_url: None,
            ..orbit()
        };
        let plain = |s: &str| {
            crate::width::segments(s)
                .iter()
                .filter_map(|seg| match seg {
                    crate::width::Segment::Text(t) => Some(*t),
                    _ => None,
                })
                .collect::<String>()
        };
        let inp = Line2Input {
            info: &info,
            dirty: None,
            ahead: None,
            behind: None,
            lines_text: "",
            duration_text: "",
            pr: None,
        };
        assert_eq!(plain(&render_line2(&c, &inp)), "⎇ ?  ·  orbit");
    }

    #[test]
    fn pr_badge_follows_the_branch() {
        let c = Config::default();
        let info = Identity {
            worktree_count: 0,
            web_url: None,
            ..orbit()
        };
        let plain = |s: &str| {
            crate::width::segments(s)
                .iter()
                .filter_map(|seg| match seg {
                    crate::width::Segment::Text(t) => Some(*t),
                    _ => None,
                })
                .collect::<String>()
        };
        let pr = |state: Option<&str>, kind: Option<&str>| Pr {
            number: Some(42),
            url: Some("https://github.com/o/r/pull/42".into()),
            review_state: state.map(String::from),
            kind: kind.map(String::from),
        };
        let line = |pr: &Pr| {
            render_line2(
                &c,
                &Line2Input {
                    info: &info,
                    dirty: Some(true),
                    ahead: None,
                    behind: None,
                    lines_text: "",
                    duration_text: "",
                    pr: Some(pr),
                },
            )
        };
        let approved = line(&pr(Some("approved"), None));
        assert_eq!(plain(&approved), "⎇ development ● #42 ✓  ·  orbit");
        assert!(approved.contains("\x1b]8;;https://github.com/o/r/pull/42\x07"));
        assert_eq!(
            plain(&line(&pr(Some("changes_requested"), None))),
            "⎇ development ● #42 ✗  ·  orbit"
        );
        assert_eq!(
            plain(&line(&pr(Some("pending"), Some("mr")))),
            "⎇ development ● !42 ○  ·  orbit"
        );
        assert_eq!(
            plain(&line(&pr(Some("draft"), None))),
            "⎇ development ● #42 draft  ·  orbit"
        );
        assert_eq!(
            plain(&line(&pr(None, None))),
            "⎇ development ● #42  ·  orbit"
        );
        // no number → no badge
        let no_number = Pr {
            number: None,
            ..pr(Some("approved"), None)
        };
        assert_eq!(plain(&line(&no_number)), "⎇ development ●  ·  orbit");
    }
}
