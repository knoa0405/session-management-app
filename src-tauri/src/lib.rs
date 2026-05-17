use ctx_core::{
    app_status, classify_import_markdown_content, classify_work_context_detail,
    configured_context_watch_roots, create_context_file, create_session_handoff_context_file,
    delete_resolved_context_markdown, diff_context_file_snapshots,
    discover_existing_context_file_results, injection_strategy, list_context_files_with_discovered,
    list_presets_from_resolved_overlay, list_session_handoff_contexts,
    lookup_markdown_context_index, lookup_markdown_contexts_by_tag,
    materialize_discovered_context_files, read_resolved_context_fragment,
    read_resolved_session_handoff_context, resolve_claude_session_log_roots,
    resolve_codex_session_log_roots, resolve_overlay_vault,
    review_import_classification as review_core_import_classification,
    save_preset_execution_settings as save_core_preset_execution_settings,
    save_preset_subagent_manifest as save_core_preset_subagent_manifest,
    snapshot_context_directories, sync_markdown_context_index_events,
    update_resolved_context_markdown, AppStatus, Classification, ClaudeSessionLogScanner,
    CliTarget, CodexSessionLogScanner, ContextDiscoveryResult, ContextFileChangeEvent,
    ContextFileSnapshot, ContextFragment, ContextWatchRoot, ImportSourceType,
    ImportTimeClassificationRequest, ImportTimeClassificationResult,
    IncrementalMarkdownIndexReport, LocalHeadlessCliClassificationAdapter,
    MarkdownFileMetadataRecord, OverlayMarkdownIndexLookup, PresetExecutionSettingsUpdate,
    PresetSummary, SavedSessionHandoffContext, SessionHandoffContext,
    SessionLogDetail as AgentSessionDetail, SessionLogMessage as AgentSessionMessage,
    SessionLogMetadata as AgentSessionSummary, SessionLogProvider, SessionLogScanRequest,
    SessionLogScanner, SubagentManifestUpdate, VaultError, VaultRoots, VaultScope,
    WorkContextClassificationResult, WorkContextRefineMode, WorkContextSignalSet,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    process::Command,
    time::SystemTime,
};

