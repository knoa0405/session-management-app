use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, path::PathBuf};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppStatus {
    pub name: String,
    pub version: String,
    pub vault_ready: bool,
    pub sqlite_index_ready: bool,
    pub wrapper_ready: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Eq, PartialEq, Ord, PartialOrd)]
#[serde(rename_all = "kebab-case")]
pub enum VaultScope {
    Global,
    Local,
}

#[derive(Debug, Clone, Serialize, Deserialize, Eq, PartialEq, Hash, Ord, PartialOrd)]
pub struct VaultEntryKey {
    pub relative_path: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum Classification {
    MainAgent,
    Subagent,
    Shared,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum ClassificationStatus {
    Pending,
    Classified,
    Reviewed,
    Modified,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum ImportSourceType {
    ContextMarkdown,
    ClaudeMarkdown,
    CodexAgents,
    AgentMarkdown,
    AgentsManifest,
    SkillMarkdown,
    SkillManifest,
    SubagentMarkdown,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum CliTarget {
    Claude,
    Codex,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum InjectionStrategy {
    AppendSystemPromptFile,
    AgentsMdSectionMarkerMerge,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextFragment {
    pub context_id: Uuid,
    pub title: String,
    pub content: String,
    pub file_path: PathBuf,
    pub vault_scope: VaultScope,
    pub classification: Classification,
    #[serde(default)]
    pub import_classification_suggestion: Option<Classification>,
    #[serde(default)]
    pub inferred_classification: Option<Classification>,
    pub tags: Vec<String>,
    pub folder_path: PathBuf,
    pub wikilinks: Vec<String>,
    pub backlinks: Vec<String>,
    pub import_source: Option<PathBuf>,
    #[serde(default)]
    pub import_source_type: Option<ImportSourceType>,
    pub llm_classification_status: ClassificationStatus,
    #[serde(default)]
    pub session_handoff_classification: Option<SessionHandoffClassificationMetadata>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SessionHandoffClassificationMetadata {
    pub source_tool: String,
    pub source_session_ref: String,
    pub source_working_directory: String,
    pub source_log_path: String,
    pub work_context_category: String,
    #[serde(default)]
    pub work_context_categories: Vec<String>,
    pub work_context_classification_status: ClassificationStatus,
    pub work_context_confidence_score: u8,
    pub work_context_rationale: String,
    #[serde(default)]
    pub distillation_focus: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Eq, PartialEq)]
pub struct ContextDiscoveryMetadata {
    pub title: String,
    pub vault_scope: VaultScope,
    pub classification: Classification,
    #[serde(default)]
    pub import_classification_suggestion: Option<Classification>,
    #[serde(default)]
    pub inferred_classification: Option<Classification>,
    pub tags: Vec<String>,
    pub folder_path: PathBuf,
    pub wikilinks: Vec<String>,
    pub llm_classification_status: ClassificationStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize, Eq, PartialEq)]
pub struct ContextDiscoveryResult {
    pub file_path: PathBuf,
    pub file_name: String,
    pub root_source: PathBuf,
    pub source_type: ImportSourceType,
    pub metadata: ContextDiscoveryMetadata,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, Eq, PartialEq)]
pub struct PresetMetadata {
    pub description: Option<String>,
    pub tags: Vec<String>,
    pub folder_path: PathBuf,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum PresetContextSelectionKind {
    WholeFile,
    Heading,
    LineRange,
    Anchor,
}

fn default_preset_context_selection_kind() -> PresetContextSelectionKind {
    PresetContextSelectionKind::WholeFile
}

#[derive(Debug, Clone, Serialize, Deserialize, Eq, PartialEq)]
pub struct PresetContextSelection {
    #[serde(default = "default_preset_context_selection_kind")]
    pub selection_kind: PresetContextSelectionKind,
    pub heading: Option<String>,
    pub anchor: Option<String>,
    pub line_start: Option<u32>,
    pub line_end: Option<u32>,
    #[serde(default)]
    pub include_children: bool,
}

impl Default for PresetContextSelection {
    fn default() -> Self {
        Self {
            selection_kind: PresetContextSelectionKind::WholeFile,
            heading: None,
            anchor: None,
            line_start: None,
            line_end: None,
            include_children: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Eq, PartialEq)]
pub struct PresetContextSelectionInput {
    pub context_ref: String,
    #[serde(default)]
    pub selection: PresetContextSelection,
    #[serde(default = "default_true")]
    pub required: bool,
    pub order: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Eq, PartialEq)]
pub struct PresetContextComposition {
    pub context_id: Uuid,
    pub order: usize,
    pub source_ref: String,
    pub required: bool,
    pub selection: PresetContextSelection,
}

#[derive(Debug, Clone, Serialize, Deserialize, Eq, PartialEq)]
pub struct ResolvedContextItem {
    pub context_id: Uuid,
    pub title: String,
    pub source_ref: String,
    pub file_path: PathBuf,
    pub vault_scope: VaultScope,
    pub selection: PresetContextSelection,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Eq, PartialEq)]
pub struct CliExecutionSettings {
    pub target_cli: CliTarget,
    pub working_dir: PathBuf,
    pub model: Option<String>,
    pub passthrough_args: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Eq, PartialEq)]
pub struct PresetExecutionSettingsUpdate {
    pub preset_ref: String,
    pub preset_name: Option<String>,
    pub vault_scope: VaultScope,
    pub target_cli: CliTarget,
    pub working_dir: PathBuf,
    pub model: Option<String>,
    #[serde(default)]
    pub passthrough_args: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Eq, PartialEq)]
pub struct SubagentManifestUpdate {
    pub preset_ref: String,
    pub preset_name: Option<String>,
    pub vault_scope: VaultScope,
    pub manifest: Option<SubagentManifest>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Eq, PartialEq)]
pub struct WrapperBehavior {
    pub injection_strategy: InjectionStrategy,
    pub cleanup_on_exit: bool,
    pub cleanup_stale_on_launch: bool,
    pub state_dir: PathBuf,
    pub start_marker: Option<String>,
    pub end_marker: Option<String>,
    pub agents_md_path: Option<PathBuf>,
    pub prompt_file_dir: Option<PathBuf>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, Eq, PartialEq)]
