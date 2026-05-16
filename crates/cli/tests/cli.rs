//! End-to-end tests for the `cs` CLI.
//!
//! We use `assert_cmd` to invoke the compiled binary as a real
//! subprocess, then check stdout / stderr / exit code. This is the
//! same view an agent gets — if these tests pass, the agent-facing
//! contract is intact.

use std::fs;
use std::path::PathBuf;

use assert_cmd::Command;
use predicates::str::contains;

/// Path to a small valid LD POU bundled as a test fixture.
fn good_ld() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.push("tests/fixtures/good.ld.json");
    p
}

/// Path to a small LD POU that references an undeclared variable —
/// exercises the diagnostic / ld_location wiring end-to-end.
fn bad_ld() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.push("tests/fixtures/bad.ld.json");
    p
}

/// Path to a small valid FBD POU: R_TRIG → CTU pipeline.
fn good_fbd() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.push("tests/fixtures/good.fbd.json");
    p
}

/// Path to an FBD POU with one undeclared variable on a block input
/// pin — exercises the fbd_location wiring.
fn bad_fbd() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.push("tests/fixtures/bad.fbd.json");
    p
}

/// Path to a valid SFC POU: idle → filling → draining → idle.
fn good_sfc() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.push("tests/fixtures/good.sfc.json");
    p
}

/// Path to an SFC POU whose `running` step's N action references an
/// undeclared variable `ghost` — exercises the sfc_location wiring.
fn bad_sfc() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.push("tests/fixtures/bad.sfc.json");
    p
}

fn cs() -> Command {
    Command::cargo_bin("cs").expect("compiled cs binary should exist")
}

#[test]
fn check_clean_file_exits_zero() {
    cs().arg("check")
        .arg(good_ld())
        .assert()
        .success()
        .stderr(contains("clean"));
}

#[test]
fn check_dirty_file_exits_one_and_reports_diagnostic() {
    let out = cs().arg("check").arg(bad_ld()).assert().code(1);
    // Human-readable mode: stderr carries the diagnostic + summary,
    // stdout stays empty so pipelines that capture stdout don't see
    // garbage when a file is dirty.
    let assert = out.get_output();
    let stderr = String::from_utf8_lossy(&assert.stderr);
    assert!(
        stderr.contains("Variable not defined"),
        "expected the undefined-var diagnostic in stderr; got:\n{stderr}",
    );
    assert!(
        stderr.contains("rung loose · coil 0"),
        "ld_location should be printed in human mode; got:\n{stderr}",
    );
}

#[test]
fn check_json_mode_emits_structured_payload() {
    let out = cs()
        .arg("check")
        .arg(bad_ld())
        .arg("--json")
        .assert()
        .code(1);
    let stdout = String::from_utf8_lossy(&out.get_output().stdout);
    // Parse it back as JSON — round-trip is the real contract.
    let v: serde_json::Value =
        serde_json::from_str(&stdout).expect("--json output must be valid JSON");
    assert_eq!(v["ok"], false);
    let diag = &v["files"][0]["diagnostics"][0];
    // Critical: the ld_location is structured, not a string. Agents
    // can pattern-match on `kind` without parsing free-form text.
    assert_eq!(diag["ld_location"]["kind"], "coil");
    assert_eq!(diag["ld_location"]["rung_id"], "loose");
    assert_eq!(diag["ld_location"]["coil_index"], 0);
}

#[test]
fn check_multiple_files_aggregates_results() {
    cs().arg("check")
        .arg(good_ld())
        .arg(bad_ld())
        .arg("--json")
        .assert()
        .code(1) // any-error policy
        .stdout(contains("\"ok\": false"))
        .stdout(contains("\"diagnostics\": []")) // good file is clean
        .stdout(contains("nope")); // bad file's diagnostic
}

#[test]
fn explain_known_code_prints_doc_exits_zero() {
    // P4007 is one of ironplc's most common errors — the embedded
    // RST page mentions "variable" repeatedly.
    let out = cs().arg("explain").arg("P4007").assert().success();
    let stdout = String::from_utf8_lossy(&out.get_output().stdout);
    // First line is the "code — title" summary.
    assert!(stdout.starts_with("P4007"), "got:\n{stdout}");
    // Body should include the RST title block AND prose about variables.
    assert!(
        stdout.to_lowercase().contains("variable"),
        "expected RST body to mention 'variable'; got:\n{stdout}"
    );
}

#[test]
fn explain_unknown_code_exits_one() {
    cs().arg("explain")
        .arg("XX-NOT-A-CODE")
        .assert()
        .code(1)
        .stderr(contains("no documentation"));
}

#[test]
fn check_explain_appends_problem_doc_to_human_output() {
    // Bad LD fixture references undeclared variable `nope` → P4007.
    // With `--explain`, the full RST body must follow the diagnostic.
    let out = cs()
        .arg("check")
        .arg(bad_ld())
        .arg("--explain")
        .assert()
        .code(1);
    let stderr = String::from_utf8_lossy(&out.get_output().stderr);
    // The diagnostic line itself.
    assert!(stderr.contains("P4007"), "got:\n{stderr}");
    // Context line under the diagnostic.
    assert!(
        stderr.contains("variable=nope") || stderr.contains("nope"),
        "expected context line mentioning the offending var; got:\n{stderr}"
    );
    // Explanation prose (from the embedded RST).
    assert!(
        stderr.to_lowercase().contains("example"),
        "explanation should include the RST 'Example' section; got:\n{stderr}"
    );
}

