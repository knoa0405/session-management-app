use super::models::{
    Classification, ClassificationStatus, CliTarget, ImportSourceType, VaultScope,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Child, Command, ExitStatus, Stdio},
    thread,
    time::{Duration, Instant},
};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct ContextClassificationRule {
    pub id: &'static str,
    pub classification: Classification,
    pub filename_matches: &'static [&'static str],
    pub directory_segments: &'static [&'static str],
    pub file_stem_tokens: &'static [&'static str],
    pub rationale: &'static str,
}

#[derive(Debug, Clone)]
pub struct ClassificationSuggestion {
    pub classification: Classification,
    pub status: ClassificationStatus,
    pub rationale: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Eq, PartialEq)]
pub struct ImportTimeClassificationRequest {
    pub content: String,
    #[serde(default)]
    pub file_name: Option<String>,
    #[serde(default)]
    pub folder_path: Option<PathBuf>,
    #[serde(default)]
    pub import_source_type: Option<ImportSourceType>,
    #[serde(default)]
    pub existing_tags: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Eq, PartialEq)]
pub struct ImportTimeClassificationResult {
    pub classification: Classification,
    pub status: ClassificationStatus,
    pub confidence_score: u8,
    pub rationale: String,
    #[serde(default)]
    pub rule_id: Option<String>,
    #[serde(default)]
    pub suggested_tags: Vec<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum HeadlessClassificationAdapterKind {
    Claude,
    Codex,
}

impl From<CliTarget> for HeadlessClassificationAdapterKind {
    fn from(target: CliTarget) -> Self {
        match target {
            CliTarget::Claude => Self::Claude,
            CliTarget::Codex => Self::Codex,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Eq, PartialEq)]
pub struct HeadlessClassificationRequest {
    pub request_id: Uuid,
    pub target_cli: CliTarget,
    pub context_id: Option<Uuid>,
    pub title: Option<String>,
    pub content: String,
    pub file_path: PathBuf,
    pub vault_scope: Option<VaultScope>,
    pub folder_path: PathBuf,
    pub import_source: Option<PathBuf>,
    pub import_source_type: Option<ImportSourceType>,
    #[serde(default)]
    pub existing_tags: Vec<String>,
    #[serde(default)]
    pub existing_wikilinks: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Eq, PartialEq)]
pub struct HeadlessClassificationResult {
    pub request_id: Uuid,
    pub adapter_kind: HeadlessClassificationAdapterKind,
    pub classification: Classification,
    pub status: ClassificationStatus,
    pub confidence_score: u8,
    pub rationale: String,
    pub suggested_title: Option<String>,
    #[serde(default)]
    pub suggested_tags: Vec<String>,
    pub suggested_folder_path: Option<PathBuf>,
    #[serde(default)]
    pub detected_wikilinks: Vec<String>,
    #[serde(default)]
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Eq, PartialEq)]
pub struct HeadlessClassificationError {
    pub adapter_kind: HeadlessClassificationAdapterKind,
    pub message: String,
    #[serde(default)]
    pub retryable: bool,
}

impl std::fmt::Display for HeadlessClassificationError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}", self.message)
    }
}

impl std::error::Error for HeadlessClassificationError {}

pub trait HeadlessClassificationAdapter {
    fn adapter_kind(&self) -> HeadlessClassificationAdapterKind;

    fn analyze_context(
        &self,
        request: &HeadlessClassificationRequest,
    ) -> Result<HeadlessClassificationResult, HeadlessClassificationError>;
}

#[derive(Debug, Clone, Serialize, Deserialize, Eq, PartialEq)]
pub struct LocalHeadlessCliClassificationAdapter {
    pub target_cli: CliTarget,
    #[serde(default)]
    pub program: Option<String>,
    #[serde(default)]
    pub working_dir: Option<PathBuf>,
    #[serde(default)]
    pub timeout_ms: Option<u64>,
    #[serde(default)]
    pub passthrough_args: Vec<String>,
}

impl HeadlessClassificationAdapter for LocalHeadlessCliClassificationAdapter {
    fn adapter_kind(&self) -> HeadlessClassificationAdapterKind {
        HeadlessClassificationAdapterKind::from(self.target_cli)
    }

    fn analyze_context(
        &self,
        request: &HeadlessClassificationRequest,
    ) -> Result<HeadlessClassificationResult, HeadlessClassificationError> {
        let output = run_noninteractive_cli_process(&NoninteractiveCliProcessRequest {
            target_cli: self.target_cli,
            program: self.program.clone(),
            working_dir: self.working_dir.clone(),
            timeout_ms: self.timeout_ms,
            context_content: build_headless_classification_prompt(request),
            passthrough_args: self.passthrough_args.clone(),
        })
        .map_err(|error| HeadlessClassificationError {
            adapter_kind: self.adapter_kind(),
            message: error.message,
            retryable: error.retryable,
        })?;

        parse_headless_classification_cli_output(request, self.adapter_kind(), &output)
    }
}

pub fn build_headless_classification_prompt(request: &HeadlessClassificationRequest) -> String {
    format!(
        r#"Classify this markdown context for CTX import.

Return exactly one JSON object with these fields:
- classification: one of "main-agent", "subagent", or "shared"
- confidence_score: integer 0-100
- rationale: concise reason for the suggestion
- suggested_title: optional display title
- suggested_tags: optional array of tags
- suggested_folder_path: optional relative vault folder
- detected_wikilinks: optional array of outgoing wikilink targets
- warnings: optional array of review notes

Context metadata:
- target_cli: {target_cli:?}
- title: {title}
- file_path: {file_path}
- folder_path: {folder_path}
- import_source_type: {import_source_type}
- existing_tags: {existing_tags}
- existing_wikilinks: {existing_wikilinks}

Markdown:
```markdown
{content}
```
"#,
        target_cli = request.target_cli,
        title = request.title.as_deref().unwrap_or(""),
        file_path = request.file_path.display(),
        folder_path = request.folder_path.display(),
        import_source_type = request
            .import_source_type
            .map(|source_type| format!("{source_type:?}"))
            .unwrap_or_else(|| "unknown".to_string()),
        existing_tags = request.existing_tags.join(", "),
        existing_wikilinks = request.existing_wikilinks.join(", "),
        content = request.content
    )
}

pub fn parse_headless_classification_cli_output(
    request: &HeadlessClassificationRequest,
    adapter_kind: HeadlessClassificationAdapterKind,
    output: &NoninteractiveCliProcessOutput,
) -> Result<HeadlessClassificationResult, HeadlessClassificationError> {
    parse_headless_classification_text(request, adapter_kind, &output.stdout)
}

pub fn parse_headless_classification_text(
    request: &HeadlessClassificationRequest,
    adapter_kind: HeadlessClassificationAdapterKind,
    text: &str,
) -> Result<HeadlessClassificationResult, HeadlessClassificationError> {
    let value =
        extract_classification_json_value(text).map_err(|message| HeadlessClassificationError {
            adapter_kind,
            message,
            retryable: true,
        })?;

    normalize_headless_classification_value(request, adapter_kind, &value)
}

#[derive(Debug, Clone, Serialize, Deserialize, Eq, PartialEq)]
pub struct NoninteractiveCliProcessRequest {
    pub target_cli: CliTarget,
    #[serde(default)]
    pub program: Option<String>,
    #[serde(default)]
    pub working_dir: Option<PathBuf>,
    #[serde(default)]
    pub timeout_ms: Option<u64>,
    pub context_content: String,
    #[serde(default)]
    pub passthrough_args: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Eq, PartialEq)]
pub struct NoninteractiveCliProcessOutput {
    pub target_cli: CliTarget,
    pub program: String,
    pub args: Vec<String>,
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Eq, PartialEq)]
pub struct NoninteractiveCliProcessError {
    pub target_cli: CliTarget,
    pub program: String,
    pub args: Vec<String>,
    pub message: String,
    #[serde(default)]
    pub stdout: String,
    #[serde(default)]
    pub stderr: String,
    #[serde(default)]
    pub retryable: bool,
    #[serde(default)]
    pub timed_out: bool,
}

impl std::fmt::Display for NoninteractiveCliProcessError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}", self.message)
    }
}

impl std::error::Error for NoninteractiveCliProcessError {}

pub fn noninteractive_cli_program(target_cli: CliTarget) -> &'static str {
    match target_cli {
        CliTarget::Claude => "claude",
        CliTarget::Codex => "codex",
    }
}

