use super::{
    injection::{
        default_wrapper_state_dir, injection_strategy, AGENTS_MD_FILE_NAME, CTX_END_MARKER,
        CTX_START_MARKER,
    },
    models::{
        CliExecutionSettings, CliTarget, ContextFragment, InjectionStrategy, Preset,
        PresetContextComposition, PresetContextSelection, PresetContextSelectionKind,
        PresetExecutionSettingsUpdate, PresetMetadata, SubagentManifest, SubagentManifestUpdate,
        VaultScope, WrapperBehavior,
    },
    vault::{canonical_vault_entry_key, VaultRoots, CTX_HOME_DIR},
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::{
    cmp::Ordering,
    collections::{BTreeMap, BTreeSet},
    fmt, fs,
    path::{Path, PathBuf},
};
use uuid::Uuid;

pub const MANAGED_PRESETS_DIR: &str = "presets";
pub const MAX_SUBAGENT_MANIFEST_JSON_BYTES: usize = 64 * 1024;

pub fn new_empty_preset(
    name: impl Into<String>,
    target: CliTarget,
    working_dir: PathBuf,
) -> Preset {
    Preset {
        preset_id: Uuid::new_v4(),
        preset_name: name.into(),
        preset_contexts: Vec::new(),
        preset_metadata: PresetMetadata::default(),
        preset_context_composition: Vec::new(),
        preset_target_cli: target,
        preset_working_dir: working_dir,
        preset_model: None,
        subagent_manifest: None,
    }
}

#[derive(Debug, Clone)]
pub struct LoadedPreset {
    pub preset: Preset,
    pub contexts: Vec<ContextFragment>,
    pub passthrough_args: Vec<String>,
    pub file_path: PathBuf,
    pub vault_scope: VaultScope,
}

#[derive(Debug, Clone, Serialize, Eq, PartialEq)]
pub struct PresetSummary {
    pub preset_id: Uuid,
    pub preset_name: String,
    pub preset_description: Option<String>,
    pub preset_tags: Vec<String>,
    pub preset_folder_path: PathBuf,
    pub preset_target_cli: CliTarget,
    pub preset_context_count: usize,
    pub preset_model: Option<String>,
    pub preset_context_composition: Vec<PresetContextComposition>,
    pub cli_execution_settings: CliExecutionSettings,
    pub wrapper_behavior: WrapperBehavior,
    pub subagent_manifest: Option<SubagentManifest>,
    pub file_path: PathBuf,
    pub vault_scope: VaultScope,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub enum PresetLoadError {
    NotFound {
        preset_ref: String,
        searched_dirs: Vec<PathBuf>,
        available_presets: Vec<String>,
    },
    AmbiguousPresetName {
        preset_ref: String,
        matches: Vec<PathBuf>,
    },
    TargetMismatch {
        requested: CliTarget,
        actual: CliTarget,
        path: PathBuf,
    },
    MissingContext {
        preset: String,
        context_ref: String,
    },
    Validation(String),
    Io(String),
    Parse(String),
}

impl fmt::Display for PresetLoadError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFound {
                preset_ref,
                searched_dirs,
                available_presets,
            } => {
                let searched = if searched_dirs.is_empty() {
                    "no preset directories were configured".to_string()
                } else {
                    searched_dirs
                        .iter()
                        .map(|path| path.display().to_string())
                        .collect::<Vec<_>>()
                        .join(", ")
                };
                let available = if available_presets.is_empty() {
                    "none".to_string()
                } else {
                    available_presets.join(", ")
                };
                write!(
                    formatter,
                    "preset '{preset_ref}' was not found in the resolved vault overlay. Searched: {searched}. Available presets: {available}. Create a preset JSON file under presets/ or choose one of the available presets."
                )
            }
            Self::AmbiguousPresetName {
                preset_ref,
                matches,
            } => write!(
                formatter,
                "preset name is ambiguous in resolved vault overlay: {preset_ref}. Matching files: {}",
                matches
                    .iter()
                    .map(|path| path.display().to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            Self::TargetMismatch {
                requested,
                actual,
                path,
            } => write!(
                formatter,
                "invalid preset definition in {}: preset_target_cli is {actual:?}, but launch requested {requested:?}. Use ctx launch {:?} or update preset_target_cli.",
                path.display(),
                actual
            ),
            Self::MissingContext {
                preset,
                context_ref,
            } => write!(
                formatter,
                "invalid preset definition for '{preset}': preset_contexts references missing context '{context_ref}'. Import or create that context in the resolved vault overlay, or remove it from the preset."
            ),
            Self::Validation(message) => write!(formatter, "{message}"),
            Self::Io(message) => write!(formatter, "{message}"),
            Self::Parse(message) => write!(formatter, "{message}"),
        }
    }
}

impl std::error::Error for PresetLoadError {}

#[derive(Debug, Clone)]
struct ResolvedPresetFile {
    key: String,
    path: PathBuf,
    scope: VaultScope,
}

#[derive(Debug, Clone, Deserialize)]
struct PresetFile {
    preset_id: Option<Uuid>,
    preset_name: Option<String>,
    preset_description: Option<String>,
    preset_tags: Option<Vec<String>>,
    preset_folder_path: Option<PathBuf>,
    preset_contexts: Option<Vec<PresetContextFileEntry>>,
    preset_target_cli: Option<CliTarget>,
    preset_working_dir: Option<PathBuf>,
    preset_model: Option<String>,
    #[serde(alias = "cli_execution")]
    cli_execution_settings: Option<PresetCliExecutionSettingsFile>,
    #[serde(alias = "wrapper")]
    wrapper_behavior: Option<PresetWrapperBehaviorFile>,
    subagent_manifest: Option<SubagentManifest>,
}

#[derive(Debug, Clone, Deserialize)]
struct PresetCliExecutionSettingsFile {
    #[serde(alias = "preset_target_cli")]
    target_cli: Option<CliTarget>,
    #[serde(alias = "preset_working_dir")]
    working_dir: Option<PathBuf>,
    #[serde(alias = "preset_model")]
    model: Option<String>,
    passthrough_args: Option<Vec<String>>,
}

#[derive(Debug, Clone, Deserialize)]
struct PresetWrapperBehaviorFile {
    injection_strategy: Option<InjectionStrategy>,
    cleanup_on_exit: Option<bool>,
    cleanup_stale_on_launch: Option<bool>,
    state_dir: Option<PathBuf>,
    start_marker: Option<String>,
    end_marker: Option<String>,
    agents_md_path: Option<PathBuf>,
    prompt_file_dir: Option<PathBuf>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
enum PresetContextFileEntry {
    Ref(String),
    Object {
        #[serde(alias = "context_id", alias = "path", alias = "file_path")]
        context_ref: String,
        order: Option<usize>,
        required: Option<bool>,
        #[serde(default)]
        selection: Option<PresetContextSelection>,
        #[serde(default, alias = "fragment_kind", alias = "kind")]
        selection_kind: Option<PresetContextSelectionKind>,
        #[serde(default)]
        heading: Option<String>,
        #[serde(default)]
        anchor: Option<String>,
        #[serde(default)]
        line_start: Option<u32>,
        #[serde(default)]
        line_end: Option<u32>,
        #[serde(default)]
        include_children: Option<bool>,
    },
}

#[derive(Debug, Clone)]
struct ResolvedPresetContext {
    context: ContextFragment,
    composition: PresetContextComposition,
    source_index: usize,
}

pub fn managed_presets_dir(root: &Path) -> PathBuf {
    root.join(MANAGED_PRESETS_DIR)
}

pub fn validate_cli_execution_settings(
    target_cli: CliTarget,
    working_dir: &Path,
    model: Option<String>,
    passthrough_args: Vec<String>,
    default_working_dir: &Path,
) -> Result<CliExecutionSettings, PresetLoadError> {
    let working_dir = resolve_safe_working_dir(working_dir, default_working_dir)?;

    Ok(CliExecutionSettings {
        target_cli,
        working_dir,
        model: normalize_optional_string(model),
        passthrough_args: passthrough_args
            .into_iter()
            .map(|arg| arg.trim().to_string())
            .filter(|arg| !arg.is_empty())
            .collect(),
    })
}

pub fn save_preset_execution_settings(
    roots: &VaultRoots,
    update: PresetExecutionSettingsUpdate,
    default_working_dir: &Path,
) -> Result<PresetSummary, PresetLoadError> {
    let cli_execution_settings = validate_cli_execution_settings(
        update.target_cli,
        &update.working_dir,
        update.model,
        update.passthrough_args,
        default_working_dir,
    )?;
    let preset_key = safe_preset_file_stem(&update.preset_ref)?;
    let root = match update.vault_scope {
        VaultScope::Global => roots.global_root.clone(),
        VaultScope::Local => roots.local_root.clone().ok_or_else(|| {
            PresetLoadError::Validation("local vault root is not configured".to_string())
        })?,
    };
    let presets_dir = managed_presets_dir(&root);
    fs::create_dir_all(&presets_dir).map_err(|error| {
        PresetLoadError::Io(format!(
            "failed to create preset directory {}: {error}",
            presets_dir.display()
        ))
    })?;
    let file_path = presets_dir.join(format!("{preset_key}.json"));

    let mut document = if file_path.exists() {
        let content = fs::read_to_string(&file_path).map_err(|error| {
            PresetLoadError::Io(format!(
                "failed to read preset file {}: {error}",
                file_path.display()
            ))
        })?;
        serde_json::from_str::<Value>(&content).map_err(|error| {
            PresetLoadError::Parse(format!(
                "failed to parse preset file {} as JSON: {error}",
                file_path.display()
            ))
        })?
    } else {
        Value::Object(Map::new())
    };

    let object = document.as_object_mut().ok_or_else(|| {
        PresetLoadError::Parse(format!(
            "preset file {} must contain a JSON object",
            file_path.display()
        ))
    })?;

    if !object.contains_key("preset_id") {
        object.insert("preset_id".to_string(), json!(Uuid::new_v4()));
    }

    let preset_name = normalize_optional_string(update.preset_name)
        .or_else(|| {
            object
                .get("preset_name")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .unwrap_or_else(|| preset_key.replace('-', " ").replace('_', " "));
    object.insert("preset_name".to_string(), json!(preset_name));
    object.insert(
        "preset_target_cli".to_string(),
        serde_json::to_value(cli_execution_settings.target_cli)
            .expect("CliTarget should serialize"),
    );
    object.insert(
        "preset_working_dir".to_string(),
        json!(cli_execution_settings.working_dir),
    );
    object.insert(
        "preset_model".to_string(),
        match &cli_execution_settings.model {
            Some(model) => json!(model),
            None => Value::Null,
        },
    );
    object.insert(
        "cli_execution_settings".to_string(),
        serde_json::to_value(&cli_execution_settings)
            .expect("CliExecutionSettings should serialize"),
    );

    fs::write(
        &file_path,
        serde_json::to_string_pretty(&document)
            .expect("preset JSON document should serialize after validation"),
    )
    .map_err(|error| {
        PresetLoadError::Io(format!(
            "failed to write preset execution settings {}: {error}",
            file_path.display()
        ))
    })?;

    let reloaded: PresetFile = serde_json::from_value(document).map_err(|error| {
        PresetLoadError::Parse(format!(
            "failed to reload persisted preset {}: {error}",
            file_path.display()
        ))
    })?;
    let wrapper_behavior = resolve_wrapper_behavior(&reloaded, &cli_execution_settings);
    let preset_id = reloaded.preset_id.unwrap_or_else(Uuid::new_v4);
    let subagent_manifest = validate_optional_subagent_manifest(reloaded.subagent_manifest)?;
    let preset_metadata = PresetMetadata {
        description: reloaded.preset_description,
        tags: reloaded.preset_tags.unwrap_or_default(),
        folder_path: reloaded.preset_folder_path.unwrap_or_default(),
    };

    Ok(PresetSummary {
        preset_id,
        preset_name: reloaded
            .preset_name
            .unwrap_or_else(|| preset_key.replace('-', " ").replace('_', " ")),
        preset_description: preset_metadata.description,
        preset_tags: preset_metadata.tags,
        preset_folder_path: preset_metadata.folder_path,
        preset_target_cli: cli_execution_settings.target_cli,
        preset_context_count: reloaded.preset_contexts.as_ref().map_or(0, Vec::len),
        preset_model: cli_execution_settings.model.clone(),
        preset_context_composition: Vec::new(),
        cli_execution_settings,
        wrapper_behavior,
        subagent_manifest,
        file_path,
        vault_scope: update.vault_scope,
    })
}

pub fn save_preset_subagent_manifest(
    roots: &VaultRoots,
    update: SubagentManifestUpdate,
    default_working_dir: &Path,
) -> Result<PresetSummary, PresetLoadError> {
    let manifest = update
        .manifest
        .map(validate_subagent_manifest)
        .transpose()?;
    let preset_key = safe_preset_file_stem(&update.preset_ref)?;
    let root = match update.vault_scope {
        VaultScope::Global => roots.global_root.clone(),
        VaultScope::Local => roots.local_root.clone().ok_or_else(|| {
            PresetLoadError::Validation("local vault root is not configured".to_string())
        })?,
    };
    let presets_dir = managed_presets_dir(&root);
    fs::create_dir_all(&presets_dir).map_err(|error| {
        PresetLoadError::Io(format!(
            "failed to create preset directory {}: {error}",
            presets_dir.display()
        ))
    })?;
    let file_path = presets_dir.join(format!("{preset_key}.json"));

    let mut document = if file_path.exists() {
        let content = fs::read_to_string(&file_path).map_err(|error| {
            PresetLoadError::Io(format!(
                "failed to read preset file {}: {error}",
                file_path.display()
            ))
        })?;
        serde_json::from_str::<Value>(&content).map_err(|error| {
            PresetLoadError::Parse(format!(
                "failed to parse preset file {} as JSON: {error}",
                file_path.display()
            ))
        })?
    } else {
        Value::Object(Map::new())
    };

    let object = document.as_object_mut().ok_or_else(|| {
        PresetLoadError::Parse(format!(
            "preset file {} must contain a JSON object",
            file_path.display()
        ))
    })?;

    if !object.contains_key("preset_id") {
        object.insert("preset_id".to_string(), json!(Uuid::new_v4()));
    }

    let preset_name = normalize_optional_string(update.preset_name)
        .or_else(|| {
            object
                .get("preset_name")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .unwrap_or_else(|| preset_key.replace('-', " ").replace('_', " "));
    object.insert("preset_name".to_string(), json!(preset_name));

    match &manifest {
        Some(manifest) => {
            object.insert(
                "subagent_manifest".to_string(),
                serde_json::to_value(manifest).expect("SubagentManifest should serialize"),
            );
        }
        None => {
            object.remove("subagent_manifest");
        }
    }

    fs::write(
        &file_path,
        serde_json::to_string_pretty(&document)
            .expect("preset JSON document should serialize after manifest validation"),
    )
    .map_err(|error| {
        PresetLoadError::Io(format!(
            "failed to write preset subagent manifest {}: {error}",
            file_path.display()
        ))
    })?;

    let reloaded: PresetFile = serde_json::from_value(document).map_err(|error| {
        PresetLoadError::Parse(format!(
            "failed to reload persisted preset {}: {error}",
            file_path.display()
        ))
    })?;
    let cli_execution_settings =
        resolve_cli_execution_settings(&reloaded, CliTarget::Codex, default_working_dir);
    let wrapper_behavior = resolve_wrapper_behavior(&reloaded, &cli_execution_settings);
    let preset_id = reloaded.preset_id.unwrap_or_else(Uuid::new_v4);
    let subagent_manifest = validate_optional_subagent_manifest(reloaded.subagent_manifest)?;
    let preset_metadata = PresetMetadata {
        description: reloaded.preset_description,
        tags: reloaded.preset_tags.unwrap_or_default(),
        folder_path: reloaded.preset_folder_path.unwrap_or_default(),
    };

    Ok(PresetSummary {
        preset_id,
        preset_name: reloaded
            .preset_name
            .unwrap_or_else(|| preset_key.replace('-', " ").replace('_', " ")),
        preset_description: preset_metadata.description,
        preset_tags: preset_metadata.tags,
        preset_folder_path: preset_metadata.folder_path,
        preset_target_cli: cli_execution_settings.target_cli,
        preset_context_count: reloaded.preset_contexts.as_ref().map_or(0, Vec::len),
        preset_model: cli_execution_settings.model.clone(),
        preset_context_composition: Vec::new(),
        cli_execution_settings,
        wrapper_behavior,
        subagent_manifest,
        file_path,
        vault_scope: update.vault_scope,
    })
}

pub fn validate_subagent_manifest(
    mut manifest: SubagentManifest,
) -> Result<SubagentManifest, PresetLoadError> {
    manifest.manifest_version =
        normalize_optional_string(manifest.manifest_version).or_else(|| Some("1".to_string()));

    let mut errors = Vec::new();
    let mut role_ids = BTreeSet::new();

    if manifest.manifest_version.as_deref() != Some("1") {
        errors.push(format!(
            "subagent_manifest.manifest_version must be \"1\" for Phase 1 manifests, got {}",
            manifest.manifest_version.as_deref().unwrap_or("<missing>")
        ));
    }

    if manifest.roles.is_empty() {
        errors.push("subagent_manifest.roles must include at least one delegated role".to_string());
    }

    for (index, role) in manifest.roles.iter_mut().enumerate() {
        role.role_id = role.role_id.trim().to_string();
        role.role_name = role.role_name.trim().to_string();
        role.role = role.role.trim().to_string();
        role.description = normalize_optional_string(role.description.take());
        role.model = normalize_optional_string(role.model.take());
        role.capabilities = normalized_string_list(std::mem::take(&mut role.capabilities));
        role.constraints = normalized_string_list(std::mem::take(&mut role.constraints));
        role.metadata = normalized_metadata_map(std::mem::take(&mut role.metadata));
        role.assigned_contexts =
            normalized_string_list(std::mem::take(&mut role.assigned_contexts));
        role.spawn_instructions =
            normalized_string_list(std::mem::take(&mut role.spawn_instructions));
        role.spawn_guidance.select_when =
            normalized_string_list(std::mem::take(&mut role.spawn_guidance.select_when));
        role.spawn_guidance.avoid_when =
            normalized_string_list(std::mem::take(&mut role.spawn_guidance.avoid_when));
        role.spawn_guidance.delegation_prompt =
            normalize_optional_string(role.spawn_guidance.delegation_prompt.take());
        role.handoff_targets = normalized_string_list(std::mem::take(&mut role.handoff_targets));

        let role_path = format!("subagent_manifest.roles[{index}]");
        let display_role = if role.role_id.is_empty() {
            role_path.clone()
        } else {
            format!("{role_path} ({})", role.role_id)
        };

        if role.role_id.is_empty() {
            errors.push(format!("{role_path}.id must be non-empty"));
        } else {
            if !is_safe_subagent_ref(&role.role_id) {
                errors.push(format!(
                    "{display_role}.id may only contain letters, numbers, '-' and '_'"
                ));
            }
            if !role_ids.insert(role.role_id.clone()) {
                errors.push(format!(
                    "subagent_manifest contains duplicate id: {}",
                    role.role_id
                ));
            }
        }

        if role.role_name.is_empty() {
            errors.push(format!("{display_role}.name must be non-empty"));
        }
        if role.role.is_empty() {
            errors.push(format!("{display_role}.role must be non-empty"));
        }
        if role.capabilities.is_empty() {
            errors.push(format!(
                "{display_role}.capabilities must include at least one capability"
            ));
        }
        if role.constraints.is_empty() {
            errors.push(format!(
                "{display_role}.constraints must include at least one constraint"
            ));
        }
        if role.assigned_contexts.is_empty() {
            errors.push(format!(
                "{display_role}.assigned_contexts must include at least one context ref"
            ));
        }
        for context_ref in &role.assigned_contexts {
            if !is_safe_manifest_context_ref(context_ref) {
                errors.push(format!(
                    "{display_role}.assigned_contexts contains unsafe context ref: {context_ref}"
                ));
            }
        }
        if role.spawn_instructions.is_empty() {
            errors.push(format!(
                "{display_role}.spawn_instructions must include at least one instruction"
            ));
        }
        if role.spawn_guidance.select_when.is_empty() {
            errors.push(format!(
                "{display_role}.spawn_guidance.select_when must explain when to select this subagent"
            ));
        }
        if role.spawn_guidance.avoid_when.is_empty() {
            errors.push(format!(
                "{display_role}.spawn_guidance.avoid_when must explain when to avoid this subagent"
            ));
        }
        for target in &role.handoff_targets {
            if !is_safe_subagent_ref(target) {
                errors.push(format!(
                    "{display_role}.handoff_targets contains invalid role ref: {target}"
                ));
            }
        }
    }

    manifest.handoff_constraints.allowed_handoff_targets = normalized_string_list(std::mem::take(
        &mut manifest.handoff_constraints.allowed_handoff_targets,
    ));
    manifest.handoff_constraints.blocked_handoff_targets = normalized_string_list(std::mem::take(
        &mut manifest.handoff_constraints.blocked_handoff_targets,
    ));
    manifest.handoff_constraints.handoff_prompt_template =
        normalize_optional_string(manifest.handoff_constraints.handoff_prompt_template.take());

    if manifest.handoff_constraints.max_parallel_subagents == Some(0) {
        errors.push(
            "subagent_manifest.handoff_constraints.max_parallel_subagents must be greater than 0"
                .to_string(),
        );
    }

    for target in &manifest.handoff_constraints.allowed_handoff_targets {
        if !is_safe_subagent_ref(target) {
            errors.push(format!(
                "subagent_manifest.handoff_constraints.allowed_handoff_targets contains invalid role ref: {target}"
            ));
        }
    }
    for target in &manifest.handoff_constraints.blocked_handoff_targets {
        if !is_safe_subagent_ref(target) {
            errors.push(format!(
                "subagent_manifest.handoff_constraints.blocked_handoff_targets contains invalid role ref: {target}"
            ));
        }
    }

    let allowed: BTreeSet<String> = manifest
        .handoff_constraints
        .allowed_handoff_targets
        .iter()
        .cloned()
        .collect();
    for target in &manifest.handoff_constraints.blocked_handoff_targets {
        if allowed.contains(target) {
            errors.push(format!(
                "subagent_manifest.handoff_constraints cannot both allow and block handoff target: {target}"
            ));
        }
    }

    let manifest_size = serde_json::to_vec(&manifest)
        .map(|bytes| bytes.len())
        .map_err(|error| {
            PresetLoadError::Validation(format!(
                "failed to serialize subagent_manifest for validation: {error}"
            ))
        })?;
    if manifest_size > MAX_SUBAGENT_MANIFEST_JSON_BYTES {
        errors.push(format!(
            "subagent_manifest JSON payload is {manifest_size} bytes, exceeding the {MAX_SUBAGENT_MANIFEST_JSON_BYTES} byte launch limit"
        ));
    }

    if errors.is_empty() {
        Ok(manifest)
    } else {
        Err(PresetLoadError::Validation(format!(
            "invalid subagent_manifest:\n- {}",
            errors.join("\n- ")
        )))
    }
}

fn validate_optional_subagent_manifest(
    manifest: Option<SubagentManifest>,
) -> Result<Option<SubagentManifest>, PresetLoadError> {
    manifest.map(validate_subagent_manifest).transpose()
}

pub fn list_presets_from_resolved_overlay(
    roots: &VaultRoots,
    resolved_contexts: &[ContextFragment],
    default_working_dir: &Path,
) -> Result<Vec<PresetSummary>, PresetLoadError> {
    let preset_files = resolved_preset_files(roots)?;
    let mut presets = Vec::new();

    for preset_file in preset_files {
        let content = fs::read_to_string(&preset_file.path).map_err(|error| {
            PresetLoadError::Io(format!(
                "failed to read preset file {}: {error}",
                preset_file.path.display()
            ))
        })?;
        let definition = parse_preset_file_content(&preset_file.path, &content)?;
        let preset_name = definition
            .preset_name
            .clone()
            .unwrap_or_else(|| preset_file.key.replace('-', " ").replace('_', " "));
        let cli_execution_settings =
            resolve_cli_execution_settings(&definition, CliTarget::Codex, default_working_dir);
        let _preset_target_cli = cli_execution_settings.target_cli;
        let selected_contexts = resolve_preset_contexts(
            &preset_name,
            definition.preset_contexts.clone().unwrap_or_default(),
            resolved_contexts,
        )?;
        let preset_metadata = PresetMetadata {
            description: definition.preset_description.clone(),
            tags: definition.preset_tags.clone().unwrap_or_default(),
            folder_path: definition.preset_folder_path.clone().unwrap_or_default(),
        };
        let wrapper_behavior = resolve_wrapper_behavior(&definition, &cli_execution_settings);
        let subagent_manifest = validate_optional_subagent_manifest(definition.subagent_manifest)?;
        let preset = Preset {
            preset_id: definition.preset_id.unwrap_or_else(Uuid::new_v4),
            preset_name,
            preset_contexts: selected_contexts
                .iter()
                .map(|resolved| resolved.context.context_id)
                .collect(),
            preset_metadata,
            preset_context_composition: selected_contexts
                .iter()
                .map(|resolved| resolved.composition.clone())
                .collect(),
            preset_target_cli: cli_execution_settings.target_cli,
            preset_working_dir: cli_execution_settings.working_dir.clone(),
            preset_model: cli_execution_settings.model.clone(),
            subagent_manifest,
        };

        presets.push(PresetSummary {
            preset_id: preset.preset_id,
            preset_name: preset.preset_name,
            preset_description: preset.preset_metadata.description,
            preset_tags: preset.preset_metadata.tags,
            preset_folder_path: preset.preset_metadata.folder_path,
            preset_target_cli: preset.preset_target_cli,
            preset_context_count: preset.preset_contexts.len(),
            preset_model: preset.preset_model,
            preset_context_composition: preset.preset_context_composition,
            cli_execution_settings,
            wrapper_behavior,
            subagent_manifest: preset.subagent_manifest,
            file_path: preset_file.path,
            vault_scope: preset_file.scope,
        });
    }

    presets.sort_by(|left, right| {
        left.vault_scope
            .cmp(&right.vault_scope)
            .then_with(|| left.preset_name.cmp(&right.preset_name))
    });

    Ok(presets)
}

pub fn load_preset_from_resolved_overlay(
    roots: &VaultRoots,
    resolved_contexts: &[ContextFragment],
    preset_ref: &str,
    requested_target: CliTarget,
    default_working_dir: &Path,
) -> Result<LoadedPreset, PresetLoadError> {
    let preset_file = find_resolved_preset_file(roots, preset_ref)?
        .ok_or_else(|| preset_not_found_error(roots, preset_ref))?;
    let content = fs::read_to_string(&preset_file.path).map_err(|error| {
        PresetLoadError::Io(format!(
            "failed to read preset file {}: {error}",
            preset_file.path.display()
        ))
    })?;
    let definition = parse_preset_file_content(&preset_file.path, &content)?;

    let cli_execution_settings =
        resolve_cli_execution_settings(&definition, requested_target, default_working_dir);
    let actual_target = cli_execution_settings.target_cli;
    if actual_target != requested_target {
        return Err(PresetLoadError::TargetMismatch {
            requested: requested_target,
            actual: actual_target,
            path: preset_file.path,
        });
    }

    let preset_name = definition
        .preset_name
        .clone()
        .unwrap_or_else(|| preset_file.key.replace('-', " ").replace('_', " "));
    let selected_contexts = resolve_preset_contexts(
        &preset_name,
        definition.preset_contexts.unwrap_or_default(),
        resolved_contexts,
    )?;
    let preset_metadata = PresetMetadata {
        description: definition.preset_description,
        tags: definition.preset_tags.unwrap_or_default(),
        folder_path: definition.preset_folder_path.unwrap_or_default(),
    };
    let subagent_manifest = validate_optional_subagent_manifest(definition.subagent_manifest)?;
    let preset = Preset {
        preset_id: definition.preset_id.unwrap_or_else(Uuid::new_v4),
        preset_name,
        preset_contexts: selected_contexts
            .iter()
            .map(|resolved| resolved.context.context_id)
            .collect(),
        preset_metadata,
        preset_context_composition: selected_contexts
            .iter()
            .map(|resolved| resolved.composition.clone())
            .collect(),
        preset_target_cli: cli_execution_settings.target_cli,
        preset_working_dir: cli_execution_settings.working_dir,
        preset_model: cli_execution_settings.model,
        subagent_manifest,
    };

    Ok(LoadedPreset {
        preset,
        contexts: selected_contexts
            .into_iter()
            .map(|resolved| resolved.context)
            .collect(),
        passthrough_args: cli_execution_settings.passthrough_args,
        file_path: preset_file.path,
        vault_scope: preset_file.scope,
    })
}

fn parse_preset_file_content(path: &Path, content: &str) -> Result<PresetFile, PresetLoadError> {
    let document: Value = serde_json::from_str(content).map_err(|error| {
        PresetLoadError::Parse(format!(
            "invalid preset definition in {}: file must contain valid JSON ({error})",
            path.display()
        ))
    })?;
    if !document.is_object() {
        return Err(PresetLoadError::Parse(format!(
            "invalid preset definition in {}: top-level JSON value must be an object",
            path.display()
        )));
    }
    validate_raw_subagent_manifest(path, &document)?;

    serde_json::from_value(document).map_err(|error| {
        PresetLoadError::Parse(format!(
            "invalid preset definition in {}: {error}",
            path.display()
        ))
    })
}

fn validate_raw_subagent_manifest(path: &Path, document: &Value) -> Result<(), PresetLoadError> {
    let Some(manifest) = document.get("subagent_manifest") else {
        return Ok(());
    };
    if manifest.is_null() {
        return Ok(());
    }
    if !manifest.is_object() {
        return Err(PresetLoadError::Validation(format!(
            "invalid subagent_manifest in preset file {}: expected a JSON object or null",
            path.display()
        )));
    }

    let manifest_size = serde_json::to_vec(manifest)
        .map(|bytes| bytes.len())
        .map_err(|error| {
            PresetLoadError::Validation(format!(
                "failed to serialize raw subagent_manifest in preset file {}: {error}",
                path.display()
            ))
        })?;
    if manifest_size > MAX_SUBAGENT_MANIFEST_JSON_BYTES {
        return Err(PresetLoadError::Validation(format!(
            "invalid subagent_manifest in preset file {}: JSON payload is {manifest_size} bytes, exceeding the {MAX_SUBAGENT_MANIFEST_JSON_BYTES} byte launch limit",
            path.display()
        )));
    }

    Ok(())
}

fn resolve_cli_execution_settings(
    definition: &PresetFile,
    default_target: CliTarget,
    default_working_dir: &Path,
) -> CliExecutionSettings {
    let nested = definition.cli_execution_settings.as_ref();

    CliExecutionSettings {
        target_cli: nested
            .and_then(|settings| settings.target_cli)
            .or(definition.preset_target_cli)
            .unwrap_or(default_target),
        working_dir: nested
            .and_then(|settings| settings.working_dir.clone())
            .or_else(|| definition.preset_working_dir.clone())
            .unwrap_or_else(|| default_working_dir.to_path_buf()),
        model: normalize_optional_string(
            nested
                .and_then(|settings| settings.model.clone())
                .or_else(|| definition.preset_model.clone()),
        ),
        passthrough_args: nested
            .and_then(|settings| settings.passthrough_args.clone())
            .unwrap_or_default()
            .into_iter()
            .map(|arg| arg.trim().to_string())
            .filter(|arg| !arg.is_empty())
            .collect(),
    }
}

fn resolve_wrapper_behavior(
    definition: &PresetFile,
    cli_execution_settings: &CliExecutionSettings,
) -> WrapperBehavior {
    let nested = definition.wrapper_behavior.as_ref();

    WrapperBehavior {
        injection_strategy: nested
            .and_then(|wrapper| wrapper.injection_strategy)
            .unwrap_or_else(|| default_injection_strategy(cli_execution_settings.target_cli)),
        cleanup_on_exit: nested
            .and_then(|wrapper| wrapper.cleanup_on_exit)
            .unwrap_or(true),
        cleanup_stale_on_launch: nested
            .and_then(|wrapper| wrapper.cleanup_stale_on_launch)
            .unwrap_or(true),
        state_dir: nested
            .and_then(|wrapper| wrapper.state_dir.clone())
            .unwrap_or_else(default_wrapper_state_dir),
        start_marker: nested
            .and_then(|wrapper| wrapper.start_marker.clone())
            .or_else(|| {
                (cli_execution_settings.target_cli == CliTarget::Codex)
                    .then(|| CTX_START_MARKER.to_string())
            }),
        end_marker: nested
            .and_then(|wrapper| wrapper.end_marker.clone())
            .or_else(|| {
                (cli_execution_settings.target_cli == CliTarget::Codex)
                    .then(|| CTX_END_MARKER.to_string())
            }),
        agents_md_path: nested
            .and_then(|wrapper| wrapper.agents_md_path.clone())
            .or_else(|| {
                (cli_execution_settings.target_cli == CliTarget::Codex)
                    .then(|| cli_execution_settings.working_dir.join(AGENTS_MD_FILE_NAME))
            }),
        prompt_file_dir: nested
            .and_then(|wrapper| wrapper.prompt_file_dir.clone())
            .or_else(|| {
                (cli_execution_settings.target_cli == CliTarget::Claude)
                    .then(|| std::env::temp_dir().join("ctx").join("claude-prompts"))
            }),
    }
}

fn default_injection_strategy(target: CliTarget) -> InjectionStrategy {
    match injection_strategy(target) {
        "append-system-prompt-file" => InjectionStrategy::AppendSystemPromptFile,
        "agents-md-section-marker-merge" => InjectionStrategy::AgentsMdSectionMarkerMerge,
        _ => unreachable!("ctx injection strategies are exhaustive"),
    }
}

fn find_resolved_preset_file(
    roots: &VaultRoots,
    preset_ref: &str,
) -> Result<Option<ResolvedPresetFile>, PresetLoadError> {
    validate_preset_lookup_ref(preset_ref)?;

    let mut by_key = resolved_preset_files_by_key(roots)?;
    let requested_key = canonical_preset_ref(preset_ref);
    if let Some(preset_file) = by_key.remove(&requested_key) {
        return Ok(Some(preset_file));
    }

    let requested_name = canonical_preset_name(preset_ref);
    let mut matches = Vec::new();
    for preset_file in by_key.into_values() {
        let content = fs::read_to_string(&preset_file.path).map_err(|error| {
            PresetLoadError::Io(format!(
                "failed to read preset file {} while resolving --preset={}: {error}",
                preset_file.path.display(),
                preset_ref
            ))
        })?;
        let definition = parse_preset_file_content(&preset_file.path, &content)?;
        let preset_name = definition
            .preset_name
            .as_deref()
            .unwrap_or(&preset_file.key);
        if canonical_preset_name(preset_name) == requested_name {
            matches.push(preset_file);
        }
    }

    match matches.len() {
        0 => Ok(None),
        1 => Ok(matches.pop()),
        _ => Err(PresetLoadError::AmbiguousPresetName {
            preset_ref: preset_ref.to_string(),
            matches: matches.into_iter().map(|preset| preset.path).collect(),
        }),
    }
}

fn resolved_preset_files(roots: &VaultRoots) -> Result<Vec<ResolvedPresetFile>, PresetLoadError> {
    Ok(resolved_preset_files_by_key(roots)?.into_values().collect())
}

fn preset_not_found_error(roots: &VaultRoots, preset_ref: &str) -> PresetLoadError {
    let searched_dirs = searched_preset_dirs(roots);
    let available_presets = resolved_preset_files_by_key(roots)
        .map(|files| {
            files
                .into_values()
                .map(|preset| preset.key)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    PresetLoadError::NotFound {
        preset_ref: preset_ref.to_string(),
        searched_dirs,
        available_presets,
    }
}

fn searched_preset_dirs(roots: &VaultRoots) -> Vec<PathBuf> {
    let mut dirs = vec![managed_presets_dir(&roots.global_root)];
    if let Some(local_root) = &roots.local_root {
        dirs.push(managed_presets_dir(local_root));
    }
    dirs
}

fn resolved_preset_files_by_key(
    roots: &VaultRoots,
) -> Result<BTreeMap<String, ResolvedPresetFile>, PresetLoadError> {
    let mut by_key: BTreeMap<String, ResolvedPresetFile> = BTreeMap::new();

    collect_preset_files(
        &managed_presets_dir(&roots.global_root),
        VaultScope::Global,
        &mut by_key,
    )?;
    if let Some(local_root) = &roots.local_root {
        collect_preset_files(
            &managed_presets_dir(local_root),
            VaultScope::Local,
            &mut by_key,
        )?;
    }

    Ok(by_key)
}

fn collect_preset_files(
    presets_dir: &Path,
    scope: VaultScope,
    by_key: &mut BTreeMap<String, ResolvedPresetFile>,
) -> Result<(), PresetLoadError> {
    if !presets_dir.exists() {
        return Ok(());
    }

    let entries = fs::read_dir(presets_dir).map_err(|error| {
        PresetLoadError::Io(format!(
            "failed to read preset directory {}: {error}",
            presets_dir.display()
        ))
    })?;

    for entry in entries {
        let entry = entry.map_err(|error| {
            PresetLoadError::Io(format!(
                "failed to read preset directory entry in {}: {error}",
                presets_dir.display()
            ))
        })?;
        let path = entry.path();
        let metadata = entry.metadata().map_err(|error| {
            PresetLoadError::Io(format!(
                "failed to read preset path metadata {}: {error}",
                path.display()
            ))
        })?;
        if !metadata.is_file()
            || path.extension().and_then(|extension| extension.to_str()) != Some("json")
        {
            continue;
        }
        let Some(stem) = path.file_stem().and_then(|stem| stem.to_str()) else {
            continue;
        };
        let key = canonical_preset_ref(stem);

        match by_key.get(&key) {
            Some(existing) if existing.scope == VaultScope::Local => {}
            _ => {
                by_key.insert(key.clone(), ResolvedPresetFile { key, path, scope });
            }
        }
    }

    Ok(())
}

fn resolve_preset_contexts(
    preset_name: &str,
    context_refs: Vec<PresetContextFileEntry>,
    resolved_contexts: &[ContextFragment],
) -> Result<Vec<ResolvedPresetContext>, PresetLoadError> {
    let mut selected = Vec::new();

    for (array_index, entry) in context_refs.into_iter().enumerate() {
        let (context_ref, order, required, selection) = match entry {
            PresetContextFileEntry::Ref(context_ref) => (
                context_ref,
                array_index,
                true,
                PresetContextSelection::default(),
            ),
            PresetContextFileEntry::Object {
                context_ref,
                order,
                required,
                selection,
                selection_kind,
                heading,
                anchor,
                line_start,
                line_end,
                include_children,
            } => (
                context_ref,
                order.unwrap_or(array_index),
                required.unwrap_or(true),
                resolve_context_selection(
                    selection,
                    selection_kind,
                    heading,
                    anchor,
                    line_start,
                    line_end,
                    include_children,
                ),
            ),
        };
        let context = resolved_contexts
            .iter()
            .find(|context| context_matches_ref(context, &context_ref))
            .cloned()
            .ok_or_else(|| PresetLoadError::MissingContext {
                preset: preset_name.to_string(),
                context_ref: context_ref.clone(),
            })?;
        selected.push(ResolvedPresetContext {
            composition: PresetContextComposition {
                context_id: context.context_id,
                order,
                source_ref: context_ref,
                required,
                selection,
            },
            context,
            source_index: array_index,
        });
    }

    selected.sort_by(|left, right| {
        compare_resolved_preset_context_order(left, right)
            .then_with(|| left.source_index.cmp(&right.source_index))
    });

    Ok(selected)
}

fn compare_resolved_preset_context_order(
    left: &ResolvedPresetContext,
    right: &ResolvedPresetContext,
) -> Ordering {
    left.composition
        .order
        .cmp(&right.composition.order)
        .then_with(|| {
            canonical_context_ref(&left.composition.source_ref)
                .cmp(&canonical_context_ref(&right.composition.source_ref))
        })
        .then_with(|| left.context.context_id.cmp(&right.context.context_id))
        .then_with(|| left.composition.required.cmp(&right.composition.required))
}

fn resolve_context_selection(
    selection: Option<PresetContextSelection>,
    selection_kind: Option<PresetContextSelectionKind>,
    heading: Option<String>,
    anchor: Option<String>,
    line_start: Option<u32>,
    line_end: Option<u32>,
    include_children: Option<bool>,
) -> PresetContextSelection {
    let mut resolved = selection.unwrap_or_default();

    if let Some(selection_kind) = selection_kind {
        resolved.selection_kind = selection_kind;
    }
    if heading.is_some() {
        resolved.heading = heading;
    }
    if anchor.is_some() {
        resolved.anchor = anchor;
    }
    if line_start.is_some() {
        resolved.line_start = line_start;
    }
    if line_end.is_some() {
        resolved.line_end = line_end;
    }
    if let Some(include_children) = include_children {
        resolved.include_children = include_children;
    }

    resolved
}

fn context_matches_ref(context: &ContextFragment, context_ref: &str) -> bool {
    canonical_context_ref(&canonical_vault_entry_key(context).relative_path)
        == canonical_context_ref(context_ref)
        || Uuid::parse_str(context_ref)
            .map(|context_id| context_id == context.context_id)
            .unwrap_or(false)
}

fn canonical_preset_ref(value: &str) -> String {
    value.trim().trim_end_matches(".json").to_ascii_lowercase()
}

fn canonical_preset_name(value: &str) -> String {
    value
        .trim()
        .trim_end_matches(".json")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase()
}

fn validate_preset_lookup_ref(value: &str) -> Result<(), PresetLoadError> {
    let trimmed = value.trim().trim_end_matches(".json").trim();
    if trimmed.is_empty() {
        return Err(PresetLoadError::Validation(
            "preset id, file stem, or name cannot be empty".to_string(),
        ));
    }
    if trimmed.contains('/')
        || trimmed.contains('\\')
        || trimmed.contains('\0')
        || trimmed == "."
        || trimmed == ".."
    {
        return Err(PresetLoadError::Validation(format!(
            "preset lookup must be a configured preset id, file stem, or name, not a path: {value}"
        )));
    }

    Ok(())
}

fn safe_preset_file_stem(value: &str) -> Result<String, PresetLoadError> {
    let trimmed = value.trim().trim_end_matches(".json").trim();
    if trimmed.is_empty() {
        return Err(PresetLoadError::Validation(
            "preset id or file stem cannot be empty".to_string(),
        ));
    }
    if trimmed.contains('/') || trimmed.contains('\\') || trimmed == "." || trimmed == ".." {
        return Err(PresetLoadError::Validation(format!(
            "preset id must be a file stem, not a path: {value}"
        )));
    }
    if !trimmed
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'))
    {
        return Err(PresetLoadError::Validation(format!(
            "preset id may only contain letters, numbers, '-' and '_': {value}"
        )));
    }

    Ok(trimmed.to_ascii_lowercase())
}

fn canonical_context_ref(value: &str) -> String {
    value.trim().replace('\\', "/").to_ascii_lowercase()
}

fn normalize_optional_string(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn normalized_string_list(values: Vec<String>) -> Vec<String> {
    values
        .into_iter()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .collect()
}

fn normalized_metadata_map(values: BTreeMap<String, String>) -> BTreeMap<String, String> {
    values
        .into_iter()
        .filter_map(|(key, value)| {
            let key = key.trim().to_string();
            let value = value.trim().to_string();
            if key.is_empty() || value.is_empty() {
                None
            } else {
                Some((key, value))
            }
        })
        .collect()
}

fn is_safe_subagent_ref(value: &str) -> bool {
    !value.is_empty()
        && value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'))
}

fn is_safe_manifest_context_ref(value: &str) -> bool {
    if value.is_empty() || value.starts_with('/') || value.starts_with('\\') || value.contains('\0')
    {
        return false;
    }

    let normalized = value.replace('\\', "/");
    !normalized
        .split('/')
        .any(|segment| segment.is_empty() || segment == "." || segment == "..")
}

fn resolve_safe_working_dir(
    working_dir: &Path,
    default_working_dir: &Path,
) -> Result<PathBuf, PresetLoadError> {
    let candidate = if working_dir.as_os_str().is_empty() {
        default_working_dir.to_path_buf()
    } else if working_dir.is_absolute() {
        working_dir.to_path_buf()
    } else {
        default_working_dir.join(working_dir)
    };

    let canonical = candidate.canonicalize().map_err(|error| {
        PresetLoadError::Validation(format!(
            "preset working directory must exist and be accessible: {} ({error})",
            candidate.display()
        ))
    })?;

    let metadata = fs::metadata(&canonical).map_err(|error| {
        PresetLoadError::Validation(format!(
            "failed to inspect preset working directory {}: {error}",
            canonical.display()
        ))
    })?;
    if !metadata.is_dir() {
        return Err(PresetLoadError::Validation(format!(
            "preset working directory must be a directory: {}",
            canonical.display()
        )));
    }

    let ctx_vault_segment = Path::new(CTX_HOME_DIR).join(crate::GLOBAL_VAULT_DIR);
    if canonical.ends_with(&ctx_vault_segment)
        || canonical
            .ancestors()
            .any(|ancestor| ancestor.ends_with(&ctx_vault_segment))
    {
        return Err(PresetLoadError::Validation(format!(
            "preset working directory cannot be inside a ctx vault: {}",
            canonical.display()
        )));
    }

    Ok(canonical)
}

#[cfg(test)]
mod tests {
    use super::{
        list_presets_from_resolved_overlay, load_preset_from_resolved_overlay, managed_presets_dir,
        save_preset_execution_settings, save_preset_subagent_manifest,
        validate_cli_execution_settings, validate_subagent_manifest, PresetLoadError,
        MAX_SUBAGENT_MANIFEST_JSON_BYTES,
    };
    use crate::{
        create_context_file, list_context_files, CliTarget, InjectionStrategy, VaultRoots,
        VaultScope, AGENTS_MD_FILE_NAME, CTX_END_MARKER, CTX_START_MARKER,
    };
    use crate::{
        HandoffConstraints, PresetExecutionSettingsUpdate, SubagentManifest,
        SubagentManifestUpdate, SubagentRole, SubagentSpawnGuidance,
    };
    use serde_json::json;
    use std::{collections::BTreeMap, fs, path::PathBuf};
    use uuid::Uuid;

    fn test_roots() -> (VaultRoots, PathBuf) {
        let base = std::env::temp_dir().join(format!("ctx-preset-test-{}", Uuid::new_v4()));
        let roots = VaultRoots {
            global_root: base.join("global"),
            local_root: Some(base.join("project").join(".ctx").join("vault")),
        };
        (roots, base)
    }

    #[test]
    fn loads_preset_against_resolved_overlay_contexts() {
        let (roots, base) = test_roots();
        let global = create_context_file(
            &roots,
            VaultScope::Global,
            "agents",
            "rules.md",
            "# Global Rules",
        )
        .expect("global context should be created");
        let local = create_context_file(
            &roots,
            VaultScope::Local,
            "agents",
            "rules.md",
            "# Local Rules",
        )
        .expect("local context should override");
        let presets_dir = managed_presets_dir(roots.global_root.as_path());
        fs::create_dir_all(&presets_dir).expect("preset dir should be created");
        fs::write(
            presets_dir.join("daily.json"),
            r#"{
                "preset_name": "Daily",
                "preset_target_cli": "codex",
                "preset_contexts": ["agents/rules.md"]
            }"#,
        )
        .expect("preset file should be writable");

        let contexts = list_context_files(&roots).expect("contexts should resolve as overlay");
        let loaded = load_preset_from_resolved_overlay(
            &roots,
            &contexts,
            "daily",
            CliTarget::Codex,
            PathBuf::from("/workspace").as_path(),
        )
        .expect("preset should load");

        assert_eq!(loaded.preset.preset_name, "Daily");
        assert_eq!(loaded.contexts.len(), 1);
        assert_eq!(loaded.contexts[0].file_path, local.file_path);
        assert_eq!(
            loaded.preset.preset_contexts,
            vec![loaded.contexts[0].context_id]
        );
        assert!(!loaded
            .contexts
            .iter()
            .any(|context| context.file_path == global.file_path));

        fs::remove_dir_all(base).ok();
    }

    #[test]
    fn local_preset_file_overrides_global_preset_file() {
        let (roots, base) = test_roots();
        create_context_file(&roots, VaultScope::Global, "", "global.md", "# Global")
            .expect("global context should be created");
        let local_context =
            create_context_file(&roots, VaultScope::Local, "", "local.md", "# Local")
                .expect("local context should be created");

        let global_presets = managed_presets_dir(roots.global_root.as_path());
        let local_presets = managed_presets_dir(roots.local_root.as_ref().unwrap());
        fs::create_dir_all(&global_presets).expect("global preset dir should be created");
        fs::create_dir_all(&local_presets).expect("local preset dir should be created");
        fs::write(
            global_presets.join("daily.json"),
            r#"{"preset_name":"Global Daily","preset_target_cli":"claude","preset_contexts":["global.md"]}"#,
        )
        .expect("global preset should be writable");
        fs::write(
            local_presets.join("daily.json"),
            r#"{"preset_name":"Local Daily","preset_target_cli":"claude","preset_contexts":["local.md"]}"#,
        )
        .expect("local preset should be writable");

        let contexts = list_context_files(&roots).expect("contexts should resolve");
        let loaded = load_preset_from_resolved_overlay(
            &roots,
            &contexts,
            "daily",
            CliTarget::Claude,
            PathBuf::from("/workspace").as_path(),
        )
        .expect("local preset should load");

        assert_eq!(loaded.vault_scope, VaultScope::Local);
        assert_eq!(loaded.preset.preset_name, "Local Daily");
        assert_eq!(loaded.contexts.len(), 1);
        assert_eq!(loaded.contexts[0].file_path, local_context.file_path);
        assert_eq!(
            loaded.preset.preset_contexts,
            vec![loaded.contexts[0].context_id]
        );

        fs::remove_dir_all(base).ok();
    }

    #[test]
    fn loads_preset_by_configured_preset_name_when_file_stem_differs() {
        let (roots, base) = test_roots();
        let context =
            create_context_file(&roots, VaultScope::Global, "agents", "rules.md", "# Rules")
                .expect("context should be created");
        let presets_dir = managed_presets_dir(roots.global_root.as_path());
        fs::create_dir_all(&presets_dir).expect("preset dir should be created");
        fs::write(
            presets_dir.join("daily-driver.json"),
            r#"{
                "preset_name": "Daily Driver",
                "preset_target_cli": "codex",
                "preset_contexts": ["agents/rules.md"]
            }"#,
        )
        .expect("preset file should be writable");

        let contexts = list_context_files(&roots).expect("contexts should resolve");
        let loaded = load_preset_from_resolved_overlay(
            &roots,
            &contexts,
            "daily driver",
            CliTarget::Codex,
            PathBuf::from("/workspace").as_path(),
        )
        .expect("preset should resolve by configured name");

        assert_eq!(loaded.preset.preset_name, "Daily Driver");
        assert_eq!(loaded.file_path, presets_dir.join("daily-driver.json"));
        assert_eq!(loaded.contexts[0].file_path, context.file_path);
        assert_eq!(loaded.contexts[0].content, "# Rules");

        fs::remove_dir_all(base).ok();
    }

    #[test]
    fn preset_file_stem_lookup_takes_precedence_over_duplicate_preset_name() {
        let (roots, base) = test_roots();
        create_context_file(&roots, VaultScope::Global, "", "first.md", "# First")
            .expect("first context should be created");
        create_context_file(&roots, VaultScope::Global, "", "second.md", "# Second")
            .expect("second context should be created");
        let presets_dir = managed_presets_dir(roots.global_root.as_path());
        fs::create_dir_all(&presets_dir).expect("preset dir should be created");
        fs::write(
            presets_dir.join("daily.json"),
            r#"{"preset_name":"Shared Name","preset_target_cli":"codex","preset_contexts":["first.md"]}"#,
        )
        .expect("daily preset should be writable");
        fs::write(
            presets_dir.join("shared-name.json"),
            r#"{"preset_name":"Shared Name","preset_target_cli":"codex","preset_contexts":["second.md"]}"#,
        )
        .expect("shared-name preset should be writable");

        let contexts = list_context_files(&roots).expect("contexts should resolve");
        let loaded = load_preset_from_resolved_overlay(
            &roots,
            &contexts,
            "daily",
            CliTarget::Codex,
            PathBuf::from("/workspace").as_path(),
        )
        .expect("file stem should resolve without ambiguity");

        assert_eq!(loaded.file_path, presets_dir.join("daily.json"));
        assert_eq!(loaded.preset.preset_name, "Shared Name");

        fs::remove_dir_all(base).ok();
    }

    #[test]
    fn load_preset_rejects_ambiguous_configured_preset_name() {
        let (roots, base) = test_roots();
        let presets_dir = managed_presets_dir(roots.global_root.as_path());
        fs::create_dir_all(&presets_dir).expect("preset dir should be created");
        fs::write(
            presets_dir.join("first.json"),
            r#"{"preset_name":"Duplicate Daily","preset_target_cli":"codex"}"#,
        )
        .expect("first preset should be writable");
        fs::write(
            presets_dir.join("second.json"),
            r#"{"preset_name":"Duplicate Daily","preset_target_cli":"codex"}"#,
        )
        .expect("second preset should be writable");

        let error = load_preset_from_resolved_overlay(
            &roots,
            &[],
            "duplicate daily",
            CliTarget::Codex,
            PathBuf::from("/workspace").as_path(),
        )
        .expect_err("duplicate preset names should be rejected");

        assert!(error.to_string().contains("preset name is ambiguous"));
        assert!(error.to_string().contains("first.json"));
        assert!(error.to_string().contains("second.json"));

        fs::remove_dir_all(base).ok();
    }

    #[test]
    fn load_preset_rejects_path_like_lookup_refs() {
        let (roots, base) = test_roots();

        let error = load_preset_from_resolved_overlay(
            &roots,
            &[],
            "../daily",
            CliTarget::Codex,
            PathBuf::from("/workspace").as_path(),
        )
        .expect_err("path-like preset refs should be rejected");

        assert!(error.to_string().contains("not a path"));

        fs::remove_dir_all(base).ok();
    }

    #[test]
    fn load_preset_reports_missing_refs_with_searched_dirs_and_available_presets() {
        let (roots, base) = test_roots();
        let presets_dir = managed_presets_dir(roots.global_root.as_path());
        fs::create_dir_all(&presets_dir).expect("preset dir should be created");
        fs::write(
            presets_dir.join("daily.json"),
            r#"{"preset_name":"Daily Driver","preset_target_cli":"codex"}"#,
        )
        .expect("preset file should be writable");

        let error = load_preset_from_resolved_overlay(
            &roots,
            &[],
            "missing",
            CliTarget::Codex,
            PathBuf::from("/workspace").as_path(),
        )
        .expect_err("missing preset refs should return a user-facing not-found error");

        match &error {
            PresetLoadError::NotFound {
                searched_dirs,
                available_presets,
                ..
            } => {
                assert!(searched_dirs
                    .iter()
                    .any(|path| path.ends_with(PathBuf::from("global").join("presets"))));
                assert_eq!(available_presets, &vec!["daily".to_string()]);
            }
            other => panic!("expected NotFound error, got {other:?}"),
        }
        let message = error.to_string();
        assert!(message.contains("preset 'missing' was not found"));
        assert!(message.contains("Available presets: daily"));
        assert!(message.contains("Create a preset JSON file"));

        fs::remove_dir_all(base).ok();
    }

    #[test]
    fn load_preset_reports_non_object_json_as_invalid_definition() {
        let (roots, base) = test_roots();
        let presets_dir = managed_presets_dir(roots.global_root.as_path());
        fs::create_dir_all(&presets_dir).expect("preset dir should be created");
        fs::write(presets_dir.join("bad.json"), r#"["not an object"]"#)
            .expect("preset file should be writable");

        let error = load_preset_from_resolved_overlay(
            &roots,
            &[],
            "bad",
            CliTarget::Codex,
            PathBuf::from("/workspace").as_path(),
        )
        .expect_err("invalid preset definitions should fail clearly");

        assert!(error.to_string().contains("invalid preset definition"));
        assert!(error
            .to_string()
            .contains("top-level JSON value must be an object"));

        fs::remove_dir_all(base).ok();
    }

    #[test]
    fn loads_preset_metadata_and_explicit_context_composition_order() {
        let (roots, base) = test_roots();
        let setup =
            create_context_file(&roots, VaultScope::Global, "agents", "setup.md", "# Setup")
                .expect("setup context should be created");
        let rules =
            create_context_file(&roots, VaultScope::Global, "agents", "rules.md", "# Rules")
                .expect("rules context should be created");
        let presets_dir = managed_presets_dir(roots.global_root.as_path());
        fs::create_dir_all(&presets_dir).expect("preset dir should be created");
        fs::write(
            presets_dir.join("review.json"),
            r#"{
                "preset_name": "Review",
                "preset_description": "Context pack for review sessions.",
                "preset_tags": ["review", "rust"],
                "preset_folder_path": "workflows",
                "preset_target_cli": "codex",
                "preset_contexts": [
                    {"context_ref": "agents/rules.md", "order": 20, "required": false},
                    {"context_ref": "agents/setup.md", "order": 10}
                ]
            }"#,
        )
        .expect("preset file should be writable");

        let contexts = list_context_files(&roots).expect("contexts should resolve as overlay");
        let loaded = load_preset_from_resolved_overlay(
            &roots,
            &contexts,
            "review",
            CliTarget::Codex,
            PathBuf::from("/workspace").as_path(),
        )
        .expect("preset should load");

        assert_eq!(
            loaded.preset.preset_metadata.description.as_deref(),
            Some("Context pack for review sessions.")
        );
        assert_eq!(loaded.preset.preset_metadata.tags, vec!["review", "rust"]);
        assert_eq!(
            loaded.preset.preset_metadata.folder_path,
            PathBuf::from("workflows")
        );
        assert_eq!(
            loaded.preset.preset_contexts,
            vec![setup.context_id, rules.context_id]
        );
        assert_eq!(
            loaded.preset.preset_context_composition[0].source_ref,
            "agents/setup.md"
        );
        assert_eq!(loaded.preset.preset_context_composition[0].order, 10);
        assert!(loaded.preset.preset_context_composition[0].required);
        assert_eq!(
            loaded.preset.preset_context_composition[0]
                .selection
                .selection_kind,
            crate::PresetContextSelectionKind::WholeFile
        );
        assert_eq!(
            loaded.preset.preset_context_composition[1].source_ref,
            "agents/rules.md"
        );
        assert_eq!(loaded.preset.preset_context_composition[1].order, 20);
        assert!(!loaded.preset.preset_context_composition[1].required);

        let summaries = list_presets_from_resolved_overlay(
            &roots,
            &contexts,
            PathBuf::from("/workspace").as_path(),
        )
        .expect("preset summaries should load");
        assert_eq!(
            summaries[0].preset_description.as_deref(),
            Some("Context pack for review sessions.")
        );
        assert_eq!(summaries[0].preset_tags, vec!["review", "rust"]);
        assert_eq!(summaries[0].preset_folder_path, PathBuf::from("workflows"));
        assert_eq!(summaries[0].preset_context_composition.len(), 2);

        fs::remove_dir_all(base).ok();
    }

    #[test]
    fn loads_preset_contexts_with_deterministic_order_tie_breakers() {
        let (roots, base) = test_roots();
        let beta = create_context_file(&roots, VaultScope::Global, "agents", "beta.md", "# Beta")
            .expect("beta context should be created");
        let alpha =
            create_context_file(&roots, VaultScope::Global, "agents", "alpha.md", "# Alpha")
                .expect("alpha context should be created");
        let gamma =
            create_context_file(&roots, VaultScope::Global, "agents", "gamma.md", "# Gamma")
                .expect("gamma context should be created");
        let presets_dir = managed_presets_dir(roots.global_root.as_path());
        fs::create_dir_all(&presets_dir).expect("preset dir should be created");
        fs::write(
            presets_dir.join("ties.json"),
            r#"{
                "preset_name": "Ties",
                "preset_target_cli": "codex",
                "preset_contexts": [
                    {"context_ref": "agents/beta.md", "order": 10},
                    {"context_ref": "agents/gamma.md", "order": 20},
                    {"context_ref": "agents/alpha.md", "order": 10}
                ]
            }"#,
        )
        .expect("preset file should be writable");

        let contexts = list_context_files(&roots).expect("contexts should resolve as overlay");
        let loaded = load_preset_from_resolved_overlay(
            &roots,
            &contexts,
            "ties",
            CliTarget::Codex,
            PathBuf::from("/workspace").as_path(),
        )
        .expect("preset should load");

        assert_eq!(
            loaded.preset.preset_contexts,
            vec![alpha.context_id, beta.context_id, gamma.context_id]
        );
        assert_eq!(
            loaded
                .preset
                .preset_context_composition
                .iter()
                .map(|composition| composition.source_ref.as_str())
                .collect::<Vec<_>>(),
            vec!["agents/alpha.md", "agents/beta.md", "agents/gamma.md"]
        );

        fs::remove_dir_all(base).ok();
    }

    #[test]
    fn loads_context_file_and_fragment_selection_inputs() {
        let (roots, base) = test_roots();
        let whole_file =
            create_context_file(&roots, VaultScope::Global, "agents", "rules.md", "# Rules")
                .expect("rules context should be created");
        let fragment =
            create_context_file(&roots, VaultScope::Global, "skills", "rust.md", "# Rust")
                .expect("rust context should be created");
        let presets_dir = managed_presets_dir(roots.global_root.as_path());
        fs::create_dir_all(&presets_dir).expect("preset dir should be created");
        fs::write(
            presets_dir.join("fragments.json"),
            r#"{
                "preset_name": "Fragments",
                "preset_target_cli": "codex",
                "preset_contexts": [
                    "agents/rules.md",
                    {
                        "context_ref": "skills/rust.md",
                        "order": 10,
                        "selection_kind": "heading",
                        "heading": "Build checks",
                        "include_children": true
                    },
                    {
                        "context_ref": "skills/rust.md",
                        "order": 20,
                        "selection": {
                            "selection_kind": "line-range",
                            "line_start": 4,
                            "line_end": 12
                        },
                        "required": false
                    }
                ]
            }"#,
        )
        .expect("preset file should be writable");

        let contexts = list_context_files(&roots).expect("contexts should resolve as overlay");
        let loaded = load_preset_from_resolved_overlay(
            &roots,
            &contexts,
            "fragments",
            CliTarget::Codex,
            PathBuf::from("/workspace").as_path(),
        )
        .expect("preset should load");

        assert_eq!(
            loaded.preset.preset_contexts,
            vec![
                whole_file.context_id,
                fragment.context_id,
                fragment.context_id
            ]
        );
        assert_eq!(
            loaded.preset.preset_context_composition[0]
                .selection
                .selection_kind,
            crate::PresetContextSelectionKind::WholeFile
        );
        assert_eq!(
            loaded.preset.preset_context_composition[1]
                .selection
                .selection_kind,
            crate::PresetContextSelectionKind::Heading
        );
        assert_eq!(
            loaded.preset.preset_context_composition[1]
                .selection
                .heading
                .as_deref(),
            Some("Build checks")
        );
        assert!(
            loaded.preset.preset_context_composition[1]
                .selection
                .include_children
        );
        assert_eq!(
            loaded.preset.preset_context_composition[2]
                .selection
                .selection_kind,
            crate::PresetContextSelectionKind::LineRange
        );
        assert_eq!(
            loaded.preset.preset_context_composition[2]
                .selection
                .line_start,
            Some(4)
        );
        assert_eq!(
            loaded.preset.preset_context_composition[2]
                .selection
                .line_end,
            Some(12)
        );
        assert!(!loaded.preset.preset_context_composition[2].required);

        fs::remove_dir_all(base).ok();
    }

    #[test]
    fn list_presets_exposes_cli_execution_and_wrapper_schema_fields() {
        let (roots, base) = test_roots();
        let rules =
            create_context_file(&roots, VaultScope::Global, "agents", "rules.md", "# Rules")
                .expect("rules context should be created");
        let presets_dir = managed_presets_dir(roots.global_root.as_path());
        let state_dir = base.join("state");
        let working_dir = base.join("workspace");
        fs::create_dir_all(&presets_dir).expect("preset dir should be created");
        fs::write(
            presets_dir.join("implementation.json"),
            format!(
                r#"{{
                    "preset_name": "Implementation",
                    "preset_contexts": ["agents/rules.md"],
                    "cli_execution_settings": {{
                        "target_cli": "codex",
                        "working_dir": "{}",
                        "model": "gpt-5.3-codex",
                        "passthrough_args": ["--sandbox", "workspace-write"]
                    }},
                    "wrapper_behavior": {{
                        "injection_strategy": "agents-md-section-marker-merge",
                        "cleanup_on_exit": true,
                        "cleanup_stale_on_launch": false,
                        "state_dir": "{}"
                    }}
                }}"#,
                working_dir.display(),
                state_dir.display()
            ),
        )
        .expect("preset file should be writable");

        let contexts = list_context_files(&roots).expect("contexts should resolve as overlay");
        assert_eq!(contexts[0].context_id, rules.context_id);
        let summaries = list_presets_from_resolved_overlay(
            &roots,
            &contexts,
            PathBuf::from("/fallback").as_path(),
        )
        .expect("preset summary should include schema fields");

        let summary = &summaries[0];
        assert_eq!(summary.preset_target_cli, CliTarget::Codex);
        assert_eq!(summary.cli_execution_settings.target_cli, CliTarget::Codex);
        assert_eq!(summary.cli_execution_settings.working_dir, working_dir);
        assert_eq!(
            summary.cli_execution_settings.model.as_deref(),
            Some("gpt-5.3-codex")
        );
        assert_eq!(
            summary.cli_execution_settings.passthrough_args,
            vec!["--sandbox", "workspace-write"]
        );
        assert_eq!(
            summary.wrapper_behavior.injection_strategy,
            InjectionStrategy::AgentsMdSectionMarkerMerge
        );
        assert!(summary.wrapper_behavior.cleanup_on_exit);
        assert!(!summary.wrapper_behavior.cleanup_stale_on_launch);
        assert_eq!(summary.wrapper_behavior.state_dir, state_dir);
        assert_eq!(
            summary.wrapper_behavior.start_marker.as_deref(),
            Some(CTX_START_MARKER)
        );
        assert_eq!(
            summary.wrapper_behavior.end_marker.as_deref(),
            Some(CTX_END_MARKER)
        );
        assert_eq!(
            summary.wrapper_behavior.agents_md_path,
            Some(
                summary
                    .cli_execution_settings
                    .working_dir
                    .join(AGENTS_MD_FILE_NAME)
            )
        );

        fs::remove_dir_all(base).ok();
    }

    #[test]
    fn load_preset_exposes_subagent_manifest_and_handoff_constraints() {
        let (roots, base) = test_roots();
        create_context_file(
            &roots,
            VaultScope::Global,
            "subagents",
            "reviewer.md",
            "# Reviewer",
        )
        .expect("subagent context should be created");
        let presets_dir = managed_presets_dir(roots.global_root.as_path());
        fs::create_dir_all(&presets_dir).expect("preset dir should be created");
        fs::write(
            presets_dir.join("delegated-review.json"),
            r#"{
                "preset_name": "Delegated Review",
                "preset_target_cli": "codex",
                "preset_contexts": ["subagents/reviewer.md"],
                "subagent_manifest": {
                    "manifest_version": "1",
                    "roles": [
                        {
                            "id": "reviewer",
                            "name": "Reviewer",
                            "role": "Code review subagent",
                            "capabilities": ["correctness review", "risk identification"],
                            "constraints": ["Return findings with file and line references."],
                            "metadata": {"owner": "quality"},
                            "description": "Find correctness risks before handoff.",
                            "assigned_contexts": ["subagents/reviewer.md"],
                            "spawn_instructions": [
                                "Inspect the patch for behavioral regressions.",
                                "Return findings with file and line references."
                            ],
                            "spawn_guidance": {
                                "select_when": [
                                    "Use after implementation changes are ready for independent review."
                                ],
                                "avoid_when": [
                                    "Avoid when the task still needs code edits or repository exploration."
                                ],
                                "delegation_prompt": "Review changed files and return findings first."
                            },
                            "handoff_targets": ["implementer"],
                            "model": "gpt-5.3-codex"
                        }
                    ],
                    "handoff_constraints": {
                        "require_summary": true,
                        "require_changed_files": true,
                        "require_open_questions": false,
                        "max_parallel_subagents": 2,
                        "allowed_handoff_targets": ["implementer"],
                        "blocked_handoff_targets": ["release"],
                        "handoff_prompt_template": "Summarize findings before returning control."
                    }
                }
            }"#,
        )
        .expect("preset file should be writable");

        let contexts = list_context_files(&roots).expect("contexts should resolve as overlay");
        let loaded = load_preset_from_resolved_overlay(
            &roots,
            &contexts,
            "delegated-review",
            CliTarget::Codex,
            PathBuf::from("/workspace").as_path(),
        )
        .expect("preset should load");
        let summaries = list_presets_from_resolved_overlay(
            &roots,
            &contexts,
            PathBuf::from("/workspace").as_path(),
        )
        .expect("preset summaries should load");

        let manifest = loaded
            .preset
            .subagent_manifest
            .expect("manifest should be typed");
        assert_eq!(manifest.manifest_version.as_deref(), Some("1"));
        assert_eq!(manifest.roles.len(), 1);
        assert_eq!(manifest.roles[0].role_id, "reviewer");
        assert_eq!(manifest.roles[0].role_name, "Reviewer");
        assert_eq!(
            manifest.roles[0].assigned_contexts,
            vec!["subagents/reviewer.md"]
        );
        assert_eq!(manifest.roles[0].spawn_instructions.len(), 2);
        assert_eq!(
            manifest.roles[0].spawn_guidance.select_when,
            vec!["Use after implementation changes are ready for independent review."]
        );
        assert_eq!(
            manifest.roles[0].spawn_guidance.avoid_when,
            vec!["Avoid when the task still needs code edits or repository exploration."]
        );
        assert_eq!(
            manifest.roles[0]
                .spawn_guidance
                .delegation_prompt
                .as_deref(),
            Some("Review changed files and return findings first.")
        );
        assert_eq!(manifest.roles[0].handoff_targets, vec!["implementer"]);
        assert_eq!(manifest.roles[0].model.as_deref(), Some("gpt-5.3-codex"));
        assert!(manifest.handoff_constraints.require_summary);
        assert!(manifest.handoff_constraints.require_changed_files);
        assert!(!manifest.handoff_constraints.require_open_questions);
        assert_eq!(manifest.handoff_constraints.max_parallel_subagents, Some(2));
        assert_eq!(
            manifest.handoff_constraints.allowed_handoff_targets,
            vec!["implementer"]
        );
        assert_eq!(
            manifest.handoff_constraints.blocked_handoff_targets,
            vec!["release"]
        );
        assert_eq!(
            manifest
                .handoff_constraints
                .handoff_prompt_template
                .as_deref(),
            Some("Summarize findings before returning control.")
        );
        assert_eq!(
            summaries[0]
                .subagent_manifest
                .as_ref()
                .expect("summary should expose manifest")
                .roles[0]
                .role_id,
            "reviewer"
        );

        fs::remove_dir_all(base).ok();
    }

    #[test]
    fn load_preset_accepts_legacy_top_level_execution_fields() {
        let (roots, base) = test_roots();
        create_context_file(&roots, VaultScope::Global, "agents", "rules.md", "# Rules")
            .expect("rules context should be created");
        let presets_dir = managed_presets_dir(roots.global_root.as_path());
        fs::create_dir_all(&presets_dir).expect("preset dir should be created");
        fs::write(
            presets_dir.join("legacy.json"),
            r#"{
                "preset_name": "Legacy",
                "preset_contexts": ["agents/rules.md"],
                "preset_target_cli": "claude",
                "preset_working_dir": "/legacy/workspace",
                "preset_model": "claude-sonnet"
            }"#,
        )
        .expect("preset file should be writable");

        let contexts = list_context_files(&roots).expect("contexts should resolve as overlay");
        let loaded = load_preset_from_resolved_overlay(
            &roots,
            &contexts,
            "legacy",
            CliTarget::Claude,
            PathBuf::from("/fallback").as_path(),
        )
        .expect("legacy preset should load");
        let summaries = list_presets_from_resolved_overlay(
            &roots,
            &contexts,
            PathBuf::from("/fallback").as_path(),
        )
        .expect("legacy preset summary should load");

        assert_eq!(loaded.preset.preset_target_cli, CliTarget::Claude);
        assert_eq!(
            loaded.preset.preset_working_dir,
            PathBuf::from("/legacy/workspace")
        );
        assert_eq!(loaded.preset.preset_model.as_deref(), Some("claude-sonnet"));
        assert_eq!(
            summaries[0].wrapper_behavior.injection_strategy,
            InjectionStrategy::AppendSystemPromptFile
        );
        assert!(summaries[0].wrapper_behavior.agents_md_path.is_none());
        assert!(summaries[0].wrapper_behavior.prompt_file_dir.is_some());

        fs::remove_dir_all(base).ok();
    }

    #[test]
    fn save_preset_execution_settings_persists_validated_cli_fields() {
        let (roots, base) = test_roots();
        let workspace = base.join("workspace");
        fs::create_dir_all(&workspace).expect("workspace should exist");

        let summary = save_preset_execution_settings(
            &roots,
            PresetExecutionSettingsUpdate {
                preset_ref: "daily".to_string(),
                preset_name: Some("Daily Driver".to_string()),
                vault_scope: VaultScope::Local,
                target_cli: CliTarget::Codex,
                working_dir: workspace.clone(),
                model: Some(" codex ".to_string()),
                passthrough_args: vec!["--sandbox".to_string(), " ".to_string()],
            },
            &base,
        )
        .expect("valid execution settings should persist");

        assert_eq!(summary.preset_name, "Daily Driver");
        assert_eq!(summary.preset_target_cli, CliTarget::Codex);
        assert_eq!(summary.cli_execution_settings.working_dir, workspace);
        assert_eq!(
            summary.cli_execution_settings.model.as_deref(),
            Some("codex")
        );
        assert_eq!(
            summary.cli_execution_settings.passthrough_args,
            vec!["--sandbox"]
        );
        assert_eq!(
            summary.wrapper_behavior.injection_strategy,
            InjectionStrategy::AgentsMdSectionMarkerMerge
        );

        let content = fs::read_to_string(summary.file_path).expect("preset should be written");
        assert!(content.contains("\"cli_execution_settings\""));
        assert!(content.contains("\"working_dir\""));

        fs::remove_dir_all(base).ok();
    }

    #[test]
    fn save_preset_subagent_manifest_persists_normalized_schema() {
        let (roots, base) = test_roots();
        let presets_dir = managed_presets_dir(roots.local_root.as_ref().unwrap());
        fs::create_dir_all(&presets_dir).expect("preset dir should be created");
        fs::write(
            presets_dir.join("delegated-review.json"),
            r#"{
                "preset_name": "Delegated Review",
                "preset_target_cli": "codex",
                "preset_contexts": ["subagents/reviewer.md"]
            }"#,
        )
        .expect("preset file should be writable");

        let summary = save_preset_subagent_manifest(
            &roots,
            SubagentManifestUpdate {
                preset_ref: "delegated-review".to_string(),
                preset_name: None,
                vault_scope: VaultScope::Local,
                manifest: Some(SubagentManifest {
                    manifest_version: None,
                    roles: vec![SubagentRole {
                        role_id: " reviewer ".to_string(),
                        role_name: " Reviewer ".to_string(),
                        role: " Code review subagent ".to_string(),
                        capabilities: vec![" correctness review ".to_string(), "".to_string()],
                        constraints: vec![" Return findings first. ".to_string()],
                        metadata: BTreeMap::from([
                            (" owner ".to_string(), " quality ".to_string()),
                            (" ".to_string(), "ignored".to_string()),
                        ]),
                        description: Some(" ".to_string()),
                        assigned_contexts: vec![
                            " subagents/reviewer.md ".to_string(),
                            " ".to_string(),
                        ],
                        spawn_instructions: vec![
                            " Inspect the patch. ".to_string(),
                            "".to_string(),
                        ],
                        spawn_guidance: SubagentSpawnGuidance {
                            select_when: vec![
                                " Use when implementation is complete and needs review. "
                                    .to_string(),
                                "".to_string(),
                            ],
                            avoid_when: vec![
                                " Avoid when the task still needs repository exploration. "
                                    .to_string(),
                            ],
                            delegation_prompt: Some(
                                " Review changed files and return findings first. ".to_string(),
                            ),
                        },
                        handoff_targets: vec![" implementer ".to_string()],
                        model: Some(" gpt-5.3-codex ".to_string()),
                    }],
                    handoff_constraints: HandoffConstraints {
                        require_summary: true,
                        require_changed_files: true,
                        require_open_questions: false,
                        max_parallel_subagents: Some(2),
                        allowed_handoff_targets: vec![" implementer ".to_string()],
                        blocked_handoff_targets: vec![" ".to_string()],
                        handoff_prompt_template: Some(
                            " Summarize work before returning. ".to_string(),
                        ),
                    },
                }),
            },
            &base,
        )
        .expect("valid subagent manifest should persist");

        let manifest = summary
            .subagent_manifest
            .expect("summary should expose persisted manifest");
        assert_eq!(manifest.manifest_version.as_deref(), Some("1"));
        assert_eq!(manifest.roles[0].role_id, "reviewer");
        assert_eq!(manifest.roles[0].role_name, "Reviewer");
        assert_eq!(manifest.roles[0].role, "Code review subagent");
        assert_eq!(manifest.roles[0].capabilities, vec!["correctness review"]);
        assert_eq!(
            manifest.roles[0].constraints,
            vec!["Return findings first."]
        );
        assert_eq!(
            manifest.roles[0].metadata.get("owner").map(String::as_str),
            Some("quality")
        );
        assert_eq!(
            manifest.roles[0].assigned_contexts,
            vec!["subagents/reviewer.md"]
        );
        assert_eq!(
            manifest.roles[0].spawn_instructions,
            vec!["Inspect the patch."]
        );
        assert_eq!(
            manifest.roles[0].spawn_guidance.select_when,
            vec!["Use when implementation is complete and needs review."]
        );
        assert_eq!(
            manifest.roles[0].spawn_guidance.avoid_when,
            vec!["Avoid when the task still needs repository exploration."]
        );
        assert_eq!(
            manifest.roles[0]
                .spawn_guidance
                .delegation_prompt
                .as_deref(),
            Some("Review changed files and return findings first.")
        );
        assert_eq!(manifest.roles[0].handoff_targets, vec!["implementer"]);
        assert_eq!(manifest.roles[0].model.as_deref(), Some("gpt-5.3-codex"));
        assert_eq!(
            manifest
                .handoff_constraints
                .handoff_prompt_template
                .as_deref(),
            Some("Summarize work before returning.")
        );

        let persisted = fs::read_to_string(summary.file_path).expect("preset should be readable");
        assert!(persisted.contains("\"subagent_manifest\""));
        assert!(persisted.contains("\"manifest_version\": \"1\""));
        assert!(persisted.contains("\"preset_contexts\""));

        fs::remove_dir_all(base).ok();
    }

    #[test]
    fn saved_subagent_manifest_round_trips_with_composed_preset_contexts() {
        let (roots, base) = test_roots();
        let reviewer = create_context_file(
            &roots,
            VaultScope::Local,
            "subagents",
            "reviewer.md",
            "# Reviewer\n\nReview correctness risks.",
        )
        .expect("reviewer context should be created");
        let shared = create_context_file(
            &roots,
            VaultScope::Local,
            "shared",
            "rules.md",
            "# Rules\n\nUse repository conventions.",
        )
        .expect("shared context should be created");
        let presets_dir = managed_presets_dir(roots.local_root.as_ref().unwrap());
        fs::create_dir_all(&presets_dir).expect("preset dir should be created");
        fs::write(
            presets_dir.join("delegated-review.json"),
            r#"{
                "preset_name": "Delegated Review",
                "preset_target_cli": "codex",
                "preset_contexts": [
                    {"context_ref": "shared/rules.md", "order": 10},
                    {"context_ref": "subagents/reviewer.md", "order": 20}
                ]
            }"#,
        )
        .expect("preset file should be writable");

        save_preset_subagent_manifest(
            &roots,
            SubagentManifestUpdate {
                preset_ref: "delegated-review".to_string(),
                preset_name: None,
                vault_scope: VaultScope::Local,
                manifest: Some(SubagentManifest {
                    manifest_version: None,
                    roles: vec![SubagentRole {
                        role_id: "reviewer".to_string(),
                        role_name: "Reviewer".to_string(),
                        role: "Code review subagent".to_string(),
                        capabilities: vec!["correctness review".to_string()],
                        constraints: vec!["Stay within the assigned context.".to_string()],
                        metadata: BTreeMap::from([("owner".to_string(), "quality".to_string())]),
                        description: None,
                        assigned_contexts: vec!["subagents/reviewer.md".to_string()],
                        spawn_instructions: vec!["Review the composed preset.".to_string()],
                        spawn_guidance: SubagentSpawnGuidance {
                            select_when: vec![
                                "Use when composed context needs independent review.".to_string(),
                            ],
                            avoid_when: vec![
                                "Avoid when implementation changes are still in progress."
                                    .to_string(),
                            ],
                            delegation_prompt: Some(
                                "Review the composed preset and summarize risks.".to_string(),
                            ),
                        },
                        handoff_targets: Vec::new(),
                        model: None,
                    }],
                    handoff_constraints: HandoffConstraints::default(),
                }),
            },
            &base,
        )
        .expect("valid subagent manifest should persist");

        let contexts = list_context_files(&roots).expect("contexts should resolve");
        let loaded = load_preset_from_resolved_overlay(
            &roots,
            &contexts,
            "delegated-review",
            CliTarget::Codex,
            base.as_path(),
        )
        .expect("preset should reload with manifest and context composition");

        assert_eq!(
            loaded.preset.preset_contexts,
            vec![shared.context_id, reviewer.context_id]
        );
        assert_eq!(
            loaded
                .preset
                .preset_context_composition
                .iter()
                .map(|composition| composition.source_ref.as_str())
                .collect::<Vec<_>>(),
            vec!["shared/rules.md", "subagents/reviewer.md"]
        );
        let manifest = loaded
            .preset
            .subagent_manifest
            .expect("loaded preset should include saved manifest");
        assert_eq!(manifest.manifest_version.as_deref(), Some("1"));
        assert_eq!(
            manifest.roles[0].assigned_contexts,
            vec!["subagents/reviewer.md"]
        );
        assert_eq!(loaded.contexts.len(), 2);

        fs::remove_dir_all(base).ok();
    }

    #[test]
    fn save_preset_subagent_manifest_rejects_duplicate_role_ids() {
        let (roots, base) = test_roots();

        let error = save_preset_subagent_manifest(
            &roots,
            SubagentManifestUpdate {
                preset_ref: "bad-manifest".to_string(),
                preset_name: None,
                vault_scope: VaultScope::Global,
                manifest: Some(SubagentManifest {
                    manifest_version: Some("1".to_string()),
                    roles: vec![
                        SubagentRole {
                            role_id: "reviewer".to_string(),
                            role_name: "Reviewer".to_string(),
                            role: "Code review subagent".to_string(),
                            capabilities: vec!["correctness review".to_string()],
                            constraints: vec!["Stay within the assigned context.".to_string()],
                            metadata: BTreeMap::new(),
                            description: None,
                            assigned_contexts: Vec::new(),
                            spawn_instructions: Vec::new(),
                            spawn_guidance: SubagentSpawnGuidance::default(),
                            handoff_targets: Vec::new(),
                            model: None,
                        },
                        SubagentRole {
                            role_id: " reviewer ".to_string(),
                            role_name: "Second Reviewer".to_string(),
                            role: "Second code review subagent".to_string(),
                            capabilities: vec!["correctness review".to_string()],
                            constraints: vec!["Stay within the assigned context.".to_string()],
                            metadata: BTreeMap::new(),
                            description: None,
                            assigned_contexts: Vec::new(),
                            spawn_instructions: Vec::new(),
                            spawn_guidance: SubagentSpawnGuidance::default(),
                            handoff_targets: Vec::new(),
                            model: None,
                        },
                    ],
                    handoff_constraints: HandoffConstraints::default(),
                }),
            },
            &base,
        )
        .expect_err("duplicate role IDs should fail validation");

        assert!(error.to_string().contains("duplicate id: reviewer"));
        fs::remove_dir_all(base).ok();
    }

    #[test]
    fn validate_subagent_manifest_reports_all_schema_errors() {
        let error = validate_subagent_manifest(SubagentManifest {
            manifest_version: Some("2".to_string()),
            roles: vec![SubagentRole {
                role_id: "bad role".to_string(),
                role_name: " ".to_string(),
                role: " ".to_string(),
                capabilities: vec![" ".to_string()],
                constraints: vec![" ".to_string()],
                metadata: BTreeMap::from([
                    (" owner ".to_string(), " ".to_string()),
                    (" ".to_string(), "ignored".to_string()),
                ]),
                description: Some(" ".to_string()),
                assigned_contexts: vec!["../secrets.md".to_string()],
                spawn_instructions: vec![" ".to_string()],
                spawn_guidance: SubagentSpawnGuidance {
                    select_when: vec![" ".to_string()],
                    avoid_when: vec![" ".to_string()],
                    delegation_prompt: Some(" ".to_string()),
                },
                handoff_targets: vec!["handoff target".to_string()],
                model: Some(" ".to_string()),
            }],
            handoff_constraints: HandoffConstraints {
                require_summary: true,
                require_changed_files: true,
                require_open_questions: true,
                max_parallel_subagents: Some(0),
                allowed_handoff_targets: vec!["reviewer".to_string(), "bad target".to_string()],
                blocked_handoff_targets: vec!["reviewer".to_string()],
                handoff_prompt_template: Some(" ".to_string()),
            },
        })
        .expect_err("invalid manifest should report schema errors");

        let message = error.to_string();
        assert!(message.starts_with("invalid subagent_manifest:"));
        assert!(message.contains("manifest_version must be \"1\""));
        assert!(message.contains("roles[0] (bad role).id may only contain"));
        assert!(message.contains("roles[0] (bad role).name must be non-empty"));
        assert!(message.contains("roles[0] (bad role).role must be non-empty"));
        assert!(message.contains("capabilities must include at least one capability"));
        assert!(message.contains("constraints must include at least one constraint"));
        assert!(message.contains("assigned_contexts contains unsafe context ref: ../secrets.md"));
        assert!(message.contains("spawn_instructions must include at least one instruction"));
        assert!(message.contains("spawn_guidance.select_when must explain when to select"));
        assert!(message.contains("spawn_guidance.avoid_when must explain when to avoid"));
        assert!(message.contains("handoff_targets contains invalid role ref: handoff target"));
        assert!(message.contains("max_parallel_subagents must be greater than 0"));
        assert!(message.contains("allowed_handoff_targets contains invalid role ref: bad target"));
        assert!(message.contains("cannot both allow and block handoff target: reviewer"));
    }

    #[test]
    fn validate_subagent_manifest_reports_missing_required_role_fields() {
        let manifest: SubagentManifest = serde_json::from_value(json!({
            "manifest_version": "1",
            "roles": [
                {
                    "role": "Code review subagent",
                    "capabilities": ["correctness review"],
                    "constraints": ["Return findings first."],
                    "assigned_contexts": ["subagents/reviewer.md"],
                    "spawn_instructions": ["Review the active patch."],
                    "spawn_guidance": {
                        "select_when": ["Use after implementation."],
                        "avoid_when": ["Avoid before code exists."]
                    }
                }
            ]
        }))
        .expect("missing role id/name should deserialize into validation defaults");

        let error = validate_subagent_manifest(manifest)
            .expect_err("manifest missing role id/name should fail validation");
        let message = error.to_string();

        assert!(message.contains("subagent_manifest.roles[0].id must be non-empty"));
        assert!(message.contains("subagent_manifest.roles[0].name must be non-empty"));
    }

    #[test]
    fn load_preset_rejects_oversized_raw_subagent_manifest_from_disk() {
        let (roots, base) = test_roots();
        let presets_dir = managed_presets_dir(roots.global_root.as_path());
        fs::create_dir_all(&presets_dir).expect("preset dir should be created");
        let oversized_padding = "x".repeat(MAX_SUBAGENT_MANIFEST_JSON_BYTES + 1);
        fs::write(
            presets_dir.join("oversized.json"),
            format!(
                r#"{{
                    "preset_name": "Oversized Manifest",
                    "preset_target_cli": "codex",
                    "subagent_manifest": {{
                        "manifest_version": "1",
                        "padding": "{oversized_padding}"
                    }}
                }}"#
            ),
        )
        .expect("preset file should be writable");

        let error =
            load_preset_from_resolved_overlay(&roots, &[], "oversized", CliTarget::Codex, &base)
                .expect_err("oversized raw manifest should be rejected before launch");

        assert!(error.to_string().contains("invalid subagent_manifest"));
        assert!(error.to_string().contains("exceeding the"));
        assert!(error.to_string().contains("byte launch limit"));
    }

    #[test]
    fn load_preset_rejects_non_object_subagent_manifest_from_disk() {
        let (roots, base) = test_roots();
        let presets_dir = managed_presets_dir(roots.global_root.as_path());
        fs::create_dir_all(&presets_dir).expect("preset dir should be created");
        fs::write(
            presets_dir.join("bad-type.json"),
            r#"{
                "preset_name": "Bad Manifest Type",
                "preset_target_cli": "codex",
                "subagent_manifest": "subagents/reviewer.md"
            }"#,
        )
        .expect("preset file should be writable");

        let error =
            load_preset_from_resolved_overlay(&roots, &[], "bad-type", CliTarget::Codex, &base)
                .expect_err("non-object manifest should be rejected before launch");

        assert!(error.to_string().contains("invalid subagent_manifest"));
        assert!(error.to_string().contains("expected a JSON object or null"));
    }

    #[test]
    fn validate_subagent_manifest_rejects_empty_role_set() {
        let error = validate_subagent_manifest(SubagentManifest {
            manifest_version: None,
            roles: Vec::new(),
            handoff_constraints: HandoffConstraints::default(),
        })
        .expect_err("manifest with no roles should fail validation");

        assert!(error
            .to_string()
            .contains("roles must include at least one delegated role"));
    }

    #[test]
    fn list_presets_rejects_invalid_subagent_manifest_from_disk() {
        let (roots, base) = test_roots();
        let presets_dir = managed_presets_dir(roots.global_root.as_path());
        fs::create_dir_all(&presets_dir).expect("preset dir should be created");
        fs::write(
            presets_dir.join("bad-delegation.json"),
            r#"{
                "preset_name": "Bad Delegation",
                "preset_target_cli": "codex",
                "subagent_manifest": {
                    "manifest_version": "1",
                    "roles": [
                        {
                            "id": "reviewer",
                            "name": "Reviewer",
                            "role": "",
                            "capabilities": [],
                            "constraints": [],
                            "assigned_contexts": ["../secrets.md"],
                            "spawn_instructions": []
                        }
                    ]
                }
            }"#,
        )
        .expect("preset file should be writable");

        let error = list_presets_from_resolved_overlay(&roots, &[], &base)
            .expect_err("invalid persisted manifest should fail preset listing");

        assert!(error.to_string().contains("invalid subagent_manifest:"));
        assert!(error
            .to_string()
            .contains("assigned_contexts contains unsafe context ref: ../secrets.md"));
        assert!(error
            .to_string()
            .contains("spawn_instructions must include at least one instruction"));

        fs::remove_dir_all(base).ok();
    }

    #[test]
    fn validate_cli_execution_settings_rejects_missing_file_and_ctx_vault_paths() {
        let (_roots, base) = test_roots();
        fs::create_dir_all(&base).expect("base should exist");

        let file_path = base.join("not-a-dir.md");
        fs::write(&file_path, "# Not a directory").expect("file should be writable");
        let file_error =
            validate_cli_execution_settings(CliTarget::Claude, &file_path, None, Vec::new(), &base)
                .expect_err("file path should not be accepted as a working directory");
        assert!(file_error
            .to_string()
            .contains("preset working directory must be a directory"));

        let vault_dir = base.join(".ctx").join("vault").join("nested");
        fs::create_dir_all(&vault_dir).expect("vault dir should exist");
        let vault_error =
            validate_cli_execution_settings(CliTarget::Codex, &vault_dir, None, Vec::new(), &base)
                .expect_err("ctx vault paths should not be launch working directories");
        assert!(vault_error
            .to_string()
            .contains("cannot be inside a ctx vault"));

        fs::remove_dir_all(base).ok();
    }
}
