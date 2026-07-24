//! HMI screen documents — the project's fourth authorable artifact.
//!
//! A screen is one JSON file under `hmi/<slug>.hmi.json`, exactly parallel
//! to a graphical POU: serde types here are the single source of truth,
//! ts-rs exports them to the web renderer/editor, and `validate_hmi` is the
//! structural half of the check story (the server layers variable-existence
//! checks on top, where the program's variable index lives).
//!
//! Design doc: docs/hmi-design.md. The schema is deliberately a CLOSED,
//! small set — a generated document should be almost always valid and
//! reviewable in a diff. Agents author screens through `HmiOp` batches
//! (add/update/remove one node at a time), which is what makes incremental
//! generation — and the canvas's per-element spawn animation — possible.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Current document schema version. Bump on breaking shape changes;
/// `read_hmi` keeps accepting older versions via serde defaults.
pub const HMI_VERSION: u32 = 1;

/// Built-in symbol names the renderer implements. `validate_hmi` warns on
/// anything else (the canvas renders a placeholder, so unknown symbols are
/// visible, not fatal) — keeping this list in `project` lets the CLI and
/// the server share it without a round-trip.
pub const HMI_SYMBOLS: &[&str] = &[
    "tank",
    "valve",
    "pump",
    "motor",
    "pipe_h",
    "pipe_v",
    "gauge",
    "indicator",
    "setpoint",
    "analog",
    "bar",
    "led",
    "sparkline",
    "pipe",
    "fan",
    "conveyor",
];

fn default_hmi_version() -> u32 {
    HMI_VERSION
}
fn default_level() -> u8 {
    2
}
fn default_true() -> bool {
    true
}
fn default_window_s() -> u32 {
    300
}
fn default_pulse_ms() -> u32 {
    500
}

/// One operator screen. `title` is what operators read; the file slug is
/// the identity used by `nav` targets and URLs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct HmiDoc {
    #[serde(default = "default_hmi_version")]
    pub version: u32,
    pub title: String,
    /// ISA-101 display level (1 plant overview … 4 diagnostic detail).
    /// Informational in v1 — used by generate and by navigation grouping.
    #[serde(default = "default_level")]
    pub level: u8,
    #[serde(default)]
    pub grid: HmiGrid,
    pub root: HmiNode,
}

/// Design surface + snap step, in CSS pixels. The canvas letterboxes the
/// grid into whatever window it gets, so coordinates are stable across
/// clients (the operator tablet renders the same layout as the IDE).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct HmiGrid {
    pub w: u32,
    pub h: u32,
    pub snap: u32,
}

impl Default for HmiGrid {
    fn default() -> Self {
        Self {
            w: 1280,
            h: 800,
            snap: 8,
        }
    }
}

/// One element on a screen. Position/size are grid pixels; `w`/`h` of 0
/// mean "the node's intrinsic size". `bind` maps a prop name to a live
/// value; `action` maps a gesture to a write — both are empty objects for
/// purely static nodes (always serialized, so the wire shape matches the
/// generated TS types exactly).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct HmiNode {
    pub id: String,
    #[serde(flatten)]
    pub kind: HmiNodeKind,
    #[serde(default)]
    pub x: i32,
    #[serde(default)]
    pub y: i32,
    #[serde(default)]
    pub w: i32,
    #[serde(default)]
    pub h: i32,
    #[serde(default)]
    pub bind: BTreeMap<String, HmiBinding>,
    #[serde(default)]
    pub action: BTreeMap<String, HmiAction>,
}