pub struct SubagentManifest {
    pub manifest_version: Option<String>,
    #[serde(default)]
    pub roles: Vec<SubagentRole>,
    #[serde(default)]
    pub handoff_constraints: HandoffConstraints,
}

#[derive(Debug, Clone, Serialize, Deserialize, Eq, PartialEq)]
pub struct SubagentRole {
    #[serde(rename = "id", alias = "role_id")]
    #[serde(default)]
    pub role_id: String,
    #[serde(rename = "name", alias = "role_name")]
    #[serde(default)]
    pub role_name: String,
    #[serde(default)]
    pub role: String,
    #[serde(default)]
    pub capabilities: Vec<String>,
    #[serde(default)]
    pub constraints: Vec<String>,
    #[serde(default)]
    pub metadata: BTreeMap<String, String>,
    pub description: Option<String>,
    #[serde(default)]
    pub assigned_contexts: Vec<String>,
    #[serde(default)]
    pub spawn_instructions: Vec<String>,
    #[serde(default)]
    pub spawn_guidance: SubagentSpawnGuidance,
    #[serde(default)]
    pub handoff_targets: Vec<String>,
    pub model: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, Eq, PartialEq)]
pub struct SubagentSpawnGuidance {
    #[serde(default)]
    pub select_when: Vec<String>,
    #[serde(default)]
    pub avoid_when: Vec<String>,
    pub delegation_prompt: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Eq, PartialEq)]
pub struct HandoffConstraints {
    #[serde(default = "default_true")]
    pub require_summary: bool,
    #[serde(default = "default_true")]
    pub require_changed_files: bool,
    #[serde(default = "default_true")]
    pub require_open_questions: bool,
    pub max_parallel_subagents: Option<u16>,
    #[serde(default)]
    pub allowed_handoff_targets: Vec<String>,
    #[serde(default)]
    pub blocked_handoff_targets: Vec<String>,
    pub handoff_prompt_template: Option<String>,
}

fn default_true() -> bool {
    true
}

