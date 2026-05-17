use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::{HashMap, HashSet},
    fmt, fs,
    path::{Path, PathBuf},
    time::SystemTime,
};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Eq, PartialEq, Hash, Ord, PartialOrd)]
#[serde(rename_all = "lowercase")]
pub enum SessionLogProvider {
    Claude,
    Codex,
}

impl SessionLogProvider {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
        }
    }
}

impl fmt::Display for SessionLogProvider {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SessionLogMetadata {
    pub provider: String,
    pub session_id: String,
    pub title: String,
    pub updated_at: Option<String>,
    pub cwd: Option<String>,
    pub file_path: PathBuf,
    pub message_count: usize,
    pub last_user_message: Option<String>,
}

impl SessionLogMetadata {
    pub fn provider_kind(&self) -> Option<SessionLogProvider> {
        match self.provider.as_str() {
            "claude" => Some(SessionLogProvider::Claude),
            "codex" => Some(SessionLogProvider::Codex),
            _ => None,
        }
    }

    pub fn source_session_ref(&self) -> &str {
        &self.session_id
    }

    pub fn source_working_directory(&self) -> Option<&str> {
        self.cwd.as_deref()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SessionLogMessage {
    pub role: String,
    pub timestamp: Option<String>,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SessionLogEventRecord {
    pub provider: String,
    pub line_number: usize,
    pub timestamp: Option<String>,
    pub record_type: String,
    #[serde(default)]
    pub event_type: Option<String>,
    #[serde(default)]
    pub role: Option<String>,
    #[serde(default)]
    pub content: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SessionLogDetail {
    pub summary: SessionLogMetadata,
    pub messages: Vec<SessionLogMessage>,
    #[serde(default)]
    pub events: Vec<SessionLogEventRecord>,
    pub distilled_markdown: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SessionLogScanRequest {
    pub provider: SessionLogProvider,
    pub home_dir: PathBuf,
    pub working_dir: PathBuf,
    pub root_paths: Vec<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SessionLogScanResult {
    pub provider: SessionLogProvider,
    pub sessions: Vec<SessionLogMetadata>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum SessionLogScanErrorKind {
    RootResolution,
    Io,
    Parse,
    UnsupportedProvider,
}

#[derive(Debug, Clone, Serialize, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SessionLogScanError {
    pub provider: Option<SessionLogProvider>,
    pub kind: SessionLogScanErrorKind,
    pub message: String,
}

impl SessionLogScanError {
    pub fn new(
        provider: Option<SessionLogProvider>,
        kind: SessionLogScanErrorKind,
        message: impl Into<String>,
    ) -> Self {
        Self {
            provider,
            kind,
            message: message.into(),
        }
    }
}

impl fmt::Display for SessionLogScanError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(provider) = self.provider {
            write!(
                formatter,
                "{provider} session log scan failed: {}",
                self.message
            )
        } else {
            write!(formatter, "session log scan failed: {}", self.message)
        }
    }
}

impl std::error::Error for SessionLogScanError {}

pub trait SessionLogScanner {
    fn provider(&self) -> SessionLogProvider;

    fn scan_session_logs(
        &self,
        request: &SessionLogScanRequest,
    ) -> Result<SessionLogScanResult, SessionLogScanError>;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct ClaudeSessionLogScanner;

#[derive(Debug, Clone, Copy, Default)]
pub struct CodexSessionLogScanner;

impl SessionLogScanner for ClaudeSessionLogScanner {
    fn provider(&self) -> SessionLogProvider {
        SessionLogProvider::Claude
    }

    fn scan_session_logs(
        &self,
        request: &SessionLogScanRequest,
    ) -> Result<SessionLogScanResult, SessionLogScanError> {
        if request.provider != SessionLogProvider::Claude {
            return Err(SessionLogScanError::new(
                Some(request.provider),
                SessionLogScanErrorKind::UnsupportedProvider,
                format!("Claude scanner cannot scan {} logs", request.provider),
            ));
        }

        let sessions = scan_claude_session_log_metadata(&request.root_paths)?;
        Ok(SessionLogScanResult {
            provider: SessionLogProvider::Claude,
            sessions,
        })
    }
}

impl SessionLogScanner for CodexSessionLogScanner {
    fn provider(&self) -> SessionLogProvider {
        SessionLogProvider::Codex
    }

    fn scan_session_logs(
        &self,
        request: &SessionLogScanRequest,
    ) -> Result<SessionLogScanResult, SessionLogScanError> {
        if request.provider != SessionLogProvider::Codex {
            return Err(SessionLogScanError::new(
                Some(request.provider),
                SessionLogScanErrorKind::UnsupportedProvider,
                format!("Codex scanner cannot scan {} logs", request.provider),
            ));
        }

        let index_path = request.home_dir.join(".codex").join("session_index.jsonl");
        let index = read_codex_session_index(&index_path)?;
        let sessions = scan_codex_session_log_metadata(&request.root_paths, &index);

        Ok(SessionLogScanResult {
            provider: SessionLogProvider::Codex,
            sessions,
        })
    }
}

pub fn scan_claude_session_log_metadata(
    root_paths: &[PathBuf],
) -> Result<Vec<SessionLogMetadata>, SessionLogScanError> {
    let mut sessions = Vec::new();
    for file in enumerate_claude_session_log_paths(root_paths)? {
        if let Ok(metadata) = parse_claude_session_log_metadata(&file) {
            sessions.push(metadata);
        }
    }

    sessions.sort_by(|left, right| right.updated_at.cmp(&left.updated_at));
    Ok(sessions)
}

pub fn enumerate_claude_session_log_paths(
    root_paths: &[PathBuf],
) -> Result<Vec<PathBuf>, SessionLogScanError> {
    let mut files = Vec::new();
    let mut seen_files = HashSet::new();

    for root_path in root_paths {
        collect_readable_jsonl_files(root_path, &mut seen_files, &mut files).map_err(|error| {
            SessionLogScanError::new(
                Some(SessionLogProvider::Claude),
                SessionLogScanErrorKind::Io,
                format!(
                    "failed to enumerate Claude session logs under {}: {error}",
                    root_path.display()
                ),
            )
        })?;
    }

    files.retain(|file| !is_subagent_log_path(file));
    files.sort();
    Ok(files)
}

pub fn scan_codex_session_log_metadata(
    root_paths: &[PathBuf],
    index: &HashMap<String, CodexSessionIndexEntry>,
) -> Vec<SessionLogMetadata> {
    let mut files = Vec::new();
    let mut seen_files = HashSet::new();

    for root_path in root_paths {
        collect_readable_jsonl_files_lossy(root_path, &mut seen_files, &mut files);
    }

    let mut sessions = Vec::new();
    for file in files {
        if let Ok(metadata) = parse_codex_session_log_metadata(&file, index) {
            sessions.push(metadata);
        }
    }

    sessions.sort_by(|left, right| right.updated_at.cmp(&left.updated_at));
    sessions
}

pub fn parse_claude_session_log_metadata(
    file_path: &Path,
) -> Result<SessionLogMetadata, SessionLogScanError> {
    let content = fs::read_to_string(file_path).map_err(|error| {
        SessionLogScanError::new(
            Some(SessionLogProvider::Claude),
            SessionLogScanErrorKind::Io,
            format!(
                "failed to read Claude session log {}: {error}",
                file_path.display()
            ),
        )
    })?;
    let mut session_id = file_stem_session_id(file_path);
    let mut cwd = None;
    let mut updated_at = None;
    let mut message_count = 0;
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
                    .unwrap_or(record_type);
                let text = text_from_json_content(message.get("content").unwrap_or(&Value::Null));
                if !text.trim().is_empty() {
                    message_count += 1;
                    if role == "user" {
                        last_user_message = Some(truncate_text(&text, 220));
                    }
                }
            }
            "queue-operation" => {
                if let Some(message) = value.get("content").and_then(Value::as_str) {
                    let text = truncate_text(message, 220);
                    if !text.trim().is_empty() {
                        message_count += 1;
                        last_user_message = Some(text);
                    }
                }
            }
            _ => {}
        }
    }

    let title = last_user_message
        .clone()
        .unwrap_or_else(|| "Claude session".to_string());

    Ok(SessionLogMetadata {
        provider: SessionLogProvider::Claude.as_str().to_string(),
        session_id,
        title: truncate_text(&title, 120),
        updated_at: updated_at.or_else(|| modified_time_string(file_path)),
        cwd,
        file_path: file_path.to_path_buf(),
        message_count,
        last_user_message,
    })
}

#[derive(Debug, Clone, Default, Eq, PartialEq)]
pub struct CodexSessionIndexEntry {
    pub title: String,
    pub updated_at: String,
}

pub fn read_codex_session_index(
    index_path: &Path,
) -> Result<HashMap<String, CodexSessionIndexEntry>, SessionLogScanError> {
    let mut index = HashMap::new();
    if !index_path.exists() {
        return Ok(index);
    }

    let content = fs::read_to_string(index_path).map_err(|error| {
        SessionLogScanError::new(
            Some(SessionLogProvider::Codex),
            SessionLogScanErrorKind::Io,
            format!(
                "failed to read Codex session index {}: {error}",
                index_path.display()
            ),
        )
    })?;

    for line in content.lines() {
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        let Some(id) = value.get("id").and_then(Value::as_str) else {
            continue;
        };
        let title = value
            .get("thread_name")
            .and_then(Value::as_str)
            .unwrap_or("Codex session")
            .to_string();
        let updated_at = value
            .get("updated_at")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        index.insert(id.to_string(), CodexSessionIndexEntry { title, updated_at });
    }

    Ok(index)
}

pub fn parse_codex_session_log_metadata(
    file_path: &Path,
    index: &HashMap<String, CodexSessionIndexEntry>,
) -> Result<SessionLogMetadata, SessionLogScanError> {
    let content = fs::read_to_string(file_path).map_err(|error| {
        SessionLogScanError::new(
            Some(SessionLogProvider::Codex),
            SessionLogScanErrorKind::Io,
            format!(
                "failed to read Codex session log {}: {error}",
                file_path.display()
            ),
        )
    })?;
    let mut session_id = codex_file_stem_session_id(file_path);
    let mut cwd = None;
    let mut created_at = None;
    let mut message_count = 0;
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
            if cwd.is_none() {
                cwd = payload
                    .get("cwd")
                    .and_then(Value::as_str)
                    .map(ToString::to_string);
            }
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
                        let text = truncate_text(message, 220);
                        if !text.trim().is_empty() {
                            message_count += 1;
                            last_user_message = Some(text);
                        }
                    }
                }
                "agent_message" => {
                    if payload
                        .get("message")
                        .and_then(Value::as_str)
                        .is_some_and(|message| !message.trim().is_empty())
                    {
                        message_count += 1;
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
                .unwrap_or("assistant");
            let text = text_from_json_content(payload.get("content").unwrap_or(&Value::Null));
            if !text.trim().is_empty() {
                message_count += 1;
                if role == "user" {
                    last_user_message = Some(truncate_text(&text, 220));
                }
            }
        }
    }

