use super::{CliTarget, VaultScope};
use crate::vault::VaultRoots;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    fmt, fs,
    path::{Path, PathBuf},
};

pub const VAULT_SETTINGS_FILE_NAME: &str = "settings.json";

#[derive(Debug, Clone, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct VaultSettings {
    pub default_target_cli: Option<CliTarget>,
    pub default_model: Option<String>,
    pub auto_classification_enabled: Option<bool>,
    pub scan_roots: Option<Vec<ScanRootConfig>>,
    pub skill_scan_roots: Option<Vec<ScanRootConfig>>,
    pub claude_session_roots: Option<Vec<ScanRootConfig>>,
    pub codex_session_roots: Option<Vec<ScanRootConfig>>,
}

#[derive(Debug, Clone, Deserialize, Eq, PartialEq, Serialize)]
#[serde(untagged)]
pub enum ScanRootConfig {
    Path(PathBuf),
    Detailed {
        path: PathBuf,
        scope: Option<VaultScope>,
    },
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ConfiguredScanRoot {
    pub path: PathBuf,
    pub scope: VaultScope,
    pub source: PathBuf,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ResolvedVaultSettings {
    pub settings: VaultSettings,
    pub sources: Vec<VaultSettingsSource>,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct VaultSettingsSource {
    pub path: PathBuf,
    pub scope: VaultScope,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub enum SessionLogRootSource {
    KnownDefault,
    VaultSettings { path: PathBuf, scope: VaultScope },
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ResolvedSessionLogRoot {
    pub path: PathBuf,
    pub source: SessionLogRootSource,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub enum VaultSettingsError {
    Io(String),
    Parse(String),
    InvalidScanRoot(String),
}

impl fmt::Display for VaultSettingsError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(message) => write!(formatter, "{message}"),
            Self::Parse(message) => write!(formatter, "{message}"),
            Self::InvalidScanRoot(message) => write!(formatter, "{message}"),
        }
    }
}

impl std::error::Error for VaultSettingsError {}

pub fn vault_settings_path(root: impl AsRef<Path>) -> PathBuf {
    root.as_ref().join(VAULT_SETTINGS_FILE_NAME)
}

pub fn load_vault_settings_overlay(
    roots: &VaultRoots,
) -> Result<ResolvedVaultSettings, VaultSettingsError> {
    let mut resolved = ResolvedVaultSettings {
        settings: VaultSettings::default(),
        sources: Vec::new(),
    };

    merge_settings_file(
        vault_settings_path(&roots.global_root),
        VaultScope::Global,
        &mut resolved,
    )?;

    if let Some(local_root) = &roots.local_root {
        merge_settings_file(
            vault_settings_path(local_root),
            VaultScope::Local,
            &mut resolved,
        )?;
    }

    Ok(resolved)
}

pub fn load_configured_scan_roots(
    roots: &VaultRoots,
    working_dir: &Path,
) -> Result<Vec<ConfiguredScanRoot>, VaultSettingsError> {
    load_configured_roots_from_field(roots, working_dir, ScanRootField::Context)
}

pub fn load_configured_skill_scan_roots(
    roots: &VaultRoots,
    working_dir: &Path,
) -> Result<Vec<ConfiguredScanRoot>, VaultSettingsError> {
    load_configured_roots_from_field(roots, working_dir, ScanRootField::Skill)
}

pub fn resolve_claude_session_log_roots(
    roots: &VaultRoots,
    working_dir: &Path,
    home_dir: &Path,
) -> Result<Vec<ResolvedSessionLogRoot>, VaultSettingsError> {
    resolve_session_log_roots(
        roots,
        working_dir,
        SessionLogRootField::Claude,
        home_dir.join(".claude").join("projects"),
    )
}

pub fn resolve_codex_session_log_roots(
    roots: &VaultRoots,
    working_dir: &Path,
    home_dir: &Path,
) -> Result<Vec<ResolvedSessionLogRoot>, VaultSettingsError> {
    resolve_session_log_roots(
        roots,
        working_dir,
        SessionLogRootField::Codex,
        home_dir.join(".codex").join("sessions"),
    )
}

fn resolve_session_log_roots(
    roots: &VaultRoots,
    working_dir: &Path,
    field: SessionLogRootField,
    default_path: PathBuf,
) -> Result<Vec<ResolvedSessionLogRoot>, VaultSettingsError> {
    let mut session_roots = Vec::new();
    let mut seen = HashSet::new();

    push_existing_session_log_root(
        default_path,
        field,
        SessionLogRootSource::KnownDefault,
        &mut seen,
        &mut session_roots,
    )?;

    collect_session_roots_from_settings_file(
        vault_settings_path(&roots.global_root),
        VaultScope::Global,
        &roots.global_root,
        working_dir,
        field,
        &mut seen,
        &mut session_roots,
    )?;

    if let Some(local_root) = &roots.local_root {
        collect_session_roots_from_settings_file(
            vault_settings_path(local_root),
            VaultScope::Local,
            local_root,
            working_dir,
            field,
            &mut seen,
            &mut session_roots,
        )?;
    }

    Ok(session_roots)
}

fn load_configured_roots_from_field(
    roots: &VaultRoots,
    working_dir: &Path,
    field: ScanRootField,
) -> Result<Vec<ConfiguredScanRoot>, VaultSettingsError> {
    let mut scan_roots = Vec::new();
    let mut seen = HashSet::new();

    collect_scan_roots_from_settings_file(
        vault_settings_path(&roots.global_root),
        VaultScope::Global,
        &roots.global_root,
        working_dir,
        field,
        &mut seen,
        &mut scan_roots,
    )?;

    if let Some(local_root) = &roots.local_root {
        collect_scan_roots_from_settings_file(
            vault_settings_path(local_root),
            VaultScope::Local,
            local_root,
            working_dir,
            field,
            &mut seen,
            &mut scan_roots,
        )?;
    }

    Ok(scan_roots)
}

#[derive(Debug, Clone, Copy)]
enum ScanRootField {
    Context,
    Skill,
}

impl ScanRootField {
    fn label(self) -> &'static str {
        match self {
            Self::Context => "configured scan root",
            Self::Skill => "configured skill scan root",
        }
    }
}

fn merge_settings_file(
    path: PathBuf,
    scope: VaultScope,
    resolved: &mut ResolvedVaultSettings,
) -> Result<(), VaultSettingsError> {
    if !path.exists() {
        return Ok(());
    }

    let content = fs::read_to_string(&path).map_err(|error| {
        VaultSettingsError::Io(format!(
            "failed to read vault settings file {}: {error}",
            path.display()
        ))
    })?;
    let settings: VaultSettings = serde_json::from_str(&content).map_err(|error| {
        VaultSettingsError::Parse(format!(
            "failed to parse vault settings file {} as JSON: {error}",
            path.display()
        ))
    })?;

    merge_settings(&mut resolved.settings, settings);
    resolved.sources.push(VaultSettingsSource { path, scope });

    Ok(())
}

fn merge_settings(base: &mut VaultSettings, overlay: VaultSettings) {
    if overlay.default_target_cli.is_some() {
        base.default_target_cli = overlay.default_target_cli;
    }
    if overlay.default_model.is_some() {
        base.default_model = overlay.default_model;
    }
    if overlay.auto_classification_enabled.is_some() {
        base.auto_classification_enabled = overlay.auto_classification_enabled;
    }
    if overlay.scan_roots.is_some() {
        base.scan_roots = overlay.scan_roots;
    }
    if overlay.skill_scan_roots.is_some() {
        base.skill_scan_roots = overlay.skill_scan_roots;
    }
    if overlay.claude_session_roots.is_some() {
        base.claude_session_roots = overlay.claude_session_roots;
    }
    if overlay.codex_session_roots.is_some() {
        base.codex_session_roots = overlay.codex_session_roots;
    }
}

fn collect_scan_roots_from_settings_file(
    path: PathBuf,
    default_scope: VaultScope,
    vault_root: &Path,
    working_dir: &Path,
    field: ScanRootField,
    seen: &mut HashSet<PathBuf>,
    scan_roots: &mut Vec<ConfiguredScanRoot>,
) -> Result<(), VaultSettingsError> {
    if !path.exists() {
        return Ok(());
    }

    let content = fs::read_to_string(&path).map_err(|error| {
        VaultSettingsError::Io(format!(
            "failed to read vault settings file {}: {error}",
            path.display()
        ))
    })?;
    let settings: VaultSettings = serde_json::from_str(&content).map_err(|error| {
        VaultSettingsError::Parse(format!(
            "failed to parse vault settings file {} as JSON: {error}",
            path.display()
        ))
    })?;

    for configured in configured_roots(settings, field) {
        let root_label = field.label();
        let scope = configured.scope().unwrap_or(default_scope);
        let configured_path = configured.path();
        let base_dir = scan_root_base_dir(vault_root, working_dir);
        let expanded_path = expand_scan_root_path(configured_path, &base_dir);
        let canonical_path = expanded_path.canonicalize().map_err(|error| {
            VaultSettingsError::InvalidScanRoot(format!(
                "{root_label} {} from {} is not accessible: {error}",
                expanded_path.display(),
                path.display()
            ))
        })?;
        let metadata = fs::metadata(&canonical_path).map_err(|error| {
            VaultSettingsError::InvalidScanRoot(format!(
                "{root_label} {} from {} cannot be inspected: {error}",
                canonical_path.display(),
                path.display()
            ))
        })?;

        if !metadata.is_dir() {
            return Err(VaultSettingsError::InvalidScanRoot(format!(
                "{root_label} {} from {} is not a directory",
                canonical_path.display(),
                path.display()
            )));
        }

        if seen.insert(canonical_path.clone()) {
            scan_roots.push(ConfiguredScanRoot {
                path: canonical_path,
                scope,
                source: path.clone(),
            });
        }
    }

    Ok(())
}

fn configured_roots(settings: VaultSettings, field: ScanRootField) -> Vec<ScanRootConfig> {
    match field {
        ScanRootField::Context => settings.scan_roots.unwrap_or_default(),
        ScanRootField::Skill => settings.skill_scan_roots.unwrap_or_default(),
    }
}

impl ScanRootConfig {
    fn path(&self) -> &Path {
        match self {
            Self::Path(path) => path,
            Self::Detailed { path, .. } => path,
        }
    }

    fn scope(&self) -> Option<VaultScope> {
        match self {
            Self::Path(_) => None,
            Self::Detailed { scope, .. } => *scope,
        }
    }
}

fn scan_root_base_dir(vault_root: &Path, working_dir: &Path) -> PathBuf {
    vault_root
        .parent()
        .and_then(Path::parent)
        .map(Path::to_path_buf)
        .unwrap_or_else(|| working_dir.to_path_buf())
}

fn expand_scan_root_path(path: &Path, base_dir: &Path) -> PathBuf {
    if path.is_absolute() {
        return path.to_path_buf();
    }

    if let Some(stripped) = path.to_str().and_then(|value| value.strip_prefix("~/")) {
        if let Some(home) = std::env::var_os("HOME") {
            return PathBuf::from(home).join(stripped);
        }
    }

    base_dir.join(path)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SessionLogRootField {
    Claude,
    Codex,
}

impl SessionLogRootField {
    fn label(self) -> &'static str {
        match self {
            Self::Claude => "Claude session log root",
            Self::Codex => "Codex session log root",
        }
    }
}

fn collect_session_roots_from_settings_file(
    path: PathBuf,
    scope: VaultScope,
    vault_root: &Path,
    working_dir: &Path,
    field: SessionLogRootField,
    seen: &mut HashSet<PathBuf>,
    session_roots: &mut Vec<ResolvedSessionLogRoot>,
) -> Result<(), VaultSettingsError> {
    if !path.exists() {
        return Ok(());
    }

    let content = fs::read_to_string(&path).map_err(|error| {
        VaultSettingsError::Io(format!(
            "failed to read vault settings file {}: {error}",
            path.display()
        ))
    })?;
    let settings: VaultSettings = serde_json::from_str(&content).map_err(|error| {
        VaultSettingsError::Parse(format!(
            "failed to parse vault settings file {} as JSON: {error}",
            path.display()
        ))
    })?;

    let configured_roots = match field {
        SessionLogRootField::Claude => settings.claude_session_roots,
        SessionLogRootField::Codex => settings.codex_session_roots,
    };
    let Some(configured_roots) = configured_roots else {
        return Ok(());
    };

    let base_dir = scan_root_base_dir(vault_root, working_dir);
    for configured_root in configured_roots {
        let expanded_path = expand_scan_root_path(configured_root.path(), &base_dir);
        push_existing_session_log_root(
            expanded_path,
            field,
            SessionLogRootSource::VaultSettings {
                path: path.clone(),
                scope,
            },
            seen,
            session_roots,
        )?;
    }

    Ok(())
}

fn push_existing_session_log_root(
    path: PathBuf,
    field: SessionLogRootField,
    source: SessionLogRootSource,
    seen: &mut HashSet<PathBuf>,
    session_roots: &mut Vec<ResolvedSessionLogRoot>,
) -> Result<(), VaultSettingsError> {
    if !path.exists() {
        return Ok(());
    }

    let canonical_path = match path.canonicalize() {
        Ok(canonical_path) => canonical_path,
        Err(_) if field == SessionLogRootField::Codex => return Ok(()),
        Err(error) => {
            return Err(VaultSettingsError::InvalidScanRoot(format!(
                "{} {} is not accessible: {error}",
                field.label(),
                path.display()
            )));
        }
    };
    let metadata = match fs::metadata(&canonical_path) {
        Ok(metadata) => metadata,
        Err(_) if field == SessionLogRootField::Codex => return Ok(()),
        Err(error) => {
            return Err(VaultSettingsError::InvalidScanRoot(format!(
                "{} {} cannot be inspected: {error}",
                field.label(),
                canonical_path.display()
            )));
        }
    };

    if !metadata.is_dir() {
        if field == SessionLogRootField::Codex {
            return Ok(());
        }
        return Err(VaultSettingsError::InvalidScanRoot(format!(
            "{} {} is not a directory",
            field.label(),
            canonical_path.display()
        )));
    }

    if let Err(error) = fs::read_dir(&canonical_path) {
        if field == SessionLogRootField::Codex {
            return Ok(());
        }
        return Err(VaultSettingsError::InvalidScanRoot(format!(
            "{} {} is not readable: {error}",
            field.label(),
            canonical_path.display()
        )));
    }

    if seen.insert(canonical_path.clone()) {
        session_roots.push(ResolvedSessionLogRoot {
            path: canonical_path,
            source,
        });
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        load_configured_scan_roots, load_configured_skill_scan_roots, load_vault_settings_overlay,
        resolve_claude_session_log_roots, resolve_codex_session_log_roots, vault_settings_path,
        SessionLogRootSource, VAULT_SETTINGS_FILE_NAME,
    };
    use crate::{CliTarget, VaultRoots, VaultScope};
    use std::fs;
    use uuid::Uuid;

    #[test]
    fn local_vault_settings_override_conflicting_global_vault_settings() {
        let base = std::env::temp_dir().join(format!("ctx-settings-overlay-{}", Uuid::new_v4()));
        let roots = VaultRoots {
            global_root: base.join("home").join(".ctx").join("vault"),
            local_root: Some(base.join("project").join(".ctx").join("vault")),
        };
        fs::create_dir_all(&roots.global_root).expect("global vault root should be created");
        fs::create_dir_all(roots.local_root.as_ref().unwrap())
            .expect("local vault root should be created");
        fs::write(
            vault_settings_path(&roots.global_root),
            r#"{
  "default_target_cli": "claude",
  "default_model": "global-sonnet",
  "auto_classification_enabled": false
}"#,
        )
        .expect("global settings should be writable");
        fs::write(
            vault_settings_path(roots.local_root.as_ref().unwrap()),
            r#"{
  "default_target_cli": "codex",
  "default_model": "local-gpt",
  "auto_classification_enabled": true
}"#,
        )
        .expect("local settings should be writable");

        let resolved =
            load_vault_settings_overlay(&roots).expect("vault settings overlay should resolve");

        assert_eq!(resolved.settings.default_target_cli, Some(CliTarget::Codex));
        assert_eq!(
            resolved.settings.default_model.as_deref(),
            Some("local-gpt")
        );
        assert_eq!(resolved.settings.auto_classification_enabled, Some(true));
        assert_eq!(resolved.sources.len(), 2);
        assert_eq!(resolved.sources[0].scope, VaultScope::Global);
        assert_eq!(resolved.sources[1].scope, VaultScope::Local);
        assert!(resolved.sources[0].path.ends_with(VAULT_SETTINGS_FILE_NAME));
        assert!(resolved.sources[1].path.ends_with(VAULT_SETTINGS_FILE_NAME));

        fs::remove_dir_all(base).ok();
    }

