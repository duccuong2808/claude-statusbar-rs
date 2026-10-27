//! Subset of ~/.claude/claude-statusbar.json (the Python tool's config). Missing/invalid → defaults.

use serde_json::{Map, Value};
use std::path::{Path, PathBuf};

use crate::theme::{find, Rgb, Theme, DEFAULT_THEME, GRAPHITE};

/// Spacing around the `│` separators (`density` in the config file).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Density {
    Compact,
    #[default]
    Normal,
    Cozy,
}

impl Density {
    /// The padding written on each side of a separator.
    #[must_use]
    pub fn pad(self) -> &'static str {
        match self {
            Density::Compact => "",
            Density::Normal => " ",
            Density::Cozy => "  ",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Config {
    pub thin: bool,
    pub density: Density,
    pub show_weekly: bool,
    pub show_cost: bool,
    pub show_cache_age: bool,
    pub show_project_branch: bool,
    pub show_lines: bool,
    pub show_duration: bool,
    pub show_ahead_behind: bool,
    pub show_projection: bool,
    pub worktree_glyph: String,
    /// The built-in theme picked by the `theme` key (the default when unset or unknown).
    pub theme_name: &'static str,
    /// That theme's palette with the `color_*` overrides applied.
    pub theme: Theme,
    pub warning: f64,
    pub critical: f64,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            thin: false,
            density: Density::Normal,
            show_weekly: true,
            show_cost: false,
            show_cache_age: true,
            show_project_branch: true,
            show_lines: true,
            show_duration: false,
            show_ahead_behind: false,
            show_projection: true,
            worktree_glyph: "🌳".to_string(),
            theme_name: DEFAULT_THEME,
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

/// config._to_bool: a bool is itself; anything else is `str(v).strip().lower()` in a fixed set
/// (so `2`, `1.0`, `null`, `"maybe"` are false). The default applies only to a missing key.
fn get_bool(m: &Map<String, Value>, key: &str, default: bool) -> bool {
    let text = match m.get(key) {
        None => return default,
        Some(Value::Bool(b)) => return *b,
        Some(Value::String(s)) => s.trim().to_lowercase(),
        Some(Value::Number(n)) => n.to_string(),
        Some(_) => return false,
    };
    matches!(text.as_str(), "1" | "true" | "yes" | "on" | "y" | "t")
}

fn get_str<'a>(m: &'a Map<String, Value>, key: &str) -> Option<&'a str> {
    m.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

/// themes.parse_hex_color: `#rrggbb`, `#rgb`, `rrggbb`, `rgb`.
pub fn parse_hex(s: &str) -> Option<Rgb> {
    let s = s.trim().trim_start_matches('#');
    if !s.is_ascii() {
        return None;
    }
    let s: String = if s.len() == 3 {
        s.chars().flat_map(|c| [c, c]).collect()
    } else {
        s.to_string()
    };
    if s.len() != 6 {
        return None;
    }
    let ch = |i: usize| u8::from_str_radix(s.get(i..i + 2)?, 16).ok();
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
        c.density = match get_str(&m, "density") {
            Some("compact") => Density::Compact,
            Some("cozy") => Density::Cozy,
            _ => Density::Normal,
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
        // themes.get_theme: an unknown name is the default theme. The color_* keys then
        // override single colours of whichever theme was picked (Python has the last four).
        if let Some(t) = get_str(&m, "theme").and_then(find) {
            c.theme_name = t.name;
            c.theme = t.theme;
        }
        let t = &mut c.theme;
        let overrides: [(&str, &mut Rgb); 8] = [
            ("color_ink", &mut t.ink),
            ("color_mute", &mut t.mute),
            ("color_edge", &mut t.edge),
            ("color_line2", &mut t.pill_ink),
            ("color_ok", &mut t.s_ok),
            ("color_warn", &mut t.s_warn),
            ("color_hot", &mut t.s_hot),
            ("color_worktree", &mut t.wt),
        ];
        for (key, slot) in overrides {
            if let Some(rgb) = get_str(&m, key).and_then(parse_hex) {
                *slot = rgb;
            }
        }
        // progress.normalize_thresholds: a missing (or null) side takes its default, then the
        // pair must satisfy 0 <= warning < critical <= 100; anything else keeps both defaults.
        let threshold = |key: &str, default: f64| match m.get(key) {
            None | Some(Value::Null) => Some(default),
            Some(v) => v.as_f64(),
        };
        if let (Some(w), Some(cr)) = (
            threshold("warning_threshold", c.warning),
            threshold("critical_threshold", c.critical),
        ) {
            if 0.0 <= w && w < cr && cr <= 100.0 {
                c.warning = w;
                c.critical = cr;
            }
        }
        c
    }
}

/// Why `set_theme` left the config file alone.
#[derive(Debug)]
pub enum SetThemeError {
    UnknownTheme(String),
    /// The file exists but is not a JSON object; rewriting it would lose what is in it.
    NotAnObject(PathBuf),
    Io(PathBuf, std::io::Error),
}

impl std::fmt::Display for SetThemeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownTheme(name) => {
                let names: Vec<&str> = crate::theme::THEMES.iter().map(|t| t.name).collect();
                write!(f, "unknown theme {name:?} (one of: {})", names.join(", "))
            }
            Self::NotAnObject(p) => {
                write!(f, "{} is not a JSON object; left unchanged", p.display())
            }
            Self::Io(p, e) => write!(f, "{}: {e}", p.display()),
        }
    }
}