    let indexed = index.get(&session_id);
    let indexed_title = indexed
        .map(|entry| entry.title.as_str())
        .unwrap_or_default();
    let indexed_updated_at = indexed
        .map(|entry| entry.updated_at.as_str())
        .unwrap_or_default();
    let title = if indexed_title.trim().is_empty() {
        last_user_message
            .clone()
            .unwrap_or_else(|| "Codex session".to_string())
    } else {
        indexed_title.to_string()
    };
    let updated_at = if indexed_updated_at.trim().is_empty() {
        created_at.or_else(|| modified_time_string(file_path))
    } else {
        Some(indexed_updated_at.to_string())
    };

    Ok(SessionLogMetadata {
        provider: SessionLogProvider::Codex.as_str().to_string(),
        session_id,
        title: truncate_text(&title, 120),
        updated_at,
        cwd,
        file_path: file_path.to_path_buf(),
        message_count,
        last_user_message,
    })
}

pub fn parse_claude_session_log_detail(
    file_path: &Path,
) -> Result<SessionLogDetail, SessionLogScanError> {
    let content = fs::read_to_string(file_path).map_err(|error| {
        SessionLogScanError::new(
            Some(SessionLogProvider::Claude),
            SessionLogScanErrorKind::Io,
            format!(
                "failed to read Claude session log {}: {error}",
                file_path.display()
            ),
        )
    })?;
    let mut session_id = file_stem_session_id(file_path);
    let mut cwd = None;
    let mut updated_at = None;
    let mut messages = Vec::new();
    let mut events = Vec::new();
    let mut last_user_message = None;

    for (line_index, line) in content.lines().enumerate() {
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
                let content = (!text.trim().is_empty()).then(|| truncate_text(&text, 12_000));
                events.push(SessionLogEventRecord {
                    provider: SessionLogProvider::Claude.as_str().to_string(),
                    line_number: line_index + 1,
                    timestamp: timestamp.clone(),
                    record_type: record_type.to_string(),
                    event_type: Some(record_type.to_string()),
                    role: Some(role.clone()),
                    content: content.clone(),
                });
                if let Some(text) = content {
                    if role == "user" {
                        last_user_message = Some(truncate_text(&text, 220));
                    }
                    messages.push(SessionLogMessage {
                        role,
                        timestamp,
                        content: text,
                    });
                }
            }
            "queue-operation" => {
                let content = value
                    .get("content")
                    .and_then(Value::as_str)
                    .filter(|message| !message.trim().is_empty())
                    .map(|message| truncate_text(message, 12_000));
                events.push(SessionLogEventRecord {
                    provider: SessionLogProvider::Claude.as_str().to_string(),
                    line_number: line_index + 1,
                    timestamp: timestamp.clone(),
                    record_type: record_type.to_string(),
                    event_type: Some(record_type.to_string()),
                    role: Some("user".to_string()),
                    content: content.clone(),
                });
                if let Some(text) = content {
                    last_user_message = Some(truncate_text(&text, 220));
                    messages.push(SessionLogMessage {
                        role: "user".to_string(),
                        timestamp,
                        content: text,
                    });
                }
            }
            _ => {
                events.push(SessionLogEventRecord {
                    provider: SessionLogProvider::Claude.as_str().to_string(),
                    line_number: line_index + 1,
                    timestamp,
                    record_type: record_type.to_string(),
                    event_type: Some(record_type.to_string()).filter(|event| !event.is_empty()),
                    role: None,
                    content: value
                        .get("summary")
                        .and_then(Value::as_str)
                        .filter(|summary| !summary.trim().is_empty())
                        .map(|summary| truncate_text(summary, 12_000)),
                });
            }
        }
    }

    let title = last_user_message
        .clone()
        .unwrap_or_else(|| "Claude session".to_string());
    let summary = SessionLogMetadata {
        provider: SessionLogProvider::Claude.as_str().to_string(),
        session_id,
        title: truncate_text(&title, 120),
        updated_at: updated_at.or_else(|| modified_time_string(file_path)),
        cwd,
        file_path: file_path.to_path_buf(),
        message_count: messages.len(),
        last_user_message,
    };
    let distilled_markdown = distilled_session_markdown(&summary, &messages);

    Ok(SessionLogDetail {
        summary,
        messages,
        events,
        distilled_markdown,
    })
}