    #[test]
    fn local_vault_settings_preserve_unmatched_global_defaults() {
        let base = std::env::temp_dir().join(format!("ctx-settings-merge-{}", Uuid::new_v4()));
        let roots = VaultRoots {
            global_root: base.join("home").join(".ctx").join("vault"),
            local_root: Some(base.join("project").join(".ctx").join("vault")),
        };
        fs::create_dir_all(&roots.global_root).expect("global vault root should be created");
        fs::create_dir_all(roots.local_root.as_ref().unwrap())
            .expect("local vault root should be created");
        fs::write(
            vault_settings_path(&roots.global_root),
            r#"{"default_target_cli":"claude","default_model":"global-sonnet"}"#,
        )
        .expect("global settings should be writable");
        fs::write(
            vault_settings_path(roots.local_root.as_ref().unwrap()),
            r#"{"default_model":"local-gpt"}"#,
        )
        .expect("local settings should be writable");

        let resolved =
            load_vault_settings_overlay(&roots).expect("vault settings overlay should resolve");

        assert_eq!(
            resolved.settings.default_target_cli,
            Some(CliTarget::Claude)
        );
        assert_eq!(
            resolved.settings.default_model.as_deref(),
            Some("local-gpt")
        );

        fs::remove_dir_all(base).ok();
    }

