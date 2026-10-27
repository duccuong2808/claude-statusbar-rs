//! Palettes + SGR helpers. `graphite` is the default and the only theme Python 3.50+ ships;
//! the others are csr's own, built from each scheme's published colours. A theme is colours
//! only: the layout is the same for all of them.

pub type Rgb = (u8, u8, u8);

pub const RESET: &str = "\x1b[0m";
pub const BOLD: &str = "\x1b[1m";
pub const ITAL: &str = "\x1b[3m";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Theme {
    /// Primary text: percentages, cost.
    pub ink: Rgb,
    /// Secondary text: labels, countdowns, the branch glyph.
    pub mute: Rgb,
    /// Faintest: separators and the empty cells of a bar.
    pub edge: Rgb,
    pub s_ok: Rgb,
    pub s_warn: Rgb,
    pub s_hot: Rgb,
    /// Line 2's text is a blend of this and `mute` (the name is Python's).
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

/// A built-in palette and the name the config's `theme` key selects it by.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NamedTheme {
    pub name: &'static str,
    pub description: &'static str,
    pub theme: Theme,
}

pub const DEFAULT_THEME: &str = "graphite";

/// Every built-in theme, the default first.
pub const THEMES: &[NamedTheme] = &[
    NamedTheme {
        name: DEFAULT_THEME,
        description: "cool greys, the claude-statusbar default (dark terminals)",
        theme: GRAPHITE,
    },
    NamedTheme {
        name: "nord",
        description: "Nord: snow text, frost greys, aurora accents (dark)",
        theme: Theme {
            ink: (216, 222, 233),      // nord4
            mute: (123, 136, 161),     // between nord3 and nord4
            edge: (76, 86, 106),       // nord3
            s_ok: (163, 190, 140),     // nord14
            s_warn: (235, 203, 139),   // nord13
            s_hot: (191, 97, 106),     // nord11
            pill_ink: (236, 239, 244), // nord6
            wt: (180, 142, 173),       // nord15
        },
    },
    NamedTheme {
        name: "dracula",
        description: "Dracula: bright neon accents (dark)",
        theme: Theme {
            ink: (248, 248, 242),      // foreground
            mute: (98, 114, 164),      // comment
            edge: (68, 71, 90),        // current line
            s_ok: (80, 250, 123),      // green
            s_warn: (255, 184, 108),   // orange
            s_hot: (255, 85, 85),      // red
            pill_ink: (248, 248, 242), // foreground
            wt: (189, 147, 249),       // purple
        },
    },
    NamedTheme {
        name: "gruvbox",
        description: "Gruvbox dark: warm, retro (dark)",
        theme: Theme {
            ink: (235, 219, 178),      // fg1
            mute: (146, 131, 116),     // gray
            edge: (80, 73, 69),        // bg2
            s_ok: (184, 187, 38),      // bright green
            s_warn: (250, 189, 47),    // bright yellow
            s_hot: (251, 73, 52),      // bright red
            pill_ink: (251, 241, 199), // fg0
            wt: (211, 134, 155),       // bright purple
        },
    },
    NamedTheme {
        name: "catppuccin",
        description: "Catppuccin Mocha: soft pastels (dark)",
        theme: Theme {
            ink: (205, 214, 244),      // text
            mute: (127, 132, 156),     // overlay1
            edge: (69, 71, 90),        // surface1
            s_ok: (166, 227, 161),     // green
            s_warn: (249, 226, 175),   // yellow
            s_hot: (243, 139, 168),    // red
            pill_ink: (245, 224, 220), // rosewater
            wt: (203, 166, 247),       // mauve
        },
    },
    NamedTheme {
        name: "solarized-light",
        description: "Solarized light: for light terminal backgrounds",
        theme: Theme {
            ink: (7, 54, 66),      // base02
            mute: (101, 123, 131), // base00
            edge: (147, 161, 161), // base1
            s_ok: (133, 153, 0),   // green
            s_warn: (181, 137, 0), // yellow
            s_hot: (220, 50, 47),  // red
            pill_ink: (0, 43, 54), // base03
            wt: (108, 113, 196),   // violet
        },
    },
];

/// The built-in theme called `name` (surrounding space and ASCII case ignored), or None.
pub fn find(name: &str) -> Option<&'static NamedTheme> {
    THEMES
        .iter()
        .find(|t| t.name.eq_ignore_ascii_case(name.trim()))
}

/// The palette of the built-in theme called `name`, or None.
pub fn by_name(name: &str) -> Option<Theme> {
    find(name).map(|t| t.theme)
}

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
    fn themes_are_found_by_name_ignoring_case_and_the_default_is_graphite() {
        assert_eq!(by_name(DEFAULT_THEME), Some(GRAPHITE));
        assert_eq!(THEMES[0].name, DEFAULT_THEME);
        assert_eq!(by_name("Nord"), by_name("nord"));
        assert!(by_name("nord").is_some());
        assert_eq!(by_name("no-such-theme"), None);
        assert_eq!(by_name(""), None);
    }

    #[test]
    fn every_theme_has_a_unique_name_and_palette() {
        for (i, a) in THEMES.iter().enumerate() {
            assert!(!a.name.is_empty() && !a.description.is_empty());
            assert_eq!(a.name, a.name.to_ascii_lowercase(), "names are lowercase");
            for b in &THEMES[i + 1..] {
                assert_ne!(a.name, b.name);
                assert_ne!(a.theme, b.theme, "{} and {} look the same", a.name, b.name);
            }
            // the three severities must be told apart
            let t = a.theme;
            assert!(
                t.s_ok != t.s_warn && t.s_warn != t.s_hot && t.s_ok != t.s_hot,
                "{}",
                a.name
            );
        }
    }

    #[test]
    fn fg_is_truecolor_sgr() {
        assert_eq!(fg((120, 125, 132)), "\x1b[38;2;120;125;132m");
    }
}