pub fn noninteractive_cli_args(target_cli: CliTarget, passthrough_args: &[String]) -> Vec<String> {
    let mut args = match target_cli {
        CliTarget::Claude => vec!["--print".to_string()],
        CliTarget::Codex => vec!["exec".to_string(), "-".to_string()],
    };
    args.extend(
        passthrough_args
            .iter()
            .map(|arg| arg.trim().to_string())
            .filter(|arg| !arg.is_empty()),
    );
    args
}

pub fn run_noninteractive_cli_process(
    request: &NoninteractiveCliProcessRequest,
) -> Result<NoninteractiveCliProcessOutput, NoninteractiveCliProcessError> {
    let program = request
        .program
        .clone()
        .unwrap_or_else(|| noninteractive_cli_program(request.target_cli).to_string());
    let args = noninteractive_cli_args(request.target_cli, &request.passthrough_args);
    let mut command = Command::new(&program);
    command
        .args(&args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(working_dir) = &request.working_dir {
        command.current_dir(working_dir);
    }

    let mut child = command
        .spawn()
        .map_err(|error| NoninteractiveCliProcessError {
            target_cli: request.target_cli,
            program: program.clone(),
            args: args.clone(),
            message: format!(
                "failed to start {} noninteractive process: {error}",
                noninteractive_cli_program(request.target_cli)
            ),
            stdout: String::new(),
            stderr: String::new(),
            retryable: false,
            timed_out: false,
        })?;

    let stdout_handle = child.stdout.take().map(read_pipe_to_string);
    let stderr_handle = child.stderr.take().map(read_pipe_to_string);

    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| NoninteractiveCliProcessError {
            target_cli: request.target_cli,
            program: program.clone(),
            args: args.clone(),
            message: format!(
                "failed to open stdin for {} noninteractive process",
                noninteractive_cli_program(request.target_cli)
            ),
            stdout: String::new(),
            stderr: String::new(),
            retryable: true,
            timed_out: false,
        })?;
    if let Err(error) = stdin.write_all(request.context_content.as_bytes()) {
        let _ = child.kill();
        let _ = child.wait();
        let stdout = join_pipe_reader(stdout_handle);
        let stderr = join_pipe_reader(stderr_handle);
        return Err(NoninteractiveCliProcessError {
            target_cli: request.target_cli,
            program: program.clone(),
            args: args.clone(),
            message: format!(
                "failed to write context content to {} noninteractive process stdin: {error}",
                noninteractive_cli_program(request.target_cli)
            ),
            stdout,
            stderr,
            retryable: true,
            timed_out: false,
        });
    }
    drop(stdin);

    let status = match wait_for_child_with_timeout(&mut child, request.timeout_ms) {
        Ok(status) => status,
        Err(error) => {
            let stdout = join_pipe_reader(stdout_handle);
            let stderr = join_pipe_reader(stderr_handle);
            return Err(NoninteractiveCliProcessError {
                target_cli: request.target_cli,
                program: program.clone(),
                args: args.clone(),
                message: error,
                stdout,
                stderr,
                retryable: true,
                timed_out: true,
            });
        }
    };
    let stdout = join_pipe_reader(stdout_handle);
    let stderr = join_pipe_reader(stderr_handle);

    if !status.success() {
        return Err(NoninteractiveCliProcessError {
            target_cli: request.target_cli,
            program,
            args,
            message: format!(
                "{} noninteractive process exited with status {}",
                noninteractive_cli_program(request.target_cli),
                status
            ),
            stdout,
            stderr,
            retryable: true,
            timed_out: false,
        });
    }

    Ok(NoninteractiveCliProcessOutput {
        target_cli: request.target_cli,
        program,
        args,
        exit_code: status.code(),
        stdout,
        stderr,
    })
}

fn wait_for_child_with_timeout(
    child: &mut Child,
    timeout_ms: Option<u64>,
) -> Result<ExitStatus, String> {
    let Some(timeout_ms) = timeout_ms else {
        return child.wait().map_err(|error| {
            format!("failed to wait for noninteractive process without timeout: {error}")
        });
    };
    let timeout = Duration::from_millis(timeout_ms);
    let started_at = Instant::now();

    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Ok(status),
            Ok(None) if started_at.elapsed() >= timeout => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!(
                    "noninteractive process timed out after {timeout_ms}ms"
                ));
            }
            Ok(None) => thread::sleep(Duration::from_millis(10)),
            Err(error) => {
                return Err(format!(
                    "failed to wait for noninteractive process: {error}"
                ));
            }
        }
    }
}

fn read_pipe_to_string<R>(mut reader: R) -> thread::JoinHandle<String>
where
    R: Read + Send + 'static,
{
    thread::spawn(move || {
        let mut output = String::new();
        let _ = reader.read_to_string(&mut output);
        output
    })
}

fn join_pipe_reader(handle: Option<thread::JoinHandle<String>>) -> String {
    handle
        .and_then(|handle| handle.join().ok())
        .unwrap_or_default()
}