    #[test]
    fn missing_vault_settings_resolve_to_empty_defaults() {
        let base = std::env::temp_dir().join(format!("ctx-settings-empty-{}", Uuid::new_v4()));
        let roots = VaultRoots {
            global_root: base.join("home").join(".ctx").join("vault"),
            local_root: Some(base.join("project").join(".ctx").join("vault")),
        };

        let resolved =
            load_vault_settings_overlay(&roots).expect("missing settings should be ignored");

        assert_eq!(resolved.sources, Vec::new());
        assert_eq!(resolved.settings.default_target_cli, None);
        assert_eq!(resolved.settings.default_model, None);
        assert_eq!(resolved.settings.auto_classification_enabled, None);
    }

    #[test]
    fn loads_and_validates_configured_scan_roots_from_global_and_local_settings() {
        let base = std::env::temp_dir().join(format!("ctx-settings-scan-roots-{}", Uuid::new_v4()));
        let roots = VaultRoots {
            global_root: base.join("home").join(".ctx").join("vault"),
            local_root: Some(base.join("project").join(".ctx").join("vault")),
        };
        let global_scan_root = base.join("home").join("agent-contexts");
        let local_scan_root = base.join("project").join("project-contexts");
        fs::create_dir_all(&roots.global_root).expect("global vault root should be created");
        fs::create_dir_all(roots.local_root.as_ref().unwrap())
            .expect("local vault root should be created");
        fs::create_dir_all(&global_scan_root).expect("global scan root should be created");
        fs::create_dir_all(&local_scan_root).expect("local scan root should be created");
        fs::write(
            vault_settings_path(&roots.global_root),
            r#"{"scan_roots":["agent-contexts"]}"#,
        )
        .expect("global settings should be writable");
        fs::write(
            vault_settings_path(roots.local_root.as_ref().unwrap()),
            r#"{"scan_roots":[{"path":"project-contexts","scope":"local"}]}"#,
        )
        .expect("local settings should be writable");

        let scan_roots = load_configured_scan_roots(&roots, &base.join("project"))
            .expect("configured scan roots should load");

        assert_eq!(scan_roots.len(), 2);
        assert_eq!(
            scan_roots[0].path,
            global_scan_root
                .canonicalize()
                .expect("global scan root should canonicalize")
        );
        assert_eq!(scan_roots[0].scope, VaultScope::Global);
        assert_eq!(
            scan_roots[1].path,
            local_scan_root
                .canonicalize()
                .expect("local scan root should canonicalize")
        );
        assert_eq!(scan_roots[1].scope, VaultScope::Local);

        fs::remove_dir_all(base).ok();
    }

