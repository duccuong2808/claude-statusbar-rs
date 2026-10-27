# csr — Rust port of the claude-statusbar status line — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A single static Rust binary `csr` that Claude Code can run as its `statusLine` command and that renders, byte-for-byte, the same two lines the Python `claude-statusbar` 3.52.2 renders today (style `aurora`, theme `graphite`), in under 10 ms wall time and under 10 MB RSS, with no daemon.

**Architecture:** One process per render: read the JSON Claude Code pipes on stdin, read a few small files (`.git/HEAD`, `.git/worktrees/*`, `.git/config`, a 30-second dirty-state cache), format two ANSI lines, print, exit. The only slow thing (`git status`) never runs on the render path: a stale cache spawns a detached `csr git-refresh <toplevel>` child that writes the cache for the next render. No network, no transcript scanning, no cross-session quota store.

**Tech Stack:** Rust 2021 (cargo 1.99 at `~/.cargo/bin/cargo`, installed with `--no-modify-path`, so every shell must `export PATH="$HOME/.cargo/bin:$PATH"`), crates `serde` + `serde_json` (stdin and config parsing), `unicode-width` (terminal cell widths), dev-only `tempfile`. Nothing else.

---

## Read this first (context the Python source gives and this plan condenses)

- **Project dir:** `/home/someone/dev/projects/claude-statusbar-rs/` (already contains `tests/fixtures/` and this plan). Crate name `claude-statusbar-rs`, library name `csr`, binary name `csr`.
- **Reference implementation (read-only, never edit):** `/home/someone/.local/share/uv/tools/claude-statusbar/lib/python3.12/site-packages/claude_statusbar/`. The files that matter: `styles.py` (`render_aurora`, `render_identity_line_aurora`), `themes.py` (graphite palette), `progress.py` (`_blend`, `window_severity_rgb`, `_projection_color`, `get_countdown_emoji`), `core.py` (`parse_stdin_data`, `main`, reset-time formatting around line 1470), `activity.py` (`format_cache_remaining`, `format_lines`, `format_duration_short`), `identity.py` (git identity), `git_cache.py` + `_git_refresh.py` (dirty cache), `_display.py` (width/clipping), `predict.py` (`_format_projection_pct`, `_format_eta`, `_depletion_eta_seconds`). Every formula below was transcribed from those files; when in doubt, the Python is the spec.
- **Fixtures (already captured, do not regenerate unless told):**
  - `tests/fixtures/stdin-orbit.json` — the JSON Claude Code 2.1.311 sends on stdin (session in `/home/someone/dev/example/orbit`, branch `development`, repo dirty, one linked worktree).
  - `tests/fixtures/golden-orbit.raw` — what Python rendered for that stdin at the instant in `tests/fixtures/golden-orbit.now` (`1793060340.047922117`), with `COLUMNS=182`, an empty `HOME`, and the `✦ New looks` hint line removed. Plain-text view:
    ```
    5h ■■■■■■■■■■■■  1% ⏰4h40m →12% │ 7d ■■■■■■■■■■■■ 16% ⏰2d19h →32% │ Fable 5.1 ■■■■■■ 8% │ ❄ 49m
    ⎇ development ●  ·  orbit  ·  +965 −253  ·  worktree (🌳1)
    ```
  - The golden test fabricates a synthetic repo with the captured git facts in a temp dir instead of reading the live orbit checkout (Task 10 was amended on 2026-10-27).
- **Deliberate deviations from Python (decided, do not "fix"):**
  1. The `→NN%` end-of-window projection uses the plain average-pace formula (`used × window_len ÷ elapsed`), not Python's 1000-line smoothed, cross-session predictor. The golden test masks the chip's digits. The `⚠~eta` forecast chip is unreachable under that rule and is not implemented.
  2. Quota readings are shared across open Claude Code windows through one small file
     (`~/.cache/csr/rate_latest.json`, Task 6b) with a simple merge rule (later reset wins; same
     reset keeps the higher percentage). Python's extra machinery (per-account keying after
     `/login`, 120 s downgrade grace when limits are raised, frozen-session signatures) is not
     ported. Both windows missing everywhere → bars render as `--`.
  3. `❄` cache segment comes only from `prompt_cache.expires_at`. If the field is absent the segment is omitted (Python would scan the transcript).
  4. Segments never shown: language coach, relay balance, `⚠ BYPASS`, ocs line, IP/fingerprint risk lines, mode line, activity/todo lines, version/update hint, `New looks` hint, `cwd`.
- **Hard rules for the implementer:** never edit `~/.claude/settings.json`; never stop, restart or talk to the running Python daemon (`cs daemon …`); never write outside the project dir, `~/.cargo`, and temp dirs; every commit in the project repo only.
- **Time:** every function that needs "now" takes `now: f64` (Unix seconds). `main` reads env `CSR_NOW` (float) if set, else the system clock. Tests always pass a fixed `now`.
- **Python quirks to reproduce exactly:** `int()` truncates toward zero; `round()` and `f"{x:.0f}"` round half to even (`f64::round_ties_even`); `_blend` truncates each channel; the `→100%·1h12m` chip is colored *mute* because Python's `int("100%·1h12m")` fails.

---

## File structure

```
claude-statusbar-rs/
├── Cargo.toml
├── .gitignore
├── README.md                      (Task 11)
├── src/
│   ├── lib.rs                     module list only
│   ├── main.rs                    arg dispatch: render | git-refresh <toplevel> | --version
│   ├── theme.rs                   Rgb, graphite palette, blend(), fg(), SGR consts
│   ├── format.rs                  pure string formatters: reset times, emoji, cache countdown,
│   │                              lines/duration, projection chip, severity colors
│   ├── payload.rs                 serde structs for the stdin JSON
│   ├── config.rs                  subset of ~/.claude/claude-statusbar.json → Config
│   ├── width.rs                   ANSI-aware visible_width / clip_line / clip_lines
│   ├── identity.rs                .git/HEAD, worktree detection, worktree count, remote URL
│   ├── gitcache.rs                30 s dirty cache + detached refresh + `git status` parser
│   ├── quota.rs                   shared freshest 5h/7d reading across windows (rate_latest.json)
│   ├── line1.rs                   the quota/model/cache line (aurora)
│   ├── line2.rs                   the branch/project line (aurora identity line)
│   └── render.rs                  glue: payload + config + env → final string
└── tests/
    ├── fixtures/                  (exists) stdin-orbit.json, golden-orbit.raw, golden-orbit.now
    └── golden.rs                  runs the built binary against the fixture
```

Each `src/*.rs` module owns its unit tests in a `#[cfg(test)] mod tests` block at the bottom.

---

### Task 0: Scaffold the crate

**Files:**
- Create: `Cargo.toml`, `.gitignore`, `src/lib.rs`, `src/main.rs`

- [ ] **Step 1: Write Cargo.toml**

```toml
[package]
name = "claude-statusbar-rs"
version = "0.1.0"
edition = "2021"
description = "Fast single-binary Claude Code status line (aurora/graphite look of claude-statusbar)"

[lib]
name = "csr"
path = "src/lib.rs"

[[bin]]
name = "csr"
path = "src/main.rs"

[dependencies]
serde = { version = "1", features = ["derive"] }
serde_json = "1"
unicode-width = "0.2"

[dev-dependencies]
tempfile = "3"

[profile.release]
opt-level = 3
lto = "fat"
codegen-units = 1
panic = "abort"
strip = true
```

- [ ] **Step 2: Write .gitignore, lib.rs, main.rs stubs**

`.gitignore`:
```
/target
```

`src/lib.rs`:
```rust
pub mod theme;
pub mod format;
pub mod payload;
pub mod config;
pub mod width;
pub mod identity;
pub mod gitcache;
pub mod quota;
pub mod line1;
pub mod line2;
pub mod render;
```

For now create each listed module as an empty file (`touch src/theme.rs …`) so the crate compiles; later tasks fill them.

`src/main.rs` (temporary):
```rust
fn main() {
    println!("csr stub");
}
```

- [ ] **Step 3: Build and init git**

Run:
```bash
cd /home/someone/dev/projects/claude-statusbar-rs
export PATH="$HOME/.cargo/bin:$PATH"
cargo build 2>&1 | tail -3
git init -b main
git add -A
git commit -m "chore: scaffold csr crate"
```
Expected: `Finished` line from cargo; one commit.

---

### Task 1: theme.rs — palette, blend, SGR constants

**Files:**
- Create: `src/theme.rs`

- [ ] **Step 1: Write the failing tests**

```rust
// src/theme.rs
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blend_truncates_like_python_int() {
        // second bar cell of a 12-cell bar: t = 1/11, blend(ok, warn, 2t)
        assert_eq!(blend(GRAPHITE.s_ok, GRAPHITE.s_warn, 2.0 / 11.0), (106, 196, 91));
        assert_eq!(blend(GRAPHITE.s_hot, GRAPHITE.wt, 0.5), (200, 134, 171)); // model accent
        assert_eq!(blend(GRAPHITE.pill_ink, GRAPHITE.mute, 0.5), (179, 180, 178)); // line-2 BASE
        assert_eq!(blend(GRAPHITE.s_warn, GRAPHITE.mute, 0.45), (181, 154, 112)); // soft(warn) = ● color
        assert_eq!(blend(GRAPHITE.s_ok, GRAPHITE.mute, 0.45), (96, 166, 109)); // soft(ok) = +lines
        assert_eq!(blend(GRAPHITE.s_hot, GRAPHITE.mute, 0.45), (181, 120, 123)); // soft(hot) = −lines
    }

    #[test]
    fn fg_is_truecolor_sgr() {
        assert_eq!(fg((120, 125, 132)), "\x1b[38;2;120;125;132m");
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --lib theme 2>&1 | tail -5`
Expected: compile error (`blend`, `GRAPHITE`, `fg` not found).

- [ ] **Step 3: Implement**