#[derive(Debug, Clone, Default, Eq, PartialEq)]
pub struct DiscoveredContextClassificationMetadata {
    pub root_source: Option<PathBuf>,
    pub folder_path: Option<PathBuf>,
    pub title: Option<String>,
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct DiscoveredContextClassification {
    pub classification: Classification,
    pub rule_id: &'static str,
    pub rationale: &'static str,
    pub file_name: String,
    pub folder_path: PathBuf,
    pub tags: Vec<String>,
}

pub const MAIN_AGENT_FILE_NAMES: &[&str] = &[
    "CLAUDE.md",
    "claude.md",
    "AGENTS.md",
    "agent.md",
    "agents.md",
];

pub const MAIN_AGENT_DIRECTORY_PATTERNS: &[&str] = &["", ".claude", ".codex", ".agents"];

pub const SKILL_DIRECTORY_SEGMENTS: &[&str] = &["skills"];

pub const SKILL_DIRECTORY_PATTERNS: &[&str] = &[
    "skills",
    ".claude/skills",
    ".codex/skills",
    ".agents/skills",
    ".ctx/skills",
];

pub const SUBAGENT_DIRECTORY_SEGMENTS: &[&str] = &["agents", "subagents"];

pub const SUBAGENT_DIRECTORY_PATTERNS: &[&str] = &[
    "agents",
    "subagents",
    ".claude/agents",
    ".claude/subagents",
    ".codex/agents",
    ".agents",
];

pub const SUBAGENT_FILE_STEM_TOKENS: &[&str] = &["agent"];

pub const CONTEXT_CLASSIFICATION_RULES: &[ContextClassificationRule] = &[
    ContextClassificationRule {
        id: "main-agent-filename",
        classification: Classification::MainAgent,
        filename_matches: MAIN_AGENT_FILE_NAMES,
        directory_segments: &[],
        file_stem_tokens: &[],
        rationale:
            "Canonical Claude/Codex root instruction filenames define primary agent context.",
    },
    ContextClassificationRule {
        id: "skill-directory",
        classification: Classification::Shared,
        filename_matches: &[],
        directory_segments: SKILL_DIRECTORY_SEGMENTS,
        file_stem_tokens: &[],
        rationale: "Markdown under skills directories is reusable shared context.",
    },
    ContextClassificationRule {
        id: "subagent-directory-or-name",
        classification: Classification::Subagent,
        filename_matches: &[],
        directory_segments: SUBAGENT_DIRECTORY_SEGMENTS,
        file_stem_tokens: SUBAGENT_FILE_STEM_TOKENS,
        rationale:
            "Agent/subagent directories and agent-named markdown files represent delegated roles.",
    },
    ContextClassificationRule {
        id: "shared-default",
        classification: Classification::Shared,
        filename_matches: &[],
        directory_segments: &[],
        file_stem_tokens: &[],
        rationale:
            "Discovered markdown that does not match agent conventions remains shared context.",
    },
];

pub fn pending_suggestion() -> ClassificationSuggestion {
    ClassificationSuggestion {
        classification: Classification::Shared,
        status: ClassificationStatus::Pending,
        rationale: "Headless Claude/Codex adapter scaffold is pending implementation.".to_string(),
    }
}

pub fn classify_import_markdown_content(
    request: &ImportTimeClassificationRequest,
) -> ImportTimeClassificationResult {
    if let Some((classification, rationale)) = explicit_content_classification(&request.content) {
        return ImportTimeClassificationResult {
            classification,
            status: ClassificationStatus::Classified,
            confidence_score: 96,
            rationale,
            rule_id: Some("explicit-markdown-classification".to_string()),
            suggested_tags: merge_suggested_tags(
                deterministic_tags(
                    request.file_name.as_deref().unwrap_or("context.md"),
                    request
                        .folder_path
                        .as_deref()
                        .unwrap_or_else(|| Path::new("")),
                ),
                &request.existing_tags,
                classification,
            ),
        };
    }

    let file_name = request.file_name.as_deref().unwrap_or("context.md");
    let folder_path = request
        .folder_path
        .as_deref()
        .unwrap_or_else(|| Path::new(""));
    let rule = classification_rule_for(file_name, folder_path);
    if rule.id != "shared-default" {
        return ImportTimeClassificationResult {
            classification: rule.classification,
            status: ClassificationStatus::Classified,
            confidence_score: confidence_for_path_rule(rule.id, request.import_source_type),
            rationale: format!(
                "{} Matched import convention '{}'.",
                rule.rationale, rule.id
            ),
            rule_id: Some(rule.id.to_string()),
            suggested_tags: merge_suggested_tags(
                deterministic_tags(file_name, folder_path),
                &request.existing_tags,
                rule.classification,
            ),
        };
    }

    let content_signal = content_classification_signal(&request.content);
    let classification = content_signal.classification;
    ImportTimeClassificationResult {
        classification,
        status: ClassificationStatus::Classified,
        confidence_score: content_signal.confidence_score,
        rationale: content_signal.rationale,
        rule_id: Some(content_signal.rule_id),
        suggested_tags: merge_suggested_tags(
            deterministic_tags(file_name, folder_path),
            &request.existing_tags,
            classification,
        ),
    }
}

fn normalize_headless_classification_value(
    request: &HeadlessClassificationRequest,
    adapter_kind: HeadlessClassificationAdapterKind,
    value: &Value,
) -> Result<HeadlessClassificationResult, HeadlessClassificationError> {
    let object = value.as_object().ok_or_else(|| {
        classification_parse_error(adapter_kind, "classification output must be a JSON object")
    })?;

    let classification = required_string_field(object, &["classification", "category", "type"])
        .and_then(|value| parse_classification_label(value).ok_or_else(|| {
            format!(
                "unsupported classification value '{value}'. Expected main-agent, subagent, or shared"
            )
        }))
        .map_err(|message| classification_parse_error(adapter_kind, message))?;

    let status = optional_string_field(object, &["status", "llm_classification_status"])
        .map(parse_classification_status_label)
        .transpose()
        .map_err(|message| classification_parse_error(adapter_kind, message))?
        .unwrap_or(ClassificationStatus::Classified);

    let confidence_score = optional_field(object, &["confidence_score", "confidence", "score"])
        .map(normalize_confidence_score)
        .transpose()
        .map_err(|message| classification_parse_error(adapter_kind, message))?
        .unwrap_or(50);

    let rationale = required_string_field(object, &["rationale", "reason", "explanation"])
        .map(normalized_required_text)
        .map_err(|message| classification_parse_error(adapter_kind, message))?;

    let suggested_title = optional_string_field(object, &["suggested_title", "title"])
        .map(normalized_optional_text)
        .transpose()
        .map_err(|message| classification_parse_error(adapter_kind, message))?
        .flatten();

    let suggested_tags = optional_field(object, &["suggested_tags", "tags"])
        .map(normalized_string_array)
        .transpose()
        .map_err(|message| classification_parse_error(adapter_kind, message))?
        .unwrap_or_default();

    let suggested_folder_path =
        optional_string_field(object, &["suggested_folder_path", "folder_path", "folder"])
            .map(normalized_relative_folder_path)
            .transpose()
            .map_err(|message| classification_parse_error(adapter_kind, message))?
            .flatten();

    let detected_wikilinks = optional_field(object, &["detected_wikilinks", "wikilinks"])
        .map(normalized_string_array)
        .transpose()
        .map_err(|message| classification_parse_error(adapter_kind, message))?
        .unwrap_or_default();

    let warnings = optional_field(object, &["warnings"])
        .map(normalized_string_array_preserving_case)
        .transpose()
        .map_err(|message| classification_parse_error(adapter_kind, message))?
        .unwrap_or_default();

    Ok(HeadlessClassificationResult {
        request_id: request.request_id,
        adapter_kind,
        classification,
        status,
        confidence_score,
        rationale,
        suggested_title,
        suggested_tags,
        suggested_folder_path,
        detected_wikilinks,
        warnings,
    })
}

fn classification_parse_error(
    adapter_kind: HeadlessClassificationAdapterKind,
    message: impl Into<String>,
) -> HeadlessClassificationError {
    HeadlessClassificationError {
        adapter_kind,
        message: message.into(),
        retryable: false,
    }
}

fn extract_classification_json_value(text: &str) -> Result<Value, String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Err("classification output was empty".to_string());
    }

    if let Ok(value) = serde_json::from_str::<Value>(trimmed) {
        return Ok(value);
    }

    for fenced in fenced_code_blocks(trimmed) {
        if let Ok(value) = serde_json::from_str::<Value>(fenced.trim()) {
            return Ok(value);
        }
    }

    if let Some(candidate) = first_balanced_json_object(trimmed) {
        if let Ok(value) = serde_json::from_str::<Value>(candidate) {
            return Ok(value);
        }
    }

    Err("classification output did not contain a valid JSON object".to_string())
}

fn fenced_code_blocks(text: &str) -> Vec<&str> {
    let mut blocks = Vec::new();
    let mut rest = text;

    while let Some(start) = rest.find("```") {
        let after_start = &rest[start + 3..];
        let content_start = after_start.find('\n').map(|index| index + 1).unwrap_or(0);
        let after_language = &after_start[content_start..];
        let Some(end) = after_language.find("```") else {
            break;
        };
        blocks.push(&after_language[..end]);
        rest = &after_language[end + 3..];
    }

    blocks
}

fn first_balanced_json_object(text: &str) -> Option<&str> {
    let bytes = text.as_bytes();
    let mut start = None;
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;

    for (index, byte) in bytes.iter().enumerate() {
        if start.is_none() {
            if *byte == b'{' {
                start = Some(index);
                depth = 1;
            }
            continue;
        }

        if escaped {
            escaped = false;
            continue;
        }

        match *byte {
            b'\\' if in_string => escaped = true,
            b'"' => in_string = !in_string,
            b'{' if !in_string => depth += 1,
            b'}' if !in_string => {
                depth -= 1;
                if depth == 0 {
                    let start = start.expect("start should be set while scanning JSON object");
                    return text.get(start..=index);
                }
            }
            _ => {}
        }
    }

    None
}

fn optional_field<'a>(
    object: &'a serde_json::Map<String, Value>,
    names: &[&str],
) -> Option<&'a Value> {
    names.iter().find_map(|name| object.get(*name))
}

fn required_string_field<'a>(
    object: &'a serde_json::Map<String, Value>,
    names: &[&str],
) -> Result<&'a str, String> {
    optional_string_field(object, names)
        .ok_or_else(|| format!("missing required field '{}'", names[0]))?
}

fn optional_string_field<'a>(
    object: &'a serde_json::Map<String, Value>,
    names: &[&str],
) -> Option<Result<&'a str, String>> {
    optional_field(object, names).map(|value| {
        value
            .as_str()
            .ok_or_else(|| format!("field '{}' must be a string", names[0]))
            .and_then(|value| {
                if value.trim().is_empty() {
                    Err(format!("field '{}' cannot be empty", names[0]))
                } else {
                    Ok(value)
                }
            })
    })
}

fn parse_classification_label(value: &str) -> Option<Classification> {
    match normalized_label(value).as_str() {
        "main" | "main-agent" | "primary" | "primary-agent" | "root-agent" => {
            Some(Classification::MainAgent)
        }
        "subagent" | "sub-agent" | "agent-role" | "delegated-agent" | "delegate" => {
            Some(Classification::Subagent)
        }
        "shared" | "skill" | "skills" | "reusable" | "reference" | "context-fragment" => {
            Some(Classification::Shared)
        }
        _ => None,
    }
}

fn parse_classification_status_label(
    value: Result<&str, String>,
) -> Result<ClassificationStatus, String> {
    let value = value?;
    match normalized_label(value).as_str() {
        "pending" => Ok(ClassificationStatus::Pending),
        "classified" => Ok(ClassificationStatus::Classified),
        "reviewed" => Ok(ClassificationStatus::Reviewed),
        "modified" => Ok(ClassificationStatus::Modified),
        other => Err(format!(
            "unsupported classification status '{other}'. Expected pending, classified, reviewed, or modified"
        )),
    }
}

fn normalized_label(value: &str) -> String {
    value
        .trim()
        .trim_matches('"')
        .to_ascii_lowercase()
        .replace(['_', ' '], "-")
}