    #[test]
    fn loads_and_validates_configured_skill_scan_roots_from_settings() {
        let base =
            std::env::temp_dir().join(format!("ctx-settings-skill-scan-roots-{}", Uuid::new_v4()));
        let roots = VaultRoots {
            global_root: base.join("home").join(".ctx").join("vault"),
            local_root: Some(base.join("project").join(".ctx").join("vault")),
        };
        let global_skill_root = base.join("home").join("shared-skills");
        let local_skill_root = base.join("project").join("project-skills");
        fs::create_dir_all(&roots.global_root).expect("global vault root should be created");
        fs::create_dir_all(roots.local_root.as_ref().unwrap())
            .expect("local vault root should be created");
        fs::create_dir_all(&global_skill_root).expect("global skill root should be created");
        fs::create_dir_all(&local_skill_root).expect("local skill root should be created");
        fs::write(
            vault_settings_path(&roots.global_root),
            r#"{"skill_scan_roots":["shared-skills"]}"#,
        )
        .expect("global settings should be writable");
        fs::write(
            vault_settings_path(roots.local_root.as_ref().unwrap()),
            r#"{"skill_scan_roots":[{"path":"project-skills","scope":"local"}]}"#,
        )
        .expect("local settings should be writable");

        let scan_roots = load_configured_skill_scan_roots(&roots, &base.join("project"))
            .expect("configured skill scan roots should load");

        assert_eq!(scan_roots.len(), 2);
        assert_eq!(
            scan_roots[0].path,
            global_skill_root
                .canonicalize()
                .expect("global skill root should canonicalize")
        );
        assert_eq!(scan_roots[0].scope, VaultScope::Global);
        assert_eq!(
            scan_roots[1].path,
            local_skill_root
                .canonicalize()
                .expect("local skill root should canonicalize")
        );
        assert_eq!(scan_roots[1].scope, VaultScope::Local);

        fs::remove_dir_all(base).ok();
    }

