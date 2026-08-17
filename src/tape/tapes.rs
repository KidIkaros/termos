//! Tape file storage — recordings live in `$XDG_DATA_HOME/tuios/tapes`
//! (ported from TUIOS `internal/app/tapemanager.go`).

use std::path::PathBuf;

/// The recordings directory, creating it if needed.
pub fn tape_dir() -> Result<PathBuf, String> {
    let base = dirs::data_dir().ok_or("no data dir")?;
    let dir = base.join("tuios").join("tapes");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir)
}

/// List recorded `.tape` files, newest first.
pub fn list_tapes() -> Result<Vec<PathBuf>, String> {
    let dir = tape_dir()?;
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
        .map_err(|e| e.to_string())?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().map(|e| e == "tape").unwrap_or(false))
        .collect();
    files.sort_by_key(|p| std::fs::metadata(p).and_then(|m| m.modified()).ok());
    files.reverse();
    Ok(files)
}

/// A UTC timestamp suitable for a recording file name (`YYYYMMDD-HHMMSS`).
pub fn timestamp_stamp() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let days = secs / 86_400;
    let rem = secs % 86_400;
    let (h, m, s) = (rem / 3600, (rem % 3600) / 60, rem % 60);
    let z = days as i64 + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let mth = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if mth <= 2 { y + 1 } else { y };
    format!("{year:04}{mth:02}{d:02}-{h:02}{m:02}{s:02}")
}

/// Sanitize a recording name into a safe file stem.
pub fn sanitize_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' || c == '.' {
                c
            } else {
                '-'
            }
        })
        .collect();
    let cleaned = cleaned.trim_matches(['-', '_', '.']);
    if cleaned.is_empty() {
        "recording".to_string()
    } else {
        cleaned.to_string()
    }
}

/// Save tape content under `name` (a `.tape` extension is added).
pub fn save_tape(name: &str, content: &str) -> Result<PathBuf, String> {
    let dir = tape_dir()?;
    let path = dir.join(format!("{}.tape", sanitize_name(name)));
    std::fs::write(&path, content).map_err(|e| e.to_string())?;
    Ok(path)
}

/// Resolve a tape name (with or without a `.tape` extension) or path.
pub fn resolve_tape_path(name: &str) -> PathBuf {
    let dir = tape_dir().unwrap_or_default();
    let stem = sanitize_name(name);
    if std::path::Path::new(name).is_file() {
        std::path::Path::new(name).to_path_buf()
    } else if stem.ends_with(".tape") {
        dir.join(stem)
    } else {
        dir.join(format!("{stem}.tape"))
    }
}

/// Delete a recorded tape by name or path.
pub fn delete_tape(name: &str) -> Result<PathBuf, String> {
    let path = resolve_tape_path(name);
    if !path.exists() {
        return Err(format!("no such tape: {name}"));
    }
    std::fs::remove_file(&path).map_err(|e| e.to_string())?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_names() {
        assert_eq!(sanitize_name("my tape"), "my-tape");
        assert_eq!(sanitize_name("a/b\\c"), "a-b-c");
        assert_eq!(sanitize_name("..."), "recording");
        assert_eq!(sanitize_name("demo"), "demo");
    }

    #[test]
    fn save_and_delete_round_trip() {
        let dir = tape_dir().unwrap();
        let path = save_tape("unit-test-recording", "Enter\n").unwrap();
        assert!(path.ends_with("unit-test-recording.tape"));
        assert!(path.exists());
        assert!(delete_tape("unit-test-recording").is_ok());
        assert!(!path.exists());
        let _ = dir;
    }

    #[test]
    fn list_only_tape_files() {
        save_tape("unit-list-1", "Enter\n").unwrap();
        let files = list_tapes().unwrap();
        assert!(
            files.iter().any(|p| p
                .file_name()
                .unwrap()
                .to_string_lossy()
                .ends_with("unit-list-1.tape")),
            "expected unit-list-1.tape in {files:?}"
        );
        delete_tape("unit-list-1").unwrap();
    }
}
