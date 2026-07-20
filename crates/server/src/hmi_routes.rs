//! HMI screen endpoints — CRUD, incremental ops, validation, the symbol
//! catalog, and the deterministic generator. Design: docs/hmi-design.md.
//!
//! The ops endpoint is the agent's authoring surface: each batch applies
//! atomically, is persisted, and is broadcast as a `Mutation` whose detail
//! carries the touched node ids — that detail is what lets the canvas
//! animate exactly the elements an agent just placed, Pencil-style,
//! instead of re-rendering blind.

use std::collections::BTreeMap;

use axum::extract::{Path as AxumPath, State};
use axum::Json;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use project::{
    apply_hmi_ops, hmi_nav_targets, hmi_variables, hmi_write_variables, validate_hmi, HmiAction,
    HmiBinding, HmiDoc, HmiIssue, HmiNode, HmiNodeKind, HmiOp, HmiSeries, ProjectStore, StoreError,
};

use crate::error::ApiError;
use crate::events::MutationDetail;
use crate::routes::{with_project, ProjectName};
use crate::state::AppState;

// ============================================================
//  List / CRUD
// ============================================================

#[derive(Debug, Serialize, TS)]
#[ts(export)]
pub struct HmiListEntry {
    pub path: String,
    pub title: String,
    pub level: u8,
}

pub async fn list_hmis(
    State(state): State<AppState>,
    project: ProjectName,
) -> Result<Json<Vec<HmiListEntry>>, ApiError> {
    with_project(&state, &project, |store| {
        let mut out = Vec::new();
        for path in store.list_hmis()? {
            // A corrupt file still lists (title falls back to the slug) so
            // the tree shows it and the user can open + fix or delete it.
            let (title, level) = match store.read_hmi(&path) {
                Ok(doc) => (doc.title, doc.level),
                Err(_) => (path.clone(), 0),
            };
            out.push(HmiListEntry { path, title, level });
        }
        Ok(out)
    })
    .map(Json)
}

#[derive(Debug, Deserialize)]
pub struct CreateHmiRequest {
    pub path: String,
    #[serde(default)]
    pub title: Option<String>,
}

pub async fn create_hmi(
    State(state): State<AppState>,
    project: ProjectName,
    Json(req): Json<CreateHmiRequest>,
) -> Result<Json<HmiDoc>, ApiError> {
    let (doc, project_name) = with_project(&state, &project, |store| {
        let title = req.title.clone().unwrap_or_else(|| req.path.clone());
        Ok((
            store.create_hmi(&req.path, &title)?,
            store.name().to_string(),
        ))
    })?;
    state.emit_mutation(
        &project_name,
        "hmi",
        MutationDetail::HmiUpserted {
            path: req.path.clone(),
            touched: vec![],
        },
    );
    Ok(Json(doc))
}

pub async fn get_hmi(
    State(state): State<AppState>,
    project: ProjectName,
    AxumPath(path): AxumPath<String>,
) -> Result<Json<HmiDoc>, ApiError> {
    with_project(&state, &project, |store| Ok(store.read_hmi(&path)?)).map(Json)
}

/// The one persistence gate: error-severity findings block a write.
/// Every path that stores a document (PUT, /ops, generate) must agree,
/// or one path persists screens another then refuses to save.
fn structural_errors(issues: &[HmiIssue]) -> Option<String> {
    let errors: Vec<&str> = issues
        .iter()
        .filter(|i| i.severity == "error")
        .map(|i| i.message.as_str())
        .collect();
    (!errors.is_empty()).then(|| format!("screen has structural errors: {}", errors.join("; ")))
}

pub async fn put_hmi(
    State(state): State<AppState>,
    project: ProjectName,
    AxumPath(path): AxumPath<String>,
    Json(doc): Json<HmiDoc>,
) -> Result<Json<Vec<HmiIssue>>, ApiError> {
    let issues = validate_hmi(&doc);
    if let Some(msg) = structural_errors(&issues) {
        return Err(ApiError::BadRequest(msg));
    }
    let project_name = with_project(&state, &project, |store| {
        store.write_hmi(&path, &doc)?;
        Ok(store.name().to_string())
    })?;
    // Whole-document save: no per-node touched list (the canvas refreshes
    // without spawn animation — saves are edits, not generation).
    state.emit_mutation(
        &project_name,
        "hmi",
        MutationDetail::HmiUpserted {
            path,
            touched: vec![],
        },
    );
    Ok(Json(issues))
}

