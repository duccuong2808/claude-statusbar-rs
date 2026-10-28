//! payload + config + environment → the final status text (port of core.main's render branch).

use std::path::PathBuf;

use crate::config::Config;
use crate::format::{
    cache_remaining, countdown_emoji, format_duration_short, format_lines, projection_chip,
    reset_5h, reset_7d, Reset5h, FIVE_HOUR_S, MIN_ELAPSED_5H, MIN_ELAPSED_7D, SEVEN_DAY_S,
};
use crate::gitcache::dirty_with_async_refresh;
use crate::identity::{resolve_identity, Identity};
use crate::line1::{render_line1, Line1Input};
use crate::line2::{render_line2, Line2Input};
use crate::payload::{Payload, Window};
use crate::quota::{reconcile, Reading};
use crate::width::{clip_lines, visible_width};

/// Both lines drawn with made-up values that use every colour of `cfg.theme` (a warm and a
/// hot bar, a dirty tree, a linked worktree): what `csr themes` shows for each theme.
pub fn theme_preview(cfg: &Config) -> String {
    let line1 = render_line1(
        cfg,
        &Line1Input {
            msgs_pct: Some(45.0),
            weekly_pct: Some(82.0),
            reset_5h: "2h10m",
            reset_7d: "1d4h",
            countdown_emoji: "",
            projection_5h: "→61%",
            projection_7d: "→97%",
            model: "Fable 5.1",
            effort: if cfg.show_effort { "high" } else { "" },
            ctx_pct: Some(23.0),
            cost_text: "",
            cache_text: "4m",
        },
    );
    let info = Identity {
        project_name: "my-app".to_string(),
        in_git: true,
        branch: Some("main".to_string()),
        worktree_count: 1,
        ..Identity::default()
    };
    let line2 = render_line2(
        cfg,
        &Line2Input {
            info: &info,
            dirty: Some(true),
            ahead: Some(1),
            behind: None,
            lines_text: &format_lines(42, 7),
            duration_text: "",
        },
    );
    format!("{line1}\n{line2}")
}

/// The process-level inputs to a render: clock, cache location and terminal width.
#[derive(Debug, Clone, PartialEq)]
pub struct Env {
    pub now: f64,
    pub cache_dir: PathBuf,
    pub columns: Option<usize>,
}

/// core.py `re.sub(r'\s*\([^)]*context[^)]*\)', '', model)`.
pub fn strip_context_suffix(model: &str) -> String {
    let mut out = String::with_capacity(model.len());
    let mut rest = model;
    while let Some(open) = rest.find('(') {
        let Some(close_rel) = rest[open..].find(')') else {
            break;
        };
        let inner = &rest[open + 1..open + close_rel];
        if inner.contains("context") {
            out.push_str(rest[..open].trim_end());
            rest = &rest[open + close_rel + 1..];
        } else {
            out.push_str(&rest[..=(open + close_rel)]);
            rest = &rest[open + close_rel + 1..];
        }
    }
    out.push_str(rest);
    out
}

/// One quota window as displayed.
#[derive(Clone, Copy)]
struct Quota {
    pct: Option<f64>,
    resets_at: Option<f64>,
}

impl Quota {
    /// The shared store's reading when it has one, else this stdin window verbatim (Python
    /// trusts fresh stdin the same way).
    fn new(merged: Option<Reading>, stdin: Option<&Window>) -> Self {
        match (merged, stdin.filter(|w| w.is_present())) {
            (Some(r), _) => Quota {
                pct: Some(r.used_pct),
                resets_at: Some(r.resets_at),
            },
            (None, Some(w)) => Quota {
                pct: Some(w.pct()),
                resets_at: w.reset_time(),
            },
            (None, None) => Quota {
                pct: None,
                resets_at: None,
            },
        }
    }
}

/// A stdin window as a shareable reading (needs a reset time; a window without one is still
/// displayed verbatim through [`Quota::new`], it just never enters the shared store).
fn reading(w: Option<&Window>, now: f64) -> Option<Reading> {
    let w = w.filter(|w| w.is_present())?;
    Some(Reading {
        used_pct: w.pct(),
        resets_at: w.reset_time()?,
        observed_at: now,
    })
}

/// The 5h and 7d windows after sharing the freshest reading with every other open window.
fn quotas(p: &Payload, env: &Env) -> (Quota, Quota) {
    let in5 = p.rate_limits.as_ref().and_then(|r| r.five_hour.as_ref());
    let in7 = p.rate_limits.as_ref().and_then(|r| r.seven_day.as_ref());
    let (m5, m7) = reconcile(
        &env.cache_dir,
        env.now,
        reading(in5, env.now),
        reading(in7, env.now),
    );
    (Quota::new(m5, in5), Quota::new(m7, in7))
}

