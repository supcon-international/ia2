use std::fs;
use std::path::{Path, PathBuf};

use crate::errors::StoreError;
use crate::types::{
    Device, Edge, IoMap, ModbusConfig, PouFileSource, PouLanguage, PouType, ProgramInstance,
    ProjectManifest, ProjectTreeSkeleton, Protocol, ProtocolConfig, Task, Tasks,
};

const SAMPLE_MAIN_ST: &str = include_str!("../templates/main.st");

pub struct ProjectStore {
    root: PathBuf,
    manifest: ProjectManifest,
}

impl ProjectStore {
    /// Open an existing project at `root`. Fails if `project.toml` is missing.
    ///
    /// Auto-migrates legacy projects: if `applications/` exists and `pous/`
    /// doesn't, the directory is renamed. The old name was a misnomer
    /// (Codesys uses "Application" for a deployment bundle of POUs+tasks+IO;
    /// here it just held POU source files) and the rename brings the
    /// on-disk layout into line with the rest of the model.
    pub fn open(root: PathBuf) -> Result<Self, StoreError> {
        let manifest_path = root.join("project.toml");
        if !manifest_path.exists() {
            return Err(StoreError::NotFound(root.display().to_string()));
        }
        let text = fs::read_to_string(&manifest_path)?;
        let manifest: ProjectManifest = toml::from_str(&text)?;
        let store = Self { root, manifest };
        store.migrate_applications_to_pous()?;
        Ok(store)
    }

    /// Rename `applications/` → `pous/` if needed. Idempotent.
    fn migrate_applications_to_pous(&self) -> Result<(), StoreError> {
        let old = self.root.join("applications");
        let new = self.root.join("pous");
        if old.exists() && !new.exists() {
            fs::rename(&old, &new)?;
            tracing::info!(
                project = %self.manifest.name,
                "migrated applications/ → pous/"
            );
        }
        Ok(())
    }

