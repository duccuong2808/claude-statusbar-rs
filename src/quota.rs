//! Account-wide freshest 5h/7d reading shared by every open Claude Code window
//! (simplified port of predict.reconcile_account / rate_latest.json).

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

use crate::atomic_file;
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

pub fn store_path(cache_dir: &Path) -> PathBuf {
    cache_dir.join("rate_latest.json")
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
pub fn merge(
    stored: Option<Reading>,
    incoming: Option<Reading>,
    now: f64,
    window_len: f64,
) -> Option<Reading> {
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
    fs::read_to_string(path)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

fn save(path: &Path, store: &Store) {
    let Some(dir) = path.parent() else { return };
    if fs::create_dir_all(dir).is_err() {
        return;
    }
    if let Ok(bytes) = serde_json::to_vec(store) {
        let _ = atomic_file::write(path, &bytes);
    }
}

/// Merge this render's stdin readings into the shared file and return what to display.
/// Writes only when the merged content differs from what was on disk (idle windows re-sending
/// the same numbers never touch the file). Never fails: an unreadable/unwritable home just
/// means "no shared store".
pub fn reconcile(
    cache_dir: &Path,
    now: f64,
    five: Option<Reading>,
    seven: Option<Reading>,
) -> (Option<Reading>, Option<Reading>) {
    let path = store_path(cache_dir);
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::format::{FIVE_HOUR_S, SEVEN_DAY_S};

    fn r(used: f64, reset: f64, at: f64) -> Option<Reading> {
        Some(Reading {
            used_pct: used,
            resets_at: reset,
            observed_at: at,
        })
    }

    #[test]
    fn merge_rules() {
        let now = 1000.0;
        let len = FIVE_HOUR_S;
        // nothing stored → incoming
        assert_eq!(
            merge(None, r(5.0, 5000.0, now), now, len),
            r(5.0, 5000.0, now)
        );
        // same reset: higher percentage wins, whichever side it is on (usage only grows in a window)
        assert_eq!(
            merge(r(9.0, 5000.0, 900.0), r(5.0, 5000.0, now), now, len),
            r(9.0, 5000.0, 900.0)
        );
        assert_eq!(
            merge(r(5.0, 5000.0, 900.0), r(9.0, 5000.0, now), now, len),
            r(9.0, 5000.0, now)
        );
        // later reset replaces, earlier reset is ignored
        assert_eq!(
            merge(r(90.0, 5000.0, 900.0), r(1.0, 23000.0, now), now, len),
            r(1.0, 23000.0, now)
        );
        assert_eq!(
            merge(r(1.0, 23000.0, 900.0), r(90.0, 5000.0, now), now, len),
            r(1.0, 23000.0, 900.0)
        );
        // a stored reading whose window already rolled over (> 60 s past reset) is dropped
        assert_eq!(merge(r(50.0, 900.0, 800.0), None, now, len), None);
        assert_eq!(
            merge(r(50.0, 950.0, 800.0), None, now, len),
            r(50.0, 950.0, 800.0)
        ); // inside the 60 s grace
           // an implausible incoming reset (further than window + 1 day) is ignored
        assert_eq!(
            merge(None, r(5.0, now + len + 86400.0 + 1.0, now), now, len),
            None
        );
        assert_eq!(
            merge(None, r(5.0, now + len + 86400.0 - 1.0, now), now, len),
            r(5.0, now + len + 86400.0 - 1.0, now)
        );
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
        let (m5, m7) = reconcile(
            home.path(),
            now + 5.0,
            r(0.0, 1793077200.0, now + 5.0),
            r(5.0, 1793304000.0, now + 5.0),
        );
        assert_eq!(m5.unwrap().used_pct, 1.0);
        assert_eq!(m7.unwrap().used_pct, 16.0);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), first);
        // a session that just started (no rate_limits in stdin) is backfilled from the store
        let (m5, m7) = reconcile(home.path(), now + 6.0, None, None);
        assert_eq!(m5.map(|x| x.used_pct), Some(1.0));
        assert_eq!(m7.map(|x| x.used_pct), Some(16.0));
        // a fresh higher reading updates the file
        let (_, m7only) = reconcile(
            home.path(),
            now + 7.0,
            None,
            r(20.0, 1793304000.0, now + 7.0),
        );
        assert_eq!(m7only.unwrap().used_pct, 20.0);
        assert_ne!(std::fs::read_to_string(&path).unwrap(), first);
        // unreadable home: behaves as "no store", returns the incoming reading
        let (m5, _) = reconcile(
            std::path::Path::new("/nonexistent/csr-home"),
            now,
            five.clone(),
            None,
        );
        assert_eq!(m5, five);
    }
}
