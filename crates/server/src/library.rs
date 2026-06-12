//! Importable FB-library registry.
//!
//! A registry is a directory (`--library-dir`, dev default `./library`)
//! with one subdirectory per library:
//!
//! ```text
//! library/
//! └── process-control/
//!     ├── library.toml      # name / version / description
//!     ├── README.md
//!     └── pous/
//!         ├── fb_pid.st     # one FUNCTION_BLOCK per file
//!         └── ...
//! ```
//!
//! Importing copies block files into the project's `pous/lib/<name>/`
//! (vendored — an edge deploy stays a self-contained project) and
//! records `name = version` in the project.toml `[libraries]` table.
//! Files whose declarations include a PROGRAM (demo mains) are listed
//! nowhere and never imported: the registry deals in blocks.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// `library.toml` at a registry library's root. Optional — a bare
/// directory of `pous/*.st` is still importable under its dir name.
#[derive(Debug, Default, Deserialize)]
struct LibraryManifest {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    version: Option<String>,
    #[serde(default)]
    description: Option<String>,
}

/// One importable block file, as served to the IDE / agents.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct LibraryBlock {
    /// IEC declaration name(s) inside the file, e.g. `FB_PID`. Joined
    /// with `, ` in the (rare) multi-declaration case.
    pub name: String,
    /// Bare file name (`fb_pid.st`) — the unit of import.
    pub file: String,
    /// First comment line of the source, as a one-line summary. Empty
    /// when the file doesn't open with a comment.
    pub summary: String,
}

/// Wire shape of `GET /api/library`: one registry library plus its
/// imported state in the addressed project.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct LibrarySummary {
    pub name: String,
    pub version: String,
    pub description: String,
    pub blocks: Vec<LibraryBlock>,
    /// Version recorded in the project's `[libraries]` table, when the
    /// library has been imported into the addressed project.
    pub imported_version: Option<String>,
    /// Slug stems present under the project's `pous/lib/<name>/`
    /// (e.g. `fb_pid` — compare against `blocks[].file` minus `.st`).
    pub imported_files: Vec<String>,
}

/// A registry library with enough on-disk detail to serve both the
/// listing (metadata) and the import (file paths to copy).
pub struct RegistryLibrary {
    pub name: String,
    pub version: String,
    pub description: String,
    pub blocks: Vec<RegistryBlockFile>,
}

pub struct RegistryBlockFile {
    pub file: String,
    pub path: PathBuf,
    pub decl_names: Vec<String>,
    pub summary: String,
}

impl RegistryLibrary {
    pub fn to_summary(&self) -> LibrarySummary {
        LibrarySummary {
            name: self.name.clone(),
            version: self.version.clone(),
            description: self.description.clone(),
            blocks: self
                .blocks
                .iter()
                .map(|b| LibraryBlock {
                    name: b.decl_names.join(", "),
                    file: b.file.clone(),
                    summary: b.summary.clone(),
                })
                .collect(),
            imported_version: None,
            imported_files: Vec::new(),
        }
    }
}

/// Scan the registry root. Tolerant by design: a missing root, an
/// unreadable subdirectory or a block that fails to parse skips that
/// entry rather than failing the listing — /api/library is a browse
/// surface, not a validator.
pub fn scan(root: Option<&Path>) -> Vec<RegistryLibrary> {
    let Some(root) = root else {
        return Vec::new();
    };
    let Ok(entries) = fs::read_dir(root) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for entry in entries.flatten() {
        let dir = entry.path();
        if !dir.is_dir() {
            continue;
        }
        let Some(dir_name) = dir.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if dir_name.starts_with('.') {
            continue;
        }
        let pous = dir.join("pous");
        if !pous.is_dir() {
            continue;
        }
        let manifest: LibraryManifest = fs::read_to_string(dir.join("library.toml"))
            .ok()
            .and_then(|text| toml::from_str(&text).ok())
            .unwrap_or_default();
        let blocks = scan_blocks(&pous);
        if blocks.is_empty() {
            continue;
        }
        out.push(RegistryLibrary {
            name: manifest.name.unwrap_or_else(|| dir_name.to_string()),
            version: manifest.version.unwrap_or_else(|| "0.0.0".into()),
            description: manifest.description.unwrap_or_default(),
            blocks,
        });
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

fn scan_blocks(pous: &Path) -> Vec<RegistryBlockFile> {
    let Ok(entries) = fs::read_dir(pous) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(file) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if !file.ends_with(".st") || file.starts_with('.') || !path.is_file() {
            continue;
        }
        let Ok(source) = fs::read_to_string(&path) else {
            continue;
        };
        let decls = ironplc_bridge::extract_pou_declarations(&source, project::PouLanguage::St);
        // Blocks only: a file declaring a PROGRAM (a demo main, say) is
        // not importable and not listed.
        if decls.is_empty()
            || decls
                .iter()
                .any(|d| matches!(d.type_, project::PouType::Program))
        {
            continue;
        }
        out.push(RegistryBlockFile {
            file: file.to_string(),
            path: path.clone(),
            decl_names: decls.into_iter().map(|d| d.name).collect(),
            summary: first_comment_line(&source),
        });
    }
    out.sort_by(|a, b| a.file.cmp(&b.file));
    out
}

/// The first content line of a leading `(* ... *)` comment becomes the
/// block summary. The library convention puts `(*` on its own line and
/// the `FB_X — what it is` line right after, so scan a handful of
/// lines for the first one with real text. No leading comment → empty.
fn first_comment_line(source: &str) -> String {
    let mut lines = source.lines().map(str::trim).filter(|l| !l.is_empty());
    let Some(first) = lines.next() else {
        return String::new();
    };
    let Some(rest) = first.strip_prefix("(*") else {
        return String::new();
    };
    let clean = |s: &str| s.trim_end_matches("*)").trim().to_string();
    if !clean(rest).is_empty() {
        return clean(rest);
    }
    for line in lines.take(5) {
        if line.starts_with("*)") {
            break;
        }
        if !clean(line).is_empty() {
            return clean(line);
        }
    }
    String::new()
}