pub fn parse_codex_session_log_detail(
    file_path: &Path,
    index: Option<&HashMap<String, CodexSessionIndexEntry>>,
) -> Result<SessionLogDetail, SessionLogScanError> {
    let content = fs::read_to_string(file_path).map_err(|error| {
        SessionLogScanError::new(
            Some(SessionLogProvider::Codex),
            SessionLogScanErrorKind::Io,
            format!(
                "failed to read Codex session log {}: {error}",
                file_path.display()
            ),
        )
    })?;
    let mut session_id = codex_file_stem_session_id(file_path);
    let mut cwd = None;
    let mut created_at = None;
    let mut messages = Vec::new();
    let mut events = Vec::new();
    let mut last_user_message = None;

    for (line_index, line) in content.lines().enumerate() {
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
        let payload_type = payload
            .get("type")
            .and_then(Value::as_str)
            .map(ToString::to_string);

        if record_type == "session_meta" {
            if let Some(id) = payload.get("id").and_then(Value::as_str) {
                session_id = id.to_string();
            }
            if cwd.is_none() {
                cwd = payload
                    .get("cwd")
                    .and_then(Value::as_str)
                    .map(ToString::to_string);
            }
            created_at = payload
                .get("timestamp")
                .and_then(Value::as_str)
                .map(ToString::to_string)
                .or(timestamp);
            events.push(SessionLogEventRecord {
                provider: SessionLogProvider::Codex.as_str().to_string(),
                line_number: line_index + 1,
                timestamp: created_at.clone(),
                record_type: record_type.to_string(),
                event_type: payload_type,
                role: None,
                content: None,
            });
            continue;
        }

        if record_type == "event_msg" {
            match payload_type.as_deref().unwrap_or_default() {
                "user_message" => {
                    let content = payload
                        .get("message")
                        .and_then(Value::as_str)
                        .filter(|message| !message.trim().is_empty())
                        .map(|message| truncate_text(message, 12_000));
                    events.push(SessionLogEventRecord {
                        provider: SessionLogProvider::Codex.as_str().to_string(),
                        line_number: line_index + 1,
                        timestamp: timestamp.clone(),
                        record_type: record_type.to_string(),
                        event_type: payload_type,
                        role: Some("user".to_string()),
                        content: content.clone(),
                    });
                    if let Some(text) = content {
                        last_user_message = Some(truncate_text(&text, 220));
                        messages.push(SessionLogMessage {
                            role: "user".to_string(),
                            timestamp,
                            content: text,
                        });
                    }
                }
                "agent_message" => {
                    let content = payload
                        .get("message")
                        .and_then(Value::as_str)
                        .filter(|message| !message.trim().is_empty())
                        .map(|message| truncate_text(message, 12_000));
                    events.push(SessionLogEventRecord {
                        provider: SessionLogProvider::Codex.as_str().to_string(),
                        line_number: line_index + 1,
                        timestamp: timestamp.clone(),
                        record_type: record_type.to_string(),
                        event_type: payload_type,
                        role: Some("assistant".to_string()),
                        content: content.clone(),
                    });
                    if let Some(text) = content {
                        messages.push(SessionLogMessage {
                            role: "assistant".to_string(),
                            timestamp,
                            content: text,
                        });
                    }
                }
                _ => {
                    events.push(SessionLogEventRecord {
                        provider: SessionLogProvider::Codex.as_str().to_string(),
                        line_number: line_index + 1,
                        timestamp,
                        record_type: record_type.to_string(),
                        event_type: payload_type,
                        role: None,
                        content: payload
                            .get("message")
                            .and_then(Value::as_str)
                            .filter(|message| !message.trim().is_empty())
                            .map(|message| truncate_text(message, 12_000)),
                    });
                }
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
            let content = (!text.trim().is_empty()).then(|| truncate_text(&text, 12_000));
            events.push(SessionLogEventRecord {
                provider: SessionLogProvider::Codex.as_str().to_string(),
                line_number: line_index + 1,
                timestamp: timestamp.clone(),
                record_type: record_type.to_string(),
                event_type: payload_type,
                role: Some(role.clone()),
                content: content.clone(),
            });
            if !text.trim().is_empty() {
                let text = content.expect("non-empty content should be present");
                if role == "user" {
                    last_user_message = Some(truncate_text(&text, 220));
                }
                messages.push(SessionLogMessage {
                    role,
                    timestamp,
                    content: text,
                });
            }
            continue;
        }

        events.push(SessionLogEventRecord {
            provider: SessionLogProvider::Codex.as_str().to_string(),
            line_number: line_index + 1,
            timestamp,
            record_type: record_type.to_string(),
            event_type: payload_type,
            role: payload
                .get("role")
                .and_then(Value::as_str)
                .map(ToString::to_string),
            content: payload
                .get("message")
                .and_then(Value::as_str)
                .filter(|message| !message.trim().is_empty())
                .map(|message| truncate_text(message, 12_000)),
        });
    }

    let indexed = index.and_then(|entries| entries.get(&session_id));
    let indexed_title = indexed
        .map(|entry| entry.title.as_str())
        .unwrap_or_default();
    let indexed_updated_at = indexed
        .map(|entry| entry.updated_at.as_str())
        .unwrap_or_default();
    let title = if indexed_title.trim().is_empty() {
        last_user_message
            .clone()
            .unwrap_or_else(|| "Codex session".to_string())
    } else {
        indexed_title.to_string()
    };
    let updated_at = if indexed_updated_at.trim().is_empty() {
        created_at.or_else(|| modified_time_string(file_path))
    } else {
        Some(indexed_updated_at.to_string())
    };
    let summary = SessionLogMetadata {
        provider: SessionLogProvider::Codex.as_str().to_string(),
        session_id,
        title: truncate_text(&title, 120),
        updated_at,
        cwd,
        file_path: file_path.to_path_buf(),
        message_count: messages.len(),
        last_user_message,
    };
    let distilled_markdown = distilled_session_markdown(&summary, &messages);

    Ok(SessionLogDetail {
        summary,
        messages,
        events,
        distilled_markdown,
    })
}

fn collect_readable_jsonl_files(
    root: &Path,
    seen_files: &mut HashSet<PathBuf>,
    files: &mut Vec<PathBuf>,
) -> std::io::Result<()> {
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            collect_readable_jsonl_files(&path, seen_files, files)?;
        } else if is_readable_jsonl_file(&path) {
            let canonical_path = path.canonicalize()?;
            if seen_files.insert(canonical_path.clone()) {
                files.push(canonical_path);
            }
        }
    }

    Ok(())
}