pub async fn delete_hmi(
    State(state): State<AppState>,
    project: ProjectName,
    AxumPath(path): AxumPath<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let project_name = with_project(&state, &project, |store| {
        store.delete_hmi(&path)?;
        Ok(store.name().to_string())
    })?;
    state.emit_mutation(&project_name, "hmi", MutationDetail::HmiDeleted { path });
    Ok(Json(serde_json::json!({ "ok": true })))
}

// ============================================================
//  Incremental ops — the agent authoring path
// ============================================================

#[derive(Debug, Deserialize)]
pub struct HmiOpsRequest {
    pub ops: Vec<HmiOp>,
}

#[derive(Debug, Serialize, TS)]
#[ts(export)]
pub struct HmiOpsResponse {
    /// Node ids created or modified by this batch — also broadcast over
    /// SSE so every open canvas animates exactly these elements.
    pub touched: Vec<String>,
    /// Structural findings on the document AFTER the batch. Warnings
    /// don't block; a batch whose result carries error-severity issues
    /// is rejected (422) without persisting — same gate as PUT.
    pub issues: Vec<HmiIssue>,
}

pub async fn hmi_ops(
    State(state): State<AppState>,
    project: ProjectName,
    AxumPath(path): AxumPath<String>,
    Json(req): Json<HmiOpsRequest>,
) -> Result<Json<HmiOpsResponse>, ApiError> {
    if req.ops.is_empty() {
        return Err(ApiError::BadRequest("ops must not be empty".into()));
    }
    let (touched, issues, project_name) = with_project(&state, &project, |store| {
        let mut doc = store.read_hmi(&path)?;
        let touched = apply_hmi_ops(&mut doc, &req.ops).map_err(ApiError::BadRequest)?;
        let issues = validate_hmi(&doc);
        if let Some(msg) = structural_errors(&issues) {
            return Err(ApiError::Unprocessable(msg));
        }
        store.write_hmi(&path, &doc)?;
        Ok((touched, issues, store.name().to_string()))
    })?;
    state.emit_mutation(
        &project_name,
        "hmi",
        MutationDetail::HmiUpserted {
            path,
            touched: touched.clone(),
        },
    );
    Ok(Json(HmiOpsResponse { touched, issues }))
}

// ============================================================
//  Check — structure + variable existence
// ============================================================

pub async fn check_hmi(
    State(state): State<AppState>,
    project: ProjectName,
    AxumPath(path): AxumPath<String>,
) -> Result<Json<Vec<HmiIssue>>, ApiError> {
    with_project(&state, &project, |store| {
        let doc = store.read_hmi(&path)?;
        check_hmi_doc(store, &doc)
    })
    .map(Json)
}

fn hmi_warn(node_id: String, message: String) -> HmiIssue {
    HmiIssue {
        severity: "warning".into(),
        node_id: Some(node_id),
        message,
    }
}

