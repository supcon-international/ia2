//! Read / write the RETAIN variable state file.
//!
//! The file is a single JSON object the runtime writes atomically
//! (tmp + rename) every few seconds and on graceful shutdown:
//!
//! ```json
//! { "schema": 1,
//!   "saved_at_us": 1700000000000000,
//!   "scan_count": 12345,
//!   "vars": { "setpoint": 42, "counter": 17 } }
//! ```
//!
//! Each value is the i32 the VM's `write_variable` API accepts. That
//! covers BOOL / SINT / INT / DINT / REAL (bit-pattern) and the
//! corresponding U-types — i.e. every type ironplc currently
//! round-trips through its write path. LREAL / LINT / LWORD are
//! silently truncated to 32 bits; a follow-up upstream change to
//! ironplc could broaden the write API to u64. We keep the schema
//! versioned so a wider type later doesn't force a state-file reset.

use std::collections::HashMap;
use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct RetainState {
    /// Schema version — bump when the value encoding changes
    /// incompatibly. Loading a higher schema than this binary
    /// supports falls back to "no restored state".
    pub schema: u32,
    /// Microseconds since the program started when this file was
    /// last written. Useful for "stale state" diagnostics; not used
    /// for correctness.
    #[serde(default)]
    pub saved_at_us: u64,
    /// Scan count at flush time. Same role as `saved_at_us` —
    /// surfaces "when did this snapshot come from".
    #[serde(default)]
    pub scan_count: u64,
    /// Variable name → last-known VM value, as the i32 the VM's
    /// `write_variable` API accepts. Names match `VarDebugInfo.name`
    /// (lower-cased; see `extract_retain_vars` in lib.rs).
    pub vars: HashMap<String, i32>,
}

/// Current schema version. Increment when the on-disk shape changes
/// in a way older versions can't read.
const SCHEMA: u32 = 1;

/// Read `path` and return parsed state, or `Ok(None)` if the file
/// doesn't exist (first run / fresh deploy). Returns `Err` on I/O or
/// parse errors so the caller can log loudly without aborting startup.
pub fn load(path: &Path) -> std::io::Result<Option<RetainState>> {
    let bytes = match fs::read(path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e),
    };
    let state: RetainState = serde_json::from_slice(&bytes)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    if state.schema > SCHEMA {
        tracing::warn!(
            ?path,
            file_schema = state.schema,
            binary_schema = SCHEMA,
            "retain state file has higher schema than this binary supports; ignoring"
        );
        return Ok(None);
    }
    Ok(Some(state))
}

/// Write atomically: serialise → write to `<path>.tmp` → fsync →
/// rename over `path`. A crash between rename steps leaves either
/// the previous good file or the new good file — never a partial
/// write. The parent directory is created if missing so callers don't
/// have to pre-mkdir `state/`.
pub fn save(path: &Path, state: &RetainState) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)?;
        }
    }
    let mut tmp = path.to_path_buf();
    let tmp_name = match path.file_name().and_then(|s| s.to_str()) {
        Some(name) => format!("{name}.tmp"),
        None => "retain.json.tmp".to_string(),
    };
    tmp.set_file_name(tmp_name);

    let bytes = serde_json::to_vec_pretty(state)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    {
        // Scope the file so it's closed before we rename — on some
        // filesystems renaming over an open handle is fine, on others
        // (Windows) it's a hard error. Closing first is safe everywhere.
        use std::io::Write;
        let mut f = fs::File::create(&tmp)?;
        f.write_all(&bytes)?;
        f.sync_all()?;
    }
    fs::rename(&tmp, path)?;
    Ok(())
}

/// Builder helper so the runtime doesn't have to know the schema
/// version or field layout.
pub fn build(vars: HashMap<String, i32>, saved_at_us: u64, scan_count: u64) -> RetainState {
    RetainState {
        schema: SCHEMA,
        saved_at_us,
        scan_count,
        vars,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_returns_none_for_missing_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nope.json");
        assert!(load(&path).unwrap().is_none());
    }

    #[test]
    fn save_then_load_roundtrip_preserves_vars() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state").join("retain.json");
        let mut vars = HashMap::new();
        vars.insert("setpoint".into(), 42);
        vars.insert("counter".into(), -17);
        save(&path, &build(vars.clone(), 12345, 100)).unwrap();
        let loaded = load(&path).unwrap().unwrap();
        assert_eq!(loaded.vars, vars);
        assert_eq!(loaded.scan_count, 100);
        assert_eq!(loaded.schema, SCHEMA);
    }

    #[test]
    fn load_rejects_corrupt_file_loudly() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("retain.json");
        std::fs::write(&path, b"not json at all").unwrap();
        let err = load(&path).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
    }

    #[test]
    fn load_ignores_future_schema_versions() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("retain.json");
        let future = serde_json::json!({
            "schema": SCHEMA + 5,
            "vars": { "x": 1 }
        });
        std::fs::write(&path, future.to_string()).unwrap();
        assert!(load(&path).unwrap().is_none());
    }
}
