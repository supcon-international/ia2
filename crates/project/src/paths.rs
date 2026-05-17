use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Brand directory name used inside `~/Documents/` and the per-user
/// config dir. Single source of truth so changing it here updates
/// both the projects root and the state file location.
const BRAND_DIR: &str = "IA2";

/// Legacy directory name from the project's pre-IA2 working title.
/// Migrated on first run via [`migrate_legacy_dirs`]; kept here so
/// existing dev installs don't lose their projects or last-opened
/// state.
const LEGACY_BRAND_DIR: &str = "controlsoftware";

/// Default directory where new projects are created.
///
/// Resolves to `~/Documents/IA2/` on macOS,
/// `$XDG_DOCUMENTS_DIR/IA2/` on Linux,
/// `~/Documents/IA2/` on Windows. Falls back to `./projects/`
/// only if the platform's documents dir can't be located.
pub fn default_projects_dir() -> PathBuf {
    dirs::document_dir()
        .map(|d| d.join(BRAND_DIR))
        .unwrap_or_else(|| PathBuf::from("./projects"))
}

/// Path to the persisted "last opened project" state file. Lives under the
/// platform's per-user config dir.
pub fn default_state_path() -> PathBuf {
    let base = dirs::config_dir().unwrap_or_else(|| PathBuf::from("."));
    base.join(BRAND_DIR).join("state.toml")
}

/// One-time migration: if the new IA2 directories don't exist but
/// the legacy `controlsoftware/` ones do, rename them in place. Idem-
/// potent and silent — log warnings on errors but never fail startup.
///
/// We do this rather than supporting both paths because:
///   - Two paths means two truths; users will get confused which one
///     is "real" when they move files manually.
///   - Renaming is atomic, fast, and preserves all timestamps.
///   - When the legacy directory doesn't exist, this is a no-op.
pub fn migrate_legacy_dirs() {
    if let Some(docs) = dirs::document_dir() {
        let new_dir = docs.join(BRAND_DIR);
        let old_dir = docs.join(LEGACY_BRAND_DIR);
        if !new_dir.exists() && old_dir.exists() {
            match fs::rename(&old_dir, &new_dir) {
                Ok(()) => tracing::info!(
                    from = %old_dir.display(),
                    to = %new_dir.display(),
                    "migrated legacy projects directory"
                ),
                Err(e) => tracing::warn!(
                    from = %old_dir.display(),
                    %e,
                    "failed to migrate legacy projects directory"
                ),
            }
        }
    }
    if let Some(cfg) = dirs::config_dir() {
        let new_dir = cfg.join(BRAND_DIR);
        let old_dir = cfg.join(LEGACY_BRAND_DIR);
        if !new_dir.exists() && old_dir.exists() {
            if let Err(e) = fs::rename(&old_dir, &new_dir) {
                tracing::warn!(
                    from = %old_dir.display(),
                    %e,
                    "failed to migrate legacy config dir"
                );
            }
        }
    }
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