    #[test]
    fn resolves_claude_session_log_roots_from_default_and_settings() {
        let base =
            std::env::temp_dir().join(format!("ctx-claude-session-roots-{}", Uuid::new_v4()));
        let roots = VaultRoots {
            global_root: base.join("home").join(".ctx").join("vault"),
            local_root: Some(base.join("project").join(".ctx").join("vault")),
        };
        let home_dir = base.join("home");
        let default_root = home_dir.join(".claude").join("projects");
        let global_root = base.join("home").join("extra-claude-sessions");
        let local_root = base.join("project").join("project-claude-sessions");

        fs::create_dir_all(&roots.global_root).expect("global vault root should be created");
        fs::create_dir_all(roots.local_root.as_ref().unwrap())
            .expect("local vault root should be created");
        fs::create_dir_all(&default_root).expect("default Claude root should be created");
        fs::create_dir_all(&global_root).expect("global Claude root should be created");
        fs::create_dir_all(&local_root).expect("local Claude root should be created");

        fs::write(
            vault_settings_path(&roots.global_root),
            r#"{"claude_session_roots":["extra-claude-sessions"]}"#,
        )
        .expect("global settings should be writable");
        fs::write(
            vault_settings_path(roots.local_root.as_ref().unwrap()),
            r#"{"claude_session_roots":[{"path":"project-claude-sessions"}]}"#,
        )
        .expect("local settings should be writable");

        let session_roots =
            resolve_claude_session_log_roots(&roots, &base.join("project"), &home_dir)
                .expect("Claude session log roots should resolve");

        assert_eq!(session_roots.len(), 3);
        assert_eq!(
            session_roots[0].path,
            default_root
                .canonicalize()
                .expect("default root should canonicalize")
        );
        assert_eq!(session_roots[0].source, SessionLogRootSource::KnownDefault);
        assert_eq!(
            session_roots[1].path,
            global_root
                .canonicalize()
                .expect("global root should canonicalize")
        );
        assert_eq!(
            session_roots[1].source,
            SessionLogRootSource::VaultSettings {
                path: vault_settings_path(&roots.global_root),
                scope: VaultScope::Global,
            }
        );
        assert_eq!(
            session_roots[2].path,
            local_root
                .canonicalize()
                .expect("local root should canonicalize")
        );

        fs::remove_dir_all(base).ok();
    }