    /// Create a fresh project at `root`. Fails if `root` already contains a
    /// `project.toml`. Seeds the directory with a sample PROGRAM and an
    /// empty iomap.
    pub fn create(root: PathBuf, name: &str) -> Result<Self, StoreError> {
        validate_path(name)?;
        let manifest_path = root.join("project.toml");
        if manifest_path.exists() {
            return Err(StoreError::AlreadyExists(root.display().to_string()));
        }

        fs::create_dir_all(&root)?;
        fs::create_dir_all(root.join("pous"))?;
        fs::create_dir_all(root.join("devices"))?;
        fs::create_dir_all(root.join("edges"))?;

        let manifest = ProjectManifest {
            name: name.to_string(),
            version: "0.1".into(),
        };
        fs::write(&manifest_path, toml::to_string_pretty(&manifest)?)?;
        fs::write(
            root.join("iomap.toml"),
            toml::to_string_pretty(&IoMap::default())?,
        )?;
        fs::write(root.join("pous/main.st"), SAMPLE_MAIN_ST)?;
        // Seed tasks.toml with a single 100 ms task running `main`. Users
        // edit this via the Tasks pane.
        let seed_tasks = Tasks {
            tasks: vec![Task {
                name: "plc_task".into(),
                interval_ms: 100,
                priority: 1,
            }],
            programs: vec![ProgramInstance {
                instance: "main_inst".into(),
                program: "main".into(),
                task: "plc_task".into(),
            }],
        };
        fs::write(
            root.join("tasks.toml"),
            toml::to_string_pretty(&seed_tasks)?,
        )?;

        Ok(Self { root, manifest })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn name(&self) -> &str {
        &self.manifest.name
    }

    /// Project tree without parsed declarations — the server fills those
    /// in by calling `ironplc_bridge::extract_pou_declarations` on each
    /// source. Doing it this way keeps the project crate parser-free
    /// (no dependency on the bridge / ironplc).
    pub fn tree_skeleton(&self) -> Result<ProjectTreeSkeleton, StoreError> {
        let pou_paths = self.list_pou_paths()?;
        let mut pous = Vec::with_capacity(pou_paths.len());
        for path in pou_paths {
            let language = self.pou_file_language(&path)?;
            let source = self.read_pou_source(&path)?;
            pous.push(PouFileSource {
                path,
                source,
                language,
            });
        }
        let devices = self.list_devices()?;
        let edges = self.list_edges()?;
        let pou_folders = self.list_folders("pous")?;
        let device_folders = self.list_folders("devices")?;
        let edge_folders = self.list_folders("edges")?;
        let iomap = self.read_iomap()?;
        let tasks = self.read_tasks()?.unwrap_or_default();
        Ok(ProjectTreeSkeleton {
            name: self.manifest.name.clone(),
            path: self.root.display().to_string(),
            pous,
            pou_folders,
            devices,
            device_folders,
            edges,
            edge_folders,
            iomap,
            tasks,
        })
    }

    // ---------------- POU files ----------------
    //
    // The store deals in *files* — `pous/<path>.st`. POU *declarations*
    // (PROGRAM / FUNCTION_BLOCK / FUNCTION) inside each file are parser-
    // driven and exposed via the bridge in the server layer; the store
    // stays parser-free.

    pub fn list_pou_paths(&self) -> Result<Vec<String>, StoreError> {
        let root = self.root.join("pous");
        if !root.exists() {
            return Ok(vec![]);
        }
        let mut out = Vec::new();
        walk_pou_files(&root, "", &mut |slug, _lang, _src| {
            out.push(slug.to_string());
            Ok(())
        })?;
        out.sort();
        out.dedup();
        Ok(out)
    }

    /// Locate the on-disk file for a POU slug, returning both the path
    /// and the language detected from its extension. We probe `.st`
    /// first (the common case) then `.ld.json`. When graphical languages
    /// expand we add more probes here — one source of truth for the
    /// "slug → file" mapping.
    pub fn pou_file_path(&self, slug: &str) -> Option<(PathBuf, PouLanguage)> {
        let dir = self.root.join("pous");
        let st = dir.join(format!("{slug}.st"));
        if st.exists() {
            return Some((st, PouLanguage::St));
        }
        let ld = dir.join(format!("{slug}.ld.json"));
        if ld.exists() {
            return Some((ld, PouLanguage::Ld));
        }
        None
    }

    /// Read the raw on-disk contents of a POU file, regardless of
    /// language. For `.st` this is the IEC source; for `.ld.json` it's
    /// the JSON literal — the caller (typically the bridge) decides
    /// what to do with it based on the language hint from
    /// [`pou_file_language`](Self::pou_file_language).
    pub fn read_pou_source(&self, path: &str) -> Result<String, StoreError> {
        validate_path(path)?;
        let (file, _lang) = self
            .pou_file_path(path)
            .ok_or_else(|| StoreError::PouNotFound(path.into()))?;
        Ok(fs::read_to_string(&file)?)
    }

    /// Read the language of an existing POU. Returns `PouNotFound` if
    /// the slug doesn't resolve to a file on disk.
    pub fn pou_file_language(&self, path: &str) -> Result<PouLanguage, StoreError> {
        validate_path(path)?;
        let (_, lang) = self
            .pou_file_path(path)
            .ok_or_else(|| StoreError::PouNotFound(path.into()))?;
        Ok(lang)
    }

    /// Overwrite the POU file at `path`. The on-disk language is
    /// preserved — we don't allow a save to change `.st` into `.ld.json`
    /// or vice versa, because that would silently invalidate any
    /// in-flight UI state. Creating a new POU of a different language
    /// is `create_pou_file`.
    pub fn write_pou_source(&self, path: &str, source: &str) -> Result<(), StoreError> {
        validate_path(path)?;
        let (file, _lang) = match self.pou_file_path(path) {
            Some(pair) => pair,
            None => {
                // First write of a freshly-named POU — fall back to
                // `.st`. New non-ST POUs come through `create_pou_file`
                // which sets the extension explicitly.
                let p = self.root.join("pous").join(format!("{path}.st"));
                (p, PouLanguage::St)
            }
        };
        if let Some(parent) = file.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(file, source)?;
        Ok(())
    }

    /// Create a new POU file with a single declaration of `type_` in
    /// `language`. Returns the seeded source.
    ///
    /// Supported languages: ST and LD. Others (FBD / SFC / IL / CFC)
    /// still error out cleanly so the UI can surface a "coming soon"
    /// message rather than silently writing a file the bridge can't
    /// compile.
    pub fn create_pou_file(
        &self,
        path: &str,
        type_: PouType,
        language: PouLanguage,
    ) -> Result<String, StoreError> {
        validate_path(path)?;
        if self.pou_file_path(path).is_some() {
            return Err(StoreError::AlreadyExists(path.into()));
        }
        let leaf = leaf_segment(path);
        let (file, source) = match language {
            PouLanguage::St => (
                self.root.join("pous").join(format!("{path}.st")),
                template_for(leaf, type_),
            ),
            PouLanguage::Ld => (
                self.root.join("pous").join(format!("{path}.ld.json")),
                template_for_ld(leaf, type_),
            ),
            other => {
                return Err(StoreError::UnsupportedLanguage(format!("{other:?}")));
            }
        };
        fs::create_dir_all(file.parent().unwrap())?;
        fs::write(&file, &source)?;
        Ok(source)
    }

    pub fn delete_pou_file(&self, path: &str) -> Result<(), StoreError> {
        validate_path(path)?;
        let (file, _lang) = self
            .pou_file_path(path)
            .ok_or_else(|| StoreError::PouNotFound(path.into()))?;
        fs::remove_file(file)?;
        Ok(())
    }

    pub fn create_pou_folder(&self, path: &str) -> Result<(), StoreError> {
        self.create_folder("pous", path)
    }

    pub fn delete_pou_folder(&self, path: &str) -> Result<(), StoreError> {
        self.delete_folder("pous", path)
    }

    // ---------------- Devices ----------------

    pub fn list_devices(&self) -> Result<Vec<Device>, StoreError> {
        let root = self.root.join("devices");
        if !root.exists() {
            return Ok(vec![]);
        }
        let mut out = Vec::new();
        walk_files(&root, "", "toml", &mut |rel, text| {
            let mut device: Device = toml::from_str(&text)?;
            // Honour the on-disk location as the canonical name. Old single-
            // segment files keep their existing name (rel == device.name);
            // nested files get the folder-qualified path so iomap mapping
            // can target the right device unambiguously.
            device.name = rel.to_string();
            out.push(device);
            Ok(())
        })?;
        out.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(out)
    }

    pub fn read_device(&self, name: &str) -> Result<Device, StoreError> {
        validate_path(name)?;
        let path = self.root.join("devices").join(format!("{name}.toml"));
        if !path.exists() {
            return Err(StoreError::DeviceNotFound(name.into()));
        }
        let text = fs::read_to_string(&path)?;
        let mut device: Device = toml::from_str(&text)?;
        device.name = name.into();
        Ok(device)
    }

    pub fn write_device(&self, device: &Device) -> Result<(), StoreError> {
        validate_path(&device.name)?;
        let leaf_name = leaf_segment(&device.name).to_string();
        let path = self
            .root
            .join("devices")
            .join(format!("{}.toml", device.name));
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        // Serialised form stores just the leaf — the folder path is implicit
        // from the file location, and double-encoding it would confuse old
        // top-level loaders. The list/read paths overwrite `name` with the
        // full path again.
        let on_disk = Device {
            name: leaf_name,
            config: device.config.clone(),
        };
        fs::write(path, toml::to_string_pretty(&on_disk)?)?;
        Ok(())
    }

    pub fn create_device(&self, name: &str, protocol: Protocol) -> Result<Device, StoreError> {
        validate_path(name)?;
        let path = self.root.join("devices").join(format!("{name}.toml"));
        if path.exists() {
            return Err(StoreError::AlreadyExists(name.into()));
        }
        let config = default_config_for(protocol);
        let device = Device {
            name: name.into(),
            config,
        };
        self.write_device(&device)?;
        Ok(device)
    }

    pub fn delete_device(&self, name: &str) -> Result<(), StoreError> {
        validate_path(name)?;
        let path = self.root.join("devices").join(format!("{name}.toml"));
        if !path.exists() {
            return Err(StoreError::DeviceNotFound(name.into()));
        }
        fs::remove_file(path)?;
        Ok(())
    }

    pub fn create_device_folder(&self, path: &str) -> Result<(), StoreError> {
        self.create_folder("devices", path)
    }

    // ---------------- Edges (deploy targets) ----------------

    pub fn list_edges(&self) -> Result<Vec<Edge>, StoreError> {
        let root = self.root.join("edges");
        if !root.exists() {
            return Ok(vec![]);
        }
        let mut out = Vec::new();
        walk_files(&root, "", "toml", &mut |rel, text| {
            let mut edge: Edge = toml::from_str(&text)?;
            edge.name = rel.to_string();
            out.push(edge);
            Ok(())
        })?;
        out.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(out)
    }

    pub fn read_edge(&self, name: &str) -> Result<Edge, StoreError> {
        validate_path(name)?;
        let path = self.root.join("edges").join(format!("{name}.toml"));
        if !path.exists() {
            return Err(StoreError::EdgeNotFound(name.into()));
        }
        let text = fs::read_to_string(&path)?;
        let mut edge: Edge = toml::from_str(&text)?;
        edge.name = name.into();
        Ok(edge)
    }

    pub fn write_edge(&self, edge: &Edge) -> Result<(), StoreError> {
        validate_path(&edge.name)?;
        let leaf_name = leaf_segment(&edge.name).to_string();
        let path = self.root.join("edges").join(format!("{}.toml", edge.name));
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let on_disk = Edge {
            name: leaf_name,
            ..edge.clone()
        };
        fs::write(path, toml::to_string_pretty(&on_disk)?)?;
        Ok(())
    }

    pub fn create_edge(&self, name: &str, host: &str) -> Result<Edge, StoreError> {
        validate_path(name)?;
        let path = self.root.join("edges").join(format!("{name}.toml"));
        if path.exists() {
            return Err(StoreError::AlreadyExists(name.into()));
        }
        let edge = Edge {
            name: name.into(),
            host: host.into(),
            ssh_port: 22,
            ssh_user: String::new(),
            install_dir: "/opt/controlsoftware".into(),
            runtime_port: 13001,
            notes: String::new(),
        };
        self.write_edge(&edge)?;
        Ok(edge)
    }

    pub fn delete_edge(&self, name: &str) -> Result<(), StoreError> {
        validate_path(name)?;
        let path = self.root.join("edges").join(format!("{name}.toml"));
        if !path.exists() {
            return Err(StoreError::EdgeNotFound(name.into()));
        }
        fs::remove_file(path)?;
        Ok(())
    }

    pub fn create_edge_folder(&self, path: &str) -> Result<(), StoreError> {
        self.create_folder("edges", path)
    }

    // ---------------- Folder helpers ----------------

    /// All subdirectories under `subdir` (e.g. "applications" / "devices"),
    /// returned as forward-slash separated relative paths. Includes empty
    /// folders — the UI needs them so it can render a folder before any
    /// items live inside.
    pub fn list_folders(&self, subdir: &str) -> Result<Vec<String>, StoreError> {
        let root = self.root.join(subdir);
        if !root.exists() {
            return Ok(vec![]);
        }
        let mut out = Vec::new();
        walk_dirs(&root, "", &mut |rel| {
            out.push(rel.to_string());
            Ok(())
        })?;
        out.sort();
        Ok(out)
    }

    fn create_folder(&self, subdir: &str, path: &str) -> Result<(), StoreError> {
        validate_path(path)?;
        let dir = self.root.join(subdir).join(path);
        if dir.exists() {
            return Err(StoreError::FolderExists(path.into()));
        }
        fs::create_dir_all(&dir)?;
        Ok(())
    }

    pub fn delete_device_folder(&self, path: &str) -> Result<(), StoreError> {
        self.delete_folder("devices", path)
    }

    pub fn delete_edge_folder(&self, path: &str) -> Result<(), StoreError> {
        self.delete_folder("edges", path)
    }

    /// Remove a folder under `subdir`. Requires the folder to be empty
    /// (no .st / .toml children, no sub-folders) — agents and humans
    /// alike should delete contents explicitly first so an accidental
    /// recursive wipe isn't possible via a single API call.
    fn delete_folder(&self, subdir: &str, path: &str) -> Result<(), StoreError> {
        validate_path(path)?;
        let dir = self.root.join(subdir).join(path);
        if !dir.exists() {
            return Err(StoreError::FolderNotFound(path.into()));
        }
        let mut entries = fs::read_dir(&dir)?;
        if entries.next().is_some() {
            return Err(StoreError::FolderNotEmpty(path.into()));
        }
        fs::remove_dir(&dir)?;
        Ok(())
    }

    // ---------------- IO Mapping ----------------

    pub fn read_iomap(&self) -> Result<IoMap, StoreError> {
        let path = self.root.join("iomap.toml");
        if !path.exists() {
            return Ok(IoMap::default());
        }
        let text = fs::read_to_string(&path)?;
        Ok(toml::from_str(&text)?)
    }

    pub fn write_iomap(&self, iomap: &IoMap) -> Result<(), StoreError> {
        let path = self.root.join("iomap.toml");
        fs::write(path, toml::to_string_pretty(iomap)?)?;
        Ok(())
    }

    // ---------------- Tasks (project-level scheduling) ----------------

    /// Read tasks.toml. Returns `None` when the file doesn't exist — that
    /// distinction matters: a fresh open of an old project needs to know
    /// the file is missing so it can offer migration.
    pub fn read_tasks(&self) -> Result<Option<Tasks>, StoreError> {
        let path = self.root.join("tasks.toml");
        if !path.exists() {
            return Ok(None);
        }
        let text = fs::read_to_string(&path)?;
        Ok(Some(toml::from_str(&text)?))
    }

    pub fn write_tasks(&self, tasks: &Tasks) -> Result<(), StoreError> {
        let path = self.root.join("tasks.toml");
        fs::write(path, toml::to_string_pretty(tasks)?)?;
        Ok(())
    }

    /// Auto-migrate an old project from per-POU inline CONFIGURATION to a
    /// project-level `tasks.toml`. Idempotent — if `tasks.toml` already
    /// exists this returns `Ok(MigrationReport::Skipped)`.
    ///
    /// Strategy: scan every POU file for `CONFIGURATION ... END_CONFIGURATION`
    /// blocks; extract the TASK definitions and `PROGRAM <inst> WITH <task>`
    /// bindings; merge into a single `Tasks` record; write `tasks.toml`;
    /// strip the CONFIGURATION blocks out of the POU files in place (git
    /// is the backup).
    pub fn migrate_tasks(&self) -> Result<MigrationReport, StoreError> {
        if self.read_tasks()?.is_some() {
            return Ok(MigrationReport::Skipped);
        }
        let pou_paths = self.list_pou_paths()?;
        let mut tasks: Vec<Task> = Vec::new();
        let mut programs: Vec<ProgramInstance> = Vec::new();
        let mut stripped: Vec<String> = Vec::new();
        let mut seen_task_names = std::collections::HashSet::<String>::new();
        let mut seen_instance_names = std::collections::HashSet::<String>::new();

        for path in &pou_paths {
            let source = self.read_pou_source(path)?;
            let extracted = extract_inline_configuration(&source);
            if extracted.is_empty() {
                continue;
            }
            // Strip every CONFIGURATION block from this POU.
            let stripped_source = strip_inline_configuration(&source);
            self.write_pou_source(path, &stripped_source)?;
            stripped.push(path.clone());

            for inline in extracted {
                for t in inline.tasks {
                    // Dedup by task name: first wins. Subsequent POUs that
                    // declared the same task name keep their PROGRAM
                    // instances mapped to it.
                    if seen_task_names.insert(t.name.clone()) {
                        tasks.push(t);
                    }
                }
                for p in inline.programs {
                    if seen_instance_names.insert(p.instance.clone()) {
                        programs.push(p);
                    }
                }
            }
        }

        // Seed defaults when migration found nothing to lift (e.g. project
        // with FB-only POUs that never had CONFIGURATION blocks). The user
        // can edit the Tasks pane to actually bind a program.
        if tasks.is_empty() {
            tasks.push(Task {
                name: "plc_task".into(),
                interval_ms: 100,
                priority: 1,
            });
        }
        let result = Tasks { tasks, programs };
        self.write_tasks(&result)?;
        Ok(MigrationReport::Migrated {
            tasks_count: result.tasks.len(),
            programs_count: result.programs.len(),
            pous_modified: stripped,
        })
    }
}

/// Pick a reasonable NIC default for EtherCAT. The user almost always
/// overrides this in the editor; we just want to avoid an empty box.
fn default_ethercat_nic() -> &'static str {
    if cfg!(target_os = "macos") {
        "en0"
    } else if cfg!(target_os = "windows") {
        "Ethernet"
    } else {
        "eth0"
    }
}

