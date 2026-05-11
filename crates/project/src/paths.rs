use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Default directory where new projects are created.
///
/// Resolves to `~/Documents/controlsoftware/` on macOS,
/// `$XDG_DOCUMENTS_DIR/controlsoftware/` on Linux,
/// `~/Documents/controlsoftware/` on Windows. Falls back to `./projects/`
/// only if the platform's documents dir can't be located.
pub fn default_projects_dir() -> PathBuf {
    dirs::document_dir()
        .map(|d| d.join("controlsoftware"))
        .unwrap_or_else(|| PathBuf::from("./projects"))
}

/// Path to the persisted "last opened project" state file. Lives under the
/// platform's per-user config dir.
pub fn default_state_path() -> PathBuf {
    let base = dirs::config_dir().unwrap_or_else(|| PathBuf::from("."));
    base.join("controlsoftware").join("state.toml")
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct AppState {
    last_opened: Option<PathBuf>,
}

/// Read the most recently opened project path, if any. Silently treats
/// missing or corrupt state files as "no last opened" — startup should
/// not fail on a bad state file.
pub fn load_last_opened() -> Option<PathBuf> {
    let path = default_state_path();
    let text = fs::read_to_string(&path).ok()?;
    let state: AppState = toml::from_str(&text).ok()?;
    state.last_opened
}

/// Persist the most recently opened project path. Creates the parent
/// config directory as needed; logs and swallows any I/O error.
pub fn save_last_opened(project_path: &Path) {
    let state_path = default_state_path();
    if let Some(parent) = state_path.parent() {
        if let Err(e) = fs::create_dir_all(parent) {
            tracing::warn!(?parent, %e, "failed to create state dir");
            return;
        }
    }
    let state = AppState {
        last_opened: Some(project_path.to_path_buf()),
    };
    match toml::to_string(&state) {
        Ok(text) => {
            if let Err(e) = fs::write(&state_path, text) {
                tracing::warn!(?state_path, %e, "failed to write state file");
            }
        }
        Err(e) => tracing::warn!(%e, "failed to serialize app state"),
    }
}
