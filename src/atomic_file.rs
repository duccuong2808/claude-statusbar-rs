//! Crash-safe replacement of the small JSON cache files: write a per-process temp file, then
//! rename it over the target, so readers only ever see a whole old or a whole new file.

use std::fs;
use std::io;
use std::path::Path;

/// Replace `path` with `bytes` atomically. The parent directory must already exist.
///
/// # Errors
///
/// Returns the write or rename error; the temp file is removed either way.
pub fn write(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let tmp = path.with_extension(format!("tmp{}", std::process::id()));
    let result = fs::write(&tmp, bytes).and_then(|()| fs::rename(&tmp, path));
    if result.is_err() {
        let _ = fs::remove_file(&tmp); // a failed write can leave a partial temp file too
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replaces_the_target() {
        let d = tempfile::tempdir().unwrap();
        let path = d.path().join("entry.json");
        fs::write(&path, b"old").unwrap();
        write(&path, b"new").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"new");
    }

    #[test]
    fn failed_write_leaves_no_temp_file() {
        let d = tempfile::tempdir().unwrap();
        let path = d.path().join("entry.json");
        fs::create_dir(&path).unwrap(); // rename over a directory fails
        assert!(write(&path, b"{}").is_err());
        let names: Vec<_> = fs::read_dir(d.path())
            .unwrap()
            .flatten()
            .map(|e| e.file_name())
            .collect();
        assert_eq!(names, vec![std::ffi::OsString::from("entry.json")]);
    }
}