fn collect_readable_jsonl_files_lossy(
    root: &Path,
    seen_files: &mut HashSet<PathBuf>,
    files: &mut Vec<PathBuf>,
) {
    let Ok(entries) = fs::read_dir(root) else {
        return;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_readable_jsonl_files_lossy(&path, seen_files, files);
        } else if is_readable_jsonl_file(&path) {
            let Ok(canonical_path) = path.canonicalize() else {
                continue;
            };
            if seen_files.insert(canonical_path.clone()) {
                files.push(canonical_path);
            }
        }
    }
}

fn is_readable_jsonl_file(path: &Path) -> bool {
    path.extension().and_then(|extension| extension.to_str()) == Some("jsonl")
        && fs::File::open(path).is_ok()
}

fn is_subagent_log_path(path: &Path) -> bool {
    path.components()
        .any(|component| component.as_os_str() == "subagents")
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

fn truncate_text(text: &str, limit: usize) -> String {
    let mut truncated = String::new();
    for character in text.chars().take(limit) {
        truncated.push(character);
    }
    if text.chars().count() > limit {
        truncated.push_str("...");
    }
    truncated
}

fn file_stem_session_id(file_path: &Path) -> String {
    file_path
        .file_stem()
        .and_then(|name| name.to_str())
        .unwrap_or("unknown-session")
        .to_string()
}

fn codex_file_stem_session_id(file_path: &Path) -> String {
    file_stem_session_id(file_path)
        .trim_start_matches("rollout-")
        .to_string()
}

fn distilled_session_markdown(
    summary: &SessionLogMetadata,
    messages: &[SessionLogMessage],
) -> String {
    let mut markdown = String::new();
    markdown.push_str("---\n");
    markdown.push_str("classification: shared\n");
    markdown.push_str("tags: [session-history, resume-context]\n");
    markdown.push_str("---\n\n");
    markdown.push_str("# Previous Session Context\n\n");
    markdown.push_str(&format!("- Provider: {}\n", summary.provider));
    markdown.push_str(&format!("- Session ID: {}\n", summary.session_id));
    if let Some(updated_at) = &summary.updated_at {
        markdown.push_str(&format!("- Updated: {updated_at}\n"));
    }
    if let Some(cwd) = &summary.cwd {
        markdown.push_str(&format!("- Working directory: `{cwd}`\n"));
    }
    markdown.push_str(&format!(
        "- Source log: `{}`\n\n",
        summary.file_path.display()
    ));
    markdown.push_str("## New Session Handoff\n\n");
    markdown.push_str("- Review this distilled context before relying on it.\n");
    markdown.push_str("- Remove stale decisions, noisy tool output, and sensitive details.\n\n");
    markdown.push_str("## Conversation Timeline\n\n");

    for message in messages.iter().take(80) {
        let role = match message.role.as_str() {
            "user" => "User",
            "assistant" => "Assistant",
            other => other,
        };
        if let Some(timestamp) = &message.timestamp {
            markdown.push_str(&format!("### {role} ({timestamp})\n\n"));
        } else {
            markdown.push_str(&format!("### {role}\n\n"));
        }
        markdown.push_str(message.content.trim());
        markdown.push_str("\n\n");
    }

    if messages.len() > 80 {
        markdown.push_str(&format!(
            "_{} later messages omitted from this draft._\n",
            messages.len() - 80
        ));
    }

    markdown
}

fn modified_time_string(path: &Path) -> Option<String> {
    let modified = fs::metadata(path).ok()?.modified().ok()?;
    let duration = modified.duration_since(SystemTime::UNIX_EPOCH).ok()?;
    Some(format!("unix:{}", duration.as_secs()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    #[test]
    fn claude_scanner_discovers_logs_and_normalizes_metadata() {
        let base = std::env::temp_dir().join(format!("ctx-claude-scanner-{}", Uuid::new_v4()));
        let root = base.join(".claude").join("projects").join("project-a");
        let nested = root.join("nested");
        let subagents = root.join("subagents");
        fs::create_dir_all(&nested).expect("nested Claude log directory should be created");
        fs::create_dir_all(&subagents).expect("subagent log directory should be created");
        let log_path = nested.join("fallback-id.jsonl");
        fs::write(
            &log_path,
            r#"{"type":"summary","summary":"ignored"}
{"sessionId":"claude-session-1","cwd":"/tmp/work","timestamp":"2026-05-11T00:00:00Z","type":"user","message":{"role":"user","content":[{"type":"text","text":"Implement scanner metadata"}]}}
{"sessionId":"claude-session-1","cwd":"/tmp/work","timestamp":"2026-05-11T00:00:01Z","type":"assistant","message":{"role":"assistant","content":"Done"}}
"#,
        )
        .expect("Claude log should be writable");
        fs::write(
            subagents.join("ignored.jsonl"),
            r#"{"sessionId":"subagent-session","type":"user","message":{"role":"user","content":"Ignore me"}}"#,
        )
        .expect("subagent Claude log should be writable");

        let scanner = ClaudeSessionLogScanner;
        let result = scanner
            .scan_session_logs(&SessionLogScanRequest {
                provider: SessionLogProvider::Claude,
                home_dir: base.clone(),
                working_dir: base.clone(),
                root_paths: vec![base.join(".claude").join("projects")],
            })
            .expect("Claude scanner should scan readable logs");

        assert_eq!(result.provider, SessionLogProvider::Claude);
        assert_eq!(result.sessions.len(), 1);
        let metadata = &result.sessions[0];
        assert_eq!(metadata.provider, "claude");
        assert_eq!(metadata.session_id, "claude-session-1");
        assert_eq!(metadata.cwd.as_deref(), Some("/tmp/work"));
        assert_eq!(metadata.updated_at.as_deref(), Some("2026-05-11T00:00:01Z"));
        assert_eq!(metadata.message_count, 2);
        assert_eq!(
            metadata.last_user_message.as_deref(),
            Some("Implement scanner metadata")
        );
        assert_eq!(metadata.title, "Implement scanner metadata");
        assert_eq!(metadata.file_path, log_path.canonicalize().unwrap());

        fs::remove_dir_all(base).ok();
    }

    #[test]
    fn claude_log_path_enumerator_returns_known_readable_session_logs() {
        let base =
            std::env::temp_dir().join(format!("ctx-claude-log-path-enumerator-{}", Uuid::new_v4()));
        let known_root = base.join(".claude").join("projects");
        let project_dir = known_root.join("workspace-app");
        let nested_dir = project_dir.join("nested");
        let subagent_dir = project_dir.join("subagents");
        fs::create_dir_all(&nested_dir).expect("nested Claude session directory should be created");
        fs::create_dir_all(&subagent_dir)
            .expect("subagent Claude session directory should be created");
        let first_log = project_dir.join("session-a.jsonl");
        let second_log = nested_dir.join("session-b.jsonl");
        let ignored_text = project_dir.join("notes.md");
        let ignored_subagent_log = subagent_dir.join("subagent.jsonl");
        fs::write(&first_log, "{}\n").expect("first Claude session log should be writable");
        fs::write(&second_log, "{}\n").expect("second Claude session log should be writable");
        fs::write(&ignored_text, "not a session log").expect("non-jsonl file should be writable");
        fs::write(&ignored_subagent_log, "{}\n").expect("subagent Claude log should be writable");

        let paths = enumerate_claude_session_log_paths(&[known_root.clone(), project_dir])
            .expect("Claude session log paths should enumerate from known roots");

        assert_eq!(
            paths,
            vec![
                second_log
                    .canonicalize()
                    .expect("second log should canonicalize"),
                first_log
                    .canonicalize()
                    .expect("first log should canonicalize"),
            ]
        );

        fs::remove_dir_all(base).ok();
    }

    #[test]
    fn claude_scanner_rejects_other_provider_requests() {
        let scanner = ClaudeSessionLogScanner;
        let error = scanner
            .scan_session_logs(&SessionLogScanRequest {
                provider: SessionLogProvider::Codex,
                home_dir: PathBuf::new(),
                working_dir: PathBuf::new(),
                root_paths: Vec::new(),
            })
            .expect_err("Claude scanner should reject non-Claude provider requests");

        assert_eq!(error.kind, SessionLogScanErrorKind::UnsupportedProvider);
    }

    #[test]
    fn codex_scanner_discovers_logs_and_normalizes_metadata() {
        let base = std::env::temp_dir().join(format!("ctx-codex-scanner-{}", Uuid::new_v4()));
        let home = base.join("home");
        let root = home
            .join(".codex")
            .join("sessions")
            .join("2026")
            .join("05")
            .join("11");
        fs::create_dir_all(&root).expect("Codex log directory should be created");
        fs::create_dir_all(home.join(".codex")).expect("Codex home directory should be created");
        let log_path = root.join("rollout-fallback-id.jsonl");
        fs::write(
            &log_path,
            r#"{"type":"session_meta","timestamp":"2026-05-11T00:00:00Z","payload":{"id":"codex-session-1","cwd":"/tmp/codex-work","timestamp":"2026-05-11T00:00:00Z"}}
{"type":"event_msg","timestamp":"2026-05-11T00:00:01Z","payload":{"type":"user_message","message":"Implement Codex scanner metadata"}}
{"type":"event_msg","timestamp":"2026-05-11T00:00:02Z","payload":{"type":"agent_message","message":"Done"}}
{"type":"response_item","timestamp":"2026-05-11T00:00:03Z","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"Verified"}]}}
"#,
        )
        .expect("Codex log should be writable");
        fs::write(
            home.join(".codex").join("session_index.jsonl"),
            r#"{"id":"codex-session-1","thread_name":"Indexed Codex title","updated_at":"2026-05-11T00:10:00Z"}"#,
        )
        .expect("Codex session index should be writable");

        let scanner = CodexSessionLogScanner;
        let result = scanner
            .scan_session_logs(&SessionLogScanRequest {
                provider: SessionLogProvider::Codex,
                home_dir: home.clone(),
                working_dir: base.clone(),
                root_paths: vec![home.join(".codex").join("sessions")],
            })
            .expect("Codex scanner should scan readable logs");

        assert_eq!(result.provider, SessionLogProvider::Codex);
        assert_eq!(result.sessions.len(), 1);
        let metadata = &result.sessions[0];
        assert_eq!(metadata.provider, "codex");
        assert_eq!(metadata.session_id, "codex-session-1");
        assert_eq!(metadata.title, "Indexed Codex title");
        assert_eq!(metadata.updated_at.as_deref(), Some("2026-05-11T00:10:00Z"));
        assert_eq!(metadata.cwd.as_deref(), Some("/tmp/codex-work"));
        assert_eq!(metadata.message_count, 3);
        assert_eq!(
            metadata.last_user_message.as_deref(),
            Some("Implement Codex scanner metadata")
        );
        assert_eq!(metadata.file_path, log_path.canonicalize().unwrap());

        fs::remove_dir_all(base).ok();
    }

    #[test]
    fn shared_scanner_contract_returns_consistent_normalized_metadata_for_claude_and_codex() {
        let base = std::env::temp_dir().join(format!("ctx-shared-scanner-{}", Uuid::new_v4()));
        let home = base.join("home");
        let project = base.join("project");
        let claude_root = home.join(".claude").join("projects").join("project-a");
        let codex_root = home
            .join(".codex")
            .join("sessions")
            .join("2026")
            .join("05")
            .join("11");
        fs::create_dir_all(&claude_root).expect("Claude fixture directory should be created");
        fs::create_dir_all(&codex_root).expect("Codex fixture directory should be created");
        fs::create_dir_all(home.join(".codex")).expect("Codex home directory should be created");

        let expected_session_id = "shared-normalized-session";
        let expected_cwd = "/tmp/shared-project";
        let expected_title = "Normalize shared session metadata";
        let expected_updated_at = "2026-05-11T01:00:00Z";

        let claude_log = claude_root.join("fallback-claude.jsonl");
        fs::write(
            &claude_log,
            format!(
                r#"{{"sessionId":"{expected_session_id}","cwd":"{expected_cwd}","timestamp":"2026-05-11T00:59:00Z","type":"user","message":{{"role":"user","content":"{expected_title}"}}}}
{{"sessionId":"{expected_session_id}","cwd":"{expected_cwd}","timestamp":"{expected_updated_at}","type":"assistant","message":{{"role":"assistant","content":"Done"}}}}
"#
            ),
        )
        .expect("Claude fixture log should be writable");

        let codex_log = codex_root.join("rollout-fallback-codex.jsonl");
        fs::write(
            &codex_log,
            format!(
                r#"{{"type":"session_meta","timestamp":"2026-05-11T00:58:00Z","payload":{{"id":"{expected_session_id}","cwd":"{expected_cwd}","timestamp":"2026-05-11T00:58:00Z"}}}}
{{"type":"event_msg","timestamp":"2026-05-11T00:59:00Z","payload":{{"type":"user_message","message":"{expected_title}"}}}}
{{"type":"event_msg","timestamp":"{expected_updated_at}","payload":{{"type":"agent_message","message":"Done"}}}}
"#
            ),
        )
        .expect("Codex fixture log should be writable");
        fs::write(
            home.join(".codex").join("session_index.jsonl"),
            format!(
                r#"{{"id":"{expected_session_id}","thread_name":"{expected_title}","updated_at":"{expected_updated_at}"}}"#
            ),
        )
        .expect("Codex fixture index should be writable");

        let claude_metadata = scan_single_session(
            &ClaudeSessionLogScanner,
            SessionLogProvider::Claude,
            &home,
            &project,
            claude_root,
        );
        let codex_metadata = scan_single_session(
            &CodexSessionLogScanner,
            SessionLogProvider::Codex,
            &home,
            &project,
            home.join(".codex").join("sessions"),
        );

        assert_eq!(
            claude_metadata.provider_kind(),
            Some(SessionLogProvider::Claude)
        );
        assert_eq!(
            codex_metadata.provider_kind(),
            Some(SessionLogProvider::Codex)
        );
        for metadata in [&claude_metadata, &codex_metadata] {
            assert_eq!(metadata.source_session_ref(), expected_session_id);
            assert_eq!(
                metadata.source_working_directory(),
                Some(expected_cwd),
                "scanner should normalize cwd for {} logs",
                metadata.provider
            );
            assert_eq!(metadata.title, expected_title);
            assert_eq!(metadata.updated_at.as_deref(), Some(expected_updated_at));
            assert_eq!(metadata.message_count, 2);
            assert_eq!(metadata.last_user_message.as_deref(), Some(expected_title));
        }
        assert_eq!(
            claude_metadata.file_path,
            claude_log.canonicalize().unwrap()
        );
        assert_eq!(codex_metadata.file_path, codex_log.canonicalize().unwrap());

        fs::remove_dir_all(base).ok();
    }

    #[test]
    fn codex_scanner_rejects_other_provider_requests() {
        let scanner = CodexSessionLogScanner;
        let error = scanner
            .scan_session_logs(&SessionLogScanRequest {
                provider: SessionLogProvider::Claude,
                home_dir: PathBuf::new(),
                working_dir: PathBuf::new(),
                root_paths: Vec::new(),
            })
            .expect_err("Codex scanner should reject non-Codex provider requests");

        assert_eq!(error.kind, SessionLogScanErrorKind::UnsupportedProvider);
    }

    #[test]
    fn claude_detail_parser_normalizes_messages_and_event_records() {
        let base = std::env::temp_dir().join(format!("ctx-claude-detail-{}", Uuid::new_v4()));
        fs::create_dir_all(&base).expect("Claude detail fixture directory should be created");
        let log_path = base.join("fallback-claude-detail.jsonl");
        fs::write(
            &log_path,
            r#"{"type":"summary","summary":"Worked on parser"}
{"sessionId":"claude-detail-1","cwd":"/tmp/claude-detail","timestamp":"2026-05-11T00:00:00Z","type":"user","message":{"role":"user","content":[{"type":"text","text":"Parse Claude messages"}]}}
{"sessionId":"claude-detail-1","cwd":"/tmp/claude-detail","timestamp":"2026-05-11T00:00:01Z","type":"assistant","message":{"role":"assistant","content":{"type":"text","text":"Implemented parser"}}}
{"sessionId":"claude-detail-1","cwd":"/tmp/claude-detail","timestamp":"2026-05-11T00:00:02Z","type":"queue-operation","content":"Follow-up queued"}
"#,
        )
        .expect("Claude detail log should be writable");

        let detail =
            parse_claude_session_log_detail(&log_path).expect("Claude detail should parse");

        assert_eq!(detail.summary.provider, "claude");
        assert_eq!(detail.summary.session_id, "claude-detail-1");
        assert_eq!(detail.summary.cwd.as_deref(), Some("/tmp/claude-detail"));
        assert_eq!(detail.summary.message_count, 3);
        assert_eq!(
            detail.summary.last_user_message.as_deref(),
            Some("Follow-up queued")
        );
        assert_eq!(detail.messages.len(), 3);
        assert_eq!(detail.messages[0].role, "user");
        assert_eq!(detail.messages[0].content, "Parse Claude messages");
        assert_eq!(detail.messages[1].role, "assistant");
        assert_eq!(detail.messages[1].content, "Implemented parser");
        assert_eq!(detail.messages[2].role, "user");
        assert_eq!(detail.messages[2].content, "Follow-up queued");
        assert_eq!(detail.events.len(), 4);
        assert_eq!(detail.events[0].record_type, "summary");
        assert_eq!(
            detail.events[0].content.as_deref(),
            Some("Worked on parser")
        );
        assert_eq!(detail.events[1].line_number, 2);
        assert_eq!(detail.events[1].role.as_deref(), Some("user"));
        assert_eq!(
            detail.events[1].content.as_deref(),
            Some("Parse Claude messages")
        );
        assert!(detail
            .distilled_markdown
            .contains("## Conversation Timeline"));

        fs::remove_dir_all(base).ok();
    }

    #[test]
    fn codex_detail_parser_normalizes_messages_and_event_records() {
        let base = std::env::temp_dir().join(format!("ctx-codex-detail-{}", Uuid::new_v4()));
        fs::create_dir_all(&base).expect("Codex detail fixture directory should be created");
        let log_path = base.join("rollout-fallback-codex-detail.jsonl");
        fs::write(
            &log_path,
            r#"{"type":"session_meta","timestamp":"2026-05-11T00:00:00Z","payload":{"id":"codex-detail-1","cwd":"/tmp/codex-detail","timestamp":"2026-05-11T00:00:00Z"}}
{"type":"event_msg","timestamp":"2026-05-11T00:00:01Z","payload":{"type":"user_message","message":"Parse Codex events"}}
{"type":"event_msg","timestamp":"2026-05-11T00:00:02Z","payload":{"type":"agent_message","message":"Event parser complete"}}
{"type":"response_item","timestamp":"2026-05-11T00:00:03Z","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"Structured response item"}]}}
{"type":"event_msg","timestamp":"2026-05-11T00:00:04Z","payload":{"type":"token_count","message":"metadata only"}}
"#,
        )
        .expect("Codex detail log should be writable");
        let mut index = HashMap::new();
        index.insert(
            "codex-detail-1".to_string(),
            CodexSessionIndexEntry {
                title: "Indexed detail title".to_string(),
                updated_at: "2026-05-11T00:05:00Z".to_string(),
            },
        );

        let detail = parse_codex_session_log_detail(&log_path, Some(&index))
            .expect("Codex detail should parse");

        assert_eq!(detail.summary.provider, "codex");
        assert_eq!(detail.summary.session_id, "codex-detail-1");
        assert_eq!(detail.summary.title, "Indexed detail title");
        assert_eq!(
            detail.summary.updated_at.as_deref(),
            Some("2026-05-11T00:05:00Z")
        );
        assert_eq!(detail.summary.cwd.as_deref(), Some("/tmp/codex-detail"));
        assert_eq!(detail.summary.message_count, 3);
        assert_eq!(detail.messages.len(), 3);
        assert_eq!(detail.messages[0].role, "user");
        assert_eq!(detail.messages[1].role, "assistant");
        assert_eq!(detail.messages[2].content, "Structured response item");
        assert_eq!(detail.events.len(), 5);
        assert_eq!(detail.events[0].record_type, "session_meta");
        assert_eq!(detail.events[1].event_type.as_deref(), Some("user_message"));
        assert_eq!(detail.events[1].role.as_deref(), Some("user"));
        assert_eq!(detail.events[3].event_type.as_deref(), Some("message"));
        assert_eq!(detail.events[4].event_type.as_deref(), Some("token_count"));
        assert_eq!(detail.events[4].content.as_deref(), Some("metadata only"));
        assert!(detail
            .distilled_markdown
            .contains("Structured response item"));

        fs::remove_dir_all(base).ok();
    }

    fn scan_single_session(
        scanner: &impl SessionLogScanner,
        provider: SessionLogProvider,
        home_dir: &Path,
        working_dir: &Path,
        root_path: PathBuf,
    ) -> SessionLogMetadata {
        let result = scanner
            .scan_session_logs(&SessionLogScanRequest {
                provider,
                home_dir: home_dir.to_path_buf(),
                working_dir: working_dir.to_path_buf(),
                root_paths: vec![root_path],
            })
            .expect("shared scanner fixture should scan successfully");
        assert_eq!(result.provider, provider);
        assert_eq!(result.sessions.len(), 1);
        result.sessions.into_iter().next().unwrap()
    }
}
