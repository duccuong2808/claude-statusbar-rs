//! Fast single-binary Claude Code status line: rendering, config, git and quota caches.

pub mod atomic_file;
pub mod cachedir;
pub mod clock;
pub mod config;
pub mod format;
pub mod gitcache;
pub mod gitkey;
pub mod identity;
pub mod line1;
pub mod line2;
pub mod payload;
pub mod quota;
pub mod render;
pub mod theme;
pub mod width;