impl Default for HandoffConstraints {
    fn default() -> Self {
        Self {
            require_summary: true,
            require_changed_files: true,
            require_open_questions: true,
            max_parallel_subagents: None,
            allowed_handoff_targets: Vec::new(),
            blocked_handoff_targets: Vec::new(),
            handoff_prompt_template: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn context_fragment_deserializes_legacy_payload_without_inferred_classification() {
        let payload = json!({
            "context_id": Uuid::new_v4(),
            "title": "Legacy Context",
            "content": "# Legacy",
            "file_path": "/tmp/legacy.md",
            "vault_scope": "local",
            "classification": "shared",
            "tags": [],
            "folder_path": "",
            "wikilinks": [],
            "backlinks": [],
            "import_source": null,
            "llm_classification_status": "pending"
        });

        let context: ContextFragment =
            serde_json::from_value(payload).expect("legacy context payload should deserialize");

        assert_eq!(context.classification, Classification::Shared);
        assert_eq!(context.inferred_classification, None);
        assert_eq!(context.import_source_type, None);
        assert_eq!(context.session_handoff_classification, None);
    }

    #[test]
    fn context_fragment_serializes_inferred_classification() {
        let context = ContextFragment {
            context_id: Uuid::new_v4(),
            title: "Agent Context".to_string(),
            content: "# Agent".to_string(),
            file_path: PathBuf::from("/tmp/agent.md"),
            vault_scope: VaultScope::Local,
            classification: Classification::Shared,
            import_classification_suggestion: Some(Classification::MainAgent),
            inferred_classification: Some(Classification::MainAgent),
            tags: Vec::new(),
            folder_path: PathBuf::new(),
            wikilinks: Vec::new(),
            backlinks: Vec::new(),
            import_source: None,
            import_source_type: None,
            llm_classification_status: ClassificationStatus::Classified,
            session_handoff_classification: None,
        };

        let value = serde_json::to_value(context).expect("context should serialize");

        assert_eq!(value["classification"], "shared");
        assert_eq!(value["import_classification_suggestion"], "main-agent");
        assert_eq!(value["inferred_classification"], "main-agent");
    }

    #[test]
    fn subagent_role_serializes_manifest_entry_schema() {
        let role = SubagentRole {
            role_id: "reviewer".to_string(),
            role_name: "Reviewer".to_string(),
            role: "Code review subagent".to_string(),
            capabilities: vec!["correctness review".to_string()],
            constraints: vec!["Return findings first.".to_string()],
            metadata: BTreeMap::from([("owner".to_string(), "quality".to_string())]),
            description: None,
            assigned_contexts: vec!["subagents/reviewer.md".to_string()],
            spawn_instructions: vec!["Inspect changed files.".to_string()],
            spawn_guidance: SubagentSpawnGuidance {
                select_when: vec![
                    "Use after implementation changes are ready for review.".to_string()
                ],
                avoid_when: vec!["Avoid for tasks that still need code edits.".to_string()],
                delegation_prompt: Some(
                    "Review changed files and return findings first.".to_string(),
                ),
            },
            handoff_targets: Vec::new(),
            model: None,
        };

        let value = serde_json::to_value(role).expect("role should serialize");

        assert_eq!(value["id"], "reviewer");
        assert_eq!(value["name"], "Reviewer");
        assert_eq!(value["role"], "Code review subagent");
        assert_eq!(value["capabilities"], json!(["correctness review"]));
        assert_eq!(value["constraints"], json!(["Return findings first."]));
        assert_eq!(value["metadata"]["owner"], "quality");
        assert_eq!(
            value["spawn_guidance"]["select_when"],
            json!(["Use after implementation changes are ready for review."])
        );
        assert_eq!(
            value["spawn_guidance"]["avoid_when"],
            json!(["Avoid for tasks that still need code edits."])
        );
        assert!(value.get("role_id").is_none());
        assert!(value.get("role_name").is_none());
    }

    #[test]
    fn subagent_role_deserializes_legacy_role_id_and_role_name() {
        let payload = json!({
            "role_id": "reviewer",
            "role_name": "Reviewer",
            "role": "Code review subagent",
            "capabilities": ["correctness review"],
            "constraints": ["Return findings first."],
            "metadata": {"owner": "quality"},
            "assigned_contexts": ["subagents/reviewer.md"],
            "spawn_instructions": ["Inspect changed files."]
        });

        let role: SubagentRole =
            serde_json::from_value(payload).expect("legacy role keys should deserialize");

        assert_eq!(role.role_id, "reviewer");
        assert_eq!(role.role_name, "Reviewer");
        assert_eq!(role.role, "Code review subagent");
        assert_eq!(role.capabilities, vec!["correctness review"]);
        assert_eq!(role.constraints, vec!["Return findings first."]);
        assert_eq!(
            role.metadata.get("owner").map(String::as_str),
            Some("quality")
        );
        assert!(role.spawn_guidance.select_when.is_empty());
        assert!(role.spawn_guidance.avoid_when.is_empty());
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Preset {
    pub preset_id: Uuid,
    pub preset_name: String,
    pub preset_contexts: Vec<Uuid>,
    pub preset_metadata: PresetMetadata,
    pub preset_context_composition: Vec<PresetContextComposition>,
    pub preset_target_cli: CliTarget,
    pub preset_working_dir: PathBuf,
    pub preset_model: Option<String>,
    pub subagent_manifest: Option<SubagentManifest>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InjectionMarkers {
    pub preset_name: String,
    pub session_id: Uuid,
    pub start_marker: String,
    pub end_marker: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionRecord {
    pub session_id: Uuid,
    pub session_preset_id: Uuid,
    pub session_pid: u32,
    pub session_status: SessionStatus,
    pub cli_execution_settings: Option<CliExecutionSettings>,
    pub wrapper_behavior: Option<WrapperBehavior>,
    pub injection_markers: Option<InjectionMarkers>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum SessionStatus {
    Active,
    Terminated,
    Orphaned,
}