    #[test]
    fn resolves_claude_session_log_roots_deduplicating_missing_and_overlapping_paths() {
        let base = std::env::temp_dir().join(format!(
            "ctx-claude-session-roots-dedupe-{}",
            Uuid::new_v4()
        ));
        let roots = VaultRoots {
            global_root: base.join("home").join(".ctx").join("vault"),
            local_root: None,
        };
        let home_dir = base.join("home");
        let default_root = home_dir.join(".claude").join("projects");

        fs::create_dir_all(&roots.global_root).expect("global vault root should be created");
        fs::create_dir_all(&default_root).expect("default Claude root should be created");
        fs::write(
            vault_settings_path(&roots.global_root),
            r#"{"claude_session_roots":[".claude/projects","missing-claude-sessions"]}"#,
        )
        .expect("global settings should be writable");

        let session_roots = resolve_claude_session_log_roots(&roots, &base, &home_dir)
            .expect("Claude session log roots should resolve");

        assert_eq!(session_roots.len(), 1);
        assert_eq!(
            session_roots[0].path,
            default_root
                .canonicalize()
                .expect("default root should canonicalize")
        );

        fs::remove_dir_all(base).ok();
    }

    #[test]
    fn resolves_codex_session_log_roots_from_default_and_settings() {
        let base = std::env::temp_dir().join(format!("ctx-codex-session-roots-{}", Uuid::new_v4()));
        let roots = VaultRoots {
            global_root: base.join("home").join(".ctx").join("vault"),
            local_root: Some(base.join("project").join(".ctx").join("vault")),
        };
        let home_dir = base.join("home");
        let default_root = home_dir.join(".codex").join("sessions");
        let global_root = base.join("home").join("extra-codex-sessions");
        let local_root = base.join("project").join("project-codex-sessions");

        fs::create_dir_all(&roots.global_root).expect("global vault root should be created");
        fs::create_dir_all(roots.local_root.as_ref().unwrap())
            .expect("local vault root should be created");
        fs::create_dir_all(&default_root).expect("default Codex root should be created");
        fs::create_dir_all(&global_root).expect("global Codex root should be created");
        fs::create_dir_all(&local_root).expect("local Codex root should be created");

        fs::write(
            vault_settings_path(&roots.global_root),
            r#"{"codex_session_roots":["extra-codex-sessions"]}"#,
        )
        .expect("global settings should be writable");
        fs::write(
            vault_settings_path(roots.local_root.as_ref().unwrap()),
            r#"{"codex_session_roots":[{"path":"project-codex-sessions"}]}"#,
        )
        .expect("local settings should be writable");

        let session_roots =
            resolve_codex_session_log_roots(&roots, &base.join("project"), &home_dir)
                .expect("Codex session log roots should resolve");

        assert_eq!(session_roots.len(), 3);
        assert_eq!(
            session_roots[0].path,
            default_root
                .canonicalize()
                .expect("default root should canonicalize")
        );
        assert_eq!(session_roots[0].source, SessionLogRootSource::KnownDefault);
        assert_eq!(
            session_roots[1].path,
            global_root
                .canonicalize()
                .expect("global root should canonicalize")
        );
        assert_eq!(
            session_roots[1].source,
            SessionLogRootSource::VaultSettings {
                path: vault_settings_path(&roots.global_root),
                scope: VaultScope::Global,
            }
        );
        assert_eq!(
            session_roots[2].path,
            local_root
                .canonicalize()
                .expect("local root should canonicalize")
        );

        fs::remove_dir_all(base).ok();
    }

