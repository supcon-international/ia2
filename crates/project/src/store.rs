use std::fs;
use std::path::{Path, PathBuf};

use crate::errors::StoreError;
use crate::types::{
    Application, ApplicationKind, ApplicationSummary, Device, DeviceSummary, IoMap, ModbusConfig,
    ProjectManifest, ProjectTree, Protocol, ProtocolConfig,
};

const SAMPLE_MAIN_ST: &str = include_str!("../templates/main.st");

pub struct ProjectStore {
    root: PathBuf,
    manifest: ProjectManifest,
}

impl ProjectStore {
    /// Open an existing project at `root`. Fails if `project.toml` is missing.
    pub fn open(root: PathBuf) -> Result<Self, StoreError> {
        let manifest_path = root.join("project.toml");
        if !manifest_path.exists() {
            return Err(StoreError::NotFound(root.display().to_string()));
        }
        let text = fs::read_to_string(&manifest_path)?;
        let manifest: ProjectManifest = toml::from_str(&text)?;
        Ok(Self { root, manifest })
    }

    /// Create a fresh project at `root`. Fails if `root` already contains a
    /// `project.toml`. Seeds the directory with a sample PROGRAM and an
    /// empty iomap.
    pub fn create(root: PathBuf, name: &str) -> Result<Self, StoreError> {
        validate_name(name)?;
        let manifest_path = root.join("project.toml");
        if manifest_path.exists() {
            return Err(StoreError::AlreadyExists(root.display().to_string()));
        }

        fs::create_dir_all(&root)?;
        fs::create_dir_all(root.join("applications"))?;
        fs::create_dir_all(root.join("devices"))?;

        let manifest = ProjectManifest {
            name: name.to_string(),
            version: "0.1".into(),
        };
        fs::write(&manifest_path, toml::to_string_pretty(&manifest)?)?;
        fs::write(
            root.join("iomap.toml"),
            toml::to_string_pretty(&IoMap::default())?,
        )?;
        fs::write(root.join("applications/main.st"), SAMPLE_MAIN_ST)?;

        Ok(Self { root, manifest })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn name(&self) -> &str {
        &self.manifest.name
    }

    /// Full snapshot for the frontend project tree.
    pub fn tree(&self) -> Result<ProjectTree, StoreError> {
        let apps = self.list_applications()?;
        let devices = self.list_devices()?;
        let iomap = self.read_iomap()?;
        Ok(ProjectTree {
            name: self.manifest.name.clone(),
            path: self.root.display().to_string(),
            applications: apps
                .into_iter()
                .map(|a| ApplicationSummary {
                    name: a.name,
                    kind: a.kind,
                })
                .collect(),
            devices: devices
                .into_iter()
                .map(|d| DeviceSummary {
                    name: d.name,
                    protocol: d.config.protocol(),
                })
                .collect(),
            iomap,
        })
    }

    // ---------------- Applications (POUs) ----------------

    pub fn list_applications(&self) -> Result<Vec<Application>, StoreError> {
        let dir = self.root.join("applications");
        if !dir.exists() {
            return Ok(vec![]);
        }
        let mut out = Vec::new();
        for entry in fs::read_dir(&dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) != Some("st") {
                continue;
            }
            let source = fs::read_to_string(&path)?;
            let name = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_string();
            out.push(Application {
                kind: detect_kind(&source),
                name,
                source,
            });
        }
        out.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(out)
    }

    pub fn read_application(&self, name: &str) -> Result<Application, StoreError> {
        validate_name(name)?;
        let path = self.root.join("applications").join(format!("{name}.st"));
        if !path.exists() {
            return Err(StoreError::AppNotFound(name.into()));
        }
        let source = fs::read_to_string(&path)?;
        Ok(Application {
            name: name.into(),
            kind: detect_kind(&source),
            source,
        })
    }

    pub fn write_application(&self, name: &str, source: &str) -> Result<(), StoreError> {
        validate_name(name)?;
        let dir = self.root.join("applications");
        fs::create_dir_all(&dir)?;
        fs::write(dir.join(format!("{name}.st")), source)?;
        Ok(())
    }

