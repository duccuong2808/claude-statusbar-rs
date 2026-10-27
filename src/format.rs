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

/// The 5-hour window's reset countdown.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reset5h {
    /// `{h}h{mm}m`, `{m}m`, or `--` when there is no reset time.
    pub text: String,
    /// Minutes to reset; None when there is no reset time.
    pub minutes: Option<i64>,
}

impl Reset5h {
    /// Shown when no reset time is known.
    #[must_use]
    pub fn unknown() -> Self {
        Reset5h {
            text: "--".to_string(),
            minutes: None,
        }
    }
}

/// core.py: `total_min = max(0, int(diff.total_seconds() / 60))` → `{h}h{mm}m` or `{m}m`.
#[must_use]
pub fn reset_5h(resets_at: Option<f64>, now: f64) -> Reset5h {
    let Some(r) = resets_at else {
        return Reset5h::unknown();
    };
    let total_min = (((r - now) / 60.0).trunc() as i64).max(0);
    let (h, m) = (total_min / 60, total_min % 60);
    let text = if h > 0 {
        format!("{h}h{m:02}m")
    } else {
        format!("{m}m")
    };
    Reset5h {
        text,
        minutes: Some(total_min),
    }
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

/// Prompt-cache countdown in whole minutes: "COLD" once gone, else `XhMMm` / `Mm` / `<1m`.
/// Minutes, not Python's `MmSSs`, because the status line refreshes only once a minute (Claude
/// Code re-renders by itself at `expires_at`). Floored, so a render up to a minute old is never
/// more than a minute off.
pub fn cache_remaining(remaining: f64) -> String {
    if remaining <= 0.0 {
        return "COLD".to_string();
    }
    let m = remaining as i64 / 60;
    if m >= 60 {
        format!("{}h{:02}m", m / 60, m % 60)
    } else if m > 0 {
        format!("{m}m")
    } else {
        "<1m".to_string()
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
        if m != 0 {
            format!("{h}h{m:02}m")
        } else {
            format!("{h}h")
        }
    } else {
        format!("{}d{}h", s / 86400, (s % 86400) / 3600)
    }
}

/// Simplified end-of-window projection (DEVIATION from Python's smoothed predictor):
/// average pace so far → `→NN%`; `→--` when too early or no usage; "" when inputs missing;
/// `·<eta>` appended when the pace overshoots the cap (predict._depletion_eta_seconds).
pub fn projection_chip(
    window_len: f64,
    min_elapsed: f64,
    used: Option<f64>,
    resets_at: Option<f64>,
    now: f64,
) -> String {
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
    if eta < ttr {
        Some(eta)
    } else {
        None
    }
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
pub fn window_severity_rgb(
    t: &Theme,
    current_pct: Option<f64>,
    chip: &str,
    warning: f64,
    critical: f64,
) -> Option<Rgb> {
    if let Some(p) = projection_pct(chip) {
        return Some(severity(t, p, PROJECTION_WARNING, PROJECTION_CRITICAL));
    }
    current_pct.map(|p| severity(t, p, warning, critical))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::GRAPHITE;

    const NOW: f64 = 1793060340.0479221; // tests/fixtures/golden-orbit.now (1793060340.047922117) as f64

    fn r5(text: &str, minutes: Option<i64>) -> Reset5h {
        Reset5h {
            text: text.to_string(),
            minutes,
        }
    }

    #[test]
    fn reset_5h_formats() {
        // 1793077200 − NOW = 16859.95 s → int(16859.95/60) = 280 min → 4h40m (NOT 4h41m: truncation)
        assert_eq!(reset_5h(Some(1793077200.0), NOW), r5("4h40m", Some(280)));
        // +0.5 s margins: NOW is fractional, so `(NOW + k) - NOW` may land a hair under k in f64
        assert_eq!(
            reset_5h(Some(NOW + 59.0 * 60.0 + 0.5), NOW),
            r5("59m", Some(59))
        );
        assert_eq!(reset_5h(Some(NOW - 10.0), NOW), r5("0m", Some(0)));
        assert_eq!(reset_5h(None, NOW), r5("--", None));
    }

    #[test]
    fn reset_7d_formats() {
        // 1793304000 − NOW = 243659.95 s → 2d 19h 40m → "2d19h"
        assert_eq!(reset_7d(Some(1793304000.0), NOW), "2d19h");
        assert_eq!(
            reset_7d(Some(NOW + 3.0 * 3600.0 + 5.0 * 60.0 + 0.5), NOW),
            "3h05m"
        );
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
        assert_eq!(cache_remaining(2947.95), "49m"); // floor
        assert_eq!(cache_remaining(3600.0), "1h00m");
        assert_eq!(cache_remaining(3725.0), "1h02m");
        assert_eq!(cache_remaining(299.5), "4m");
        assert_eq!(cache_remaining(60.0), "1m");
        assert_eq!(cache_remaining(45.0), "<1m");
        assert_eq!(cache_remaining(0.2), "<1m");
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
        assert_eq!(
            projection_chip(
                FIVE_HOUR_S,
                MIN_ELAPSED_5H,
                Some(1.0),
                Some(1793077200.0),
                NOW
            ),
            "→16%"
        );
        // 7d: used 16%, 243659.95 s to reset → elapsed 361140.05 → 26.78 → →27%
        assert_eq!(
            projection_chip(
                SEVEN_DAY_S,
                MIN_ELAPSED_7D,
                Some(16.0),
                Some(1793304000.0),
                NOW
            ),
            "→27%"
        );
        // too early / no usage → placeholder; missing input → no chip
        assert_eq!(
            projection_chip(
                FIVE_HOUR_S,
                MIN_ELAPSED_5H,
                Some(3.0),
                Some(NOW + FIVE_HOUR_S - 100.0),
                NOW
            ),
            "→--"
        );
        assert_eq!(
            projection_chip(
                FIVE_HOUR_S,
                MIN_ELAPSED_5H,
                Some(0.0),
                Some(1793077200.0),
                NOW
            ),
            "→--"
        );
        assert_eq!(
            projection_chip(FIVE_HOUR_S, MIN_ELAPSED_5H, None, Some(1793077200.0), NOW),
            ""
        );
        // overshoot: 60% used at 1h elapsed of 5h → raw 300% → clamp 100, eta = ttr*(40)/(240) = 4h*1/6 = 40m
        // (integer `now` so every intermediate value is exact in f64)
        let now = 1_793_060_000.0_f64;
        assert_eq!(
            projection_chip(
                FIVE_HOUR_S,
                MIN_ELAPSED_5H,
                Some(60.0),
                Some(now + 4.0 * 3600.0),
                now
            ),
            "→100%·40m"
        );
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
        assert_eq!(
            window_severity_rgb(&GRAPHITE, Some(1.0), "→16%", 30.0, 70.0),
            Some(GRAPHITE.s_ok)
        );
        assert_eq!(
            window_severity_rgb(&GRAPHITE, Some(1.0), "→90%", 30.0, 70.0),
            Some(GRAPHITE.s_hot)
        );
        assert_eq!(
            window_severity_rgb(&GRAPHITE, Some(35.0), "→--", 30.0, 70.0),
            Some(GRAPHITE.s_warn)
        );
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