/// Outcome of `ProjectStore::migrate_tasks`.
#[derive(Debug, Clone)]
pub enum MigrationReport {
    /// `tasks.toml` already exists; no work done.
    Skipped,
    /// Inline CONFIGURATION blocks were extracted into `tasks.toml` and
    /// stripped from the POUs listed in `pous_modified`.
    Migrated {
        tasks_count: usize,
        programs_count: usize,
        pous_modified: Vec<String>,
    },
}

/// Tasks + program bindings extracted from one POU's inline CONFIGURATION
/// block. Internal-only intermediate type for migration.
struct InlineConfig {
    tasks: Vec<Task>,
    programs: Vec<ProgramInstance>,
}

/// Find every `CONFIGURATION … END_CONFIGURATION` block in a source file and
/// extract the TASK declarations + PROGRAM instance bindings. Best-effort
/// regex-free parsing — exact enough for legacy projects written by our
/// own template, tolerant of whitespace and case.
fn extract_inline_configuration(source: &str) -> Vec<InlineConfig> {
    let mut out = Vec::new();
    let lower = source.to_ascii_lowercase();
    let mut cursor = 0usize;
    while let Some(start) = lower[cursor..].find("configuration ") {
        let abs = cursor + start;
        let Some(end) = lower[abs..].find("end_configuration") else {
            break;
        };
        let abs_end = abs + end + "end_configuration".len();
        let block = &source[abs..abs_end];
        out.push(parse_configuration_block(block));
        cursor = abs_end;
    }
    out
}