impl std::error::Error for SetThemeError {}

/// Sets `"theme"` in the config file at `path`, keeping every other key; creates the file (and
/// its directory) when missing. The file is replaced atomically, rewritten as pretty JSON.
///
/// # Errors
///
/// An unknown theme name, a file that exists but is not a JSON object, or a read/write failure.
/// The file is untouched in every error case.
pub fn set_theme(path: &Path, name: &str) -> Result<(), SetThemeError> {
    let theme = find(name).ok_or_else(|| SetThemeError::UnknownTheme(name.to_string()))?;
    let mut map = match std::fs::read_to_string(path) {
        Ok(text) if text.trim().is_empty() => Map::new(),
        Ok(text) => match serde_json::from_str::<Value>(&text) {
            Ok(Value::Object(m)) => m,
            _ => return Err(SetThemeError::NotAnObject(path.to_path_buf())),
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Map::new(),
        Err(e) => return Err(SetThemeError::Io(path.to_path_buf(), e)),
    };
    map.insert("theme".to_string(), Value::from(theme.name));
    let mut text = serde_json::to_string_pretty(&Value::Object(map))
        .expect("a JSON map of JSON values always serializes");
    text.push('\n');
    let io = |e| SetThemeError::Io(path.to_path_buf(), e);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(io)?;
    }
    crate::atomic_file::write(path, text.as_bytes()).map_err(io)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::GRAPHITE;

    #[test]
    fn defaults_when_missing() {
        let c = Config::load(std::path::Path::new("/nonexistent/claude-statusbar.json"));
        assert!(!c.thin);
        assert_eq!(c.density, Density::Normal);
        assert!(
            c.show_weekly
                && c.show_cache_age
                && c.show_project_branch
                && c.show_lines
                && c.show_projection
        );
        assert!(!c.show_cost && !c.show_duration && !c.show_ahead_behind);
        assert_eq!(c.worktree_glyph, "🌳");
        assert_eq!(c.theme, GRAPHITE);
        assert_eq!((c.warning, c.critical), (30.0, 70.0));
    }

    #[test]
    fn density_pads() {
        let pads = [Density::Compact, Density::Normal, Density::Cozy].map(Density::pad);
        assert_eq!(pads, ["", " ", "  "]);
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("cfg.json");
        std::fs::write(&p, r#"{"density":"cozy"}"#).unwrap();
        assert_eq!(Config::load(&p).density, Density::Cozy);
    }

    #[test]
    fn reads_subset() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("cfg.json");
        std::fs::write(&p, r##"{"bar_glyph":"thin","density":"compact","show_cost":true,"show_weekly":"false","worktree_glyph":"⑂","color_ok":"#4ec85b","color_hot":"fff","warning_threshold":50,"critical_threshold":90,"unknown_key":1}"##).unwrap();
        let c = Config::load(&p);
        assert!(c.thin);
        assert_eq!(c.density, Density::Compact);
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

    #[test]
    fn theme_key_picks_the_palette_and_colour_keys_override_it() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("cfg.json");
        let load = |json: &str| {
            std::fs::write(&p, json).unwrap();
            Config::load(&p)
        };
        let nord = crate::theme::by_name("nord").unwrap();
        assert_eq!(load(r#"{"theme":"nord"}"#).theme, nord);
        assert_eq!(load(r#"{"theme":"NORD"}"#).theme_name, "nord");
        assert_eq!(load(r#"{"theme":"no-such"}"#).theme_name, DEFAULT_THEME);
        assert_eq!(load(r#"{"theme":" Nord "}"#).theme, nord);
        // unknown or non-string names fall back to the default, like Python's get_theme
        assert_eq!(load(r#"{"theme":"no-such"}"#).theme, GRAPHITE);
        assert_eq!(load(r#"{"theme":7}"#).theme, GRAPHITE);

        // every colour can be overridden on top of the chosen theme, whatever the key order
        let c = load(
            r##"{"color_ink":"#010203","color_mute":"#040506","color_edge":"#070809",
                 "color_line2":"#0a0b0c","color_ok":"#111","color_warn":"#222",
                 "color_hot":"#333","color_worktree":"#444","theme":"nord"}"##,
        );
        let want = Theme {
            ink: (1, 2, 3),
            mute: (4, 5, 6),
            edge: (7, 8, 9),
            pill_ink: (10, 11, 12),
            s_ok: (0x11, 0x11, 0x11),
            s_warn: (0x22, 0x22, 0x22),
            s_hot: (0x33, 0x33, 0x33),
            wt: (0x44, 0x44, 0x44),
        };
        assert_eq!(c.theme, want);
        // an override that does not parse keeps the theme's own colour
        assert_eq!(
            load(r#"{"theme":"nord","color_ink":"nope"}"#).theme.ink,
            nord.ink
        );
    }

    #[test]
    fn non_ascii_colour_does_not_panic() {
        assert_eq!(parse_hex("€"), None);
        assert_eq!(parse_hex("a€bc"), None);
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("cfg.json");
        std::fs::write(
            &p,
            r#"{"color_ok":"€","color_hot":"a€bc","color_warn":"ab€"}"#,
        )
        .unwrap();
        let c = Config::load(&p);
        assert_eq!(c.theme, GRAPHITE);
    }

    #[test]
    fn a_single_threshold_is_completed_from_the_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("cfg.json");
        let load = |json: &str| {
            std::fs::write(&p, json).unwrap();
            let c = Config::load(&p);
            (c.warning, c.critical)
        };
        assert_eq!(load(r#"{"warning_threshold":50}"#), (50.0, 70.0));
        assert_eq!(load(r#"{"critical_threshold":20}"#), (30.0, 70.0)); // 30 < 20 fails → defaults
        assert_eq!(load(r#"{"critical_threshold":90}"#), (30.0, 90.0));
        assert_eq!(
            load(r#"{"warning_threshold":null,"critical_threshold":90}"#),
            (30.0, 90.0)
        );
    }

    #[test]
    fn booleans_follow_python_to_bool() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("cfg.json");
        std::fs::write(&p, r#"{"show_weekly":2,"show_cost":"y","show_lines":null,"show_cache_age":"maybe","show_projection":1.0,"show_duration":" T ","show_project_branch":1}"#).unwrap();
        let c = Config::load(&p);
        assert!(!c.show_weekly); // str(2) = "2"
        assert!(c.show_cost);
        assert!(!c.show_lines); // str(None) = "None"
        assert!(!c.show_cache_age);
        assert!(!c.show_projection); // str(1.0) = "1.0"
        assert!(c.show_duration);
        assert!(c.show_project_branch);
        assert!(!c.show_ahead_behind); // missing → default
    }

    #[test]
    fn set_theme_keeps_other_keys_and_refuses_what_it_cannot_parse() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("sub").join("cfg.json");
        // an empty file has nothing to lose
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, " \n").unwrap();
        set_theme(&p, "nord").unwrap();
        assert_eq!(Config::load(&p).theme_name, "nord");
        std::fs::remove_dir_all(p.parent().unwrap()).unwrap();
        // missing file (and directory): created with just the theme
        set_theme(&p, "Dracula").unwrap();
        assert_eq!(Config::load(&p).theme_name, "dracula");

        std::fs::write(&p, r#"{"show_cost":true,"theme":"nord","ui_version":3}"#).unwrap();
        set_theme(&p, "gruvbox").unwrap();
        let v: Value = serde_json::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap();
        assert_eq!(v["theme"], "gruvbox");
        assert_eq!(
            (v["show_cost"].clone(), v["ui_version"].clone()),
            (Value::from(true), Value::from(3))
        );

        let before = std::fs::read_to_string(&p).unwrap();
        assert!(matches!(
            set_theme(&p, "no-such"),
            Err(SetThemeError::UnknownTheme(_))
        ));
        assert_eq!(std::fs::read_to_string(&p).unwrap(), before);

        for junk in ["[1,2]", "{not json", "null"] {
            std::fs::write(&p, junk).unwrap();
            assert!(
                matches!(set_theme(&p, "nord"), Err(SetThemeError::NotAnObject(_))),
                "{junk:?}"
            );
            assert_eq!(std::fs::read_to_string(&p).unwrap(), junk);
        }
    }
}