```rust
// src/theme.rs
//! Palette + SGR helpers. Only the graphite theme exists (Python 3.50 removed the rest).

pub type Rgb = (u8, u8, u8);

pub const RESET: &str = "\x1b[0m";
pub const BOLD: &str = "\x1b[1m";
pub const ITAL: &str = "\x1b[3m";

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Theme {
    pub ink: Rgb,
    pub mute: Rgb,
    pub edge: Rgb,
    pub s_ok: Rgb,
    pub s_warn: Rgb,
    pub s_hot: Rgb,
    pub pill_ink: Rgb,
    /// Linked-worktree hue (its own hue on purpose: not a severity).
    pub wt: Rgb,
}

pub const GRAPHITE: Theme = Theme {
    ink: (218, 221, 225),
    mute: (120, 125, 132),
    edge: (75, 80, 88),
    s_ok: (78, 200, 91),
    s_warn: (232, 178, 96),
    s_hot: (232, 116, 116),
    pill_ink: (238, 235, 224),
    wt: (168, 152, 226),
};

/// 24-bit foreground SGR.
pub fn fg(c: Rgb) -> String {
    format!("\x1b[38;2;{};{};{}m", c.0, c.1, c.2)
}

/// Python `_blend`: `int(a + (b - a) * t)` per channel — truncation, not rounding.
pub fn blend(a: Rgb, b: Rgb, t: f64) -> Rgb {
    let ch = |x: u8, y: u8| (x as f64 + (y as f64 - x as f64) * t) as u8;
    (ch(a.0, b.0), ch(a.1, b.1), ch(a.2, b.2))
}

/// Three-level severity used by every bar: >= critical → hot, >= warning → warn, else ok.
pub fn severity(t: &Theme, pct: f64, warning: f64, critical: f64) -> Rgb {
    if pct >= critical {
        t.s_hot
    } else if pct >= warning {
        t.s_warn
    } else {
        t.s_ok
    }
}
```
(keep the tests block from Step 1 below this code)

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --lib theme 2>&1 | tail -5`
Expected: `test result: ok. 2 passed`

- [ ] **Step 5: Commit**

```bash
git add src/theme.rs && git commit -m "feat: graphite palette, blend and SGR helpers"
```

---

### Task 2: format.rs — pure formatters

**Files:**
- Create: `src/format.rs`

- [ ] **Step 1: Write the failing tests**

```rust
// src/format.rs
#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::GRAPHITE;

    const NOW: f64 = 1793060340.047922117; // tests/fixtures/golden-orbit.now

    #[test]
    fn reset_5h_formats() {
        // 1793077200 − NOW = 16859.95 s → int(16859.95/60) = 280 min → 4h40m (NOT 4h41m: truncation)
        assert_eq!(reset_5h(Some(1793077200.0), NOW), ("4h40m".to_string(), Some(280)));
        // +0.5 s margins: NOW is fractional, so `(NOW + k) - NOW` may land a hair under k in f64
        assert_eq!(reset_5h(Some(NOW + 59.0 * 60.0 + 0.5), NOW), ("59m".to_string(), Some(59)));
        assert_eq!(reset_5h(Some(NOW - 10.0), NOW), ("0m".to_string(), Some(0)));
        assert_eq!(reset_5h(None, NOW), ("--".to_string(), None));
    }

    #[test]
    fn reset_7d_formats() {
        // 1793304000 − NOW = 243659.95 s → 2d 19h 40m → "2d19h"
        assert_eq!(reset_7d(Some(1793304000.0), NOW), "2d19h");
        assert_eq!(reset_7d(Some(NOW + 3.0 * 3600.0 + 5.0 * 60.0 + 0.5), NOW), "3h05m");
        assert_eq!(reset_7d(Some(NOW + 7.0 * 60.0 + 0.5), NOW), "7m");
        assert_eq!(reset_7d(None, NOW), "");
    }

    #[test]
    fn countdown_emoji_bands() {
        assert_eq!(countdown_emoji(None), "");
        assert_eq!(countdown_emoji(Some(1)), " 🎉");
        assert_eq!(countdown_emoji(Some(10)), " ✨");
        assert_eq!(countdown_emoji(Some(30)), " ⚡");
        assert_eq!(countdown_emoji(Some(31)), "");
    }

    #[test]
    fn cache_countdown() {
        assert_eq!(cache_remaining(0.0), "COLD");
        assert_eq!(cache_remaining(-5.0), "COLD");
        assert_eq!(cache_remaining(2947.95), "49m08s"); // ceil → 2948
        assert_eq!(cache_remaining(3600.0), "1h00m00s");
        assert_eq!(cache_remaining(45.0), "45s");
        assert_eq!(aurora_cache_text("49m08s"), "49m");
        assert_eq!(aurora_cache_text("1h00m00s"), "1h00m");
        assert_eq!(aurora_cache_text("4m23s"), "4m23s");
        assert_eq!(aurora_cache_text("45s"), "45s");
        assert_eq!(aurora_cache_text("COLD"), "COLD");
    }

    #[test]
    fn lines_and_duration() {
        assert_eq!(format_lines(965, 253), "+965 -253");
        assert_eq!(format_lines(5, 0), "+5");
        assert_eq!(format_lines(0, 3), "-3");
        assert_eq!(format_lines(0, 0), "");
        assert_eq!(format_duration_short(0), "");
        assert_eq!(format_duration_short(45_000), "45s");
        assert_eq!(format_duration_short(349_388), "5m");
        assert_eq!(format_duration_short(3_900_000), "1h05m");
    }

    #[test]
    fn projection_simple_average_pace() {
        // 5h: used 1%, 16859.95 s to reset → elapsed 1140.05 s ≥ 600 → 1 + 1/1140.05*16859.95 = 15.79 → →16%
        assert_eq!(projection_chip(FIVE_HOUR_S, MIN_ELAPSED_5H, Some(1.0), Some(1793077200.0), NOW), "→16%");
        // 7d: used 16%, 243659.95 s to reset → elapsed 361140.05 → 26.78 → →27%
        assert_eq!(projection_chip(SEVEN_DAY_S, MIN_ELAPSED_7D, Some(16.0), Some(1793304000.0), NOW), "→27%");
        // too early / no usage → placeholder; missing input → no chip
        assert_eq!(projection_chip(FIVE_HOUR_S, MIN_ELAPSED_5H, Some(3.0), Some(NOW + FIVE_HOUR_S - 100.0), NOW), "→--");
        assert_eq!(projection_chip(FIVE_HOUR_S, MIN_ELAPSED_5H, Some(0.0), Some(1793077200.0), NOW), "→--");
        assert_eq!(projection_chip(FIVE_HOUR_S, MIN_ELAPSED_5H, None, Some(1793077200.0), NOW), "");
        // overshoot: 60% used at 1h elapsed of 5h → raw 300% → clamp 100, eta = ttr*(40)/(240) = 4h*1/6 = 40m
        // (integer `now` so every intermediate value is exact in f64)
        let now = 1_793_060_000.0_f64;
        assert_eq!(projection_chip(FIVE_HOUR_S, MIN_ELAPSED_5H, Some(60.0), Some(now + 4.0 * 3600.0), now), "→100%·40m");
    }

    #[test]
    fn projection_parsing_and_colors() {
        assert_eq!(projection_pct("→16%"), Some(16.0));
        assert_eq!(projection_pct("→100%·40m"), Some(100.0));
        assert_eq!(projection_pct("→--"), None);
        assert_eq!(projection_pct(""), None);
        assert_eq!(projection_color(&GRAPHITE, "→16%"), GRAPHITE.mute);
        assert_eq!(projection_color(&GRAPHITE, "→70%"), GRAPHITE.s_warn);
        assert_eq!(projection_color(&GRAPHITE, "→85%"), GRAPHITE.s_hot);
        assert_eq!(projection_color(&GRAPHITE, "→--"), GRAPHITE.mute);
        // Python quirk: int("100%·40m") fails → mute
        assert_eq!(projection_color(&GRAPHITE, "→100%·40m"), GRAPHITE.mute);
        // window color follows the projection when one exists, else current usage
        assert_eq!(window_severity_rgb(&GRAPHITE, Some(1.0), "→16%", 30.0, 70.0), Some(GRAPHITE.s_ok));
        assert_eq!(window_severity_rgb(&GRAPHITE, Some(1.0), "→90%", 30.0, 70.0), Some(GRAPHITE.s_hot));
        assert_eq!(window_severity_rgb(&GRAPHITE, Some(35.0), "→--", 30.0, 70.0), Some(GRAPHITE.s_warn));
        assert_eq!(window_severity_rgb(&GRAPHITE, None, "", 30.0, 70.0), None);
    }

    #[test]
    fn eta_format() {
        assert_eq!(format_eta(30.0), "<1m");
        assert_eq!(format_eta(2400.0), "40m");
        assert_eq!(format_eta(4320.0), "1h12m");
        assert_eq!(format_eta(7200.0), "2h");
        assert_eq!(format_eta(90000.0), "1d1h");
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --lib format 2>&1 | tail -5`
Expected: compile errors (functions not found).

- [ ] **Step 3: Implement**

```rust
// src/format.rs
//! Pure string formatters transcribed from core.py / activity.py / predict.py / progress.py.

use crate::theme::{severity, Rgb, Theme};

pub const FIVE_HOUR_S: f64 = 5.0 * 3600.0;
pub const SEVEN_DAY_S: f64 = 7.0 * 86400.0;
pub const MIN_ELAPSED_5H: f64 = 10.0 * 60.0;
pub const MIN_ELAPSED_7D: f64 = 60.0 * 60.0;
pub const PROJECTION_WARNING: f64 = 70.0;
pub const PROJECTION_CRITICAL: f64 = 85.0;

/// Python `round()` / `f"{x:.0f}"`: round half to even.
pub fn round_even(x: f64) -> i64 {
    x.round_ties_even() as i64
}

/// core.py: `total_min = max(0, int(diff.total_seconds() / 60))` → `{h}h{mm}m` or `{m}m`.
/// Returns the text and the minutes-to-reset (None when no reset time).
pub fn reset_5h(resets_at: Option<f64>, now: f64) -> (String, Option<i64>) {
    let Some(r) = resets_at else {
        return ("--".to_string(), None);
    };
    let total_min = (((r - now) / 60.0).trunc() as i64).max(0);
    let (h, m) = (total_min / 60, total_min % 60);
    let text = if h > 0 { format!("{h}h{m:02}m") } else { format!("{m}m") };
    (text, Some(total_min))
}

/// core.py: `{d}d{hh}h` / `{h}h{mm}m` / `{m}m`; "" when no reset time.
pub fn reset_7d(resets_at: Option<f64>, now: f64) -> String {
    let Some(r) = resets_at else {
        return String::new();
    };
    let total = ((r - now).trunc() as i64).max(0);
    let d = total / 86400;
    let h = (total % 86400) / 3600;
    let m = (total % 3600) / 60;
    if d > 0 {
        format!("{d}d{h:02}h")
    } else if h > 0 {
        format!("{h}h{m:02}m")
    } else {
        format!("{m}m")
    }
}

/// progress.get_countdown_emoji — leading space included, exactly like Python.
pub fn countdown_emoji(minutes_to_reset: Option<i64>) -> &'static str {
    match minutes_to_reset {
        None => "",
        Some(m) if m <= 1 => " 🎉",
        Some(m) if m <= 10 => " ✨",
        Some(m) if m <= 30 => " ⚡",
        _ => "",
    }
}

/// activity.format_cache_remaining: "COLD" once gone, else `XhMMmSSs` / `MmSSs` / `Ss` (ceil).
pub fn cache_remaining(remaining: f64) -> String {
    if remaining <= 0.0 {
        return "COLD".to_string();
    }
    let r = remaining.ceil() as i64;
    let secs = r % 60;
    if r >= 3600 {
        return format!("{}h{:02}m{:02}s", r / 3600, (r % 3600) / 60, secs);
    }
    let mins = r / 60;
    if mins > 0 {
        format!("{mins}m{secs:02}s")
    } else {
        format!("{secs}s")
    }
}

/// styles._aurora_cache_text: drop the seconds once ≥ 10 minutes or when hours are present.
/// Regex in Python: `(\d+)m\d+s$`.
pub fn aurora_cache_text(text: &str) -> String {
    let Some(stripped) = text.strip_suffix('s') else {
        return text.to_string();
    };
    let Some(mpos) = stripped.rfind('m') else {
        return text.to_string();
    };
    let (head, secs) = stripped.split_at(mpos);
    let secs = &secs[1..];
    if secs.is_empty() || !secs.bytes().all(|b| b.is_ascii_digit()) {
        return text.to_string();
    }
    let digits_start = head
        .rfind(|c: char| !c.is_ascii_digit())
        .map(|i| i + 1)
        .unwrap_or(0);
    let mins = &head[digits_start..];
    if mins.is_empty() {
        return text.to_string();
    }
    let m: i64 = mins.parse().unwrap_or(0);
    if text.contains('h') || m >= 10 {
        format!("{head}m")
    } else {
        text.to_string()
    }
}

/// activity.format_lines: `+182 -47`, `+5`, `-3`, "" when both zero.
pub fn format_lines(added: i64, removed: i64) -> String {
    let mut parts = Vec::new();
    if added > 0 {
        parts.push(format!("+{added}"));
    }
    if removed > 0 {
        parts.push(format!("-{removed}"));
    }
    parts.join(" ")
}

/// activity.format_duration_short: `45s`, `12m`, `1h05m`, "" for 0.
pub fn format_duration_short(ms: i64) -> String {
    let s = ms / 1000;
    if s <= 0 {
        String::new()
    } else if s < 60 {
        format!("{s}s")
    } else if s < 3600 {
        format!("{}m", s / 60)
    } else {
        format!("{}h{:02}m", s / 3600, (s % 3600) / 60)
    }
}

/// predict._format_eta
pub fn format_eta(seconds: f64) -> String {
    let s = (seconds.trunc() as i64).max(0);
    if s < 60 {
        "<1m".to_string()
    } else if s < 3600 {
        format!("{}m", s / 60)
    } else if s < 86400 {
        let (h, m) = ((s / 60) / 60, (s / 60) % 60);
        if m != 0 { format!("{h}h{m:02}m") } else { format!("{h}h") }
    } else {
        format!("{}d{}h", s / 86400, (s % 86400) / 3600)
    }
}

/// Simplified end-of-window projection (DEVIATION from Python's smoothed predictor):
/// average pace so far → `→NN%`; `→--` when too early or no usage; "" when inputs missing;
/// `·<eta>` appended when the pace overshoots the cap (predict._depletion_eta_seconds).
pub fn projection_chip(window_len: f64, min_elapsed: f64, used: Option<f64>, resets_at: Option<f64>, now: f64) -> String {
    let (Some(used), Some(reset)) = (used, resets_at) else {
        return String::new();
    };
    let ttr = (reset - now).max(0.0);
    let elapsed = window_len - ttr;
    if used <= 0.0 || elapsed < min_elapsed {
        return "→--".to_string();
    }
    let raw = used + used / elapsed * ttr;
    let proj = raw.clamp(0.0, 100.0);
    let mut chip = format!("→{}%", round_even(proj));
    if proj >= 99.5 {
        if let Some(eta) = depletion_eta(used, ttr, raw) {
            chip.push('·');
            chip.push_str(&format_eta(eta));
        }
    }
    chip
}

fn depletion_eta(used: f64, ttr: f64, raw: f64) -> Option<f64> {
    if raw <= 100.0 || raw <= used || ttr <= 0.0 || used >= 100.0 {
        return None;
    }
    let eta = ttr * (100.0 - used) / (raw - used);
    if eta < ttr { Some(eta) } else { None }
}

/// progress.projection_pct: regex `→?\s*(\d+(?:\.\d+)?)%` anchored at the start.
pub fn projection_pct(chip: &str) -> Option<f64> {
    let body = chip.strip_prefix('→').unwrap_or(chip).trim_start();
    let end = body.find('%')?;
    let num = &body[..end];
    if num.is_empty() || !num.bytes().all(|b| b.is_ascii_digit() || b == b'.') {
        return None;
    }
    num.parse::<f64>().ok()
}

/// progress._projection_color: `chip.lstrip("→").rstrip("%")` then `int()`; failure → mute.
pub fn projection_color(t: &Theme, chip: &str) -> Rgb {
    let body = chip.trim_start_matches('→').trim_end_matches('%');
    match body.trim().parse::<i64>() {
        Ok(v) if (v as f64) >= PROJECTION_CRITICAL => t.s_hot,
        Ok(v) if (v as f64) >= PROJECTION_WARNING => t.s_warn,
        _ => t.mute,
    }
}