#[test]
fn check_json_includes_context_related_explanation() {
    let out = cs()
        .arg("check")
        .arg(bad_ld())
        .arg("--json")
        .assert()
        .code(1);
    let stdout = String::from_utf8_lossy(&out.get_output().stdout);
    let v: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    let diag = &v["files"][0]["diagnostics"][0];
    // Three new agent-facing fields. context/related are serialised
    // with `skip_serializing_if = "Vec::is_empty"`, so an empty array
    // doesn't appear on the wire — we treat absent OR array as fine.
    if !diag["context"].is_null() {
        assert!(diag["context"].is_array());
    }
    if !diag["related"].is_null() {
        assert!(diag["related"].is_array());
    }
    let expl = diag["explanation"]
        .as_str()
        .expect("ironplc P-codes must carry an embedded explanation");
    assert!(
        expl.to_lowercase().contains("variable"),
        "explanation should describe the error: {expl}"
    );
    // For P4007 we definitely expect a context entry naming `nope`.
    let ctx = diag["context"]
        .as_array()
        .expect("P4007 always carries a `variable=…` context line");
    assert!(
        ctx.iter().any(|c| c.as_str().unwrap_or("").contains("nope")),
        "expected `variable=nope` in context, got: {ctx:?}"
    );
}

#[test]
fn check_unknown_extension_is_usage_error() {
    let tmp = tempfile::NamedTempFile::with_suffix(".plc").unwrap();
    cs().arg("check")
        .arg(tmp.path())
        .assert()
        // anyhow wraps the can't-infer-language error, surfaced via
        // exit code 3 (infra) — language inference is a precondition,
        // not a user-source error. The agent reads stderr to learn what
        // went wrong; exit code 3 says "fix your invocation, don't fix
        // your source".
        .code(3)
        .stderr(contains("can't infer language"));
}

#[test]
fn transpile_ld_emits_st_on_stdout() {
    let out = cs().arg("transpile").arg(good_ld()).assert().success();
    let stdout = String::from_utf8_lossy(&out.get_output().stdout);
    assert!(stdout.contains("PROGRAM good"), "got:\n{stdout}");
    assert!(stdout.contains("VAR_INPUT"));
    // FB calls must hoist before the coil assignment
    assert!(
        stdout.contains("armTimer(IN := start_btn"),
        "FB call should be present; got:\n{stdout}"
    );
}

#[test]
fn transpile_with_map_includes_source_map() {
    let out = cs()
        .arg("transpile")
        .arg(good_ld())
        .arg("--with-map")
        .assert()
        .success();
    let stdout = String::from_utf8_lossy(&out.get_output().stdout);
    let v: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert!(v["st"].is_string(), "st field must be a string");
    assert!(v["source_map"].is_array(), "source_map must be an array");
    // The map should contain at least one Variable entry and one
    // Rung/Coil entry — proves it isn't just nulls.
    let map = v["source_map"].as_array().unwrap();
    let has_variable = map
        .iter()
        .any(|e| !e.is_null() && e["kind"] == "variable");
    let has_rung_or_coil = map
        .iter()
        .any(|e| !e.is_null() && (e["kind"] == "rung" || e["kind"] == "coil"));
    assert!(has_variable && has_rung_or_coil, "got:\n{stdout}");
}

#[test]
fn transpile_st_file_echoes_source() {
    // ST is its own intermediate; `transpile` should be a no-op echo.
    let tmp_dir = tempfile::tempdir().unwrap();
    let st = tmp_dir.path().join("foo.st");
    fs::write(&st, "PROGRAM foo\n  VAR x : BOOL; END_VAR\nEND_PROGRAM\n").unwrap();
    let out = cs().arg("transpile").arg(&st).assert().success();
    let stdout = String::from_utf8_lossy(&out.get_output().stdout);
    assert!(stdout.contains("PROGRAM foo"));
}

#[test]
fn project_info_lists_pous_and_devices() {
    let proj = setup_demo_project();
    let out = cs()
        .arg("project")
        .arg("info")
        .arg(proj.path())
        .arg("--json")
        .assert()
        .success();
    let stdout = String::from_utf8_lossy(&out.get_output().stdout);
    let v: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(v["name"], "smoke");
    let pous = v["pous"].as_array().unwrap();
    assert_eq!(pous.len(), 1);
    assert_eq!(pous[0], "main");
}

#[test]
fn fbd_check_clean_file_exits_zero() {
    cs().arg("check")
        .arg(good_fbd())
        .assert()
        .success()
        .stderr(contains("clean"));
}