    #[cfg(unix)]
    #[test]
    fn existing_claude_session_log_roots_must_be_readable() {
        use std::os::unix::fs::PermissionsExt;

        let base = std::env::temp_dir().join(format!(
            "ctx-claude-session-roots-unreadable-{}",
            Uuid::new_v4()
        ));
        let roots = VaultRoots {
            global_root: base.join("home").join(".ctx").join("vault"),
            local_root: None,
        };
        let home_dir = base.join("home");
        let unreadable_root = home_dir.join(".claude").join("projects");

        fs::create_dir_all(&roots.global_root).expect("global vault root should be created");
        fs::create_dir_all(&unreadable_root).expect("default Claude root should be created");
        let mut permissions = fs::metadata(&unreadable_root)
            .expect("default Claude root metadata should be readable")
            .permissions();
        permissions.set_mode(0o000);
        fs::set_permissions(&unreadable_root, permissions)
            .expect("default Claude root should be made unreadable");

        let error = resolve_claude_session_log_roots(&roots, &base, &home_dir)
            .expect_err("unreadable Claude session log root should fail");

        let mut restore_permissions = fs::metadata(&unreadable_root)
            .expect("default Claude root metadata should still be readable")
            .permissions();
        restore_permissions.set_mode(0o700);
        fs::set_permissions(&unreadable_root, restore_permissions)
            .expect("default Claude root permissions should be restored");

        assert!(error.to_string().contains("Claude session log root"));
        assert!(error.to_string().contains("is not readable"));

        fs::remove_dir_all(base).ok();
    }

    #[test]
    fn rejects_missing_configured_scan_root() {
        let base =
            std::env::temp_dir().join(format!("ctx-settings-missing-scan-root-{}", Uuid::new_v4()));
        let roots = VaultRoots {
            global_root: base.join("home").join(".ctx").join("vault"),
            local_root: None,
        };
        fs::create_dir_all(&roots.global_root).expect("global vault root should be created");
        fs::write(
            vault_settings_path(&roots.global_root),
            r#"{"scan_roots":["missing-contexts"]}"#,
        )
        .expect("global settings should be writable");

        let error = load_configured_scan_roots(&roots, &base)
            .expect_err("missing configured scan root should be rejected")
            .to_string();

        assert!(error.contains("configured scan root"));
        assert!(error.contains("is not accessible"));

        fs::remove_dir_all(base).ok();
    }
}
