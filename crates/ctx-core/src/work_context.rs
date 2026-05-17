use crate::{
    models::{ClassificationStatus, CliTarget, InjectionStrategy},
    session_logs::{SessionLogDetail, SessionLogProvider},
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const WORK_CONTEXT_CLASSIFICATION_OUTPUT_FORMAT_VERSION: u8 = 1;
pub const SESSION_HANDOFF_CONTEXT_OUTPUT_FORMAT_VERSION: u8 = 1;
pub const MAX_HANDOFF_MARKDOWN_CHARS: usize = 12_000;
pub const MAX_HANDOFF_MARKDOWN_LINES: usize = 220;
pub const MAX_HANDOFF_SUMMARY_CHARS: usize = 500;
pub const MAX_HANDOFF_SIGNAL_CHARS: usize = 500;
pub const MAX_HANDOFF_PARAGRAPH_CHARS: usize = 1_200;
pub const MAX_HANDOFF_GOALS: usize = 3;
pub const MAX_HANDOFF_CHANGED_FILES: usize = 12;
pub const MAX_HANDOFF_COMMANDS: usize = 8;
pub const MAX_HANDOFF_DECISIONS: usize = 8;
pub const MAX_HANDOFF_VERIFICATION_RESULTS: usize = 8;
pub const MAX_HANDOFF_REMAINING_WORK: usize = 8;
pub const SESSION_HANDOFF_CONTEXT_REQUIRED_MVP_FIELDS: &[&str] = &[
    "source_tool",
    "source_session_ref",
    "source_working_directory",
    "title",
    "summary",
    "key_changed_files",
    "decisions",
    "verification_results",
    "remaining_work",
    "created_at",
    "handoff_markdown",
    "tags",
    "cleanup_applied",
    "refine_mode",
    "launch_target",
    "injection_method",
];
pub const SESSION_HANDOFF_CONTEXT_REQUIRED_FRONTMATTER_FIELDS: &[&str] = &[
    "session_handoff_format_version",
    "classification",
    "tags",
    "source_tool",
    "source_session_ref",
    "source_working_directory",
    "source_log_path",
    "source_updated_at",
    "title",
    "work_context_category",
    "work_context_categories",
    "work_context_classification_status",
    "work_context_confidence_score",
    "work_context_rationale",
    "goals",
    "summary",
    "key_changed_files",
    "commands",
    "decisions",
    "verification_results",
    "remaining_work",
    "created_at",
    "cleanup_applied",
    "refine_mode",
    "launch_target",
    "injection_method",
    "distillation_focus",
];

#[derive(Debug, Clone, Copy, Serialize, Eq, PartialEq)]
pub struct SessionHandoffContextFieldDefinition {
    pub name: &'static str,
    pub description: &'static str,
    pub required: bool,
    pub validation_rule: &'static str,
}

pub const SESSION_HANDOFF_CONTEXT_MVP_SCHEMA: &[SessionHandoffContextFieldDefinition] = &[
    SessionHandoffContextFieldDefinition {
        name: "source_tool",
        description: "Original session log provider.",
        required: true,
        validation_rule: "must be claude or codex",
    },
    SessionHandoffContextFieldDefinition {
        name: "source_session_ref",
        description: "Original provider session ID or stable file-derived reference.",
        required: true,
        validation_rule: "non-empty scalar",
    },
    SessionHandoffContextFieldDefinition {
        name: "source_working_directory",
        description: "Working directory captured from the original session log.",
        required: true,
        validation_rule: "non-empty scalar",
    },
    SessionHandoffContextFieldDefinition {
        name: "title",
        description: "Human-readable title for the work session.",
        required: true,
        validation_rule: "non-empty scalar",
    },
    SessionHandoffContextFieldDefinition {
        name: "summary",
        description: "Concise summary of completed work.",
        required: true,
        validation_rule: "non-empty scalar, <= MAX_HANDOFF_SUMMARY_CHARS, represented in handoff_markdown",
    },
    SessionHandoffContextFieldDefinition {
        name: "key_changed_files",
        description: "Important files changed or discussed in the session.",
        required: true,
        validation_rule: "normalized list, <= MAX_HANDOFF_CHANGED_FILES entries, each represented in handoff_markdown",
    },
    SessionHandoffContextFieldDefinition {
        name: "decisions",
        description: "Important decisions made during the session.",
        required: true,
        validation_rule: "normalized list, <= MAX_HANDOFF_DECISIONS entries, each represented in handoff_markdown",
    },
    SessionHandoffContextFieldDefinition {
        name: "verification_results",
        description: "Tests, checks, or validation results captured from the session.",
        required: true,
        validation_rule: "normalized list, <= MAX_HANDOFF_VERIFICATION_RESULTS entries, each represented in handoff_markdown",
    },
    SessionHandoffContextFieldDefinition {
        name: "remaining_work",
        description: "Known follow-up tasks or unresolved items.",
        required: true,
        validation_rule: "normalized list, <= MAX_HANDOFF_REMAINING_WORK entries, each represented in handoff_markdown",
    },
    SessionHandoffContextFieldDefinition {
        name: "created_at",
        description: "Timestamp when the reusable handoff entry was generated.",
        required: true,
        validation_rule: "non-empty scalar",
    },
    SessionHandoffContextFieldDefinition {
        name: "handoff_markdown",
        description: "Launch-ready markdown body injected into a future Claude or Codex session.",
        required: true,
        validation_rule: "non-empty markdown, <= MAX_HANDOFF_MARKDOWN_CHARS and MAX_HANDOFF_MARKDOWN_LINES, readable/actionable with future-session sections",
    },
    SessionHandoffContextFieldDefinition {
        name: "tags",
        description: "Labels for search and organization.",
        required: true,
        validation_rule: "normalized non-empty list",
    },
    SessionHandoffContextFieldDefinition {
        name: "cleanup_applied",
        description: "Whether sensitive information and transcript noise were cleaned.",
        required: true,
        validation_rule: "boolean",
    },
    SessionHandoffContextFieldDefinition {
        name: "refine_mode",
        description: "Whether the context is raw or refined.",
        required: true,
        validation_rule: "must be raw or refined",
    },
    SessionHandoffContextFieldDefinition {
        name: "launch_target",
        description: "Target CLI for launching with this saved handoff context.",
        required: true,
        validation_rule: "must be claude or codex",
    },
    SessionHandoffContextFieldDefinition {
        name: "injection_method",
        description: "Tool-specific automatic injection mechanism.",
        required: true,
        validation_rule: "must match launch_target: claude uses append-system-prompt-file; codex uses agents-md-section-marker-merge",
    },
];

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Eq, PartialEq, Hash, Ord, PartialOrd)]
#[serde(rename_all = "kebab-case")]
pub enum WorkContextCategory {
    Implementation,
    Debugging,
    Review,
    Planning,
    Refactor,
    Research,
    Verification,
    Launch,
    General,
}

impl WorkContextCategory {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Implementation => "implementation",
            Self::Debugging => "debugging",
            Self::Review => "review",
            Self::Planning => "planning",
            Self::Refactor => "refactor",
            Self::Research => "research",
            Self::Verification => "verification",
            Self::Launch => "launch",
            Self::General => "general",
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub struct WorkContextCategoryDefinition {
    pub category: WorkContextCategory,
    pub label: &'static str,
    pub description: &'static str,
    pub distillation_focus: &'static [&'static str],
}

pub const WORK_CONTEXT_CATEGORY_TAXONOMY: &[WorkContextCategoryDefinition] = &[
    WorkContextCategoryDefinition {
        category: WorkContextCategory::Implementation,
        label: "Implementation",
        description: "Feature, integration, or product behavior work that changed or added code.",
        distillation_focus: &[
            "implemented behavior",
            "key changed files",
            "design decisions",
            "verification results",
            "remaining integration work",
        ],
    },
    WorkContextCategoryDefinition {
        category: WorkContextCategory::Debugging,
        label: "Debugging",
        description: "Bug investigation, failure analysis, or corrective fixes.",
        distillation_focus: &[
            "symptoms",
            "root cause",
            "fix applied",
            "regression checks",
            "remaining risks",
        ],
    },
    WorkContextCategoryDefinition {
        category: WorkContextCategory::Review,
        label: "Review",
        description: "Code review, QA review, risk assessment, or regression analysis.",
        distillation_focus: &[
            "findings",
            "affected files",
            "risk rationale",
            "recommended follow-up",
            "verification gaps",
        ],
    },
    WorkContextCategoryDefinition {
        category: WorkContextCategory::Planning,
        label: "Planning",
        description: "Requirements, architecture, task planning, or execution strategy.",
        distillation_focus: &[
            "requirements",
            "constraints",
            "chosen plan",
            "open questions",
            "next implementation steps",
        ],
    },
    WorkContextCategoryDefinition {
        category: WorkContextCategory::Refactor,
        label: "Refactor",
        description:
            "Structure, cleanup, rename, or maintainability work without primary feature scope.",
        distillation_focus: &[
            "refactored surface",
            "behavior preservation decisions",
            "changed files",
            "verification results",
            "migration follow-up",
        ],
    },
    WorkContextCategoryDefinition {
        category: WorkContextCategory::Research,
        label: "Research",
        description: "Exploration, source inspection, technical discovery, or option comparison.",
        distillation_focus: &[
            "question investigated",
            "sources inspected",
            "conclusions",
            "decisions informed",
            "unresolved unknowns",
        ],
    },
    WorkContextCategoryDefinition {
        category: WorkContextCategory::Verification,
        label: "Verification",
        description:
            "Testing, build validation, reproduction checks, or release-readiness verification.",
        distillation_focus: &[
            "commands run",
            "results",
            "coverage",
            "failures",
            "remaining validation",
        ],
    },
    WorkContextCategoryDefinition {
        category: WorkContextCategory::Launch,
        label: "Launch",
        description: "CLI/session launch, context injection, cleanup, or handoff execution work.",
        distillation_focus: &[
            "launch target",
            "injection method",
            "cleanup behavior",
            "safety checks",
            "remaining launch gaps",
        ],
    },
    WorkContextCategoryDefinition {
        category: WorkContextCategory::General,
        label: "General",
        description: "Session work without a stronger supported category signal.",
        distillation_focus: &[
            "summary",
            "important decisions",
            "changed files",
            "verification results",
            "remaining work",
        ],
    },
];