#[tauri::command]
fn health_check() -> AppStatus {
    app_status()
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct CtxIntegrationProbe {
    cli_name: String,
    sidecar_name: String,
    sidecar_configured: bool,
    wrapper_ready: bool,
    supported_targets: Vec<CliTarget>,
    notes: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CtxIntegrationRequest {
    target: CliTarget,
    preset_id: Option<String>,
    working_dir: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreateContextFileRequest {
    file_name: String,
    content: Option<String>,
    folder_path: Option<PathBuf>,
    vault_scope: Option<VaultScope>,
    working_dir: Option<PathBuf>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ListContextFilesRequest {
    working_dir: Option<PathBuf>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct OpenMarkdownContextRequest {
    file_path: PathBuf,
    working_dir: Option<PathBuf>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SaveMarkdownContextRequest {
    file_path: PathBuf,
    content: String,
    working_dir: Option<PathBuf>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReviewImportClassificationRequest {
    file_path: PathBuf,
    classification: Classification,
    working_dir: Option<PathBuf>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DeleteMarkdownContextRequest {
    file_path: PathBuf,
    working_dir: Option<PathBuf>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ClassifyImportMarkdownRequest {
    content: String,
    file_name: Option<String>,
    folder_path: Option<PathBuf>,
    import_source_type: Option<ImportSourceType>,
    target_cli: Option<CliTarget>,
    use_llm: Option<bool>,
    timeout_ms: Option<u64>,
    #[serde(default)]
    existing_tags: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PollContextWatchRequest {
    working_dir: Option<PathBuf>,
    previous_snapshot: Option<ContextFileSnapshot>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LookupMarkdownIndexRequest {
    file_path: Option<PathBuf>,
    tag: Option<String>,
    working_dir: Option<PathBuf>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct LookupMarkdownIndexResponse {
    file: Option<OverlayMarkdownIndexLookup>,
    tagged_contexts: Vec<MarkdownFileMetadataRecord>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ContextWatchPollResponse {
    roots: Vec<ContextWatchRoot>,
    snapshot: ContextFileSnapshot,
    events: Vec<ContextFileChangeEvent>,
    incremental_index_reports: Vec<IncrementalMarkdownIndexReport>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct CtxIntegrationPreview {
    target: CliTarget,
    preset_id: Option<String>,
    working_dir: Option<String>,
    strategy: &'static str,
    command_preview: Vec<String>,
    will_spawn_process: bool,
    will_mutate_files: bool,
    message: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReadAgentSessionRequest {
    provider: String,
    file_path: PathBuf,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RefineSessionContextRequest {
    draft_content: String,
    target_cli: Option<CliTarget>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct AgentSessionSummaryResponse {
    provider: String,
    session_id: String,
    title: String,
    updated_at: Option<String>,
    cwd: Option<String>,
    file_path: PathBuf,
    message_count: usize,
    last_user_message: Option<String>,
    classification_metadata: Option<SessionClassificationMetadata>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct AgentSessionDetailResponse {
    summary: AgentSessionSummary,
    messages: Vec<AgentSessionMessage>,
    distilled_markdown: String,
    classification_metadata: SessionClassificationMetadata,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct SessionClassificationMetadata {
    source_tool: String,
    source_session_ref: String,
    source_working_directory: String,
    source_log_path: String,
    work_context_category: String,
    work_context_categories: Vec<String>,
    work_context_classification_status: ctx_core::ClassificationStatus,
    work_context_confidence_score: u8,
    work_context_rationale: String,
    distillation_focus: Vec<String>,
}

impl From<WorkContextClassificationResult> for SessionClassificationMetadata {
    fn from(classification: WorkContextClassificationResult) -> Self {
        Self {
            source_tool: classification.source_tool.as_str().to_string(),
            source_session_ref: classification.source_session_ref,
            source_working_directory: classification.source_working_directory,
            source_log_path: classification.source_log_path,
            work_context_category: classification.category.as_str().to_string(),
            work_context_categories: classification
                .categories
                .into_iter()
                .map(|category| category.as_str().to_string())
                .collect(),
            work_context_classification_status: classification.status,
            work_context_confidence_score: classification.confidence_score,
            work_context_rationale: classification.rationale,
            distillation_focus: classification.distillation_focus,
        }
    }
}

impl From<AgentSessionSummary> for AgentSessionSummaryResponse {
    fn from(summary: AgentSessionSummary) -> Self {
        Self {
            provider: summary.provider,
            session_id: summary.session_id,
            title: summary.title,
            updated_at: summary.updated_at,
            cwd: summary.cwd,
            file_path: summary.file_path,
            message_count: summary.message_count,
            last_user_message: summary.last_user_message,
            classification_metadata: None,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SaveAgentSessionContextRequest {
    provider: String,
    file_path: PathBuf,
    content: String,
    working_dir: Option<PathBuf>,
}

#[tauri::command]
fn probe_ctx_integration() -> CtxIntegrationProbe {
    let status = app_status();

    CtxIntegrationProbe {
        cli_name: "ctx".to_string(),
        sidecar_name: "ctx".to_string(),
        sidecar_configured: true,
        wrapper_ready: status.wrapper_ready,
        supported_targets: vec![CliTarget::Claude, CliTarget::Codex],
        notes: vec![
            "Bundled sidecar is declared in tauri.conf.json as bin/ctx.".to_string(),
            "This probe is a placeholder and does not spawn the ctx wrapper.".to_string(),
        ],
    }
}

#[tauri::command]
fn invoke_ctx_integration(request: CtxIntegrationRequest) -> CtxIntegrationPreview {
    build_ctx_launch_preview(request)
}

#[tauri::command]
fn preview_ctx_launch(request: CtxIntegrationRequest) -> CtxIntegrationPreview {
    build_ctx_launch_preview(request)
}

#[tauri::command]
fn list_agent_sessions() -> Result<Vec<AgentSessionSummaryResponse>, String> {
    let home_dir = home_dir()?;
    let working_dir = std::env::current_dir()
        .map_err(|error| format!("failed to resolve current working directory: {error}"))?;
    let mut sessions = Vec::new();

    sessions.extend(list_codex_sessions(&home_dir, &working_dir)?);
    sessions.extend(list_claude_sessions(&home_dir, &working_dir)?);
    sessions.sort_by(|left, right| right.updated_at.cmp(&left.updated_at));
    sessions.truncate(250);

    Ok(sessions
        .into_iter()
        .map(session_summary_with_classification)
        .collect())
}

#[tauri::command]
fn read_agent_session(
    request: ReadAgentSessionRequest,
) -> Result<AgentSessionDetailResponse, String> {
    let file_path = request.file_path;
    if !file_path.exists() {
        return Err(format!(
            "session file does not exist: {}",
            file_path.display()
        ));
    }

    let provider = request.provider.to_lowercase();
    let detail = match provider.as_str() {
        "codex" => parse_codex_session_file(&file_path, None)?,
        "claude" => parse_claude_session_file(&file_path)?,
        other => return Err(format!("unsupported session provider: {other}")),
    };

    let classification = classify_work_context_detail(&detail)
        .map_err(|error| format!("failed to classify session context: {error}"))?;

    Ok(AgentSessionDetailResponse {
        summary: detail.summary,
        messages: detail.messages,
        distilled_markdown: detail.distilled_markdown,
        classification_metadata: SessionClassificationMetadata::from(classification),
    })
}

#[tauri::command]
fn save_agent_session_context(
    request: SaveAgentSessionContextRequest,
) -> Result<ContextFragment, String> {
    if request.content.trim().is_empty() {
        return Err("session handoff content cannot be empty".to_string());
    }

    let working_dir = resolve_requested_working_dir(request.working_dir)?;
    let detail = parse_agent_session_detail(&request.provider, &request.file_path)?;
    let classification = classify_work_context_detail(&detail)
        .map_err(|error| format!("failed to classify session context: {error}"))?;
    let signal_set = WorkContextSignalSet::from_session_detail(&detail)
        .map_err(|error| format!("failed to normalize session handoff context: {error}"))?;
    let handoff = SessionHandoffContext::from_classified_signals(
        &signal_set,
        &classification,
        current_timestamp_string()?,
        &request.content,
        default_launch_target_for_session_detail(&detail),
        WorkContextRefineMode::Raw,
    )
    .map_err(|error| format!("failed to extract distilled session handoff fields: {error}"))?;
    handoff
        .validate_for_save()
        .map_err(|error| format!("invalid distilled session handoff context: {error}"))?;
    let roots = VaultRoots::discover(&working_dir);
    let file_name = format!(
        "{}-{}.md",
        sanitize_context_file_token(&detail.summary.provider),
        sanitize_context_file_token(&detail.summary.session_id)
    );

    create_session_handoff_context_file(
        &roots,
        VaultScope::Local,
        PathBuf::from("session-history"),
        &file_name,
        &handoff,
    )
    .map(|saved| saved.fragment)
    .map_err(|error| error.to_string())
}

fn default_launch_target_for_session_detail(detail: &AgentSessionDetail) -> CliTarget {
    match detail.summary.provider_kind() {
        Some(SessionLogProvider::Claude) => CliTarget::Claude,
        Some(SessionLogProvider::Codex) | None => CliTarget::Codex,
    }
}

fn current_timestamp_string() -> Result<String, String> {
    let seconds = SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|error| format!("failed to resolve current timestamp: {error}"))?
        .as_secs();
    Ok(format!("unix-seconds:{seconds}"))
}

fn parse_agent_session_detail(
    provider: &str,
    file_path: &Path,
) -> Result<AgentSessionDetail, String> {
    if !file_path.exists() {
        return Err(format!(
            "session file does not exist: {}",
            file_path.display()
        ));
    }

    match provider.to_lowercase().as_str() {
        "codex" => parse_codex_session_file(file_path, None),
        "claude" => parse_claude_session_file(file_path),
        other => Err(format!("unsupported session provider: {other}")),
    }
}

fn session_summary_with_classification(
    summary: AgentSessionSummary,
) -> AgentSessionSummaryResponse {
    let classification_metadata = parse_agent_session_detail(&summary.provider, &summary.file_path)
        .ok()
        .and_then(|detail| classify_work_context_detail(&detail).ok())
        .map(SessionClassificationMetadata::from);

    AgentSessionSummaryResponse {
        classification_metadata,
        ..AgentSessionSummaryResponse::from(summary)
    }
}

fn sanitize_context_file_token(value: &str) -> String {
    let sanitized = value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | '-') {
                character
            } else {
                '-'
            }
        })
        .collect::<String>()
        .trim_matches('-')
        .to_string();

    if sanitized.is_empty() {
        "session".to_string()
    } else {
        sanitized.chars().take(120).collect()
    }
}

#[tauri::command]
fn refine_session_context(request: RefineSessionContextRequest) -> Result<String, String> {
    if request.draft_content.trim().is_empty() {
        return Err("refinement draft cannot be empty".to_string());
    }

    let target = request.target_cli.unwrap_or(CliTarget::Claude);
    let prompt = build_session_refinement_prompt(&request.draft_content);
    run_refinement_cli(target, &prompt)
}

fn build_session_refinement_prompt(draft: &str) -> String {
    format!(
        r#"You are preparing context for a new coding-agent session.

Rewrite the raw previous-session transcript into a concise Korean handoff note.
Do not invent facts. Preserve concrete filenames, commands, decisions, verification results, blockers, and remaining work when present.
Remove noisy tool output and repeated status chatter.

Return only markdown with this exact structure. Keep the heading text in English so the app can validate and save the result, but write the bullet content in Korean when useful:

# Previous Session Context

## Handoff Summary

### Goals

### Current state

### Key changed files

### Decisions

### Verification results

### Remaining work

### Notes for next session

Raw previous-session draft:

```markdown
{draft}
```
"#
    )
}

fn run_refinement_cli(target: CliTarget, prompt: &str) -> Result<String, String> {
    let output = match target {
        CliTarget::Claude => {
            let program = std::env::var("CTX_CLAUDE_BIN").unwrap_or_else(|_| "claude".to_string());
            Command::new(&program)
                .arg("--print")
                .arg("--output-format")
                .arg("text")
                .arg(prompt)
                .output()
                .map_err(|error| {
                    format!("failed to launch Claude refinement CLI '{program}': {error}")
                })?
        }
        CliTarget::Codex => {
            let program = std::env::var("CTX_CODEX_BIN").unwrap_or_else(|_| "codex".to_string());
            Command::new(&program)
                .arg("exec")
                .arg(prompt)
                .output()
                .map_err(|error| {
                    format!("failed to launch Codex refinement CLI '{program}': {error}")
                })?
        }
    };

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "refinement CLI exited with {}: {}",
            output.status,
            stderr.trim()
        ));
    }

    let refined = String::from_utf8(output.stdout)
        .map_err(|error| format!("refinement CLI returned non-UTF8 output: {error}"))?;
    let refined = refined.trim();
    if refined.is_empty() {
        return Err("refinement CLI returned empty output".to_string());
    }

    Ok(refined.to_string())
}

fn home_dir() -> Result<PathBuf, String> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or_else(|| "HOME directory is not available".to_string())
}

fn list_codex_sessions(
    home_dir: &Path,
    working_dir: &Path,
) -> Result<Vec<AgentSessionSummary>, String> {
    let roots = VaultRoots::discover(working_dir);
    let codex_roots = resolve_codex_session_log_roots(&roots, working_dir, home_dir)
        .map_err(|error| format!("failed to resolve Codex session log roots: {error}"))?;
    let root_paths = codex_roots
        .into_iter()
        .map(|root| root.path)
        .collect::<Vec<_>>();
    let scanner = CodexSessionLogScanner;
    let result = scanner
        .scan_session_logs(&SessionLogScanRequest {
            provider: SessionLogProvider::Codex,
            home_dir: home_dir.to_path_buf(),
            working_dir: working_dir.to_path_buf(),
            root_paths,
        })
        .map_err(|error| error.to_string())?;

    Ok(result.sessions)
}

fn list_claude_sessions(
    home_dir: &Path,
    working_dir: &Path,
) -> Result<Vec<AgentSessionSummary>, String> {
    let roots = VaultRoots::discover(working_dir);
    let claude_roots = resolve_claude_session_log_roots(&roots, working_dir, home_dir)
        .map_err(|error| format!("failed to resolve Claude session log roots: {error}"))?;
    let root_paths = claude_roots
        .into_iter()
        .map(|root| root.path)
        .collect::<Vec<_>>();
    let scanner = ClaudeSessionLogScanner;
    let result = scanner
        .scan_session_logs(&SessionLogScanRequest {
            provider: SessionLogProvider::Claude,
            home_dir: home_dir.to_path_buf(),
            working_dir: working_dir.to_path_buf(),
            root_paths,
        })
        .map_err(|error| error.to_string())?;

    Ok(result.sessions)
}

fn parse_codex_session_file(
    file_path: &Path,
    index: Option<&HashMap<String, (String, String)>>,
) -> Result<AgentSessionDetail, String> {
    let content = fs::read_to_string(file_path).map_err(|error| error.to_string())?;
    let mut session_id = file_stem_session_id(file_path);
    let mut cwd = None;
    let mut created_at = None;
    let mut messages = Vec::new();
    let mut last_user_message = None;

    for line in content.lines() {
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        let timestamp = value
            .get("timestamp")
            .and_then(Value::as_str)
            .map(ToString::to_string);
        let record_type = value
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let payload = value.get("payload").unwrap_or(&Value::Null);

        if record_type == "session_meta" {
            if let Some(id) = payload.get("id").and_then(Value::as_str) {
                session_id = id.to_string();
            }
            cwd = payload
                .get("cwd")
                .and_then(Value::as_str)
                .map(ToString::to_string);
            created_at = payload
                .get("timestamp")
                .and_then(Value::as_str)
                .map(ToString::to_string)
                .or(timestamp);
            continue;
        }

        if record_type == "event_msg" {
            match payload
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or_default()
            {
                "user_message" => {
                    if let Some(message) = payload.get("message").and_then(Value::as_str) {
                        let text = truncate_text(message, 12_000);
                        last_user_message = Some(truncate_text(&text, 220));
                        messages.push(AgentSessionMessage {
                            role: "user".to_string(),
                            timestamp,
                            content: text,
                        });
                    }
                }
                "agent_message" => {
                    if let Some(message) = payload.get("message").and_then(Value::as_str) {
                        messages.push(AgentSessionMessage {
                            role: "assistant".to_string(),
                            timestamp,
                            content: truncate_text(message, 12_000),
                        });
                    }
                }
                _ => {}
            }
            continue;
        }

        if record_type == "response_item"
            && payload.get("type").and_then(Value::as_str) == Some("message")
        {
            let role = payload
                .get("role")
                .and_then(Value::as_str)
                .unwrap_or("assistant")
                .to_string();
            let text = text_from_json_content(payload.get("content").unwrap_or(&Value::Null));
            if !text.trim().is_empty() {
                messages.push(AgentSessionMessage {
                    role,
                    timestamp,
                    content: truncate_text(&text, 12_000),
                });
            }
        }
    }

    let (indexed_title, indexed_updated_at) = index
        .and_then(|entries| entries.get(&session_id))
        .cloned()
        .unwrap_or_default();
    let title = if indexed_title.trim().is_empty() {
        last_user_message
            .clone()
            .unwrap_or_else(|| "Codex 세션".to_string())
    } else {
        indexed_title
    };
    let updated_at = if indexed_updated_at.trim().is_empty() {
        created_at.or_else(|| modified_time_string(file_path))
    } else {
        Some(indexed_updated_at)
    };
    let summary = AgentSessionSummary {
        provider: "codex".to_string(),
        session_id,
        title: truncate_text(&title, 120),
        updated_at,
        cwd,
        file_path: file_path.to_path_buf(),
        message_count: messages.len(),
        last_user_message,
    };
    let distilled_markdown = distilled_session_markdown(&summary, &messages);

    Ok(AgentSessionDetail {
        summary,
        messages,
        events: Vec::new(),
        distilled_markdown,
    })
}

fn parse_claude_session_file(file_path: &Path) -> Result<AgentSessionDetail, String> {
    let content = fs::read_to_string(file_path).map_err(|error| error.to_string())?;
    let mut session_id = file_stem_session_id(file_path);
    let mut cwd = None;
    let mut updated_at = None;
    let mut messages = Vec::new();
    let mut last_user_message = None;

    for line in content.lines() {
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        let record_type = value
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if let Some(id) = value.get("sessionId").and_then(Value::as_str) {
            session_id = id.to_string();
        }
        if cwd.is_none() {
            cwd = value
                .get("cwd")
                .and_then(Value::as_str)
                .map(ToString::to_string);
        }
        let timestamp = value
            .get("timestamp")
            .and_then(Value::as_str)
            .map(ToString::to_string);
        if timestamp.is_some() {
            updated_at = timestamp.clone();
        }

        match record_type {
            "user" | "assistant" => {
                let message = value.get("message").unwrap_or(&Value::Null);
                let role = message
                    .get("role")
                    .and_then(Value::as_str)
                    .unwrap_or(record_type)
                    .to_string();
                let text = text_from_json_content(message.get("content").unwrap_or(&Value::Null));
                if !text.trim().is_empty() {
                    let text = truncate_text(&text, 12_000);
                    if role == "user" {
                        last_user_message = Some(truncate_text(&text, 220));
                    }
                    messages.push(AgentSessionMessage {
                        role,
                        timestamp,
                        content: text,
                    });
                }
            }
            "queue-operation" => {
                if let Some(message) = value.get("content").and_then(Value::as_str) {
                    let text = truncate_text(message, 12_000);
                    last_user_message = Some(truncate_text(&text, 220));
                    messages.push(AgentSessionMessage {
                        role: "user".to_string(),
                        timestamp,
                        content: text,
                    });
                }
            }
            _ => {}
        }
    }

    let title = last_user_message
        .clone()
        .unwrap_or_else(|| "Claude 세션".to_string());
    let summary = AgentSessionSummary {
        provider: "claude".to_string(),
        session_id,
        title: truncate_text(&title, 120),
        updated_at: updated_at.or_else(|| modified_time_string(file_path)),
        cwd,
        file_path: file_path.to_path_buf(),
        message_count: messages.len(),
        last_user_message,
    };
    let distilled_markdown = distilled_session_markdown(&summary, &messages);

    Ok(AgentSessionDetail {
        summary,
        messages,
        events: Vec::new(),
        distilled_markdown,
    })
}

fn text_from_json_content(content: &Value) -> String {
    match content {
        Value::String(text) => text.clone(),
        Value::Array(items) => items
            .iter()
            .filter_map(|item| {
                item.as_str()
                    .map(ToString::to_string)
                    .or_else(|| {
                        item.get("text")
                            .and_then(Value::as_str)
                            .map(ToString::to_string)
                    })
                    .or_else(|| {
                        item.get("content")
                            .map(text_from_json_content)
                            .filter(|text| !text.trim().is_empty())
                    })
            })
            .collect::<Vec<_>>()
            .join("\n\n"),
        Value::Object(map) => map
            .get("text")
            .and_then(Value::as_str)
            .map(ToString::to_string)
            .or_else(|| {
                map.get("content")
                    .map(text_from_json_content)
                    .filter(|text| !text.trim().is_empty())
            })
            .unwrap_or_default(),
        _ => String::new(),
    }
}

fn distilled_session_markdown(
    summary: &AgentSessionSummary,
    messages: &[AgentSessionMessage],
) -> String {
    let mut markdown = String::new();
    markdown.push_str("---\n");
    markdown.push_str("classification: shared\n");
    markdown.push_str("tags: [session-history, resume-context]\n");
    markdown.push_str("---\n\n");
    markdown.push_str("# 세션 컨텍스트\n\n");
    markdown.push_str(&format!("- 제공자: {}\n", summary.provider));
    markdown.push_str(&format!("- 세션 ID: {}\n", summary.session_id));
    if let Some(updated_at) = &summary.updated_at {
        markdown.push_str(&format!("- 마지막 업데이트: {updated_at}\n"));
    }
    if let Some(cwd) = &summary.cwd {
        markdown.push_str(&format!("- 작업 디렉터리: `{cwd}`\n"));
    }
    markdown.push_str(&format!(
        "- 원본 로그: `{}`\n\n",
        summary.file_path.display()
    ));
    markdown.push_str("## 다음 세션에 전달할 핵심 맥락\n\n");
    markdown.push_str(
        "- 이 초안은 이전 세션 로그에서 사용자 요청과 어시스턴트 응답을 추출한 것입니다.\n",
    );
    markdown.push_str(
        "- 새 세션에 넣기 전에 불필요한 중간 출력, 민감 정보, 오래된 결정을 정리하세요.\n\n",
    );
    markdown.push_str("## 대화 타임라인\n\n");

    for message in messages.iter().take(80) {
        let role = match message.role.as_str() {
            "user" => "사용자",
            "assistant" => "어시스턴트",
            other => other,
        };
        markdown.push_str(&format!("### {role}\n\n"));
        markdown.push_str(message.content.trim());
        markdown.push_str("\n\n");
    }

    if messages.len() > 80 {
        markdown.push_str(&format!(
            "_이후 메시지 {}개는 초안 길이 제한으로 생략되었습니다._\n",
            messages.len() - 80
        ));
    }

    markdown
}

fn file_stem_session_id(file_path: &Path) -> String {
    file_path
        .file_stem()
        .and_then(|name| name.to_str())
        .unwrap_or("unknown-session")
        .trim_start_matches("rollout-")
        .to_string()
}

fn modified_time_string(file_path: &Path) -> Option<String> {
    fs::metadata(file_path)
        .and_then(|metadata| metadata.modified())
        .ok()
        .and_then(|time| time.duration_since(SystemTime::UNIX_EPOCH).ok())
        .map(|duration| format!("{}초", duration.as_secs()))
}

fn truncate_text(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }

    let mut truncated = text.chars().take(max_chars).collect::<String>();
    truncated.push_str("...");
    truncated
}

fn default_working_dir() -> Result<PathBuf, String> {
    let cwd = std::env::current_dir()
        .map_err(|error| format!("failed to resolve current working directory: {error}"))?;

    if cwd.file_name().and_then(|name| name.to_str()) == Some("src-tauri") {
        if let Some(parent) = cwd.parent() {
            if parent.join("package.json").exists() && parent.join("src-tauri").exists() {
                return Ok(parent.to_path_buf());
            }
        }
    }

    Ok(cwd)
}

fn resolve_requested_working_dir(working_dir: Option<PathBuf>) -> Result<PathBuf, String> {
    working_dir.map_or_else(default_working_dir, Ok)
}

#[tauri::command]
fn create_markdown_context(request: CreateContextFileRequest) -> Result<ContextFragment, String> {
    let working_dir = resolve_requested_working_dir(request.working_dir)?;
    let roots = VaultRoots::discover(&working_dir);
    let folder_path = request.folder_path.unwrap_or_default();
    let content = request.content.unwrap_or_default();

    create_context_file(
        &roots,
        request.vault_scope.unwrap_or(VaultScope::Local),
        folder_path,
        &request.file_name,
        &content,
    )
    .map_err(|error| error.to_string())
}

#[tauri::command]
fn list_markdown_contexts(
    request: Option<ListContextFilesRequest>,
) -> Result<Vec<ContextFragment>, String> {
    let working_dir =
        resolve_requested_working_dir(request.and_then(|request| request.working_dir))?;
    resolve_overlay_vault(&working_dir)
        .map(|vault| vault.contexts)
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn list_saved_session_contexts(
    request: Option<ListContextFilesRequest>,
) -> Result<Vec<SavedSessionHandoffContext>, String> {
    let working_dir =
        resolve_requested_working_dir(request.and_then(|request| request.working_dir))?;
    let roots = VaultRoots::discover(&working_dir);

    list_session_handoff_contexts(&roots).map_err(|error| error.to_string())
}

#[tauri::command]
fn discover_markdown_contexts(
    request: Option<ListContextFilesRequest>,
) -> Result<Vec<ContextFragment>, String> {
    let working_dir =
        resolve_requested_working_dir(request.and_then(|request| request.working_dir))?;

    list_context_files_with_discovered(&working_dir).map_err(|error| error.to_string())
}

#[tauri::command]
fn scan_existing_markdown_contexts(
    request: Option<ListContextFilesRequest>,
) -> Result<Vec<ContextDiscoveryResult>, String> {
    let working_dir =
        resolve_requested_working_dir(request.and_then(|request| request.working_dir))?;

    discover_existing_context_file_results(&working_dir).map_err(|error| error.to_string())
}

#[tauri::command]
fn import_markdown_contexts(
    request: Option<ListContextFilesRequest>,
) -> Result<Vec<ContextFragment>, String> {
    let working_dir =
        resolve_requested_working_dir(request.and_then(|request| request.working_dir))?;

    materialize_discovered_context_files(&working_dir).map_err(|error| error.to_string())
}

#[tauri::command]
fn classify_import_markdown(
    request: ClassifyImportMarkdownRequest,
) -> Result<ImportTimeClassificationResult, String> {
    if request.content.trim().is_empty() {
        return Err("markdown content cannot be empty for import classification".to_string());
    }

    if request.use_llm.unwrap_or(false) {
        let target_cli = request.target_cli.unwrap_or(CliTarget::Claude);
        let file_path = request
            .file_name
            .as_deref()
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("import.md"));
        let folder_path = request.folder_path.clone().unwrap_or_default();
        let adapter = LocalHeadlessCliClassificationAdapter {
            target_cli,
            program: None,
            working_dir: None,
            timeout_ms: request.timeout_ms.or(Some(30_000)),
            passthrough_args: Vec::new(),
        };
        let result = ctx_core::HeadlessClassificationAdapter::analyze_context(
            &adapter,
            &ctx_core::HeadlessClassificationRequest {
                request_id: uuid::Uuid::new_v4(),
                target_cli,
                context_id: None,
                title: request.file_name.clone(),
                content: request.content,
                file_path,
                vault_scope: None,
                folder_path,
                import_source: None,
                import_source_type: request.import_source_type,
                existing_tags: request.existing_tags,
                existing_wikilinks: Vec::new(),
            },
        )
        .map_err(|error| error.to_string())?;

        return Ok(ImportTimeClassificationResult {
            classification: result.classification,
            status: result.status,
            confidence_score: result.confidence_score,
            rationale: result.rationale,
            rule_id: Some(format!("{:?}", result.adapter_kind)),
            suggested_tags: result.suggested_tags,
        });
    }

    Ok(classify_import_markdown_content(
        &ImportTimeClassificationRequest {
            content: request.content,
            file_name: request.file_name,
            folder_path: request.folder_path,
            import_source_type: request.import_source_type,
            existing_tags: request.existing_tags,
        },
    ))
}

#[tauri::command]
fn open_markdown_context(request: OpenMarkdownContextRequest) -> Result<ContextFragment, String> {
    let working_dir = resolve_requested_working_dir(request.working_dir)?;

    read_resolved_context_fragment(&working_dir, &request.file_path)
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn open_saved_session_context(
    request: OpenMarkdownContextRequest,
) -> Result<SavedSessionHandoffContext, String> {
    let working_dir = resolve_requested_working_dir(request.working_dir)?;

    read_resolved_session_handoff_context(&working_dir, &request.file_path)
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn save_markdown_context(request: SaveMarkdownContextRequest) -> Result<String, String> {
    let working_dir = resolve_requested_working_dir(request.working_dir)?;

    update_resolved_context_markdown(&working_dir, &request.file_path, &request.content)
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn review_import_classification(
    request: ReviewImportClassificationRequest,
) -> Result<ContextFragment, String> {
    let working_dir = resolve_requested_working_dir(request.working_dir)?;

    review_core_import_classification(&working_dir, &request.file_path, request.classification)
        .map_err(format_review_import_classification_error)
}

fn format_review_import_classification_error(error: VaultError) -> String {
    match error {
        VaultError::EmptyFileName
        | VaultError::InvalidFileName(_)
        | VaultError::InvalidExtension(_)
        | VaultError::InvalidFolderPath(_)
        | VaultError::MissingContext(_)
        | VaultError::MissingLocalVault => format!("validation error: {error}"),
        VaultError::Io(message) if message.contains("no import metadata to review") => {
            format!("validation error: {message}")
        }
        other => other.to_string(),
    }
}

#[tauri::command]
fn delete_markdown_context(request: DeleteMarkdownContextRequest) -> Result<PathBuf, String> {
    let working_dir = resolve_requested_working_dir(request.working_dir)?;

    delete_resolved_context_markdown(&working_dir, &request.file_path)
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn poll_context_watch(
    request: Option<PollContextWatchRequest>,
) -> Result<ContextWatchPollResponse, String> {
    let request = request.unwrap_or(PollContextWatchRequest {
        working_dir: None,
        previous_snapshot: None,
    });
    let working_dir = resolve_requested_working_dir(request.working_dir)?;
    let roots = configured_context_watch_roots(&working_dir).map_err(|error| error.to_string())?;
    let snapshot = snapshot_context_directories(&roots).map_err(|error| error.to_string())?;
    let has_previous_snapshot = request.previous_snapshot.is_some();
    let previous_snapshot = request.previous_snapshot.unwrap_or_default();
    let events = diff_context_file_snapshots(&previous_snapshot, &snapshot);
    let incremental_index_reports = if has_previous_snapshot {
        sync_markdown_context_index_events(&working_dir, &events)
            .map_err(|error| error.to_string())?
    } else {
        Vec::new()
    };

    Ok(ContextWatchPollResponse {
        roots,
        snapshot,
        events,
        incremental_index_reports,
    })
}

#[tauri::command]
fn lookup_markdown_index(
    request: LookupMarkdownIndexRequest,
) -> Result<LookupMarkdownIndexResponse, String> {
    let working_dir = resolve_requested_working_dir(request.working_dir)?;

    let file = match request.file_path {
        Some(path) => {
            lookup_markdown_context_index(&working_dir, &path).map_err(|error| error.to_string())?
        }
        None => None,
    };
    let tagged_contexts = match request.tag {
        Some(tag) if !tag.trim().is_empty() => lookup_markdown_contexts_by_tag(&working_dir, &tag)
            .map_err(|error| error.to_string())?,
        _ => Vec::new(),
    };

    Ok(LookupMarkdownIndexResponse {
        file,
        tagged_contexts,
    })
}

#[tauri::command]
fn list_presets(request: Option<ListContextFilesRequest>) -> Result<Vec<PresetSummary>, String> {
    let working_dir =
        resolve_requested_working_dir(request.and_then(|request| request.working_dir))?;
    let vault = resolve_overlay_vault(&working_dir).map_err(|error| error.to_string())?;

    list_presets_from_resolved_overlay(&vault.roots, &vault.contexts, &working_dir)
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn save_preset_execution_settings(
    request: PresetExecutionSettingsUpdate,
    working_dir: Option<PathBuf>,
) -> Result<PresetSummary, String> {
    let working_dir = resolve_requested_working_dir(working_dir)?;
    let roots = VaultRoots::discover(&working_dir);

    save_core_preset_execution_settings(&roots, request, &working_dir)
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn save_preset_subagent_manifest(
    request: SubagentManifestUpdate,
    working_dir: Option<PathBuf>,
) -> Result<PresetSummary, String> {
    let working_dir = resolve_requested_working_dir(working_dir)?;
    let roots = VaultRoots::discover(&working_dir);

    save_core_preset_subagent_manifest(&roots, request, &working_dir)
        .map_err(|error| error.to_string())
}

fn build_ctx_launch_preview(request: CtxIntegrationRequest) -> CtxIntegrationPreview {
    let target_name = match request.target {
        CliTarget::Claude => "claude",
        CliTarget::Codex => "codex",
    };
    let mut command_preview = vec![
        "ctx".to_string(),
        "launch".to_string(),
        target_name.to_string(),
    ];

    if let Some(preset_id) = &request.preset_id {
        command_preview.push("--preset".to_string());
        command_preview.push(preset_id.clone());
    }

    if let Some(working_dir) = &request.working_dir {
        command_preview.push("--working-dir".to_string());
        command_preview.push(working_dir.clone());
    }

    CtxIntegrationPreview {
        target: request.target,
        preset_id: request.preset_id,
        working_dir: request.working_dir,
        strategy: injection_strategy(request.target),
        command_preview,
        will_spawn_process: false,
        will_mutate_files: false,
        message: "Placeholder preview only: full wrapper launch and cleanup are implemented in a later phase."
            .to_string(),
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            classify_import_markdown,
            create_markdown_context,
            delete_markdown_context,
            discover_markdown_contexts,
            health_check,
            import_markdown_contexts,
            invoke_ctx_integration,
            list_agent_sessions,
            lookup_markdown_index,
            list_presets,
            list_markdown_contexts,
            list_saved_session_contexts,
            open_markdown_context,
            open_saved_session_context,
            poll_context_watch,
            probe_ctx_integration,
            preview_ctx_launch,
            read_agent_session,
            refine_session_context,
            review_import_classification,
            save_agent_session_context,
            save_markdown_context,
            save_preset_execution_settings,
            save_preset_subagent_manifest,
            scan_existing_markdown_contexts
        ])
        .run(tauri::generate_context!())
        .expect("failed to run CTX desktop application");
}

#[cfg(test)]
mod tests {
    use super::{
        classify_import_markdown, create_markdown_context, delete_markdown_context,
        discover_markdown_contexts, import_markdown_contexts, invoke_ctx_integration,
        list_claude_sessions, list_codex_sessions, list_markdown_contexts, list_presets,
        list_saved_session_contexts, open_markdown_context, open_saved_session_context,
        poll_context_watch, preview_ctx_launch, probe_ctx_integration, read_agent_session,
        review_import_classification, save_agent_session_context, save_markdown_context,
        save_preset_execution_settings, save_preset_subagent_manifest,
        scan_existing_markdown_contexts, ClassifyImportMarkdownRequest, CreateContextFileRequest,
        CtxIntegrationRequest, DeleteMarkdownContextRequest, ListContextFilesRequest,
        OpenMarkdownContextRequest, PollContextWatchRequest, ReadAgentSessionRequest,
        ReviewImportClassificationRequest, SaveAgentSessionContextRequest,
        SaveMarkdownContextRequest,
    };
    use ctx_core::{
        create_session_handoff_context_file, managed_contexts_dir, managed_presets_dir,
        ClassificationStatus, CliTarget, ContextFileChangeKind, HandoffConstraints,
        InjectionStrategy, PresetExecutionSettingsUpdate, SessionHandoffContext,
        SessionLogProvider, SubagentManifest, SubagentManifestUpdate, SubagentRole, VaultRoots,
        VaultScope, WorkContextCategory, WorkContextRefineMode,
    };
    use std::{
        collections::BTreeMap,
        env, fs,
        path::{Path, PathBuf},
        sync::Mutex,
    };
    use uuid::Uuid;

    static HOME_ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn probe_reports_configured_placeholder_sidecar() {
        let probe = probe_ctx_integration();

        assert_eq!(probe.cli_name, "ctx");
        assert!(probe.sidecar_configured);
        assert!(!probe.wrapper_ready);
        assert_eq!(
            probe.supported_targets,
            vec![CliTarget::Claude, CliTarget::Codex]
        );
    }

    #[test]
    fn preview_ctx_launch_builds_non_mutating_codex_command() {
        let preview = preview_ctx_launch(CtxIntegrationRequest {
            target: CliTarget::Codex,
            preset_id: Some("daily-driver".to_string()),
            working_dir: Some("/tmp/project".to_string()),
        });

        assert_eq!(
            preview.command_preview,
            vec![
                "ctx",
                "launch",
                "codex",
                "--preset",
                "daily-driver",
                "--working-dir",
                "/tmp/project"
            ]
        );
        assert_eq!(preview.strategy, "AGENTS.md section-marker merge");
        assert!(!preview.will_spawn_process);
        assert!(!preview.will_mutate_files);
    }

    #[test]
    fn invoke_ctx_integration_is_placeholder_only() {
        let preview = invoke_ctx_integration(CtxIntegrationRequest {
            target: CliTarget::Claude,
            preset_id: None,
            working_dir: None,
        });

        assert_eq!(preview.command_preview, vec!["ctx", "launch", "claude"]);
        assert_eq!(preview.strategy, "Claude append-system-prompt-file");
        assert!(!preview.will_spawn_process);
        assert!(!preview.will_mutate_files);
    }

    #[test]
    fn create_markdown_context_uses_local_managed_contexts_directory() {
        let base = std::env::temp_dir().join(format!("ctx-tauri-test-{}", Uuid::new_v4()));
        fs::create_dir_all(&base).expect("test working directory should be created");

        let context = create_markdown_context(CreateContextFileRequest {
            file_name: "agent.md".to_string(),
            content: Some("Review [[Shared Context]].".to_string()),
            folder_path: Some(PathBuf::from("agents")),
            vault_scope: Some(VaultScope::Local),
            working_dir: Some(base.clone()),
        })
        .expect("tauri command should create context markdown");

        assert_eq!(
            context.file_path,
            managed_contexts_dir(&base.join(".ctx").join("vault"))
                .join("agents")
                .join("agent.md")
        );
        assert_eq!(context.wikilinks, vec!["Shared Context"]);
        assert_eq!(
            fs::read_to_string(&context.file_path).expect("created file should be readable"),
            "Review [[Shared Context]]."
        );

        fs::remove_dir_all(base).ok();
    }

    #[test]
    fn create_markdown_context_rejects_duplicate_paths() {
        let base = std::env::temp_dir().join(format!("ctx-tauri-test-{}", Uuid::new_v4()));
        fs::create_dir_all(&base).expect("test working directory should be created");
        let request = CreateContextFileRequest {
            file_name: "duplicate.md".to_string(),
            content: Some("first".to_string()),
            folder_path: None,
            vault_scope: Some(VaultScope::Local),
            working_dir: Some(base.clone()),
        };

        create_markdown_context(request.clone()).expect("initial create should pass");
        let error = create_markdown_context(request).expect_err("duplicate create should fail");

        assert!(error.contains("context file already exists"));
        fs::remove_dir_all(base).ok();
    }

    #[test]
    fn poll_context_watch_returns_snapshot_and_normalized_events() {
        let base = std::env::temp_dir().join(format!("ctx-tauri-watch-test-{}", Uuid::new_v4()));
        let local_contexts = managed_contexts_dir(&base.join(".ctx").join("vault"));
        fs::create_dir_all(&local_contexts).expect("local context directory should be created");
        fs::write(local_contexts.join("agent.md"), "# Agent\n")
            .expect("watched markdown should be writable");

        let first = poll_context_watch(Some(PollContextWatchRequest {
            working_dir: Some(base.clone()),
            previous_snapshot: None,
        }))
        .expect("watch poll should snapshot current contexts");

        let local_contexts = local_contexts
            .canonicalize()
            .expect("local context directory should canonicalize");
        assert!(first.roots.iter().any(|root| root.path == local_contexts));
        assert_eq!(first.events.len(), 1);
        assert_eq!(first.events[0].kind, ContextFileChangeKind::Create);
        assert_eq!(first.events[0].relative_path, PathBuf::from("agent.md"));

        let second = poll_context_watch(Some(PollContextWatchRequest {
            working_dir: Some(base.clone()),
            previous_snapshot: Some(first.snapshot),
        }))
        .expect("second watch poll should diff from previous snapshot");

        assert!(second.events.is_empty());
        fs::remove_dir_all(base).ok();
    }

    #[test]
    fn list_markdown_contexts_reflects_newly_created_context_file() {
        let base = std::env::temp_dir().join(format!("ctx-tauri-list-test-{}", Uuid::new_v4()));
        fs::create_dir_all(&base).expect("test working directory should be created");

        let created = create_markdown_context(CreateContextFileRequest {
            file_name: "new-context.md".to_string(),
            content: Some("See [[Related Notes]].".to_string()),
            folder_path: Some(PathBuf::from("agents")),
            vault_scope: Some(VaultScope::Local),
            working_dir: Some(base.clone()),
        })
        .expect("context should be created before refresh");

        let contexts = list_markdown_contexts(Some(ListContextFilesRequest {
            working_dir: Some(base.clone()),
        }))
        .expect("context list refresh should read the vault");

        let refreshed = contexts
            .iter()
            .find(|context| context.file_path == created.file_path)
            .expect("refreshed list should include the newly created markdown file");

        assert_eq!(refreshed.title, "new context");
        assert_eq!(refreshed.wikilinks, vec!["Related Notes"]);
        fs::remove_dir_all(base).ok();
    }

    #[test]
    fn open_markdown_context_reads_selected_file_contents() {
        let base = std::env::temp_dir().join(format!("ctx-tauri-open-test-{}", Uuid::new_v4()));
        fs::create_dir_all(&base).expect("test working directory should be created");
        let roots = ctx_core::VaultRoots::discover(&base);
        let created = ctx_core::create_context_file(
            &roots,
            VaultScope::Local,
            "session-history",
            "claude-open-metadata.md",
            r#"---
classification: shared
tags: [session-history, resume-context, claude, review]
source_tool: claude
source_session_ref: "claude-open-metadata"
source_working_directory: "/tmp/review"
source_log_path: "/tmp/claude-open-metadata.jsonl"
work_context_category: review
work_context_categories: [review, verification]
work_context_classification_status: classified
work_context_confidence_score: 84
work_context_rationale: "Review signals were detected."
distillation_focus: [findings, verification gaps]
---

# Agent

Selected context body."#,
        )
        .expect("selected context should be writable");

        let context = open_markdown_context(OpenMarkdownContextRequest {
            file_path: created.file_path,
            working_dir: Some(base.clone()),
        })
        .expect("selected markdown context should open");

        assert!(context.content.contains("Selected context body."));
        assert_eq!(
            context.llm_classification_status,
            ctx_core::ClassificationStatus::Classified
        );
        assert!(context
            .session_handoff_classification
            .as_ref()
            .is_some_and(
                |metadata| metadata.source_session_ref == "claude-open-metadata"
                    && metadata.work_context_category == "review"
            ));
        fs::remove_dir_all(base).ok();
    }

    #[test]
    fn read_agent_session_returns_classification_metadata() {
        let base =
            std::env::temp_dir().join(format!("ctx-tauri-session-read-test-{}", Uuid::new_v4()));
        fs::create_dir_all(&base).expect("test working directory should be created");
        let session_path = base.join("codex-read-metadata.jsonl");
        fs::write(
            &session_path,
            codex_session_log(
                "codex-read-metadata",
                "Implement session handoff launch flow and verify cleanup",
            ),
        )
        .expect("session log should be writable");

        let detail = read_agent_session(ReadAgentSessionRequest {
            provider: "codex".to_string(),
            file_path: session_path.clone(),
        })
        .expect("session detail should load with classification metadata");

        assert_eq!(detail.summary.session_id, "codex-read-metadata");
        assert_eq!(detail.classification_metadata.source_tool, "codex");
        assert_eq!(
            detail.classification_metadata.source_session_ref,
            "codex-read-metadata"
        );
        assert!(!detail
            .classification_metadata
            .work_context_category
            .is_empty());
        assert!(detail.classification_metadata.work_context_confidence_score > 0);
        fs::remove_dir_all(base).ok();
    }

    #[test]
    fn save_markdown_context_updates_selected_file_contents() {
        let base = std::env::temp_dir().join(format!("ctx-tauri-save-test-{}", Uuid::new_v4()));
        fs::create_dir_all(&base).expect("test working directory should be created");
        let file_path = base.join("agent.md");
        fs::write(&file_path, "# Agent").expect("selected context should be writable");

        let content = save_markdown_context(SaveMarkdownContextRequest {
            file_path: file_path.clone(),
            content: "# Agent\n\nUpdated body.".to_string(),
            working_dir: Some(base.clone()),
        })
        .expect("selected markdown context should save");

        assert_eq!(content, "# Agent\n\nUpdated body.");
        assert_eq!(
            fs::read_to_string(&file_path).expect("saved context should be readable"),
            "# Agent\n\nUpdated body."
        );
        fs::remove_dir_all(base).ok();
    }

    #[test]
    fn save_agent_session_context_writes_classification_metadata() {
        let base =
            std::env::temp_dir().join(format!("ctx-tauri-session-save-test-{}", Uuid::new_v4()));
        fs::create_dir_all(&base).expect("test working directory should be created");
        let session_path = base.join("codex-save-metadata.jsonl");
        fs::write(
            &session_path,
            r#"{"timestamp":"2026-05-11T00:00:00Z","type":"session_meta","payload":{"id":"codex-save-metadata","cwd":"/tmp/project","timestamp":"2026-05-11T00:00:00Z"}}
{"timestamp":"2026-05-11T00:00:01Z","type":"event_msg","payload":{"type":"user_message","message":"Implement session handoff launch flow and verify cleanup"}}
{"timestamp":"2026-05-11T00:00:02Z","type":"event_msg","payload":{"type":"agent_message","message":"Summary: Implemented launch handoff.\nChanged files: src-tauri/src/lib.rs.\nDecision: Persist distilled session fields when saving.\nVerified with cargo test -p ctx-tauri save_agent_session_context_writes_classification_metadata.\nRemaining work: launch saved entries."}}
"#,
        )
        .expect("session log should be writable");

        let context = save_agent_session_context(SaveAgentSessionContextRequest {
            provider: "codex".to_string(),
            file_path: session_path.clone(),
            content: "---\ntags: [old]\n---\n\n# Previous Session Context\n\n## Handoff Summary\n\nImplemented launch handoff.\n\n### Goals\n\n- Implement session handoff launch flow and verify cleanup\n\n### Key changed files\n\n- src-tauri/src/lib.rs\n\n### Decisions\n\n- Persist distilled session fields when saving.\n\n### Verification results\n\n- Verified with cargo test -p ctx-tauri save_agent_session_context_writes_classification_metadata.\n\n### Remaining work\n\n- launch saved entries."
                .to_string(),
            working_dir: Some(base.clone()),
        })
        .expect("session handoff should save");

        let saved =
            fs::read_to_string(&context.file_path).expect("saved handoff should be readable");
        assert!(saved.contains("source_tool: codex"));
        assert!(saved.contains("source_session_ref: \"codex-save-metadata\""));
        assert!(saved.contains("work_context_classification_status: classified"));
        assert!(saved.contains("work_context_categories: ["));
        assert!(saved.contains("distillation_focus: ["));
        assert!(saved.contains("session_handoff_format_version: 1"));
        assert!(saved.contains("summary: \"Implemented launch handoff.\""));
        assert!(saved.contains("key_changed_files: [\"src-tauri/src/lib.rs\"]"));
        assert!(saved.contains("decisions: [\"Persist distilled session fields when saving.\"]"));
        assert!(saved.contains("remaining_work: [\"launch saved entries.\"]"));
        assert!(saved.contains("launch_target: codex"));
        assert!(saved.contains("injection_method: agents-md-section-marker-merge"));
        assert!(!saved.contains("tags: [old]"));

        let reloaded = list_markdown_contexts(Some(ListContextFilesRequest {
            working_dir: Some(base.clone()),
        }))
        .expect("saved contexts should reload")
        .into_iter()
        .find(|reloaded| reloaded.file_path == context.file_path)
        .expect("saved session context should be listed");

        assert_eq!(
            reloaded.llm_classification_status,
            ctx_core::ClassificationStatus::Classified
        );
        assert!(reloaded
            .session_handoff_classification
            .as_ref()
            .is_some_and(|metadata| metadata.source_session_ref == "codex-save-metadata"));
        fs::remove_dir_all(base).ok();
    }

    #[test]
    fn save_agent_session_context_maps_distilled_content_into_handoff_fields() {
        let base = std::env::temp_dir().join(format!(
            "ctx-tauri-session-save-distilled-map-test-{}",
            Uuid::new_v4()
        ));
        fs::create_dir_all(&base).expect("test working directory should be created");
        let session_path = base.join("codex-save-distilled-map.jsonl");
        fs::write(
            &session_path,
            r#"{"timestamp":"2026-05-11T00:00:00Z","type":"session_meta","payload":{"id":"codex-save-distilled-map","cwd":"/tmp/project","timestamp":"2026-05-11T00:00:00Z"}}
{"timestamp":"2026-05-11T00:00:01Z","type":"event_msg","payload":{"type":"user_message","message":"Original request before refinement"}}
{"timestamp":"2026-05-11T00:00:02Z","type":"event_msg","payload":{"type":"agent_message","message":"Summary: Original raw scan summary.\nChanged files: crates/ctx-core/src/session_logs.rs.\nDecision: Original raw decision.\nVerified with original raw check.\nRemaining work: Original raw follow-up."}}
"#,
        )
        .expect("session log should be writable");

        let context = save_agent_session_context(SaveAgentSessionContextRequest {
            provider: "codex".to_string(),
            file_path: session_path,
            content: "# Previous Session Context\n\n## Handoff Summary\n\nDistilled save flow now persists the reviewed handoff body.\n\n### Goals\n\n- Save distilled scan output with classification metadata\n\n### Key changed files\n\n- crates/ctx-core/src/work_context.rs\n\n### Decisions\n\n- Use sectioned distilled content for reusable handoff fields.\n\n### Verification results\n\n- cargo test -p ctx-tauri save_agent_session_context_maps_distilled_content_into_handoff_fields\n\n### Remaining work\n\n- Launch saved entries with automatic injection."
                .to_string(),
            working_dir: Some(base.clone()),
        })
        .expect("session handoff should save from distilled content");

        let saved =
            fs::read_to_string(&context.file_path).expect("saved handoff should be readable");
        assert!(saved.contains("source_tool: codex"));
        assert!(saved.contains("source_session_ref: \"codex-save-distilled-map\""));
        assert!(saved
            .contains("summary: \"Distilled save flow now persists the reviewed handoff body.\""));
        assert!(
            saved.contains("goals: [\"Save distilled scan output with classification metadata\"]")
        );
        assert!(saved.contains("key_changed_files: [\"crates/ctx-core/src/work_context.rs\"]"));
        assert!(saved.contains(
            "decisions: [\"Use sectioned distilled content for reusable handoff fields.\"]"
        ));
        assert!(saved.contains("verification_results: [\"cargo test -p ctx-tauri save_agent_session_context_maps_distilled_content_into_handoff_fields\"]"));
        assert!(
            saved.contains("remaining_work: [\"Launch saved entries with automatic injection.\"]")
        );
        assert!(!saved.contains("Original raw scan summary."));
        assert!(!saved.contains("Original raw decision."));
        fs::remove_dir_all(base).ok();
    }

    #[test]
    fn save_agent_session_context_rejects_incomplete_distilled_context_before_persistence() {
        let base = std::env::temp_dir().join(format!(
            "ctx-tauri-session-save-validation-test-{}",
            Uuid::new_v4()
        ));
        fs::create_dir_all(&base).expect("test working directory should be created");
        let session_path = base.join("codex-save-validation.jsonl");
        fs::write(
            &session_path,
            r#"{"timestamp":"2026-05-11T00:00:00Z","type":"session_meta","payload":{"id":"codex-save-validation","cwd":"/tmp/project","timestamp":"2026-05-11T00:00:00Z"}}
{"timestamp":"2026-05-11T00:00:01Z","type":"event_msg","payload":{"type":"user_message","message":"Implement session handoff launch flow and verify cleanup"}}
{"timestamp":"2026-05-11T00:00:02Z","type":"event_msg","payload":{"type":"agent_message","message":"Summary: Implemented launch handoff.\nChanged files: src-tauri/src/lib.rs.\nDecision: Validate distilled handoff fields before saving.\nVerified with cargo test -p ctx-tauri save_agent_session_context_rejects_incomplete_distilled_context_before_persistence.\nRemaining work: launch saved entries."}}
"#,
        )
        .expect("session log should be writable");

        let error = save_agent_session_context(SaveAgentSessionContextRequest {
            provider: "codex".to_string(),
            file_path: session_path,
            content:
                "# Previous Session Context\n\n## Handoff Summary\n\nImplemented launch handoff."
                    .to_string(),
            working_dir: Some(base.clone()),
        })
        .expect_err("incomplete distilled handoff content should be rejected");

        assert!(error.contains("invalid distilled session handoff context"));
        assert!(error.contains("lost essential distilled field"));
        assert!(!base.join(".ctx").exists());
        fs::remove_dir_all(base).ok();
    }

    #[test]
    fn save_agent_session_context_validation_error_does_not_mutate_existing_handoff() {
        let base = std::env::temp_dir().join(format!(
            "ctx-tauri-session-save-validation-mutation-test-{}",
            Uuid::new_v4()
        ));
        fs::create_dir_all(&base).expect("test working directory should be created");
        let session_path = base.join("codex-save-validation-mutation.jsonl");
        fs::write(
            &session_path,
            r#"{"timestamp":"2026-05-11T00:00:00Z","type":"session_meta","payload":{"id":"codex-save-validation-mutation","cwd":"/tmp/project","timestamp":"2026-05-11T00:00:00Z"}}
{"timestamp":"2026-05-11T00:00:01Z","type":"event_msg","payload":{"type":"user_message","message":"Implement session handoff launch flow and verify cleanup"}}
{"timestamp":"2026-05-11T00:00:02Z","type":"event_msg","payload":{"type":"agent_message","message":"Summary: Implemented launch handoff.\nChanged files: src-tauri/src/lib.rs.\nDecision: Validate distilled handoff fields before saving.\nVerified with cargo test -p ctx-tauri save_agent_session_context_validation_error_does_not_mutate_existing_handoff.\nRemaining work: launch saved entries."}}
"#,
        )
        .expect("session log should be writable");

        let saved = save_agent_session_context(SaveAgentSessionContextRequest {
            provider: "codex".to_string(),
            file_path: session_path.clone(),
            content: "# Previous Session Context\n\n## Handoff Summary\n\nImplemented launch handoff.\n\n### Goals\n\n- Implement session handoff launch flow and verify cleanup\n\n### Key changed files\n\n- src-tauri/src/lib.rs\n\n### Decisions\n\n- Validate distilled handoff fields before saving.\n\n### Verification results\n\n- Verified with cargo test -p ctx-tauri save_agent_session_context_validation_error_does_not_mutate_existing_handoff.\n\n### Remaining work\n\n- launch saved entries."
                .to_string(),
            working_dir: Some(base.clone()),
        })
        .expect("complete session handoff should save");
        let saved_content_before =
            fs::read_to_string(&saved.file_path).expect("saved handoff should be readable");

        let error = save_agent_session_context(SaveAgentSessionContextRequest {
            provider: "codex".to_string(),
            file_path: session_path,
            content:
                "# Previous Session Context\n\n## Handoff Summary\n\nImplemented launch handoff."
                    .to_string(),
            working_dir: Some(base.clone()),
        })
        .expect_err("validation failure should be returned to the caller");

        let saved_content_after =
            fs::read_to_string(&saved.file_path).expect("saved handoff should remain readable");
        let listed = list_saved_session_contexts(Some(ListContextFilesRequest {
            working_dir: Some(base.clone()),
        }))
        .expect("saved context list should remain readable after validation failure");

        assert!(error.contains("invalid distilled session handoff context"));
        assert!(error.contains("lost essential distilled field"));
        assert_eq!(saved_content_after, saved_content_before);
        assert_eq!(
            listed
                .iter()
                .filter(|context| context.fragment.file_path == saved.file_path)
                .count(),
            1
        );
        fs::remove_dir_all(base).ok();
    }

    #[test]
    fn saved_session_context_detail_apis_return_full_handoff_schema_with_empty_values() {
        let base =
            std::env::temp_dir().join(format!("ctx-tauri-saved-detail-test-{}", Uuid::new_v4()));
        let home = base.join("home");
        let working_dir = base.join("project");
        let roots = VaultRoots {
            global_root: home
                .join(ctx_core::CTX_HOME_DIR)
                .join(ctx_core::GLOBAL_VAULT_DIR),
            local_root: Some(
                working_dir
                    .join(ctx_core::CTX_HOME_DIR)
                    .join(ctx_core::GLOBAL_VAULT_DIR),
            ),
        };
        let handoff = SessionHandoffContext {
            source_tool: SessionLogProvider::Claude,
            source_session_ref: "claude-empty-fields".to_string(),
            source_working_directory: "/tmp/session-project".to_string(),
            source_log_path: "/tmp/claude-empty-fields.jsonl".to_string(),
            source_updated_at: None,
            title: "Preserve empty distilled fields".to_string(),
            category: WorkContextCategory::General,
            categories: vec![WorkContextCategory::General],
            classification_status: ClassificationStatus::Classified,
            classification_confidence_score: 77,
            classification_rationale: "General session handoff signals were detected.".to_string(),
            goals: Vec::new(),
            summary: "Documented a minimal saved session handoff.".to_string(),
            key_changed_files: Vec::new(),
            commands: Vec::new(),
            decisions: Vec::new(),
            verification_results: Vec::new(),
            remaining_work: Vec::new(),
            created_at: "2026-05-11T00:00:00Z".to_string(),
            handoff_markdown: "# Previous Session Context\n\n## Summary\n\nDocumented a minimal saved session handoff.\n\n## Notes\n\n- No key files were captured.\n- No decisions were captured.\n- No verification commands were captured."
                .to_string(),
            tags: vec![
                "session-history".to_string(),
                "resume-context".to_string(),
                "claude".to_string(),
                "general".to_string(),
            ],
            cleanup_applied: false,
            refine_mode: WorkContextRefineMode::Refined,
            launch_target: CliTarget::Claude,
            injection_method: InjectionStrategy::AppendSystemPromptFile,
        };

        let saved = create_session_handoff_context_file(
            &roots,
            VaultScope::Local,
            "session-history",
            "claude-empty-fields.md",
            &handoff,
        )
        .expect("minimal session handoff should persist");

        with_home(&home, || {
            let listed = list_saved_session_contexts(Some(ListContextFilesRequest {
                working_dir: Some(working_dir.clone()),
            }))
            .expect("saved session list API should return structured handoff entries");
            let listed_entry = listed
                .iter()
                .find(|entry| entry.fragment.file_path == saved.fragment.file_path)
                .expect("saved session should be listed");

            assert_eq!(listed_entry.handoff, handoff);
            assert!(listed_entry.handoff.source_updated_at.is_none());
            assert!(listed_entry.handoff.key_changed_files.is_empty());
            assert!(listed_entry.handoff.commands.is_empty());
            assert!(listed_entry.handoff.decisions.is_empty());
            assert!(listed_entry.handoff.verification_results.is_empty());
            assert!(listed_entry.handoff.remaining_work.is_empty());
            assert_eq!(
                listed_entry.handoff.refine_mode,
                WorkContextRefineMode::Refined
            );
            assert_eq!(
                listed_entry.handoff.injection_method,
                InjectionStrategy::AppendSystemPromptFile
            );
            assert_eq!(
                listed_entry
                    .fragment
                    .session_handoff_classification
                    .as_ref()
                    .expect("fragment metadata should still be hydrated")
                    .source_session_ref,
                "claude-empty-fields"
            );

            let list_json = serde_json::to_value(&listed)
                .expect("saved session list response should serialize for the API");
            let listed_json_entry = list_json
                .as_array()
                .expect("saved session list response should be an array")
                .iter()
                .find(|entry| {
                    entry["fragment"]["file_path"]
                        == serde_json::json!(saved.fragment.file_path.to_string_lossy())
                })
                .expect("saved session should be present in serialized list response");

            assert_eq!(
                listed_json_entry["fragment"]["title"],
                "claude empty fields"
            );
            assert_eq!(
                listed_json_entry["fragment"]["vault_scope"],
                serde_json::json!("local")
            );
            assert_eq!(
                listed_json_entry["fragment"]["tags"],
                serde_json::json!(["session-history", "resume-context", "claude", "general"])
            );
            assert_eq!(
                listed_json_entry["fragment"]["llm_classification_status"],
                serde_json::json!("classified")
            );
            assert_eq!(
                listed_json_entry["fragment"]["session_handoff_classification"]["sourceSessionRef"],
                "claude-empty-fields"
            );
            assert_eq!(
                listed_json_entry["handoff"]["source_tool"],
                serde_json::json!("claude")
            );
            assert_eq!(
                listed_json_entry["handoff"]["source_session_ref"],
                "claude-empty-fields"
            );
            assert_eq!(
                listed_json_entry["handoff"]["source_working_directory"],
                "/tmp/session-project"
            );
            assert_eq!(
                listed_json_entry["handoff"]["source_log_path"],
                "/tmp/claude-empty-fields.jsonl"
            );
            assert!(listed_json_entry["handoff"]["source_updated_at"].is_null());
            assert_eq!(
                listed_json_entry["handoff"]["title"],
                "Preserve empty distilled fields"
            );
            assert_eq!(
                listed_json_entry["handoff"]["category"],
                serde_json::json!("general")
            );
            assert_eq!(
                listed_json_entry["handoff"]["categories"],
                serde_json::json!(["general"])
            );
            assert_eq!(
                listed_json_entry["handoff"]["classification_status"],
                serde_json::json!("classified")
            );
            assert_eq!(
                listed_json_entry["handoff"]["classification_confidence_score"],
                serde_json::json!(77)
            );
            assert_eq!(
                listed_json_entry["handoff"]["classification_rationale"],
                "General session handoff signals were detected."
            );
            assert_eq!(listed_json_entry["handoff"]["goals"], serde_json::json!([]));
            assert_eq!(
                listed_json_entry["handoff"]["summary"],
                "Documented a minimal saved session handoff."
            );
            assert_eq!(
                listed_json_entry["handoff"]["key_changed_files"],
                serde_json::json!([])
            );
            assert_eq!(
                listed_json_entry["handoff"]["commands"],
                serde_json::json!([])
            );
            assert_eq!(
                listed_json_entry["handoff"]["decisions"],
                serde_json::json!([])
            );
            assert_eq!(
                listed_json_entry["handoff"]["verification_results"],
                serde_json::json!([])
            );
            assert_eq!(
                listed_json_entry["handoff"]["remaining_work"],
                serde_json::json!([])
            );
            assert_eq!(
                listed_json_entry["handoff"]["created_at"],
                "2026-05-11T00:00:00Z"
            );
            assert_eq!(
                listed_json_entry["handoff"]["handoff_markdown"],
                "# Previous Session Context\n\n## Summary\n\nDocumented a minimal saved session handoff.\n\n## Notes\n\n- No key files were captured.\n- No decisions were captured.\n- No verification commands were captured."
            );
            assert_eq!(
                listed_json_entry["handoff"]["tags"],
                serde_json::json!(["session-history", "resume-context", "claude", "general"])
            );
            assert_eq!(
                listed_json_entry["handoff"]["cleanup_applied"],
                serde_json::json!(false)
            );
            assert_eq!(
                listed_json_entry["handoff"]["refine_mode"],
                serde_json::json!("refined")
            );
            assert_eq!(
                listed_json_entry["handoff"]["launch_target"],
                serde_json::json!("claude")
            );
            assert_eq!(
                listed_json_entry["handoff"]["injection_method"],
                serde_json::json!("append-system-prompt-file")
            );

            let detail = open_saved_session_context(OpenMarkdownContextRequest {
                file_path: saved.fragment.file_path.clone(),
                working_dir: Some(working_dir.clone()),
            })
            .expect("saved session detail API should return structured handoff entry");
            assert_eq!(detail.handoff, handoff);

            let json = serde_json::to_value(&detail)
                .expect("saved session detail response should serialize for the API");
            assert!(json["handoff"]["source_updated_at"].is_null());
            assert_eq!(json["handoff"]["key_changed_files"], serde_json::json!([]));
            assert_eq!(json["handoff"]["commands"], serde_json::json!([]));
            assert_eq!(json["handoff"]["decisions"], serde_json::json!([]));
            assert_eq!(
                json["handoff"]["verification_results"],
                serde_json::json!([])
            );
            assert_eq!(json["handoff"]["remaining_work"], serde_json::json!([]));
            assert_eq!(
                json["handoff"]["handoff_markdown"],
                "# Previous Session Context\n\n## Summary\n\nDocumented a minimal saved session handoff.\n\n## Notes\n\n- No key files were captured.\n- No decisions were captured.\n- No verification commands were captured."
            );
        });

        fs::remove_dir_all(base).ok();
    }

    #[test]
    fn review_import_classification_reports_validation_errors() {
        let base =
            std::env::temp_dir().join(format!("ctx-tauri-review-invalid-{}", Uuid::new_v4()));
        fs::create_dir_all(&base).expect("test working directory should be created");
        let created = create_markdown_context(CreateContextFileRequest {
            file_name: "manual.md".to_string(),
            content: Some("# Manual context".to_string()),
            folder_path: None,
            vault_scope: Some(VaultScope::Local),
            working_dir: Some(base.clone()),
        })
        .expect("manual context should be created");

        let error = review_import_classification(ReviewImportClassificationRequest {
            file_path: created.file_path,
            classification: ctx_core::Classification::Shared,
            working_dir: Some(base.clone()),
        })
        .expect_err("manual contexts without import metadata cannot be reviewed");

        assert!(error.starts_with("validation error:"));
        assert!(error.contains("no import metadata to review"));
        fs::remove_dir_all(base).ok();
    }

    #[test]
    fn delete_markdown_context_removes_selected_context_file() {
        let base = std::env::temp_dir().join(format!("ctx-tauri-delete-test-{}", Uuid::new_v4()));
        fs::create_dir_all(&base).expect("test working directory should be created");
        let file_path = base.join("agent.md");
        fs::write(&file_path, "# Agent").expect("selected context should be writable");

        let deleted_path = delete_markdown_context(DeleteMarkdownContextRequest {
            file_path: file_path.clone(),
            working_dir: Some(base.clone()),
        })
        .expect("selected markdown context should delete");

        assert_eq!(deleted_path, file_path);
        assert!(!deleted_path.exists());
        fs::remove_dir_all(base).ok();
    }

    #[test]
    fn delete_markdown_context_reports_invalid_paths() {
        let base =
            std::env::temp_dir().join(format!("ctx-tauri-delete-invalid-test-{}", Uuid::new_v4()));
        fs::create_dir_all(&base).expect("test working directory should be created");
        let file_path = base.join("outside.md");
        fs::write(&file_path, "# Outside").expect("outside markdown should be writable");

        let error = delete_markdown_context(DeleteMarkdownContextRequest {
            file_path: file_path.clone(),
            working_dir: Some(base.join("project")),
        })
        .expect_err("context outside resolved overlay should be rejected");

        assert!(error.contains("resolved vault overlay"));
        assert!(file_path.exists());
        fs::remove_dir_all(base).ok();
    }

    #[test]
    fn save_preset_execution_settings_persists_safe_working_dir_from_tauri() {
        let base =
            std::env::temp_dir().join(format!("ctx-tauri-preset-save-test-{}", Uuid::new_v4()));
        let workspace = base.join("workspace");
        fs::create_dir_all(&workspace).expect("workspace should be created");

        let summary = save_preset_execution_settings(
            PresetExecutionSettingsUpdate {
                preset_ref: "codex-default".to_string(),
                preset_name: Some("Codex Default".to_string()),
                vault_scope: VaultScope::Local,
                target_cli: CliTarget::Codex,
                working_dir: workspace.clone(),
                model: Some("codex".to_string()),
                passthrough_args: vec!["--sandbox".to_string(), "workspace-write".to_string()],
            },
            Some(base.clone()),
        )
        .expect("tauri command should persist preset execution settings");

        assert_eq!(summary.preset_name, "Codex Default");
        assert_eq!(summary.cli_execution_settings.working_dir, workspace);
        assert_eq!(
            fs::read_to_string(&summary.file_path).expect("saved preset should be readable"),
            fs::read_to_string(
                managed_presets_dir(&base.join(".ctx").join("vault")).join("codex-default.json")
            )
            .expect("local preset file should exist")
        );

        fs::remove_dir_all(base).ok();
    }

    #[test]
    fn save_preset_subagent_manifest_persists_schema_from_tauri() {
        let base = std::env::temp_dir().join(format!(
            "ctx-tauri-subagent-manifest-test-{}",
            Uuid::new_v4()
        ));
        let project_dir = base.join("project");
        let presets_dir = managed_presets_dir(&project_dir.join(".ctx").join("vault"));
        fs::create_dir_all(&presets_dir).expect("preset directory should be created");
        fs::write(
            presets_dir.join("delegated-review.json"),
            r#"{"preset_name":"Delegated Review","preset_target_cli":"codex"}"#,
        )
        .expect("preset should be writable");

        let summary = save_preset_subagent_manifest(
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
                        capabilities: vec![" correctness review ".to_string()],
                        constraints: vec![" Return findings first. ".to_string()],
                        metadata: BTreeMap::from([(
                            " owner ".to_string(),
                            " quality ".to_string(),
                        )]),
                        description: Some(" Find correctness risks. ".to_string()),
                        assigned_contexts: vec![" subagents/reviewer.md ".to_string()],
                        spawn_instructions: vec![" Review the active patch. ".to_string()],
                        spawn_guidance: Default::default(),
                        handoff_targets: vec![" implementer ".to_string()],
                        model: Some(" gpt-5.3-codex ".to_string()),
                    }],
                    handoff_constraints: HandoffConstraints {
                        require_summary: true,
                        require_changed_files: true,
                        require_open_questions: false,
                        max_parallel_subagents: Some(2),
                        allowed_handoff_targets: vec![" implementer ".to_string()],
                        blocked_handoff_targets: Vec::new(),
                        handoff_prompt_template: Some(" Return findings first. ".to_string()),
                    },
                }),
            },
            Some(project_dir.clone()),
        )
        .expect("tauri command should persist subagent manifest");

        let manifest = summary
            .subagent_manifest
            .expect("summary should include persisted manifest");
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
            vec!["Review the active patch."]
        );
        assert_eq!(manifest.roles[0].handoff_targets, vec!["implementer"]);
        assert_eq!(manifest.roles[0].model.as_deref(), Some("gpt-5.3-codex"));
        assert_eq!(
            manifest
                .handoff_constraints
                .handoff_prompt_template
                .as_deref(),
            Some("Return findings first.")
        );

        let persisted = fs::read_to_string(summary.file_path).expect("preset should be readable");
        assert!(persisted.contains("\"subagent_manifest\""));
        assert!(persisted.contains("\"manifest_version\": \"1\""));

        fs::remove_dir_all(base).ok();
    }

    #[test]
    fn save_preset_subagent_manifest_returns_clear_validation_errors_from_tauri() {
        let base = std::env::temp_dir().join(format!(
            "ctx-tauri-subagent-manifest-validation-test-{}",
            Uuid::new_v4()
        ));
        let project_dir = base.join("project");
        fs::create_dir_all(&project_dir).expect("project directory should be created");

        let error = save_preset_subagent_manifest(
            SubagentManifestUpdate {
                preset_ref: "bad-manifest".to_string(),
                preset_name: None,
                vault_scope: VaultScope::Local,
                manifest: Some(SubagentManifest {
                    manifest_version: Some("2".to_string()),
                    roles: vec![SubagentRole {
                        role_id: "bad role".to_string(),
                        role_name: " ".to_string(),
                        role: " ".to_string(),
                        capabilities: Vec::new(),
                        constraints: Vec::new(),
                        metadata: BTreeMap::new(),
                        description: None,
                        assigned_contexts: vec!["/tmp/secret.md".to_string()],
                        spawn_instructions: Vec::new(),
                        spawn_guidance: Default::default(),
                        handoff_targets: Vec::new(),
                        model: None,
                    }],
                    handoff_constraints: HandoffConstraints::default(),
                }),
            },
            Some(project_dir.clone()),
        )
        .expect_err("tauri command should surface manifest validation errors");

        assert!(error.starts_with("invalid subagent_manifest:"));
        assert!(error.contains("manifest_version must be \"1\""));
        assert!(error.contains("roles[0] (bad role).id may only contain"));
        assert!(error.contains("assigned_contexts contains unsafe context ref: /tmp/secret.md"));
        assert!(error.contains("spawn_instructions must include at least one instruction"));

        fs::remove_dir_all(base).ok();
    }

    #[test]
    fn scan_existing_markdown_contexts_discovers_project_agent_files() {
        let base = std::env::temp_dir().join(format!("ctx-tauri-scan-test-{}", Uuid::new_v4()));
        fs::create_dir_all(base.join("skills")).expect("skills directory should be created");
        fs::write(base.join("AGENTS.md"), "# Codex rules").expect("AGENTS.md should be writable");
        fs::write(base.join("skills").join("review.md"), "# Review skill")
            .expect("skill markdown should be writable");

        let contexts = scan_existing_markdown_contexts(Some(ListContextFilesRequest {
            working_dir: Some(base.clone()),
        }))
        .expect("scan should return discovered contexts");

        assert!(contexts
            .iter()
            .any(|context| context.file_path == base.join("AGENTS.md")
                && context.file_name == "AGENTS.md"
                && context.root_source == base
                && context.metadata.vault_scope == VaultScope::Local));
        assert!(contexts.iter().any(|context| context.file_path
            == base.join("skills").join("review.md")
            && context.file_name == "review.md"
            && context.root_source == base
            && context.metadata.tags.contains(&"skills".to_string())));
        fs::remove_dir_all(base).ok();
    }

    #[test]
    fn list_codex_sessions_enumerates_readable_logs_from_discovered_roots() {
        let base =
            std::env::temp_dir().join(format!("ctx-tauri-codex-session-scan-{}", Uuid::new_v4()));
        let home = base.join("home");
        let project = base.join("project");
        let default_log_dir = home
            .join(".codex")
            .join("sessions")
            .join("2026")
            .join("05")
            .join("11");
        let configured_log_dir = project.join("project-codex-sessions").join("nested");
        let local_vault = project.join(".ctx").join("vault");

        fs::create_dir_all(&default_log_dir).expect("default Codex log dir should be created");
        fs::create_dir_all(&configured_log_dir)
            .expect("configured Codex log dir should be created");
        fs::create_dir_all(&local_vault).expect("local vault should be created");
        fs::write(
            local_vault.join("settings.json"),
            r#"{"codex_session_roots":[{"path":"project-codex-sessions","scope":"local"}]}"#,
        )
        .expect("local settings should be writable");
        fs::write(
            default_log_dir.join("default.jsonl"),
            codex_session_log("default-session", "Default Codex work"),
        )
        .expect("default Codex session log should be writable");
        fs::write(
            configured_log_dir.join("configured.jsonl"),
            codex_session_log("configured-session", "Configured Codex work"),
        )
        .expect("configured Codex session log should be writable");
        fs::write(configured_log_dir.join("notes.txt"), "not a session")
            .expect("non-jsonl file should be writable");

        with_home(&home, || {
            let sessions = list_codex_sessions(&home, &project)
                .expect("Codex sessions should be listed from discovered roots");
            let session_ids = sessions
                .iter()
                .map(|session| session.session_id.as_str())
                .collect::<Vec<_>>();

            assert_eq!(sessions.len(), 2);
            assert!(session_ids.contains(&"default-session"));
            assert!(session_ids.contains(&"configured-session"));
            assert!(sessions.iter().all(|session| session.provider == "codex"));
        });

        fs::remove_dir_all(base).ok();
    }

    #[test]
    fn list_claude_sessions_uses_shared_scanner_metadata() {
        let base =
            std::env::temp_dir().join(format!("ctx-tauri-claude-session-scan-{}", Uuid::new_v4()));
        let home = base.join("home");
        let project = base.join("project");
        let default_log_dir = home.join(".claude").join("projects").join("project-a");
        let local_vault = project.join(".ctx").join("vault");
        let configured_log_dir = project.join("project-claude-sessions");

        fs::create_dir_all(&default_log_dir).expect("default Claude log dir should be created");
        fs::create_dir_all(&configured_log_dir)
            .expect("configured Claude log dir should be created");
        fs::create_dir_all(&local_vault).expect("local vault should be created");
        fs::write(
            local_vault.join("settings.json"),
            r#"{"claude_session_roots":[{"path":"project-claude-sessions","scope":"local"}]}"#,
        )
        .expect("local settings should be writable");
        fs::write(
            default_log_dir.join("default.jsonl"),
            r#"{"type":"system","sessionId":"default-claude","cwd":"/tmp/default","timestamp":"2026-05-11T00:00:00Z"}"#,
        )
        .expect("default Claude log should be writable");
        fs::write(
            configured_log_dir.join("configured.jsonl"),
            r#"{"sessionId":"configured-claude","cwd":"/tmp/configured","timestamp":"2026-05-11T00:00:01Z","type":"user","message":{"role":"user","content":"Configured Claude work"}}"#,
        )
        .expect("configured Claude log should be writable");

        with_home(&home, || {
            let sessions = list_claude_sessions(&home, &project)
                .expect("Claude sessions should be listed from shared scanner roots");
            let session_ids = sessions
                .iter()
                .map(|session| session.session_id.as_str())
                .collect::<Vec<_>>();

            assert_eq!(sessions.len(), 2);
            assert!(session_ids.contains(&"default-claude"));
            assert!(session_ids.contains(&"configured-claude"));
            assert!(sessions.iter().all(|session| session.provider == "claude"));
            assert!(sessions
                .iter()
                .any(|session| session.title == "Configured Claude work"
                    && session.cwd.as_deref() == Some("/tmp/configured")));
        });

        fs::remove_dir_all(base).ok();
    }

    #[test]
    fn import_markdown_contexts_materializes_discovered_files() {
        let base = std::env::temp_dir().join(format!("ctx-tauri-import-test-{}", Uuid::new_v4()));
        fs::create_dir_all(base.join("skills")).expect("skills directory should be created");
        fs::write(base.join("AGENTS.md"), "# Codex").expect("AGENTS.md should be writable");
        fs::write(base.join("skills").join("review.md"), "# Review")
            .expect("skill context should be writable");

        let imported = import_markdown_contexts(Some(ListContextFilesRequest {
            working_dir: Some(base.clone()),
        }))
        .expect("import should materialize discovered contexts");

        assert_eq!(imported.len(), 2);
        assert!(imported.iter().any(|context| {
            context.file_path
                == base
                    .join(".ctx")
                    .join("vault")
                    .join("contexts")
                    .join("AGENTS.md")
        }));
        assert!(imported.iter().any(|context| {
            context.file_path
                == base
                    .join(".ctx")
                    .join("vault")
                    .join("contexts")
                    .join("skills")
                    .join("review.md")
        }));

        fs::remove_dir_all(base).ok();
    }

    #[test]
    fn classify_import_markdown_returns_reviewable_suggestion() {
        let suggestion = classify_import_markdown(ClassifyImportMarkdownRequest {
            content: "# Reviewer\nThis subagent handles delegated handoff reviews.".to_string(),
            file_name: Some("reviewer.md".to_string()),
            folder_path: Some(PathBuf::from("docs")),
            import_source_type: None,
            target_cli: None,
            use_llm: None,
            timeout_ms: None,
            existing_tags: vec!["Review".to_string()],
        })
        .expect("classification command should return a suggestion");

        assert_eq!(
            suggestion.classification,
            ctx_core::Classification::Subagent
        );
        assert_eq!(
            suggestion.status,
            ctx_core::ClassificationStatus::Classified
        );
        assert!(suggestion.confidence_score >= 70);
        assert!(suggestion.rationale.contains("delegation"));
        assert!(suggestion.suggested_tags.contains(&"review".to_string()));
    }

    #[test]
    fn discover_markdown_contexts_combines_vault_and_existing_files() {
        let base = std::env::temp_dir().join(format!("ctx-tauri-discover-test-{}", Uuid::new_v4()));
        fs::create_dir_all(&base).expect("test working directory should be created");
        fs::write(base.join("agent.md"), "# Existing agent")
            .expect("existing agent should be writable");
        let created = create_markdown_context(CreateContextFileRequest {
            file_name: "shared.md".to_string(),
            content: Some("# Managed shared".to_string()),
            folder_path: None,
            vault_scope: Some(VaultScope::Local),
            working_dir: Some(base.clone()),
        })
        .expect("managed context should be created");

        let contexts = discover_markdown_contexts(Some(ListContextFilesRequest {
            working_dir: Some(base.clone()),
        }))
        .expect("combined discovery should return contexts");

        assert!(contexts.iter().any(
            |context| context.file_path == created.file_path && context.import_source.is_none()
        ));
        assert!(contexts
            .iter()
            .any(|context| context.file_path == base.join("agent.md")
                && context.import_source.is_some()));
        fs::remove_dir_all(base).ok();
    }

    #[test]
    fn open_markdown_context_rejects_shadowed_global_overlay_file() {
        let base =
            std::env::temp_dir().join(format!("ctx-tauri-shadowed-open-test-{}", Uuid::new_v4()));
        let roots = ctx_core::VaultRoots {
            global_root: base.join("global"),
            local_root: Some(base.join("project").join(".ctx").join("vault")),
        };
        let global = ctx_core::create_context_file(
            &roots,
            VaultScope::Global,
            "agents",
            "rules.md",
            "# Global Rules",
        )
        .expect("global context should be created");
        ctx_core::create_context_file(
            &roots,
            VaultScope::Local,
            "agents",
            "rules.md",
            "# Local Rules",
        )
        .expect("local context should override global context");

        let error = open_markdown_context(OpenMarkdownContextRequest {
            file_path: global.file_path,
            working_dir: Some(base.join("project")),
        })
        .expect_err("shadowed global context should not be readable through app overlay");

        assert!(error.contains("resolved vault overlay"));
        fs::remove_dir_all(base).ok();
    }

    #[test]
    fn app_reads_prefer_local_context_over_global_context_with_same_overlay_key() {
        let base =
            std::env::temp_dir().join(format!("ctx-tauri-overlay-read-test-{}", Uuid::new_v4()));
        let home = base.join("home");
        let project = base.join("project");
        fs::create_dir_all(&project).expect("project directory should be created");

        with_home(&home, || {
            let roots = ctx_core::VaultRoots::discover(&project);
            let global = ctx_core::create_context_file(
                &roots,
                VaultScope::Global,
                "agents",
                "rules.md",
                "# Global App Rules",
            )
            .expect("global context should be created");
            let local = ctx_core::create_context_file(
                &roots,
                VaultScope::Local,
                "agents",
                "rules.md",
                "# Local App Rules",
            )
            .expect("local context should override global context");

            let contexts = list_markdown_contexts(Some(ListContextFilesRequest {
                working_dir: Some(project.clone()),
            }))
            .expect("app list should resolve vault overlay");
            let content = open_markdown_context(OpenMarkdownContextRequest {
                file_path: local.file_path.clone(),
                working_dir: Some(project.clone()),
            })
            .expect("app open should read the resolved local context");
            let global_error = open_markdown_context(OpenMarkdownContextRequest {
                file_path: global.file_path.clone(),
                working_dir: Some(project.clone()),
            })
            .expect_err("app open should reject shadowed global context path");

            assert_eq!(contexts.len(), 1);
            assert_eq!(contexts[0].vault_scope, VaultScope::Local);
            assert_eq!(contexts[0].file_path, local.file_path);
            assert_eq!(contexts[0].content, "# Local App Rules");
            assert_eq!(content.content, "# Local App Rules");
            assert!(global_error.contains("resolved vault overlay"));
        });

        fs::remove_dir_all(base).ok();
    }

    #[test]
    fn app_reads_fall_back_to_global_context_when_local_overlay_has_no_match() {
        let base =
            std::env::temp_dir().join(format!("ctx-tauri-global-fallback-test-{}", Uuid::new_v4()));
        let home = base.join("home");
        let project = base.join("project");
        fs::create_dir_all(&project).expect("project directory should be created");

        with_home(&home, || {
            let roots = ctx_core::VaultRoots::discover(&project);
            let global = ctx_core::create_context_file(
                &roots,
                VaultScope::Global,
                "shared",
                "fallback.md",
                "# Global App Fallback",
            )
            .expect("global context should be created");

            let contexts = list_markdown_contexts(Some(ListContextFilesRequest {
                working_dir: Some(project.clone()),
            }))
            .expect("app list should include global fallback context");
            let content = open_markdown_context(OpenMarkdownContextRequest {
                file_path: global.file_path.clone(),
                working_dir: Some(project.clone()),
            })
            .expect("app open should read global context when no local override exists");

            assert_eq!(contexts.len(), 1);
            assert_eq!(contexts[0].vault_scope, VaultScope::Global);
            assert_eq!(contexts[0].file_path, global.file_path);
            assert_eq!(content.content, "# Global App Fallback");
        });

        fs::remove_dir_all(base).ok();
    }

    #[test]
    fn delete_markdown_context_rejects_shadowed_global_overlay_file() {
        let base =
            std::env::temp_dir().join(format!("ctx-tauri-shadowed-delete-test-{}", Uuid::new_v4()));
        let roots = ctx_core::VaultRoots {
            global_root: base.join("global"),
            local_root: Some(base.join("project").join(".ctx").join("vault")),
        };
        let global = ctx_core::create_context_file(
            &roots,
            VaultScope::Global,
            "agents",
            "rules.md",
            "# Global Rules",
        )
        .expect("global context should be created");
        ctx_core::create_context_file(
            &roots,
            VaultScope::Local,
            "agents",
            "rules.md",
            "# Local Rules",
        )
        .expect("local context should override global context");

        let error = delete_markdown_context(DeleteMarkdownContextRequest {
            file_path: global.file_path.clone(),
            working_dir: Some(base.join("project")),
        })
        .expect_err("shadowed global context should not be deletable through app overlay");

        assert!(error.contains("resolved vault overlay"));
        assert!(global.file_path.exists());
        fs::remove_dir_all(base).ok();
    }

    #[test]
    fn delete_markdown_context_reports_missing_file() {
        let base =
            std::env::temp_dir().join(format!("ctx-tauri-delete-missing-test-{}", Uuid::new_v4()));
        fs::create_dir_all(&base).expect("test working directory should be created");

        let error = delete_markdown_context(DeleteMarkdownContextRequest {
            file_path: base.join("agent.md"),
            working_dir: Some(base.clone()),
        })
        .expect_err("missing markdown context should report a useful error");

        assert!(error.contains("resolved vault overlay"));
        fs::remove_dir_all(base).ok();
    }

    #[test]
    fn list_presets_reads_local_overlay_preset_and_execution_settings() {
        let base = std::env::temp_dir().join(format!("ctx-tauri-presets-test-{}", Uuid::new_v4()));
        let project_dir = base.join("project");
        let roots = ctx_core::VaultRoots {
            global_root: base.join("global"),
            local_root: Some(project_dir.join(".ctx").join("vault")),
        };
        ctx_core::create_context_file(&roots, VaultScope::Global, "", "shared.md", "# Global")
            .expect("global context should be created");
        ctx_core::create_context_file(&roots, VaultScope::Local, "", "shared.md", "# Local")
            .expect("local context should override");

        let global_presets = managed_presets_dir(&roots.global_root);
        let local_presets = managed_presets_dir(roots.local_root.as_ref().unwrap());
        fs::create_dir_all(&global_presets).expect("global preset directory should be created");
        fs::create_dir_all(&local_presets).expect("local preset directory should be created");
        fs::write(
            global_presets.join("daily.json"),
            r#"{"preset_name":"Global Daily","preset_target_cli":"codex","preset_contexts":["shared.md"]}"#,
        )
        .expect("global preset should be writable");
        fs::write(
            local_presets.join("daily.json"),
            format!(
                r#"{{
                    "preset_name": "Local Daily",
                    "preset_contexts": ["shared.md"],
                    "cli_execution_settings": {{
                        "target_cli": "claude",
                        "working_dir": "{}",
                        "model": "claude-sonnet",
                        "passthrough_args": ["--debug"]
                    }}
                }}"#,
                project_dir.display()
            ),
        )
        .expect("local preset should be writable");

        let presets = list_presets(Some(ListContextFilesRequest {
            working_dir: Some(project_dir.clone()),
        }))
        .expect("presets should be listed through resolved overlay");

        assert_eq!(presets.len(), 1);
        assert_eq!(presets[0].preset_name, "Local Daily");
        assert_eq!(presets[0].preset_target_cli, CliTarget::Claude);
        assert_eq!(presets[0].preset_model.as_deref(), Some("claude-sonnet"));
        assert_eq!(
            presets[0].cli_execution_settings.target_cli,
            CliTarget::Claude
        );
        assert_eq!(presets[0].cli_execution_settings.working_dir, project_dir);
        assert_eq!(
            presets[0].cli_execution_settings.model.as_deref(),
            Some("claude-sonnet")
        );
        assert_eq!(
            presets[0].cli_execution_settings.passthrough_args,
            vec!["--debug"]
        );
        assert_eq!(presets[0].preset_context_count, 1);
        assert_eq!(presets[0].vault_scope, VaultScope::Local);
        fs::remove_dir_all(base).ok();
    }

    fn with_home(home: &Path, test: impl FnOnce()) {
        let _guard = HOME_ENV_LOCK
            .lock()
            .expect("HOME env lock should not be poisoned");
        let previous_home = env::var_os("HOME");
        env::set_var("HOME", home);
        test();
        match previous_home {
            Some(value) => env::set_var("HOME", value),
            None => env::remove_var("HOME"),
        }
    }

    fn codex_session_log(session_id: &str, user_message: &str) -> String {
        format!(
            r#"{{"timestamp":"2026-05-11T00:00:00Z","type":"session_meta","payload":{{"id":"{session_id}","cwd":"/tmp/project","timestamp":"2026-05-11T00:00:00Z"}}}}
{{"timestamp":"2026-05-11T00:00:01Z","type":"event_msg","payload":{{"type":"user_message","message":"{user_message}"}}}}
"#
        )
    }
}