/// Structure + project context: variable existence, writes to
/// input-direction (read-only) variables, dangling nav targets. Shared
/// by /check and /api/project/validate so both surface the same findings
/// (the docs/hmi-design.md diagnostics contract).
pub(crate) fn check_hmi_doc(store: &ProjectStore, doc: &HmiDoc) -> Result<Vec<HmiIssue>, ApiError> {
    let mut issues = validate_hmi(doc);
    let known = project_variable_directions(store)?;
    // Multi-PROGRAM `instance.variable` names resolve at runtime; check
    // the bare tail so both spellings pass.
    let tail_of = |var: &str| var.rsplit('.').next().unwrap_or(var).to_string();
    for (node_id, var) in hmi_variables(doc) {
        if !known.contains_key(&tail_of(&var)) {
            issues.push(hmi_warn(
                node_id,
                format!("variable '{var}' not found in any POU"),
            ));
        }
    }
    for (node_id, var) in hmi_write_variables(doc) {
        // A bare tail can match declarations in several POUs; flag only
        // when every one is an input — those are fed by the scan, so an
        // HMI write is overwritten (read-only by contract). Warning, not
        // error: same weight as the existence check above.
        if let Some(dirs) = known.get(&tail_of(&var)) {
            if dirs.iter().all(|d| d == "input") {
                issues.push(hmi_warn(
                    node_id,
                    format!("action writes '{var}' but it is declared as an input (read-only)"),
                ));
            }
        }
    }
    let screens: std::collections::HashSet<String> = store.list_hmis()?.into_iter().collect();
    for (node_id, target) in hmi_nav_targets(doc) {
        // Empty targets are validate_hmi's finding already.
        if !target.trim().is_empty() && !screens.contains(&target) {
            issues.push(hmi_warn(
                node_id,
                format!("nav target '{target}' is not a screen in this project"),
            ));
        }
    }
    Ok(issues)
}

/// Bare variable name → every declaration direction it appears with
/// across the project's POUs (a name can repeat between programs).
fn project_variable_directions(
    store: &ProjectStore,
) -> Result<std::collections::HashMap<String, Vec<String>>, ApiError> {
    // Language-aware on purpose: graphical POUs carry their variables in
    // the JSON document, not in ST the bridge can parse — same extraction
    // the generator uses, so check and generate can never disagree.
    let mut out: std::collections::HashMap<String, Vec<String>> = std::collections::HashMap::new();
    for path in store.list_pou_paths()? {
        for v in pou_gen_vars(store, &path) {
            out.entry(v.name).or_default().push(v.direction);
        }
    }
    Ok(out)
}

// ============================================================
//  Symbol catalog — the agent's palette reference
// ============================================================

#[derive(Debug, Serialize, TS)]
#[ts(export)]
pub struct HmiSymbolInfo {
    pub name: String,
    pub description: String,
    /// Bindable live inputs (`bind` keys the renderer understands).
    pub binds: Vec<String>,
    /// Static props (`props` keys) with a one-line meaning each.
    pub props: Vec<String>,
    /// Sensible default w×h on the grid.
    pub default_size: [u32; 2],
}

fn sym(
    name: &str,
    description: &str,
    binds: &[&str],
    props: &[&str],
    default_size: [u32; 2],
) -> HmiSymbolInfo {
    HmiSymbolInfo {
        name: name.into(),
        description: description.into(),
        binds: binds.iter().map(|s| s.to_string()).collect(),
        props: props.iter().map(|s| s.to_string()).collect(),
        default_size,
    }
}

/// The palette, as data. Kept next to the handlers (not in `project`)
/// because the CONTRACT (which binds/props each symbol honours) is a
/// renderer fact; `project` only knows the legal names for validation.
pub fn symbol_catalog() -> Vec<HmiSymbolInfo> {
    vec![
        sym(
            "tank",
            "Vessel with a 0-100 fill level and optional alarm ring.",
            &["value", "alarm"],
            &["label: string", "unit: string (default %)"],
            [120, 180],
        ),
        sym(
            "valve",
            "Block valve; bowtie glyph, filled when open, warn ring on fault.",
            &["open", "fault"],
            &["label: string", "orientation: h|v (default h)"],
            [48, 48],
        ),
        sym(
            "pump",
            "Centrifugal pump circle; filled when running.",
            &["running", "fault"],
            &["label: string"],
            [56, 56],
        ),
        sym(
            "motor",
            "Motor box (M); filled when running.",
            &["running", "fault"],
            &["label: string"],
            [56, 56],
        ),
        sym(
            "pipe_h",
            "Horizontal process line (static).",
            &[],
            &[],
            [120, 8],
        ),
        sym(
            "pipe_v",
            "Vertical process line (static).",
            &[],
            &[],
            [8, 120],
        ),
        sym(
            "gauge",
            "Radial gauge for a 0-100 value (use sparingly per ISA-101).",
            &["value"],
            &["label: string", "unit: string"],
            [96, 96],
        ),
        sym(
            "indicator",
            "Status dot + label; ISA-101 calm when off/normal.",
            &["on", "alarm"],
            &["label: string"],
            [140, 24],
        ),
        sym(
            "setpoint",
            "Read-only setpoint chip (pair with an input for entry).",
            &["value"],
            &["label: string", "unit: string"],
            [140, 32],
        ),
    ]
}