#[derive(Debug, Clone, Serialize, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub struct WorkContextClassificationResult {
    pub output_format_version: u8,
    pub source_tool: SessionLogProvider,
    pub source_session_ref: String,
    pub source_working_directory: String,
    pub source_log_path: String,
    pub title: String,
    pub category: WorkContextCategory,
    pub categories: Vec<WorkContextCategory>,
    pub status: ClassificationStatus,
    pub confidence_score: u8,
    pub rationale: String,
    pub signal_counts: WorkContextSignalCounts,
    pub suggested_tags: Vec<String>,
    pub distillation_focus: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub struct WorkContextSignalCounts {
    pub goals: usize,
    pub summaries: usize,
    pub changed_files: usize,
    pub commands: usize,
    pub decisions: usize,
    pub verification_results: usize,
    pub remaining_work: usize,
    pub tags: usize,
    pub sensitive_content: usize,
    pub noise: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub struct WorkContextSignalSet {
    pub source_tool: SessionLogProvider,
    pub source_session_ref: String,
    pub source_working_directory: String,
    pub source_log_path: String,
    pub title: String,
    pub updated_at: Option<String>,
    pub message_count: usize,
    pub signals: Vec<WorkContextSignal>,
}

impl WorkContextSignalSet {
    pub fn from_session_detail(detail: &SessionLogDetail) -> Result<Self, WorkContextSchemaError> {
        let source_tool = detail
            .summary
            .provider_kind()
            .ok_or_else(|| WorkContextSchemaError {
                message: format!(
                    "unsupported session log provider for work-context signals: {}",
                    detail.summary.provider
                ),
            })?;

        Ok(Self {
            source_tool,
            source_session_ref: detail.summary.source_session_ref().to_string(),
            source_working_directory: detail
                .summary
                .source_working_directory()
                .unwrap_or_default()
                .to_string(),
            source_log_path: detail.summary.file_path.display().to_string(),
            title: detail.summary.title.clone(),
            updated_at: detail.summary.updated_at.clone(),
            message_count: detail.summary.message_count,
            signals: extract_work_context_signals(detail),
        })
    }

    pub fn classify(&self) -> WorkContextClassificationResult {
        classify_work_context_signals(self)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum WorkContextSignalKind {
    Goal,
    Summary,
    ChangedFile,
    Command,
    Decision,
    VerificationResult,
    RemainingWork,
    Tag,
    SensitiveContent,
    Noise,
}

#[derive(Debug, Clone, Serialize, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub struct WorkContextSignal {
    pub kind: WorkContextSignalKind,
    pub value: String,
    pub confidence: u8,
    #[serde(default)]
    pub evidence: Vec<WorkContextSignalEvidence>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub struct WorkContextSignalEvidence {
    pub message_index: usize,
    pub role: String,
    pub timestamp: Option<String>,
    pub excerpt: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub struct WorkContextFilteredContent {
    pub source_tool: SessionLogProvider,
    pub source_session_ref: String,
    pub source_working_directory: String,
    pub source_log_path: String,
    pub title: String,
    pub included_records: Vec<WorkContextFilteredRecord>,
    pub excluded_records: Vec<WorkContextFilteredRecord>,
    pub signal_counts: WorkContextSignalCounts,
    pub cleanup_applied: bool,
    pub handoff_markdown: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub struct WorkContextFilteredRecord {
    pub message_index: usize,
    pub role: String,
    pub timestamp: Option<String>,
    pub content: String,
    pub relevance_score: u8,
    pub reasons: Vec<WorkContextFilterReason>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Eq, PartialEq, Hash, Ord, PartialOrd)]
#[serde(rename_all = "kebab-case")]
pub enum WorkContextFilterReason {
    UserRequest,
    Summary,
    ChangedFile,
    Decision,
    VerificationResult,
    RemainingWork,
    WorkKeyword,
    SensitiveContent,
    Noise,
    ConversationChatter,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum WorkContextRefineMode {
    Raw,
    Refined,
}

#[derive(Debug, Clone, Serialize, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub struct DistilledSessionHandoffFields {
    pub goals: Vec<String>,
    pub summary: String,
    pub key_changed_files: Vec<String>,
    pub commands: Vec<String>,
    pub decisions: Vec<String>,
    pub verification_results: Vec<String>,
    pub remaining_work: Vec<String>,
    pub tags: Vec<String>,
    pub cleanup_applied: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub struct SessionHandoffContext {
    pub source_tool: SessionLogProvider,
    pub source_session_ref: String,
    pub source_working_directory: String,
    pub source_log_path: String,
    pub source_updated_at: Option<String>,
    pub title: String,
    pub category: WorkContextCategory,
    pub categories: Vec<WorkContextCategory>,
    pub classification_status: ClassificationStatus,
    pub classification_confidence_score: u8,
    pub classification_rationale: String,
    pub goals: Vec<String>,
    pub summary: String,
    pub key_changed_files: Vec<String>,
    pub commands: Vec<String>,
    pub decisions: Vec<String>,
    pub verification_results: Vec<String>,
    pub remaining_work: Vec<String>,
    pub created_at: String,
    pub handoff_markdown: String,
    pub tags: Vec<String>,
    pub cleanup_applied: bool,
    pub refine_mode: WorkContextRefineMode,
    pub launch_target: CliTarget,
    pub injection_method: InjectionStrategy,
}

impl SessionHandoffContext {
    pub fn from_session_detail(
        detail: &SessionLogDetail,
        created_at: impl Into<String>,
        launch_target: CliTarget,
        refine_mode: WorkContextRefineMode,
    ) -> Result<Self, WorkContextSchemaError> {
        let signal_set = WorkContextSignalSet::from_session_detail(detail)?;
        let filtered_content = filter_work_relevant_content(detail)?;
        let classification = signal_set.classify();

        Self::from_classified_signals_with_overlay(
            &signal_set,
            &classification,
            created_at,
            filtered_content.handoff_markdown,
            launch_target,
            refine_mode,
            false,
        )
    }

    pub fn from_classified_signals(
        signal_set: &WorkContextSignalSet,
        classification: &WorkContextClassificationResult,
        created_at: impl Into<String>,
        handoff_markdown: impl Into<String>,
        launch_target: CliTarget,
        refine_mode: WorkContextRefineMode,
    ) -> Result<Self, WorkContextSchemaError> {
        Self::from_classified_signals_with_overlay(
            signal_set,
            classification,
            created_at,
            handoff_markdown,
            launch_target,
            refine_mode,
            true,
        )
    }

    fn from_classified_signals_with_overlay(
        signal_set: &WorkContextSignalSet,
        classification: &WorkContextClassificationResult,
        created_at: impl Into<String>,
        handoff_markdown: impl Into<String>,
        launch_target: CliTarget,
        refine_mode: WorkContextRefineMode,
        overlay_markdown_fields: bool,
    ) -> Result<Self, WorkContextSchemaError> {
        let mut fields = extract_distilled_session_handoff_fields(signal_set, classification)?;
        let handoff_markdown = handoff_markdown.into();
        if overlay_markdown_fields {
            overlay_distilled_markdown_fields(&mut fields, &handoff_markdown);
        }
        normalize_distilled_session_handoff_fields(&mut fields)?;

        Ok(Self {
            source_tool: signal_set.source_tool,
            source_session_ref: signal_set.source_session_ref.clone(),
            source_working_directory: signal_set.source_working_directory.clone(),
            source_log_path: signal_set.source_log_path.clone(),
            source_updated_at: signal_set.updated_at.clone(),
            title: signal_set.title.clone(),
            category: classification.category,
            categories: classification.categories.clone(),
            classification_status: classification.status,
            classification_confidence_score: classification.confidence_score,
            classification_rationale: classification.rationale.clone(),
            goals: fields.goals,
            summary: fields.summary,
            key_changed_files: fields.key_changed_files,
            commands: fields.commands,
            decisions: fields.decisions,
            verification_results: fields.verification_results,
            remaining_work: fields.remaining_work,
            created_at: created_at.into(),
            handoff_markdown,
            tags: fields.tags,
            cleanup_applied: fields.cleanup_applied,
            refine_mode,
            launch_target,
            injection_method: injection_method_for_launch_target(launch_target),
        })
    }

    pub fn from_signals(
        signal_set: &WorkContextSignalSet,
        created_at: impl Into<String>,
        handoff_markdown: impl Into<String>,
        launch_target: CliTarget,
        refine_mode: WorkContextRefineMode,
    ) -> Self {
        let classification = signal_set.classify();
        Self::from_classified_signals(
            signal_set,
            &classification,
            created_at,
            handoff_markdown,
            launch_target,
            refine_mode,
        )
        .expect("classification generated from this signal set must match source metadata")
    }

    pub fn validate_for_save(&self) -> Result<(), WorkContextSchemaError> {
        let normalized = self.normalized_for_save()?;
        let mut missing = Vec::new();

        for (field, value) in [
            ("source_session_ref", normalized.source_session_ref.as_str()),
            (
                "source_working_directory",
                normalized.source_working_directory.as_str(),
            ),
            ("source_log_path", normalized.source_log_path.as_str()),
            ("title", normalized.title.as_str()),
            ("summary", normalized.summary.as_str()),
            ("created_at", normalized.created_at.as_str()),
            ("handoff_markdown", normalized.handoff_markdown.as_str()),
        ] {
            if value.trim().is_empty() {
                missing.push(field);
            }
        }

        if normalized.tags.iter().all(|tag| tag.trim().is_empty()) {
            missing.push("tags");
        }

        let expected_injection_method =
            injection_method_for_launch_target(normalized.launch_target);
        if normalized.injection_method != expected_injection_method {
            return Err(WorkContextSchemaError {
                message: format!(
                    "session handoff injection_method {} does not match launch_target {}",
                    injection_strategy_label(normalized.injection_method),
                    cli_target_label(normalized.launch_target)
                ),
            });
        }

        if normalized.classification_status == ClassificationStatus::Pending {
            return Err(WorkContextSchemaError {
                message: "session handoff context must be classified before saving".to_string(),
            });
        }

        if !missing.is_empty() {
            return Err(WorkContextSchemaError {
                message: format!(
                    "session handoff context is missing required save field(s): {}",
                    missing.join(", ")
                ),
            });
        }

        normalized.validate_concision_limits()?;
        normalized.validate_structured_fields()?;
        normalized.validate_essential_context_preserved()?;
        normalized.validate_readable_handoff_markdown()?;

        Ok(())
    }

    fn normalized_for_save(&self) -> Result<Self, WorkContextSchemaError> {
        let mut normalized = self.clone();
        normalized.summary = normalize_freeform_structured_value(&normalized.summary);
        normalized.source_session_ref =
            normalize_freeform_structured_value(&normalized.source_session_ref);
        normalized.source_working_directory =
            normalize_freeform_structured_value(&normalized.source_working_directory);
        normalized.source_log_path =
            normalize_freeform_structured_value(&normalized.source_log_path);
        normalized.title = normalize_freeform_structured_value(&normalized.title);
        normalized.created_at = normalize_freeform_structured_value(&normalized.created_at);
        normalized.classification_rationale =
            normalize_freeform_structured_value(&normalized.classification_rationale);
        normalized.goals = normalize_freeform_list(&normalized.goals, "goals")?;
        normalized.key_changed_files =
            normalize_changed_file_list(&normalized.key_changed_files, "key_changed_files")?;
        normalized.commands = normalize_freeform_list(&normalized.commands, "commands")?;
        normalized.decisions = normalize_freeform_list(&normalized.decisions, "decisions")?;
        normalized.verification_results =
            normalize_freeform_list(&normalized.verification_results, "verification_results")?;
        normalized.remaining_work =
            normalize_freeform_list(&normalized.remaining_work, "remaining_work")?;
        normalized.tags = normalize_tag_list(&normalized.tags, "tags")?;
        Ok(normalized)
    }

    fn validate_concision_limits(&self) -> Result<(), WorkContextSchemaError> {
        let mut violations = Vec::new();
        let handoff_chars = char_count(&self.handoff_markdown);
        if handoff_chars > MAX_HANDOFF_MARKDOWN_CHARS {
            violations.push(format!(
                "handoff_markdown has {handoff_chars} chars, exceeding {MAX_HANDOFF_MARKDOWN_CHARS}"
            ));
        }
        let handoff_lines = self.handoff_markdown.lines().count();
        if handoff_lines > MAX_HANDOFF_MARKDOWN_LINES {
            violations.push(format!(
                "handoff_markdown has {handoff_lines} lines, exceeding {MAX_HANDOFF_MARKDOWN_LINES}"
            ));
        }
        push_length_violation(
            &mut violations,
            "summary",
            &self.summary,
            MAX_HANDOFF_SUMMARY_CHARS,
        );
        push_list_length_violations(
            &mut violations,
            "goals",
            &self.goals,
            MAX_HANDOFF_SIGNAL_CHARS,
        );
        push_list_length_violations(
            &mut violations,
            "key_changed_files",
            &self.key_changed_files,
            MAX_HANDOFF_SIGNAL_CHARS,
        );
        push_list_length_violations(
            &mut violations,
            "commands",
            &self.commands,
            MAX_HANDOFF_SIGNAL_CHARS,
        );
        push_list_length_violations(
            &mut violations,
            "decisions",
            &self.decisions,
            MAX_HANDOFF_SIGNAL_CHARS,
        );
        push_list_length_violations(
            &mut violations,
            "verification_results",
            &self.verification_results,
            MAX_HANDOFF_SIGNAL_CHARS,
        );
        push_list_length_violations(
            &mut violations,
            "remaining_work",
            &self.remaining_work,
            MAX_HANDOFF_SIGNAL_CHARS,
        );

        if violations.is_empty() {
            Ok(())
        } else {
            Err(WorkContextSchemaError {
                message: format!(
                    "session handoff context exceeds concision limit(s): {}",
                    violations.join("; ")
                ),
            })
        }
    }

    fn validate_essential_context_preserved(&self) -> Result<(), WorkContextSchemaError> {
        let mut missing = Vec::new();
        push_missing_markdown_value(
            &mut missing,
            &self.handoff_markdown,
            "summary",
            &self.summary,
        );
        push_missing_markdown_values(
            &mut missing,
            &self.handoff_markdown,
            "goals",
            &self.goals,
            MAX_HANDOFF_GOALS,
        );
        push_missing_markdown_values(
            &mut missing,
            &self.handoff_markdown,
            "key_changed_files",
            &self.key_changed_files,
            MAX_HANDOFF_CHANGED_FILES,
        );
        push_missing_markdown_values(
            &mut missing,
            &self.handoff_markdown,
            "commands",
            &self.commands,
            MAX_HANDOFF_COMMANDS,
        );
        push_missing_markdown_values(
            &mut missing,
            &self.handoff_markdown,
            "decisions",
            &self.decisions,
            MAX_HANDOFF_DECISIONS,
        );
        push_missing_markdown_values(
            &mut missing,
            &self.handoff_markdown,
            "verification_results",
            &self.verification_results,
            MAX_HANDOFF_VERIFICATION_RESULTS,
        );
        push_missing_markdown_values(
            &mut missing,
            &self.handoff_markdown,
            "remaining_work",
            &self.remaining_work,
            MAX_HANDOFF_REMAINING_WORK,
        );

        if missing.is_empty() {
            Ok(())
        } else {
            Err(WorkContextSchemaError {
                message: format!(
                    "session handoff context lost essential distilled field(s): {}",
                    missing.join(", ")
                ),
            })
        }
    }

    fn validate_readable_handoff_markdown(&self) -> Result<(), WorkContextSchemaError> {
        let body = strip_saved_handoff_frontmatter(&self.handoff_markdown).trim();
        let lines = body.lines().collect::<Vec<_>>();
        let non_empty_lines = lines
            .iter()
            .map(|line| line.trim())
            .filter(|line| !line.is_empty())
            .collect::<Vec<_>>();
        let heading_count = non_empty_lines
            .iter()
            .filter(|line| line.starts_with("# "))
            .count();
        let section_heading_count = non_empty_lines
            .iter()
            .filter(|line| line.starts_with("## ") || line.starts_with("### "))
            .count();
        let bullet_count = non_empty_lines
            .iter()
            .filter(|line| line.starts_with("- ") || line.starts_with("* "))
            .count();
        let longest_paragraph_chars = body
            .split("\n\n")
            .map(|paragraph| paragraph.trim())
            .filter(|paragraph| !paragraph.is_empty())
            .map(char_count)
            .max()
            .unwrap_or(0);

        let mut violations = Vec::new();
        if heading_count == 0 {
            violations.push("include a clear top-level markdown heading".to_string());
        }
        if !body.to_ascii_lowercase().contains("session")
            || !(body.to_ascii_lowercase().contains("handoff")
                || body.to_ascii_lowercase().contains("context"))
        {
            violations.push("identify the content as session handoff context".to_string());
        }
        if section_heading_count == 0 {
            violations.push("organize content with section headings".to_string());
        }
        if bullet_count < 3 {
            violations.push("use bullet lists for actionable fields".to_string());
        }
        if longest_paragraph_chars > MAX_HANDOFF_PARAGRAPH_CHARS {
            violations.push(format!(
                "keep paragraphs readable; longest paragraph has {longest_paragraph_chars} chars, exceeding {MAX_HANDOFF_PARAGRAPH_CHARS}"
            ));
        }
        if looks_like_raw_transcript_or_tool_dump(body) {
            violations.push("avoid raw transcript or tool-output dump formatting".to_string());
        }
        if !has_plain_language_signal(body) {
            violations.push(
                "include plain-language narrative text, not only paths or commands".to_string(),
            );
        }
        push_future_session_section_violations(&mut violations, body, self);

        if violations.is_empty() {
            Ok(())
        } else {
            Err(WorkContextSchemaError {
                message: format!(
                    "session handoff markdown is not readable/actionable: {}",
                    violations.join("; ")
                ),
            })
        }
    }

    fn validate_structured_fields(&self) -> Result<(), WorkContextSchemaError> {
        let mut violations = Vec::new();
        push_list_count_violation(
            &mut violations,
            "goals",
            self.goals.len(),
            MAX_HANDOFF_GOALS,
        );
        push_list_count_violation(
            &mut violations,
            "key_changed_files",
            self.key_changed_files.len(),
            MAX_HANDOFF_CHANGED_FILES,
        );
        push_list_count_violation(
            &mut violations,
            "commands",
            self.commands.len(),
            MAX_HANDOFF_COMMANDS,
        );
        push_list_count_violation(
            &mut violations,
            "decisions",
            self.decisions.len(),
            MAX_HANDOFF_DECISIONS,
        );
        push_list_count_violation(
            &mut violations,
            "verification_results",
            self.verification_results.len(),
            MAX_HANDOFF_VERIFICATION_RESULTS,
        );
        push_list_count_violation(
            &mut violations,
            "remaining_work",
            self.remaining_work.len(),
            MAX_HANDOFF_REMAINING_WORK,
        );

        if violations.is_empty() {
            Ok(())
        } else {
            Err(WorkContextSchemaError {
                message: format!(
                    "session handoff context has invalid structured field(s): {}",
                    violations.join("; ")
                ),
            })
        }
    }

    pub fn to_saved_markdown(&self) -> Result<String, WorkContextSchemaError> {
        let normalized = self.normalized_for_save()?;
        normalized.validate_for_save()?;
        let body = strip_saved_handoff_frontmatter(&normalized.handoff_markdown).trim_start();

        Ok(format!(
            "---\nsession_handoff_format_version: {}\nclassification: shared\ntags: [{}]\nsource_tool: {}\nsource_session_ref: {}\nsource_working_directory: {}\nsource_log_path: {}\nsource_updated_at: {}\ntitle: {}\nwork_context_category: {}\nwork_context_categories: [{}]\nwork_context_classification_status: {}\nwork_context_confidence_score: {}\nwork_context_rationale: {}\ngoals: [{}]\nsummary: {}\nkey_changed_files: [{}]\ncommands: [{}]\ndecisions: [{}]\nverification_results: [{}]\nremaining_work: [{}]\ncreated_at: {}\ncleanup_applied: {}\nrefine_mode: {}\nlaunch_target: {}\ninjection_method: {}\ndistillation_focus: [{}]\n---\n\n{}",
            SESSION_HANDOFF_CONTEXT_OUTPUT_FORMAT_VERSION,
            yaml_list(&normalized.tags),
            normalized.source_tool.as_str(),
            yaml_quoted(&normalized.source_session_ref),
            yaml_quoted(&normalized.source_working_directory),
            yaml_quoted(&normalized.source_log_path),
            yaml_optional_string(normalized.source_updated_at.as_deref()),
            yaml_quoted(&normalized.title),
            normalized.category.as_str(),
            normalized.categories
                .iter()
                .map(|category| category.as_str())
                .collect::<Vec<_>>()
                .join(", "),
            classification_status_label(normalized.classification_status),
            normalized.classification_confidence_score,
            yaml_quoted(&normalized.classification_rationale),
            yaml_list(&normalized.goals),
            yaml_quoted(&normalized.summary),
            yaml_list(&normalized.key_changed_files),
            yaml_list(&normalized.commands),
            yaml_list(&normalized.decisions),
            yaml_list(&normalized.verification_results),
            yaml_list(&normalized.remaining_work),
            yaml_quoted(&normalized.created_at),
            normalized.cleanup_applied,
            refine_mode_label(normalized.refine_mode),
            cli_target_label(normalized.launch_target),
            injection_strategy_label(normalized.injection_method),
            distillation_focus_for_categories(&normalized.categories)
                .iter()
                .map(|focus| yaml_quoted(focus))
                .collect::<Vec<_>>()
                .join(", "),
            body
        ))
    }

    pub fn from_saved_markdown(content: &str) -> Result<Self, WorkContextSchemaError> {
        let (raw_frontmatter, body) = split_saved_handoff_frontmatter(content)?;
        let fields = parse_saved_handoff_frontmatter(raw_frontmatter)?;
        let version = parse_required_u8(&fields, "session_handoff_format_version")?;
        if version != SESSION_HANDOFF_CONTEXT_OUTPUT_FORMAT_VERSION {
            return Err(WorkContextSchemaError {
                message: format!(
                    "unsupported session_handoff_format_version {version}; expected {}",
                    SESSION_HANDOFF_CONTEXT_OUTPUT_FORMAT_VERSION
                ),
            });
        }

        let launch_target = parse_required_cli_target(&fields, "launch_target")?;
        let mut context = Self {
            source_tool: parse_required_source_tool(&fields, "source_tool")?,
            source_session_ref: required_scalar(&fields, "source_session_ref")?,
            source_working_directory: required_scalar(&fields, "source_working_directory")?,
            source_log_path: required_scalar(&fields, "source_log_path")?,
            source_updated_at: optional_scalar(&fields, "source_updated_at"),
            title: required_scalar(&fields, "title")?,
            category: parse_required_work_context_category(&fields, "work_context_category")?,
            categories: parse_required_work_context_categories(&fields, "work_context_categories")?,
            classification_status: parse_required_classification_status(
                &fields,
                "work_context_classification_status",
            )?,
            classification_confidence_score: parse_required_u8(
                &fields,
                "work_context_confidence_score",
            )?,
            classification_rationale: required_scalar(&fields, "work_context_rationale")?,
            goals: list_field(&fields, "goals"),
            summary: required_scalar(&fields, "summary")?,
            key_changed_files: list_field(&fields, "key_changed_files"),
            commands: list_field(&fields, "commands"),
            decisions: list_field(&fields, "decisions"),
            verification_results: list_field(&fields, "verification_results"),
            remaining_work: list_field(&fields, "remaining_work"),
            created_at: required_scalar(&fields, "created_at")?,
            handoff_markdown: body.trim_start().to_string(),
            tags: list_field(&fields, "tags"),
            cleanup_applied: parse_required_bool(&fields, "cleanup_applied")?,
            refine_mode: parse_required_refine_mode(&fields, "refine_mode")?,
            launch_target,
            injection_method: parse_required_injection_strategy(&fields, "injection_method")?,
        };
        context = context.normalized_for_save()?;
        context.validate_for_save()?;
        Ok(context)
    }
}

pub fn injection_method_for_launch_target(launch_target: CliTarget) -> InjectionStrategy {
    match launch_target {
        CliTarget::Claude => InjectionStrategy::AppendSystemPromptFile,
        CliTarget::Codex => InjectionStrategy::AgentsMdSectionMarkerMerge,
    }
}

pub fn classify_work_context_detail(
    detail: &SessionLogDetail,
) -> Result<WorkContextClassificationResult, WorkContextSchemaError> {
    let signal_set = WorkContextSignalSet::from_session_detail(detail)?;
    Ok(classify_work_context_signals(&signal_set))
}

pub fn filter_work_relevant_content(
    detail: &SessionLogDetail,
) -> Result<WorkContextFilteredContent, WorkContextSchemaError> {
    let signal_set = WorkContextSignalSet::from_session_detail(detail)?;
    Ok(filter_work_relevant_signals(detail, &signal_set))
}

pub fn filter_work_relevant_signals(
    detail: &SessionLogDetail,
    signal_set: &WorkContextSignalSet,
) -> WorkContextFilteredContent {
    let mut included_records = Vec::new();
    let mut excluded_records = Vec::new();

    for (message_index, message) in detail.messages.iter().enumerate() {
        for line in normalized_signal_lines(&message.content) {
            let Some(record) = filtered_record_from_line(message_index, message, &line) else {
                continue;
            };
            if record.relevance_score > 0
                && !record.reasons.iter().any(|reason| {
                    matches!(
                        reason,
                        WorkContextFilterReason::SensitiveContent | WorkContextFilterReason::Noise
                    )
                })
            {
                included_records.push(record);
            } else {
                excluded_records.push(record);
            }
        }
    }

    let signal_counts = WorkContextSignalCounts::from_signal_set(signal_set);
    let cleanup_applied = signal_counts.sensitive_content > 0
        || signal_counts.noise > 0
        || !excluded_records.is_empty();
    let handoff_markdown =
        filtered_handoff_markdown(detail, signal_set, &included_records, cleanup_applied);

    WorkContextFilteredContent {
        source_tool: signal_set.source_tool,
        source_session_ref: signal_set.source_session_ref.clone(),
        source_working_directory: signal_set.source_working_directory.clone(),
        source_log_path: signal_set.source_log_path.clone(),
        title: signal_set.title.clone(),
        included_records,
        excluded_records,
        signal_counts,
        cleanup_applied,
        handoff_markdown,
    }
}

pub fn classify_work_context_signals(
    signal_set: &WorkContextSignalSet,
) -> WorkContextClassificationResult {
    let categories = work_context_categories(signal_set);
    let category = categories
        .first()
        .copied()
        .unwrap_or(WorkContextCategory::General);
    let signal_counts = WorkContextSignalCounts::from_signal_set(signal_set);
    let suggested_tags = classification_tags(signal_set, &categories);
    let distillation_focus = distillation_focus_for_categories(&categories);

    WorkContextClassificationResult {
        output_format_version: WORK_CONTEXT_CLASSIFICATION_OUTPUT_FORMAT_VERSION,
        source_tool: signal_set.source_tool,
        source_session_ref: signal_set.source_session_ref.clone(),
        source_working_directory: signal_set.source_working_directory.clone(),
        source_log_path: signal_set.source_log_path.clone(),
        title: signal_set.title.clone(),
        category,
        categories,
        status: ClassificationStatus::Classified,
        confidence_score: work_context_category_confidence(signal_set, category, &signal_counts),
        rationale: work_context_category_rationale(category, signal_set),
        signal_counts,
        suggested_tags,
        distillation_focus,
    }
}

pub fn extract_distilled_session_handoff_fields(
    signal_set: &WorkContextSignalSet,
    classification: &WorkContextClassificationResult,
) -> Result<DistilledSessionHandoffFields, WorkContextSchemaError> {
    validate_classification_matches_signal_set(signal_set, classification)?;

    let goals = signal_values(signal_set, WorkContextSignalKind::Goal);
    let summary = first_signal_value(signal_set, WorkContextSignalKind::Summary)
        .or_else(|| goals.first().cloned())
        .unwrap_or_else(|| signal_set.title.clone());
    let key_changed_files = signal_values(signal_set, WorkContextSignalKind::ChangedFile);
    let commands = signal_values(signal_set, WorkContextSignalKind::Command);
    let decisions = signal_values(signal_set, WorkContextSignalKind::Decision);
    let verification_results = signal_values(signal_set, WorkContextSignalKind::VerificationResult);
    let remaining_work = signal_values(signal_set, WorkContextSignalKind::RemainingWork);
    let tags = distilled_handoff_tags(signal_set, classification);
    let cleanup_applied = signal_set.signals.iter().any(|signal| {
        matches!(
            signal.kind,
            WorkContextSignalKind::SensitiveContent | WorkContextSignalKind::Noise
        )
    });

    Ok(DistilledSessionHandoffFields {
        goals,
        summary,
        key_changed_files,
        commands,
        decisions,
        verification_results,
        remaining_work,
        tags,
        cleanup_applied,
    })
}

pub fn work_context_category_definition(
    category: WorkContextCategory,
) -> Option<&'static WorkContextCategoryDefinition> {
    WORK_CONTEXT_CATEGORY_TAXONOMY
        .iter()
        .find(|definition| definition.category == category)
}

fn validate_classification_matches_signal_set(
    signal_set: &WorkContextSignalSet,
    classification: &WorkContextClassificationResult,
) -> Result<(), WorkContextSchemaError> {
    let mut mismatches = Vec::new();
    if classification.source_tool != signal_set.source_tool {
        mismatches.push("source_tool");
    }
    if classification.source_session_ref != signal_set.source_session_ref {
        mismatches.push("source_session_ref");
    }
    if classification.source_working_directory != signal_set.source_working_directory {
        mismatches.push("source_working_directory");
    }
    if classification.source_log_path != signal_set.source_log_path {
        mismatches.push("source_log_path");
    }
    if classification.title != signal_set.title {
        mismatches.push("title");
    }

    if mismatches.is_empty() {
        Ok(())
    } else {
        Err(WorkContextSchemaError {
            message: format!(
                "work-context classification does not match session signal set: {}",
                mismatches.join(", ")
            ),
        })
    }
}

fn distilled_handoff_tags(
    signal_set: &WorkContextSignalSet,
    classification: &WorkContextClassificationResult,
) -> Vec<String> {
    let mut tags = classification.suggested_tags.clone();
    for tag in signal_values(signal_set, WorkContextSignalKind::Tag) {
        if !tags.iter().any(|existing| existing == &tag) {
            tags.push(tag);
        }
    }
    for tag in [
        "session-history".to_string(),
        "resume-context".to_string(),
        signal_set.source_tool.as_str().to_string(),
    ] {
        if !tags.iter().any(|existing| existing == &tag) {
            tags.push(tag);
        }
    }
    for category in &classification.categories {
        let tag = category.as_str().to_string();
        if !tags.iter().any(|existing| existing == &tag) {
            tags.push(tag);
        }
    }
    tags
}

pub fn extract_work_context_signals(detail: &SessionLogDetail) -> Vec<WorkContextSignal> {
    let mut builder = WorkContextSignalBuilder::default();

    for tag in [
        "session-history",
        "resume-context",
        detail.summary.provider.as_str(),
    ] {
        builder.push(WorkContextSignalKind::Tag, tag, 90, Vec::new());
    }

    if !detail.summary.title.trim().is_empty() {
        builder.push(
            WorkContextSignalKind::Goal,
            detail.summary.title.trim(),
            70,
            Vec::new(),
        );
        builder.push(
            WorkContextSignalKind::Summary,
            detail.summary.title.trim(),
            60,
            Vec::new(),
        );
    }

    for (message_index, message) in detail.messages.iter().enumerate() {
        let evidence = WorkContextSignalEvidence {
            message_index,
            role: message.role.clone(),
            timestamp: message.timestamp.clone(),
            excerpt: truncate_for_signal(&message.content, 260),
        };
        for line in normalized_signal_lines(&message.content) {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }

            if let Some(summary) = prefixed_value(line, &["summary:", "result:", "completed:"]) {
                builder.push(
                    WorkContextSignalKind::Summary,
                    summary,
                    88,
                    vec![evidence.clone()],
                );
            }
            if let Some(goal) = goal_signal(line, &message.role) {
                builder.push(
                    WorkContextSignalKind::Goal,
                    &goal,
                    84,
                    vec![evidence.clone()],
                );
            }
            if let Some(command) = command_signal(line) {
                builder.push(
                    WorkContextSignalKind::Command,
                    &command,
                    84,
                    vec![evidence.clone()],
                );
            }
            if let Some(decision) = decision_signal(line) {
                builder.push(
                    WorkContextSignalKind::Decision,
                    &decision,
                    86,
                    vec![evidence.clone()],
                );
            }
            if let Some(remaining) = remaining_work_signal(line) {
                builder.push(
                    WorkContextSignalKind::RemainingWork,
                    &remaining,
                    84,
                    vec![evidence.clone()],
                );
            }
            if is_verification_signal(line) {
                builder.push(
                    WorkContextSignalKind::VerificationResult,
                    line,
                    82,
                    vec![evidence.clone()],
                );
            }
            if is_noise_signal(line) {
                builder.push(
                    WorkContextSignalKind::Noise,
                    line,
                    70,
                    vec![evidence.clone()],
                );
            }
            if is_sensitive_signal(line) {
                builder.push(
                    WorkContextSignalKind::SensitiveContent,
                    line,
                    92,
                    vec![evidence.clone()],
                );
            }
            for path in changed_file_signals(line) {
                builder.push(
                    WorkContextSignalKind::ChangedFile,
                    &path,
                    80,
                    vec![evidence.clone()],
                );
            }
            for tag in tag_signals(line) {
                builder.push(WorkContextSignalKind::Tag, &tag, 72, vec![evidence.clone()]);
            }
        }
    }

    builder.finish()
}

impl WorkContextSignalCounts {
    pub fn from_signal_set(signal_set: &WorkContextSignalSet) -> Self {
        let mut counts = Self::default();
        for signal in &signal_set.signals {
            match signal.kind {
                WorkContextSignalKind::Goal => counts.goals += 1,
                WorkContextSignalKind::Summary => counts.summaries += 1,
                WorkContextSignalKind::ChangedFile => counts.changed_files += 1,
                WorkContextSignalKind::Command => counts.commands += 1,
                WorkContextSignalKind::Decision => counts.decisions += 1,
                WorkContextSignalKind::VerificationResult => counts.verification_results += 1,
                WorkContextSignalKind::RemainingWork => counts.remaining_work += 1,
                WorkContextSignalKind::Tag => counts.tags += 1,
                WorkContextSignalKind::SensitiveContent => counts.sensitive_content += 1,
                WorkContextSignalKind::Noise => counts.noise += 1,
            }
        }
        counts
    }
}

fn work_context_categories(signal_set: &WorkContextSignalSet) -> Vec<WorkContextCategory> {
    let mut scored_categories = scored_work_context_categories(signal_set);
    scored_categories.sort_by_key(|(category, score)| {
        (
            std::cmp::Reverse(*score),
            std::cmp::Reverse(category_precedence(*category)),
        )
    });

    let mut categories = scored_categories
        .into_iter()
        .filter(|(_, score)| *score > 0)
        .map(|(category, _)| category)
        .collect::<Vec<_>>();
    if categories.is_empty() {
        categories.push(WorkContextCategory::General);
    }
    categories
}

fn scored_work_context_categories(
    signal_set: &WorkContextSignalSet,
) -> Vec<(WorkContextCategory, usize)> {
    let text = classification_haystack(signal_set);
    vec![
        (
            WorkContextCategory::Review,
            keyword_score(
                &text,
                &[
                    "review",
                    "qa",
                    "risk",
                    "regression",
                    "finding",
                    "findings",
                    "검토",
                    "리뷰",
                ],
            ),
        ),
        (
            WorkContextCategory::Debugging,
            keyword_score(
                &text,
                &[
                    "bug",
                    "fix",
                    "fixed",
                    "error",
                    "failure",
                    "failed",
                    "debug",
                    "root cause",
                    "오류",
                    "실패",
                ],
            ),
        ),
        (
            WorkContextCategory::Planning,
            keyword_score(
                &text,
                &[
                    "plan",
                    "planning",
                    "requirements",
                    "architecture",
                    "design",
                    "interview",
                    "roadmap",
                    "설계",
                    "기획",
                ],
            ),
        ),
        (
            WorkContextCategory::Refactor,
            keyword_score(
                &text,
                &[
                    "refactor",
                    "cleanup",
                    "rename",
                    "restructure",
                    "migration",
                    "리팩터",
                ],
            ),
        ),
        (
            WorkContextCategory::Research,
            keyword_score(
                &text,
                &[
                    "research",
                    "investigate",
                    "explore",
                    "inspect",
                    "source",
                    "option",
                    "tradeoff",
                    "discovery",
                ],
            ),
        ),
        (
            WorkContextCategory::Verification,
            keyword_score(
                &text,
                &[
                    "verified",
                    "verification",
                    "playwright",
                    "build passed",
                    "validation",
                    "release-readiness",
                ],
            ) + usize::from(
                signal_set
                    .signals
                    .iter()
                    .any(|signal| signal.kind == WorkContextSignalKind::VerificationResult),
            ),
        ),
        (
            WorkContextCategory::Launch,
            keyword_score(
                &text,
                &[
                    "launch",
                    "claude",
                    "codex",
                    "prompt file",
                    "agents.md",
                    "injection",
                    "cleanup",
                    "handoff",
                ],
            ),
        ),
        (
            WorkContextCategory::Implementation,
            keyword_score(
                &text,
                &[
                    "implement",
                    "implemented",
                    "add",
                    "added",
                    "build",
                    "built",
                    "feature",
                    "wire",
                    "integrate",
                    "구현",
                ],
            ) + usize::from(
                signal_set
                    .signals
                    .iter()
                    .any(|signal| signal.kind == WorkContextSignalKind::ChangedFile),
            ),
        ),
    ]
}

fn classification_haystack(signal_set: &WorkContextSignalSet) -> String {
    let mut parts = vec![
        signal_set.title.as_str(),
        signal_set.source_session_ref.as_str(),
        signal_set.source_working_directory.as_str(),
    ];
    parts.extend(
        signal_set
            .signals
            .iter()
            .map(|signal| signal.value.as_str()),
    );
    parts.join("\n").to_ascii_lowercase()
}

fn keyword_score(haystack: &str, keywords: &[&str]) -> usize {
    keywords
        .iter()
        .filter(|keyword| haystack.contains(&keyword.to_ascii_lowercase()))
        .count()
}

fn category_precedence(category: WorkContextCategory) -> u8 {
    match category {
        WorkContextCategory::Launch => 8,
        WorkContextCategory::Implementation => 7,
        WorkContextCategory::Debugging => 6,
        WorkContextCategory::Review => 5,
        WorkContextCategory::Planning => 4,
        WorkContextCategory::Refactor => 3,
        WorkContextCategory::Research => 2,
        WorkContextCategory::Verification => 1,
        WorkContextCategory::General => 0,
    }
}

fn work_context_category_confidence(
    signal_set: &WorkContextSignalSet,
    category: WorkContextCategory,
    signal_counts: &WorkContextSignalCounts,
) -> u8 {
    if category == WorkContextCategory::General {
        return 50;
    }

    let mut confidence = 62u8;
    if signal_counts.summaries > 0 {
        confidence = confidence.saturating_add(6);
    }
    if signal_counts.changed_files > 0 {
        confidence = confidence.saturating_add(6);
    }
    if signal_counts.commands > 0 {
        confidence = confidence.saturating_add(4);
    }
    if signal_counts.decisions > 0 {
        confidence = confidence.saturating_add(6);
    }
    if signal_counts.verification_results > 0 {
        confidence = confidence.saturating_add(5);
    }
    if signal_counts.remaining_work > 0 {
        confidence = confidence.saturating_add(4);
    }

    let category_hits = keyword_score(
        &classification_haystack(signal_set),
        category_keywords(category),
    );
    confidence = confidence.saturating_add((category_hits.min(4) as u8) * 3);
    confidence.min(95)
}

fn category_keywords(category: WorkContextCategory) -> &'static [&'static str] {
    match category {
        WorkContextCategory::Implementation => &["implement", "implemented", "build", "feature"],
        WorkContextCategory::Debugging => &["bug", "fix", "error", "failure", "debug"],
        WorkContextCategory::Review => &["review", "qa", "risk", "regression", "finding"],
        WorkContextCategory::Planning => &["plan", "requirements", "architecture", "design"],
        WorkContextCategory::Refactor => &["refactor", "cleanup", "rename", "restructure"],
        WorkContextCategory::Research => &["research", "investigate", "explore", "inspect"],
        WorkContextCategory::Verification => &["test", "verified", "verification"],
        WorkContextCategory::Launch => &["launch", "injection", "handoff", "agents.md"],
        WorkContextCategory::General => &[],
    }
}

fn work_context_category_rationale(
    category: WorkContextCategory,
    signal_set: &WorkContextSignalSet,
) -> String {
    if category == WorkContextCategory::General {
        return "no strong supported work-context category signal found".to_string();
    }

    let matched = category_keywords(category)
        .iter()
        .copied()
        .filter(|keyword| classification_haystack(signal_set).contains(keyword))
        .collect::<Vec<_>>();
    if matched.is_empty() {
        format!(
            "classified as {} from extracted session structure",
            category.as_str()
        )
    } else {
        format!(
            "classified as {} from session terms: {}",
            category.as_str(),
            matched.join(", ")
        )
    }
}

fn distillation_focus_for_categories(categories: &[WorkContextCategory]) -> Vec<String> {
    let mut focus_items = Vec::new();
    for category in categories {
        if let Some(definition) = work_context_category_definition(*category) {
            for focus in definition.distillation_focus {
                if !focus_items.iter().any(|existing| existing == focus) {
                    focus_items.push((*focus).to_string());
                }
            }
        }
    }
    focus_items
}

fn classification_tags(
    signal_set: &WorkContextSignalSet,
    categories: &[WorkContextCategory],
) -> Vec<String> {
    let mut tags = signal_values(signal_set, WorkContextSignalKind::Tag);
    for tag in [
        "session-history".to_string(),
        "resume-context".to_string(),
        signal_set.source_tool.as_str().to_string(),
    ] {
        if !tags.iter().any(|existing| existing == &tag) {
            tags.push(tag);
        }
    }
    for category in categories {
        let tag = category.as_str().to_string();
        if !tags.iter().any(|existing| existing == &tag) {
            tags.push(tag);
        }
    }
    tags
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct WorkContextSchemaError {
    pub message: String,
}

impl std::fmt::Display for WorkContextSchemaError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for WorkContextSchemaError {}

fn strip_saved_handoff_frontmatter(content: &str) -> &str {
    split_saved_handoff_frontmatter(content)
        .map(|(_, body)| body)
        .unwrap_or(content)
}

fn split_saved_handoff_frontmatter(content: &str) -> Result<(&str, &str), WorkContextSchemaError> {
    let Some(rest) = content.strip_prefix("---\n") else {
        return Err(WorkContextSchemaError {
            message: "saved session handoff markdown is missing YAML frontmatter".to_string(),
        });
    };
    let Some(end_index) = rest.find("\n---") else {
        return Err(WorkContextSchemaError {
            message: "saved session handoff markdown has unterminated YAML frontmatter".to_string(),
        });
    };
    let after_marker = &rest[end_index + "\n---".len()..];
    Ok((
        &rest[..end_index],
        after_marker.strip_prefix('\n').unwrap_or(after_marker),
    ))
}

fn parse_saved_handoff_frontmatter(
    raw: &str,
) -> Result<BTreeMap<String, SavedHandoffFrontmatterValue>, WorkContextSchemaError> {
    let mut fields = BTreeMap::new();
    let mut active_list_key: Option<String> = None;

    for line in raw.lines() {
        let trimmed_line = line.trim();
        if trimmed_line.is_empty() || trimmed_line.starts_with('#') {
            continue;
        }

        if let Some(key) = active_list_key.as_deref() {
            if let Some(item) = trimmed_line.strip_prefix("- ") {
                match fields.get_mut(key) {
                    Some(SavedHandoffFrontmatterValue::List(values)) => {
                        values.push(unquote_yaml_scalar(item));
                        continue;
                    }
                    _ => {
                        return Err(WorkContextSchemaError {
                            message: format!(
                                "saved session handoff frontmatter list field {key} is malformed"
                            ),
                        });
                    }
                }
            }
            active_list_key = None;
        }

        let Some((key, value)) = line.split_once(':') else {
            return Err(WorkContextSchemaError {
                message: format!(
                    "saved session handoff frontmatter line is not a key/value pair: {line}"
                ),
            });
        };
        let key = key.trim().to_string();
        if fields.contains_key(&key) {
            return Err(WorkContextSchemaError {
                message: format!("duplicate saved session handoff frontmatter field: {key}"),
            });
        }
        let value = value.trim();
        if value.is_empty() {
            fields.insert(key.clone(), SavedHandoffFrontmatterValue::List(Vec::new()));
            active_list_key = Some(key);
        } else if value.starts_with('[') && value.ends_with(']') {
            fields.insert(
                key,
                SavedHandoffFrontmatterValue::List(parse_yaml_inline_list(value)?),
            );
        } else {
            fields.insert(
                key,
                SavedHandoffFrontmatterValue::Scalar(unquote_yaml_scalar(value)),
            );
        }
    }

    Ok(fields)
}

#[derive(Debug, Clone, Eq, PartialEq)]
enum SavedHandoffFrontmatterValue {
    Scalar(String),
    List(Vec<String>),
}

fn parse_yaml_inline_list(value: &str) -> Result<Vec<String>, WorkContextSchemaError> {
    let inner = value
        .trim()
        .strip_prefix('[')
        .and_then(|value| value.strip_suffix(']'))
        .unwrap_or(value)
        .trim();
    if inner.is_empty() {
        return Ok(Vec::new());
    }

    let mut values = Vec::new();
    let mut current = String::new();
    let mut in_quote = false;
    let mut escaped = false;
    for character in inner.chars() {
        if escaped {
            current.push(character);
            escaped = false;
            continue;
        }
        if in_quote && character == '\\' {
            current.push(character);
            escaped = true;
            continue;
        }
        if character == '"' {
            current.push(character);
            in_quote = !in_quote;
            continue;
        }
        if character == ',' && !in_quote {
            values.push(unquote_yaml_scalar(&current));
            current.clear();
            continue;
        }
        current.push(character);
    }
    if in_quote {
        return Err(WorkContextSchemaError {
            message: "saved session handoff frontmatter list has unterminated quote".to_string(),
        });
    }
    values.push(unquote_yaml_scalar(&current));
    Ok(values
        .into_iter()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .collect())
}

fn unquote_yaml_scalar(value: &str) -> String {
    let value = value.trim().trim_end_matches(',');
    if value == "null" {
        return String::new();
    }
    let quoted = value
        .strip_prefix('"')
        .and_then(|value| value.strip_suffix('"'))
        .unwrap_or(value);
    let mut output = String::new();
    let mut escaped = false;
    for character in quoted.chars() {
        if escaped {
            output.push(match character {
                '"' => '"',
                '\\' => '\\',
                other => other,
            });
            escaped = false;
        } else if character == '\\' {
            escaped = true;
        } else {
            output.push(character);
        }
    }
    output.trim().to_string()
}

fn required_scalar(
    fields: &BTreeMap<String, SavedHandoffFrontmatterValue>,
    key: &str,
) -> Result<String, WorkContextSchemaError> {
    match fields.get(key) {
        Some(SavedHandoffFrontmatterValue::Scalar(value)) if !value.trim().is_empty() => {
            Ok(value.clone())
        }
        Some(_) => Err(WorkContextSchemaError {
            message: format!("saved session handoff field {key} must be a non-empty scalar"),
        }),
        None => Err(WorkContextSchemaError {
            message: format!("saved session handoff is missing required field: {key}"),
        }),
    }
}

fn optional_scalar(
    fields: &BTreeMap<String, SavedHandoffFrontmatterValue>,
    key: &str,
) -> Option<String> {
    match fields.get(key) {
        Some(SavedHandoffFrontmatterValue::Scalar(value)) if !value.trim().is_empty() => {
            Some(value.clone())
        }
        _ => None,
    }
}

fn list_field(fields: &BTreeMap<String, SavedHandoffFrontmatterValue>, key: &str) -> Vec<String> {
    match fields.get(key) {
        Some(SavedHandoffFrontmatterValue::List(values)) => values.clone(),
        Some(SavedHandoffFrontmatterValue::Scalar(value)) => parse_yaml_inline_list(value)
            .unwrap_or_default()
            .into_iter()
            .collect(),
        None => Vec::new(),
    }
}

fn parse_required_u8(
    fields: &BTreeMap<String, SavedHandoffFrontmatterValue>,
    key: &str,
) -> Result<u8, WorkContextSchemaError> {
    let value = required_scalar(fields, key)?;
    value
        .parse::<u16>()
        .map(|value| value.min(255) as u8)
        .map_err(|_| WorkContextSchemaError {
            message: format!("saved session handoff field {key} must be an integer"),
        })
}

fn parse_required_bool(
    fields: &BTreeMap<String, SavedHandoffFrontmatterValue>,
    key: &str,
) -> Result<bool, WorkContextSchemaError> {
    match required_scalar(fields, key)?.to_ascii_lowercase().as_str() {
        "true" => Ok(true),
        "false" => Ok(false),
        _ => Err(WorkContextSchemaError {
            message: format!("saved session handoff field {key} must be true or false"),
        }),
    }
}

fn parse_required_source_tool(
    fields: &BTreeMap<String, SavedHandoffFrontmatterValue>,
    key: &str,
) -> Result<SessionLogProvider, WorkContextSchemaError> {
    match required_scalar(fields, key)?.to_ascii_lowercase().as_str() {
        "claude" => Ok(SessionLogProvider::Claude),
        "codex" => Ok(SessionLogProvider::Codex),
        _ => Err(WorkContextSchemaError {
            message: format!("saved session handoff field {key} must be claude or codex"),
        }),
    }
}

fn parse_required_work_context_category(
    fields: &BTreeMap<String, SavedHandoffFrontmatterValue>,
    key: &str,
) -> Result<WorkContextCategory, WorkContextSchemaError> {
    parse_work_context_category(&required_scalar(fields, key)?).ok_or_else(|| {
        WorkContextSchemaError {
            message: format!("saved session handoff field {key} has an unsupported category"),
        }
    })
}

fn parse_required_work_context_categories(
    fields: &BTreeMap<String, SavedHandoffFrontmatterValue>,
    key: &str,
) -> Result<Vec<WorkContextCategory>, WorkContextSchemaError> {
    let categories = list_field(fields, key)
        .into_iter()
        .map(|value| {
            parse_work_context_category(&value).ok_or_else(|| WorkContextSchemaError {
                message: format!(
                    "saved session handoff field {key} has unsupported category {value}"
                ),
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    if categories.is_empty() {
        return Err(WorkContextSchemaError {
            message: format!(
                "saved session handoff field {key} must contain at least one category"
            ),
        });
    }
    Ok(categories)
}

fn parse_work_context_category(value: &str) -> Option<WorkContextCategory> {
    match value.trim().to_ascii_lowercase().as_str() {
        "implementation" => Some(WorkContextCategory::Implementation),
        "debugging" => Some(WorkContextCategory::Debugging),
        "review" => Some(WorkContextCategory::Review),
        "planning" => Some(WorkContextCategory::Planning),
        "refactor" => Some(WorkContextCategory::Refactor),
        "research" => Some(WorkContextCategory::Research),
        "verification" => Some(WorkContextCategory::Verification),
        "launch" => Some(WorkContextCategory::Launch),
        "general" => Some(WorkContextCategory::General),
        _ => None,
    }
}

fn parse_required_classification_status(
    fields: &BTreeMap<String, SavedHandoffFrontmatterValue>,
    key: &str,
) -> Result<ClassificationStatus, WorkContextSchemaError> {
    match required_scalar(fields, key)?.to_ascii_lowercase().as_str() {
        "pending" => Ok(ClassificationStatus::Pending),
        "classified" => Ok(ClassificationStatus::Classified),
        "reviewed" => Ok(ClassificationStatus::Reviewed),
        "modified" => Ok(ClassificationStatus::Modified),
        _ => Err(WorkContextSchemaError {
            message: format!("saved session handoff field {key} has an unsupported status"),
        }),
    }
}

fn parse_required_refine_mode(
    fields: &BTreeMap<String, SavedHandoffFrontmatterValue>,
    key: &str,
) -> Result<WorkContextRefineMode, WorkContextSchemaError> {
    match required_scalar(fields, key)?.to_ascii_lowercase().as_str() {
        "raw" => Ok(WorkContextRefineMode::Raw),
        "refined" => Ok(WorkContextRefineMode::Refined),
        _ => Err(WorkContextSchemaError {
            message: format!("saved session handoff field {key} must be raw or refined"),
        }),
    }
}

fn parse_required_cli_target(
    fields: &BTreeMap<String, SavedHandoffFrontmatterValue>,
    key: &str,
) -> Result<CliTarget, WorkContextSchemaError> {
    match required_scalar(fields, key)?.to_ascii_lowercase().as_str() {
        "claude" => Ok(CliTarget::Claude),
        "codex" => Ok(CliTarget::Codex),
        _ => Err(WorkContextSchemaError {
            message: format!("saved session handoff field {key} must be claude or codex"),
        }),
    }
}

fn parse_required_injection_strategy(
    fields: &BTreeMap<String, SavedHandoffFrontmatterValue>,
    key: &str,
) -> Result<InjectionStrategy, WorkContextSchemaError> {
    match required_scalar(fields, key)?.to_ascii_lowercase().as_str() {
        "append-system-prompt-file" => Ok(InjectionStrategy::AppendSystemPromptFile),
        "agents-md-section-marker-merge" => Ok(InjectionStrategy::AgentsMdSectionMarkerMerge),
        _ => Err(WorkContextSchemaError {
            message: format!(
                "saved session handoff field {key} has an unsupported injection method"
            ),
        }),
    }
}

fn normalize_distilled_session_handoff_fields(
    fields: &mut DistilledSessionHandoffFields,
) -> Result<(), WorkContextSchemaError> {
    fields.goals = normalize_freeform_list(&fields.goals, "goals")?;
    fields.summary = normalize_freeform_structured_value(&fields.summary);
    fields.key_changed_files =
        normalize_changed_file_list(&fields.key_changed_files, "key_changed_files")?;
    fields.commands = normalize_freeform_list(&fields.commands, "commands")?;
    fields.decisions = normalize_freeform_list(&fields.decisions, "decisions")?;
    fields.verification_results =
        normalize_freeform_list(&fields.verification_results, "verification_results")?;
    fields.remaining_work = normalize_freeform_list(&fields.remaining_work, "remaining_work")?;
    fields.tags = normalize_tag_list(&fields.tags, "tags")?;
    Ok(())
}

fn overlay_distilled_markdown_fields(fields: &mut DistilledSessionHandoffFields, markdown: &str) {
    let sections = parse_distilled_handoff_markdown_sections(markdown);
    if sections.is_empty() {
        return;
    }

    if let Some(summary) = distilled_section_summary(&sections, &["handoff summary", "summary"]) {
        fields.summary = summary;
    }

    overlay_distilled_section_list(&sections, &mut fields.goals, &["goals", "objectives"]);
    overlay_distilled_section_list(
        &sections,
        &mut fields.key_changed_files,
        &[
            "key changed files",
            "changed files",
            "files changed",
            "important files",
        ],
    );
    overlay_distilled_section_list(&sections, &mut fields.commands, &["commands"]);
    overlay_distilled_section_list(&sections, &mut fields.decisions, &["decisions"]);
    overlay_distilled_section_list(
        &sections,
        &mut fields.verification_results,
        &["verification results", "verification", "tests", "checks"],
    );
    overlay_distilled_section_list(
        &sections,
        &mut fields.remaining_work,
        &[
            "remaining work",
            "next steps",
            "follow-up",
            "follow up",
            "todos",
        ],
    );
}

fn overlay_distilled_section_list(
    sections: &BTreeMap<String, Vec<String>>,
    target: &mut Vec<String>,
    accepted_names: &[&str],
) {
    let values = distilled_section_bullets(sections, accepted_names);
    if !values.is_empty() {
        *target = values;
    }
}

fn parse_distilled_handoff_markdown_sections(markdown: &str) -> BTreeMap<String, Vec<String>> {
    let body = strip_saved_handoff_frontmatter(markdown);
    let mut sections: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut active_heading: Option<String> = None;

    for line in body.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('#') {
            let heading = trimmed.trim_start_matches('#').trim();
            if !heading.is_empty() {
                let key = normalize_markdown_heading_key(heading);
                sections.entry(key.clone()).or_default();
                active_heading = Some(key);
            }
            continue;
        }

        if let Some(heading) = active_heading.as_ref() {
            sections
                .entry(heading.clone())
                .or_default()
                .push(trimmed.to_string());
        }
    }

    sections
}

fn normalize_markdown_heading_key(heading: &str) -> String {
    collapse_internal_whitespace(
        &heading
            .trim()
            .trim_matches(|character: char| matches!(character, ':' | '`' | '"' | '\''))
            .to_ascii_lowercase(),
    )
}

fn distilled_section_summary(
    sections: &BTreeMap<String, Vec<String>>,
    accepted_names: &[&str],
) -> Option<String> {
    sections
        .iter()
        .find(|(heading, _)| heading_matches_any(heading, accepted_names))
        .and_then(|(_, lines)| {
            lines
                .iter()
                .map(|line| line.trim())
                .filter(|line| !line.is_empty())
                .filter(|line| !line.starts_with('#'))
                .filter(|line| !line.starts_with("- ") && !line.starts_with("* "))
                .map(normalize_freeform_structured_value)
                .find(|line| !line.is_empty())
        })
}

fn distilled_section_bullets(
    sections: &BTreeMap<String, Vec<String>>,
    accepted_names: &[&str],
) -> Vec<String> {
    sections
        .iter()
        .filter(|(heading, _)| heading_matches_any(heading, accepted_names))
        .flat_map(|(_, lines)| {
            lines
                .iter()
                .map(|line| line.trim())
                .filter(|line| line.starts_with("- ") || line.starts_with("* "))
                .map(normalize_freeform_structured_value)
                .filter(|line| !line.is_empty())
                .collect::<Vec<_>>()
        })
        .collect()
}

fn heading_matches_any(heading: &str, accepted_names: &[&str]) -> bool {
    accepted_names
        .iter()
        .any(|accepted| heading == *accepted || heading.contains(accepted))
}

fn normalize_freeform_list(
    values: &[String],
    field: &str,
) -> Result<Vec<String>, WorkContextSchemaError> {
    normalize_structured_list(values, field, normalize_freeform_structured_value)
}

fn normalize_changed_file_list(
    values: &[String],
    field: &str,
) -> Result<Vec<String>, WorkContextSchemaError> {
    normalize_structured_list(values, field, normalize_changed_file_value)
}

fn normalize_tag_list(
    values: &[String],
    field: &str,
) -> Result<Vec<String>, WorkContextSchemaError> {
    normalize_structured_list(values, field, normalize_tag_value)
}

fn normalize_structured_list<F>(
    values: &[String],
    field: &str,
    normalize: F,
) -> Result<Vec<String>, WorkContextSchemaError>
where
    F: Fn(&str) -> String,
{
    let mut normalized = Vec::new();
    for value in values {
        reject_malformed_structured_value(field, value)?;
        let value = normalize(value);
        if value.is_empty() {
            continue;
        }
        if !normalized
            .iter()
            .any(|existing: &String| existing.eq_ignore_ascii_case(&value))
        {
            normalized.push(value);
        }
    }
    Ok(normalized)
}

fn reject_malformed_structured_value(
    field: &str,
    value: &str,
) -> Result<(), WorkContextSchemaError> {
    if value.chars().any(|character| {
        matches!(
            character,
            '\0' | '\u{0001}'
                | '\u{0002}'
                | '\u{0003}'
                | '\u{0004}'
                | '\u{0005}'
                | '\u{0006}'
                | '\u{0007}'
                | '\u{0008}'
                | '\u{000b}'
                | '\u{000c}'
                | '\u{000e}'
                | '\u{000f}'
                | '\u{0010}'
                | '\u{0011}'
                | '\u{0012}'
                | '\u{0013}'
                | '\u{0014}'
                | '\u{0015}'
                | '\u{0016}'
                | '\u{0017}'
                | '\u{0018}'
                | '\u{0019}'
                | '\u{001a}'
                | '\u{001b}'
                | '\u{001c}'
                | '\u{001d}'
                | '\u{001e}'
                | '\u{001f}'
        )
    }) {
        return Err(WorkContextSchemaError {
            message: format!("session handoff field {field} contains control characters"),
        });
    }
    if value.lines().any(|line| line.trim() == "---") {
        return Err(WorkContextSchemaError {
            message: format!("session handoff field {field} contains a frontmatter delimiter"),
        });
    }
    Ok(())
}

fn normalize_freeform_structured_value(value: &str) -> String {
    collapse_internal_whitespace(
        value
            .trim()
            .trim_start_matches(['-', '*'])
            .trim()
            .trim_matches(|character: char| matches!(character, '`' | '"' | '\'')),
    )
}

fn normalize_changed_file_value(value: &str) -> String {
    collapse_internal_whitespace(
        value
            .trim()
            .trim_start_matches(['-', '*'])
            .trim()
            .trim_matches(|character: char| {
                matches!(
                    character,
                    '`' | '\'' | '"' | ',' | ';' | ':' | ')' | '(' | '[' | ']' | '{' | '}'
                )
            })
            .trim_end_matches('.'),
    )
    .replace('\\', "/")
}

fn normalize_tag_value(value: &str) -> String {
    let mut tag = String::new();
    let mut previous_separator = false;
    for character in collapse_internal_whitespace(value).chars() {
        if character.is_ascii_alphanumeric() {
            tag.push(character.to_ascii_lowercase());
            previous_separator = false;
        } else if matches!(character, '-' | '_' | '/' | ' ' | '.') && !previous_separator {
            tag.push('-');
            previous_separator = true;
        }
    }
    tag.trim_matches('-').to_string()
}

fn collapse_internal_whitespace(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn push_list_count_violation(
    violations: &mut Vec<String>,
    field: &str,
    count: usize,
    max_count: usize,
) {
    if count > max_count {
        violations.push(format!(
            "{field} has {count} item(s), exceeding {max_count}"
        ));
    }
}

fn yaml_list(values: &[String]) -> String {
    values
        .iter()
        .filter(|value| !value.trim().is_empty())
        .map(|value| yaml_quoted(value))
        .collect::<Vec<_>>()
        .join(", ")
}

fn yaml_optional_string(value: Option<&str>) -> String {
    value.map(yaml_quoted).unwrap_or_else(|| "null".to_string())
}

fn yaml_quoted(value: &str) -> String {
    format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
}

fn classification_status_label(status: ClassificationStatus) -> &'static str {
    match status {
        ClassificationStatus::Pending => "pending",
        ClassificationStatus::Classified => "classified",
        ClassificationStatus::Reviewed => "reviewed",
        ClassificationStatus::Modified => "modified",
    }
}

fn refine_mode_label(refine_mode: WorkContextRefineMode) -> &'static str {
    match refine_mode {
        WorkContextRefineMode::Raw => "raw",
        WorkContextRefineMode::Refined => "refined",
    }
}

fn cli_target_label(target: CliTarget) -> &'static str {
    match target {
        CliTarget::Claude => "claude",
        CliTarget::Codex => "codex",
    }
}

fn injection_strategy_label(strategy: InjectionStrategy) -> &'static str {
    match strategy {
        InjectionStrategy::AppendSystemPromptFile => "append-system-prompt-file",
        InjectionStrategy::AgentsMdSectionMarkerMerge => "agents-md-section-marker-merge",
    }
}

fn char_count(value: &str) -> usize {
    value.chars().count()
}

fn push_length_violation(violations: &mut Vec<String>, field: &str, value: &str, max_chars: usize) {
    let count = char_count(value);
    if count > max_chars {
        violations.push(format!("{field} has {count} chars, exceeding {max_chars}"));
    }
}

fn push_list_length_violations(
    violations: &mut Vec<String>,
    field: &str,
    values: &[String],
    max_chars: usize,
) {
    for (index, value) in values.iter().enumerate() {
        let count = char_count(value);
        if count > max_chars {
            violations.push(format!(
                "{field}[{index}] has {count} chars, exceeding {max_chars}"
            ));
        }
    }
}

fn push_missing_markdown_value(
    missing: &mut Vec<String>,
    markdown: &str,
    field: &str,
    value: &str,
) {
    let value = value.trim();
    if !value.is_empty() && !markdown.contains(value) {
        missing.push(field.to_string());
    }
}

fn push_missing_markdown_values(
    missing: &mut Vec<String>,
    markdown: &str,
    field: &str,
    values: &[String],
    limit: usize,
) {
    let missing_values = values
        .iter()
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .take(limit)
        .filter(|value| !markdown.contains(*value))
        .count();
    if missing_values > 0 {
        missing.push(format!("{field} ({missing_values} item(s))"));
    }
}

fn looks_like_raw_transcript_or_tool_dump(markdown: &str) -> bool {
    let lower = markdown.to_ascii_lowercase();
    let tool_dump_markers = [
        "chunk id:",
        "original token count",
        "wall time:",
        "process exited with code",
        "\"type\":\"event_msg\"",
        "\"type\":\"session_meta\"",
        "\"payload\":",
        "tool_use_id",
    ];
    if tool_dump_markers
        .iter()
        .any(|marker| lower.contains(marker))
    {
        return true;
    }

    let non_empty = markdown
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>();
    if non_empty.len() < 3 {
        return false;
    }

    let jsonish_lines = non_empty
        .iter()
        .filter(|line| {
            (line.starts_with('{') && line.ends_with('}'))
                || (line.starts_with('[') && line.ends_with(']'))
        })
        .count();
    jsonish_lines * 2 >= non_empty.len()
}

fn has_plain_language_signal(markdown: &str) -> bool {
    markdown
        .split(|character: char| matches!(character, '.' | '\n'))
        .map(str::trim)
        .any(|segment| {
            let word_count = segment
                .split_whitespace()
                .filter(|word| word.chars().any(|character| character.is_alphabetic()))
                .count();
            word_count >= 4 && segment.chars().any(|character| character == ' ')
        })
}

fn push_future_session_section_violations(
    violations: &mut Vec<String>,
    body: &str,
    context: &SessionHandoffContext,
) {
    let sections = markdown_section_headings(body);
    if !context.goals.is_empty() && !has_named_section(&sections, &["goal", "objective"]) {
        violations.push("include a goals section for the next session".to_string());
    }
    if !has_named_section(&sections, &["current state", "state", "status", "summary"]) {
        violations.push("include current state or summary for the next session".to_string());
    }
    if !context.decisions.is_empty() && !has_named_section(&sections, &["decision"]) {
        violations.push("include decisions that should carry forward".to_string());
    }
    if !context.remaining_work.is_empty()
        && !has_named_section(
            &sections,
            &["next step", "remaining work", "follow-up", "todo"],
        )
    {
        violations.push("include next steps or remaining work for the next session".to_string());
    }
}

fn markdown_section_headings(markdown: &str) -> Vec<String> {
    markdown
        .lines()
        .map(str::trim)
        .filter(|line| line.starts_with('#'))
        .map(|line| line.trim_start_matches('#').trim().to_ascii_lowercase())
        .collect()
}

fn has_named_section(sections: &[String], accepted_names: &[&str]) -> bool {
    sections.iter().any(|section| {
        accepted_names
            .iter()
            .any(|accepted| section.contains(accepted))
    })
}

fn first_signal_value(
    signal_set: &WorkContextSignalSet,
    kind: WorkContextSignalKind,
) -> Option<String> {
    signal_set
        .signals
        .iter()
        .filter(|signal| signal.kind == kind && !signal.value.trim().is_empty())
        .max_by_key(|signal| signal.confidence)
        .map(|signal| signal.value.clone())
}

fn signal_values(signal_set: &WorkContextSignalSet, kind: WorkContextSignalKind) -> Vec<String> {
    signal_set
        .signals
        .iter()
        .filter(|signal| signal.kind == kind)
        .map(|signal| signal.value.trim())
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
        .collect()
}

#[derive(Default)]
struct WorkContextSignalBuilder {
    signals: Vec<WorkContextSignal>,
}

impl WorkContextSignalBuilder {
    fn push(
        &mut self,
        kind: WorkContextSignalKind,
        value: &str,
        confidence: u8,
        evidence: Vec<WorkContextSignalEvidence>,
    ) {
        let value = clean_signal_value(value);
        if value.is_empty() {
            return;
        }
        if self
            .signals
            .iter()
            .any(|signal| signal.kind == kind && signal.value.eq_ignore_ascii_case(&value))
        {
            return;
        }
        self.signals.push(WorkContextSignal {
            kind,
            value,
            confidence,
            evidence,
        });
    }

    fn finish(self) -> Vec<WorkContextSignal> {
        self.signals
    }
}

fn normalized_signal_lines(content: &str) -> Vec<String> {
    content
        .lines()
        .map(|line| {
            line.trim()
                .trim_start_matches(['-', '*'])
                .trim()
                .to_string()
        })
        .filter(|line| !line.is_empty())
        .collect()
}

fn prefixed_value<'a>(line: &'a str, prefixes: &[&str]) -> Option<&'a str> {
    let lower = line.to_ascii_lowercase();
    prefixes
        .iter()
        .find_map(|prefix| {
            lower
                .strip_prefix(prefix)
                .map(|_| line[prefix.len()..].trim())
        })
        .filter(|value| !value.is_empty())
}

fn goal_signal(line: &str, role: &str) -> Option<String> {
    if let Some(value) = prefixed_value(
        line,
        &[
            "goal:",
            "goals:",
            "objective:",
            "objectives:",
            "task:",
            "request:",
        ],
    ) {
        return Some(value.to_string());
    }

    if role == "user" && contains_work_keyword(line) {
        return Some(line.to_string());
    }

    None
}

fn command_signal(line: &str) -> Option<String> {
    let lower = line.to_ascii_lowercase();
    let markers = [
        "cargo test",
        "cargo fmt",
        "cargo clippy",
        "cargo check",
        "npm test",
        "npm run",
        "pnpm test",
        "pnpm run",
        "yarn test",
        "yarn run",
        "pytest",
        "playwright",
        "vitest",
    ];
    let marker = markers.iter().find(|marker| lower.contains(**marker))?;
    let start = lower.find(marker)?;
    let command = line[start..]
        .trim()
        .trim_end_matches(|character: char| matches!(character, '.' | ';'));
    Some(command.to_string())
}

fn decision_signal(line: &str) -> Option<String> {
    let lower = line.to_ascii_lowercase();
    if let Some(value) = prefixed_value(line, &["decision:", "decided:", "decisions:"]) {
        return Some(value.to_string());
    }
    for marker in [
        "we decided to ",
        "i decided to ",
        "decided to ",
        "decision was to ",
        "kept ",
        "chose to ",
    ] {
        if lower.contains(marker) {
            return Some(line.to_string());
        }
    }
    None
}

fn remaining_work_signal(line: &str) -> Option<String> {
    if let Some(value) = prefixed_value(
        line,
        &[
            "remaining:",
            "remaining work:",
            "follow-up:",
            "follow up:",
            "todo:",
            "next:",
            "unresolved:",
        ],
    ) {
        return Some(value.to_string());
    }
    let lower = line.to_ascii_lowercase();
    for marker in [
        "remaining work",
        "follow-up",
        "follow up",
        "still need",
        "needs follow",
        "unresolved",
        "todo",
    ] {
        if lower.contains(marker) {
            return Some(line.to_string());
        }
    }
    None
}

fn is_verification_signal(line: &str) -> bool {
    let lower = line.to_ascii_lowercase();
    let command_hit = [
        "cargo test",
        "cargo fmt",
        "cargo clippy",
        "npm test",
        "npm run",
        "pnpm test",
        "pnpm run",
        "yarn test",
        "pytest",
        "playwright",
        "vitest",
    ]
    .iter()
    .any(|marker| lower.contains(marker));
    let result_hit = [
        "verification",
        "verified",
        "tests passed",
        "test passed",
        "all tests passed",
        "checks passed",
        "failed",
    ]
    .iter()
    .any(|marker| lower.contains(marker));

    command_hit || result_hit
}

fn is_noise_signal(line: &str) -> bool {
    let lower = line.to_ascii_lowercase();
    [
        "token count",
        "wall time:",
        "process exited with code",
        "chunk id:",
        "original token count",
        "tool output",
        "[ac_start:",
        "[ac_complete:",
    ]
    .iter()
    .any(|marker| lower.contains(marker))
}

fn is_sensitive_signal(line: &str) -> bool {
    let lower = line.to_ascii_lowercase();
    [
        "api_key",
        "api key",
        "secret",
        "password",
        "private key",
        "bearer ",
        "authorization:",
    ]
    .iter()
    .any(|marker| lower.contains(marker))
}

fn changed_file_signals(line: &str) -> Vec<String> {
    line.split_whitespace()
        .filter_map(candidate_path_signal)
        .collect()
}

fn candidate_path_signal(token: &str) -> Option<String> {
    let trimmed = token.trim_matches(|character: char| {
        matches!(
            character,
            '`' | '\'' | '"' | ',' | ';' | ':' | ')' | '(' | '[' | ']' | '{' | '}'
        )
    });
    let trimmed = trimmed.trim_end_matches('.');
    if trimmed.len() < 3 || trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        return None;
    }
    let has_separator = trimmed.contains('/') || trimmed.contains('\\');
    let has_known_extension = [
        ".rs", ".ts", ".tsx", ".js", ".jsx", ".json", ".jsonl", ".toml", ".md", ".yml", ".yaml",
        ".css", ".html", ".sql", ".sh",
    ]
    .iter()
    .any(|extension| trimmed.ends_with(extension));
    if has_separator && has_known_extension {
        Some(trimmed.to_string())
    } else {
        None
    }
}