/// progress.window_severity_rgb: projection (70/85) wins; else current usage on the configured
/// thresholds; None when neither exists.
pub fn window_severity_rgb(t: &Theme, current_pct: Option<f64>, chip: &str, warning: f64, critical: f64) -> Option<Rgb> {
    if let Some(p) = projection_pct(chip) {
        return Some(severity(t, p, PROJECTION_WARNING, PROJECTION_CRITICAL));
    }
    current_pct.map(|p| severity(t, p, warning, critical))
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --lib format 2>&1 | tail -5`
Expected: `test result: ok. 8 passed`

- [ ] **Step 5: Commit**

```bash
git add src/format.rs && git commit -m "feat: time, cache, lines and projection formatters"
```

---

### Task 3: payload.rs + config.rs — stdin JSON and config file

**Files:**
- Create: `src/payload.rs`, `src/config.rs`

- [ ] **Step 1: Write the failing tests**

```rust
// src/payload.rs
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_fixture() {
        let raw = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/stdin-orbit.json")).unwrap();
        let p: Payload = serde_json::from_str(&raw).unwrap();
        assert_eq!(p.model.display_name, "Fable 5.1");
        assert_eq!(p.workspace.repo.as_ref().unwrap().name.as_deref(), Some("orbit"));
        assert_eq!(p.cost.total_lines_added, 965);
        let rl = p.rate_limits.as_ref().unwrap();
        assert_eq!(rl.five_hour.as_ref().unwrap().pct(), 1.0);
        assert_eq!(rl.five_hour.as_ref().unwrap().resets_at, Some(1793077200.0));
        assert_eq!(rl.seven_day.as_ref().unwrap().pct(), 16.0);
        assert_eq!(p.context_window.as_ref().unwrap().ctx_pct(), Some(8.0));
        assert_eq!(p.prompt_cache.as_ref().unwrap().expires_at, Some(1793063288.0));
    }

    #[test]
    fn tolerates_missing_and_null() {
        let p: Payload = serde_json::from_str(r#"{"model":{"id":"x"},"context_window":{"context_window_size":200000,"used_percentage":null}}"#).unwrap();
        assert_eq!(p.model.display_name, "");
        assert!(p.rate_limits.is_none());
        assert_eq!(p.context_window.as_ref().unwrap().ctx_pct(), None);
        // key absent → Python default 0
        let p: Payload = serde_json::from_str(r#"{"context_window":{"context_window_size":200000}}"#).unwrap();
        assert_eq!(p.context_window.as_ref().unwrap().ctx_pct(), Some(0.0));
        let p: Payload = serde_json::from_str("{}").unwrap();
        assert!(p.context_window.is_none());
    }

    #[test]
    fn pct_coercion_matches_python() {
        let w: Window = serde_json::from_str(r#"{"used_percentage":56.00000000000001}"#).unwrap();
        assert_eq!(w.pct(), 56.0);
        let w: Window = serde_json::from_str(r#"{"used_percentage":1793077200}"#).unwrap(); // leaked epoch
        assert_eq!(w.pct(), 0.0);
        let w: Window = serde_json::from_str(r#"{"used_percentage":-3}"#).unwrap();
        assert_eq!(w.pct(), 0.0);
        let w: Window = serde_json::from_str(r#"{"resets_at":0}"#).unwrap();
        assert_eq!(w.pct(), 0.0);
        assert_eq!(w.reset_time(), None); // 0 is falsy in Python → "--"
    }
}
```

```rust
// src/config.rs
#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::GRAPHITE;

    #[test]
    fn defaults_when_missing() {
        let c = Config::load(std::path::Path::new("/nonexistent/claude-statusbar.json"));
        assert_eq!(c.thin, false);
        assert_eq!(c.pad, " ");
        assert!(c.show_weekly && c.show_cache_age && c.show_project_branch && c.show_lines && c.show_projection);
        assert!(!c.show_cost && !c.show_duration && !c.show_ahead_behind);
        assert_eq!(c.worktree_glyph, "🌳");
        assert_eq!(c.theme, GRAPHITE);
        assert_eq!((c.warning, c.critical), (30.0, 70.0));
    }

    #[test]
    fn reads_subset() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("cfg.json");
        std::fs::write(&p, r#"{"bar_glyph":"thin","density":"compact","show_cost":true,"show_weekly":"false","worktree_glyph":"⑂","color_ok":"#4ec85b","color_hot":"fff","warning_threshold":50,"critical_threshold":90,"unknown_key":1}"#).unwrap();
        let c = Config::load(&p);
        assert!(c.thin);
        assert_eq!(c.pad, "");
        assert!(c.show_cost);
        assert!(!c.show_weekly);
        assert_eq!(c.worktree_glyph, "⑂");
        assert_eq!(c.theme.s_ok, (0x4e, 0xc8, 0x5b));
        assert_eq!(c.theme.s_hot, (255, 255, 255));
        assert_eq!((c.warning, c.critical), (50.0, 90.0));
        // invalid threshold pair → defaults
        std::fs::write(&p, r#"{"warning_threshold":80,"critical_threshold":20}"#).unwrap();
        let c = Config::load(&p);
        assert_eq!((c.warning, c.critical), (30.0, 70.0));
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --lib payload config 2>&1 | tail -5` — Expected: compile errors.

- [ ] **Step 3: Implement payload.rs**

```rust
// src/payload.rs
//! The JSON Claude Code pipes to the statusLine command. Every field optional; unknown ignored.

use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;

use crate::format::round_even;

#[derive(Deserialize, Default, Debug)]
#[serde(default)]
pub struct Payload {
    pub session_id: String,
    pub transcript_path: String,
    pub cwd: String,
    pub model: Model,
    pub workspace: Workspace,
    pub version: String,
    pub cost: Cost,
    pub context_window: Option<ContextWindow>,
    pub prompt_cache: Option<PromptCache>,
    pub rate_limits: Option<RateLimits>,
    /// Stamped by the Python daemon only; honored when present (COLUMNS).
    #[serde(rename = "_cs_env")]
    pub cs_env: Option<HashMap<String, String>>,
}

#[derive(Deserialize, Default, Debug)]
#[serde(default)]
pub struct Model {
    pub id: String,
    pub display_name: String,
}

#[derive(Deserialize, Default, Debug)]
#[serde(default)]
pub struct Workspace {
    pub current_dir: Option<String>,
    pub project_dir: Option<String>,
    pub repo: Option<Repo>,
}

#[derive(Deserialize, Default, Debug)]
#[serde(default)]
pub struct Repo {
    pub name: Option<String>,
}

#[derive(Deserialize, Default, Debug)]
#[serde(default)]
pub struct Cost {
    pub total_cost_usd: f64,
    pub total_duration_ms: i64,
    pub total_lines_added: i64,
    pub total_lines_removed: i64,
}

fn zero() -> Value {
    Value::from(0)
}

#[derive(Deserialize, Debug)]
#[serde(default)]
pub struct ContextWindow {
    pub context_window_size: f64,
    /// Raw so that "key absent" (→ 0, Python default) and "null" (→ unknown) stay distinct.
    #[serde(default = "zero")]
    pub used_percentage: Value,
}

impl Default for ContextWindow {
    fn default() -> Self {
        ContextWindow { context_window_size: 0.0, used_percentage: zero() }
    }
}

impl ContextWindow {
    pub fn ctx_pct(&self) -> Option<f64> {
        self.used_percentage.as_f64()
    }
}

#[derive(Deserialize, Default, Debug)]
#[serde(default)]
pub struct PromptCache {
    pub expires_at: Option<f64>,
}

#[derive(Deserialize, Default, Debug)]
#[serde(default)]
pub struct RateLimits {
    pub five_hour: Option<Window>,
    pub seven_day: Option<Window>,
}

#[derive(Deserialize, Debug)]
#[serde(default)]
pub struct Window {
    #[serde(default = "zero")]
    pub used_percentage: Value,
    pub resets_at: Option<f64>,
}

impl Default for Window {
    fn default() -> Self {
        Window { used_percentage: zero(), resets_at: None }
    }
}

impl Window {
    /// core.parse_stdin_data `_pct`: non-number → 0, NaN/inf → 0, ≥ 100000 (leaked epoch) → 0,
    /// else `max(0, int(round(f)))`.
    pub fn pct(&self) -> f64 {
        let f = self.used_percentage.as_f64().unwrap_or(0.0);
        if !f.is_finite() || f >= 100000.0 {
            return 0.0;
        }
        round_even(f).max(0) as f64
    }

    /// Python `if resets_at:` — 0 / missing both mean "unknown".
    pub fn reset_time(&self) -> Option<f64> {
        self.resets_at.filter(|r| *r != 0.0)
    }
}
```

- [ ] **Step 4: Implement config.rs**

```rust
// src/config.rs
//! Subset of ~/.claude/claude-statusbar.json (the Python tool's config). Missing/invalid → defaults.

use serde_json::{Map, Value};
use std::path::{Path, PathBuf};

use crate::theme::{Rgb, Theme, GRAPHITE};

#[derive(Clone, Debug)]
pub struct Config {
    pub thin: bool,
    pub pad: &'static str,
    pub show_weekly: bool,
    pub show_cost: bool,
    pub show_cache_age: bool,
    pub show_project_branch: bool,
    pub show_lines: bool,
    pub show_duration: bool,
    pub show_ahead_behind: bool,
    pub show_projection: bool,
    pub worktree_glyph: String,
    pub theme: Theme,
    pub warning: f64,
    pub critical: f64,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            thin: false,
            pad: " ",
            show_weekly: true,
            show_cost: false,
            show_cache_age: true,
            show_project_branch: true,
            show_lines: true,
            show_duration: false,
            show_ahead_behind: false,
            show_projection: true,
            worktree_glyph: "🌳".to_string(),
            theme: GRAPHITE,
            warning: 30.0,
            critical: 70.0,
        }
    }
}

/// `$CSR_CONFIG` if set, else `$HOME/.claude/claude-statusbar.json`.
pub fn config_path(home: &Path) -> PathBuf {
    match std::env::var_os("CSR_CONFIG") {
        Some(p) => PathBuf::from(p),
        None => home.join(".claude").join("claude-statusbar.json"),
    }
}

fn get_bool(m: &Map<String, Value>, key: &str, default: bool) -> bool {
    match m.get(key) {
        Some(Value::Bool(b)) => *b,
        Some(Value::String(s)) => match s.trim().to_ascii_lowercase().as_str() {
            "1" | "true" | "yes" | "on" => true,
            "0" | "false" | "no" | "off" => false,
            _ => default,
        },
        Some(Value::Number(n)) => n.as_f64().map(|f| f != 0.0).unwrap_or(default),
        _ => default,
    }
}

fn get_str<'a>(m: &'a Map<String, Value>, key: &str) -> Option<&'a str> {
    m.get(key).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty())
}

/// themes.parse_hex_color: `#rrggbb`, `#rgb`, `rrggbb`, `rgb`.
pub fn parse_hex(s: &str) -> Option<Rgb> {
    let s = s.trim().trim_start_matches('#');
    let s: String = if s.len() == 3 { s.chars().flat_map(|c| [c, c]).collect() } else { s.to_string() };
    if s.len() != 6 {
        return None;
    }
    let ch = |i: usize| u8::from_str_radix(&s[i..i + 2], 16).ok();
    Some((ch(0)?, ch(2)?, ch(4)?))
}

impl Config {
    pub fn load(path: &Path) -> Config {
        let mut c = Config::default();
        let Ok(text) = std::fs::read_to_string(path) else {
            return c;
        };
        let Ok(Value::Object(m)) = serde_json::from_str::<Value>(&text) else {
            return c;
        };
        c.thin = get_str(&m, "bar_glyph") == Some("thin");
        c.pad = match get_str(&m, "density") {
            Some("compact") => "",
            Some("cozy") => "  ",
            _ => " ",
        };
        c.show_weekly = get_bool(&m, "show_weekly", c.show_weekly);
        c.show_cost = get_bool(&m, "show_cost", c.show_cost);
        c.show_cache_age = get_bool(&m, "show_cache_age", c.show_cache_age);
        c.show_project_branch = get_bool(&m, "show_project_branch", c.show_project_branch);
        c.show_lines = get_bool(&m, "show_lines", c.show_lines);
        c.show_duration = get_bool(&m, "show_duration", c.show_duration);
        c.show_ahead_behind = get_bool(&m, "show_ahead_behind", c.show_ahead_behind);
        c.show_projection = get_bool(&m, "show_projection", c.show_projection);
        if let Some(g) = get_str(&m, "worktree_glyph") {
            c.worktree_glyph = g.to_string();
        }
        if let Some(rgb) = get_str(&m, "color_ok").and_then(parse_hex) {
            c.theme.s_ok = rgb;
        }
        if let Some(rgb) = get_str(&m, "color_warn").and_then(parse_hex) {
            c.theme.s_warn = rgb;
        }
        if let Some(rgb) = get_str(&m, "color_hot").and_then(parse_hex) {
            c.theme.s_hot = rgb;
        }
        if let Some(rgb) = get_str(&m, "color_worktree").and_then(parse_hex) {
            c.theme.wt = rgb;
        }
        let w = m.get("warning_threshold").and_then(Value::as_f64);
        let cr = m.get("critical_threshold").and_then(Value::as_f64);
        if let (Some(w), Some(cr)) = (w, cr) {
            if (0.0..=100.0).contains(&w) && (0.0..=100.0).contains(&cr) && w < cr {
                c.warning = w;
                c.critical = cr;
            }
        }
        c
    }
}
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test --lib payload config 2>&1 | tail -5` — Expected: `5 passed`.

- [ ] **Step 6: Commit**

```bash
git add src/payload.rs src/config.rs && git commit -m "feat: stdin payload and config parsing"
```

---

### Task 4: width.rs — ANSI-aware width and clipping

**Files:**
- Create: `src/width.rs`

- [ ] **Step 1: Write the failing tests**

```rust
// src/width.rs
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn widths_ignore_escapes_and_count_cells() {
        assert_eq!(visible_width("5h"), 2);
        assert_eq!(visible_width("\x1b[38;2;1;2;3m5h\x1b[0m"), 2);
        assert_eq!(visible_width("\x1b]8;;https://x\x07orbit\x1b]8;;\x07"), 5);
        assert_eq!(visible_width("⏰"), 2);
        assert_eq!(visible_width("🌳"), 2);
        assert_eq!(visible_width("■→│❄⎇●·"), 7);
    }

    #[test]
    fn clip_keeps_escapes_and_closes_state() {
        assert_eq!(clip_line("abcdef", Some(10)), "abcdef");
        assert_eq!(clip_line("abcdef", Some(4)), "abc…");
        assert_eq!(clip_line("abcdef", Some(0)), "");
        // SGR left open at the cut → RESET appended after the ellipsis
        assert_eq!(clip_line("\x1b[1mabcdef\x1b[0m", Some(4)), "\x1b[1mabc…\x1b[0m");
        // OSC 8 link left open at the cut → link closed, then RESET not needed
        assert_eq!(clip_line("\x1b]8;;https://x\x07abcdef\x1b]8;;\x07", Some(4)), "\x1b]8;;https://x\x07abc…\x1b]8;;\x07");
        // wide char that would straddle the limit is dropped
        assert_eq!(clip_line("ab🌳cd", Some(3)), "ab…");
        assert_eq!(clip_lines("abcdef\nabcdef", Some(4)), "abc…\nabc…");
        assert_eq!(clip_lines("abcdef\nabcdef", None), "abcdef\nabcdef");
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --lib width 2>&1 | tail -5` — Expected: compile errors.

- [ ] **Step 3: Implement**

```rust
// src/width.rs
//! Terminal-cell width and clipping that skip ANSI sequences (port of _display.py).

use unicode_width::UnicodeWidthChar;

use crate::theme::RESET;

const LINK_CLOSE: &str = "\x1b]8;;\x07";

pub enum Segment<'a> {
    Text(&'a str),
    Ansi(&'a str),
}

/// Split into text and escape sequences. Mirrors the Python regex
/// `\x1b(?: \][^\x07\x1b]*(?:\x07|\x1b\\) | \[[0-?]*[ -/]*[@-~] | [ -/]*[@-~] )`.
pub fn segments(s: &str) -> Vec<Segment<'_>> {
    let b = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    let mut text_start = 0;
    while i < b.len() {
        if b[i] != 0x1b {
            i += 1;
            continue;
        }
        let start = i;
        let mut j = i + 1;
        let end = if j < b.len() && b[j] == b']' {
            j += 1;
            while j < b.len() && b[j] != 0x07 && b[j] != 0x1b {
                j += 1;
            }
            if j < b.len() && b[j] == 0x07 {
                Some(j + 1)
            } else if j + 1 < b.len() && b[j] == 0x1b && b[j + 1] == b'\\' {
                Some(j + 2)
            } else {
                None
            }
        } else if j < b.len() && b[j] == b'[' {
            j += 1;
            while j < b.len() && (0x30..=0x3f).contains(&b[j]) {
                j += 1;
            }
            while j < b.len() && (0x20..=0x2f).contains(&b[j]) {
                j += 1;
            }
            if j < b.len() && (0x40..=0x7e).contains(&b[j]) { Some(j + 1) } else { None }
        } else {
            while j < b.len() && (0x20..=0x2f).contains(&b[j]) {
                j += 1;
            }
            if j < b.len() && (0x40..=0x7e).contains(&b[j]) { Some(j + 1) } else { None }
        };
        match end {
            Some(e) => {
                if text_start < start {
                    out.push(Segment::Text(&s[text_start..start]));
                }
                out.push(Segment::Ansi(&s[start..e]));
                i = e;
                text_start = e;
            }
            None => i += 1,
        }
    }
    if text_start < b.len() {
        out.push(Segment::Text(&s[text_start..]));
    }
    out
}

/// Python `_char_width`: combining / Cf / Cc / Cs → 0; East Asian W/F → 2; else 1.
pub fn char_width(c: char) -> usize {
    if c.is_control() {
        return 0;
    }
    c.width().unwrap_or(0)
}

pub fn visible_width(s: &str) -> usize {
    segments(s)
        .iter()
        .map(|seg| match seg {
            Segment::Text(t) => t.chars().map(char_width).sum(),
            Segment::Ansi(_) => 0,
        })
        .sum()
}

/// `_sgr_active`: does an SGR sequence leave styling on?
fn sgr_state(active: bool, seq: &str) -> bool {
    let Some(params) = seq.strip_prefix("\x1b[").and_then(|s| s.strip_suffix('m')) else {
        return active;
    };
    if !params.bytes().all(|b| b.is_ascii_digit() || b == b';' || b == b':') {
        return active;
    }
    if params.is_empty() {
        return false;
    }
    let neutral = ["", "22", "23", "24", "25", "27", "28", "29", "39", "49"];
    let mut a = active;
    for v in params.replace(':', ";").split(';') {
        if v == "0" {
            a = false;
        } else if !neutral.contains(&v) {
            a = true;
        }
    }
    a
}

/// `_link_open`: an OSC 8 with a non-empty URL opens, with an empty one closes.
fn link_state(open: bool, seq: &str) -> bool {
    let Some(rest) = seq.strip_prefix("\x1b]8;") else {
        return open;
    };
    let body = rest.strip_suffix('\x07').or_else(|| rest.strip_suffix("\x1b\\")).unwrap_or(rest);
    match body.split_once(';') {
        Some((_, url)) => !url.is_empty(),
        None => open,
    }
}

/// Clip one physical line to `max_width` cells with a trailing `…`, never splitting an escape,
/// closing an open link / SGR at the cut.
pub fn clip_line(text: &str, max_width: Option<usize>) -> String {
    let Some(max) = max_width else {
        return text.to_string();
    };
    if visible_width(text) <= max {
        return text.to_string();
    }
    if max == 0 {
        return String::new();
    }
    let limit = max - 1; // "…" is one cell
    let mut out = String::new();
    let mut used = 0;
    let mut sgr = false;
    let mut link = false;
    for seg in segments(text) {
        match seg {
            Segment::Text(t) => {
                for c in t.chars() {
                    let w = char_width(c);
                    if used + w > limit {
                        out.push('…');
                        if link {
                            out.push_str(LINK_CLOSE);
                        }
                        if sgr {
                            out.push_str(RESET);
                        }
                        return out;
                    }
                    out.push(c);
                    used += w;
                }
            }
            Segment::Ansi(a) => {
                out.push_str(a);
                sgr = sgr_state(sgr, a);
                link = link_state(link, a);
            }
        }
    }
    text.to_string()
}

pub fn clip_lines(text: &str, max_width: Option<usize>) -> String {
    if max_width.is_none() || text.is_empty() {
        return text.to_string();
    }
    text.split('\n').map(|l| clip_line(l, max_width)).collect::<Vec<_>>().join("\n")
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --lib width 2>&1 | tail -5` — Expected: `2 passed`. If `visible_width("⏰")` or `"🌳"` is not 2, check the `unicode-width` version resolved in `Cargo.lock` (must be 0.2.x) before changing anything else.

- [ ] **Step 5: Commit**

```bash
git add src/width.rs && git commit -m "feat: ANSI-aware width and clipping"
```

---

### Task 5: identity.rs — git identity from the filesystem

**Files:**
- Create: `src/identity.rs`

- [ ] **Step 1: Write the failing tests**

```rust
// src/identity.rs
#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

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
        assert_eq!(read_head(&repo.join("sub/dir")), Some(("feat/x".to_string(), false)));
        assert_eq!(resolve_toplevel(&repo.join("sub")), Some(repo.canonicalize().unwrap()));
        write(&repo.join(".git/HEAD"), "0123456789abcdef0123456789abcdef01234567\n");
        assert_eq!(read_head(&repo), Some(("0123456".to_string(), true)));
        write(&repo.join(".git/HEAD"), "ref: refs/remotes/origin/HEAD\n");
        assert_eq!(read_head(&repo), Some(("HEAD".to_string(), false)));
        assert_eq!(read_head(d.path()), None);
    }

    #[test]
    fn linked_worktree_detection_and_count() {
        let d = tempfile::tempdir().unwrap();
        let repo = d.path().join("myrepo");
        let wt = d.path().join("myrepo-wt1");
        let ghost = repo.join(".git/worktrees/ghost");
        write(&repo.join(".git/HEAD"), "ref: refs/heads/main\n");
        write(&repo.join(".git/worktrees/wt1/HEAD"), "ref: refs/heads/topic\n");
        write(&repo.join(".git/worktrees/wt1/gitdir"), &format!("{}/.git\n", wt.display()));
        write(&repo.join(".git/worktrees/wt1/commondir"), "../..\n");
        write(&ghost.join("gitdir"), "/nonexistent/.git\n");
        write(&wt.join(".git"), &format!("gitdir: {}/.git/worktrees/wt1\n", repo.display()));

        let w = parse_worktree(&wt.join("deep")).expect("linked worktree");
        assert_eq!(w.name, "wt1");
        assert_eq!(w.repo_name, "myrepo");
        assert_eq!(w.registry, repo.join(".git/worktrees"));
        assert_eq!(count_live_worktrees(&w.registry), 1); // ghost not counted
        assert_eq!(read_head(&wt), Some(("topic".to_string(), false)));
        assert!(parse_worktree(&repo).is_none());
        assert_eq!(strip_repo_prefix("myrepo-wt1", "myrepo"), "wt1");
        assert_eq!(strip_repo_prefix("myrepo", "myrepo"), "myrepo");
        assert_eq!(strip_repo_prefix("other", "myrepo"), "other");
    }

    #[test]
    fn remote_urls() {
        assert_eq!(remote_web_url("git@github.com:exampleco/orbit.git"), Some("https://github.com/exampleco/orbit".into()));
        assert_eq!(remote_web_url("https://user:tok@github.com/o/r.git"), Some("https://github.com/o/r".into()));
        assert_eq!(remote_web_url("ssh://git@gitlab.com:22/grp/sub/r"), Some("https://gitlab.com/grp/sub/r".into()));
        assert_eq!(remote_web_url("https://github.com/o/r/"), Some("https://github.com/o/r".into()));
        assert_eq!(remote_web_url("git@host:1234/r"), None); // scp form with a leading port-like path
        assert_eq!(remote_web_url("https://h/o/../r"), None);
        assert_eq!(remote_web_url("file:///x"), None);
        assert_eq!(branch_web_url(Some("https://github.com/o/r"), Some("feat/a b")), Some("https://github.com/o/r/tree/feat/a%20b".into()));
        assert_eq!(branch_web_url(Some("https://gitlab.com/o/r"), Some("main")), Some("https://gitlab.com/o/r/-/tree/main".into()));
        assert_eq!(branch_web_url(Some("https://bitbucket.org/o/r"), Some("main")), Some("https://bitbucket.org/o/r/src/main".into()));
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
        write(&wt.join(".git"), &format!("gitdir: {}/.git/worktrees/wt\n", repo.display()));
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
        p.workspace.repo = Some(crate::payload::Repo { name: Some("orbit".into()) });
        assert_eq!(resolve_identity(&p).project_name, "orbit");
        p.workspace.current_dir = Some(d.path().join("nogit").to_string_lossy().into_owned());
        p.workspace.project_dir = None;
        p.workspace.repo = None;
        let id = resolve_identity(&p);
        assert!(!id.in_git);
        assert_eq!(id.project_name, "nogit");
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --lib identity 2>&1 | tail -5` — Expected: compile errors.

- [ ] **Step 3: Implement**

```rust
// src/identity.rs
//! Project + branch identity from the stdin payload and the local filesystem (port of identity.py).
//! Pure file reads; never runs git.

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
            let p = if p.is_absolute() { p } else { c.join(p).canonicalize().ok()? };
            return if p.exists() { Some(p) } else { None };
        }
    }
    None
}

/// (branch or 7-char sha, detached) or None when not in a git repo.
pub fn read_head(start: &Path) -> Option<(String, bool)> {
    let gitdir = resolve_gitdir(start)?;
    let text = fs::read_to_string(gitdir.join("HEAD")).ok()?;
    let text = text.trim();
    if let Some(r) = text.strip_prefix("ref:") {
        let r = r.trim();
        let name = match r.strip_prefix("refs/heads/") {
            Some(n) => n.to_string(),
            None => r.rsplit('/').next().unwrap_or(r).to_string(),
        };
        return Some((name, false));
    }
    if text.len() == 40 && text.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)) {
        return Some((text[..7].to_string(), true));
    }
    None
}