/// The closed node vocabulary. Anything an operator screen needs is either
/// here or is a `symbol` — there is intentionally no free-form vector or
/// rich-text node in v1.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "snake_case")]
#[ts(export)]
pub enum HmiNodeKind {
    /// Container. `absolute` children carry their own x/y; `vertical` /
    /// `horizontal` stack them with `gap` (headers, button bars).
    Group {
        #[serde(default)]
        layout: HmiLayout,
        #[serde(default)]
        gap: u32,
        #[serde(default)]
        children: Vec<HmiNode>,
    },
    /// Static label. `style` picks one of the fixed text roles; `props`
    /// optionally overrides presentation (`color`, `size`, `align`,
    /// `weight`). Bind `text` (with a map) to turn it into a live state
    /// label, `color` to recolor it by value.
    Text {
        text: String,
        #[serde(default)]
        style: HmiTextStyle,
        #[serde(default)]
        #[ts(type = "Record<string, unknown>")]
        props: BTreeMap<String, serde_json::Value>,
    },
    /// Live readout: bind `value`; shows label, formatted value, unit.
    Value {
        #[serde(default)]
        label: Option<String>,
        #[serde(default)]
        unit: Option<String>,
    },
    /// Instance of a built-in symbol (see [`HMI_SYMBOLS`]). Props are
    /// symbol-specific plain JSON (label, orientation …); binds are the
    /// symbol's live inputs (a valve's `open`, a tank's `value`).
    Symbol {
        symbol: String,
        #[serde(default)]
        #[ts(type = "Record<string, unknown>")]
        props: BTreeMap<String, serde_json::Value>,
    },
    /// Strip chart over the live snapshot stream (client-side ring buffer,
    /// `window_s` seconds).
    Trend {
        series: Vec<HmiSeries>,
        #[serde(default = "default_window_s")]
        window_s: u32,
    },
    /// Renders the run's fault (`last_error`) + per-device health. One per
    /// screen is the convention; generate puts it at the top.
    Alarmbar {},
    /// Momentary control surface — pair with an `action.tap`. Optional
    /// `bind.on`: the renderer lights the button while the bound value
    /// is truthy (indicator rules), so a toggle shows the state it
    /// controls.
    Button { label: String },
    /// Numeric entry — pair with an `action.commit` of kind `set_value`.
    Input {
        #[serde(default)]
        label: Option<String>,
        #[serde(default)]
        unit: Option<String>,
    },
    /// Jump to another screen by slug.
    Nav { label: String, target: String },
    /// Process graphics: rect/ellipse vessels, line/polyline pipes.
    /// `props` styles the shape (`fill`, `stroke`, `stroke_width`, `rx`,
    /// `dash`); bind `fill`/`stroke` (with maps) for state-driven color.
    Shape {
        shape: HmiShapeKind,
        #[serde(default)]
        points: Vec<[i32; 2]>,
        #[serde(default)]
        #[ts(type = "Record<string, unknown>")]
        props: BTreeMap<String, serde_json::Value>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum HmiLayout {
    #[default]
    Absolute,
    Vertical,
    Horizontal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum HmiTextStyle {
    #[default]
    Body,
    Section,
    Title,
    Caption,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum HmiShapeKind {
    Rect,
    Ellipse,
    Line,
    Polyline,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct HmiSeries {
    pub variable: String,
    #[serde(default)]
    pub label: Option<String>,
}

/// A live binding: either just a variable name (the 90% case, resolved
/// with Monitor's rules, including `instance.variable`), or a spec with a
/// single-variable expression (`x` is the bound value) and a printf-ish
/// format. Single-variable on purpose — cross-variable logic belongs in a
/// POU, not hidden in a screen.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(untagged)]
#[ts(export)]
pub enum HmiBinding {
    Var(String),
    Spec(HmiBindingSpec),
}

impl HmiBinding {
    pub fn variable(&self) -> &str {
        match self {
            HmiBinding::Var(v) => v,
            HmiBinding::Spec(s) => &s.variable,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct HmiBindingSpec {
    pub variable: String,
    /// Pure arithmetic/comparison over `x` (the bound value), e.g.
    /// `x / 100`, `x > 50`. Evaluated client-side; no side effects.
    #[serde(default)]
    pub expr: Option<String>,
    /// printf-ish: `%.1f`, `%d`, `%s` (raw value text). Applied after
    /// `expr` when no map entry matches.
    #[serde(default)]
    pub format: Option<String>,
    /// Value → output mapping, tried in order after `expr`; the first
    /// matching entry wins. Outputs are strings — a color (token or CSS)
    /// for color-class props, display text for text-class props. This is
    /// the declarative equivalent of a SCADA "map transform": state
    /// colors and state labels without scripting.
    #[serde(default)]
    pub map: Option<Vec<HmiMapEntry>>,
}

/// One map rule. `eq` matches exactly; otherwise `min`/`max` bound a
/// half-open range `[min, max)` (either side may be omitted). An entry
/// with no condition matches everything — put it last as the fallback.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct HmiMapEntry {
    #[serde(default)]
    pub eq: Option<f64>,
    #[serde(default)]
    pub min: Option<f64>,
    #[serde(default)]
    pub max: Option<f64>,
    pub out: String,
}

/// The write path. Actions are the ONLY way a screen mutates the plant,
/// each declares itself in the document (diff-reviewable), and `confirm`
/// defaults to true — silent writes are opt-in per action, not the norm.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[ts(export)]
pub enum HmiAction {
    /// Write a fixed value.
    Write {
        variable: String,
        value: f64,
        #[serde(default = "default_true")]
        confirm: bool,
    },
    /// Read the bound value, write its boolean inverse.
    Toggle {
        variable: String,
        #[serde(default = "default_true")]
        confirm: bool,
    },
    /// Write 1, then 0 after `ms` (jog / reset pulses).
    Pulse {
        variable: String,
        #[serde(default = "default_pulse_ms")]
        ms: u32,
        #[serde(default = "default_true")]
        confirm: bool,
    },
    /// Commit the entered number (Input nodes). Bounds are enforced
    /// client-side AND rechecked by validate.
    SetValue {
        variable: String,
        #[serde(default)]
        min: Option<f64>,
        #[serde(default)]
        max: Option<f64>,
        #[serde(default = "default_true")]
        confirm: bool,
    },
    /// Client-side navigation to another screen.
    Nav { target: String },
}

impl HmiAction {
    pub fn variable(&self) -> Option<&str> {
        match self {
            HmiAction::Write { variable, .. }
            | HmiAction::Toggle { variable, .. }
            | HmiAction::Pulse { variable, .. }
            | HmiAction::SetValue { variable, .. } => Some(variable),
            HmiAction::Nav { .. } => None,
        }
    }
}

// ============================================================
//  Incremental ops — the agent's authoring surface
// ============================================================

/// One structured edit. Agents generate screens by POSTing small batches
/// of these (usually one node per call) so the canvas can render each
/// element as it lands; the whole-document PUT stays for editors that
/// already hold the full tree.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(tag = "op", rename_all = "snake_case")]
#[ts(export)]
pub enum HmiOp {
    /// Insert `node` under `parent` (root group when omitted) at `index`
    /// (append when omitted). Fails if the id already exists anywhere.
    AddNode {
        #[serde(default)]
        parent: Option<String>,
        node: HmiNode,
        #[serde(default)]
        index: Option<usize>,
    },
    /// Shallow-merge `patch` into the node object with this id: object
    /// fields merge one level (`bind`, `action`, `props`), scalars and
    /// arrays replace, `null` removes a key. `id` and `type` cannot be
    /// changed — remove + add instead.
    UpdateNode {
        id: String,
        #[ts(type = "Record<string, unknown>")]
        patch: serde_json::Value,
    },
    /// Remove the node (and, for groups, its whole subtree).
    RemoveNode { id: String },
    /// Update document metadata without touching the tree.
    SetMeta {
        #[serde(default)]
        title: Option<String>,
        #[serde(default)]
        level: Option<u8>,
        #[serde(default)]
        grid: Option<HmiGrid>,
    },
}

/// Apply a batch atomically: any failing op rejects the whole batch and
/// leaves `doc` untouched. Returns the ids the batch created or modified —
/// the server forwards them over SSE so the canvas knows which elements to
/// animate in.
pub fn apply_hmi_ops(doc: &mut HmiDoc, ops: &[HmiOp]) -> Result<Vec<String>, String> {
    let mut work = doc.clone();
    let mut touched = Vec::new();
    for (i, op) in ops.iter().enumerate() {
        apply_one(&mut work, op, &mut touched).map_err(|e| format!("op #{i}: {e}"))?;
    }
    *doc = work;
    Ok(touched)
}

fn apply_one(doc: &mut HmiDoc, op: &HmiOp, touched: &mut Vec<String>) -> Result<(), String> {
    match op {
        HmiOp::AddNode {
            parent,
            node,
            index,
        } => {
            // Identity checks cover the whole added subtree, not just the
            // top node — a group carrying duplicate or empty child ids
            // must never persist (PUT would reject the resulting doc).
            let mut fresh = std::collections::HashSet::new();
            for id in collect_ids(node) {
                if id.trim().is_empty() {
                    return Err("node id must not be empty".into());
                }
                if !fresh.insert(id.clone()) {
                    return Err(format!("duplicate id '{id}' within the added subtree"));
                }
                if find_node(&doc.root, &id).is_some() {
                    let what = if id == node.id { "id" } else { "descendant id" };
                    return Err(format!("{what} '{id}' already exists"));
                }
            }
            let parent_id = parent.as_deref().unwrap_or(&doc.root.id).to_string();
            let target = find_node_mut(&mut doc.root, &parent_id)
                .ok_or_else(|| format!("parent '{parent_id}' not found"))?;
            let HmiNodeKind::Group { children, .. } = &mut target.kind else {
                return Err(format!("parent '{parent_id}' is not a group"));
            };
            let at = index.unwrap_or(children.len()).min(children.len());
            touched.extend(collect_ids(node));
            children.insert(at, node.clone());
            Ok(())
        }
        HmiOp::UpdateNode { id, patch } => {
            let node =
                find_node_mut(&mut doc.root, id).ok_or_else(|| format!("node '{id}' not found"))?;
            let mut value = serde_json::to_value(&*node).map_err(|e| e.to_string())?;
            let before_type = value.get("type").cloned();
            merge_shallow(&mut value, patch);
            if value.get("id") != Some(&serde_json::Value::String(id.clone())) {
                return Err("patch may not change 'id'".into());
            }
            if value.get("type") != before_type.as_ref() {
                return Err("patch may not change 'type' — remove + add instead".into());
            }
            let next: HmiNode = serde_json::from_value(value)
                .map_err(|e| format!("patched node is not valid: {e}"))?;
            *node = next;
            touched.push(id.clone());
            Ok(())
        }
        HmiOp::RemoveNode { id } => {
            if *id == doc.root.id {
                return Err("cannot remove the root group".into());
            }
            if !remove_node(&mut doc.root, id) {
                return Err(format!("node '{id}' not found"));
            }
            touched.push(id.clone());
            Ok(())
        }
        HmiOp::SetMeta { title, level, grid } => {
            if let Some(t) = title {
                doc.title = t.clone();
            }
            if let Some(l) = level {
                doc.level = *l;
            }
            if let Some(g) = grid {
                doc.grid = *g;
            }
            Ok(())
        }
    }
}

/// One level of object merge: object-valued keys in `patch` merge into
/// object-valued keys in `base` (one level deep — enough for `bind` /
/// `action` / `props`), `null` deletes, everything else replaces.
fn merge_shallow(base: &mut serde_json::Value, patch: &serde_json::Value) {
    let (Some(base_map), Some(patch_map)) = (base.as_object_mut(), patch.as_object()) else {
        return;
    };
    for (k, v) in patch_map {
        match v {
            serde_json::Value::Null => {
                base_map.remove(k);
            }
            serde_json::Value::Object(inner) => {
                let slot = base_map
                    .entry(k.clone())
                    .or_insert_with(|| serde_json::Value::Object(Default::default()));
                if let Some(slot_map) = slot.as_object_mut() {
                    for (ik, iv) in inner {
                        if iv.is_null() {
                            slot_map.remove(ik);
                        } else {
                            slot_map.insert(ik.clone(), iv.clone());
                        }
                    }
                } else {
                    *slot = v.clone();
                }
            }
            other => {
                base_map.insert(k.clone(), other.clone());
            }
        }
    }
}

pub fn find_node<'a>(root: &'a HmiNode, id: &str) -> Option<&'a HmiNode> {
    if root.id == id {
        return Some(root);
    }
    if let HmiNodeKind::Group { children, .. } = &root.kind {
        children.iter().find_map(|c| find_node(c, id))
    } else {
        None
    }
}

fn find_node_mut<'a>(root: &'a mut HmiNode, id: &str) -> Option<&'a mut HmiNode> {
    if root.id == id {
        return Some(root);
    }
    if let HmiNodeKind::Group { children, .. } = &mut root.kind {
        children.iter_mut().find_map(|c| find_node_mut(c, id))
    } else {
        None
    }
}

fn remove_node(root: &mut HmiNode, id: &str) -> bool {
    if let HmiNodeKind::Group { children, .. } = &mut root.kind {
        if let Some(pos) = children.iter().position(|c| c.id == id) {
            children.remove(pos);
            return true;
        }
        children.iter_mut().any(|c| remove_node(c, id))
    } else {
        false
    }
}

fn collect_ids(node: &HmiNode) -> Vec<String> {
    let mut out = vec![node.id.clone()];
    if let HmiNodeKind::Group { children, .. } = &node.kind {
        for c in children {
            out.extend(collect_ids(c));
        }
    }
    out
}

/// Every variable a document references (bindings, actions, trend series)
/// — the server checks these against the project's variable index.
pub fn hmi_variables(doc: &HmiDoc) -> Vec<(String, String)> {
    let mut out = Vec::new();
    walk(&doc.root, &mut |n| {
        for b in n.bind.values() {
            out.push((n.id.clone(), b.variable().to_string()));
        }
        for a in n.action.values() {
            if let Some(v) = a.variable() {
                out.push((n.id.clone(), v.to_string()));
            }
        }
        if let HmiNodeKind::Trend { series, .. } = &n.kind {
            for s in series {
                out.push((n.id.clone(), s.variable.clone()));
            }
        }
    });
    out
}

/// Variables a document WRITES (write/toggle/pulse/set_value action
/// targets) — the server checks these against declaration directions.
/// Nav actions are excluded: they navigate, they never touch the plant.
pub fn hmi_write_variables(doc: &HmiDoc) -> Vec<(String, String)> {
    let mut out = Vec::new();
    walk(&doc.root, &mut |n| {
        for a in n.action.values() {
            if let Some(v) = a.variable() {
                out.push((n.id.clone(), v.to_string()));
            }
        }
    });
    out
}

/// Navigation targets — from `nav` nodes and nav-kind actions. Empty
/// targets are validate_hmi's finding; the server checks the rest
/// against the project's screen list.
pub fn hmi_nav_targets(doc: &HmiDoc) -> Vec<(String, String)> {
    let mut out = Vec::new();
    walk(&doc.root, &mut |n| {
        if let HmiNodeKind::Nav { target, .. } = &n.kind {
            out.push((n.id.clone(), target.clone()));
        }
        for a in n.action.values() {
            if let HmiAction::Nav { target } = a {
                out.push((n.id.clone(), target.clone()));
            }
        }
    });
    out
}

fn walk(node: &HmiNode, f: &mut impl FnMut(&HmiNode)) {
    f(node);
    if let HmiNodeKind::Group { children, .. } = &node.kind {
        for c in children {
            walk(c, f);
        }
    }
}

/// One structural finding. `severity` mirrors CheckDiagnostic's vocabulary
/// so the server can map these 1:1.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct HmiIssue {
    pub severity: String,
    pub node_id: Option<String>,
    pub message: String,
}

fn err(node: Option<&str>, msg: impl Into<String>) -> HmiIssue {
    HmiIssue {
        severity: "error".into(),
        node_id: node.map(str::to_string),
        message: msg.into(),
    }
}
fn warn(node: Option<&str>, msg: impl Into<String>) -> HmiIssue {
    HmiIssue {
        severity: "warning".into(),
        node_id: node.map(str::to_string),
        message: msg.into(),
    }
}

/// Node types whose renderer affordance is a control surface — the only
/// types the canvas wires gestures on. Everything else must stay inert so
/// a write can't hide behind an innocent-looking label (the actions-are-
/// the-reviewable-write-surface fence in docs/hmi-design.md).
fn hosts_actions(kind: &HmiNodeKind) -> bool {
    matches!(
        kind,
        HmiNodeKind::Button { .. }
            | HmiNodeKind::Input { .. }
            | HmiNodeKind::Symbol { .. }
            | HmiNodeKind::Nav { .. }
    )
}

fn kind_name(kind: &HmiNodeKind) -> &'static str {
    match kind {
        HmiNodeKind::Group { .. } => "group",
        HmiNodeKind::Text { .. } => "text",
        HmiNodeKind::Value { .. } => "value",
        HmiNodeKind::Symbol { .. } => "symbol",
        HmiNodeKind::Trend { .. } => "trend",
        HmiNodeKind::Alarmbar {} => "alarmbar",
        HmiNodeKind::Button { .. } => "button",
        HmiNodeKind::Input { .. } => "input",
        HmiNodeKind::Nav { .. } => "nav",
        HmiNodeKind::Shape { .. } => "shape",
    }
}

/// Structural validation — pure, no project context. Duplicate/empty ids,
/// non-group root, unknown symbols (warning), empty trend series, actions
/// on nodes that can't host them, out-of-order SetValue bounds.
pub fn validate_hmi(doc: &HmiDoc) -> Vec<HmiIssue> {
    let mut issues = Vec::new();
    if !matches!(doc.root.kind, HmiNodeKind::Group { .. }) {
        issues.push(err(Some(&doc.root.id), "root node must be a group"));
    }
    if doc.title.trim().is_empty() {
        issues.push(warn(None, "screen has no title"));
    }
    if !(1..=4).contains(&doc.level) {
        issues.push(warn(
            None,
            format!("level {} outside ISA-101 1..4", doc.level),
        ));
    }

    let mut seen = std::collections::HashSet::new();
    walk(&doc.root, &mut |n| {
        if n.id.trim().is_empty() {
            issues.push(err(None, "node with empty id"));
        } else if !seen.insert(n.id.clone()) {
            issues.push(err(Some(&n.id), format!("duplicate id '{}'", n.id)));
        }
        match &n.kind {
            HmiNodeKind::Symbol { symbol, .. } if !HMI_SYMBOLS.contains(&symbol.as_str()) => {
                issues.push(warn(
                    Some(&n.id),
                    format!(
                        "unknown symbol '{symbol}' (built-ins: {})",
                        HMI_SYMBOLS.join(", ")
                    ),
                ));
            }
            // Flow layouts were designed but never implemented, and the
            // decision (issue #23) is to retire them: screens are spatial
            // documents — absolute coordinates are the truth the arrange
            // editor, spawn overlays and letterboxing all build on.
            HmiNodeKind::Group { layout, .. } if *layout != HmiLayout::Absolute => {
                issues.push(warn(
                    Some(&n.id),
                    "flow layouts are retired — children render at their absolute x/y; \
                     lay this group out with coordinates",
                ));
            }
            // Warnings, not errors: the palette places empty trends and
            // navs on purpose and the canvas renders a visible
            // placeholder — incomplete must stay saveable, or every
            // whole-document save after a palette drop would bounce.
            HmiNodeKind::Trend { series, .. } if series.is_empty() => {
                issues.push(warn(Some(&n.id), "trend has no series"));
            }
            HmiNodeKind::Nav { target, .. } if target.trim().is_empty() => {
                issues.push(warn(Some(&n.id), "nav target is empty"));
            }
            HmiNodeKind::Input { .. } if !n.action.contains_key("commit") => {
                issues.push(warn(
                    Some(&n.id),
                    "input has no `commit` action — entries will go nowhere",
                ));
            }
            _ => {}
        }
        if !n.action.is_empty() && !hosts_actions(&n.kind) {
            issues.push(err(
                Some(&n.id),
                format!(
                    "{} node cannot host actions (allowed on: button, input, symbol, nav)",
                    kind_name(&n.kind)
                ),
            ));
        }
        for (gesture, a) in &n.action {
            if let HmiAction::SetValue {
                min: Some(lo),
                max: Some(hi),
                ..
            } = a
            {
                if lo > hi {
                    issues.push(err(
                        Some(&n.id),
                        format!("action '{gesture}': min {lo} > max {hi}"),
                    ));
                }
            }
            if let HmiAction::Pulse { ms, .. } = a {
                // The runtime holds the value high for the whole window
                // (the reset is server-side by contract) — a multi-second
                // "pulse" is almost always a mis-modelled toggle.
                if *ms > 10_000 {
                    issues.push(warn(
                        Some(&n.id),
                        format!(
                            "action '{gesture}': pulse holds for {ms} ms — \
                             longer than 10 s is probably a toggle, not a pulse"
                        ),
                    ));
                }
            }
        }
        for (prop, b) in &n.bind {
            let HmiBinding::Spec(spec) = b else { continue };
            for (i, entry) in spec.map.iter().flatten().enumerate() {
                if entry.out.trim().is_empty() {
                    issues.push(err(
                        Some(&n.id),
                        format!("bind '{prop}': map entry #{i} has empty out"),
                    ));
                }
                if let (Some(lo), Some(hi)) = (entry.min, entry.max) {
                    if lo > hi {
                        issues.push(err(
                            Some(&n.id),
                            format!("bind '{prop}': map entry #{i} min {lo} > max {hi}"),
                        ));
                    }
                }
            }
        }
    });
    issues
}

/// A fresh, valid, empty screen — what `create_hmi` writes.
pub fn empty_hmi(title: &str) -> HmiDoc {
    HmiDoc {
        version: HMI_VERSION,
        title: title.to_string(),
        level: 2,
        grid: HmiGrid::default(),
        root: HmiNode {
            id: "root".into(),
            kind: HmiNodeKind::Group {
                layout: HmiLayout::Absolute,
                gap: 0,
                children: Vec::new(),
            },
            x: 0,
            y: 0,
            w: 0,
            h: 0,
            bind: BTreeMap::new(),
            action: BTreeMap::new(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(id: &str, kind: HmiNodeKind) -> HmiNode {
        HmiNode {
            id: id.into(),
            kind,
            x: 0,
            y: 0,
            w: 0,
            h: 0,
            bind: BTreeMap::new(),
            action: BTreeMap::new(),
        }
    }

    #[test]
    fn round_trips_through_json() {
        let mut doc = empty_hmi("Overview");
        let mut v = node(
            "lvl",
            HmiNodeKind::Value {
                label: Some("Level".into()),
                unit: Some("%".into()),
            },
        );
        v.bind
            .insert("value".into(), HmiBinding::Var("level_pct".into()));
        v.action.insert(
            "tap".into(),
            HmiAction::Toggle {
                variable: "pump_cmd".into(),
                confirm: true,
            },
        );
        apply_hmi_ops(
            &mut doc,
            &[HmiOp::AddNode {
                parent: None,
                node: v,
                index: None,
            }],
        )
        .unwrap();

        let json = serde_json::to_string_pretty(&doc).unwrap();
        let back: HmiDoc = serde_json::from_str(&json).unwrap();
        assert_eq!(doc, back);
        // Bare-string binding stays a bare string on the wire.
        assert!(json.contains("\"value\": \"level_pct\""));
    }

    #[test]
    fn add_rejects_duplicate_and_missing_parent() {
        let mut doc = empty_hmi("t");
        let n = node("a", HmiNodeKind::Alarmbar {});
        apply_hmi_ops(
            &mut doc,
            &[HmiOp::AddNode {
                parent: None,
                node: n.clone(),
                index: None,
            }],
        )
        .unwrap();
        let dup = apply_hmi_ops(
            &mut doc,
            &[HmiOp::AddNode {
                parent: None,
                node: n.clone(),
                index: None,
            }],
        );
        assert!(dup.unwrap_err().contains("already exists"));
        let orphan = apply_hmi_ops(
            &mut doc,
            &[HmiOp::AddNode {
                parent: Some("nope".into()),
                node: node("b", HmiNodeKind::Alarmbar {}),
                index: None,
            }],
        );
        assert!(orphan.unwrap_err().contains("not found"));
    }

    #[test]
    fn add_validates_ids_across_the_whole_subtree() {
        let group = |children: Vec<HmiNode>| HmiNodeKind::Group {
            layout: HmiLayout::Absolute,
            gap: 0,
            children,
        };
        let add = |n: HmiNode| HmiOp::AddNode {
            parent: None,
            node: n,
            index: None,
        };

        // Two children sharing an id — the batch must fail, not persist a
        // document PUT would reject.
        let mut doc = empty_hmi("t");
        let dup = apply_hmi_ops(
            &mut doc,
            &[add(node(
                "g",
                group(vec![
                    node("dup", HmiNodeKind::Alarmbar {}),
                    node("dup", HmiNodeKind::Alarmbar {}),
                ]),
            ))],
        );
        assert!(dup.unwrap_err().contains("within the added subtree"));

        // Empty ids anywhere in the subtree, not just the top node.
        let empty = apply_hmi_ops(
            &mut doc,
            &[add(node(
                "g",
                group(vec![node("", HmiNodeKind::Alarmbar {})]),
            ))],
        );
        assert!(empty.unwrap_err().contains("must not be empty"));

        // A descendant colliding with a node already in the tree.
        apply_hmi_ops(&mut doc, &[add(node("a", HmiNodeKind::Alarmbar {}))]).unwrap();
        let clash = apply_hmi_ops(
            &mut doc,
            &[add(node(
                "g",
                group(vec![node("a", HmiNodeKind::Alarmbar {})]),
            ))],
        );
        assert!(clash
            .unwrap_err()
            .contains("descendant id 'a' already exists"));
    }

    #[test]
    fn batch_is_atomic_and_reports_touched_ids() {
        let mut doc = empty_hmi("t");
        let touched = apply_hmi_ops(
            &mut doc,
            &[
                HmiOp::AddNode {
                    parent: None,
                    node: node("a", HmiNodeKind::Alarmbar {}),
                    index: None,
                },
                HmiOp::AddNode {
                    parent: None,
                    node: node("b", HmiNodeKind::Button { label: "Go".into() }),
                    index: None,
                },
            ],
        )
        .unwrap();
        assert_eq!(touched, vec!["a".to_string(), "b".to_string()]);

        // Second op fails → first op must not have landed.
        let before = doc.clone();
        let res = apply_hmi_ops(
            &mut doc,
            &[
                HmiOp::AddNode {
                    parent: None,
                    node: node("c", HmiNodeKind::Alarmbar {}),
                    index: None,
                },
                HmiOp::RemoveNode { id: "ghost".into() },
            ],
        );
        assert!(res.is_err());
        assert_eq!(doc, before, "failed batch must leave the doc untouched");
    }

    #[test]
    fn update_merges_shallow_and_guards_identity() {
        let mut doc = empty_hmi("t");
        let mut v = node(
            "v1",
            HmiNodeKind::Value {
                label: Some("Flow".into()),
                unit: None,
            },
        );
        v.bind
            .insert("value".into(), HmiBinding::Var("flow".into()));
        apply_hmi_ops(
            &mut doc,
            &[HmiOp::AddNode {
                parent: None,
                node: v,
                index: None,
            }],
        )
        .unwrap();

        // Move it + add a second binding; existing binding survives the
        // one-level merge.
        apply_hmi_ops(
            &mut doc,
            &[HmiOp::UpdateNode {
                id: "v1".into(),
                patch: serde_json::json!({ "x": 40, "bind": { "alarm": "flow_hh" } }),
            }],
        )
        .unwrap();
        let n = find_node(&doc.root, "v1").unwrap();
        assert_eq!(n.x, 40);
        assert_eq!(n.bind.len(), 2);

        // null deletes a key inside a merged object.
        apply_hmi_ops(
            &mut doc,
            &[HmiOp::UpdateNode {
                id: "v1".into(),
                patch: serde_json::json!({ "bind": { "alarm": null } }),
            }],
        )
        .unwrap();
        assert_eq!(find_node(&doc.root, "v1").unwrap().bind.len(), 1);

        let bad = apply_hmi_ops(
            &mut doc,
            &[HmiOp::UpdateNode {
                id: "v1".into(),
                patch: serde_json::json!({ "type": "button" }),
            }],
        );
        assert!(bad.unwrap_err().contains("may not change 'type'"));
    }

    #[test]
    fn validate_flags_the_documented_problems() {
        let mut doc = empty_hmi("t");
        apply_hmi_ops(
            &mut doc,
            &[
                HmiOp::AddNode {
                    parent: None,
                    node: node(
                        "s1",
                        HmiNodeKind::Symbol {
                            symbol: "flux_capacitor".into(),
                            props: BTreeMap::new(),
                        },
                    ),
                    index: None,
                },
                HmiOp::AddNode {
                    parent: None,
                    node: node(
                        "t1",
                        HmiNodeKind::Trend {
                            series: vec![],
                            window_s: 300,
                        },
                    ),
                    index: None,
                },
                HmiOp::AddNode {
                    parent: None,
                    node: node(
                        "i1",
                        HmiNodeKind::Input {
                            label: None,
                            unit: None,
                        },
                    ),
                    index: None,
                },
            ],
        )
        .unwrap();
        let issues = validate_hmi(&doc);
        let sev = |frag: &str| {
            issues
                .iter()
                .find(|i| i.message.contains(frag))
                .unwrap_or_else(|| panic!("no issue containing '{frag}'"))
                .severity
                .clone()
        };
        assert_eq!(sev("unknown symbol"), "warning");
        assert_eq!(sev("no `commit` action"), "warning");
        // Incomplete-but-renderable states are warnings: the palette
        // places empty trends/navs on purpose, so they must stay
        // saveable through PUT and /ops.
        assert_eq!(sev("no series"), "warning");
    }

    #[test]
    fn validate_warns_on_empty_nav_target() {
        let mut doc = empty_hmi("t");
        apply_hmi_ops(
            &mut doc,
            &[HmiOp::AddNode {
                parent: None,
                node: node(
                    "n1",
                    HmiNodeKind::Nav {
                        label: "Detail".into(),
                        target: "".into(),
                    },
                ),
                index: None,
            }],
        )
        .unwrap();
        let issues = validate_hmi(&doc);
        let nav = issues
            .iter()
            .find(|i| i.message.contains("nav target is empty"))
            .unwrap();
        assert_eq!(nav.severity, "warning");
    }

    #[test]
    fn validate_rejects_actions_on_non_host_nodes() {
        let write = HmiAction::Write {
            variable: "valve_cmd".into(),
            value: 1.0,
            confirm: false,
        };

        // The regression: a confirm:false write hidden behind a plain
        // text label. Must be an error-severity finding.
        let mut doc = empty_hmi("t");
        let mut label = node(
            "l1",
            HmiNodeKind::Text {
                text: "Just a label".into(),
                style: HmiTextStyle::Body,
                props: BTreeMap::new(),
            },
        );
        label.action.insert("tap".into(), write.clone());
        apply_hmi_ops(
            &mut doc,
            &[HmiOp::AddNode {
                parent: None,
                node: label,
                index: None,
            }],
        )
        .unwrap();
        let issues = validate_hmi(&doc);
        let hit = issues
            .iter()
            .find(|i| i.message.contains("cannot host actions"))
            .unwrap();
        assert_eq!(hit.severity, "error");
        assert_eq!(hit.node_id.as_deref(), Some("l1"));

        // Control-surface types keep hosting actions.
        let mut doc = empty_hmi("t");
        let mut btn = node("b1", HmiNodeKind::Button { label: "Go".into() });
        btn.action.insert("tap".into(), write);
        let mut sym = node(
            "s1",
            HmiNodeKind::Symbol {
                symbol: "valve".into(),
                props: BTreeMap::new(),
            },
        );
        sym.action.insert(
            "tap".into(),
            HmiAction::Toggle {
                variable: "valve_cmd".into(),
                confirm: true,
            },
        );
        apply_hmi_ops(
            &mut doc,
            &[
                HmiOp::AddNode {
                    parent: None,
                    node: btn,
                    index: None,
                },
                HmiOp::AddNode {
                    parent: None,
                    node: sym,
                    index: None,
                },
            ],
        )
        .unwrap();
        assert!(!validate_hmi(&doc)
            .iter()
            .any(|i| i.message.contains("cannot host actions")));
    }

    #[test]
    fn write_and_nav_surfaces_are_collected() {
        let mut doc = empty_hmi("t");
        let mut btn = node("b", HmiNodeKind::Button { label: "Go".into() });
        btn.action.insert(
            "tap".into(),
            HmiAction::Write {
                variable: "valve_cmd".into(),
                value: 1.0,
                confirm: true,
            },
        );
        let mut jump = node(
            "j",
            HmiNodeKind::Nav {
                label: "Detail".into(),
                target: "detail".into(),
            },
        );
        jump.action.insert(
            "tap".into(),
            HmiAction::Nav {
                target: "other".into(),
            },
        );
        apply_hmi_ops(
            &mut doc,
            &[
                HmiOp::AddNode {
                    parent: None,
                    node: btn,
                    index: None,
                },
                HmiOp::AddNode {
                    parent: None,
                    node: jump,
                    index: None,
                },
            ],
        )
        .unwrap();

        // Writes exclude nav actions; nav targets come from both the nav
        // node's own target and nav-kind actions.
        let writes: Vec<_> = hmi_write_variables(&doc)
            .into_iter()
            .map(|(_, v)| v)
            .collect();
        assert_eq!(writes, vec!["valve_cmd"]);
        let navs: Vec<_> = hmi_nav_targets(&doc).into_iter().map(|(_, t)| t).collect();
        assert_eq!(navs, vec!["detail", "other"]);
    }

    #[test]
    fn map_binding_round_trips_and_validates() {
        let mut doc = empty_hmi("t");
        let mut v = node(
            "v",
            HmiNodeKind::Value {
                label: Some("Temp".into()),
                unit: Some("°C".into()),
            },
        );
        v.bind.insert(
            "color".into(),
            HmiBinding::Spec(HmiBindingSpec {
                variable: "temp".into(),
                expr: None,
                format: None,
                map: Some(vec![
                    HmiMapEntry {
                        eq: None,
                        min: Some(80.0),
                        max: None,
                        out: "alarm".into(),
                    },
                    HmiMapEntry {
                        eq: None,
                        min: None,
                        max: None,
                        out: "ok".into(),
                    },
                ]),
            }),
        );
        apply_hmi_ops(
            &mut doc,
            &[HmiOp::AddNode {
                parent: None,
                node: v,
                index: None,
            }],
        )
        .unwrap();
        let json = serde_json::to_string(&doc).unwrap();
        let back: HmiDoc = serde_json::from_str(&json).unwrap();
        assert_eq!(doc, back);
        assert!(validate_hmi(&doc).is_empty());

        // Inverted range and empty out are structural errors.
        apply_hmi_ops(
            &mut doc,
            &[HmiOp::UpdateNode {
                id: "v".into(),
                patch: serde_json::json!({ "bind": { "color": {
                    "variable": "temp",
                    "map": [
                        { "min": 50.0, "max": 10.0, "out": "warn" },
                        { "out": "" }
                    ]
                } } }),
            }],
        )
        .unwrap();
        let issues = validate_hmi(&doc);
        assert!(issues.iter().any(|i| i.message.contains("min 50 > max 10")));
        assert!(issues.iter().any(|i| i.message.contains("empty out")));
    }

    #[test]
    fn variables_are_collected_from_all_surfaces() {
        let mut doc = empty_hmi("t");
        let mut v = node(
            "v",
            HmiNodeKind::Value {
                label: None,
                unit: None,
            },
        );
        v.bind.insert(
            "value".into(),
            HmiBinding::Spec(HmiBindingSpec {
                variable: "a".into(),
                expr: Some("x / 10".into()),
                format: None,
                map: None,
            }),
        );
        v.action.insert(
            "tap".into(),
            HmiAction::Write {
                variable: "b".into(),
                value: 1.0,
                confirm: true,
            },
        );
        let t = node(
            "t",
            HmiNodeKind::Trend {
                series: vec![HmiSeries {
                    variable: "c".into(),
                    label: None,
                }],
                window_s: 60,
            },
        );
        apply_hmi_ops(
            &mut doc,
            &[
                HmiOp::AddNode {
                    parent: None,
                    node: v,
                    index: None,
                },
                HmiOp::AddNode {
                    parent: None,
                    node: t,
                    index: None,
                },
            ],
        )
        .unwrap();
        let vars: Vec<String> = hmi_variables(&doc).into_iter().map(|(_, v)| v).collect();
        assert_eq!(vars, vec!["a", "b", "c"]);
    }
}
