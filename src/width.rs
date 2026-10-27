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
            if j < b.len() && (0x40..=0x7e).contains(&b[j]) {
                Some(j + 1)
            } else {
                None
            }
        } else {
            while j < b.len() && (0x20..=0x2f).contains(&b[j]) {
                j += 1;
            }
            if j < b.len() && (0x40..=0x7e).contains(&b[j]) {
                Some(j + 1)
            } else {
                None
            }
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
    if !params
        .bytes()
        .all(|b| b.is_ascii_digit() || b == b';' || b == b':')
    {
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
    let body = rest
        .strip_suffix('\x07')
        .or_else(|| rest.strip_suffix("\x1b\\"))
        .unwrap_or(rest);
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
    text.split('\n')
        .map(|l| clip_line(l, max_width))
        .collect::<Vec<_>>()
        .join("\n")
}

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
        assert_eq!(
            clip_line("\x1b[1mabcdef\x1b[0m", Some(4)),
            "\x1b[1mabc…\x1b[0m"
        );
        // OSC 8 link left open at the cut → link closed, then RESET not needed
        assert_eq!(
            clip_line("\x1b]8;;https://x\x07abcdef\x1b]8;;\x07", Some(4)),
            "\x1b]8;;https://x\x07abc…\x1b]8;;\x07"
        );
        // wide char that would straddle the limit is dropped
        assert_eq!(clip_line("ab🌳cd", Some(3)), "ab…");
        assert_eq!(clip_lines("abcdef\nabcdef", Some(4)), "abc…\nabc…");
        assert_eq!(clip_lines("abcdef\nabcdef", None), "abcdef\nabcdef");
    }
}