fn tag_signals(line: &str) -> Vec<String> {
    let lower = line.to_ascii_lowercase();
    let mut tags = Vec::new();
    for (marker, tag) in [
        ("rust", "rust"),
        ("tauri", "tauri"),
        ("react", "react"),
        ("frontend", "frontend"),
        ("backend", "backend"),
        ("test", "verification"),
        ("launch", "launch"),
        ("session", "session-history"),
    ] {
        if lower.contains(marker) && !tags.iter().any(|existing| existing == tag) {
            tags.push(tag.to_string());
        }
    }
    tags
}

fn filtered_record_from_line(
    message_index: usize,
    message: &crate::session_logs::SessionLogMessage,
    line: &str,
) -> Option<WorkContextFilteredRecord> {
    let content = clean_signal_value(line);
    if content.is_empty() {
        return None;
    }

    let mut reasons = Vec::new();
    let mut relevance_score = 0u8;

    if is_sensitive_signal(&content) {
        reasons.push(WorkContextFilterReason::SensitiveContent);
        return Some(WorkContextFilteredRecord {
            message_index,
            role: message.role.clone(),
            timestamp: message.timestamp.clone(),
            content,
            relevance_score: 0,
            reasons,
        });
    }
    if is_noise_signal(&content) {
        reasons.push(WorkContextFilterReason::Noise);
        return Some(WorkContextFilteredRecord {
            message_index,
            role: message.role.clone(),
            timestamp: message.timestamp.clone(),
            content,
            relevance_score: 0,
            reasons,
        });
    }

    if message.role == "user" {
        relevance_score = relevance_score.max(58);
        reasons.push(WorkContextFilterReason::UserRequest);
    }
    if goal_signal(&content, &message.role).is_some() {
        relevance_score = relevance_score.max(86);
        reasons.push(WorkContextFilterReason::Summary);
    }
    if prefixed_value(&content, &["summary:", "result:", "completed:"]).is_some() {
        relevance_score = relevance_score.max(92);
        reasons.push(WorkContextFilterReason::Summary);
    }
    if !changed_file_signals(&content).is_empty() {
        relevance_score = relevance_score.max(88);
        reasons.push(WorkContextFilterReason::ChangedFile);
    }
    if command_signal(&content).is_some() {
        relevance_score = relevance_score.max(84);
        reasons.push(WorkContextFilterReason::VerificationResult);
    }
    if decision_signal(&content).is_some() {
        relevance_score = relevance_score.max(88);
        reasons.push(WorkContextFilterReason::Decision);
    }
    if is_verification_signal(&content) {
        relevance_score = relevance_score.max(84);
        reasons.push(WorkContextFilterReason::VerificationResult);
    }
    if remaining_work_signal(&content).is_some() {
        relevance_score = relevance_score.max(84);
        reasons.push(WorkContextFilterReason::RemainingWork);
    }
    if contains_work_keyword(&content) {
        relevance_score = relevance_score.max(66);
        reasons.push(WorkContextFilterReason::WorkKeyword);
    }

    if reasons.is_empty() {
        reasons.push(WorkContextFilterReason::ConversationChatter);
    }

    Some(WorkContextFilteredRecord {
        message_index,
        role: message.role.clone(),
        timestamp: message.timestamp.clone(),
        content,
        relevance_score,
        reasons,
    })
}