fn normalize_confidence_score(value: &Value) -> Result<u8, String> {
    let score = match value {
        Value::Number(number) => number
            .as_f64()
            .ok_or_else(|| "confidence_score must be a finite number".to_string())?,
        Value::String(text) => text
            .trim()
            .trim_end_matches('%')
            .parse::<f64>()
            .map_err(|_| "confidence_score string must contain a number".to_string())?,
        _ => return Err("confidence_score must be a number or numeric string".to_string()),
    };

    if !score.is_finite() {
        return Err("confidence_score must be finite".to_string());
    }

    let normalized = if (0.0..=1.0).contains(&score) {
        score * 100.0
    } else {
        score
    };

    Ok(normalized.round().clamp(0.0, 100.0) as u8)
}

fn normalized_required_text(value: &str) -> String {
    value
        .trim()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn normalized_optional_text(value: Result<&str, String>) -> Result<Option<String>, String> {
    let normalized = normalized_required_text(value?);
    Ok((!normalized.is_empty()).then_some(normalized))
}

fn normalized_string_array(value: &Value) -> Result<Vec<String>, String> {
    normalized_string_array_with(value, |item| {
        item.trim()
            .trim_start_matches('#')
            .trim_start_matches("[[")
            .trim_end_matches("]]")
            .trim()
            .to_ascii_lowercase()
    })
}

fn normalized_string_array_preserving_case(value: &Value) -> Result<Vec<String>, String> {
    normalized_string_array_with(value, |item| normalized_required_text(item))
}

fn normalized_string_array_with<F>(value: &Value, normalize: F) -> Result<Vec<String>, String>
where
    F: Fn(&str) -> String,
{
    let raw_values: Vec<&str> = match value {
        Value::Array(values) => values
            .iter()
            .map(|value| {
                value
                    .as_str()
                    .ok_or_else(|| "string list fields must contain only strings".to_string())
            })
            .collect::<Result<Vec<_>, _>>()?,
        Value::String(value) => value
            .split(',')
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .collect(),
        _ => {
            return Err(
                "string list fields must be an array of strings or comma-separated string"
                    .to_string(),
            )
        }
    };

    let mut normalized = Vec::new();
    for value in raw_values {
        let item = normalize(value);
        if !item.is_empty()
            && !normalized
                .iter()
                .any(|existing: &String| existing.eq_ignore_ascii_case(&item))
        {
            normalized.push(item);
        }
    }
    Ok(normalized)
}

fn normalized_relative_folder_path(value: Result<&str, String>) -> Result<Option<PathBuf>, String> {
    let normalized = value?
        .trim()
        .trim_matches('/')
        .replace('\\', "/")
        .split('/')
        .map(str::trim)
        .filter(|segment| !segment.is_empty() && *segment != ".")
        .collect::<Vec<_>>()
        .join("/");

    if normalized.is_empty() {
        return Ok(None);
    }

    let path = PathBuf::from(&normalized);
    if path.is_absolute()
        || path
            .components()
            .any(|component| component.as_os_str().to_str() == Some(".."))
    {
        return Err("suggested_folder_path must be a relative path inside the vault".to_string());
    }

    Ok(Some(path))
}

pub fn deterministic_classification(file_name: &str, folder_path: &Path) -> Classification {
    classification_rule_for(file_name, folder_path).classification
}

pub fn classify_discovered_context(
    file_path: &Path,
    metadata: &DiscoveredContextClassificationMetadata,
) -> DiscoveredContextClassification {
    let file_name = file_path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default()
        .to_string();
    let folder_path = discovered_folder_path(file_path, metadata);
    let rule = classification_rule_for_with_metadata(&file_name, &folder_path, metadata);
    let mut tags = deterministic_tags(&file_name, &folder_path);

    for tag in &metadata.tags {
        if !tags
            .iter()
            .any(|existing| existing.eq_ignore_ascii_case(tag))
        {
            tags.push(tag.to_string());
        }
    }

    DiscoveredContextClassification {
        classification: rule.classification,
        rule_id: rule.id,
        rationale: rule.rationale,
        file_name,
        folder_path,
        tags,
    }
}

pub fn classification_rule_for(
    file_name: &str,
    folder_path: &Path,
) -> &'static ContextClassificationRule {
    classification_rule_for_with_metadata(
        file_name,
        folder_path,
        &DiscoveredContextClassificationMetadata::default(),
    )
}

fn classification_rule_for_with_metadata(
    file_name: &str,
    folder_path: &Path,
    metadata: &DiscoveredContextClassificationMetadata,
) -> &'static ContextClassificationRule {
    CONTEXT_CLASSIFICATION_RULES
        .iter()
        .find(|rule| rule_matches(rule, file_name, folder_path, metadata))
        .unwrap_or_else(|| {
            CONTEXT_CLASSIFICATION_RULES
                .last()
                .expect("classification rules must include shared default")
        })
}

pub fn deterministic_tags(file_name: &str, folder_path: &Path) -> Vec<String> {
    let mut tags = vec!["discovered".to_string()];

    if path_contains_segment(folder_path, "skills") {
        tags.push("skills".to_string());
    }
    if path_contains_segment(folder_path, "agents")
        || path_contains_segment(folder_path, "subagents")
    {
        tags.push("agents".to_string());
    }
    if file_name.eq_ignore_ascii_case("AGENTS.md") {
        tags.push("codex".to_string());
    }
    if file_name.eq_ignore_ascii_case("CLAUDE.md") || file_name.eq_ignore_ascii_case("claude.md") {
        tags.push("claude".to_string());
    }

    tags
}

fn rule_matches(
    rule: &ContextClassificationRule,
    file_name: &str,
    folder_path: &Path,
    metadata: &DiscoveredContextClassificationMetadata,
) -> bool {
    if rule.id == "shared-default" {
        return true;
    }

    rule.filename_matches
        .iter()
        .any(|candidate| file_name.eq_ignore_ascii_case(candidate))
        || rule
            .directory_segments
            .iter()
            .any(|segment| path_contains_segment(folder_path, segment))
        || rule
            .file_stem_tokens
            .iter()
            .any(|token| file_stem_contains(file_name, token))
        || metadata_matches_rule(rule, metadata)
}

fn metadata_matches_rule(
    rule: &ContextClassificationRule,
    metadata: &DiscoveredContextClassificationMetadata,
) -> bool {
    let metadata_values = metadata
        .title
        .iter()
        .map(String::as_str)
        .chain(metadata.tags.iter().map(String::as_str));

    match rule.classification {
        Classification::MainAgent => metadata_values
            .clone()
            .any(|value| normalized_metadata_value(value) == "main-agent"),
        Classification::Subagent => metadata_values.clone().any(|value| {
            matches!(
                normalized_metadata_value(value).as_str(),
                "subagent" | "sub-agent" | "agent-role"
            )
        }),
        Classification::Shared => metadata_values.clone().any(|value| {
            matches!(
                normalized_metadata_value(value).as_str(),
                "shared" | "skill" | "skills"
            )
        }),
    }
}

fn discovered_folder_path(
    file_path: &Path,
    metadata: &DiscoveredContextClassificationMetadata,
) -> PathBuf {
    if let Some(folder_path) = &metadata.folder_path {
        return folder_path.clone();
    }

    let Some(parent) = file_path.parent() else {
        return PathBuf::new();
    };

    metadata
        .root_source
        .as_ref()
        .and_then(|root| parent.strip_prefix(root).ok())
        .filter(|relative| !relative.as_os_str().is_empty())
        .map(Path::to_path_buf)
        .unwrap_or_else(|| parent.to_path_buf())
}

fn file_stem_contains(file_name: &str, needle: &str) -> bool {
    Path::new(file_name)
        .file_stem()
        .and_then(|stem| stem.to_str())
        .map(|stem| stem.to_ascii_lowercase().contains(needle))
        .unwrap_or(false)
}

fn path_contains_segment(path: &Path, segment: &str) -> bool {
    path.components().any(|component| {
        component
            .as_os_str()
            .to_str()
            .map(|value| value.eq_ignore_ascii_case(segment))
            .unwrap_or(false)
    })
}

fn normalized_metadata_value(value: &str) -> String {
    value.trim().to_ascii_lowercase().replace(['_', ' '], "-")
}

#[derive(Debug, Clone, Eq, PartialEq)]
struct ContentClassificationSignal {
    classification: Classification,
    confidence_score: u8,
    rationale: String,
    rule_id: String,
}