/// core.get_current_model + `display_name if display_name != 'Unknown' else model_id`, and the
/// context percentage (the `(… context)` suffix is dropped only when a context bar is shown).
fn model_and_context(p: &Payload) -> (String, Option<f64>) {
    let id = if p.model.id.is_empty() {
        "unknown"
    } else {
        p.model.id.as_str()
    };
    let display = match p.model.display_name.as_str() {
        "" | "Unknown" => id.to_string(),
        d => d.to_string(),
    };
    match &p.context_window {
        Some(cw) if cw.context_window_size > 0.0 => (strip_context_suffix(&display), cw.ctx_pct()),
        _ => (display, None),
    }
}

/// Python: `prompt_cache.expires_at` → countdown; otherwise a transcript scan, which for a
/// session that has a transcript but no assistant turn yet says "COLD" (and nothing at all
/// without a transcript path). We don't scan transcripts, so "COLD" is that fallback.
fn cache_text(p: &Payload, cfg: &Config, env: &Env) -> String {
    if !cfg.show_cache_age {
        return String::new();
    }
    match p.prompt_cache.as_ref().and_then(|c| c.expires_at) {
        Some(exp) => cache_remaining(exp - env.now),
        None if !p.transcript_path.is_empty() => "COLD".to_string(),
        None => String::new(),
    }
}

/// `effort.level` when shown; empty when switched off or the model has no effort parameter.
fn effort_text(p: &Payload, cfg: &Config) -> String {
    if !cfg.show_effort {
        return String::new();
    }
    p.effort
        .as_ref()
        .and_then(|e| e.level.as_deref())
        .unwrap_or_default()
        .trim()
        .to_string()
}

/// Line 1's optional segments; an empty string means "not shown".
struct Optional {
    cost: String,
    cache: String,
    effort: String,
    proj5: String,
    proj7: String,
}

impl Optional {
    /// narrow_layout "drop": shed one segment, least important first. False when none is left.
    fn shed(&mut self) -> bool {
        if !self.cache.is_empty() {
            self.cache.clear();
        } else if !self.cost.is_empty() {
            self.cost.clear();
        } else if !self.effort.is_empty() {
            self.effort.clear();
        } else if !(self.proj5.is_empty() && self.proj7.is_empty()) {
            self.proj5.clear();
            self.proj7.clear();
        } else {
            return false;
        }
        true
    }
}

/// Both status lines for one payload, clipped to `env.columns`.
///
/// Side effects: merges the rate-limit readings into the shared quota store and may spawn a
/// detached `csr git-refresh` when the git cache is stale. Never blocks on git.
pub fn render(p: &Payload, cfg: &Config, env: &Env) -> String {
    let mut out = first_line(p, cfg, env);
    if cfg.show_project_branch {
        out.push('\n');
        out.push_str(&second_line(p, cfg, env));
    }
    clip_lines(&out, env.columns)
}

fn first_line(p: &Payload, cfg: &Config, env: &Env) -> String {
    let (q5, q7) = quotas(p, env);
    let has_official = q5.pct.is_some() || q7.pct.is_some();
    let reset5 = if has_official {
        reset_5h(q5.resets_at, env.now)
    } else {
        Reset5h::unknown()
    };
    let reset_7d_text = if has_official {
        reset_7d(q7.resets_at, env.now)
    } else {
        String::new()
    };
    let emoji = if has_official {
        countdown_emoji(reset5.minutes)
    } else {
        ""
    };
    let show_projection = has_official && cfg.show_projection;
    let projection = |len, min_elapsed, q: Quota| {
        if show_projection {
            projection_chip(len, min_elapsed, q.pct, q.resets_at, env.now)
        } else {
            String::new()
        }
    };
    let (model, ctx_pct) = model_and_context(p);
    let mut optional = Optional {
        cost: if cfg.show_cost && p.cost.total_cost_usd >= 0.0 {
            format!("{:.2}", p.cost.total_cost_usd)
        } else {
            String::new()
        },
        cache: cache_text(p, cfg, env),
        effort: effort_text(p, cfg),
        proj5: projection(FIVE_HOUR_S, MIN_ELAPSED_5H, q5),
        proj7: projection(SEVEN_DAY_S, MIN_ELAPSED_7D, q7),
    };

    let line = |o: &Optional| {
        render_line1(
            cfg,
            &Line1Input {
                msgs_pct: q5.pct,
                weekly_pct: q7.pct,
                reset_5h: &reset5.text,
                reset_7d: &reset_7d_text,
                countdown_emoji: emoji,
                projection_5h: &o.proj5,
                projection_7d: &o.proj7,
                model: &model,
                effort: &o.effort,
                ctx_pct,
                cost_text: &o.cost,
                cache_text: &o.cache,
            },
        )
    };
    let mut first = line(&optional);
    if let Some(max) = env.columns {
        while visible_width(&first) > max && optional.shed() {
            first = line(&optional);
        }
    }
    first
}

