//! The JSON Claude Code pipes to the statusLine command. Every field optional; unknown ignored.

use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;

use crate::format::round_even;

/// `null` or a wrong type for a field yields that field's default instead of failing the whole
/// payload (Python reads every field with `.get()` and coerces).
fn lenient<'de, D, T>(d: D) -> Result<T, D::Error>
where
    D: serde::Deserializer<'de>,
    T: serde::de::DeserializeOwned + Default,
{
    let v = Value::deserialize(d)?;
    Ok(serde_json::from_value(v).unwrap_or_default())
}

#[derive(Deserialize, Default, Debug, Clone, PartialEq)]
#[serde(default)]
pub struct Payload {
    #[serde(default, deserialize_with = "lenient")]
    pub session_id: String,
    #[serde(default, deserialize_with = "lenient")]
    pub transcript_path: String,
    #[serde(default, deserialize_with = "lenient")]
    pub cwd: String,
    #[serde(default, deserialize_with = "lenient")]
    pub model: Model,
    #[serde(default, deserialize_with = "lenient")]
    pub workspace: Workspace,
    #[serde(default, deserialize_with = "lenient")]
    pub version: String,
    #[serde(default, deserialize_with = "lenient")]
    pub effort: Option<Effort>,
    #[serde(default, deserialize_with = "lenient")]
    pub cost: Cost,
    #[serde(default, deserialize_with = "lenient")]
    pub context_window: Option<ContextWindow>,
    #[serde(default, deserialize_with = "lenient")]
    pub prompt_cache: Option<PromptCache>,
    #[serde(default, deserialize_with = "lenient")]
    pub rate_limits: Option<RateLimits>,
    /// Stamped by the Python daemon only; honored when present (COLUMNS).
    #[serde(rename = "_cs_env", default, deserialize_with = "lenient")]
    pub cs_env: Option<HashMap<String, String>>,
}

#[derive(Deserialize, Default, Debug, Clone, PartialEq)]
#[serde(default)]
pub struct Model {
    #[serde(default, deserialize_with = "lenient")]
    pub id: String,
    #[serde(default, deserialize_with = "lenient")]
    pub display_name: String,
}

/// Absent when the model does not support the effort parameter.
#[derive(Deserialize, Default, Debug, Clone, PartialEq)]
#[serde(default)]
pub struct Effort {
    #[serde(default, deserialize_with = "lenient")]
    pub level: Option<String>,
}

#[derive(Deserialize, Default, Debug, Clone, PartialEq)]
#[serde(default)]
pub struct Workspace {
    #[serde(default, deserialize_with = "lenient")]
    pub current_dir: Option<String>,
    #[serde(default, deserialize_with = "lenient")]
    pub project_dir: Option<String>,
    #[serde(default, deserialize_with = "lenient")]
    pub repo: Option<Repo>,
    #[serde(default, deserialize_with = "lenient")]
    pub git_worktree: Option<String>,
}

#[derive(Deserialize, Default, Debug, Clone, PartialEq)]
#[serde(default)]
pub struct Repo {
    #[serde(default, deserialize_with = "lenient")]
    pub name: Option<String>,
}

#[derive(Deserialize, Default, Debug, Clone, PartialEq)]
#[serde(default)]
pub struct Cost {
    #[serde(default, deserialize_with = "lenient")]
    pub total_cost_usd: f64,
    #[serde(default, deserialize_with = "lenient")]
    pub total_duration_ms: f64,
    #[serde(default, deserialize_with = "lenient")]
    pub total_lines_added: f64,
    #[serde(default, deserialize_with = "lenient")]
    pub total_lines_removed: f64,
}

fn zero() -> Value {
    Value::from(0)
}

#[derive(Deserialize, Debug, Clone, PartialEq)]
#[serde(default)]
pub struct ContextWindow {
    #[serde(default, deserialize_with = "lenient")]
    pub context_window_size: f64,
    /// Raw so that "key absent" (→ 0, Python default) and "null" (→ unknown) stay distinct.
    #[serde(default = "zero", deserialize_with = "lenient")]
    pub used_percentage: Value,
}

impl Default for ContextWindow {
    fn default() -> Self {
        ContextWindow {
            context_window_size: 0.0,
            used_percentage: zero(),
        }
    }
}

impl ContextWindow {
    pub fn ctx_pct(&self) -> Option<f64> {
        self.used_percentage.as_f64()
    }
}

#[derive(Deserialize, Default, Debug, Clone, PartialEq)]
#[serde(default)]
pub struct PromptCache {
    #[serde(default, deserialize_with = "lenient")]
    pub expires_at: Option<f64>,
    /// Cache reads over all input tokens, 0–1; `null` until the session has input tokens.
    #[serde(default, deserialize_with = "lenient")]
    pub hit_ratio: Option<f64>,
    #[serde(default, deserialize_with = "lenient")]
    pub misses: Option<f64>,
    #[serde(default, deserialize_with = "lenient")]
    pub requests: Option<f64>,
    #[serde(default, deserialize_with = "lenient")]
    pub last_miss_cause: Option<MissCause>,
}