#[test]
fn fbd_check_dirty_file_reports_fbd_location() {
    let out = cs()
        .arg("check")
        .arg(bad_fbd())
        .arg("--json")
        .assert()
        .code(1);
    let stdout = String::from_utf8_lossy(&out.get_output().stdout);
    let v: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    let diag = &v["files"][0]["diagnostics"][0];
    // The ghost variable is on block b0's IN pin; ironplc reports the
    // error on the line of the FB call, so fbd_location should be
    // `Block { block_id: "b0" }` — NOT ld_location.
    assert!(diag["ld_location"].is_null());
    assert_eq!(diag["fbd_location"]["kind"], "block");
    assert_eq!(diag["fbd_location"]["block_id"], "b0");
}

#[test]
fn fbd_transpile_emits_topo_sorted_calls() {
    let out = cs().arg("transpile").arg(good_fbd()).assert().success();
    let stdout = String::from_utf8_lossy(&out.get_output().stdout);
    // VAR section declares both FB instances
    assert!(stdout.contains("rt : R_TRIG;"), "got:\n{stdout}");
    assert!(stdout.contains("cu : CTU;"), "got:\n{stdout}");
    // CTU references R_TRIG via dot access on the source instance
    assert!(stdout.contains("cu(CU := rt.Q"), "got:\n{stdout}");
    // Topo order: edge before counter
    let edge = stdout.find("rt(CLK").unwrap();
    let counter = stdout.find("cu(CU").unwrap();
    assert!(edge < counter, "edge block must execute before counter");
    // Output binding lands at the end
    assert!(stdout.contains("done := cu.Q;"), "got:\n{stdout}");
}

#[test]
fn sfc_check_clean_file_exits_zero() {
    cs().arg("check")
        .arg(good_sfc())
        .assert()
        .success()
        .stderr(contains("clean"));
}

#[test]
fn sfc_check_dirty_file_reports_sfc_location() {
    let out = cs()
        .arg("check")
        .arg(bad_sfc())
        .arg("--json")
        .assert()
        .code(1);
    let stdout = String::from_utf8_lossy(&out.get_output().stdout);
    let v: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    let diag = &v["files"][0]["diagnostics"][0];
    // The `ghost` reference is inside the `running` step's action[0]
    // body; ironplc reports the error on that emitted action line, so
    // sfc_location should be Action(running, 0) — NOT ld/fbd.
    assert!(diag["ld_location"].is_null());
    assert!(diag["fbd_location"].is_null());
    assert_eq!(diag["sfc_location"]["kind"], "action");
    assert_eq!(diag["sfc_location"]["step"], "running");
    assert_eq!(diag["sfc_location"]["action_index"], 0);
}

#[test]
fn sfc_transpile_emits_step_dispatch_and_transition_cascade() {
    let out = cs().arg("transpile").arg(good_sfc()).assert().success();
    let stdout = String::from_utf8_lossy(&out.get_output().stdout);
    // SFC bookkeeping variables
    assert!(
        stdout.contains("__sfc_step : STRING[31] := 'idle';"),
        "got:\n{stdout}"
    );
    // Per-step IF dispatch
    assert!(
        stdout.contains("IF __sfc_step = 'filling' THEN"),
        "got:\n{stdout}"
    );
    assert!(stdout.contains("inlet := TRUE;"), "got:\n{stdout}");
    // Transition cascade — IF/ELSIF chain in author order
    assert!(
        stdout.contains("IF __sfc_step = 'idle' AND (start_btn) THEN"),
        "got:\n{stdout}"
    );
    assert!(
        stdout.contains("ELSIF __sfc_step = 'filling' AND (tank_full) THEN"),
        "got:\n{stdout}"
    );
    // Prev snapshot sits between actions and transitions
    let actions_pos = stdout.find("(* === SFC actions === *)").unwrap();
    let snap_pos = stdout.find("__sfc_prev := __sfc_step;").unwrap();
    let trans_pos = stdout.find("(* === SFC transitions === *)").unwrap();
    assert!(actions_pos < snap_pos && snap_pos < trans_pos);
}

#[test]
fn project_check_clean_exits_zero() {
    let proj = setup_demo_project();
    cs().arg("project")
        .arg("check")
        .arg(proj.path())
        .assert()
        .success()
        .stderr(contains("compiles cleanly"));
}

/// Build a minimum viable project on a tempdir — one POU, no devices,
/// a single task binding `main` so `compile_project` succeeds.
fn setup_demo_project() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("project.toml"),
        "name = \"smoke\"\nversion = \"0.1\"\n",
    )
    .unwrap();
    fs::write(
        dir.path().join("tasks.toml"),
        r#"
[[tasks]]
name = "t1"
interval_ms = 100
priority = 1

[[programs]]
instance = "main"
program = "main"
task = "t1"
"#,
    )
    .unwrap();
    fs::create_dir(dir.path().join("pous")).unwrap();
    fs::write(
        dir.path().join("pous/main.st"),
        "PROGRAM main\n  VAR x : BOOL; END_VAR\n  x := TRUE;\nEND_PROGRAM\n",
    )
    .unwrap();
    fs::create_dir(dir.path().join("devices")).unwrap();
    fs::create_dir(dir.path().join("edges")).unwrap();
    fs::write(dir.path().join("iomap.toml"), "[[mappings]]\n").unwrap();
    dir
}