/// First ancestor (including `start`) that contains a `.git` file or dir.
pub fn resolve_toplevel(start: &Path) -> Option<PathBuf> {
    ancestors(start).into_iter().find(|c| c.join(".git").exists())
}

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
            const MARK: &str = "/.git/worktrees/";
            let idx = norm.rfind(MARK)?;
            let root = norm[..idx].trim_end_matches('/');
            let name = &norm[idx + MARK.len()..];
            if root.is_empty() || name.is_empty() || name.contains('/') {
                return None;
            }
            let repo_name = root.rsplit('/').next().filter(|s| !s.is_empty()).unwrap_or(root).to_string();
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
            fs::read_to_string(e.path().join("gitdir"))
                .map(|g| {
                    let g = g.trim();
                    !g.is_empty() && Path::new(g).exists()
                })
                .unwrap_or(false)
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
    let (host, path) = match scheme_rest {
        Some(rest) => {
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
        }
        None => {
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
            let digits = path.bytes().take_while(|b| b.is_ascii_digit()).count();
            if digits > 0 && path[digits..].starts_with('/') {
                return None; // the `(?!\d+/)` lookahead: that is a port, not a path
            }
            (host, path)
        }
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
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'_' | b'.' | b'-' | b'~' | b'/' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
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
        Ok(rel) => gitdir.join(rel.trim()).canonicalize().unwrap_or_else(|_| gitdir.clone()),
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

fn basename(s: &str) -> String {
    let t = s.trim_end_matches('/');
    match t.rsplit('/').next() {
        Some(b) if !b.is_empty() => b.to_string(),
        _ => s.to_string(),
    }
}

pub fn resolve_identity(p: &Payload) -> Identity {
    let non_empty = |s: &Option<String>| s.clone().filter(|x| !x.is_empty());
    let repo_name = p.workspace.repo.as_ref().and_then(|r| non_empty(&r.name));
    let project_dir = non_empty(&p.workspace.project_dir);
    let current_dir = non_empty(&p.workspace.current_dir).or_else(|| Some(p.cwd.clone()).filter(|s| !s.is_empty()));
    let cwd_env = std::env::current_dir().map(|d| d.to_string_lossy().into_owned()).unwrap_or_default();
    let start = PathBuf::from(current_dir.clone().or_else(|| project_dir.clone()).unwrap_or_else(|| cwd_env.clone()));

    let head = read_head(&start);
    let toplevel = if head.is_some() { resolve_toplevel(&start) } else { None };
    let wt = if head.is_some() { parse_worktree(&start) } else { None };
    let worktree_name = wt.as_ref().map(|w| strip_repo_prefix(&w.name, &w.repo_name));
    let worktree_count = match (&wt, &toplevel) {
        (Some(w), _) => count_live_worktrees(&w.registry),
        (None, Some(t)) => count_live_worktrees(&t.join(".git").join("worktrees")),
        _ => 0,
    };
    let project_name = repo_name
        .or_else(|| wt.as_ref().map(|w| w.repo_name.clone()))
        .or_else(|| project_dir.as_deref().map(basename))
        .or_else(|| current_dir.as_deref().map(basename))
        .unwrap_or_else(|| {
            let b = basename(&cwd_env);
            if b.is_empty() { "?".to_string() } else { b }
        });
    let web_url = if head.is_some() { read_web_url(&start) } else { None };

    Identity {
        project_name,
        in_git: head.is_some(),
        branch: head.as_ref().map(|h| h.0.clone()),
        detached: head.as_ref().map(|h| h.1).unwrap_or(false),
        worktree_name,
        toplevel,
        is_worktree: wt.is_some(),
        worktree_count,
        web_url,
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --lib identity 2>&1 | tail -5` — Expected: `5 passed`.

- [ ] **Step 5: Commit**

```bash
git add src/identity.rs && git commit -m "feat: git identity from HEAD, worktrees and remote config"
```

---

### Task 6: gitcache.rs — dirty-state cache and detached refresh

**Files:**
- Create: `src/gitcache.rs`

- [ ] **Step 1: Write the failing tests**

```rust
// src/gitcache.rs
#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    #[test]
    fn parses_porcelain_branch_header() {
        assert_eq!(parse_status("## main...origin/main [ahead 2, behind 1]\n M a.rs\n?? b\n"), (true, Some(2), Some(1)));
        assert_eq!(parse_status("## main...origin/main\n"), (false, Some(0), Some(0)));
        assert_eq!(parse_status("## main\n"), (false, None, None));
        assert_eq!(parse_status("## main\n\n"), (false, None, None));
        assert_eq!(parse_status("## HEAD (no branch)\n M x\n"), (true, None, None));
    }

    #[test]
    fn cache_path_is_stable_and_freshness_window_is_30s() {
        let home = Path::new("/h");
        let p = cache_path(home, "/repo");
        assert_eq!(p.parent().unwrap(), Path::new("/h/.cache/csr/git"));
        assert_eq!(p, cache_path(home, "/repo"));
        assert_ne!(p, cache_path(home, "/repo2"));
        let e = Entry { toplevel: "/repo".into(), dirty: Some(true), ahead: None, behind: None, ts: 1000.0 };
        assert!(is_fresh(&e, 1000.0));
        assert!(is_fresh(&e, 1029.9));
        assert!(!is_fresh(&e, 1030.0));
        assert!(!is_fresh(&e, 999.0));
    }

    #[test]
    fn refresh_writes_entry_for_a_real_repo() {
        let d = tempfile::tempdir().unwrap();
        let home = d.path().join("home");
        let repo = d.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        let git = |args: &[&str]| {
            assert!(Command::new("git").args(args).current_dir(&repo).output().unwrap().status.success());
        };
        git(&["init", "-q"]); // no `-b`: Ubuntu 20.04 ships git 2.25, which lacks it
        git(&["-c", "user.email=t@t", "-c", "user.name=t", "commit", "-q", "--allow-empty", "-m", "init"]);
        let top = repo.canonicalize().unwrap().to_string_lossy().into_owned();
        refresh(&home, &top);
        let e = read_entry(&cache_path(&home, &top)).expect("entry written");
        assert_eq!(e.dirty, Some(false));
        assert_eq!((e.ahead, e.behind), (None, None));
        std::fs::write(repo.join("new.txt"), "x").unwrap();
        std::fs::remove_file(cache_path(&home, &top)).unwrap();
        refresh(&home, &top);
        assert_eq!(read_entry(&cache_path(&home, &top)).unwrap().dirty, Some(true));
        assert!(!cache_path(&home, &top).with_extension("lock").exists());
        assert!(!cache_path(&home, &top).with_extension("inflight").exists());
    }

    #[test]
    fn fresh_cache_is_returned_without_spawning() {
        let d = tempfile::tempdir().unwrap();
        let home = d.path();
        let path = cache_path(home, "/repo");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, r#"{"toplevel":"/repo","dirty":true,"ahead":0,"behind":3,"ts":5000.0}"#).unwrap();
        let (dirty, entry) = dirty_with_async_refresh(home, "/repo", 5010.0);
        assert_eq!(dirty, Some(true));
        assert_eq!(entry.unwrap().behind, Some(3));
        assert!(!path.with_extension("inflight").exists());
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --lib gitcache 2>&1 | tail -5` — Expected: compile errors.

- [ ] **Step 3: Implement**

```rust
// src/gitcache.rs
//! 30-second cache of `git status` facts (port of git_cache.py + _git_refresh.py).
//! The render path only reads the cache; a stale cache spawns a detached `csr git-refresh`.

use serde::{Deserialize, Serialize};
use std::fs;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

pub const TTL_S: f64 = 30.0;
pub const INFLIGHT_MAX_AGE_S: f64 = 30.0;
const LOCK_MAX_AGE_S: u64 = 60;

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
pub struct Entry {
    pub toplevel: String,
    pub dirty: Option<bool>,
    pub ahead: Option<i64>,
    pub behind: Option<i64>,
    pub ts: f64,
}

pub fn fnv1a64(s: &str) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in s.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

pub fn cache_path(home: &Path, toplevel: &str) -> PathBuf {
    home.join(".cache").join("csr").join("git").join(format!("{:016x}.json", fnv1a64(toplevel)))
}

pub fn read_entry(path: &Path) -> Option<Entry> {
    serde_json::from_str(&fs::read_to_string(path).ok()?).ok()
}

pub fn is_fresh(e: &Entry, now: f64) -> bool {
    let age = now - e.ts;
    (0.0..TTL_S).contains(&age)
}

fn file_age_secs(path: &Path) -> Option<f64> {
    fs::metadata(path).ok()?.modified().ok()?.elapsed().ok().map(|d| d.as_secs_f64())
}

/// Cached dirty flag (None = unknown) plus the whole entry. Kicks off a detached refresh when the
/// cache is stale or missing. Never blocks on git.
pub fn dirty_with_async_refresh(home: &Path, toplevel: &str, now: f64) -> (Option<bool>, Option<Entry>) {
    let path = cache_path(home, toplevel);
    let entry = read_entry(&path);
    if let Some(e) = &entry {
        if is_fresh(e, now) {
            return (e.dirty, entry);
        }
    }
    let inflight = path.with_extension("inflight");
    let inflight_recent = file_age_secs(&inflight).map(|a| a < INFLIGHT_MAX_AGE_S).unwrap_or(false);
    if !inflight_recent {
        if let Some(dir) = path.parent() {
            let _ = fs::create_dir_all(dir);
        }
        let _ = fs::write(&inflight, b"");
        let spawned = std::env::current_exe().ok().and_then(|exe| {
            // All three stdio to null: an inherited stdout would hold Claude Code's pipe open
            // until `git status` finishes (seconds on a big repo).
            Command::new(exe)
                .arg("git-refresh")
                .arg(toplevel)
                .env("HOME", home)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .process_group(0)
                .spawn()
                .ok()
        });
        if spawned.is_none() {
            let _ = fs::remove_file(&inflight);
        }
    }
    (entry.as_ref().and_then(|e| e.dirty), entry)
}

/// `git status --porcelain=v1 --branch` → (dirty, ahead, behind). ahead/behind are None without an
/// upstream (`...` absent from the `## ` header), 0 when in sync.
pub fn parse_status(stdout: &str) -> (bool, Option<i64>, Option<i64>) {
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
    (dirty, ahead, behind)
}

fn leading_int(s: &str) -> i64 {
    s.bytes().take_while(|b| b.is_ascii_digit()).fold(0i64, |acc, b| acc * 10 + (b - b'0') as i64)
}

fn unix_now() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

fn write_atomic(path: &Path, e: &Entry) {
    let tmp = path.with_extension(format!("tmp{}", std::process::id()));
    if let Ok(bytes) = serde_json::to_vec(e) {
        if fs::write(&tmp, bytes).is_ok() && fs::rename(&tmp, path).is_err() {
            let _ = fs::remove_file(&tmp);
        }
    }
}

/// The `csr git-refresh <toplevel>` subcommand body. Runs git (no optional locks, so a killed
/// run can never strand `.git/index.lock`), writes the cache, clears markers. Exits silently.
pub fn refresh(home: &Path, toplevel: &str) {
    let path = cache_path(home, toplevel);
    if let Some(dir) = path.parent() {
        let _ = fs::create_dir_all(dir);
    }
    let lock = path.with_extension("lock");
    if let Some(age) = file_age_secs(&lock) {
        if age < LOCK_MAX_AGE_S as f64 {
            return; // another refresh is running
        }
        let _ = fs::remove_file(&lock);
    }
    if fs::OpenOptions::new().write(true).create_new(true).open(&lock).is_err() {
        return;
    }
    let result = Command::new("git")
        .args(["-C", toplevel, "--no-optional-locks", "status", "--porcelain=v1", "--branch"])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| parse_status(&String::from_utf8_lossy(&o.stdout)));
    let entry = match result {
        Some((dirty, ahead, behind)) => Entry { toplevel: toplevel.to_string(), dirty: Some(dirty), ahead, behind, ts: unix_now() },
        None => {
            let mut e = read_entry(&path).unwrap_or_default();
            e.toplevel = toplevel.to_string();
            e.ts = unix_now();
            e
        }
    };
    write_atomic(&path, &entry);
    let _ = fs::remove_file(path.with_extension("inflight"));
    let _ = fs::remove_file(&lock);
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --lib gitcache 2>&1 | tail -5` — Expected: `4 passed`.

- [ ] **Step 5: Commit**

```bash
git add src/gitcache.rs && git commit -m "feat: git dirty cache with detached refresh"
```

---

### Task 6b: quota.rs — freshest 5h/7d reading shared across windows

**Why:** Claude Code only refreshes `rate_limits` when *that* session calls the API, so a window
left idle re-sends the same stale percentages every second. Python keeps one shared
"latest reading" file that every render merges into, so all windows show the newest number
(the one that matches the claude.ai usage page). This is the simplified port.

**Files:**
- Create: `src/quota.rs`

- [ ] **Step 1: Write the failing tests**

```rust
// src/quota.rs
#[cfg(test)]
mod tests {
    use super::*;
    use crate::format::{FIVE_HOUR_S, SEVEN_DAY_S};

    fn r(used: f64, reset: f64, at: f64) -> Option<Reading> {
        Some(Reading { used_pct: used, resets_at: reset, observed_at: at })
    }

    #[test]
    fn merge_rules() {
        let now = 1000.0;
        let len = FIVE_HOUR_S;
        // nothing stored → incoming
        assert_eq!(merge(None, r(5.0, 5000.0, now), now, len), r(5.0, 5000.0, now));
        // same reset: higher percentage wins, whichever side it is on (usage only grows in a window)
        assert_eq!(merge(r(9.0, 5000.0, 900.0), r(5.0, 5000.0, now), now, len), r(9.0, 5000.0, 900.0));
        assert_eq!(merge(r(5.0, 5000.0, 900.0), r(9.0, 5000.0, now), now, len), r(9.0, 5000.0, now));
        // later reset replaces, earlier reset is ignored
        assert_eq!(merge(r(90.0, 5000.0, 900.0), r(1.0, 23000.0, now), now, len), r(1.0, 23000.0, now));
        assert_eq!(merge(r(1.0, 23000.0, 900.0), r(90.0, 5000.0, now), now, len), r(1.0, 23000.0, 900.0));
        // a stored reading whose window already rolled over (> 60 s past reset) is dropped
        assert_eq!(merge(r(50.0, 900.0, 800.0), None, now, len), None);
        assert_eq!(merge(r(50.0, 950.0, 800.0), None, now, len), r(50.0, 950.0, 800.0)); // inside the 60 s grace
        // an implausible incoming reset (further than window + 1 day) is ignored
        assert_eq!(merge(None, r(5.0, now + len + 86400.0 + 1.0, now), now, len), None);
        assert_eq!(merge(None, r(5.0, now + len + 86400.0 - 1.0, now), now, len), r(5.0, now + len + 86400.0 - 1.0, now));
        // no stored, no incoming
        assert_eq!(merge(None, None, now, SEVEN_DAY_S), None);
    }

    #[test]
    fn reconcile_round_trips_through_the_file_and_writes_only_on_change() {
        let home = tempfile::tempdir().unwrap();
        let now = 1793060340.0;
        let five = r(1.0, 1793077200.0, now);
        let seven = r(16.0, 1793304000.0, now);
        // first render writes the store
        let (m5, m7) = reconcile(home.path(), now, five.clone(), seven.clone());
        assert_eq!((m5, m7), (five.clone(), seven.clone()));
        let path = store_path(home.path());
        let first = std::fs::read_to_string(&path).unwrap();
        // an idle window re-sending an older, lower reading gets the shared (higher) value back
        // and does not rewrite the file
        let (m5, m7) = reconcile(home.path(), now + 5.0, r(0.0, 1793077200.0, now + 5.0), r(5.0, 1793304000.0, now + 5.0));
        assert_eq!(m5.unwrap().used_pct, 1.0);
        assert_eq!(m7.unwrap().used_pct, 16.0);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), first);
        // a session that just started (no rate_limits in stdin) is backfilled from the store
        let (m5, m7) = reconcile(home.path(), now + 6.0, None, None);
        assert_eq!(m5.map(|x| x.used_pct), Some(1.0));
        assert_eq!(m7.map(|x| x.used_pct), Some(16.0));
        // a fresh higher reading updates the file
        let (m7only, _) = (reconcile(home.path(), now + 7.0, None, r(20.0, 1793304000.0, now + 7.0)).1, ());
        assert_eq!(m7only.unwrap().used_pct, 20.0);
        assert_ne!(std::fs::read_to_string(&path).unwrap(), first);
        // unreadable home: behaves as "no store", returns the incoming reading
        let (m5, _) = reconcile(std::path::Path::new("/nonexistent/csr-home"), now, five.clone(), None);
        assert_eq!(m5, five);
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --lib quota 2>&1 | tail -5` — Expected: compile errors.

- [ ] **Step 3: Implement**

```rust
// src/quota.rs
//! Account-wide freshest 5h/7d reading shared by every open Claude Code window
//! (simplified port of predict.reconcile_account / rate_latest.json).

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

use crate::format::{FIVE_HOUR_S, SEVEN_DAY_S};

/// How long past `resets_at` a reading is still believed (clock skew / late refresh).
const ROLLOVER_GRACE_S: f64 = 60.0;

#[derive(Serialize, Deserialize, Default, Clone, Debug, PartialEq)]
pub struct Reading {
    pub used_pct: f64,
    pub resets_at: f64,
    pub observed_at: f64,
}

#[derive(Serialize, Deserialize, Default, Clone, Debug, PartialEq)]
pub struct Store {
    pub five_hour: Option<Reading>,
    pub seven_day: Option<Reading>,
}

pub fn store_path(home: &Path) -> PathBuf {
    home.join(".cache").join("csr").join("rate_latest.json")
}

fn live(r: &Reading, now: f64) -> bool {
    r.resets_at > now - ROLLOVER_GRACE_S
}

fn plausible(r: &Reading, now: f64, window_len: f64) -> bool {
    live(r, now) && r.resets_at <= now + window_len + 86400.0
}

/// Merge rule for one window: drop anything rolled over or implausible; a later `resets_at`
/// replaces; the same `resets_at` keeps the higher percentage (usage only grows inside a window);
/// an earlier `resets_at` is ignored.
pub fn merge(stored: Option<Reading>, incoming: Option<Reading>, now: f64, window_len: f64) -> Option<Reading> {
    let stored = stored.filter(|r| plausible(r, now, window_len));
    let incoming = incoming.filter(|r| plausible(r, now, window_len));
    match (stored, incoming) {
        (None, i) => i,
        (s, None) => s,
        (Some(s), Some(i)) => {
            if i.resets_at > s.resets_at {
                Some(i)
            } else if i.resets_at < s.resets_at {
                Some(s)
            } else if i.used_pct > s.used_pct {
                Some(i)
            } else {
                Some(s)
            }
        }
    }
}

fn load(path: &Path) -> Store {
    fs::read_to_string(path).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

fn save(path: &Path, store: &Store) {
    let Some(dir) = path.parent() else { return };
    if fs::create_dir_all(dir).is_err() {
        return;
    }
    let tmp = path.with_extension(format!("tmp{}", std::process::id()));
    if let Ok(bytes) = serde_json::to_vec(store) {
        if fs::write(&tmp, bytes).is_ok() && fs::rename(&tmp, path).is_err() {
            let _ = fs::remove_file(&tmp);
        }
    }
}

/// Merge this render's stdin readings into the shared file and return what to display.
/// Writes only when the merged content differs from what was on disk (idle windows re-sending
/// the same numbers never touch the file). Never fails: an unreadable/unwritable home just
/// means "no shared store".
pub fn reconcile(home: &Path, now: f64, five: Option<Reading>, seven: Option<Reading>) -> (Option<Reading>, Option<Reading>) {
    let path = store_path(home);
    let on_disk = load(&path);
    let merged = Store {
        five_hour: merge(on_disk.five_hour.clone(), five, now, FIVE_HOUR_S),
        seven_day: merge(on_disk.seven_day.clone(), seven, now, SEVEN_DAY_S),
    };
    if merged != on_disk {
        save(&path, &merged);
    }
    (merged.five_hour, merged.seven_day)
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --lib quota 2>&1 | tail -5` — Expected: `2 passed`.

- [ ] **Step 5: Commit**

```bash
git add src/quota.rs && git commit -m "feat: shared freshest quota reading across windows"
```

---

### Task 7: line1.rs — the quota / model / cache line

**Files:**
- Create: `src/line1.rs`

- [ ] **Step 1: Write the failing tests**

The expected strings are literal substrings of `tests/fixtures/golden-orbit.raw` (first line).

```rust
// src/line1.rs
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
        assert_eq!(got, format!("{MUTE}7d\x1b[0m {} {MUTE}--%\x1b[0m {EDGE}⏰{MUTE}--\x1b[0m", bar(&c, None, 12)));
    }

    #[test]
    fn full_line_matches_golden_first_line() {
        let c = cfg();
        let golden = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/golden-orbit.raw")).unwrap();
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
            msgs_pct: None, weekly_pct: None, reset_5h: "--", reset_7d: "", countdown_emoji: "",
            projection_5h: "", projection_7d: "", model: "Opus 5.5 (1M context)", ctx_pct: None,
            cost_text: "1.53", cache_text: "COLD",
        };
        let out = render_line1(&c, &inp);
        assert!(out.contains("\x1b[38;2;200;134;171mOpus 5.5 (1M context)\x1b[0m"));
        assert!(out.contains(&format!("{MUTE}$ \x1b[38;2;218;221;225m1.53\x1b[0m")));
        assert!(out.ends_with("\x1b[38;2;232;116;116m❄ COLD\x1b[0m"));
        assert_eq!(out.matches('│').count(), 3); // 4 parts (5h, model, cost, cache) → 3 separators
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --lib line1 2>&1 | tail -5` — Expected: compile errors.

- [ ] **Step 3: Implement**

```rust
// src/line1.rs
//! Aurora first line: `5h ■■■ 1% ⏰4h40m →12% │ 7d … │ Model ■■■■■■ 8% │ ❄ 49m` (port of styles.render_aurora).

use crate::config::Config;
use crate::format::{aurora_cache_text, projection_color, round_even, window_severity_rgb};
use crate::theme::{blend, fg, severity, Rgb, Theme, BOLD, RESET};

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
            let tt = if width > 1 { i as f64 / (width - 1) as f64 } else { 0.0 };
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
        Some(p) => format!("{}{BOLD}{:>2}%{RESET}", fg(rgb.unwrap_or(t.ink)), round_even(p)),
    }
}

/// One quota window segment: label, 12-cell bar, percent, ⏰reset(+emoji), optional projection chip.
pub fn window(cfg: &Config, label: &str, p: Option<f64>, chip: &str, reset: &str, emoji: &str) -> String {
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

/// `\s*\([^()\n]*\)$` — the trailing parenthesised token suffix aurora drops next to the ctx bar.
fn strip_paren_suffix(model: &str) -> &str {
    if let Some(stripped) = model.strip_suffix(')') {
        if let Some(i) = stripped.rfind('(') {
            if !stripped[i + 1..].contains(')') && !stripped[i + 1..].contains('\n') {
                return model[..i].trim_end();
            }
        }
    }
    model
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
    let sep = format!("{pad}{edge}│{RESET}{pad}", pad = cfg.pad);
    let mut parts: Vec<String> = Vec::with_capacity(5);

    parts.push(window(cfg, "5h", inp.msgs_pct, inp.projection_5h, inp.reset_5h, inp.countdown_emoji));
    if cfg.show_weekly {
        parts.push(window(cfg, "7d", inp.weekly_pct, inp.projection_7d, inp.reset_7d, ""));
    }

    // A rose between hot and the worktree hue: reads as "the model", never as a severity.
    let accent = fg(blend(t.s_hot, t.wt, 0.5));
    match inp.ctx_pct {
        Some(c) => {
            let name = strip_paren_suffix(inp.model);
            let rgb = severity(t, c, 70.0, 85.0);
            parts.push(format!("{accent}{name}{RESET} {} {}{}%{RESET}", bar(cfg, Some(c), 6), fg(rgb), round_even(c)));
        }
        None => parts.push(format!("{accent}{}{RESET}", inp.model)),
    }

    if !inp.cost_text.is_empty() {
        parts.push(format!("{mute}$ {ink}{}{RESET}", inp.cost_text));
    }
    if !inp.cache_text.is_empty() {
        parts.push(format!("{}❄ {}{RESET}", fg(cache_color(t, inp.cache_text)), aurora_cache_text(inp.cache_text)));
    }
    parts.join(&sep)
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --lib line1 2>&1 | tail -5` — Expected: `4 passed`. If `full_line_matches_golden_first_line` fails, print both strings with `{:?}` and diff them escape by escape; the usual culprits are the `{:>2}` width on the percent, a missing space before the chip, or the ctx bar's `%` having no bold.

- [ ] **Step 5: Commit**

```bash
git add src/line1.rs && git commit -m "feat: aurora quota/model/cache line"
```

---

### Task 8: line2.rs — the branch / project identity line

**Files:**
- Create: `src/line2.rs`

- [ ] **Step 1: Write the failing tests**

```rust
// src/line2.rs
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
        let golden = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/golden-orbit.raw")).unwrap();
        let second = golden.lines().nth(1).unwrap();
        let info = orbit();
        let inp = Line2Input { info: &info, dirty: Some(true), ahead: None, behind: None, lines_text: "+965 -253", duration_text: "" };
        assert_eq!(render_line2(&Config::default(), &inp), second);
    }

    #[test]
    fn variants() {
        let c = Config::default();
        let mut info = orbit();
        info.worktree_count = 0;
        info.web_url = None;
        let plain = |s: &str| crate::width::segments(s).iter().filter_map(|seg| match seg { crate::width::Segment::Text(t) => Some(*t), _ => None }).collect::<String>();
        let inp = Line2Input { info: &info, dirty: Some(false), ahead: Some(2), behind: Some(1), lines_text: "", duration_text: "5m" };
        assert_eq!(plain(&render_line2(&c, &inp)), "⎇ development ↑2↓1  ·  orbit  ·  ⏱ 5m");
        // dirty unknown → no dot; detached → italic sha, no link
        info.detached = true;
        info.branch = Some("abc1234".into());
        info.web_url = Some("https://github.com/o/r".into());
        let inp = Line2Input { info: &info, dirty: None, ahead: None, behind: None, lines_text: "", duration_text: "" };
        let out = render_line2(&c, &inp);
        assert_eq!(plain(&out), "⎇ abc1234  ·  orbit");
        assert!(out.contains("\x1b[38;2;120;125;132m\x1b[3mabc1234\x1b[0m"));
        assert!(!out.contains("/tree/"));
        assert!(out.contains("\x1b]8;;https://github.com/o/r\x07"));
        // linked worktree leads the line; main-checkout marker absent
        let wt = Identity { is_worktree: true, worktree_name: Some("feature-x".into()), worktree_count: 2, detached: false, branch: Some("feature-x".into()), ..orbit() };
        let inp = Line2Input { info: &wt, dirty: Some(true), ahead: None, behind: None, lines_text: "", duration_text: "" };
        assert_eq!(plain(&render_line2(&c, &inp)), "worktree (🌳2)  ·  ⎇ feature-x ●  ·  orbit");
        let wt2 = Identity { worktree_name: Some("orbit-fix".into()), ..wt.clone() };
        assert_eq!(worktree_label(&wt2, "🌳"), "fix (🌳2)");
        // no git
        let ng = Identity { project_name: "notes".into(), ..Identity::default() };
        let inp = Line2Input { info: &ng, dirty: None, ahead: None, behind: None, lines_text: "", duration_text: "" };
        assert_eq!(plain(&render_line2(&c, &inp)), "notes  ·  no git");
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --lib line2 2>&1 | tail -5` — Expected: compile errors.

- [ ] **Step 3: Implement**

```rust
// src/line2.rs
//! Aurora identity line: `⎇ main ●↓1  ·  project  ·  +244 −3  ·  ⏱ 4m  ·  worktree (🌳1)`
//! (port of styles.render_identity_line_aurora). Muted palette: line 1 carries the color.

use crate::config::Config;
use crate::identity::{branch_web_url, strip_repo_prefix, Identity};
use crate::theme::{blend, fg, Rgb, ITAL, RESET};

pub struct Line2Input<'a> {
    pub info: &'a Identity,
    pub dirty: Option<bool>,
    pub ahead: Option<i64>,
    pub behind: Option<i64>,
    pub lines_text: &'a str,
    pub duration_text: &'a str,
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
    let rest = u.strip_prefix("https://").or_else(|| u.strip_prefix("http://"));
    matches!(rest, Some(r) if !r.is_empty() && u.bytes().all(|b| (0x21..=0x7e).contains(&b)))
}

fn ahead_behind_glyphs(ahead: Option<i64>, behind: Option<i64>) -> String {
    let mut s = String::new();
    if let Some(a) = ahead.filter(|a| *a != 0) {
        s.push_str(&format!("↑{a}"));
    }
    if let Some(b) = behind.filter(|b| *b != 0) {
        s.push_str(&format!("↓{b}"));
    }
    s
}

/// Display name of a linked worktree: its name minus a redundant repo prefix, collapsing to
/// `worktree` when unknown or when it would only repeat the repo / branch; `(🌳N)` appended.
pub fn worktree_label(info: &Identity, glyph: &str) -> String {
    let raw = info.worktree_name.as_deref().unwrap_or("").trim();
    let project = info.project_name.trim();
    let mut name = if raw.is_empty() { "worktree".to_string() } else { strip_repo_prefix(raw, project) };
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
        let branch = info.branch.as_deref().unwrap_or("?");
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
        segs.push(c(&dim, &format!("worktree ({}{})", cfg.worktree_glyph, info.worktree_count)));
    }
    segs.join(&c(&edge, QUIET_SEP))
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --lib line2 2>&1 | tail -5` — Expected: `2 passed`.

- [ ] **Step 5: Commit**

```bash
git add src/line2.rs && git commit -m "feat: aurora branch/project identity line"
```

---

### Task 9: render.rs + main.rs — glue, degradation, CLI

**Files:**
- Create: `src/render.rs`
- Modify: `src/main.rs` (replace the stub)

- [ ] **Step 1: Write the failing tests**

```rust
// src/render.rs
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_context_suffix_anywhere() {
        assert_eq!(strip_context_suffix("Sonnet 4.5 (1M context)"), "Sonnet 4.5");
        assert_eq!(strip_context_suffix("Opus 5.5"), "Opus 5.5");
        assert_eq!(strip_context_suffix("X (200k context) Y"), "X Y");
        assert_eq!(strip_context_suffix("X (beta)"), "X (beta)");
    }

    #[test]
    fn waiting_payload_renders_dashes_and_no_chips() {
        let p: crate::payload::Payload = serde_json::from_str(r#"{"model":{"display_name":"Opus 5.5"},"workspace":{"current_dir":"/nonexistent/x"}}"#).unwrap();
        let cfg = crate::config::Config::default();
        let home = tempfile::tempdir().unwrap();
        let env = Env { now: 1000.0, home: home.path().to_path_buf(), columns: None };
        let out = render(&p, &cfg, &env);
        let plain = strip_ansi(&out);
        assert_eq!(plain, "5h ■■■■■■■■■■■■ --% ⏰-- │ 7d ■■■■■■■■■■■■ --% ⏰-- │ Opus 5.5\nx  ·  no git");
    }

    #[test]
    fn degrades_cache_then_cost_then_chips_before_clipping() {
        let raw = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/stdin-orbit.json")).unwrap();
        let p: crate::payload::Payload = serde_json::from_str(&raw).unwrap();
        let mut cfg = crate::config::Config::default();
        cfg.show_project_branch = false;
        cfg.show_cost = true;
        let now: f64 = 1793060340.047922117;
        let home = tempfile::tempdir().unwrap();
        let env = |columns: Option<usize>| Env { now, home: home.path().to_path_buf(), columns };
        let full = strip_ansi(&render(&p, &cfg, &env(None)));
        assert!(full.contains("❄") && full.contains("$ 1.53") && full.contains("→"));
        let w = crate::width::visible_width(&full);
        let no_cache = strip_ansi(&render(&p, &cfg, &env(Some(w - 1))));
        assert!(!no_cache.contains("❄") && no_cache.contains("$ 1.53"));
        let no_cost = strip_ansi(&render(&p, &cfg, &env(Some(w - 10))));
        assert!(!no_cost.contains("$ ") && no_cost.contains("→"));
        let no_chips = strip_ansi(&render(&p, &cfg, &env(Some(w - 20))));
        assert!(!no_chips.contains("→"));
        let clipped = strip_ansi(&render(&p, &cfg, &env(Some(20))));
        assert_eq!(crate::width::visible_width(&clipped), 20);
        assert!(clipped.ends_with('…'));
    }

    #[test]
    fn new_session_without_rate_limits_is_backfilled_from_the_shared_store() {
        let home = tempfile::tempdir().unwrap();
        let now: f64 = 1793060340.047922117;
        let mut cfg = crate::config::Config::default();
        cfg.show_project_branch = false;
        // a render with quota fills the store …
        let raw = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/stdin-orbit.json")).unwrap();
        let p: crate::payload::Payload = serde_json::from_str(&raw).unwrap();
        render(&p, &cfg, &Env { now, home: home.path().to_path_buf(), columns: None });
        // … and a fresh session with no rate_limits shows the shared numbers instead of `--`
        let p2: crate::payload::Payload = serde_json::from_str(r#"{"model":{"display_name":"Opus 5.5"}}"#).unwrap();
        let out = strip_ansi(&render(&p2, &cfg, &Env { now: now + 2.0, home: home.path().to_path_buf(), columns: None }));
        assert!(out.starts_with("5h ■■■■■■■■■■■■  1% ⏰4h40m →"), "{out}");
        assert!(out.contains("7d ■■■■■■■■■■■■ 16% ⏰2d19h →"), "{out}");
    }

    fn strip_ansi(s: &str) -> String {
        crate::width::segments(s).iter().filter_map(|seg| match seg { crate::width::Segment::Text(t) => Some(*t), _ => None }).collect()
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --lib render 2>&1 | tail -5` — Expected: compile errors.

Note for the golden test (Task 10) and these tests: every render now reads/writes
`$HOME/.cache/csr/rate_latest.json`, which is why they all run under a `tempfile::tempdir()` home.

- [ ] **Step 3: Implement render.rs**

```rust
// src/render.rs
//! payload + config + environment → the final status text (port of core.main's render branch).

use std::path::PathBuf;

use crate::config::Config;
use crate::format::{
    cache_remaining, countdown_emoji, format_duration_short, format_lines, projection_chip, reset_5h, reset_7d,
    FIVE_HOUR_S, MIN_ELAPSED_5H, MIN_ELAPSED_7D, SEVEN_DAY_S,
};
use crate::gitcache::dirty_with_async_refresh;
use crate::identity::resolve_identity;
use crate::line1::{render_line1, Line1Input};
use crate::line2::{render_line2, Line2Input};
use crate::payload::{Payload, Window};
use crate::quota::{reconcile, Reading};
use crate::width::{clip_lines, visible_width};

pub struct Env {
    pub now: f64,
    pub home: PathBuf,
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
            out.push_str(&rest[..open + close_rel + 1]);
            rest = &rest[open + close_rel + 1..];
        }
    }
    out.push_str(rest);
    out
}

fn window_values(w: Option<&Window>) -> (Option<f64>, Option<f64>) {
    match w {
        Some(w) => (Some(w.pct()), w.reset_time()),
        None => (None, None),
    }
}

/// A stdin window as a shareable reading (needs a reset time; a window without one is still
/// displayed verbatim through `window_values`, it just never enters the shared store).
fn reading(w: Option<&Window>, now: f64) -> Option<Reading> {
    let w = w?;
    Some(Reading { used_pct: w.pct(), resets_at: w.reset_time()?, observed_at: now })
}

pub fn render(p: &Payload, cfg: &Config, env: &Env) -> String {
    let in5 = p.rate_limits.as_ref().and_then(|r| r.five_hour.as_ref());
    let in7 = p.rate_limits.as_ref().and_then(|r| r.seven_day.as_ref());
    // Share the freshest reading with every other open window (Task 6b); fall back to this
    // stdin verbatim when the store has nothing usable (Python trusts fresh stdin the same way).
    let (m5, m7) = reconcile(&env.home, env.now, reading(in5, env.now), reading(in7, env.now));
    let (pct5, reset5_at) = match m5 {
        Some(r) => (Some(r.used_pct), Some(r.resets_at)),
        None => window_values(in5),
    };
    let (pct7, reset7_at) = match m7 {
        Some(r) => (Some(r.used_pct), Some(r.resets_at)),
        None => window_values(in7),
    };
    let has_official = pct5.is_some() || pct7.is_some();

    let (reset_5h_text, minutes) = if has_official { reset_5h(reset5_at, env.now) } else { ("--".to_string(), None) };
    let reset_7d_text = if has_official { reset_7d(reset7_at, env.now) } else { String::new() };
    let emoji = if has_official { countdown_emoji(minutes) } else { "" };
    let (mut proj5, mut proj7) = if has_official && cfg.show_projection {
        (
            projection_chip(FIVE_HOUR_S, MIN_ELAPSED_5H, pct5, reset5_at, env.now),
            projection_chip(SEVEN_DAY_S, MIN_ELAPSED_7D, pct7, reset7_at, env.now),
        )
    } else {
        (String::new(), String::new())
    };

    let display = if p.model.display_name.is_empty() || p.model.display_name == "Unknown" {
        if p.model.id.is_empty() { "Unknown".to_string() } else { p.model.id.clone() }
    } else {
        p.model.display_name.clone()
    };
    let (ctx_pct, model) = match &p.context_window {
        Some(cw) if cw.context_window_size > 0.0 => (cw.ctx_pct(), strip_context_suffix(&display)),
        _ => (None, display),
    };

    let mut cost_text = if cfg.show_cost && p.cost.total_cost_usd >= 0.0 { format!("{:.2}", p.cost.total_cost_usd) } else { String::new() };
    let mut cache_text = match p.prompt_cache.as_ref().and_then(|c| c.expires_at) {
        Some(exp) if cfg.show_cache_age => cache_remaining(exp - env.now),
        _ => String::new(),
    };
    let lines_text = if cfg.show_lines { format_lines(p.cost.total_lines_added, p.cost.total_lines_removed) } else { String::new() };
    let duration_text = if cfg.show_duration { format_duration_short(p.cost.total_duration_ms) } else { String::new() };

    let line1 = |cost: &str, cache: &str, p5: &str, p7: &str| {
        render_line1(
            cfg,
            &Line1Input {
                msgs_pct: pct5,
                weekly_pct: pct7,
                reset_5h: &reset_5h_text,
                reset_7d: &reset_7d_text,
                countdown_emoji: emoji,
                projection_5h: p5,
                projection_7d: p7,
                model: &model,
                ctx_pct,
                cost_text: cost,
                cache_text: cache,
            },
        )
    };
    let mut first = line1(&cost_text, &cache_text, &proj5, &proj7);
    // narrow_layout "drop": shed optional segments least-important-first until line 1 fits.
    if let Some(max) = env.columns {
        if visible_width(&first) > max && !cache_text.is_empty() {
            cache_text.clear();
            first = line1(&cost_text, &cache_text, &proj5, &proj7);
        }
        if visible_width(&first) > max && !cost_text.is_empty() {
            cost_text.clear();
            first = line1(&cost_text, &cache_text, &proj5, &proj7);
        }
        if visible_width(&first) > max && !(proj5.is_empty() && proj7.is_empty()) {
            proj5.clear();
            proj7.clear();
            first = line1(&cost_text, &cache_text, &proj5, &proj7);
        }
    }

    let mut out = first;
    if cfg.show_project_branch {
        let info = resolve_identity(p);
        let (dirty, entry) = match &info.toplevel {
            Some(t) => dirty_with_async_refresh(&env.home, &t.to_string_lossy(), env.now),
            None => (None, None),
        };
        let (ahead, behind) = if cfg.show_ahead_behind {
            entry.map(|e| (e.ahead, e.behind)).unwrap_or((None, None))
        } else {
            (None, None)
        };
        out.push('\n');
        out.push_str(&render_line2(
            cfg,
            &Line2Input { info: &info, dirty, ahead, behind, lines_text: &lines_text, duration_text: &duration_text },
        ));
    }
    clip_lines(&out, env.columns)
}
```

- [ ] **Step 4: Implement main.rs**

```rust
// src/main.rs
use std::io::{Read, Write};
use std::path::PathBuf;

use csr::config::{config_path, Config};
use csr::payload::Payload;
use csr::render::{render, Env};

fn home() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/"))
}

fn now() -> f64 {
    if let Some(v) = std::env::var("CSR_NOW").ok().and_then(|s| s.trim().parse::<f64>().ok()) {
        return v;
    }
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

fn columns(p: &Payload) -> Option<usize> {
    p.cs_env
        .as_ref()
        .and_then(|m| m.get("COLUMNS").cloned())
        .or_else(|| std::env::var("COLUMNS").ok())
        .and_then(|s| s.trim().parse::<usize>().ok())
        .filter(|c| *c > 0)
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("git-refresh") => {
            if let Some(toplevel) = args.get(2) {
                csr::gitcache::refresh(&home(), toplevel);
            }
        }
        Some("--version") | Some("-V") => println!("csr {}", env!("CARGO_PKG_VERSION")),
        Some("--help") | Some("-h") => println!("usage: csr            render the status line from Claude Code's stdin JSON\n       csr git-refresh <toplevel>   (internal) refresh the dirty cache"),
        _ => {
            let mut raw = String::new();
            let _ = std::io::stdin().read_to_string(&mut raw);
            let home = home();
            let cfg = Config::load(&config_path(&home));
            let out = match serde_json::from_str::<Payload>(&raw) {
                Ok(p) if !raw.trim().is_empty() => {
                    let env = Env { now: now(), home, columns: columns(&p) };
                    render(&p, &cfg, &env)
                }
                _ => "⚠ Run inside Claude Code statusLine for rate-limit data".to_string(),
            };
            let stdout = std::io::stdout();
            let mut lock = stdout.lock();
            let _ = lock.write_all(out.as_bytes());
            let _ = lock.write_all(b"\n");
            let _ = lock.flush();
        }
    }
}
```

- [ ] **Step 5: Run all unit tests**

Run: `cargo test --lib 2>&1 | tail -5` — Expected: all pass (`30 passed` or so; the exact count is not important, zero failures is).

- [ ] **Step 6: Commit**

```bash
git add src/render.rs src/main.rs && git commit -m "feat: render pipeline, width degradation and csr CLI"
```

---

### Task 10: Golden integration test against the Python render

**Files:**
- Create: `tests/golden.rs`

- [ ] **Step 1: Write the test**

```rust
// tests/golden.rs
//! Runs the built `csr` binary on tests/fixtures/stdin-orbit.json at the instant recorded in
//! golden-orbit.now and compares to what Python claude-statusbar 3.52.2 rendered. The only
//! masked part is the `→NN%` projection chip (csr uses the simple average-pace formula).
//!
//! Depends on the real repo /home/someone/dev/example/orbit (branch `development`, origin
//! github.com/exampleco/orbit, exactly one live linked worktree). Re-capture the golden with the
//! command in README.md if that repo changes.

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
    let now = fs::read_to_string(format!("{root}/tests/fixtures/golden-orbit.now")).unwrap().trim().to_string();

    // Isolated HOME: no user config, and a pre-seeded fresh dirty cache so no refresh is spawned
    // and the ● renders (Python's golden had its cache filled by a prior run).
    let home = tempfile::tempdir().unwrap();
    let toplevel = "/home/someone/dev/example/orbit";
    let cache = csr::gitcache::cache_path(home.path(), toplevel);
    fs::create_dir_all(cache.parent().unwrap()).unwrap();
    fs::write(&cache, format!(r#"{{"toplevel":"{toplevel}","dirty":true,"ahead":0,"behind":3,"ts":{now}}}"#)).unwrap();

    let mut child = Command::new(env!("CARGO_BIN_EXE_csr"))
        .env_clear()
        .env("HOME", home.path())
        .env("PATH", "/usr/bin:/bin")
        .env("CSR_NOW", &now)
        .env("COLUMNS", "182")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(fixture.as_bytes()).unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(out.status.success());
    let got = String::from_utf8(out.stdout).unwrap();

    let got_lines: Vec<String> = got.trim_end_matches('\n').lines().map(mask_projection).collect();
    let want_lines: Vec<String> = golden.trim_end_matches('\n').lines().map(mask_projection).collect();
    assert_eq!(got_lines.len(), 2, "got:\n{got:?}");
    for (i, (g, w)) in got_lines.iter().zip(want_lines.iter()).enumerate() {
        assert_eq!(g, w, "line {} differs\n got: {:?}\nwant: {:?}", i + 1, g, w);
    }
    assert!(!cache.with_extension("inflight").exists(), "fresh cache must not spawn a refresh");
}
```

- [ ] **Step 2: Run the golden test**

Run: `cargo test --test golden 2>&1 | tail -15`
Expected: `1 passed`. If it fails on line 2 with the `worktree (🌳1)` segment missing or counting differently, check `git -C /home/someone/dev/example/orbit worktree list` — the golden assumed exactly one linked worktree; if the machine changed, re-capture per Task 11 instead of editing the test.

- [ ] **Step 3: Commit**

```bash
git add tests/golden.rs && git commit -m "test: golden comparison against the Python render"
```

---

### Task 11: README, release build, acceptance numbers

**Files:**
- Create: `README.md`

- [ ] **Step 1: Write README.md**

````markdown
# csr — fast Claude Code status line

Single-binary Rust port of the `aurora` / `graphite` look of
[claude-statusbar](https://pypi.org/project/claude-statusbar/) 3.52.2. No daemon, no network,
no transcript scanning: one process per render that reads Claude Code's stdin JSON and a few
small git files, prints two lines, exits.

```
5h ■■■■■■■■■■■■  1% ⏰4h40m →16% │ 7d ■■■■■■■■■■■■ 16% ⏰2d19h →27% │ Fable 5.1 ■■■■■■ 8% │ ❄ 49m
⎇ development ●  ·  orbit  ·  +965 −253  ·  worktree (🌳1)
```

## Build

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cargo build --release          # → target/release/csr
cargo test --release
```

## Use as the Claude Code status line

In `~/.claude/settings.json` (replace the existing `statusLine` block; stop the Python daemon
first with `cs daemon stop` if it is running):

```json
"statusLine": { "type": "command", "command": "/home/someone/dev/projects/claude-statusbar-rs/target/release/csr", "refreshInterval": 1 }
```

## What it reads

- stdin: `rate_limits`, `model`, `context_window`, `prompt_cache.expires_at`, `cost`, `workspace`, `cwd`.
- `$HOME/.claude/claude-statusbar.json` (same file as the Python tool, subset of keys:
  `bar_glyph`, `density`, `show_weekly`, `show_cost`, `show_cache_age`, `show_project_branch`,
  `show_lines`, `show_duration`, `show_ahead_behind`, `show_projection`, `worktree_glyph`,
  `color_ok/warn/hot/worktree`, `warning_threshold`, `critical_threshold`). `CSR_CONFIG` overrides the path.
- git: `.git/HEAD`, `.git/worktrees/*/gitdir`, `.git/config` (origin URL for the links).
- `$HOME/.cache/csr/git/<hash>.json`: dirty / ahead / behind, refreshed at most every 30 s by a
  detached `csr git-refresh <toplevel>` child running `git --no-optional-locks status`.
- `$HOME/.cache/csr/rate_latest.json`: the freshest 5h/7d reading seen by any open Claude Code
  window (Claude Code only refreshes a session's quota when that session calls the API, so an
  idle window would otherwise show stale numbers). Merge rule: later reset wins; same reset keeps
  the higher percentage.

`CSR_NOW=<unix seconds>` freezes the clock (tests only).

## Differences from the Python tool

- `→NN%` projection = plain average pace over the window so far (no smoothing, no cross-session store).
- The shared quota file is one account only and has no "limits were raised" downgrade grace
  (a lower percentage with the same reset time is kept at the higher value until the window
  rolls over); cache countdown only from `prompt_cache.expires_at`.
- Not rendered: language coach, relay balance, BYPASS, ocs, IP/fingerprint risk, mode line,
  todos/tools/agents, version/update hint, `New looks` hint.

## Re-capturing the golden fixture

Only when the orbit repo state changes (branch, remote, worktree count):

```bash
P=/home/someone/dev/projects/claude-statusbar-rs
FAKEHOME=$(mktemp -d)
HOME=$FAKEHOME COLUMNS=182 cs < $P/tests/fixtures/stdin-orbit.json > /dev/null   # fills the git cache
sleep 6
date +%s.%N > $P/tests/fixtures/golden-orbit.now
HOME=$FAKEHOME COLUMNS=182 cs < $P/tests/fixtures/stdin-orbit.json | grep -v 'New looks' > $P/tests/fixtures/golden-orbit.raw
```
````

- [ ] **Step 2: Release build and full test run**

Run:
```bash
cargo build --release 2>&1 | tail -2 && cargo test --release 2>&1 | grep -E "^test result|FAILED|panicked" ; ls -la target/release/csr
```
Expected: every `test result: ok`, binary present (expect roughly 0.5–1.5 MB).

- [ ] **Step 3: Acceptance numbers (record the output in the final report)**

```bash
cd /home/someone/dev/projects/claude-statusbar-rs
# memory (must be < 10240 kB) and a coarse wall time
HOME=$(mktemp -d) /usr/bin/time -f "maxRSS=%MkB wall=%es" ./target/release/csr < tests/fixtures/stdin-orbit.json
# wall time, 100 runs (mean and max must be < 10 ms; the Python harness adds ~1 ms of its own)
python3 - <<'EOF'
import subprocess, time, statistics
d = open('tests/fixtures/stdin-orbit.json', 'rb').read()
ts = []
for _ in range(100):
    t = time.perf_counter()
    subprocess.run(['./target/release/csr'], input=d, stdout=subprocess.DEVNULL, env={'HOME': '/tmp/csr-bench', 'COLUMNS': '182', 'PATH': '/usr/bin:/bin'})
    ts.append((time.perf_counter() - t) * 1000)
print(f"mean={statistics.mean(ts):.2f}ms  p50={statistics.median(ts):.2f}ms  max={max(ts):.2f}ms")
EOF
# dirty-dot flow with a real (temp) HOME: 1st run spawns the refresh, 2nd run shows ●
rm -rf /tmp/csr-bench; HOME=/tmp/csr-bench COLUMNS=182 ./target/release/csr < tests/fixtures/stdin-orbit.json | sed -n 2p | sed 's/\x1b\[[0-9;]*m//g; s/\x1b\]8;;[^\x07]*\x07//g'
sleep 6
HOME=/tmp/csr-bench COLUMNS=182 ./target/release/csr < tests/fixtures/stdin-orbit.json | sed -n 2p | sed 's/\x1b\[[0-9;]*m//g; s/\x1b\]8;;[^\x07]*\x07//g'
cat /tmp/csr-bench/.cache/csr/git/*.json; echo
```
Expected: first line-2 print has no `●`; the second has `⎇ development ●`; the cache file shows `"dirty":true`.

- [ ] **Step 4: Commit**

```bash
git add README.md && git commit -m "docs: README with usage, deviations and golden re-capture"
```

---

## Self-review notes (done while writing)

- Spec coverage: both lines (Tasks 7–8), stdin + config (3), git facts without blocking (5–6), shared quota reading across windows (6b), width clipping + degradation (4, 9), timing/memory acceptance (11), golden fidelity (10). The projection simplification and the skipped segments are listed under *Deliberate deviations*.
- Names used consistently: `Config` fields (`thin`, `pad`, `show_*`, `worktree_glyph`, `theme`, `warning`, `critical`), `Payload`/`Window::pct()`/`Window::reset_time()`/`ContextWindow::ctx_pct()`, `Identity`, `Line1Input`, `Line2Input`, `Env`, `gitcache::{cache_path, read_entry, is_fresh, refresh, dirty_with_async_refresh, parse_status}`, `width::{segments, Segment, visible_width, clip_line, clip_lines}`.
- Relative-time tests add a 0.5 s margin (or use an integer `now`) because the fixture instant is fractional and `(now + k) - now` is not exact in f64.