/// Why Claude Code thinks the last miss happened; `null` until the first miss, and again when
/// it could not tell.
#[derive(Deserialize, Default, Debug, Clone, PartialEq)]
#[serde(default)]
pub struct MissCause {
    /// Such as `tools_changed`, `system_prompt_changed`, `ttl_expired_5m`, `likely_server_side`.
    #[serde(default, deserialize_with = "lenient")]
    pub causes: Vec<String>,
}

#[derive(Deserialize, Default, Debug, Clone, PartialEq)]
#[serde(default)]
pub struct RateLimits {
    #[serde(default, deserialize_with = "lenient")]
    pub five_hour: Option<Window>,
    #[serde(default, deserialize_with = "lenient")]
    pub seven_day: Option<Window>,
}

/// Any present value, `null` included, becomes `Some` (a plain `Option` would turn `null` into
/// `None`): Python treats `{"used_percentage": None}` as a present window with 0 %.
fn present<'de, D>(d: D) -> Result<Option<Value>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Value::deserialize(d).map(Some)
}

#[derive(Deserialize, Default, Debug, Clone, PartialEq)]
#[serde(default)]
pub struct Window {
    #[serde(default, deserialize_with = "present")]
    pub used_percentage: Option<Value>,
    #[serde(default, deserialize_with = "lenient")]
    pub resets_at: Option<f64>,
}

impl Window {
    /// core.parse_stdin_data `_pct`: non-number → 0, NaN/inf → 0, ≥ 100000 (leaked epoch) → 0,
    /// else `max(0, int(round(f)))`.
    pub fn pct(&self) -> f64 {
        let f = self
            .used_percentage
            .as_ref()
            .and_then(Value::as_f64)
            .unwrap_or(0.0);
        if !f.is_finite() || f >= 100000.0 {
            return 0.0;
        }
        round_even(f).max(0) as f64
    }

    /// Python `if fh:` — an empty `{}` window is falsy, i.e. absent.
    pub fn is_present(&self) -> bool {
        self.used_percentage.is_some() || self.resets_at.is_some()
    }

    /// Python `if resets_at:` — 0 / missing both mean "unknown".
    pub fn reset_time(&self) -> Option<f64> {
        self.resets_at.filter(|r| *r != 0.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_fixture() {
        let raw = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/stdin-orbit.json"
        ))
        .unwrap();
        let p: Payload = serde_json::from_str(&raw).unwrap();
        assert_eq!(p.model.display_name, "Fable 5.1");
        assert_eq!(p.effort.as_ref().unwrap().level.as_deref(), Some("xhigh"));
        assert_eq!(
            p.workspace.repo.as_ref().unwrap().name.as_deref(),
            Some("orbit")
        );
        assert_eq!(p.cost.total_lines_added, 965.0);
        let rl = p.rate_limits.as_ref().unwrap();
        assert_eq!(rl.five_hour.as_ref().unwrap().pct(), 1.0);
        assert_eq!(rl.five_hour.as_ref().unwrap().resets_at, Some(1793077200.0));
        assert_eq!(rl.seven_day.as_ref().unwrap().pct(), 16.0);
        assert_eq!(p.context_window.as_ref().unwrap().ctx_pct(), Some(8.0));
        let pc = p.prompt_cache.as_ref().unwrap();
        assert_eq!(pc.expires_at, Some(1793063288.0));
        assert_eq!(
            (pc.hit_ratio, pc.misses, pc.requests),
            (Some(0.98), Some(0.0), Some(4.0))
        );
    }

    #[test]
    fn tolerates_missing_and_null() {
        let p: Payload = serde_json::from_str(r#"{"model":{"id":"x"},"context_window":{"context_window_size":200000,"used_percentage":null}}"#).unwrap();
        assert_eq!(p.model.display_name, "");
        assert!(p.rate_limits.is_none());
        assert!(p.effort.is_none());
        assert_eq!(p.context_window.as_ref().unwrap().ctx_pct(), None);
        // key absent → Python default 0
        let p: Payload =
            serde_json::from_str(r#"{"context_window":{"context_window_size":200000}}"#).unwrap();
        assert_eq!(p.context_window.as_ref().unwrap().ctx_pct(), Some(0.0));
        let p: Payload = serde_json::from_str("{}").unwrap();
        assert!(p.context_window.is_none());
    }

    #[test]
    fn parses_last_miss_cause() {
        let pc: PromptCache = serde_json::from_str(
            r#"{"misses":2,"last_miss_cause":{"causes":["tools_changed"],"tools_added":1}}"#,
        )
        .unwrap();
        assert_eq!(pc.last_miss_cause.unwrap().causes, ["tools_changed"]);
        let pc: PromptCache = serde_json::from_str(r#"{"last_miss_cause":null}"#).unwrap();
        assert!(pc.last_miss_cause.is_none());
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

    #[test]
    fn empty_window_object_is_absent() {
        let w: Window = serde_json::from_str("{}").unwrap();
        assert!(!w.is_present());
        let w: Window = serde_json::from_str(r#"{"used_percentage":null}"#).unwrap();
        assert!(w.is_present());
        assert_eq!(w.pct(), 0.0);
        let w: Window = serde_json::from_str(r#"{"resets_at":1793077200}"#).unwrap();
        assert!(w.is_present());
    }
}