pub async fn hmi_symbols() -> Json<Vec<HmiSymbolInfo>> {
    Json(symbol_catalog())
}

// ============================================================
//  Deterministic generate — the boring-but-correct baseline
// ============================================================

#[derive(Debug, Default, Deserialize)]
pub struct GenerateHmiRequest {
    /// Overwrite an existing screen at `path`. Off by default so an agent
    /// can't clobber a curated screen by accident.
    #[serde(default)]
    pub force: bool,
    #[serde(default)]
    pub title: Option<String>,
}

/// Build a first-pass screen from project truth: alarmbar on top, one
/// section per POU file, BOOLs as indicators, numerics as value readouts,
/// `*_sp`-named numerics as setpoint inputs, plus one trend over the first
/// two numerics. Deterministic on purpose — same project, same screen —
/// so it is a baseline the creative pass edits via /ops, not a competitor
/// to it.
pub async fn generate_hmi(
    State(state): State<AppState>,
    project: ProjectName,
    AxumPath(path): AxumPath<String>,
    body: Option<Json<GenerateHmiRequest>>,
) -> Result<Json<HmiDoc>, ApiError> {
    let req = body.map(|Json(b)| b).unwrap_or_default();
    let (doc, project_name) = with_project(&state, &project, |store| {
        if !req.force {
            match store.read_hmi(&path) {
                Ok(_) => {
                    return Err(ApiError::Conflict(format!(
                        "hmi '{path}' already exists — pass {{\"force\":true}} to regenerate"
                    )))
                }
                // Absent is the green light. Present-but-unparseable is
                // still a curated screen (often one typo from valid) —
                // overwriting it must stay an explicit force.
                Err(StoreError::HmiNotFound(_)) => {}
                Err(StoreError::HmiCorrupt(..)) => {
                    return Err(ApiError::Conflict(format!(
                        "hmi '{path}' exists but is not readable as JSON — fix it or pass \
                         {{\"force\":true}} to regenerate"
                    )))
                }
                Err(e) => return Err(e.into()),
            }
        }
        let title = req
            .title
            .clone()
            .unwrap_or_else(|| format!("{} — Overview", store.name()));
        let doc = build_generated(store, &title)?;
        // Same gate as PUT — the generator must never persist a document
        // the editor would then refuse to save.
        if let Some(msg) = structural_errors(&validate_hmi(&doc)) {
            return Err(ApiError::Internal(format!("generated {msg}")));
        }
        store.write_hmi(&path, &doc)?;
        Ok((doc, store.name().to_string()))
    })?;
    let touched: Vec<String> = collect_touched(&doc);
    state.emit_mutation(
        &project_name,
        "hmi",
        MutationDetail::HmiUpserted { path, touched },
    );
    Ok(Json(doc))
}

fn collect_touched(doc: &HmiDoc) -> Vec<String> {
    fn walk(n: &HmiNode, out: &mut Vec<String>) {
        out.push(n.id.clone());
        if let HmiNodeKind::Group { children, .. } = &n.kind {
            for c in children {
                walk(c, out);
            }
        }
    }
    let mut out = Vec::new();
    if let HmiNodeKind::Group { children, .. } = &doc.root.kind {
        for c in children {
            walk(c, &mut out);
        }
    }
    out
}

fn is_alarmish(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    n.ends_with("_hh")
        || n.ends_with("_ll")
        || n.ends_with("_alm")
        || n.ends_with("_trip")
        || n.contains("fault")
        || n.contains("alarm")
}

fn is_setpointish(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    n.ends_with("_sp") || n.contains("setpoint")
}