/// Replace every `CONFIGURATION … END_CONFIGURATION` block in `source`
/// with an empty line so the rest of the file stays intact.
fn strip_inline_configuration(source: &str) -> String {
    let lower = source.to_ascii_lowercase();
    let mut out = String::with_capacity(source.len());
    let mut cursor = 0usize;
    while let Some(start_rel) = lower[cursor..].find("configuration ") {
        let abs = cursor + start_rel;
        out.push_str(&source[cursor..abs]);
        let Some(end_rel) = lower[abs..].find("end_configuration") else {
            // No matching close — fall back to copying the rest verbatim.
            out.push_str(&source[abs..]);
            cursor = source.len();
            break;
        };
        let abs_end = abs + end_rel + "end_configuration".len();
        cursor = abs_end;
        // Skip a trailing newline so we don't leave a blank line behind.
        if source.as_bytes().get(cursor).copied() == Some(b'\n') {
            cursor += 1;
        }
    }
    out.push_str(&source[cursor..]);
    out
}

/// Pull TASK and PROGRAM-instance declarations out of a single
/// CONFIGURATION block. Tolerates the formatting our own template
/// emits; not a full IEC parser — anything fancy lands in `tasks.toml`
/// as a no-op that the user can fix by hand.
fn parse_configuration_block(block: &str) -> InlineConfig {
    let mut tasks = Vec::new();
    let mut programs = Vec::new();
    for raw_line in block.lines() {
        let line = raw_line.trim();
        let lower = line.to_ascii_lowercase();
        if let Some(rest) = lower.strip_prefix("task ") {
            // TASK <name>(INTERVAL := T#<spec>, PRIORITY := <num>);
            if let Some((name, args)) = split_paren(rest) {
                let interval_ms = parse_interval_ms(&args).unwrap_or(100);
                let priority = parse_priority(&args).unwrap_or(1);
                tasks.push(Task {
                    name: name.trim().to_string(),
                    interval_ms,
                    priority,
                });
            }
        } else if let Some(rest) = lower.strip_prefix("program ") {
            // PROGRAM <instance> WITH <task> : <program_type>;
            // Split by " with " then by " : ", case-insensitive.
            if let Some(with_idx) = rest.find(" with ") {
                let instance = rest[..with_idx].trim().to_string();
                let after = &rest[with_idx + 6..];
                if let Some(colon_idx) = after.find(':') {
                    let task = after[..colon_idx].trim().to_string();
                    let prog = after[colon_idx + 1..]
                        .trim()
                        .trim_end_matches(';')
                        .trim()
                        .to_string();
                    programs.push(ProgramInstance {
                        instance,
                        program: prog,
                        task,
                    });
                }
            }
        }
    }
    InlineConfig { tasks, programs }
}

