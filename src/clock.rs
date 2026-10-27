//! Wall-clock time as Unix seconds, the unit every cache timestamp and reset time uses.

use std::time::{SystemTime, UNIX_EPOCH};

/// Seconds since the Unix epoch; 0 if the system clock reads before 1970.
#[must_use]
pub fn unix_now() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0.0, |d| d.as_secs_f64())
}