fn numericish(type_name: &str) -> bool {
    matches!(
        type_name.to_ascii_uppercase().as_str(),
        "INT"
            | "DINT"
            | "SINT"
            | "LINT"
            | "UINT"
            | "UDINT"
            | "USINT"
            | "ULINT"
            | "REAL"
            | "LREAL"
            | "WORD"
            | "DWORD"
            | "TIME"
    )
}

/// A variable fact for generation, normalised across languages.
struct GenVar {
    name: String,
    type_name: String,
    direction: String,
}

/// Variables of one POU file regardless of language: ST goes through the
/// bridge's AST extraction; graphical POUs carry their variable table in
/// the JSON document itself.
fn pou_gen_vars(store: &ProjectStore, pou_path: &str) -> Vec<GenVar> {
    let Ok(source) = store.read_pou_source(pou_path) else {
        return Vec::new();
    };
    let lang = store
        .pou_file_language(pou_path)
        .unwrap_or(project::PouLanguage::St);
    let from_ld = |vars: &[project::LdVariable]| {
        vars.iter()
            .map(|v| GenVar {
                name: v.name.clone(),
                type_name: v.type_name.clone(),
                direction: match v.section {
                    project::LdVarSection::Input => "input".into(),
                    project::LdVarSection::Output => "output".into(),
                    project::LdVarSection::Internal => "local".into(),
                },
            })
            .collect()
    };
    match lang {
        project::PouLanguage::St => ironplc_bridge::extract_variables(&source)
            .into_iter()
            .map(|v| GenVar {
                name: v.name,
                type_name: v.type_name,
                direction: v.direction,
            })
            .collect(),
        project::PouLanguage::Ld => serde_json::from_str::<project::LdProgram>(&source)
            .map(|p| from_ld(&p.variables))
            .unwrap_or_default(),
        project::PouLanguage::Fbd => serde_json::from_str::<project::FbdProgram>(&source)
            .map(|p| from_ld(&p.variables))
            .unwrap_or_default(),
        project::PouLanguage::Sfc => serde_json::from_str::<project::SfcProgram>(&source)
            .map(|p| from_ld(&p.variables))
            .unwrap_or_default(),
    }
}