fn split_paren(s: &str) -> Option<(String, String)> {
    let open = s.find('(')?;
    let close = s.rfind(')')?;
    if close < open {
        return None;
    }
    Some((s[..open].to_string(), s[open + 1..close].to_string()))
}

fn parse_interval_ms(args: &str) -> Option<u32> {
    // INTERVAL := T#<spec>
    let lower = args.to_ascii_lowercase();
    let idx = lower.find("interval")?;
    let after = &args[idx..];
    let assign = after.find(":=")?;
    let value = after[assign + 2..]
        .trim_start()
        .split([',', ')'])
        .next()?
        .trim();
    // Accept T#100ms, T#1s, T#1m, LTIME#…, etc.
    let stripped = value
        .trim_start_matches('T')
        .trim_start_matches('t')
        .trim_start_matches("LTIME")
        .trim_start_matches("ltime")
        .trim_start_matches('#');
    parse_time_to_ms(stripped)
}

fn parse_time_to_ms(spec: &str) -> Option<u32> {
    // Very tolerant: pull <num><unit> tokens and sum them up. Units we
    // recognise: ms, s, m, h. Anything unknown is ignored (0).
    let mut total: u32 = 0;
    let mut num = String::new();
    let mut unit = String::new();
    let mut chars = spec.chars().peekable();
    while let Some(c) = chars.next() {
        if c.is_ascii_digit() || c == '_' {
            if c.is_ascii_digit() {
                num.push(c);
            }
        } else if c.is_alphabetic() {
            unit.push(c.to_ascii_lowercase());
            while let Some(&p) = chars.peek() {
                if p.is_alphabetic() {
                    unit.push(p.to_ascii_lowercase());
                    chars.next();
                } else {
                    break;
                }
            }
            let n: u32 = num.parse().ok()?;
            let factor: u32 = match unit.as_str() {
                "ms" => 1,
                "s" => 1_000,
                "m" => 60_000,
                "h" => 3_600_000,
                _ => 0,
            };
            total = total.saturating_add(n.saturating_mul(factor));
            num.clear();
            unit.clear();
        }
    }
    if total > 0 { Some(total) } else { None }
}