fn second_line(p: &Payload, cfg: &Config, env: &Env) -> String {
    let info = resolve_identity(p);
    let (dirty, entry) = match &info.toplevel {
        Some(t) => dirty_with_async_refresh(&env.cache_dir, &t.to_string_lossy(), env.now),
        None => (None, None),
    };
    let (ahead, behind) = match entry {
        Some(e) if cfg.show_ahead_behind => (e.ahead, e.behind),
        _ => (None, None),
    };
    let lines_text = if cfg.show_lines {
        format_lines(
            p.cost.total_lines_added as i64,
            p.cost.total_lines_removed as i64,
        )
    } else {
        String::new()
    };
    let duration_text = if cfg.show_duration {
        format_duration_short(p.cost.total_duration_ms as i64)
    } else {
        String::new()
    };
    render_line2(
        cfg,
        &Line2Input {
            info: &info,
            dirty,
            ahead,
            behind,
            lines_text: &lines_text,
            duration_text: &duration_text,
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_context_suffix_anywhere() {
        assert_eq!(
            strip_context_suffix("Sonnet 4.5 (1M context)"),
            "Sonnet 4.5"
        );
        assert_eq!(strip_context_suffix("Opus 5.5"), "Opus 5.5");
        assert_eq!(strip_context_suffix("X (200k context) Y"), "X Y");
        assert_eq!(strip_context_suffix("X (beta)"), "X (beta)");
    }

    #[test]
    fn waiting_payload_renders_dashes_and_no_chips() {
        let p: crate::payload::Payload = serde_json::from_str(
            r#"{"model":{"display_name":"Opus 5.5"},"workspace":{"current_dir":"/nonexistent/x"}}"#,
        )
        .unwrap();
        let cfg = crate::config::Config::default();
        let home = tempfile::tempdir().unwrap();
        let env = Env {
            now: 1000.0,
            cache_dir: home.path().to_path_buf(),
            columns: None,
        };
        let out = render(&p, &cfg, &env);
        let plain = strip_ansi(&out);
        assert_eq!(
            plain,
            "5h ■■■■■■■■■■■■ --% ⏰-- │ 7d ■■■■■■■■■■■■ --% ⏰-- │ Opus 5.5\nx  ·  no git"
        );
    }

    #[test]
    fn degrades_cache_then_cost_then_effort_then_chips_before_clipping() {
        let raw = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/stdin-orbit.json"
        ))
        .unwrap();
        let p: crate::payload::Payload = serde_json::from_str(&raw).unwrap();
        let cfg = crate::config::Config {
            show_project_branch: false,
            show_cost: true,
            ..Default::default()
        };
        let now: f64 = 1793060340.0479221; // golden-orbit.now as f64
        let home = tempfile::tempdir().unwrap();
        let env = |columns: Option<usize>| Env {
            now,
            cache_dir: home.path().to_path_buf(),
            columns,
        };
        let full = strip_ansi(&render(&p, &cfg, &env(None)));
        assert!(full.contains("❄") && full.contains("$ 1.53") && full.contains("→"));
        assert!(full.contains("Fable 5.1 xhigh ■"));
        let w = crate::width::visible_width(&full);
        let no_cache = strip_ansi(&render(&p, &cfg, &env(Some(w - 1))));
        assert!(!no_cache.contains("❄") && no_cache.contains("$ 1.53"));
        let no_cost = strip_ansi(&render(&p, &cfg, &env(Some(w - 10))));
        assert!(!no_cost.contains("$ ") && no_cost.contains("xhigh"));
        let no_effort = strip_ansi(&render(&p, &cfg, &env(Some(w - 20))));
        assert!(!no_effort.contains("xhigh") && no_effort.contains("→"));
        let no_chips = strip_ansi(&render(&p, &cfg, &env(Some(w - 26))));
        assert!(!no_chips.contains("→"));
        let clipped = strip_ansi(&render(&p, &cfg, &env(Some(20))));
        assert_eq!(crate::width::visible_width(&clipped), 20);
        assert!(clipped.ends_with('…'));
    }

    #[test]
    fn effort_follows_the_payload_and_the_switch() {
        let home = tempfile::tempdir().unwrap();
        let env = Env {
            now: 1000.0,
            cache_dir: home.path().to_path_buf(),
            columns: None,
        };
        let first_line = |cfg: &crate::config::Config, json: &str| {
            let p: crate::payload::Payload = serde_json::from_str(json).unwrap();
            strip_ansi(&render(&p, cfg, &env))
                .lines()
                .next()
                .unwrap()
                .to_string()
        };
        let on = crate::config::Config::default();
        let off = crate::config::Config {
            show_effort: false,
            ..Default::default()
        };
        let with = r#"{"cwd":"/nonexistent/x","model":{"display_name":"Opus 5.5"},"effort":{"level":"max"}}"#;
        assert!(first_line(&on, with).ends_with(" │ Opus 5.5 max"));
        assert!(first_line(&off, with).ends_with(" │ Opus 5.5"));
        // no effort, a null level or an empty one → nothing after the name
        for json in [
            r#"{"cwd":"/nonexistent/x","model":{"display_name":"Opus 5.5"}}"#,
            r#"{"cwd":"/nonexistent/x","model":{"display_name":"Opus 5.5"},"effort":{"level":null}}"#,
            r#"{"cwd":"/nonexistent/x","model":{"display_name":"Opus 5.5"},"effort":{"level":""}}"#,
        ] {
            assert!(first_line(&on, json).ends_with(" │ Opus 5.5"), "{json}");
        }
    }

    #[test]
    fn new_session_without_rate_limits_is_backfilled_from_the_shared_store() {
        let home = tempfile::tempdir().unwrap();
        let now: f64 = 1793060340.0479221; // golden-orbit.now as f64
        let cfg = crate::config::Config {
            show_project_branch: false,
            ..Default::default()
        };
        // a render with quota fills the store …
        let raw = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/stdin-orbit.json"
        ))
        .unwrap();
        let p: crate::payload::Payload = serde_json::from_str(&raw).unwrap();
        render(
            &p,
            &cfg,
            &Env {
                now,
                cache_dir: home.path().to_path_buf(),
                columns: None,
            },
        );
        // … and a fresh session with no rate_limits shows the shared numbers instead of `--`
        let p2: crate::payload::Payload =
            serde_json::from_str(r#"{"model":{"display_name":"Opus 5.5"}}"#).unwrap();
        let out = strip_ansi(&render(
            &p2,
            &cfg,
            &Env {
                now: now + 2.0,
                cache_dir: home.path().to_path_buf(),
                columns: None,
            },
        ));
        assert!(out.starts_with("5h ■■■■■■■■■■■■  1% ⏰4h40m →"), "{out}");
        assert!(out.contains("7d ■■■■■■■■■■■■ 16% ⏰2d19h →"), "{out}");
    }

    #[test]
    fn nulls_and_wrong_types_do_not_break_the_render() {
        let cfg = crate::config::Config::default();
        let inputs = [
            r#"{"cwd":"/nonexistent/x","workspace":null}"#,
            r#"{"cwd":"/nonexistent/x","cost":null}"#,
            r#"{"cwd":"/nonexistent/x","cost":{"total_duration_ms":1234.0}}"#,
            r#"{"cwd":"/nonexistent/x","rate_limits":{"five_hour":{"used_percentage":1,"resets_at":"1793077200"}}}"#,
            r#"{"cwd":"/nonexistent/x","session_id":null}"#,
        ];
        for inp in inputs {
            let p: crate::payload::Payload =
                serde_json::from_str(inp).unwrap_or_else(|e| panic!("{inp}: {e}"));
            let home = tempfile::tempdir().unwrap();
            let out = strip_ansi(&render(
                &p,
                &cfg,
                &Env {
                    now: 1000.0,
                    cache_dir: home.path().to_path_buf(),
                    columns: None,
                },
            ));
            assert!(out.starts_with("5h "), "{inp}: {out}");
            assert_eq!(out.lines().count(), 2, "{inp}: {out}");
        }
        let p: crate::payload::Payload = serde_json::from_str(
            r#"{"cwd":"/nonexistent/x","cost":{"total_duration_ms":1234.0,"total_lines_added":3}}"#,
        )
        .unwrap();
        let home = tempfile::tempdir().unwrap();
        let out = strip_ansi(&render(
            &p,
            &cfg,
            &Env {
                now: 1000.0,
                cache_dir: home.path().to_path_buf(),
                columns: None,
            },
        ));
        assert_eq!(out.lines().nth(1), Some("x  ·  no git  ·  +3"), "{out}");
    }

    #[test]
    fn empty_window_objects_count_as_absent() {
        let cfg = crate::config::Config::default();
        let home = tempfile::tempdir().unwrap();
        let env = Env {
            now: 1000.0,
            cache_dir: home.path().to_path_buf(),
            columns: None,
        };
        let p: crate::payload::Payload = serde_json::from_str(
            r#"{"cwd":"/nonexistent/x","rate_limits":{"five_hour":{},"seven_day":{}}}"#,
        )
        .unwrap();
        let out = strip_ansi(&render(&p, &cfg, &env));
        assert!(
            out.starts_with("5h ■■■■■■■■■■■■ --% ⏰-- │ 7d ■■■■■■■■■■■■ --% ⏰-- │ "),
            "{out}"
        );
        let p: crate::payload::Payload = serde_json::from_str(
            r#"{"cwd":"/nonexistent/x","rate_limits":{"five_hour":{},"seven_day":{"used_percentage":16,"resets_at":1793304000}}}"#,
        )
        .unwrap();
        let env = Env {
            now: 1793060340.0,
            ..env
        };
        let out = strip_ansi(&render(&p, &cfg, &env));
        assert!(
            out.starts_with("5h ■■■■■■■■■■■■ --% ⏰-- │ 7d ■■■■■■■■■■■■ 16% ⏰2d19h"),
            "{out}"
        );
    }

    #[test]
    fn model_fallback_matches_get_current_model() {
        let cfg = crate::config::Config::default();
        let home = tempfile::tempdir().unwrap();
        let env = Env {
            now: 1000.0,
            cache_dir: home.path().to_path_buf(),
            columns: None,
        };
        let model_of = |json: &str| {
            let p: crate::payload::Payload = serde_json::from_str(json).unwrap();
            let out = strip_ansi(&render(&p, &cfg, &env));
            out.lines()
                .next()
                .unwrap()
                .rsplit(" │ ")
                .next()
                .unwrap()
                .to_string()
        };
        assert_eq!(model_of(r#"{"cwd":"/nonexistent/x"}"#), "unknown");
        assert_eq!(
            model_of(r#"{"cwd":"/nonexistent/x","model":{"id":"claude-x"}}"#),
            "claude-x"
        );
        assert_eq!(
            model_of(
                r#"{"cwd":"/nonexistent/x","model":{"id":"claude-x","display_name":"Unknown"}}"#
            ),
            "claude-x"
        );
        assert_eq!(
            model_of(r#"{"cwd":"/nonexistent/x","model":{"display_name":"Unknown"}}"#),
            "unknown"
        );
        assert_eq!(
            model_of(
                r#"{"cwd":"/nonexistent/x","model":{"id":"claude-x","display_name":"Fable 5.1"}}"#
            ),
            "Fable 5.1"
        );
    }

    #[test]
    fn missing_prompt_cache_renders_cold_like_python() {
        // core.get_cache_age_text: no expires_at → transcript scan; a session with a transcript
        // but no assistant turn yet reads "COLD"; no transcript path at all → no segment.
        let cfg = crate::config::Config::default();
        let home = tempfile::tempdir().unwrap();
        let env = Env {
            now: 1000.0,
            cache_dir: home.path().to_path_buf(),
            columns: None,
        };
        let first_line = |json: &str| {
            let p: crate::payload::Payload = serde_json::from_str(json).unwrap();
            strip_ansi(&render(&p, &cfg, &env))
                .lines()
                .next()
                .unwrap()
                .to_string()
        };
        assert!(
            first_line(r#"{"cwd":"/nonexistent/x","transcript_path":"/nonexistent/t.jsonl"}"#)
                .ends_with(" │ ❄ COLD")
        );
        assert!(first_line(r#"{"cwd":"/nonexistent/x","transcript_path":"/nonexistent/t.jsonl","prompt_cache":{"warm":false}}"#).ends_with(" │ ❄ COLD"));
        assert!(!first_line(r#"{"cwd":"/nonexistent/x"}"#).contains('❄'));
        assert!(first_line(r#"{"cwd":"/nonexistent/x","transcript_path":"/nonexistent/t.jsonl","prompt_cache":{"expires_at":3948.0}}"#).ends_with(" │ ❄ 49m"));
    }

    fn strip_ansi(s: &str) -> String {
        crate::width::segments(s)
            .iter()
            .filter_map(|seg| match seg {
                crate::width::Segment::Text(t) => Some(*t),
                _ => None,
            })
            .collect()
    }
}