fn explicit_content_classification(content: &str) -> Option<(Classification, String)> {
    content.lines().take(40).find_map(|line| {
        let trimmed = line.trim().trim_start_matches('#').trim();
        let (key, value) = trimmed.split_once(':')?;
        let key = normalized_label(key);
        if !matches!(
            key.as_str(),
            "classification" | "category" | "ctx-classification" | "agent-context"
        ) {
            return None;
        }

        parse_classification_label(value).map(|classification| {
            (
                classification,
                format!(
                    "Markdown declares import classification '{}' in a heading or frontmatter field.",
                    classification_label_for_rationale(classification)
                ),
            )
        })
    })
}

fn content_classification_signal(content: &str) -> ContentClassificationSignal {
    let normalized = normalized_content_for_matching(content);
    let subagent_score = keyword_score(
        &normalized,
        &[
            "subagent",
            "sub-agent",
            "delegated agent",
            "delegate",
            "handoff",
            "spawn",
            "assigned contexts",
            "tools:",
        ],
    );
    let main_agent_score = keyword_score(
        &normalized,
        &[
            "system prompt",
            "main agent",
            "primary agent",
            "root instructions",
            "you are",
            "always",
            "never",
        ],
    );
    let shared_score = keyword_score(
        &normalized,
        &[
            "shared",
            "reusable",
            "skill",
            "reference",
            "runbook",
            "style guide",
            "conventions",
            "knowledge",
        ],
    );

    if subagent_score >= 2 && subagent_score >= main_agent_score && subagent_score >= shared_score {
        return ContentClassificationSignal {
            classification: Classification::Subagent,
            confidence_score: (64 + subagent_score * 7).min(90),
            rationale:
                "Markdown content describes delegation, handoff, or scoped helper-agent behavior."
                    .to_string(),
            rule_id: "content-subagent-signals".to_string(),
        };
    }

    if main_agent_score >= 3 && main_agent_score > shared_score {
        return ContentClassificationSignal {
            classification: Classification::MainAgent,
            confidence_score: (60 + main_agent_score * 6).min(88),
            rationale:
                "Markdown content reads as primary session instructions for the active agent."
                    .to_string(),
            rule_id: "content-main-agent-signals".to_string(),
        };
    }

    if shared_score >= 2 {
        return ContentClassificationSignal {
            classification: Classification::Shared,
            confidence_score: (62 + shared_score * 6).min(88),
            rationale:
                "Markdown content is framed as reusable reference, skill, or convention material."
                    .to_string(),
            rule_id: "content-shared-signals".to_string(),
        };
    }

    ContentClassificationSignal {
        classification: Classification::Shared,
        confidence_score: 55,
        rationale:
            "No explicit agent or subagent signals were found, so import defaults to shared context."
                .to_string(),
        rule_id: "shared-default".to_string(),
    }
}