fn parse_priority(args: &str) -> Option<i32> {
    let lower = args.to_ascii_lowercase();
    let idx = lower.find("priority")?;
    let after = &args[idx..];
    let assign = after.find(":=")?;
    let value = after[assign + 2..]
        .trim_start()
        .split([',', ')'])
        .next()?
        .trim();
    value.parse().ok()
}

/// Validate a project-relative path. Accepts forward-slash separated
/// segments (e.g. `pid_loops/temperature_pid`). Rejects empty segments,
/// dot-prefixed segments (no hidden files, no `..` traversal), and
/// platform-special characters that confuse cross-platform path joining.
fn validate_path(name: &str) -> Result<(), StoreError> {
    if name.is_empty() {
        return Err(StoreError::InvalidName(name.into()));
    }
    for segment in name.split('/') {
        if segment.is_empty()
            || segment.starts_with('.')
            || segment.contains('\\')
            || segment.contains(':')
        {
            return Err(StoreError::InvalidName(name.into()));
        }
    }
    Ok(())
}

/// Last segment of a slash-separated path; the IEC POU identifier name.
fn leaf_segment(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

/// Recursively walk pou files under `root`, recognising both `.st`
/// and `.ld.json` extensions. The slug returned strips the extension(s)
/// so callers can pass it back through `pou_file_path` / `read_pou_*`.
fn walk_pou_files(
    root: &Path,
    prefix: &str,
    cb: &mut dyn FnMut(&str, PouLanguage, String) -> Result<(), StoreError>,
) -> Result<(), StoreError> {
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let path = entry.path();
        let ftype = entry.file_type()?;
        let name_os = entry.file_name();
        let name = match name_os.to_str() {
            Some(s) => s,
            None => continue,
        };
        if name.starts_with('.') {
            continue;
        }
        if ftype.is_dir() {
            let next_prefix = if prefix.is_empty() {
                name.to_string()
            } else {
                format!("{prefix}/{name}")
            };
            walk_pou_files(&path, &next_prefix, cb)?;
            continue;
        }
        if !ftype.is_file() {
            continue;
        }
        // Match the longest known suffix first so `.ld.json` doesn't
        // get classified as a plain `.json`.
        let (slug_stem, lang) = if let Some(s) = name.strip_suffix(".ld.json") {
            (s, PouLanguage::Ld)
        } else if let Some(s) = name.strip_suffix(".st") {
            (s, PouLanguage::St)
        } else {
            continue;
        };
        let slug = if prefix.is_empty() {
            slug_stem.to_string()
        } else {
            format!("{prefix}/{slug_stem}")
        };
        let contents = fs::read_to_string(&path)?;
        cb(&slug, lang, contents)?;
    }
    Ok(())
}