fn contains_work_keyword(line: &str) -> bool {
    let lower = line.to_ascii_lowercase();
    [
        "implement",
        "implemented",
        "extract",
        "extracted",
        "changed",
        "added",
        "fixed",
        "refactor",
        "review",
        "plan",
        "launch",
        "handoff",
        "session",
        "tests",
        "verified",
    ]
    .iter()
    .any(|marker| lower.contains(marker))
}

fn filtered_handoff_markdown(
    detail: &SessionLogDetail,
    signal_set: &WorkContextSignalSet,
    records: &[WorkContextFilteredRecord],
    cleanup_applied: bool,
) -> String {
    let classification = classify_work_context_signals(signal_set);
    let goals = signal_values(signal_set, WorkContextSignalKind::Goal);
    let summaries = signal_values(signal_set, WorkContextSignalKind::Summary);
    let changed_files = signal_values(signal_set, WorkContextSignalKind::ChangedFile);
    let commands = signal_values(signal_set, WorkContextSignalKind::Command);
    let decisions = signal_values(signal_set, WorkContextSignalKind::Decision);
    let verification_results = signal_values(signal_set, WorkContextSignalKind::VerificationResult);
    let remaining_work = signal_values(signal_set, WorkContextSignalKind::RemainingWork);

    let mut markdown = String::new();
    markdown.push_str("---\n");
    markdown.push_str("classification: shared\n");
    markdown.push_str("tags: [session-history, resume-context]\n");
    markdown.push_str("---\n\n");
    markdown.push_str("# Previous Session Context\n\n");
    markdown.push_str(&format!("- Provider: {}\n", detail.summary.provider));
    markdown.push_str(&format!("- Session ID: {}\n", detail.summary.session_id));
    if let Some(updated_at) = &detail.summary.updated_at {
        markdown.push_str(&format!("- Updated: {updated_at}\n"));
    }
    if let Some(cwd) = &detail.summary.cwd {
        markdown.push_str(&format!("- Working directory: `{cwd}`\n"));
    }
    markdown.push_str(&format!(
        "- Source log: `{}`\n",
        detail.summary.file_path.display()
    ));
    markdown.push_str(&format!(
        "- Work category: {}\n",
        classification.category.as_str()
    ));
    markdown.push_str(&format!("- Cleanup applied: {cleanup_applied}\n\n"));

    markdown.push_str("## Handoff Summary\n\n");
    markdown.push_str(&format!(
        "{}\n\n",
        concise_handoff_summary(signal_set, &summaries, &goals)
    ));

    push_limited_signal_section(
        &mut markdown,
        "Current state",
        &current_state_values(
            signal_set,
            &summaries,
            &changed_files,
            &verification_results,
        ),
        5,
    );
    push_limited_signal_section(&mut markdown, "Goals", &goals, MAX_HANDOFF_GOALS);
    push_limited_signal_section(&mut markdown, "Summary", &summaries, 3);
    push_limited_signal_section(
        &mut markdown,
        "Key changed files",
        &changed_files,
        MAX_HANDOFF_CHANGED_FILES,
    );
    push_limited_signal_section(&mut markdown, "Commands", &commands, MAX_HANDOFF_COMMANDS);
    push_limited_signal_section(
        &mut markdown,
        "Decisions",
        &decisions,
        MAX_HANDOFF_DECISIONS,
    );
    push_limited_signal_section(
        &mut markdown,
        "Verification results",
        &verification_results,
        MAX_HANDOFF_VERIFICATION_RESULTS,
    );
    push_limited_signal_section(
        &mut markdown,
        "Next steps",
        &next_step_values(&remaining_work),
        MAX_HANDOFF_REMAINING_WORK,
    );
    push_limited_signal_section(
        &mut markdown,
        "Remaining work",
        &remaining_work,
        MAX_HANDOFF_REMAINING_WORK,
    );
    push_limited_signal_section(
        &mut markdown,
        "Distillation focus",
        &classification.distillation_focus,
        8,
    );

    if summaries.is_empty()
        && changed_files.is_empty()
        && decisions.is_empty()
        && verification_results.is_empty()
        && remaining_work.is_empty()
        && !records.is_empty()
    {
        let fallback = records
            .iter()
            .map(|record| record.content.clone())
            .collect::<Vec<_>>();
        push_limited_signal_section(&mut markdown, "Fallback extracted context", &fallback, 5);
    }

    markdown
}