    pub fn create_application(
        &self,
        name: &str,
        kind: ApplicationKind,
    ) -> Result<Application, StoreError> {
        validate_name(name)?;
        let path = self.root.join("applications").join(format!("{name}.st"));
        if path.exists() {
            return Err(StoreError::AlreadyExists(name.into()));
        }
        let source = template_for(name, kind);
        fs::create_dir_all(path.parent().unwrap())?;
        fs::write(&path, &source)?;
        Ok(Application {
            name: name.into(),
            kind,
            source,
        })
    }

    pub fn delete_application(&self, name: &str) -> Result<(), StoreError> {
        validate_name(name)?;
        let path = self.root.join("applications").join(format!("{name}.st"));
        if !path.exists() {
            return Err(StoreError::AppNotFound(name.into()));
        }
        fs::remove_file(path)?;
        Ok(())
    }

    // ---------------- Devices ----------------

    pub fn list_devices(&self) -> Result<Vec<Device>, StoreError> {
        let dir = self.root.join("devices");
        if !dir.exists() {
            return Ok(vec![]);
        }
        let mut out = Vec::new();
        for entry in fs::read_dir(&dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) != Some("toml") {
                continue;
            }
            let text = fs::read_to_string(&path)?;
            let device: Device = toml::from_str(&text)?;
            out.push(device);
        }
        out.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(out)
    }

    pub fn read_device(&self, name: &str) -> Result<Device, StoreError> {
        validate_name(name)?;
        let path = self.root.join("devices").join(format!("{name}.toml"));
        if !path.exists() {
            return Err(StoreError::DeviceNotFound(name.into()));
        }
        let text = fs::read_to_string(&path)?;
        Ok(toml::from_str(&text)?)
    }

    pub fn write_device(&self, device: &Device) -> Result<(), StoreError> {
        validate_name(&device.name)?;
        let dir = self.root.join("devices");
        fs::create_dir_all(&dir)?;
        let path = dir.join(format!("{}.toml", device.name));
        fs::write(path, toml::to_string_pretty(device)?)?;
        Ok(())
    }

    pub fn create_device(&self, name: &str, protocol: Protocol) -> Result<Device, StoreError> {
        validate_name(name)?;
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
        validate_name(name)?;
        let path = self.root.join("devices").join(format!("{name}.toml"));
        if !path.exists() {
            return Err(StoreError::DeviceNotFound(name.into()));
        }
        fs::remove_file(path)?;
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
}

fn validate_name(name: &str) -> Result<(), StoreError> {
    if name.is_empty() || name.contains('/') || name.contains('\\') || name.starts_with('.') {
        return Err(StoreError::InvalidName(name.into()));
    }
    Ok(())
}

fn detect_kind(source: &str) -> ApplicationKind {
    for line in source.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("FUNCTION_BLOCK") {
            return ApplicationKind::FunctionBlock;
        }
        if trimmed.starts_with("PROGRAM") {
            return ApplicationKind::Program;
        }
    }
    ApplicationKind::Program
}

fn template_for(name: &str, kind: ApplicationKind) -> String {
    match kind {
        ApplicationKind::Program => format!(
            "PROGRAM {name}\n    VAR\n    END_VAR\n\n    \
             (* Add your program logic here *)\n\nEND_PROGRAM\n\n\
             CONFIGURATION config\n    RESOURCE plc_res ON PLC\n        \
             TASK plc_task(INTERVAL := T#100ms, PRIORITY := 1);\n        \
             PROGRAM plc_task_instance WITH plc_task : {name};\n    \
             END_RESOURCE\nEND_CONFIGURATION\n"
        ),
        ApplicationKind::FunctionBlock => format!(
            "FUNCTION_BLOCK {name}\n    VAR_INPUT\n    END_VAR\n    \
             VAR_OUTPUT\n    END_VAR\n    VAR\n    END_VAR\n\n    \
             (* Add your FB logic here *)\n\nEND_FUNCTION_BLOCK\n"
        ),
    }
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
            nic: "eth0".into(),
            slaves: vec![],
        }),
    }
}