/// Recursively walk every file under `root` whose extension matches `ext`,
/// invoking `cb(rel_path_without_ext, contents)` for each.
fn walk_files(
    root: &Path,
    prefix: &str,
    ext: &str,
    cb: &mut dyn FnMut(&str, String) -> Result<(), StoreError>,
) -> Result<(), StoreError> {
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let path = entry.path();
        let ftype = entry.file_type()?;
        let name_os = entry.file_name();
        let name = match name_os.to_str() {
            Some(s) => s,
            None => continue,
        };
        if name.starts_with('.') {
            continue;
        }
        if ftype.is_dir() {
            let next_prefix = if prefix.is_empty() {
                name.to_string()
            } else {
                format!("{prefix}/{name}")
            };
            walk_files(&path, &next_prefix, ext, cb)?;
        } else if ftype.is_file()
            && path.extension().and_then(|s| s.to_str()) == Some(ext)
        {
            let stem = match path.file_stem().and_then(|s| s.to_str()) {
                Some(s) => s,
                None => continue,
            };
            let rel = if prefix.is_empty() {
                stem.to_string()
            } else {
                format!("{prefix}/{stem}")
            };
            let contents = fs::read_to_string(&path)?;
            cb(&rel, contents)?;
        }
    }
    Ok(())
}

