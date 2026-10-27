//! Aurora first line: `5h ■■■ 1% ⏰4h40m →12% │ 7d … │ Model ■■■■■■ 8% │ ❄ 49m` (port of styles.render_aurora).

use crate::config::Config;
use crate::format::{aurora_cache_text, projection_color, round_even, window_severity_rgb};
use crate::theme::{blend, fg, severity, Rgb, Theme, BOLD, RESET};

/// Everything line 1 shows, already formatted by [`crate::render`].
#[derive(Debug, Clone, Copy)]
pub struct Line1Input<'a> {
    pub msgs_pct: Option<f64>,
    pub weekly_pct: Option<f64>,
    pub reset_5h: &'a str,
    pub reset_7d: &'a str,
    pub countdown_emoji: &'a str,
    pub projection_5h: &'a str,
    pub projection_7d: &'a str,
    pub model: &'a str,
    pub ctx_pct: Option<f64>,
    pub cost_text: &'a str,
    pub cache_text: &'a str,
}

/// Gradient bar: each filled cell's hue is fixed by position (ok→warn over the first half,
/// warn→hot over the second); empty cells are edge grey. `None` → all edge.
pub fn bar(cfg: &Config, p: Option<f64>, width: usize) -> String {
    let t = &cfg.theme;
    let cell = if cfg.thin { "━" } else { "■" };
    let edge = fg(t.edge);
    let Some(p) = p else {
        return format!("{edge}{}{RESET}", cell.repeat(width));
    };
    let clamped = p.clamp(0.0, 100.0);
    let mut filled = (clamped / 100.0 * width as f64 + 0.5) as usize;
    if p > 0.0 && filled == 0 {
        filled = 1;
    }
    let mut out = String::new();
    for i in 0..width {
        if i < filled {
            let tt = if width > 1 {
                i as f64 / (width - 1) as f64
            } else {
                0.0
            };
            let rgb = if tt < 0.5 {
                blend(t.s_ok, t.s_warn, tt * 2.0)
            } else {
                blend(t.s_warn, t.s_hot, (tt - 0.5) * 2.0)
            };
            out.push_str(&fg(rgb));
        } else {
            out.push_str(&edge);
        }
        out.push_str(cell);
    }
    out.push_str(RESET);
    out
}

fn pct_text(t: &Theme, p: Option<f64>, rgb: Option<Rgb>) -> String {
    match p {
        None => format!("{}--%{RESET}", fg(t.mute)),
        Some(p) => format!(
            "{}{BOLD}{:>2}%{RESET}",
            fg(rgb.unwrap_or(t.ink)),
            round_even(p)
        ),
    }
}

/// One quota window segment: label, 12-cell bar, percent, ⏰reset(+emoji), optional projection chip.
pub fn window(
    cfg: &Config,
    label: &str,
    p: Option<f64>,
    chip: &str,
    reset: &str,
    emoji: &str,
) -> String {
    let t = &cfg.theme;
    let rgb = window_severity_rgb(t, p, chip, cfg.warning, cfg.critical);
    let reset = if reset.is_empty() { "--" } else { reset };
    let mut out = format!(
        "{mute}{label}{RESET} {} {} {edge}⏰{mute}{reset}{emoji}{RESET}",
        bar(cfg, p, 12),
        pct_text(t, p, rgb),
        mute = fg(t.mute),
        edge = fg(t.edge),
    );
    if !chip.is_empty() {
        out.push(' ');
        out.push_str(&fg(projection_color(t, chip)));
        out.push_str(chip);
        out.push_str(RESET);
    }
    out
}

/// COLD → hot; any `m`/`h` in the text → ok; bare seconds → warn.
pub fn cache_color(t: &Theme, text: &str) -> Rgb {
    if text == "COLD" {
        t.s_hot
    } else if text.contains('m') || text.contains('h') {
        t.s_ok
    } else {
        t.s_warn
    }
}

