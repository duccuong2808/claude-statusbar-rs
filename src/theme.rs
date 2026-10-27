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
    let ch = |x: u8, y: u8| (f64::from(x) + (f64::from(y) - f64::from(x)) * t) as u8;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blend_truncates_like_python_int() {
        // second bar cell of a 12-cell bar: t = 1/11, blend(ok, warn, 2t)
        assert_eq!(
            blend(GRAPHITE.s_ok, GRAPHITE.s_warn, 2.0 / 11.0),
            (106, 196, 91)
        );
        assert_eq!(blend(GRAPHITE.s_hot, GRAPHITE.wt, 0.5), (200, 134, 171)); // model accent
        assert_eq!(
            blend(GRAPHITE.pill_ink, GRAPHITE.mute, 0.5),
            (179, 180, 178)
        ); // line-2 BASE
        assert_eq!(blend(GRAPHITE.s_warn, GRAPHITE.mute, 0.45), (181, 154, 112)); // soft(warn) = ● color
        assert_eq!(blend(GRAPHITE.s_ok, GRAPHITE.mute, 0.45), (96, 166, 109)); // soft(ok) = +lines
        assert_eq!(blend(GRAPHITE.s_hot, GRAPHITE.mute, 0.45), (181, 120, 123));
        // soft(hot) = −lines
    }

    #[test]
    fn fg_is_truecolor_sgr() {
        assert_eq!(fg((120, 125, 132)), "\x1b[38;2;120;125;132m");
    }
}