fn normalized_content_for_matching(content: &str) -> String {
    content
        .trim()
        .to_ascii_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn keyword_score(content: &str, keywords: &[&str]) -> u8 {
    keywords
        .iter()
        .filter(|keyword| content.contains(**keyword))
        .count()
        .try_into()
        .unwrap_or(u8::MAX)
}

fn confidence_for_path_rule(rule_id: &str, import_source_type: Option<ImportSourceType>) -> u8 {
    match (rule_id, import_source_type) {
        (
            "main-agent-filename",
            Some(
                ImportSourceType::ClaudeMarkdown
                | ImportSourceType::CodexAgents
                | ImportSourceType::AgentMarkdown,
            ),
        ) => 94,
        (
            "skill-directory",
            Some(ImportSourceType::SkillMarkdown | ImportSourceType::SkillManifest),
        ) => 93,
        ("subagent-directory-or-name", Some(ImportSourceType::SubagentMarkdown)) => 93,
        ("main-agent-filename", _) => 90,
        ("skill-directory", _) | ("subagent-directory-or-name", _) => 86,
        _ => 72,
    }
}

fn merge_suggested_tags(
    mut tags: Vec<String>,
    existing_tags: &[String],
    classification: Classification,
) -> Vec<String> {
    let classification_tag = classification_label_for_rationale(classification).to_string();
    if !tags
        .iter()
        .any(|existing| existing.eq_ignore_ascii_case(&classification_tag))
    {
        tags.push(classification_tag);
    }

    for tag in existing_tags {
        let normalized = tag.trim().trim_start_matches('#').to_ascii_lowercase();
        if !normalized.is_empty()
            && !tags
                .iter()
                .any(|existing| existing.eq_ignore_ascii_case(&normalized))
        {
            tags.push(normalized);
        }
    }

    tags
}

fn classification_label_for_rationale(classification: Classification) -> &'static str {
    match classification {
        Classification::MainAgent => "main-agent",
        Classification::Subagent => "subagent",
        Classification::Shared => "shared",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::{env, fs, path::PathBuf};

    struct StubHeadlessAdapter {
        kind: HeadlessClassificationAdapterKind,
    }

    impl HeadlessClassificationAdapter for StubHeadlessAdapter {
        fn adapter_kind(&self) -> HeadlessClassificationAdapterKind {
            self.kind
        }

        fn analyze_context(
            &self,
            request: &HeadlessClassificationRequest,
        ) -> Result<HeadlessClassificationResult, HeadlessClassificationError> {
            Ok(HeadlessClassificationResult {
                request_id: request.request_id,
                adapter_kind: self.kind,
                classification: Classification::Subagent,
                status: ClassificationStatus::Classified,
                confidence_score: 91,
                rationale: "The markdown describes a delegated review role.".to_string(),
                suggested_title: Some("Reviewer".to_string()),
                suggested_tags: vec!["review".to_string(), "agent".to_string()],
                suggested_folder_path: Some(PathBuf::from("agents")),
                detected_wikilinks: vec!["Shared Style Guide".to_string()],
                warnings: Vec::new(),
            })
        }
    }

    #[test]
    fn cli_target_maps_to_headless_classification_adapter_kind() {
        assert_eq!(
            HeadlessClassificationAdapterKind::from(CliTarget::Claude),
            HeadlessClassificationAdapterKind::Claude
        );
        assert_eq!(
            HeadlessClassificationAdapterKind::from(CliTarget::Codex),
            HeadlessClassificationAdapterKind::Codex
        );
    }

    #[test]
    fn headless_classification_result_serializes_structured_review_schema() {
        let request_id = Uuid::new_v4();
        let result = HeadlessClassificationResult {
            request_id,
            adapter_kind: HeadlessClassificationAdapterKind::Claude,
            classification: Classification::Shared,
            status: ClassificationStatus::Classified,
            confidence_score: 84,
            rationale: "Reusable project convention context.".to_string(),
            suggested_title: Some("Project Conventions".to_string()),
            suggested_tags: vec!["conventions".to_string(), "shared".to_string()],
            suggested_folder_path: Some(PathBuf::from("shared")),
            detected_wikilinks: vec!["Testing".to_string()],
            warnings: vec!["Low content volume.".to_string()],
        };

        let value = serde_json::to_value(result).expect("result should serialize to JSON");

        assert_eq!(value["request_id"], request_id.to_string());
        assert_eq!(value["adapter_kind"], "claude");
        assert_eq!(value["classification"], "shared");
        assert_eq!(value["status"], "classified");
        assert_eq!(value["confidence_score"], 84);
        assert_eq!(value["suggested_folder_path"], "shared");
        assert_eq!(value["detected_wikilinks"], json!(["Testing"]));
    }

    #[test]
    fn headless_classification_request_deserializes_cli_analysis_payload() {
        let request_id = Uuid::new_v4();
        let context_id = Uuid::new_v4();
        let payload = json!({
            "request_id": request_id,
            "target_cli": "codex",
            "context_id": context_id,
            "title": "Codex Reviewer",
            "content": "# Reviewer\nUse [[Shared Style Guide]].",
            "file_path": "/workspace/.codex/agents/reviewer.md",
            "vault_scope": "local",
            "folder_path": ".codex/agents",
            "import_source": "/workspace/.codex/agents/reviewer.md",
            "import_source_type": "subagent-markdown",
            "existing_tags": ["review"],
            "existing_wikilinks": ["Shared Style Guide"]
        });

        let request: HeadlessClassificationRequest =
            serde_json::from_value(payload).expect("request should deserialize");

        assert_eq!(request.request_id, request_id);
        assert_eq!(request.target_cli, CliTarget::Codex);
        assert_eq!(request.context_id, Some(context_id));
        assert_eq!(request.vault_scope, Some(VaultScope::Local));
        assert_eq!(
            request.import_source_type,
            Some(ImportSourceType::SubagentMarkdown)
        );
        assert_eq!(request.existing_wikilinks, vec!["Shared Style Guide"]);
    }

    #[test]
    fn headless_classification_adapter_trait_returns_structured_result() {
        let adapter = StubHeadlessAdapter {
            kind: HeadlessClassificationAdapterKind::Codex,
        };
        let request = HeadlessClassificationRequest {
            request_id: Uuid::new_v4(),
            target_cli: CliTarget::Codex,
            context_id: None,
            title: Some("Reviewer".to_string()),
            content: "# Reviewer".to_string(),
            file_path: PathBuf::from("/workspace/agents/reviewer.md"),
            vault_scope: Some(VaultScope::Local),
            folder_path: PathBuf::from("agents"),
            import_source: None,
            import_source_type: Some(ImportSourceType::SubagentMarkdown),
            existing_tags: Vec::new(),
            existing_wikilinks: Vec::new(),
        };

        let result = adapter
            .analyze_context(&request)
            .expect("adapter should return structured result");

        assert_eq!(
            adapter.adapter_kind(),
            HeadlessClassificationAdapterKind::Codex
        );
        assert_eq!(
            result.adapter_kind,
            HeadlessClassificationAdapterKind::Codex
        );
        assert_eq!(result.classification, Classification::Subagent);
        assert_eq!(result.status, ClassificationStatus::Classified);
        assert_eq!(result.confidence_score, 91);
        assert_eq!(result.detected_wikilinks, vec!["Shared Style Guide"]);
    }

    #[test]
    fn headless_classification_prompt_is_standalone_and_json_oriented() {
        let request = classification_request_fixture(CliTarget::Claude);
        let prompt = build_headless_classification_prompt(&request);

        assert!(prompt.contains("Classify this markdown context for CTX import."));
        assert!(prompt.contains("Return exactly one JSON object"));
        assert!(prompt.contains("classification: one of"));
        assert!(prompt.contains("```markdown\n# Reviewer\n```"));
    }

    #[cfg(unix)]
    #[test]
    fn local_headless_cli_classification_adapter_runs_noninteractive_cli() {
        let script = executable_fixture(
            "ctx-headless-classifier",
            "#!/bin/sh\ncat >/dev/null\nprintf '{\"classification\":\"shared\",\"confidence_score\":82,\"rationale\":\"Reusable reference material.\"}'\n",
        );
        let request = classification_request_fixture(CliTarget::Claude);
        let adapter = LocalHeadlessCliClassificationAdapter {
            target_cli: CliTarget::Claude,
            program: Some(script.display().to_string()),
            working_dir: None,
            timeout_ms: Some(1_000),
            passthrough_args: Vec::new(),
        };

        let result = adapter
            .analyze_context(&request)
            .expect("local headless adapter should parse CLI JSON output");

        assert_eq!(
            result.adapter_kind,
            HeadlessClassificationAdapterKind::Claude
        );
        assert_eq!(result.classification, Classification::Shared);
        assert_eq!(result.confidence_score, 82);
        assert_eq!(result.rationale, "Reusable reference material.");
    }

    #[test]
    fn import_time_classification_api_returns_explicit_category_confidence_and_rationale() {
        let result = classify_import_markdown_content(&ImportTimeClassificationRequest {
            content: "---\nclassification: subagent\n---\n# Reviewer\nUse [[Style Guide]]."
                .to_string(),
            file_name: Some("reviewer.md".to_string()),
            folder_path: Some(PathBuf::from("docs")),
            import_source_type: None,
            existing_tags: vec!["Review".to_string()],
        });

        assert_eq!(result.classification, Classification::Subagent);
        assert_eq!(result.status, ClassificationStatus::Classified);
        assert_eq!(result.confidence_score, 96);
        assert!(result.rationale.contains("declares import classification"));
        assert_eq!(
            result.rule_id.as_deref(),
            Some("explicit-markdown-classification")
        );
        assert!(result.suggested_tags.contains(&"subagent".to_string()));
        assert!(result.suggested_tags.contains(&"review".to_string()));
    }

    #[test]
    fn import_time_classification_explicit_frontmatter_overrides_path_conventions() {
        let result = classify_import_markdown_content(&ImportTimeClassificationRequest {
            content: "---\nclassification: shared\ntags: [team]\n---\n# AGENTS\nTeam-wide reference notes."
                .to_string(),
            file_name: Some("AGENTS.md".to_string()),
            folder_path: Some(PathBuf::from(".codex")),
            import_source_type: Some(ImportSourceType::CodexAgents),
            existing_tags: vec!["Codex".to_string()],
        });

        assert_eq!(result.classification, Classification::Shared);
        assert_eq!(result.status, ClassificationStatus::Classified);
        assert_eq!(
            result.rule_id.as_deref(),
            Some("explicit-markdown-classification")
        );
        assert_eq!(result.confidence_score, 96);
        assert!(result.suggested_tags.contains(&"shared".to_string()));
        assert!(result.suggested_tags.contains(&"codex".to_string()));
    }

    #[test]
    fn import_time_classification_api_uses_path_conventions_before_content_fallback() {
        let result = classify_import_markdown_content(&ImportTimeClassificationRequest {
            content: "# Notes\nReusable testing references.".to_string(),
            file_name: Some("CLAUDE.md".to_string()),
            folder_path: Some(PathBuf::from(".claude")),
            import_source_type: Some(ImportSourceType::ClaudeMarkdown),
            existing_tags: Vec::new(),
        });

        assert_eq!(result.classification, Classification::MainAgent);
        assert_eq!(result.confidence_score, 94);
        assert_eq!(result.rule_id.as_deref(), Some("main-agent-filename"));
        assert!(result.rationale.contains("Matched import convention"));
    }

    #[test]
    fn import_time_classification_api_scores_content_signals() {
        let result = classify_import_markdown_content(&ImportTimeClassificationRequest {
            content:
                "# Code Review Helper\nThis subagent receives delegated agent handoff notes and assigned contexts."
                    .to_string(),
            file_name: Some("reviewer.md".to_string()),
            folder_path: Some(PathBuf::from("docs")),
            import_source_type: None,
            existing_tags: Vec::new(),
        });

        assert_eq!(result.classification, Classification::Subagent);
        assert!(result.confidence_score >= 78);
        assert_eq!(result.rule_id.as_deref(), Some("content-subagent-signals"));
        assert!(result.rationale.contains("delegation"));
    }

    #[test]
    fn parse_headless_classification_text_normalizes_fenced_cli_json() {
        let request = classification_request_fixture(CliTarget::Claude);
        let output = r##"
Claude analysis:

```json
{
  "category": "Sub Agent",
  "status": "classified",
  "confidence": 0.875,
  "reason": "  Describes a delegated review role.\n\nIt should be launched as a helper. ",
  "title": "  Reviewer Agent ",
  "tags": [" Review ", "#Agent", "review"],
  "folder": " .claude\\agents/ ",
  "wikilinks": ["[[Shared Style Guide]]", "shared style guide", "Testing"],
  "warnings": ["Uses generic language."]
}
```
"##;

        let result = parse_headless_classification_text(
            &request,
            HeadlessClassificationAdapterKind::Claude,
            output,
        )
        .expect("fenced JSON should parse and normalize");

        assert_eq!(result.request_id, request.request_id);
        assert_eq!(
            result.adapter_kind,
            HeadlessClassificationAdapterKind::Claude
        );
        assert_eq!(result.classification, Classification::Subagent);
        assert_eq!(result.status, ClassificationStatus::Classified);
        assert_eq!(result.confidence_score, 88);
        assert_eq!(
            result.rationale,
            "Describes a delegated review role. It should be launched as a helper."
        );
        assert_eq!(result.suggested_title.as_deref(), Some("Reviewer Agent"));
        assert_eq!(result.suggested_tags, vec!["review", "agent"]);
        assert_eq!(
            result.suggested_folder_path,
            Some(PathBuf::from(".claude/agents"))
        );
        assert_eq!(
            result.detected_wikilinks,
            vec!["shared style guide", "testing"]
        );
        assert_eq!(result.warnings, vec!["Uses generic language."]);
    }

    #[test]
    fn parse_headless_classification_cli_output_extracts_embedded_json_object() {
        let request = classification_request_fixture(CliTarget::Codex);
        let output = NoninteractiveCliProcessOutput {
            target_cli: CliTarget::Codex,
            program: "codex".to_string(),
            args: vec!["exec".to_string(), "-".to_string()],
            exit_code: Some(0),
            stdout: r#"Result: {"classification":"MAIN_AGENT","confidence_score":"99%","rationale":"Root instructions for the CLI.","suggested_tags":"codex, instructions"} done."#.to_string(),
            stderr: String::new(),
        };

        let result = parse_headless_classification_cli_output(
            &request,
            HeadlessClassificationAdapterKind::Codex,
            &output,
        )
        .expect("embedded JSON object should parse");

        assert_eq!(result.classification, Classification::MainAgent);
        assert_eq!(result.status, ClassificationStatus::Classified);
        assert_eq!(result.confidence_score, 99);
        assert_eq!(result.suggested_tags, vec!["codex", "instructions"]);
    }

    #[test]
    fn parse_headless_classification_text_rejects_missing_required_fields() {
        let request = classification_request_fixture(CliTarget::Claude);
        let error = parse_headless_classification_text(
            &request,
            HeadlessClassificationAdapterKind::Claude,
            r#"{"classification":"shared","confidence_score":73}"#,
        )
        .expect_err("rationale is required for review UI");

        assert_eq!(
            error.adapter_kind,
            HeadlessClassificationAdapterKind::Claude
        );
        assert!(error.message.contains("missing required field 'rationale'"));
        assert!(!error.retryable);
    }

    #[test]
    fn parse_headless_classification_text_rejects_invalid_folder_escape() {
        let request = classification_request_fixture(CliTarget::Codex);
        let error = parse_headless_classification_text(
            &request,
            HeadlessClassificationAdapterKind::Codex,
            r#"{"classification":"shared","rationale":"Reusable reference.","suggested_folder_path":"../outside"}"#,
        )
        .expect_err("folder suggestions cannot escape the vault");

        assert!(error
            .message
            .contains("suggested_folder_path must be a relative path inside the vault"));
    }

    #[test]
    fn noninteractive_cli_args_use_target_specific_batch_modes() {
        assert_eq!(
            noninteractive_cli_args(
                CliTarget::Claude,
                &["--model".to_string(), "sonnet".to_string()]
            ),
            vec![
                "--print".to_string(),
                "--model".to_string(),
                "sonnet".to_string()
            ]
        );
        assert_eq!(
            noninteractive_cli_args(
                CliTarget::Codex,
                &["--model".to_string(), "gpt-5.3-codex".to_string()]
            ),
            vec![
                "exec".to_string(),
                "-".to_string(),
                "--model".to_string(),
                "gpt-5.3-codex".to_string()
            ]
        );
    }

    #[cfg(unix)]
    #[test]
    fn noninteractive_cli_process_writes_supplied_context_to_stdin_and_captures_output() {
        let script = executable_fixture(
            "ctx-noninteractive-success",
            "#!/bin/sh\nprintf 'arg1=%s\\n' \"$1\"\nprintf 'arg2=%s\\n' \"$2\"\nprintf 'stdin='; cat\n",
        );
        let request = NoninteractiveCliProcessRequest {
            target_cli: CliTarget::Claude,
            program: Some(script.display().to_string()),
            working_dir: None,
            timeout_ms: Some(1_000),
            context_content: "Classify this context.".to_string(),
            passthrough_args: vec!["--model".to_string(), "sonnet".to_string()],
        };

        let output = run_noninteractive_cli_process(&request)
            .expect("fixture process should receive stdin and exit successfully");

        assert_eq!(output.target_cli, CliTarget::Claude);
        assert_eq!(
            output.args,
            vec![
                "--print".to_string(),
                "--model".to_string(),
                "sonnet".to_string()
            ]
        );
        assert_eq!(output.exit_code, Some(0));
        assert!(output.stdout.contains("arg1=--print"));
        assert!(output.stdout.contains("arg2=--model"));
        assert!(output.stdout.contains("stdin=Classify this context."));
        assert!(output.stderr.is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn noninteractive_cli_process_reports_nonzero_exit_with_captured_stderr() {
        let script = executable_fixture(
            "ctx-noninteractive-failure",
            "#!/bin/sh\ncat >/dev/null\nprintf 'fixture failed\\n' >&2\nexit 42\n",
        );
        let request = NoninteractiveCliProcessRequest {
            target_cli: CliTarget::Codex,
            program: Some(script.display().to_string()),
            working_dir: None,
            timeout_ms: Some(1_000),
            context_content: "Context that triggers failure.".to_string(),
            passthrough_args: Vec::new(),
        };

        let error = run_noninteractive_cli_process(&request)
            .expect_err("nonzero fixture exit should become a structured error");

        assert_eq!(error.target_cli, CliTarget::Codex);
        assert_eq!(error.args, vec!["exec".to_string(), "-".to_string()]);
        assert!(error
            .message
            .contains("codex noninteractive process exited"));
        assert!(error.stderr.contains("fixture failed"));
        assert!(error.retryable);
        assert!(!error.timed_out);
    }

    #[cfg(unix)]
    #[test]
    fn noninteractive_cli_process_times_out_and_reports_structured_error() {
        let script = executable_fixture(
            "ctx-noninteractive-timeout",
            "#!/bin/sh\nprintf 'started\\n'\nwhile :; do :; done\n",
        );
        let request = NoninteractiveCliProcessRequest {
            target_cli: CliTarget::Claude,
            program: Some(script.display().to_string()),
            working_dir: None,
            timeout_ms: Some(50),
            context_content: "Context that should not hang.".to_string(),
            passthrough_args: Vec::new(),
        };

        let started_at = std::time::Instant::now();
        let error = run_noninteractive_cli_process(&request)
            .expect_err("long-running fixture should time out");

        assert!(started_at.elapsed() < std::time::Duration::from_secs(2));
        assert_eq!(error.target_cli, CliTarget::Claude);
        assert_eq!(error.args, vec!["--print".to_string()]);
        assert!(error.message.contains("timed out after 50ms"));
        assert!(error.stdout.contains("started"));
        assert!(error.retryable);
        assert!(error.timed_out);
    }

    #[test]
    fn deterministic_rules_classify_main_agent_files() {
        assert_eq!(
            deterministic_classification("CLAUDE.md", Path::new("")),
            Classification::MainAgent
        );
        assert_eq!(
            deterministic_classification("agent.md", Path::new("agents")),
            Classification::MainAgent
        );
        assert_eq!(
            deterministic_classification("AGENTS.md", Path::new(".codex")),
            Classification::MainAgent
        );
        assert_eq!(
            deterministic_classification("agents.md", Path::new(".agents")),
            Classification::MainAgent
        );
    }

    #[test]
    fn deterministic_rules_classify_agent_folder_markdown_as_subagent() {
        assert_eq!(
            deterministic_classification("reviewer.md", &PathBuf::from("agents")),
            Classification::Subagent
        );
        assert_eq!(
            deterministic_classification("planner.md", &PathBuf::from(".claude/subagents")),
            Classification::Subagent
        );
    }

    #[test]
    fn deterministic_rules_classify_skill_folder_markdown_as_shared() {
        assert_eq!(
            deterministic_classification("skill-one.md", &PathBuf::from(".claude/skills")),
            Classification::Shared
        );
    }

    #[test]
    fn filename_based_classification_prefers_canonical_main_agent_files() {
        for file_name in [
            "CLAUDE.md",
            "claude.md",
            "AGENTS.md",
            "agent.md",
            "agents.md",
        ] {
            let rule = classification_rule_for(file_name, Path::new("agents"));

            assert_eq!(
                rule.classification,
                Classification::MainAgent,
                "{file_name} should classify as primary agent context even in an agent directory"
            );
            assert_eq!(rule.id, "main-agent-filename");
        }
    }

    #[test]
    fn directory_based_classification_maps_agent_and_skill_folders() {
        let subagent_rule = classification_rule_for("reviewer.md", Path::new(".claude/agents"));
        let shared_rule = classification_rule_for("reviewer-agent.md", Path::new(".ctx/skills"));

        assert_eq!(subagent_rule.classification, Classification::Subagent);
        assert_eq!(subagent_rule.id, "subagent-directory-or-name");
        assert_eq!(shared_rule.classification, Classification::Shared);
        assert_eq!(shared_rule.id, "skill-directory");
    }

    #[test]
    fn folder_based_classification_covers_known_context_directories() {
        for (file_name, folder_path, expected_classification, expected_rule) in [
            (
                "reviewer.md",
                ".claude/agents",
                Classification::Subagent,
                "subagent-directory-or-name",
            ),
            (
                "planner.md",
                ".claude/subagents",
                Classification::Subagent,
                "subagent-directory-or-name",
            ),
            (
                "typescript.md",
                ".claude/skills",
                Classification::Shared,
                "skill-directory",
            ),
            (
                "codex-style.md",
                ".ctx/skills",
                Classification::Shared,
                "skill-directory",
            ),
        ] {
            let rule = classification_rule_for(file_name, Path::new(folder_path));

            assert_eq!(
                rule.classification, expected_classification,
                "{folder_path}/{file_name} should classify by folder"
            );
            assert_eq!(rule.id, expected_rule);
        }
    }

    #[test]
    fn filename_based_classification_covers_canonical_context_files() {
        for (file_name, folder_path) in [
            ("CLAUDE.md", ""),
            ("claude.md", "docs"),
            ("AGENTS.md", ".codex"),
            ("agent.md", "subagents"),
            ("agents.md", ".agents"),
        ] {
            let classification = classify_discovered_context(
                &PathBuf::from(folder_path).join(file_name),
                &DiscoveredContextClassificationMetadata {
                    folder_path: Some(PathBuf::from(folder_path)),
                    ..DiscoveredContextClassificationMetadata::default()
                },
            );

            assert_eq!(classification.classification, Classification::MainAgent);
            assert_eq!(classification.rule_id, "main-agent-filename");
        }
    }

    #[test]
    fn default_classification_keeps_unmatched_markdown_shared() {
        for (file_name, folder_path) in [
            ("research-notes.md", "docs"),
            ("prompt-fragments.md", "knowledge/base"),
            ("runbook.md", ""),
        ] {
            let classification = classify_discovered_context(
                &PathBuf::from(folder_path).join(file_name),
                &DiscoveredContextClassificationMetadata {
                    folder_path: Some(PathBuf::from(folder_path)),
                    ..DiscoveredContextClassificationMetadata::default()
                },
            );

            assert_eq!(classification.classification, Classification::Shared);
            assert_eq!(classification.rule_id, "shared-default");
            assert_eq!(classification.tags, vec!["discovered".to_string()]);
        }
    }

    #[test]
    fn ambiguous_classification_cases_use_stable_precedence() {
        let canonical_file_in_subagent_folder = classify_discovered_context(
            Path::new("/workspace/.claude/subagents/agent.md"),
            &DiscoveredContextClassificationMetadata {
                root_source: Some(PathBuf::from("/workspace")),
                ..DiscoveredContextClassificationMetadata::default()
            },
        );
        let skill_named_like_agent = classify_discovered_context(
            Path::new("/workspace/.ctx/skills/reviewer-agent.md"),
            &DiscoveredContextClassificationMetadata {
                root_source: Some(PathBuf::from("/workspace")),
                ..DiscoveredContextClassificationMetadata::default()
            },
        );
        let metadata_tag_on_unmatched_path = classify_discovered_context(
            Path::new("/workspace/docs/research.md"),
            &DiscoveredContextClassificationMetadata {
                root_source: Some(PathBuf::from("/workspace")),
                tags: vec!["agent role".to_string()],
                ..DiscoveredContextClassificationMetadata::default()
            },
        );

        assert_eq!(
            canonical_file_in_subagent_folder.classification,
            Classification::MainAgent
        );
        assert_eq!(
            canonical_file_in_subagent_folder.rule_id,
            "main-agent-filename"
        );
        assert_eq!(
            skill_named_like_agent.classification,
            Classification::Shared
        );
        assert_eq!(skill_named_like_agent.rule_id, "skill-directory");
        assert_eq!(
            metadata_tag_on_unmatched_path.classification,
            Classification::Subagent
        );
        assert_eq!(
            metadata_tag_on_unmatched_path.rule_id,
            "subagent-directory-or-name"
        );
    }

    #[test]
    fn nested_path_classification_uses_root_relative_folder() {
        let classification = classify_discovered_context(
            Path::new("/workspace/packages/app/.codex/agents/reviewer.md"),
            &DiscoveredContextClassificationMetadata {
                root_source: Some(PathBuf::from("/workspace")),
                ..DiscoveredContextClassificationMetadata::default()
            },
        );

        assert_eq!(
            classification.folder_path,
            PathBuf::from("packages/app/.codex/agents")
        );
        assert_eq!(classification.classification, Classification::Subagent);
        assert_eq!(classification.rule_id, "subagent-directory-or-name");
        assert!(classification.tags.contains(&"agents".to_string()));
    }

    #[test]
    fn fallback_classification_defaults_unmatched_markdown_to_shared() {
        let classification = classify_discovered_context(
            Path::new("/workspace/docs/research-notes.md"),
            &DiscoveredContextClassificationMetadata {
                root_source: Some(PathBuf::from("/workspace")),
                ..DiscoveredContextClassificationMetadata::default()
            },
        );

        assert_eq!(classification.folder_path, PathBuf::from("docs"));
        assert_eq!(classification.classification, Classification::Shared);
        assert_eq!(classification.rule_id, "shared-default");
        assert_eq!(classification.tags, vec!["discovered".to_string()]);
    }

    #[cfg(unix)]
    fn executable_fixture(name: &str, content: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;

        let fixture_dir = env::temp_dir().join(format!("{name}-{}", Uuid::new_v4()));
        fs::create_dir_all(&fixture_dir).expect("fixture dir should be created");
        let fixture_path = fixture_dir.join(name);
        fs::write(&fixture_path, content).expect("fixture script should be writable");
        let mut permissions = fs::metadata(&fixture_path)
            .expect("fixture metadata should be readable")
            .permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&fixture_path, permissions)
            .expect("fixture script should be executable");
        fixture_path
    }

    #[test]
    fn classification_rule_for_exposes_the_matched_rule() {
        assert_eq!(
            classification_rule_for("planner.md", &PathBuf::from("subagents")).id,
            "subagent-directory-or-name"
        );
        assert_eq!(
            classification_rule_for("reference.md", &PathBuf::from("notes")).id,
            "shared-default"
        );
    }

    #[test]
    fn classification_rules_define_supported_conventions() {
        assert!(MAIN_AGENT_FILE_NAMES.contains(&"agents.md"));
        assert!(MAIN_AGENT_DIRECTORY_PATTERNS.contains(&".codex"));
        assert!(SKILL_DIRECTORY_PATTERNS.contains(&".ctx/skills"));
        assert!(SUBAGENT_DIRECTORY_PATTERNS.contains(&".claude/subagents"));
        assert!(SUBAGENT_FILE_STEM_TOKENS.contains(&"agent"));
    }

    #[test]
    fn classify_discovered_context_derives_folder_relative_to_root() {
        let classification = classify_discovered_context(
            Path::new("/workspace/.claude/agents/reviewer.md"),
            &DiscoveredContextClassificationMetadata {
                root_source: Some(PathBuf::from("/workspace")),
                ..DiscoveredContextClassificationMetadata::default()
            },
        );

        assert_eq!(classification.file_name, "reviewer.md");
        assert_eq!(classification.folder_path, PathBuf::from(".claude/agents"));
        assert_eq!(classification.classification, Classification::Subagent);
        assert_eq!(classification.rule_id, "subagent-directory-or-name");
        assert!(classification.tags.contains(&"agents".to_string()));
    }

    #[test]
    fn classify_discovered_context_uses_metadata_when_path_is_ambiguous() {
        let classification = classify_discovered_context(
            Path::new("/workspace/docs/research.md"),
            &DiscoveredContextClassificationMetadata {
                root_source: Some(PathBuf::from("/workspace")),
                tags: vec!["subagent".to_string(), "review".to_string()],
                ..DiscoveredContextClassificationMetadata::default()
            },
        );

        assert_eq!(classification.classification, Classification::Subagent);
        assert_eq!(classification.rule_id, "subagent-directory-or-name");
        assert!(classification.tags.contains(&"subagent".to_string()));
        assert!(classification.tags.contains(&"review".to_string()));
    }

    #[test]
    fn classify_discovered_context_keeps_skills_shared_before_agent_name_tokens() {
        let classification = classify_discovered_context(
            Path::new("/workspace/skills/agent-tool.md"),
            &DiscoveredContextClassificationMetadata {
                root_source: Some(PathBuf::from("/workspace")),
                ..DiscoveredContextClassificationMetadata::default()
            },
        );

        assert_eq!(classification.classification, Classification::Shared);
        assert_eq!(classification.rule_id, "skill-directory");
        assert!(classification.tags.contains(&"skills".to_string()));
    }

    fn classification_request_fixture(target_cli: CliTarget) -> HeadlessClassificationRequest {
        HeadlessClassificationRequest {
            request_id: Uuid::new_v4(),
            target_cli,
            context_id: None,
            title: Some("Reviewer".to_string()),
            content: "# Reviewer".to_string(),
            file_path: PathBuf::from("/workspace/.claude/agents/reviewer.md"),
            vault_scope: Some(VaultScope::Local),
            folder_path: PathBuf::from(".claude/agents"),
            import_source: None,
            import_source_type: Some(ImportSourceType::SubagentMarkdown),
            existing_tags: Vec::new(),
            existing_wikilinks: Vec::new(),
        }
    }
}