/// Recursively walk every directory under `root`, emitting their forward-
/// slash relative paths via `cb`.
fn walk_dirs(
    root: &Path,
    prefix: &str,
    cb: &mut dyn FnMut(&str) -> Result<(), StoreError>,
) -> Result<(), StoreError> {
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let path = entry.path();
        let ftype = entry.file_type()?;
        let name_os = entry.file_name();
        let name = match name_os.to_str() {
            Some(s) => s,
            None => continue,
        };
        if name.starts_with('.') || !ftype.is_dir() {
            continue;
        }
        let rel = if prefix.is_empty() {
            name.to_string()
        } else {
            format!("{prefix}/{name}")
        };
        cb(&rel)?;
        walk_dirs(&path, &rel, cb)?;
    }
    Ok(())
}

/// Source template for a new single-POU file. We bake the IEC POU
/// header + an empty VAR block; the runtime's compile path enables
/// `allow_empty_var_blocks` so this is valid input.
fn template_for(name: &str, type_: PouType) -> String {
    match type_ {
        PouType::Program => format!(
            "PROGRAM {name}\n    VAR\n    END_VAR\n\n    \
             (* Add your program logic here. Bind this PROGRAM to a task\n       \
             in the Tasks pane to actually run it. *)\n\nEND_PROGRAM\n"
        ),
        PouType::FunctionBlock => format!(
            "FUNCTION_BLOCK {name}\n    VAR_INPUT\n    END_VAR\n    \
             VAR_OUTPUT\n    END_VAR\n    VAR\n    END_VAR\n\n    \
             (* Add your FB logic here *)\n\nEND_FUNCTION_BLOCK\n"
        ),
        PouType::Function => format!(
            "FUNCTION {name} : INT\n    VAR_INPUT\n    END_VAR\n    \
             VAR\n    END_VAR\n\n    \
             (* Return a value via the function name: {name} := ... *)\n\
             \nEND_FUNCTION\n"
        ),
    }
}

/// Source template for a new LD POU. Returns a JSON literal that
/// matches the [`crate::ld::LdProgram`] schema — one always-passing
/// rung that asserts the coil, so the new file compiles end-to-end
/// without further authoring. The user replaces the rung with their
/// actual logic.
fn template_for_ld(name: &str, type_: PouType) -> String {
    use crate::ld::{
        LdCoil, LdCoilKind, LdNode, LdPouType, LdProgram, LdRung, LdVarSection, LdVariable,
    };
    let pou_type = match type_ {
        PouType::Program => LdPouType::Program,
        PouType::FunctionBlock => LdPouType::FunctionBlock,
        // Functions in LD are uncommon and not exposed in the UI yet
        // (see `LdPouType` docs). Fall back to Program so the seeded
        // file is at least valid; the caller is responsible for
        // gating Function creation in LD.
        PouType::Function => LdPouType::Program,
    };
    let prog = LdProgram {
        name: name.into(),
        pou_type,
        variables: vec![LdVariable {
            name: "out".into(),
            type_name: "BOOL".into(),
            section: LdVarSection::Internal,
            init: None,
        }],
        rungs: vec![LdRung {
            id: "r0".into(),
            label: Some("Replace this rung with your logic".into()),
            logic: LdNode::Const { value: true },
            coils: vec![LdCoil {
                var: "out".into(),
                kind: LdCoilKind::Standard,
            }],
        }],
    };
    serde_json::to_string_pretty(&prog).expect("LD template serialises")
}

fn default_config_for(protocol: Protocol) -> ProtocolConfig {
    match protocol {
        Protocol::Modbus => ProtocolConfig::Modbus(ModbusConfig {
            host: "127.0.0.1".into(),
            port: 502,
            slave_id: 1,
            poll_interval_ms: 100,
            channels: vec![],
        }),
        Protocol::Ethercat => ProtocolConfig::Ethercat(crate::types::EthercatConfig {
            nic: default_ethercat_nic().into(),
            cycle_us: 1_000,
            slaves: vec![],
            channels: vec![],
        }),
    }
}
