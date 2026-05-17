//! Bridge build script: embed ironplc's problem documentation (one RST
//! per error code) into the binary so `CheckDiagnostic` can ship the
//! explanation alongside every diagnostic.
//!
//! Pattern adapted from `vendor/ironplc/compiler/mcp/build.rs` —
//! they emit a `lookup_problem_doc(code) -> Option<(rst, title)>`
//! function from a build.rs `include_str!` table. We do the same so
//! the docs live in the binary and lookups are zero-allocation.

use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

fn main() {
    let manifest_dir = env::var("CARGO_MANIFEST_DIR").unwrap();
    generate_problem_docs(&manifest_dir);
}

fn generate_problem_docs(manifest_dir: &str) {
    // Paths into the vendored ironplc repo. The bridge crate sits at
    // `crates/ironplc-bridge/`; ironplc's docs/problems CSV are at
    // `vendor/ironplc/...` — two ".." segments up plus a hop down.
    let problems_dir =
        Path::new(manifest_dir).join("../../vendor/ironplc/docs/reference/compiler/problems");
    let csv_path = Path::new(manifest_dir)
        .join("../../vendor/ironplc/compiler/problems/resources/problem-codes.csv");

    println!("cargo:rerun-if-changed={}", problems_dir.display());
    println!("cargo:rerun-if-changed={}", csv_path.display());

    // Read CSV: Code,Name,Message — keep `Message` as the human title.
    let mut titles: BTreeMap<String, String> = BTreeMap::new();
    if let Ok(csv_content) = fs::read_to_string(&csv_path) {
        for line in csv_content.lines().skip(1) {
            let fields: Vec<&str> = line.splitn(3, ',').collect();
            if fields.len() == 3 {
                titles.insert(fields[0].to_string(), fields[2].to_string());
            }
        }
    }

    // Collect P####.rst files. Sort so the generated source is
    // deterministic (helps incremental rebuilds + diffs).
    let mut entries: BTreeMap<String, PathBuf> = BTreeMap::new();
    if let Ok(dir) = fs::read_dir(&problems_dir) {
        for entry in dir.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with('P') && name.ends_with(".rst") {
                let code = name.trim_end_matches(".rst").to_string();
                let abs_path = fs::canonicalize(entry.path()).unwrap();
                println!("cargo:rerun-if-changed={}", abs_path.display());
                entries.insert(code, abs_path);
            }
        }
    }

    let out_dir = env::var("OUT_DIR").unwrap();
    let dest = Path::new(&out_dir).join("problem_docs.rs");

    let mut code = String::from(
        "// Auto-generated from vendor/ironplc/docs/reference/compiler/problems/.\n\
         // Do not edit — change the .rst sources upstream and rebuild.\n\n",
    );
    code.push_str("/// Returns `(rst_body, title)` for a known problem code, or `None`\n");
    code.push_str("/// if the code isn't in ironplc's registry.\n");
    code.push_str(
        "pub fn lookup_problem_doc(code: &str) -> Option<(&'static str, &'static str)> {\n",
    );
    code.push_str("    match code {\n");
    for (problem_code, abs_path) in &entries {
        let title = titles.get(problem_code).map(|s| s.as_str()).unwrap_or("");
        let path_str = abs_path.display().to_string().replace('\\', "/");
        let escaped_title = title.replace('\\', "\\\\").replace('"', "\\\"");
        code.push_str(&format!(
            "        \"{problem_code}\" => Some((include_str!(\"{path_str}\"), \"{escaped_title}\")),\n"
        ));
    }
    code.push_str("        _ => None,\n");
    code.push_str("    }\n");
    code.push_str("}\n");

    fs::write(&dest, code).unwrap();
}