fn build_generated(store: &ProjectStore, title: &str) -> Result<HmiDoc, ApiError> {
    const COL_W: i32 = 300;
    const ROW_H: i32 = 34;
    const PAD: i32 = 24;
    const SECTION_HEAD: i32 = 34;
    const TOP: i32 = 64; // below the alarmbar
    const COLS: usize = 4; // masonry: fills the 1280 grid, then stacks

    let mut doc = project::hmi::empty_hmi(title);
    let mut children: Vec<HmiNode> = Vec::new();

    let mk = |id: String, kind: HmiNodeKind, x: i32, y: i32, w: i32, h: i32| HmiNode {
        id,
        kind,
        x,
        y,
        w,
        h,
        bind: BTreeMap::new(),
        action: BTreeMap::new(),
    };

    // POU paths flatten '/' to '_' for ids, so distinct paths can collide
    // (pous/a/b vs pous/a_b — and their variables too). A numeric suffix
    // keeps every id unique, and stays deterministic because
    // list_pou_paths is sorted.
    let mut used_ids = std::collections::HashSet::new();
    let mut unique_id = move |base: String| -> String {
        if used_ids.insert(base.clone()) {
            return base;
        }
        let mut n = 2;
        loop {
            let candidate = format!("{base}_{n}");
            if used_ids.insert(candidate.clone()) {
                return candidate;
            }
            n += 1;
        }
    };

    children.push(mk(
        unique_id("alarms".into()),
        HmiNodeKind::Alarmbar {},
        PAD,
        16,
        1280 - 2 * PAD,
        32,
    ));

    // Per-column running bottoms — each new section lands in the
    // currently-shortest column, so many-POU projects stack instead of
    // running off the right edge of the grid.
    let mut col_bottom = [TOP; COLS];
    let mut trend_candidates: Vec<String> = Vec::new();

    for pou_path in store.list_pou_paths()? {
        // Library blocks are implementation detail, not operator surface.
        if pou_path.starts_with("lib/") {
            continue;
        }
        let vars = pou_gen_vars(store, &pou_path);
        if vars.is_empty() {
            continue;
        }
        let col = (0..COLS).min_by_key(|&c| col_bottom[c]).unwrap_or(0);
        let sec_x = PAD + (col as i32) * (COL_W + PAD);
        let mut y = col_bottom[col];
        let slug = pou_path.replace('/', "_");

        children.push(mk(
            unique_id(format!("sec_{slug}")),
            HmiNodeKind::Text {
                text: pou_path.clone(),
                style: project::HmiTextStyle::Section,
            },
            sec_x,
            y,
            COL_W,
            24,
        ));
        y += SECTION_HEAD;

        for v in vars.iter().take(14) {
            if v.direction == "fb_instance" {
                continue;
            }
            let id = unique_id(format!("{}_{}", slug, v.name.to_ascii_lowercase()));
            let upper = v.type_name.to_ascii_uppercase();
            let mut node = if upper.starts_with("BOOL") {
                let mut n = mk(
                    id,
                    HmiNodeKind::Symbol {
                        symbol: "indicator".into(),
                        props: BTreeMap::from([(
                            "label".to_string(),
                            serde_json::Value::String(v.name.clone()),
                        )]),
                    },
                    sec_x,
                    y,
                    COL_W,
                    24,
                );
                let key = if is_alarmish(&v.name) { "alarm" } else { "on" };
                n.bind.insert(key.into(), HmiBinding::Var(v.name.clone()));
                n
            } else if numericish(&v.type_name) && is_setpointish(&v.name) {
                let mut n = mk(
                    id,
                    HmiNodeKind::Input {
                        label: Some(v.name.clone()),
                        unit: None,
                    },
                    sec_x,
                    y,
                    COL_W,
                    28,
                );
                n.bind
                    .insert("value".into(), HmiBinding::Var(v.name.clone()));
                n.action.insert(
                    "commit".into(),
                    HmiAction::SetValue {
                        variable: v.name.clone(),
                        min: None,
                        max: None,
                        confirm: true,
                    },
                );
                n
            } else if numericish(&v.type_name) {
                trend_candidates.push(v.name.clone());
                let mut n = mk(
                    id,
                    HmiNodeKind::Value {
                        label: Some(v.name.clone()),
                        unit: None,
                    },
                    sec_x,
                    y,
                    COL_W,
                    24,
                );
                n.bind
                    .insert("value".into(), HmiBinding::Var(v.name.clone()));
                n
            } else {
                continue;
            };
            node.y = y;
            children.push(node);
            y += ROW_H;
        }
        col_bottom[col] = y + PAD;
    }

    if !trend_candidates.is_empty() {
        let series: Vec<HmiSeries> = trend_candidates
            .iter()
            .take(2)
            .map(|v| HmiSeries {
                variable: v.clone(),
                label: None,
            })
            .collect();
        children.push(mk(
            unique_id("trend_main".into()),
            HmiNodeKind::Trend {
                series,
                window_s: 300,
            },
            PAD,
            560,
            1280 - 2 * PAD,
            200,
        ));
    }

    if let HmiNodeKind::Group {
        children: root_children,
        ..
    } = &mut doc.root.kind
    {
        *root_children = children;
    }
    Ok(doc)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store_in(dir: &std::path::Path) -> ProjectStore {
        ProjectStore::create(dir.join("p"), "p").unwrap()
    }

    fn all_ids(doc: &HmiDoc) -> Vec<String> {
        fn walk(n: &HmiNode, out: &mut Vec<String>) {
            out.push(n.id.clone());
            if let HmiNodeKind::Group { children, .. } = &n.kind {
                for c in children {
                    walk(c, out);
                }
            }
        }
        let mut out = Vec::new();
        walk(&doc.root, &mut out);
        out
    }

    #[test]
    fn structural_errors_blocks_only_error_severity() {
        let warn = HmiIssue {
            severity: "warning".into(),
            node_id: None,
            message: "screen has no title".into(),
        };
        let err = HmiIssue {
            severity: "error".into(),
            node_id: Some("x".into()),
            message: "duplicate id 'x'".into(),
        };
        assert_eq!(structural_errors(std::slice::from_ref(&warn)), None);
        assert_eq!(
            structural_errors(&[warn, err]).as_deref(),
            Some("screen has structural errors: duplicate id 'x'")
        );
    }

    #[test]
    fn generate_survives_slug_collisions() {
        let dir = tempfile::tempdir().unwrap();
        let store = store_in(dir.path());
        // pous/a/b and pous/a_b both flatten to slug `a_b`; the shared
        // variable name makes the per-variable ids collide too.
        std::fs::create_dir_all(dir.path().join("p/pous/a")).unwrap();
        std::fs::write(
            dir.path().join("p/pous/a/b.st"),
            "PROGRAM ab_nested\nVAR\n  x : BOOL;\nEND_VAR\nx := x;\nEND_PROGRAM\n",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("p/pous/a_b.st"),
            "PROGRAM ab_flat\nVAR\n  x : BOOL;\nEND_VAR\nx := x;\nEND_PROGRAM\n",
        )
        .unwrap();

        let doc = build_generated(&store, "t").unwrap();
        let ids = all_ids(&doc);
        let unique: std::collections::HashSet<&String> = ids.iter().collect();
        assert_eq!(ids.len(), unique.len(), "duplicate node ids in {ids:?}");
        // The generated doc must pass the same gate PUT enforces.
        assert_eq!(structural_errors(&validate_hmi(&doc)), None);
    }

    #[test]
    fn check_flags_input_writes_and_dangling_navs() {
        let dir = tempfile::tempdir().unwrap();
        let store = store_in(dir.path());
        std::fs::write(
            dir.path().join("p/pous/plant.st"),
            "PROGRAM plant\nVAR_INPUT\n  sensor_in : REAL;\nEND_VAR\nVAR\n  valve_cmd : BOOL;\nEND_VAR\nvalve_cmd := valve_cmd;\nEND_PROGRAM\n",
        )
        .unwrap();
        store.create_hmi("detail", "Detail").unwrap();

        let mut doc = project::hmi::empty_hmi("t");
        let mk_button = |id: &str, var: &str| {
            let mut n = HmiNode {
                id: id.into(),
                kind: HmiNodeKind::Button { label: id.into() },
                x: 0,
                y: 0,
                w: 0,
                h: 0,
                bind: BTreeMap::new(),
                action: BTreeMap::new(),
            };
            n.action.insert(
                "tap".into(),
                HmiAction::Write {
                    variable: var.into(),
                    value: 1.0,
                    confirm: true,
                },
            );
            n
        };
        let nav = |id: &str, target: &str| HmiNode {
            id: id.into(),
            kind: HmiNodeKind::Nav {
                label: id.into(),
                target: target.into(),
            },
            x: 0,
            y: 0,
            w: 0,
            h: 0,
            bind: BTreeMap::new(),
            action: BTreeMap::new(),
        };
        if let HmiNodeKind::Group { children, .. } = &mut doc.root.kind {
            children.push(mk_button("w_input", "sensor_in"));
            children.push(mk_button("w_local", "valve_cmd"));
            children.push(nav("n_ok", "detail"));
            children.push(nav("n_dangling", "no_such_screen"));
        }

        let issues = check_hmi_doc(&store, &doc).unwrap();
        let hits = |frag: &str| -> Vec<&str> {
            issues
                .iter()
                .filter(|i| i.message.contains(frag))
                .map(|i| i.node_id.as_deref().unwrap_or(""))
                .collect()
        };
        // Only the input-direction write and the dangling target warn.
        assert_eq!(hits("declared as an input"), vec!["w_input"]);
        assert_eq!(hits("is not a screen"), vec!["n_dangling"]);
        assert!(issues.iter().all(|i| i.severity == "warning"), "{issues:?}");
    }
}