pub fn render_line1(cfg: &Config, inp: &Line1Input) -> String {
    let t = &cfg.theme;
    let (ink, mute, edge) = (fg(t.ink), fg(t.mute), fg(t.edge));
    let sep = format!("{pad}{edge}│{RESET}{pad}", pad = cfg.density.pad());
    let mut parts: Vec<String> = Vec::with_capacity(5);

    parts.push(window(
        cfg,
        "5h",
        inp.msgs_pct,
        inp.projection_5h,
        inp.reset_5h,
        inp.countdown_emoji,
    ));
    if cfg.show_weekly {
        parts.push(window(
            cfg,
            "7d",
            inp.weekly_pct,
            inp.projection_7d,
            inp.reset_7d,
            "",
        ));
    }

    // A rose between hot and the worktree hue: reads as "the model", never as a severity.
    let accent = fg(blend(t.s_hot, t.wt, 0.5));
    match inp.ctx_pct {
        Some(c) => {
            // Python strips only the `(used/size)` it appended itself; the name's own parentheses
            // stay (`(… context …)` was already removed by render::strip_context_suffix).
            let name = inp.model;
            let rgb = severity(t, c, 70.0, 85.0);
            parts.push(format!(
                "{accent}{name}{RESET} {} {}{}%{RESET}",
                bar(cfg, Some(c), 6),
                fg(rgb),
                round_even(c)
            ));
        }
        None => parts.push(format!("{accent}{}{RESET}", inp.model)),
    }

    if !inp.cost_text.is_empty() {
        parts.push(format!("{mute}$ {ink}{}{RESET}", inp.cost_text));
    }
    if !inp.cache_text.is_empty() {
        parts.push(format!(
            "{}❄ {}{RESET}",
            fg(cache_color(t, inp.cache_text)),
            aurora_cache_text(inp.cache_text)
        ));
    }
    parts.join(&sep)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    const EDGE: &str = "\x1b[38;2;75;80;88m";
    const MUTE: &str = "\x1b[38;2;120;125;132m";

    fn cfg() -> Config {
        Config::default()
    }

    #[test]
    fn bar_gradient_and_fill() {
        let c = cfg();
        // 1% of 12 cells → 1 filled cell in pure ok, 11 edge cells
        let b = bar(&c, Some(1.0), 12);
        assert!(b.starts_with("\x1b[38;2;78;200;91m■\x1b[38;2;75;80;88m■"));
        assert_eq!(b.matches('■').count(), 12);
        assert!(b.ends_with("\x1b[0m"));
        // 16% → int(1.92 + 0.5) = 2 filled; second cell is the blended (106,196,91)
        let b = bar(&c, Some(16.0), 12);
        assert!(b.starts_with("\x1b[38;2;78;200;91m■\x1b[38;2;106;196;91m■\x1b[38;2;75;80;88m■"));
        // None → all edge
        assert_eq!(bar(&c, None, 3), format!("{EDGE}■■■\x1b[0m"));
        // 100% → last cell is pure hot
        assert!(bar(&c, Some(100.0), 12).ends_with("\x1b[38;2;232;116;116m■\x1b[0m"));
        // thin glyph
        let mut t = cfg();
        t.thin = true;
        assert!(bar(&t, Some(50.0), 4).contains('━'));
    }

    #[test]
    fn five_hour_window_matches_golden() {
        let c = cfg();
        let got = window(&c, "5h", Some(1.0), "→12%", "4h40m", "");
        let want = format!(
            "{MUTE}5h\x1b[0m {} \x1b[38;2;78;200;91m\x1b[1m 1%\x1b[0m {EDGE}⏰{MUTE}4h40m\x1b[0m {MUTE}→12%\x1b[0m",
            bar(&c, Some(1.0), 12)
        );
        assert_eq!(got, want);
        // no reset time → "--", no chip → nothing after
        let got = window(&c, "7d", None, "", "", "");
        assert_eq!(
            got,
            format!(
                "{MUTE}7d\x1b[0m {} {MUTE}--%\x1b[0m {EDGE}⏰{MUTE}--\x1b[0m",
                bar(&c, None, 12)
            )
        );
    }

    #[test]
    fn full_line_matches_golden_first_line() {
        let c = cfg();
        let golden = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/golden-orbit.raw"
        ))
        .unwrap();
        let first = golden.lines().next().unwrap();
        let inp = Line1Input {
            msgs_pct: Some(1.0),
            weekly_pct: Some(16.0),
            reset_5h: "4h40m",
            reset_7d: "2d19h",
            countdown_emoji: "",
            projection_5h: "→12%",
            projection_7d: "→32%",
            model: "Fable 5.1",
            ctx_pct: Some(8.0),
            cost_text: "",
            cache_text: "49m08s",
        };
        assert_eq!(render_line1(&c, &inp), first);
    }

    #[test]
    fn model_without_context_and_cost_segment() {
        let mut c = cfg();
        c.show_weekly = false;
        let inp = Line1Input {
            msgs_pct: None,
            weekly_pct: None,
            reset_5h: "--",
            reset_7d: "",
            countdown_emoji: "",
            projection_5h: "",
            projection_7d: "",
            model: "Opus 5.5 (1M context)",
            ctx_pct: None,
            cost_text: "1.53",
            cache_text: "COLD",
        };
        let out = render_line1(&c, &inp);
        assert!(out.contains("\x1b[38;2;200;134;171mOpus 5.5 (1M context)\x1b[0m"));
        assert!(out.contains(&format!("{MUTE}$ \x1b[38;2;218;221;225m1.53\x1b[0m")));
        assert!(out.ends_with("\x1b[38;2;232;116;116m❄ COLD\x1b[0m"));
        assert_eq!(out.matches('│').count(), 3); // 4 parts (5h, model, cost, cache) → 3 separators
    }

    #[test]
    fn model_own_parentheses_survive_next_to_the_ctx_bar() {
        let c = cfg();
        let inp = Line1Input {
            msgs_pct: None,
            weekly_pct: None,
            reset_5h: "--",
            reset_7d: "",
            countdown_emoji: "",
            projection_5h: "",
            projection_7d: "",
            model: "Opus (beta)",
            ctx_pct: Some(10.0),
            cost_text: "",
            cache_text: "",
        };
        let out = render_line1(&c, &inp);
        assert!(
            out.contains(&format!(
                "\x1b[38;2;200;134;171mOpus (beta)\x1b[0m {}",
                bar(&c, Some(10.0), 6)
            )),
            "{out:?}"
        );
    }
}