fn concise_handoff_summary(
    signal_set: &WorkContextSignalSet,
    summaries: &[String],
    goals: &[String],
) -> String {
    summaries
        .first()
        .or_else(|| goals.first())
        .cloned()
        .unwrap_or_else(|| {
            format!(
                "Resume prior {} session `{}` for `{}`.",
                signal_set.source_tool.as_str(),
                signal_set.source_session_ref,
                signal_set.title
            )
        })
}

fn current_state_values(
    signal_set: &WorkContextSignalSet,
    summaries: &[String],
    changed_files: &[String],
    verification_results: &[String],
) -> Vec<String> {
    let mut values = Vec::new();
    values.push(
        summaries
            .first()
            .cloned()
            .unwrap_or_else(|| format!("Resume `{}`.", signal_set.title)),
    );
    if !changed_files.is_empty() {
        values.push(format!(
            "Key files touched or discussed: {}.",
            changed_files
                .iter()
                .take(3)
                .cloned()
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    if !verification_results.is_empty() {
        values.push(format!(
            "Latest verification: {}",
            verification_results
                .first()
                .map(String::as_str)
                .unwrap_or_default()
        ));
    }
    values
}

fn next_step_values(remaining_work: &[String]) -> Vec<String> {
    if remaining_work.is_empty() {
        return Vec::new();
    }
    remaining_work.to_vec()
}

fn push_limited_signal_section(
    markdown: &mut String,
    heading: &str,
    values: &[String],
    limit: usize,
) {
    if values.is_empty() {
        return;
    }
    markdown.push_str(&format!("### {heading}\n\n"));
    for value in values.iter().take(limit) {
        markdown.push_str(&format!("- {value}\n"));
    }
    if values.len() > limit {
        markdown.push_str(&format!(
            "- ... {} additional item(s) omitted for concise handoff.\n",
            values.len() - limit
        ));
    }
    markdown.push('\n');
}

fn clean_signal_value(value: &str) -> String {
    truncate_for_signal(
        value
            .trim()
            .trim_matches(|character: char| matches!(character, '`' | '"' | '\'')),
        500,
    )
}

fn truncate_for_signal(text: &str, limit: usize) -> String {
    let mut truncated = String::new();
    for character in text.chars().take(limit) {
        truncated.push(character);
    }
    if text.chars().count() > limit {
        truncated.push_str("...");
    }
    truncated
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session_logs::{
        parse_codex_session_log_detail, CodexSessionIndexEntry, SessionLogMessage,
        SessionLogMetadata,
    };
    use std::{collections::HashMap, fs, path::PathBuf};
    use uuid::Uuid;

    #[test]
    fn signal_set_preserves_required_source_metadata_from_parsed_session_detail() {
        let detail = session_detail("codex");

        let signal_set = WorkContextSignalSet::from_session_detail(&detail)
            .expect("known providers should build a work-context signal set");

        assert_eq!(signal_set.source_tool, SessionLogProvider::Codex);
        assert_eq!(signal_set.source_session_ref, "session-123");
        assert_eq!(signal_set.source_working_directory, "/workspace/app");
        assert_eq!(signal_set.source_log_path, "/logs/session-123.jsonl");
        assert_eq!(signal_set.title, "Implement handoff flow");
        assert_eq!(
            signal_set.updated_at.as_deref(),
            Some("2026-05-11T00:00:00Z")
        );
        assert_eq!(signal_set.message_count, 2);
        assert!(signal_set.signals.iter().any(|signal| {
            signal.kind == WorkContextSignalKind::Summary
                && signal.value == "Implement handoff flow"
        }));
        assert!(signal_set.signals.iter().any(|signal| {
            signal.kind == WorkContextSignalKind::Tag && signal.value == "session-history"
        }));
    }

    #[test]
    fn extracts_work_context_signals_from_parsed_claude_session_messages() {
        let detail = SessionLogDetail {
            summary: SessionLogMetadata {
                provider: "claude".to_string(),
                session_id: "claude-123".to_string(),
                title: "Build Claude handoff extraction".to_string(),
                updated_at: Some("2026-05-11T00:10:00Z".to_string()),
                cwd: Some("/workspace/app".to_string()),
                file_path: PathBuf::from("/logs/claude-123.jsonl"),
                message_count: 3,
                last_user_message: Some("Extract work context".to_string()),
            },
            messages: vec![
                SessionLogMessage {
                    role: "user".to_string(),
                    timestamp: Some("2026-05-11T00:00:00Z".to_string()),
                    content: "Extract work context from crates/ctx-core/src/work_context.rs"
                        .to_string(),
                },
                SessionLogMessage {
                    role: "assistant".to_string(),
                    timestamp: Some("2026-05-11T00:05:00Z".to_string()),
                    content: "\
Summary: Built Claude session signal extraction.
Decision: Keep extraction session-focused instead of generic markdown import.
Changed crates/ctx-core/src/work_context.rs and crates/ctx-core/src/session_logs.rs.
Verified with cargo test -p ctx-core work_context::tests.
Remaining work: wire saved handoff entries into launch.
Dropped noisy tool output. Authorization: Bearer test-token was removed."
                        .to_string(),
                },
                SessionLogMessage {
                    role: "assistant".to_string(),
                    timestamp: Some("2026-05-11T00:06:00Z".to_string()),
                    content: "Decision: Keep extraction session-focused instead of generic markdown import."
                        .to_string(),
                },
            ],
            events: Vec::new(),
            distilled_markdown: "# Handoff".to_string(),
        };

        let signal_set = WorkContextSignalSet::from_session_detail(&detail)
            .expect("Claude details should build extracted signals");

        assert_eq!(signal_set.source_tool, SessionLogProvider::Claude);
        assert_signal(
            &signal_set,
            WorkContextSignalKind::Goal,
            "Extract work context from crates/ctx-core/src/work_context.rs",
        );
        assert_signal(
            &signal_set,
            WorkContextSignalKind::Summary,
            "Built Claude session signal extraction.",
        );
        assert_signal(
            &signal_set,
            WorkContextSignalKind::ChangedFile,
            "crates/ctx-core/src/work_context.rs",
        );
        assert_signal(
            &signal_set,
            WorkContextSignalKind::ChangedFile,
            "crates/ctx-core/src/session_logs.rs",
        );
        assert_signal(
            &signal_set,
            WorkContextSignalKind::Decision,
            "Keep extraction session-focused instead of generic markdown import.",
        );
        assert_signal(
            &signal_set,
            WorkContextSignalKind::VerificationResult,
            "Verified with cargo test -p ctx-core work_context::tests.",
        );
        assert_signal(
            &signal_set,
            WorkContextSignalKind::Command,
            "cargo test -p ctx-core work_context::tests",
        );
        assert_signal(
            &signal_set,
            WorkContextSignalKind::RemainingWork,
            "wire saved handoff entries into launch.",
        );
        assert_signal(
            &signal_set,
            WorkContextSignalKind::SensitiveContent,
            "Dropped noisy tool output. Authorization: Bearer test-token was removed.",
        );
        assert_signal(
            &signal_set,
            WorkContextSignalKind::Noise,
            "Dropped noisy tool output. Authorization: Bearer test-token was removed.",
        );
        assert_eq!(
            signal_set
                .signals
                .iter()
                .filter(|signal| {
                    signal.kind == WorkContextSignalKind::Decision
                        && signal.value
                            == "Keep extraction session-focused instead of generic markdown import."
                })
                .count(),
            1,
            "duplicate decision signals should be deduped"
        );
        let changed_file = signal_set
            .signals
            .iter()
            .find(|signal| {
                signal.kind == WorkContextSignalKind::ChangedFile
                    && signal.value == "crates/ctx-core/src/work_context.rs"
            })
            .expect("changed-file signal should exist");
        assert_eq!(changed_file.evidence.len(), 1);
        assert_eq!(changed_file.evidence[0].message_index, 0);
        assert_eq!(changed_file.evidence[0].role, "user");
    }

    #[test]
    fn extracts_work_context_signals_from_parsed_codex_session_log() {
        let base = std::env::temp_dir().join(format!("ctx-codex-signals-{}", Uuid::new_v4()));
        fs::create_dir_all(&base).expect("fixture directory should be created");
        let log_path = base.join("rollout-fallback-codex.jsonl");
        fs::write(
            &log_path,
            r#"{"type":"session_meta","timestamp":"2026-05-11T00:00:00Z","payload":{"id":"codex-signals-1","cwd":"/workspace/codex-app","timestamp":"2026-05-11T00:00:00Z"}}
{"type":"event_msg","timestamp":"2026-05-11T00:01:00Z","payload":{"type":"user_message","message":"Implement Codex handoff extraction in crates/ctx-core/src/work_context.rs and src-tauri/src/lib.rs"}}
{"type":"event_msg","timestamp":"2026-05-11T00:02:00Z","payload":{"type":"agent_message","message":"Summary: Parsed Codex logs now feed reusable handoff signals.\nDecision: Keep Codex extraction based on session JSONL messages, not generic markdown files.\nChanged files: crates/ctx-core/src/session_logs.rs, crates/ctx-core/src/work_context.rs.\nVerified with cargo test -p ctx-core work_context::tests.\nRemaining work: wire saved Codex handoff entries into launch.\nDropped tool output noise. Process exited with code 0.\nAuthorization: Bearer redacted-token was removed."}}
{"type":"response_item","timestamp":"2026-05-11T00:03:00Z","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"Follow-up: add UI affordance after MVP backend is stable."}]}}
"#,
        )
        .expect("Codex fixture log should be writable");
        let mut index = HashMap::new();
        index.insert(
            "codex-signals-1".to_string(),
            CodexSessionIndexEntry {
                title: "Codex signal extraction".to_string(),
                updated_at: "2026-05-11T00:04:00Z".to_string(),
            },
        );

        let detail = parse_codex_session_log_detail(&log_path, Some(&index))
            .expect("Codex session log should parse into a detail transcript");
        let signal_set = WorkContextSignalSet::from_session_detail(&detail)
            .expect("parsed Codex details should build extracted signals");

        assert_eq!(signal_set.source_tool, SessionLogProvider::Codex);
        assert_eq!(signal_set.source_session_ref, "codex-signals-1");
        assert_eq!(signal_set.source_working_directory, "/workspace/codex-app");
        assert_eq!(signal_set.title, "Codex signal extraction");
        assert_eq!(
            signal_set.updated_at.as_deref(),
            Some("2026-05-11T00:04:00Z")
        );
        assert_eq!(signal_set.message_count, 3);
        assert_signal(
            &signal_set,
            WorkContextSignalKind::Goal,
            "Implement Codex handoff extraction in crates/ctx-core/src/work_context.rs and src-tauri/src/lib.rs",
        );
        assert_signal(
            &signal_set,
            WorkContextSignalKind::Summary,
            "Parsed Codex logs now feed reusable handoff signals.",
        );
        assert_signal(
            &signal_set,
            WorkContextSignalKind::ChangedFile,
            "crates/ctx-core/src/work_context.rs",
        );
        assert_signal(
            &signal_set,
            WorkContextSignalKind::ChangedFile,
            "src-tauri/src/lib.rs",
        );
        assert_signal(
            &signal_set,
            WorkContextSignalKind::Decision,
            "Keep Codex extraction based on session JSONL messages, not generic markdown files.",
        );
        assert_signal(
            &signal_set,
            WorkContextSignalKind::VerificationResult,
            "Verified with cargo test -p ctx-core work_context::tests.",
        );
        assert_signal(
            &signal_set,
            WorkContextSignalKind::Command,
            "cargo test -p ctx-core work_context::tests",
        );
        assert_signal(
            &signal_set,
            WorkContextSignalKind::RemainingWork,
            "wire saved Codex handoff entries into launch.",
        );
        assert_signal(
            &signal_set,
            WorkContextSignalKind::RemainingWork,
            "add UI affordance after MVP backend is stable.",
        );
        assert_signal(
            &signal_set,
            WorkContextSignalKind::Noise,
            "Dropped tool output noise. Process exited with code 0.",
        );
        assert_signal(
            &signal_set,
            WorkContextSignalKind::SensitiveContent,
            "Authorization: Bearer redacted-token was removed.",
        );
        assert!(detail
            .distilled_markdown
            .contains("# Previous Session Context"));

        fs::remove_dir_all(base).ok();
    }

    #[test]
    fn filters_work_relevant_content_and_excludes_noise_and_sensitive_lines() {
        let detail = SessionLogDetail {
            summary: SessionLogMetadata {
                provider: "codex".to_string(),
                session_id: "codex-filter-1".to_string(),
                title: "Filter work context".to_string(),
                updated_at: Some("2026-05-11T00:10:00Z".to_string()),
                cwd: Some("/workspace/app".to_string()),
                file_path: PathBuf::from("/logs/codex-filter-1.jsonl"),
                message_count: 3,
                last_user_message: Some("Implement content filtering".to_string()),
            },
            messages: vec![
                SessionLogMessage {
                    role: "user".to_string(),
                    timestamp: Some("2026-05-11T00:00:00Z".to_string()),
                    content:
                        "Implement work-context filtering in crates/ctx-core/src/work_context.rs"
                            .to_string(),
                },
                SessionLogMessage {
                    role: "assistant".to_string(),
                    timestamp: Some("2026-05-11T00:05:00Z".to_string()),
                    content: "\
Summary: Filtered parsed session records into reusable handoff content.
Decision: Exclude sensitive and noisy records before distillation.
Changed files: crates/ctx-core/src/work_context.rs.
Verified with cargo test -p ctx-core work_context::tests.
Chunk ID: abc123
Authorization: Bearer secret-token"
                        .to_string(),
                },
                SessionLogMessage {
                    role: "assistant".to_string(),
                    timestamp: Some("2026-05-11T00:06:00Z".to_string()),
                    content: "Sounds good.".to_string(),
                },
            ],
            events: Vec::new(),
            distilled_markdown: "# Raw transcript\n\nAuthorization: Bearer secret-token"
                .to_string(),
        };

        let filtered = filter_work_relevant_content(&detail)
            .expect("parsed session detail should produce filtered work content");

        assert!(filtered.cleanup_applied);
        assert!(filtered.included_records.iter().any(|record| record
            .reasons
            .contains(&WorkContextFilterReason::ChangedFile)));
        assert!(filtered
            .excluded_records
            .iter()
            .any(|record| record.reasons.contains(&WorkContextFilterReason::Noise)));
        assert!(filtered.excluded_records.iter().any(|record| record
            .reasons
            .contains(&WorkContextFilterReason::SensitiveContent)));
        assert!(filtered
            .handoff_markdown
            .contains("Filtered parsed session records"));
        assert!(filtered
            .handoff_markdown
            .contains("crates/ctx-core/src/work_context.rs"));
        assert!(filtered.handoff_markdown.contains("## Handoff Summary"));
        assert!(filtered.handoff_markdown.contains("### Key changed files"));
        assert!(filtered.handoff_markdown.contains("### Decisions"));
        assert!(filtered
            .handoff_markdown
            .contains("### Verification results"));
        assert!(!filtered
            .handoff_markdown
            .contains("## Relevant Transcript Excerpts"));
        assert!(!filtered.handoff_markdown.contains("secret-token"));
        assert!(!filtered.handoff_markdown.contains("Chunk ID"));
        assert!(!filtered.handoff_markdown.contains("Sounds good."));
    }

    #[test]
    fn distilled_handoff_summary_is_concise_and_uses_extracted_context_fields() {
        let detail = SessionLogDetail {
            summary: SessionLogMetadata {
                provider: "codex".to_string(),
                session_id: "codex-distill-1".to_string(),
                title: "Distill session handoff".to_string(),
                updated_at: Some("2026-05-11T00:20:00Z".to_string()),
                cwd: Some("/workspace/app".to_string()),
                file_path: PathBuf::from("/logs/codex-distill-1.jsonl"),
                message_count: 4,
                last_user_message: Some("Create a concise handoff summary".to_string()),
            },
            messages: vec![
                SessionLogMessage {
                    role: "user".to_string(),
                    timestamp: Some("2026-05-11T00:00:00Z".to_string()),
                    content: "Create a concise handoff summary from prior session logs."
                        .to_string(),
                },
                SessionLogMessage {
                    role: "assistant".to_string(),
                    timestamp: Some("2026-05-11T00:10:00Z".to_string()),
                    content: "\
Summary: Distilled extracted session signals into a reusable handoff summary.
Changed files: crates/ctx-core/src/work_context.rs.
Decision: Use extracted work signals rather than raw transcript replay.
Verified with cargo test -p ctx-core work_context::tests.
Remaining work: wire saved distilled entries into launch."
                        .to_string(),
                },
                SessionLogMessage {
                    role: "assistant".to_string(),
                    timestamp: Some("2026-05-11T00:11:00Z".to_string()),
                    content: "Wall time: 0.001 seconds\nChunk ID: deadbeef".to_string(),
                },
                SessionLogMessage {
                    role: "assistant".to_string(),
                    timestamp: Some("2026-05-11T00:12:00Z".to_string()),
                    content: "Sounds good, I will continue.".to_string(),
                },
            ],
            events: Vec::new(),
            distilled_markdown: "# Previous Session Context".to_string(),
        };

        let filtered = filter_work_relevant_content(&detail)
            .expect("session detail should distill into handoff markdown");

        assert!(filtered.handoff_markdown.contains("## Handoff Summary"));
        assert!(filtered.handoff_markdown.contains("### Current state"));
        assert!(filtered.handoff_markdown.contains("### Next steps"));
        assert!(filtered
            .handoff_markdown
            .contains("Distilled extracted session signals into a reusable handoff summary."));
        assert!(filtered.handoff_markdown.contains("- Work category: "));
        assert!(filtered
            .handoff_markdown
            .contains("- crates/ctx-core/src/work_context.rs"));
        assert!(filtered
            .handoff_markdown
            .contains("- Use extracted work signals rather than raw transcript replay."));
        assert!(filtered
            .handoff_markdown
            .contains("- cargo test -p ctx-core work_context::tests"));
        assert!(filtered
            .handoff_markdown
            .contains("- wire saved distilled entries into launch."));
        assert!(filtered.handoff_markdown.contains("### Distillation focus"));
        assert!(!filtered.handoff_markdown.contains("Chunk ID"));
        assert!(!filtered.handoff_markdown.contains("Wall time"));
        assert!(!filtered.handoff_markdown.contains("Sounds good"));
        assert!(!filtered
            .handoff_markdown
            .contains("## Relevant Transcript Excerpts"));
    }

    #[test]
    fn handoff_context_maps_signals_to_launch_ready_schema_for_claude_and_codex() {
        let mut signal_set = WorkContextSignalSet::from_session_detail(&session_detail("claude"))
            .expect("Claude details should build a signal set");
        signal_set.signals = vec![
            signal(WorkContextSignalKind::Goal, "Continue the scan flow"),
            signal(WorkContextSignalKind::Summary, "Built the scan flow"),
            signal(
                WorkContextSignalKind::ChangedFile,
                "crates/ctx-core/src/session_logs.rs",
            ),
            signal(WorkContextSignalKind::Command, "cargo test -p ctx-core"),
            signal(
                WorkContextSignalKind::Decision,
                "Keep handoff context session-focused",
            ),
            signal(
                WorkContextSignalKind::VerificationResult,
                "cargo test -p ctx-core",
            ),
            signal(
                WorkContextSignalKind::RemainingWork,
                "Wire launch selection in UI",
            ),
            signal(WorkContextSignalKind::Tag, "implementation"),
            signal(
                WorkContextSignalKind::Noise,
                "Dropped repeated progress chatter",
            ),
        ];

        let claude = SessionHandoffContext::from_signals(
            &signal_set,
            "2026-05-11T00:05:00Z",
            "# Handoff",
            CliTarget::Claude,
            WorkContextRefineMode::Refined,
        );
        let codex = SessionHandoffContext::from_signals(
            &signal_set,
            "2026-05-11T00:05:00Z",
            "# Handoff",
            CliTarget::Codex,
            WorkContextRefineMode::Raw,
        );

        assert_eq!(claude.source_tool, SessionLogProvider::Claude);
        assert_eq!(claude.source_log_path, "/logs/session-123.jsonl");
        assert_eq!(
            claude.source_updated_at.as_deref(),
            Some("2026-05-11T00:00:00Z")
        );
        assert_eq!(claude.category, WorkContextCategory::Implementation);
        assert_eq!(
            claude.categories,
            vec![
                WorkContextCategory::Implementation,
                WorkContextCategory::Launch,
                WorkContextCategory::Verification,
            ]
        );
        assert_eq!(claude.summary, "Built the scan flow");
        assert_eq!(claude.goals, vec!["Continue the scan flow"]);
        assert_eq!(
            claude.key_changed_files,
            vec!["crates/ctx-core/src/session_logs.rs"]
        );
        assert_eq!(claude.commands, vec!["cargo test -p ctx-core"]);
        assert_eq!(
            claude.decisions,
            vec!["Keep handoff context session-focused"]
        );
        assert_eq!(claude.verification_results, vec!["cargo test -p ctx-core"]);
        assert_eq!(claude.remaining_work, vec!["Wire launch selection in UI"]);
        assert!(claude.cleanup_applied);
        assert!(claude.tags.contains(&"session-history".to_string()));
        assert!(claude.tags.contains(&"resume-context".to_string()));
        assert!(claude.tags.contains(&"claude".to_string()));
        assert!(claude.tags.contains(&"implementation".to_string()));
        assert_eq!(
            claude.injection_method,
            InjectionStrategy::AppendSystemPromptFile
        );
        assert_eq!(
            codex.injection_method,
            InjectionStrategy::AgentsMdSectionMarkerMerge
        );
    }

    #[test]
    fn extracts_distilled_handoff_fields_from_classified_session_context() {
        let mut signal_set = WorkContextSignalSet::from_session_detail(&session_detail("codex"))
            .expect("Codex details should build a signal set");
        signal_set.signals = vec![
            signal(
                WorkContextSignalKind::Goal,
                "Continue classified extraction work",
            ),
            signal(
                WorkContextSignalKind::Summary,
                "Derived handoff fields from classified session signals.",
            ),
            signal(
                WorkContextSignalKind::ChangedFile,
                "crates/ctx-core/src/work_context.rs",
            ),
            signal(
                WorkContextSignalKind::Command,
                "cargo test -p ctx-core work_context::tests",
            ),
            signal(
                WorkContextSignalKind::Decision,
                "Use the classification result as the extraction contract.",
            ),
            signal(
                WorkContextSignalKind::VerificationResult,
                "cargo test -p ctx-core work_context::tests",
            ),
            signal(
                WorkContextSignalKind::RemainingWork,
                "Wire the extraction path into save.",
            ),
            signal(WorkContextSignalKind::Tag, "backend"),
            signal(
                WorkContextSignalKind::Noise,
                "Dropped repeated progress chatter.",
            ),
        ];
        let classification = signal_set.classify();

        let fields = extract_distilled_session_handoff_fields(&signal_set, &classification)
            .expect("classified session signals should extract handoff fields");

        assert_eq!(
            fields.goals,
            vec!["Continue classified extraction work".to_string()]
        );
        assert_eq!(
            fields.summary,
            "Derived handoff fields from classified session signals."
        );
        assert_eq!(
            fields.key_changed_files,
            vec!["crates/ctx-core/src/work_context.rs".to_string()]
        );
        assert_eq!(
            fields.decisions,
            vec!["Use the classification result as the extraction contract.".to_string()]
        );
        assert_eq!(
            fields.verification_results,
            vec!["cargo test -p ctx-core work_context::tests".to_string()]
        );
        assert_eq!(
            fields.remaining_work,
            vec!["Wire the extraction path into save.".to_string()]
        );
        assert!(fields.tags.contains(&"backend".to_string()));
        assert!(fields.tags.contains(&"session-history".to_string()));
        assert!(fields.tags.contains(&"resume-context".to_string()));
        assert!(fields.tags.contains(&"codex".to_string()));
        assert!(fields
            .tags
            .contains(&classification.category.as_str().to_string()));
        assert!(fields.cleanup_applied);
    }

    #[test]
    fn handoff_context_maps_refined_markdown_sections_into_saved_fields() {
        let mut signal_set = WorkContextSignalSet::from_session_detail(&session_detail("codex"))
            .expect("Codex details should build a signal set");
        signal_set.signals = vec![
            signal(WorkContextSignalKind::Goal, "Original scan goal"),
            signal(WorkContextSignalKind::Summary, "Original scan summary."),
            signal(
                WorkContextSignalKind::ChangedFile,
                "crates/ctx-core/src/session_logs.rs",
            ),
            signal(WorkContextSignalKind::Command, "cargo test -p ctx-core"),
            signal(WorkContextSignalKind::Decision, "Original scan decision."),
            signal(
                WorkContextSignalKind::VerificationResult,
                "Original verification result.",
            ),
            signal(WorkContextSignalKind::RemainingWork, "Original follow-up."),
        ];
        let classification = signal_set.classify();
        let refined_markdown = "# Previous Session Context\n\n## Handoff Summary\n\nRefined save flow maps the distilled context, not stale scan text.\n\n### Goals\n\n- Save distilled scan output as a reusable handoff entry\n\n### Key changed files\n\n- crates/ctx-core/src/work_context.rs\n\n### Decisions\n\n- Prefer sectioned distilled markdown for saved handoff fields.\n\n### Verification results\n\n- cargo test -p ctx-core work_context::tests\n\n### Remaining work\n\n- Wire the save command through this mapping.";

        let context = SessionHandoffContext::from_classified_signals(
            &signal_set,
            &classification,
            "2026-05-11T00:05:00Z",
            refined_markdown,
            CliTarget::Codex,
            WorkContextRefineMode::Refined,
        )
        .expect("refined markdown should map into saved handoff fields");

        assert_eq!(
            context.summary,
            "Refined save flow maps the distilled context, not stale scan text."
        );
        assert_eq!(
            context.goals,
            vec!["Save distilled scan output as a reusable handoff entry"]
        );
        assert_eq!(
            context.key_changed_files,
            vec!["crates/ctx-core/src/work_context.rs"]
        );
        assert_eq!(
            context.decisions,
            vec!["Prefer sectioned distilled markdown for saved handoff fields."]
        );
        assert_eq!(
            context.verification_results,
            vec!["cargo test -p ctx-core work_context::tests"]
        );
        assert_eq!(
            context.remaining_work,
            vec!["Wire the save command through this mapping."]
        );
        assert_eq!(
            context.classification_status,
            ClassificationStatus::Classified
        );
        assert_eq!(context.refine_mode, WorkContextRefineMode::Refined);
        context
            .validate_for_save()
            .expect("mapped refined handoff context should validate");
    }

    #[test]
    fn classified_handoff_field_extraction_rejects_mismatched_session_metadata() {
        let signal_set = WorkContextSignalSet::from_session_detail(&session_detail("claude"))
            .expect("Claude details should build a signal set");
        let mut classification = signal_set.classify();
        classification.source_session_ref = "different-session".to_string();

        let error = extract_distilled_session_handoff_fields(&signal_set, &classification)
            .expect_err("classification from another session should not extract fields");

        assert!(error.message.contains("classification does not match"));
        assert!(error.message.contains("source_session_ref"));
    }

    #[test]
    fn handoff_context_normalizes_session_detail_into_shared_internal_representation() {
        let detail = SessionLogDetail {
            summary: SessionLogMetadata {
                provider: "claude".to_string(),
                session_id: "claude-normalize-1".to_string(),
                title: "Normalize session signals".to_string(),
                updated_at: Some("2026-05-11T00:10:00Z".to_string()),
                cwd: Some("/workspace/app".to_string()),
                file_path: PathBuf::from("/logs/claude-normalize-1.jsonl"),
                message_count: 2,
                last_user_message: Some("Normalize extracted signals".to_string()),
            },
            messages: vec![SessionLogMessage {
                role: "assistant".to_string(),
                timestamp: Some("2026-05-11T00:11:00Z".to_string()),
                content: "\
Summary: Shared normalized handoff schema is available.
Changed files: crates/ctx-core/src/work_context.rs.
Decision: Normalize from session logs, not generic markdown.
Verified with cargo test -p ctx-core work_context::tests.
Remaining work: connect saved entries."
                    .to_string(),
            }],
            events: Vec::new(),
            distilled_markdown: "# Previous Session Context\n\nShared context body.".to_string(),
        };

        let context = SessionHandoffContext::from_session_detail(
            &detail,
            "2026-05-11T00:12:00Z",
            CliTarget::Claude,
            WorkContextRefineMode::Refined,
        )
        .expect("session detail should normalize into handoff context");

        assert_eq!(context.source_tool, SessionLogProvider::Claude);
        assert_eq!(context.source_session_ref, "claude-normalize-1");
        assert_eq!(context.source_working_directory, "/workspace/app");
        assert_eq!(context.source_log_path, "/logs/claude-normalize-1.jsonl");
        assert_eq!(
            context.source_updated_at.as_deref(),
            Some("2026-05-11T00:10:00Z")
        );
        assert_eq!(context.title, "Normalize session signals");
        assert_eq!(
            context.summary,
            "Shared normalized handoff schema is available."
        );
        assert_eq!(context.goals, vec!["Normalize session signals"]);
        assert_eq!(
            context.key_changed_files,
            vec!["crates/ctx-core/src/work_context.rs"]
        );
        assert_eq!(
            context.commands,
            vec!["cargo test -p ctx-core work_context::tests"]
        );
        assert_eq!(
            context.decisions,
            vec!["Normalize from session logs, not generic markdown."]
        );
        assert_eq!(
            context.verification_results,
            vec!["Verified with cargo test -p ctx-core work_context::tests."]
        );
        assert_eq!(context.remaining_work, vec!["connect saved entries."]);
        assert!(context
            .handoff_markdown
            .contains("# Previous Session Context"));
        assert!(context
            .handoff_markdown
            .contains("Shared normalized handoff schema is available."));
        assert_eq!(
            context.injection_method,
            InjectionStrategy::AppendSystemPromptFile
        );
    }

    #[test]
    fn handoff_context_serializes_with_required_snake_case_fields() {
        let signal_set = WorkContextSignalSet::from_session_detail(&session_detail("codex"))
            .expect("Codex details should build a signal set");
        let context = SessionHandoffContext::from_signals(
            &signal_set,
            "2026-05-11T00:05:00Z",
            "# Handoff",
            CliTarget::Codex,
            WorkContextRefineMode::Raw,
        );

        let json = serde_json::to_value(&context).expect("schema should serialize");

        for key in SESSION_HANDOFF_CONTEXT_REQUIRED_MVP_FIELDS {
            assert!(json.get(*key).is_some(), "missing serialized field {key}");
        }
        for key in [
            "source_log_path",
            "source_updated_at",
            "category",
            "categories",
            "classification_status",
            "classification_confidence_score",
            "classification_rationale",
            "goals",
            "commands",
        ] {
            assert!(json.get(key).is_some(), "missing serialized field {key}");
        }
        assert_eq!(json["source_tool"], "codex");
        assert_eq!(json["category"], "launch");
        assert_eq!(
            json["categories"],
            serde_json::json!(["launch", "implementation"])
        );
        assert_eq!(json["launch_target"], "codex");
        assert_eq!(json["injection_method"], "agents-md-section-marker-merge");
    }

    #[test]
    fn handoff_context_mvp_schema_defines_required_fields_and_rules() {
        let schema_fields = SESSION_HANDOFF_CONTEXT_MVP_SCHEMA
            .iter()
            .map(|field| field.name)
            .collect::<Vec<_>>();

        assert_eq!(schema_fields, SESSION_HANDOFF_CONTEXT_REQUIRED_MVP_FIELDS);
        assert!(SESSION_HANDOFF_CONTEXT_MVP_SCHEMA
            .iter()
            .all(|field| field.required));
        assert!(SESSION_HANDOFF_CONTEXT_MVP_SCHEMA
            .iter()
            .all(|field| !field.description.trim().is_empty()));
        assert!(SESSION_HANDOFF_CONTEXT_MVP_SCHEMA
            .iter()
            .all(|field| !field.validation_rule.trim().is_empty()));

        for field in SESSION_HANDOFF_CONTEXT_REQUIRED_MVP_FIELDS {
            if *field == "handoff_markdown" {
                continue;
            }
            assert!(
                SESSION_HANDOFF_CONTEXT_REQUIRED_FRONTMATTER_FIELDS.contains(field),
                "saved frontmatter must preserve required MVP field {field}"
            );
        }

        let launch_target_rule = SESSION_HANDOFF_CONTEXT_MVP_SCHEMA
            .iter()
            .find(|field| field.name == "injection_method")
            .expect("injection_method should be defined")
            .validation_rule;
        assert!(launch_target_rule.contains("append-system-prompt-file"));
        assert!(launch_target_rule.contains("agents-md-section-marker-merge"));
    }

    #[test]
    fn handoff_context_serializes_to_valid_saved_markdown_frontmatter() {
        let mut signal_set = WorkContextSignalSet::from_session_detail(&session_detail("codex"))
            .expect("Codex details should build a signal set");
        signal_set.signals = vec![
            signal(
                WorkContextSignalKind::Goal,
                "Save distilled session context",
            ),
            signal(
                WorkContextSignalKind::Summary,
                "Implemented reusable handoff serialization.",
            ),
            signal(
                WorkContextSignalKind::ChangedFile,
                "crates/ctx-core/src/work_context.rs",
            ),
            signal(
                WorkContextSignalKind::Decision,
                "Use Markdown frontmatter plus launch-ready body.",
            ),
            signal(
                WorkContextSignalKind::VerificationResult,
                "cargo test -p ctx-core work_context::tests",
            ),
            signal(
                WorkContextSignalKind::RemainingWork,
                "Wire saved entries into launch.",
            ),
        ];
        let context = SessionHandoffContext::from_signals(
            &signal_set,
            "2026-05-11T00:05:00Z",
            "---\ntags: [old]\n---\n\n# Previous Session Context\n\n## Handoff Summary\n\nImplemented reusable handoff serialization.\n\n### Goals\n\n- Save distilled session context\n\n### Key changed files\n\n- crates/ctx-core/src/work_context.rs\n\n### Decisions\n\n- Use Markdown frontmatter plus launch-ready body.\n\n### Verification results\n\n- cargo test -p ctx-core work_context::tests\n\n### Remaining work\n\n- Wire saved entries into launch.",
            CliTarget::Codex,
            WorkContextRefineMode::Raw,
        );

        let markdown = context
            .to_saved_markdown()
            .expect("complete handoff context should serialize for save");

        assert!(markdown.starts_with("---\nsession_handoff_format_version: 1\n"));
        for key in SESSION_HANDOFF_CONTEXT_REQUIRED_FRONTMATTER_FIELDS {
            assert!(
                markdown.contains(&format!("{key}:")),
                "missing saved frontmatter field {key}"
            );
        }
        assert!(markdown.contains("classification: shared\n"));
        assert!(markdown.contains("source_tool: codex\n"));
        assert!(markdown.contains("source_session_ref: \"session-123\"\n"));
        assert!(markdown.contains("source_working_directory: \"/workspace/app\"\n"));
        assert!(markdown.contains("title: \"Implement handoff flow\"\n"));
        assert!(markdown.contains("summary: \"Implemented reusable handoff serialization.\"\n"));
        assert!(markdown.contains("key_changed_files: [\"crates/ctx-core/src/work_context.rs\"]\n"));
        assert!(markdown
            .contains("decisions: [\"Use Markdown frontmatter plus launch-ready body.\"]\n"));
        assert!(markdown
            .contains("verification_results: [\"cargo test -p ctx-core work_context::tests\"]\n"));
        assert!(markdown.contains("remaining_work: [\"Wire saved entries into launch.\"]\n"));
        assert!(markdown.contains("created_at: \"2026-05-11T00:05:00Z\"\n"));
        assert!(markdown.contains("cleanup_applied: false\n"));
        assert!(markdown.contains("refine_mode: raw\n"));
        assert!(markdown.contains("launch_target: codex\n"));
        assert!(markdown.contains("injection_method: agents-md-section-marker-merge\n"));
        assert!(markdown.ends_with("- Wire saved entries into launch."));
        assert!(!markdown.contains("tags: [old]"));
    }

    #[test]
    fn handoff_context_round_trips_from_saved_markdown_schema() {
        let mut signal_set = WorkContextSignalSet::from_session_detail(&session_detail("claude"))
            .expect("Claude details should build a signal set");
        signal_set.signals = vec![
            signal(
                WorkContextSignalKind::Goal,
                "Save distilled session context",
            ),
            signal(
                WorkContextSignalKind::Summary,
                "Implemented reusable handoff persistence.",
            ),
            signal(
                WorkContextSignalKind::ChangedFile,
                "crates/ctx-core/src/work_context.rs",
            ),
            signal(
                WorkContextSignalKind::Command,
                "cargo test -p ctx-core work_context::tests",
            ),
            signal(
                WorkContextSignalKind::Decision,
                "Round-trip the saved schema instead of parsing generic markdown.",
            ),
            signal(
                WorkContextSignalKind::VerificationResult,
                "cargo test -p ctx-core work_context::tests",
            ),
            signal(
                WorkContextSignalKind::RemainingWork,
                "Wire storage helpers into launch.",
            ),
        ];
        let context = SessionHandoffContext::from_signals(
            &signal_set,
            "2026-05-11T00:05:00Z",
            "# Previous Session Context\n\n## Handoff Summary\n\nImplemented reusable handoff persistence.\n\n### Goals\n\n- Save distilled session context\n\n### Key changed files\n\n- crates/ctx-core/src/work_context.rs\n\n### Commands\n\n- cargo test -p ctx-core work_context::tests\n\n### Decisions\n\n- Round-trip the saved schema instead of parsing generic markdown.\n\n### Verification results\n\n- cargo test -p ctx-core work_context::tests\n\n### Remaining work\n\n- Wire storage helpers into launch.",
            CliTarget::Claude,
            WorkContextRefineMode::Refined,
        );
        let markdown = context
            .to_saved_markdown()
            .expect("context should serialize");

        let parsed = SessionHandoffContext::from_saved_markdown(&markdown)
            .expect("saved context should parse");

        assert_eq!(parsed, context);
        assert_eq!(
            parsed.injection_method,
            InjectionStrategy::AppendSystemPromptFile
        );
        assert_eq!(parsed.launch_target, CliTarget::Claude);
    }

    #[test]
    fn handoff_context_validation_rejects_missing_required_save_fields() {
        let signal_set = WorkContextSignalSet::from_session_detail(&session_detail("claude"))
            .expect("Claude details should build a signal set");
        let mut context = SessionHandoffContext::from_signals(
            &signal_set,
            "",
            "  ",
            CliTarget::Claude,
            WorkContextRefineMode::Raw,
        );
        context.source_working_directory.clear();
        context.summary.clear();
        context.tags.clear();

        let error = context
            .validate_for_save()
            .expect_err("incomplete handoff context should fail validation");

        assert!(error.message.contains("source_working_directory"));
        assert!(error.message.contains("summary"));
        assert!(error.message.contains("created_at"));
        assert!(error.message.contains("handoff_markdown"));
        assert!(error.message.contains("tags"));
    }

    #[test]
    fn handoff_context_validation_rejects_mismatched_injection_method() {
        let signal_set = WorkContextSignalSet::from_session_detail(&session_detail("codex"))
            .expect("Codex details should build a signal set");
        let mut context = SessionHandoffContext::from_signals(
            &signal_set,
            "2026-05-11T00:05:00Z",
            "# Handoff",
            CliTarget::Codex,
            WorkContextRefineMode::Raw,
        );
        context.injection_method = InjectionStrategy::AppendSystemPromptFile;

        let error = context
            .validate_for_save()
            .expect_err("mismatched launch injection should fail validation");

        assert!(error.message.contains("injection_method"));
        assert!(error.message.contains("launch_target"));
    }

    #[test]
    fn handoff_context_validation_rejects_unclassified_contexts() {
        let signal_set = WorkContextSignalSet::from_session_detail(&session_detail("codex"))
            .expect("Codex details should build a signal set");
        let mut context = SessionHandoffContext::from_signals(
            &signal_set,
            "2026-05-11T00:05:00Z",
            "# Handoff",
            CliTarget::Codex,
            WorkContextRefineMode::Raw,
        );
        context.classification_status = ClassificationStatus::Pending;

        let error = context
            .validate_for_save()
            .expect_err("pending handoff classification should fail validation");

        assert!(error.message.contains("classified before saving"));
    }

    #[test]
    fn handoff_context_validation_rejects_distilled_markdown_over_concision_limits() {
        let signal_set = WorkContextSignalSet::from_session_detail(&session_detail("codex"))
            .expect("Codex details should build a signal set");
        let mut context = SessionHandoffContext::from_signals(
            &signal_set,
            "2026-05-11T00:05:00Z",
            format!(
                "Implement handoff flow\n{}",
                "x".repeat(MAX_HANDOFF_MARKDOWN_CHARS + 1)
            ),
            CliTarget::Codex,
            WorkContextRefineMode::Raw,
        );
        context.summary = "s".repeat(MAX_HANDOFF_SUMMARY_CHARS + 1);

        let error = context
            .validate_for_save()
            .expect_err("oversized handoff context should fail validation");

        assert!(error.message.contains("concision limit"));
        assert!(error.message.contains("handoff_markdown"));
        assert!(error.message.contains("summary"));
    }

    #[test]
    fn handoff_context_validation_rejects_distilled_markdown_missing_essential_context() {
        let mut signal_set = WorkContextSignalSet::from_session_detail(&session_detail("codex"))
            .expect("Codex details should build a signal set");
        signal_set.signals = vec![
            signal(
                WorkContextSignalKind::Goal,
                "Save distilled session context",
            ),
            signal(
                WorkContextSignalKind::Summary,
                "Implemented reusable handoff serialization.",
            ),
            signal(
                WorkContextSignalKind::ChangedFile,
                "crates/ctx-core/src/work_context.rs",
            ),
            signal(
                WorkContextSignalKind::Decision,
                "Keep the saved body launch-ready.",
            ),
            signal(
                WorkContextSignalKind::VerificationResult,
                "cargo test -p ctx-core work_context::tests",
            ),
            signal(
                WorkContextSignalKind::RemainingWork,
                "Wire saved entries into launch.",
            ),
        ];
        let context = SessionHandoffContext::from_signals(
            &signal_set,
            "2026-05-11T00:05:00Z",
            "# Previous Session Context\n\nImplemented reusable handoff serialization.",
            CliTarget::Codex,
            WorkContextRefineMode::Raw,
        );

        let error = context
            .validate_for_save()
            .expect_err("handoff markdown missing essential fields should fail validation");

        assert!(error.message.contains("lost essential"));
        assert!(error.message.contains("key_changed_files"));
        assert!(error.message.contains("decisions"));
        assert!(error.message.contains("verification_results"));
        assert!(error.message.contains("remaining_work"));
    }

    #[test]
    fn handoff_context_validation_rejects_unreadable_unstructured_handoff_markdown() {
        let mut signal_set = WorkContextSignalSet::from_session_detail(&session_detail("codex"))
            .expect("Codex details should build a signal set");
        signal_set.signals = vec![
            signal(
                WorkContextSignalKind::Goal,
                "Save distilled session context",
            ),
            signal(
                WorkContextSignalKind::Summary,
                "Implemented reusable handoff serialization.",
            ),
            signal(
                WorkContextSignalKind::ChangedFile,
                "crates/ctx-core/src/work_context.rs",
            ),
            signal(
                WorkContextSignalKind::Decision,
                "Keep the saved body launch-ready.",
            ),
            signal(
                WorkContextSignalKind::VerificationResult,
                "cargo test -p ctx-core work_context::tests",
            ),
            signal(
                WorkContextSignalKind::RemainingWork,
                "Wire saved entries into launch.",
            ),
        ];
        let context = SessionHandoffContext::from_signals(
            &signal_set,
            "2026-05-11T00:05:00Z",
            "Previous session context. Save distilled session context. Implemented reusable handoff serialization. crates/ctx-core/src/work_context.rs. Keep the saved body launch-ready. cargo test -p ctx-core work_context::tests. Wire saved entries into launch.",
            CliTarget::Codex,
            WorkContextRefineMode::Raw,
        );

        let error = context
            .validate_for_save()
            .expect_err("unstructured handoff markdown should fail readability validation");

        assert!(error.message.contains("readable/actionable"));
        assert!(error.message.contains("top-level markdown heading"));
        assert!(error.message.contains("bullet lists"));
    }

    #[test]
    fn handoff_context_validation_rejects_markdown_without_future_session_sections() {
        let mut signal_set = WorkContextSignalSet::from_session_detail(&session_detail("codex"))
            .expect("Codex details should build a signal set");
        signal_set.signals = vec![
            signal(
                WorkContextSignalKind::Goal,
                "Save distilled session context",
            ),
            signal(
                WorkContextSignalKind::Summary,
                "Implemented reusable handoff serialization.",
            ),
            signal(
                WorkContextSignalKind::ChangedFile,
                "crates/ctx-core/src/work_context.rs",
            ),
            signal(
                WorkContextSignalKind::Decision,
                "Keep the saved body launch-ready.",
            ),
            signal(
                WorkContextSignalKind::VerificationResult,
                "cargo test -p ctx-core work_context::tests",
            ),
            signal(
                WorkContextSignalKind::RemainingWork,
                "Wire saved entries into launch.",
            ),
        ];
        let context = SessionHandoffContext::from_signals(
            &signal_set,
            "2026-05-11T00:05:00Z",
            "# Previous Session Context\n\n## Reusable Notes\n\nImplemented reusable handoff serialization.\n\n### Carry Forward\n\n- Save distilled session context\n- crates/ctx-core/src/work_context.rs\n- Keep the saved body launch-ready.\n- cargo test -p ctx-core work_context::tests\n- Wire saved entries into launch.",
            CliTarget::Codex,
            WorkContextRefineMode::Raw,
        );

        let error = context
            .validate_for_save()
            .expect_err("handoff markdown without future-session sections should fail");

        assert!(error.message.contains("goals section"));
        assert!(error.message.contains("current state or summary"));
        assert!(error.message.contains("decisions"));
        assert!(error.message.contains("next steps or remaining work"));
    }

    #[test]
    fn handoff_context_validation_accepts_clear_structured_actionable_markdown() {
        let mut signal_set = WorkContextSignalSet::from_session_detail(&session_detail("codex"))
            .expect("Codex details should build a signal set");
        signal_set.signals = vec![
            signal(
                WorkContextSignalKind::Goal,
                "Save distilled session context",
            ),
            signal(
                WorkContextSignalKind::Summary,
                "Implemented reusable handoff serialization.",
            ),
            signal(
                WorkContextSignalKind::ChangedFile,
                "crates/ctx-core/src/work_context.rs",
            ),
            signal(
                WorkContextSignalKind::Decision,
                "Keep the saved body launch-ready.",
            ),
            signal(
                WorkContextSignalKind::VerificationResult,
                "cargo test -p ctx-core work_context::tests",
            ),
            signal(
                WorkContextSignalKind::RemainingWork,
                "Wire saved entries into launch.",
            ),
        ];
        let context = SessionHandoffContext::from_signals(
            &signal_set,
            "2026-05-11T00:05:00Z",
            "# Previous Session Context\n\n## Handoff Summary\n\nImplemented reusable handoff serialization.\n\n### Goals\n\n- Save distilled session context\n\n### Key changed files\n\n- crates/ctx-core/src/work_context.rs\n\n### Decisions\n\n- Keep the saved body launch-ready.\n\n### Verification results\n\n- cargo test -p ctx-core work_context::tests\n\n### Remaining work\n\n- Wire saved entries into launch.",
            CliTarget::Codex,
            WorkContextRefineMode::Raw,
        );

        context
            .validate_for_save()
            .expect("structured readable handoff markdown should save");
    }

    #[test]
    fn handoff_context_normalizes_structured_fields_before_save() {
        let mut signal_set = WorkContextSignalSet::from_session_detail(&session_detail("codex"))
            .expect("Codex details should build a signal set");
        signal_set.signals = vec![
            signal(
                WorkContextSignalKind::Goal,
                "  - Normalize structured fields  ",
            ),
            signal(
                WorkContextSignalKind::Summary,
                " Normalized   structured handoff fields. ",
            ),
            signal(
                WorkContextSignalKind::ChangedFile,
                " `crates\\ctx-core\\src\\work_context.rs`, ",
            ),
            signal(
                WorkContextSignalKind::ChangedFile,
                "crates/ctx-core/src/work_context.rs",
            ),
            signal(
                WorkContextSignalKind::Decision,
                "  - Keep   structured fields normalized. ",
            ),
            signal(
                WorkContextSignalKind::VerificationResult,
                " cargo   test -p ctx-core work_context::tests ",
            ),
            signal(
                WorkContextSignalKind::RemainingWork,
                " Add   launch coverage. ",
            ),
            signal(WorkContextSignalKind::Tag, " Frontend.UI "),
            signal(WorkContextSignalKind::Tag, "frontend-ui"),
        ];

        let context = SessionHandoffContext::from_signals(
            &signal_set,
            "2026-05-11T00:05:00Z",
            "# Previous Session Context\n\n## Handoff Summary\n\nNormalized structured handoff fields.\n\n### Goals\n\n- Normalize structured fields\n\n### Key changed files\n\n- crates/ctx-core/src/work_context.rs\n\n### Decisions\n\n- Keep structured fields normalized.\n\n### Verification results\n\n- cargo test -p ctx-core work_context::tests\n\n### Remaining work\n\n- Add launch coverage.",
            CliTarget::Codex,
            WorkContextRefineMode::Raw,
        );

        assert_eq!(context.goals, vec!["Normalize structured fields"]);
        assert_eq!(context.summary, "Normalized structured handoff fields.");
        assert_eq!(
            context.key_changed_files,
            vec!["crates/ctx-core/src/work_context.rs"]
        );
        assert_eq!(
            context.decisions,
            vec!["Keep structured fields normalized."]
        );
        assert_eq!(
            context.verification_results,
            vec!["cargo test -p ctx-core work_context::tests"]
        );
        assert_eq!(context.remaining_work, vec!["Add launch coverage."]);
        assert_eq!(
            context
                .tags
                .iter()
                .filter(|tag| tag.as_str() == "frontend-ui")
                .count(),
            1
        );

        let markdown = context
            .to_saved_markdown()
            .expect("normalized structured fields should serialize");

        assert!(markdown.contains("key_changed_files: [\"crates/ctx-core/src/work_context.rs\"]"));
        assert!(markdown.contains("decisions: [\"Keep structured fields normalized.\"]"));
        assert!(markdown
            .contains("verification_results: [\"cargo test -p ctx-core work_context::tests\"]"));
        assert!(markdown.contains("frontend-ui"));
        assert!(!markdown.contains("crates\\ctx-core"));
        assert!(!markdown.contains("Frontend.UI"));
    }

    #[test]
    fn handoff_context_validation_rejects_malformed_structured_fields() {
        let mut signal_set = WorkContextSignalSet::from_session_detail(&session_detail("codex"))
            .expect("Codex details should build a signal set");
        signal_set.signals = vec![
            signal(
                WorkContextSignalKind::Goal,
                "Save distilled session context",
            ),
            signal(
                WorkContextSignalKind::Summary,
                "Implemented reusable handoff serialization.",
            ),
            signal(
                WorkContextSignalKind::ChangedFile,
                "crates/ctx-core/src/work_context.rs",
            ),
            signal(
                WorkContextSignalKind::Decision,
                "Keep the saved body launch-ready.",
            ),
            signal(
                WorkContextSignalKind::VerificationResult,
                "cargo test -p ctx-core work_context::tests",
            ),
            signal(
                WorkContextSignalKind::RemainingWork,
                "Wire saved entries into launch.",
            ),
        ];
        let mut context = SessionHandoffContext::from_signals(
            &signal_set,
            "2026-05-11T00:05:00Z",
            "# Previous Session Context\n\n## Handoff Summary\n\nImplemented reusable handoff serialization.\n\n### Goals\n\n- Save distilled session context\n\n### Key changed files\n\n- crates/ctx-core/src/work_context.rs\n\n### Decisions\n\n- Keep the saved body launch-ready.\n\n### Verification results\n\n- cargo test -p ctx-core work_context::tests\n\n### Remaining work\n\n- Wire saved entries into launch.",
            CliTarget::Codex,
            WorkContextRefineMode::Raw,
        );
        context.tags.push("bad\n---\nfrontmatter".to_string());

        let error = context
            .validate_for_save()
            .expect_err("frontmatter delimiter in structured field should fail");

        assert!(error.message.contains("tags"));
        assert!(error.message.contains("frontmatter delimiter"));
    }

    #[test]
    fn work_context_taxonomy_defines_supported_session_categories() {
        let categories = WORK_CONTEXT_CATEGORY_TAXONOMY
            .iter()
            .map(|definition| definition.category)
            .collect::<Vec<_>>();

        assert_eq!(
            categories,
            vec![
                WorkContextCategory::Implementation,
                WorkContextCategory::Debugging,
                WorkContextCategory::Review,
                WorkContextCategory::Planning,
                WorkContextCategory::Refactor,
                WorkContextCategory::Research,
                WorkContextCategory::Verification,
                WorkContextCategory::Launch,
                WorkContextCategory::General,
            ]
        );
        for definition in WORK_CONTEXT_CATEGORY_TAXONOMY {
            assert!(!definition.label.is_empty());
            assert!(!definition.description.is_empty());
            assert!(
                !definition.distillation_focus.is_empty(),
                "category {:?} must define distillation focus",
                definition.category
            );
        }
    }

    #[test]
    fn classification_result_serializes_versioned_output_format() {
        let mut signal_set = WorkContextSignalSet::from_session_detail(&session_detail("codex"))
            .expect("Codex details should build a signal set");
        signal_set.signals = vec![
            signal(WorkContextSignalKind::Goal, "Build session launch handoff"),
            signal(
                WorkContextSignalKind::Summary,
                "Implemented session launch handoff",
            ),
            signal(
                WorkContextSignalKind::ChangedFile,
                "crates/ctx-core/src/work_context.rs",
            ),
            signal(
                WorkContextSignalKind::Command,
                "cargo test -p ctx-core work_context::tests",
            ),
            signal(
                WorkContextSignalKind::Decision,
                "Use AGENTS.md managed block injection for Codex launch",
            ),
            signal(
                WorkContextSignalKind::VerificationResult,
                "cargo test -p ctx-core work_context::tests",
            ),
            signal(
                WorkContextSignalKind::RemainingWork,
                "Add save/list commands",
            ),
        ];

        let result = signal_set.classify();
        let json = serde_json::to_value(&result).expect("classification should serialize");

        assert_eq!(result.category, WorkContextCategory::Launch);
        assert_eq!(
            result.categories,
            vec![
                WorkContextCategory::Launch,
                WorkContextCategory::Implementation,
                WorkContextCategory::Verification,
            ]
        );
        assert_eq!(
            result.output_format_version,
            WORK_CONTEXT_CLASSIFICATION_OUTPUT_FORMAT_VERSION
        );
        assert_eq!(result.status, ClassificationStatus::Classified);
        assert!(result.confidence_score >= 80);
        assert!(result.suggested_tags.contains(&"launch".to_string()));
        assert!(result
            .distillation_focus
            .contains(&"injection method".to_string()));
        for key in [
            "output_format_version",
            "source_tool",
            "source_session_ref",
            "source_working_directory",
            "source_log_path",
            "title",
            "category",
            "categories",
            "status",
            "confidence_score",
            "rationale",
            "signal_counts",
            "suggested_tags",
            "distillation_focus",
        ] {
            assert!(json.get(key).is_some(), "missing serialized field {key}");
        }
        assert_eq!(json["source_tool"], "codex");
        assert_eq!(json["category"], "launch");
        assert_eq!(
            json["categories"],
            serde_json::json!(["launch", "implementation", "verification"])
        );
        assert_eq!(json["status"], "classified");
        assert_eq!(json["signal_counts"]["goals"], 1);
        assert_eq!(json["signal_counts"]["changed_files"], 1);
        assert_eq!(json["signal_counts"]["commands"], 1);
    }

    #[test]
    fn classification_result_assigns_one_or_more_categories_to_sessions() {
        let mut review = WorkContextSignalSet::from_session_detail(&session_detail("claude"))
            .expect("Claude details should build a signal set");
        review.signals = vec![
            signal(WorkContextSignalKind::Summary, "Reviewed regression risk"),
            signal(
                WorkContextSignalKind::Decision,
                "Finding: missing test coverage should block merge",
            ),
        ];

        let mut debugging = WorkContextSignalSet::from_session_detail(&session_detail("codex"))
            .expect("Codex details should build a signal set");
        debugging.signals = vec![
            signal(WorkContextSignalKind::Summary, "Fixed failing parser bug"),
            signal(
                WorkContextSignalKind::VerificationResult,
                "cargo test -p ctx-core session_logs::tests passed",
            ),
        ];

        let mut general = WorkContextSignalSet::from_session_detail(&session_detail("claude"))
            .expect("Claude details should build a signal set");
        general.title = "Synced context".to_string();
        general.signals = vec![signal(WorkContextSignalKind::Summary, "Synced context")];

        let review_result = review.classify();
        let debugging_result = debugging.classify();
        let general_result = general.classify();

        assert_eq!(review_result.category, WorkContextCategory::Review);
        assert_eq!(
            review_result.categories,
            vec![
                WorkContextCategory::Review,
                WorkContextCategory::Launch,
                WorkContextCategory::Implementation,
            ]
        );
        assert_eq!(debugging_result.category, WorkContextCategory::Debugging);
        assert_eq!(
            debugging_result.categories,
            vec![
                WorkContextCategory::Debugging,
                WorkContextCategory::Launch,
                WorkContextCategory::Implementation,
                WorkContextCategory::Verification,
            ]
        );
        assert_eq!(general_result.category, WorkContextCategory::General);
        assert_eq!(
            general_result.categories,
            vec![WorkContextCategory::General]
        );
        for result in [review_result, debugging_result, general_result] {
            assert!(
                !result.categories.is_empty(),
                "each classified parsed session must have at least one category"
            );
        }
    }

    #[test]
    fn unknown_session_provider_fails_schema_construction() {
        let detail = session_detail("other");

        let error = WorkContextSignalSet::from_session_detail(&detail)
            .expect_err("only Claude and Codex logs are valid sources");

        assert!(error.message.contains("unsupported session log provider"));
    }

    fn session_detail(provider: &str) -> SessionLogDetail {
        SessionLogDetail {
            summary: SessionLogMetadata {
                provider: provider.to_string(),
                session_id: "session-123".to_string(),
                title: "Implement handoff flow".to_string(),
                updated_at: Some("2026-05-11T00:00:00Z".to_string()),
                cwd: Some("/workspace/app".to_string()),
                file_path: PathBuf::from("/logs/session-123.jsonl"),
                message_count: 2,
                last_user_message: Some("Implement handoff flow".to_string()),
            },
            messages: vec![
                SessionLogMessage {
                    role: "user".to_string(),
                    timestamp: Some("2026-05-11T00:00:00Z".to_string()),
                    content: "Implement handoff flow".to_string(),
                },
                SessionLogMessage {
                    role: "assistant".to_string(),
                    timestamp: Some("2026-05-11T00:01:00Z".to_string()),
                    content: "Done".to_string(),
                },
            ],
            events: Vec::new(),
            distilled_markdown: "# Handoff".to_string(),
        }
    }

    fn signal(kind: WorkContextSignalKind, value: &str) -> WorkContextSignal {
        WorkContextSignal {
            kind,
            value: value.to_string(),
            confidence: 90,
            evidence: Vec::new(),
        }
    }

    fn assert_signal(
        signal_set: &WorkContextSignalSet,
        kind: WorkContextSignalKind,
        expected_value: &str,
    ) {
        assert!(
            signal_set
                .signals
                .iter()
                .any(|signal| signal.kind == kind && signal.value == expected_value),
            "missing {kind:?} signal with value {expected_value:?}; signals: {:?}",
            signal_set.signals
        );
    }
}
