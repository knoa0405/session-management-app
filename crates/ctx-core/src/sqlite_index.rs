use crate::{Classification, VaultScope};
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    fmt, fs,
    path::{Path, PathBuf},
};

pub fn classification_sql_value(classification: Classification) -> &'static str {
    match classification {
        Classification::MainAgent => "main-agent",
        Classification::Subagent => "subagent",
        Classification::Shared => "shared",
    }
}

pub fn classification_status_sql_value(status: crate::ClassificationStatus) -> &'static str {
    match status {
        crate::ClassificationStatus::Pending => "pending",
        crate::ClassificationStatus::Classified => "classified",
        crate::ClassificationStatus::Reviewed => "reviewed",
        crate::ClassificationStatus::Modified => "modified",
    }
}

pub fn vault_scope_sql_value(scope: VaultScope) -> &'static str {
    match scope {
        VaultScope::Global => "global",
        VaultScope::Local => "local",
    }
}

pub fn import_source_type_sql_value(source_type: crate::ImportSourceType) -> &'static str {
    match source_type {
        crate::ImportSourceType::ContextMarkdown => "context-markdown",
        crate::ImportSourceType::ClaudeMarkdown => "claude-markdown",
        crate::ImportSourceType::CodexAgents => "codex-agents",
        crate::ImportSourceType::AgentMarkdown => "agent-markdown",
        crate::ImportSourceType::AgentsManifest => "agents-manifest",
        crate::ImportSourceType::SkillMarkdown => "skill-markdown",
        crate::ImportSourceType::SkillManifest => "skill-manifest",
        crate::ImportSourceType::SubagentMarkdown => "subagent-markdown",
    }
}

pub const CTX_INDEX_SCHEMA_VERSION: i32 = 7;
pub const CTX_INDEX_DATABASE_FILE_NAME: &str = "ctx-index.sqlite3";
pub const MARKDOWN_FILES_TABLE_NAME: &str = "markdown_files";
pub const MARKDOWN_FILE_SEARCH_TABLE_NAME: &str = "markdown_file_search";
pub const MARKDOWN_FILE_FRONTMATTER_TABLE_NAME: &str = "markdown_file_frontmatter";
pub const TAGS_TABLE_NAME: &str = "tags";
pub const MARKDOWN_FILE_TAGS_TABLE_NAME: &str = "markdown_file_tags";
pub const MARKDOWN_FILE_LINKS_TABLE_NAME: &str = "markdown_file_links";
pub const MARKDOWN_FILE_BACKLINKS_VIEW_NAME: &str = "markdown_file_backlinks";

pub const CREATE_MARKDOWN_FILES_TABLE_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS markdown_files (
    path TEXT PRIMARY KEY NOT NULL,
    context_id TEXT NOT NULL,
    title TEXT NOT NULL,
    vault_scope TEXT NOT NULL CHECK (vault_scope IN ('global', 'local')),
    relative_path TEXT NOT NULL,
    folder_path TEXT NOT NULL DEFAULT '',
    file_name TEXT NOT NULL,
    classification TEXT NOT NULL CHECK (
        classification IN ('main-agent', 'subagent', 'shared')
    ),
    import_classification_suggestion TEXT CHECK (
        import_classification_suggestion IN ('main-agent', 'subagent', 'shared')
    ),
    inferred_classification TEXT CHECK (
        inferred_classification IN ('main-agent', 'subagent', 'shared')
    ),
    llm_classification_status TEXT NOT NULL CHECK (
        llm_classification_status IN ('pending', 'classified', 'reviewed', 'modified')
    ),
    file_created_at_unix_seconds INTEGER NOT NULL,
    file_modified_at_unix_seconds INTEGER NOT NULL,
    indexed_at_unix_seconds INTEGER,
    content_hash TEXT NOT NULL,
    indexing_status TEXT NOT NULL CHECK (
        indexing_status IN ('pending', 'indexed', 'stale', 'failed')
    ),
    last_index_error TEXT,
    import_source TEXT,
    import_source_type TEXT CHECK (
        import_source_type IN (
            'context-markdown',
            'claude-markdown',
            'codex-agents',
            'agent-markdown',
            'agents-manifest',
            'skill-markdown',
            'skill-manifest',
            'subagent-markdown'
        )
    ),
    UNIQUE (vault_scope, relative_path)
);
"#;

pub const CREATE_MARKDOWN_FILE_SEARCH_TABLE_SQL: &str = r#"
CREATE VIRTUAL TABLE IF NOT EXISTS markdown_file_search
USING fts5(
    path UNINDEXED,
    title,
    relative_path,
    content
);
"#;

pub const CREATE_MARKDOWN_FILE_FRONTMATTER_TABLE_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS markdown_file_frontmatter (
    path TEXT PRIMARY KEY NOT NULL REFERENCES markdown_files(path) ON DELETE CASCADE,
    frontmatter_format TEXT NOT NULL CHECK (
        frontmatter_format IN ('none', 'yaml', 'toml', 'json', 'unknown')
    ),
    frontmatter_raw TEXT,
    frontmatter_json TEXT NOT NULL DEFAULT '{}',
    frontmatter_title TEXT,
    frontmatter_tags_json TEXT NOT NULL DEFAULT '[]',
    frontmatter_classification TEXT CHECK (
        frontmatter_classification IN ('main-agent', 'subagent', 'shared')
    ),
    parse_status TEXT NOT NULL CHECK (
        parse_status IN ('absent', 'parsed', 'unsupported', 'failed')
    ),
    parse_error TEXT,
    parsed_at_unix_seconds INTEGER
);
"#;

pub const CREATE_TAGS_TABLE_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS tags (
    tag_id TEXT PRIMARY KEY NOT NULL,
    display_name TEXT NOT NULL,
    normalized_name TEXT NOT NULL UNIQUE,
    created_at_unix_seconds INTEGER,
    CHECK (length(trim(normalized_name)) > 0),
    CHECK (normalized_name = lower(normalized_name))
);
"#;

pub const CREATE_MARKDOWN_FILE_TAGS_TABLE_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS markdown_file_tags (
    path TEXT NOT NULL REFERENCES markdown_files(path) ON DELETE CASCADE,
    tag_id TEXT NOT NULL REFERENCES tags(tag_id) ON DELETE CASCADE,
    tag_source TEXT NOT NULL CHECK (
        tag_source IN ('frontmatter', 'body', 'llm-classification', 'manual', 'import')
    ),
    tag_position INTEGER NOT NULL DEFAULT 0,
    indexed_at_unix_seconds INTEGER,
    PRIMARY KEY (path, tag_id)
);
"#;

pub const CREATE_MARKDOWN_FILE_LINKS_TABLE_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS markdown_file_links (
    link_id TEXT PRIMARY KEY NOT NULL,
    source_path TEXT NOT NULL REFERENCES markdown_files(path) ON DELETE CASCADE,
    link_kind TEXT NOT NULL CHECK (
        link_kind IN ('wikilink', 'markdown')
    ),
    raw_target TEXT NOT NULL,
    normalized_target TEXT NOT NULL,
    link_text TEXT,
    target_path TEXT REFERENCES markdown_files(path) ON DELETE SET NULL,
    target_anchor TEXT,
    target_url TEXT,
    resolved_status TEXT NOT NULL CHECK (
        resolved_status IN ('unresolved', 'resolved', 'external', 'ambiguous')
    ),
    byte_start INTEGER,
    byte_end INTEGER,
    line_number INTEGER,
    link_position INTEGER NOT NULL DEFAULT 0,
    indexed_at_unix_seconds INTEGER,
    CHECK (length(trim(raw_target)) > 0),
    CHECK (length(trim(normalized_target)) > 0),
    CHECK (
        target_path IS NOT NULL
        OR target_anchor IS NOT NULL
        OR target_url IS NOT NULL
        OR resolved_status = 'unresolved'
    )
);
"#;

pub const CREATE_MARKDOWN_FILE_BACKLINKS_VIEW_SQL: &str = r#"
CREATE VIEW IF NOT EXISTS markdown_file_backlinks AS
SELECT
    target_path AS path,
    source_path AS backlink_source_path,
    link_id,
    link_kind,
    raw_target,
    normalized_target,
    link_text,
    target_anchor,
    byte_start,
    byte_end,
    line_number,
    link_position,
    indexed_at_unix_seconds
FROM markdown_file_links
WHERE target_path IS NOT NULL
  AND resolved_status = 'resolved';
"#;

pub const CREATE_MARKDOWN_FILES_INDEXING_STATUS_INDEX_SQL: &str = r#"
CREATE INDEX IF NOT EXISTS idx_markdown_files_indexing_status
ON markdown_files (indexing_status);
"#;

pub const CREATE_MARKDOWN_FILES_CONTENT_HASH_INDEX_SQL: &str = r#"
CREATE INDEX IF NOT EXISTS idx_markdown_files_content_hash
ON markdown_files (content_hash);
"#;

pub const CREATE_MARKDOWN_FILES_VAULT_SCOPE_INDEX_SQL: &str = r#"
CREATE INDEX IF NOT EXISTS idx_markdown_files_vault_scope_relative_path
ON markdown_files (vault_scope, relative_path);
"#;

pub const CREATE_MARKDOWN_FILE_FRONTMATTER_PARSE_STATUS_INDEX_SQL: &str = r#"
CREATE INDEX IF NOT EXISTS idx_markdown_file_frontmatter_parse_status
ON markdown_file_frontmatter (parse_status);
"#;

pub const CREATE_MARKDOWN_FILE_FRONTMATTER_TITLE_INDEX_SQL: &str = r#"
CREATE INDEX IF NOT EXISTS idx_markdown_file_frontmatter_title
ON markdown_file_frontmatter (frontmatter_title);
"#;

pub const CREATE_MARKDOWN_FILE_TAGS_TAG_ID_INDEX_SQL: &str = r#"
CREATE INDEX IF NOT EXISTS idx_markdown_file_tags_tag_id
ON markdown_file_tags (tag_id);
"#;

pub const CREATE_MARKDOWN_FILE_TAGS_SOURCE_INDEX_SQL: &str = r#"
CREATE INDEX IF NOT EXISTS idx_markdown_file_tags_source
ON markdown_file_tags (tag_source);
"#;

pub const CREATE_MARKDOWN_FILE_TAGS_PATH_POSITION_INDEX_SQL: &str = r#"
CREATE INDEX IF NOT EXISTS idx_markdown_file_tags_path_position
ON markdown_file_tags (path, tag_position);
"#;

pub const CREATE_MARKDOWN_FILE_LINKS_SOURCE_INDEX_SQL: &str = r#"
CREATE INDEX IF NOT EXISTS idx_markdown_file_links_source
ON markdown_file_links (source_path, link_position);
"#;

pub const CREATE_MARKDOWN_FILE_LINKS_TARGET_INDEX_SQL: &str = r#"
CREATE INDEX IF NOT EXISTS idx_markdown_file_links_target
ON markdown_file_links (target_path);
"#;

pub const CREATE_MARKDOWN_FILE_LINKS_NORMALIZED_TARGET_INDEX_SQL: &str = r#"
CREATE INDEX IF NOT EXISTS idx_markdown_file_links_normalized_target
ON markdown_file_links (normalized_target);
"#;

pub const CREATE_MARKDOWN_FILE_LINKS_KIND_STATUS_INDEX_SQL: &str = r#"
CREATE INDEX IF NOT EXISTS idx_markdown_file_links_kind_status
ON markdown_file_links (link_kind, resolved_status);
"#;

pub const SQLITE_INDEX_SCHEMA_STATEMENTS: &[&str] = &[
    CREATE_MARKDOWN_FILES_TABLE_SQL,
    CREATE_MARKDOWN_FILE_SEARCH_TABLE_SQL,
    CREATE_MARKDOWN_FILE_FRONTMATTER_TABLE_SQL,
    CREATE_TAGS_TABLE_SQL,
    CREATE_MARKDOWN_FILE_TAGS_TABLE_SQL,
    CREATE_MARKDOWN_FILE_LINKS_TABLE_SQL,
    CREATE_MARKDOWN_FILE_BACKLINKS_VIEW_SQL,
    CREATE_MARKDOWN_FILES_INDEXING_STATUS_INDEX_SQL,
    CREATE_MARKDOWN_FILES_CONTENT_HASH_INDEX_SQL,
    CREATE_MARKDOWN_FILES_VAULT_SCOPE_INDEX_SQL,
    CREATE_MARKDOWN_FILE_FRONTMATTER_PARSE_STATUS_INDEX_SQL,
    CREATE_MARKDOWN_FILE_FRONTMATTER_TITLE_INDEX_SQL,
    CREATE_MARKDOWN_FILE_TAGS_TAG_ID_INDEX_SQL,
    CREATE_MARKDOWN_FILE_TAGS_SOURCE_INDEX_SQL,
    CREATE_MARKDOWN_FILE_TAGS_PATH_POSITION_INDEX_SQL,
    CREATE_MARKDOWN_FILE_LINKS_SOURCE_INDEX_SQL,
    CREATE_MARKDOWN_FILE_LINKS_TARGET_INDEX_SQL,
    CREATE_MARKDOWN_FILE_LINKS_NORMALIZED_TARGET_INDEX_SQL,
    CREATE_MARKDOWN_FILE_LINKS_KIND_STATUS_INDEX_SQL,
];

pub const MARKDOWN_FILES_COLUMN_MIGRATIONS: &[(&str, &str)] = &[
    ("context_id", "ALTER TABLE markdown_files ADD COLUMN context_id TEXT NOT NULL DEFAULT ''"),
    ("title", "ALTER TABLE markdown_files ADD COLUMN title TEXT NOT NULL DEFAULT ''"),
    (
        "folder_path",
        "ALTER TABLE markdown_files ADD COLUMN folder_path TEXT NOT NULL DEFAULT ''",
    ),
    (
        "classification",
        "ALTER TABLE markdown_files ADD COLUMN classification TEXT NOT NULL DEFAULT 'shared' CHECK (classification IN ('main-agent', 'subagent', 'shared'))",
    ),
    (
        "import_classification_suggestion",
        "ALTER TABLE markdown_files ADD COLUMN import_classification_suggestion TEXT CHECK (import_classification_suggestion IN ('main-agent', 'subagent', 'shared'))",
    ),
    (
        "inferred_classification",
        "ALTER TABLE markdown_files ADD COLUMN inferred_classification TEXT CHECK (inferred_classification IN ('main-agent', 'subagent', 'shared'))",
    ),
    (
        "llm_classification_status",
        "ALTER TABLE markdown_files ADD COLUMN llm_classification_status TEXT NOT NULL DEFAULT 'pending' CHECK (llm_classification_status IN ('pending', 'classified', 'reviewed', 'modified'))",
    ),
    (
        "import_source_type",
        "ALTER TABLE markdown_files ADD COLUMN import_source_type TEXT CHECK (import_source_type IN ('context-markdown', 'claude-markdown', 'codex-agents', 'agent-markdown', 'agents-manifest', 'skill-markdown', 'skill-manifest', 'subagent-markdown'))",
    ),
];

#[derive(Debug, Clone, Eq, PartialEq)]
pub enum SqliteIndexError {
    Io(String),
    Sqlite(String),
    UnsupportedFutureSchema { found: i32, supported: i32 },
}

impl fmt::Display for SqliteIndexError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(message) => write!(formatter, "{message}"),
            Self::Sqlite(message) => write!(formatter, "{message}"),
            Self::UnsupportedFutureSchema { found, supported } => write!(
                formatter,
                "ctx index schema version {found} is newer than supported version {supported}"
            ),
        }
    }
}

impl std::error::Error for SqliteIndexError {}

impl From<rusqlite::Error> for SqliteIndexError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Sqlite(error.to_string())
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct SqliteIndexMigrationReport {
    pub database_path: Option<PathBuf>,
    pub previous_schema_version: i32,
    pub applied_schema_version: i32,
    pub statements_applied: usize,
}

pub fn sqlite_index_path(vault_root: &Path) -> PathBuf {
    vault_root.join(CTX_INDEX_DATABASE_FILE_NAME)
}

pub fn apply_sqlite_index_migrations(
    database_path: &Path,
) -> Result<SqliteIndexMigrationReport, SqliteIndexError> {
    if let Some(parent) = database_path.parent() {
        fs::create_dir_all(parent).map_err(|error| {
            SqliteIndexError::Io(format!(
                "failed to create sqlite index directory {}: {error}",
                parent.display()
            ))
        })?;
    }

    let mut connection = Connection::open(database_path).map_err(|error| {
        SqliteIndexError::Sqlite(format!(
            "failed to open sqlite index database {}: {error}",
            database_path.display()
        ))
    })?;
    let mut report = apply_sqlite_index_migrations_to_connection(&mut connection)?;
    report.database_path = Some(database_path.to_path_buf());
    Ok(report)
}

pub fn apply_sqlite_index_migrations_to_connection(
    connection: &mut Connection,
) -> Result<SqliteIndexMigrationReport, SqliteIndexError> {
    connection.execute_batch("PRAGMA foreign_keys = ON;")?;
    let previous_schema_version = sqlite_user_version(connection)?;
    if previous_schema_version > CTX_INDEX_SCHEMA_VERSION {
        return Err(SqliteIndexError::UnsupportedFutureSchema {
            found: previous_schema_version,
            supported: CTX_INDEX_SCHEMA_VERSION,
        });
    }

    let transaction = connection.transaction()?;
    let mut statements_applied = 0;

    for statement in SQLITE_INDEX_SCHEMA_STATEMENTS {
        transaction.execute_batch(statement)?;
        statements_applied += 1;
    }

    let existing_columns = table_columns(&transaction, MARKDOWN_FILES_TABLE_NAME)?;
    for (column_name, statement) in MARKDOWN_FILES_COLUMN_MIGRATIONS {
        if !existing_columns.contains(*column_name) {
            transaction.execute_batch(statement)?;
            statements_applied += 1;
        }
    }

    transaction.execute_batch(&format!(
        "PRAGMA user_version = {CTX_INDEX_SCHEMA_VERSION};"
    ))?;
    transaction.commit()?;

    Ok(SqliteIndexMigrationReport {
        database_path: None,
        previous_schema_version,
        applied_schema_version: CTX_INDEX_SCHEMA_VERSION,
        statements_applied,
    })
}

fn sqlite_user_version(connection: &Connection) -> Result<i32, SqliteIndexError> {
    connection
        .query_row("PRAGMA user_version;", [], |row| row.get(0))
        .map_err(SqliteIndexError::from)
}

fn table_columns(
    connection: &Connection,
    table_name: &str,
) -> Result<BTreeSet<String>, SqliteIndexError> {
    let mut statement = connection.prepare(&format!("PRAGMA table_info({table_name});"))?;
    let rows = statement.query_map([], |row| row.get::<_, String>(1))?;
    let mut columns = BTreeSet::new();
    for row in rows {
        columns.insert(row?);
    }
    Ok(columns)
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum MarkdownFileIndexingStatus {
    Pending,
    Indexed,
    Stale,
    Failed,
}

impl MarkdownFileIndexingStatus {
    pub fn as_sql_value(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Indexed => "indexed",
            Self::Stale => "stale",
            Self::Failed => "failed",
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum FrontmatterFormat {
    None,
    Yaml,
    Toml,
    Json,
    Unknown,
}

impl FrontmatterFormat {
    pub fn as_sql_value(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Yaml => "yaml",
            Self::Toml => "toml",
            Self::Json => "json",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum FrontmatterParseStatus {
    Absent,
    Parsed,
    Unsupported,
    Failed,
}

impl FrontmatterParseStatus {
    pub fn as_sql_value(self) -> &'static str {
        match self {
            Self::Absent => "absent",
            Self::Parsed => "parsed",
            Self::Unsupported => "unsupported",
            Self::Failed => "failed",
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum MarkdownFileTagSource {
    Frontmatter,
    Body,
    LlmClassification,
    Manual,
    Import,
}

impl MarkdownFileTagSource {
    pub fn as_sql_value(self) -> &'static str {
        match self {
            Self::Frontmatter => "frontmatter",
            Self::Body => "body",
            Self::LlmClassification => "llm-classification",
            Self::Manual => "manual",
            Self::Import => "import",
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum MarkdownFileLinkKind {
    Wikilink,
    Markdown,
}

impl MarkdownFileLinkKind {
    pub fn as_sql_value(self) -> &'static str {
        match self {
            Self::Wikilink => "wikilink",
            Self::Markdown => "markdown",
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum MarkdownFileLinkResolvedStatus {
    Unresolved,
    Resolved,
    External,
    Ambiguous,
}

impl MarkdownFileLinkResolvedStatus {
    pub fn as_sql_value(self) -> &'static str {
        match self {
            Self::Unresolved => "unresolved",
            Self::Resolved => "resolved",
            Self::External => "external",
            Self::Ambiguous => "ambiguous",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Eq, PartialEq)]
pub struct NormalizedTagRecord {
    pub tag_id: String,
    pub display_name: String,
    pub normalized_name: String,
    pub created_at_unix_seconds: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Eq, PartialEq)]
pub struct MarkdownFileTagRecord {
    pub path: PathBuf,
    pub tag_id: String,
    pub tag_source: MarkdownFileTagSource,
    pub tag_position: i64,
    pub indexed_at_unix_seconds: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Eq, PartialEq)]
pub struct MarkdownFileLinkRecord {
    pub link_id: String,
    pub source_path: PathBuf,
    pub link_kind: MarkdownFileLinkKind,
    pub raw_target: String,
    pub normalized_target: String,
    pub link_text: Option<String>,
    pub target_path: Option<PathBuf>,
    pub target_anchor: Option<String>,
    pub target_url: Option<String>,
    pub resolved_status: MarkdownFileLinkResolvedStatus,
    pub byte_start: Option<i64>,
    pub byte_end: Option<i64>,
    pub line_number: Option<i64>,
    pub link_position: i64,
    pub indexed_at_unix_seconds: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Eq, PartialEq)]
pub struct MarkdownFileBacklinkRecord {
    pub path: PathBuf,
    pub backlink_source_path: PathBuf,
    pub link_id: String,
    pub link_kind: MarkdownFileLinkKind,
    pub raw_target: String,
    pub normalized_target: String,
    pub link_text: Option<String>,
    pub target_anchor: Option<String>,
    pub byte_start: Option<i64>,
    pub byte_end: Option<i64>,
    pub line_number: Option<i64>,
    pub link_position: i64,
    pub indexed_at_unix_seconds: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Eq, PartialEq)]
pub struct MarkdownFileMetadataRecord {
    pub path: PathBuf,
    pub context_id: String,
    pub title: String,
    pub vault_scope: VaultScope,
    pub relative_path: PathBuf,
    pub folder_path: PathBuf,
    pub file_name: String,
    pub classification: Classification,
    pub import_classification_suggestion: Option<Classification>,
    pub inferred_classification: Option<Classification>,
    pub llm_classification_status: crate::ClassificationStatus,
    pub file_created_at_unix_seconds: i64,
    pub file_modified_at_unix_seconds: i64,
    pub indexed_at_unix_seconds: Option<i64>,
    pub content_hash: String,
    pub indexing_status: MarkdownFileIndexingStatus,
    pub last_index_error: Option<String>,
    pub import_source: Option<PathBuf>,
    pub import_source_type: Option<crate::ImportSourceType>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Eq, PartialEq)]
pub struct MarkdownFileIndexLookup {
    pub metadata: MarkdownFileMetadataRecord,
    pub frontmatter: Option<ParsedFrontmatterMetadata>,
    pub tags: Vec<MarkdownFileTagRecord>,
    pub links: Vec<MarkdownFileLinkRecord>,
    pub backlinks: Vec<MarkdownFileBacklinkRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Eq, PartialEq)]
pub struct MarkdownFileSearchResult {
    pub path: PathBuf,
    pub title: String,
    pub relative_path: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize, Eq, PartialEq)]
pub struct ParsedFrontmatterMetadata {
    pub path: PathBuf,
    pub frontmatter_format: FrontmatterFormat,
    pub frontmatter_raw: Option<String>,
    pub frontmatter_json: String,
    pub frontmatter_title: Option<String>,
    pub frontmatter_tags: Vec<String>,
    pub frontmatter_classification: Option<Classification>,
    pub parse_status: FrontmatterParseStatus,
    pub parse_error: Option<String>,
    pub parsed_at_unix_seconds: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Eq, PartialEq)]
pub struct MarkdownFileIndexRecord {
    pub path: PathBuf,
    pub context_id: String,
    pub title: String,
    pub vault_scope: VaultScope,
    pub relative_path: PathBuf,
    pub folder_path: PathBuf,
    pub file_name: String,
    pub classification: Classification,
    pub import_classification_suggestion: Option<Classification>,
    pub inferred_classification: Option<Classification>,
    pub llm_classification_status: crate::ClassificationStatus,
    pub file_created_at_unix_seconds: i64,
    pub file_modified_at_unix_seconds: i64,
    pub indexed_at_unix_seconds: Option<i64>,
    pub content_hash: String,
    pub content: String,
    pub indexing_status: MarkdownFileIndexingStatus,
    pub last_index_error: Option<String>,
    pub import_source: Option<PathBuf>,
    pub import_source_type: Option<crate::ImportSourceType>,
    #[serde(default)]
    pub frontmatter: Option<ParsedFrontmatterMetadata>,
    #[serde(default)]
    pub tags: Vec<MarkdownFileTagRecord>,
    #[serde(default)]
    pub links: Vec<MarkdownFileLinkRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Eq, PartialEq)]
pub struct FullMarkdownReindexReport {
    pub database_path: Option<PathBuf>,
    pub cleared_markdown_files: usize,
    pub indexed_markdown_files: usize,
    pub indexed_tags: usize,
    pub indexed_links: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, Eq, PartialEq)]
pub struct IncrementalMarkdownIndexReport {
    pub database_path: Option<PathBuf>,
    pub indexed_markdown_files: usize,
    pub indexed_tags: usize,
    pub indexed_links: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, Eq, PartialEq)]
pub struct MarkdownFileIndexRemovalReport {
    pub database_path: Option<PathBuf>,
    pub removed_markdown_files: usize,
    pub updated_link_targets: usize,
}

pub fn full_reindex_markdown_files(
    database_path: &Path,
    records: &[MarkdownFileIndexRecord],
) -> Result<FullMarkdownReindexReport, SqliteIndexError> {
    apply_sqlite_index_migrations(database_path)?;
    let mut connection = Connection::open(database_path).map_err(|error| {
        SqliteIndexError::Sqlite(format!(
            "failed to open sqlite index database {}: {error}",
            database_path.display()
        ))
    })?;
    let mut report = full_reindex_markdown_files_to_connection(&mut connection, records)?;
    report.database_path = Some(database_path.to_path_buf());
    Ok(report)
}

pub fn full_reindex_markdown_files_to_connection(
    connection: &mut Connection,
    records: &[MarkdownFileIndexRecord],
) -> Result<FullMarkdownReindexReport, SqliteIndexError> {
    connection.execute_batch("PRAGMA foreign_keys = ON;")?;
    let transaction = connection.transaction()?;
    let cleared_markdown_files = clear_markdown_index_state(&transaction)?;
    let mut indexed_tags = 0;
    let mut indexed_links = 0;

    for record in records {
        upsert_markdown_file_record(&transaction, record)?;
        if let Some(frontmatter) = &record.frontmatter {
            upsert_frontmatter_record(&transaction, frontmatter)?;
        }
        for tag in &record.tags {
            upsert_tag_for_file(&transaction, tag)?;
            indexed_tags += 1;
        }
        for link in &record.links {
            upsert_link_for_file(&transaction, link)?;
            indexed_links += 1;
        }
    }

    transaction.commit()?;

    Ok(FullMarkdownReindexReport {
        database_path: None,
        cleared_markdown_files,
        indexed_markdown_files: records.len(),
        indexed_tags,
        indexed_links,
    })
}

pub fn upsert_markdown_file_index_record(
    database_path: &Path,
    record: &MarkdownFileIndexRecord,
) -> Result<IncrementalMarkdownIndexReport, SqliteIndexError> {
    apply_sqlite_index_migrations(database_path)?;
    let mut connection = Connection::open(database_path).map_err(|error| {
        SqliteIndexError::Sqlite(format!(
            "failed to open sqlite index database {}: {error}",
            database_path.display()
        ))
    })?;
    let mut report = upsert_markdown_file_index_record_to_connection(&mut connection, record)?;
    report.database_path = Some(database_path.to_path_buf());
    Ok(report)
}

pub fn upsert_markdown_file_index_record_to_connection(
    connection: &mut Connection,
    record: &MarkdownFileIndexRecord,
) -> Result<IncrementalMarkdownIndexReport, SqliteIndexError> {
    connection.execute_batch("PRAGMA foreign_keys = ON;")?;
    let transaction = connection.transaction()?;

    clear_markdown_file_index_state(&transaction, &record.path)?;
    upsert_markdown_file_record(&transaction, record)?;
    if let Some(frontmatter) = &record.frontmatter {
        upsert_frontmatter_record(&transaction, frontmatter)?;
    }
    for tag in &record.tags {
        upsert_tag_for_file(&transaction, tag)?;
    }
    for link in &record.links {
        upsert_link_for_file(&transaction, link)?;
    }
    clear_orphan_tags(&transaction)?;

    transaction.commit()?;

    Ok(IncrementalMarkdownIndexReport {
        database_path: None,
        indexed_markdown_files: 1,
        indexed_tags: record.tags.len(),
        indexed_links: record.links.len(),
    })
}

pub fn move_markdown_file_index_record(
    database_path: &Path,
    previous_path: &Path,
    record: &MarkdownFileIndexRecord,
) -> Result<IncrementalMarkdownIndexReport, SqliteIndexError> {
    apply_sqlite_index_migrations(database_path)?;
    let mut connection = Connection::open(database_path).map_err(|error| {
        SqliteIndexError::Sqlite(format!(
            "failed to open sqlite index database {}: {error}",
            database_path.display()
        ))
    })?;
    let mut report =
        move_markdown_file_index_record_to_connection(&mut connection, previous_path, record)?;
    report.database_path = Some(database_path.to_path_buf());
    Ok(report)
}

pub fn move_markdown_file_index_record_to_connection(
    connection: &mut Connection,
    previous_path: &Path,
    record: &MarkdownFileIndexRecord,
) -> Result<IncrementalMarkdownIndexReport, SqliteIndexError> {
    connection.execute_batch("PRAGMA foreign_keys = ON;")?;
    let transaction = connection.transaction()?;

    clear_markdown_file_index_state(&transaction, &record.path)?;
    upsert_markdown_file_record(&transaction, record)?;
    if let Some(frontmatter) = &record.frontmatter {
        upsert_frontmatter_record(&transaction, frontmatter)?;
    }
    for tag in &record.tags {
        upsert_tag_for_file(&transaction, tag)?;
    }
    for link in &record.links {
        upsert_link_for_file(&transaction, link)?;
    }
    if previous_path != record.path {
        update_markdown_link_targets(&transaction, previous_path, &record.path)?;
        delete_markdown_file_index_state(&transaction, previous_path)?;
    }
    clear_orphan_tags(&transaction)?;

    transaction.commit()?;

    Ok(IncrementalMarkdownIndexReport {
        database_path: None,
        indexed_markdown_files: 1,
        indexed_tags: record.tags.len(),
        indexed_links: record.links.len(),
    })
}

pub fn remove_markdown_file_index_record(
    database_path: &Path,
    path: &Path,
) -> Result<MarkdownFileIndexRemovalReport, SqliteIndexError> {
    apply_sqlite_index_migrations(database_path)?;
    let mut connection = Connection::open(database_path).map_err(|error| {
        SqliteIndexError::Sqlite(format!(
            "failed to open sqlite index database {}: {error}",
            database_path.display()
        ))
    })?;
    let mut report = remove_markdown_file_index_record_to_connection(&mut connection, path)?;
    report.database_path = Some(database_path.to_path_buf());
    Ok(report)
}

pub fn markdown_file_index_lookup(
    database_path: &Path,
    path: &Path,
) -> Result<Option<MarkdownFileIndexLookup>, SqliteIndexError> {
    apply_sqlite_index_migrations(database_path)?;
    let connection = Connection::open(database_path).map_err(|error| {
        SqliteIndexError::Sqlite(format!(
            "failed to open sqlite index database {}: {error}",
            database_path.display()
        ))
    })?;
    markdown_file_index_lookup_from_connection(&connection, path)
}

pub fn markdown_file_index_lookup_from_connection(
    connection: &Connection,
    path: &Path,
) -> Result<Option<MarkdownFileIndexLookup>, SqliteIndexError> {
    let Some(metadata) = markdown_file_metadata_record_from_connection(connection, path)? else {
        return Ok(None);
    };
    let frontmatter = markdown_file_frontmatter_record_from_connection(connection, path)?;
    let tags = markdown_file_tag_records_from_connection(connection, path)?;
    let links = markdown_file_link_records_from_connection(connection, path)?;
    let backlinks = markdown_file_backlink_records_from_connection(connection, path)?;

    Ok(Some(MarkdownFileIndexLookup {
        metadata,
        frontmatter,
        tags,
        links,
        backlinks,
    }))
}

pub fn markdown_file_metadata_record(
    database_path: &Path,
    path: &Path,
) -> Result<Option<MarkdownFileMetadataRecord>, SqliteIndexError> {
    apply_sqlite_index_migrations(database_path)?;
    let connection = Connection::open(database_path).map_err(|error| {
        SqliteIndexError::Sqlite(format!(
            "failed to open sqlite index database {}: {error}",
            database_path.display()
        ))
    })?;
    markdown_file_metadata_record_from_connection(&connection, path)
}

pub fn markdown_file_metadata_record_from_connection(
    connection: &Connection,
    path: &Path,
) -> Result<Option<MarkdownFileMetadataRecord>, SqliteIndexError> {
    connection
        .query_row(
            r#"
            SELECT
                path,
                context_id,
                title,
                vault_scope,
                relative_path,
                folder_path,
                file_name,
                classification,
                import_classification_suggestion,
                inferred_classification,
                llm_classification_status,
                file_created_at_unix_seconds,
                file_modified_at_unix_seconds,
                indexed_at_unix_seconds,
                content_hash,
                indexing_status,
                last_index_error,
                import_source,
                import_source_type
            FROM markdown_files
            WHERE path = ?1;
            "#,
            params![path_sql_value(path)],
            markdown_file_metadata_from_row,
        )
        .optional()
        .map_err(SqliteIndexError::from)
}

pub fn markdown_file_tag_records(
    database_path: &Path,
    path: &Path,
) -> Result<Vec<MarkdownFileTagRecord>, SqliteIndexError> {
    apply_sqlite_index_migrations(database_path)?;
    let connection = Connection::open(database_path).map_err(|error| {
        SqliteIndexError::Sqlite(format!(
            "failed to open sqlite index database {}: {error}",
            database_path.display()
        ))
    })?;
    markdown_file_tag_records_from_connection(&connection, path)
}

pub fn markdown_file_tag_records_from_connection(
    connection: &Connection,
    path: &Path,
) -> Result<Vec<MarkdownFileTagRecord>, SqliteIndexError> {
    let mut statement = connection.prepare(
        r#"
        SELECT path, tag_id, tag_source, tag_position, indexed_at_unix_seconds
        FROM markdown_file_tags
        WHERE path = ?1
        ORDER BY tag_position, tag_id;
        "#,
    )?;
    let records = statement
        .query_map(params![path_sql_value(path)], markdown_file_tag_from_row)?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(records)
}

pub fn markdown_file_link_records(
    database_path: &Path,
    path: &Path,
) -> Result<Vec<MarkdownFileLinkRecord>, SqliteIndexError> {
    apply_sqlite_index_migrations(database_path)?;
    let connection = Connection::open(database_path).map_err(|error| {
        SqliteIndexError::Sqlite(format!(
            "failed to open sqlite index database {}: {error}",
            database_path.display()
        ))
    })?;
    markdown_file_link_records_from_connection(&connection, path)
}

pub fn markdown_file_link_records_from_connection(
    connection: &Connection,
    path: &Path,
) -> Result<Vec<MarkdownFileLinkRecord>, SqliteIndexError> {
    let mut statement = connection.prepare(
        r#"
        SELECT
            link_id,
            source_path,
            link_kind,
            raw_target,
            normalized_target,
            link_text,
            target_path,
            target_anchor,
            target_url,
            resolved_status,
            byte_start,
            byte_end,
            line_number,
            link_position,
            indexed_at_unix_seconds
        FROM markdown_file_links
        WHERE source_path = ?1
        ORDER BY link_position, link_id;
        "#,
    )?;
    let records = statement
        .query_map(params![path_sql_value(path)], markdown_file_link_from_row)?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(records)
}

pub fn markdown_file_metadata_records_by_tag(
    database_path: &Path,
    tag: &str,
) -> Result<Vec<MarkdownFileMetadataRecord>, SqliteIndexError> {
    apply_sqlite_index_migrations(database_path)?;
    let connection = Connection::open(database_path).map_err(|error| {
        SqliteIndexError::Sqlite(format!(
            "failed to open sqlite index database {}: {error}",
            database_path.display()
        ))
    })?;
    markdown_file_metadata_records_by_tag_from_connection(&connection, tag)
}

pub fn markdown_file_metadata_records_by_tag_from_connection(
    connection: &Connection,
    tag: &str,
) -> Result<Vec<MarkdownFileMetadataRecord>, SqliteIndexError> {
    let normalized = normalize_tag_id(tag);
    if normalized.is_empty() {
        return Ok(Vec::new());
    }

    let mut statement = connection.prepare(
        r#"
        SELECT
            markdown_files.path,
            markdown_files.context_id,
            markdown_files.title,
            markdown_files.vault_scope,
            markdown_files.relative_path,
            markdown_files.folder_path,
            markdown_files.file_name,
            markdown_files.classification,
            markdown_files.import_classification_suggestion,
            markdown_files.inferred_classification,
            markdown_files.llm_classification_status,
            markdown_files.file_created_at_unix_seconds,
            markdown_files.file_modified_at_unix_seconds,
            markdown_files.indexed_at_unix_seconds,
            markdown_files.content_hash,
            markdown_files.indexing_status,
            markdown_files.last_index_error,
            markdown_files.import_source,
            markdown_files.import_source_type
        FROM markdown_file_tags
        JOIN markdown_files ON markdown_file_tags.path = markdown_files.path
        WHERE markdown_file_tags.tag_id = ?1
        ORDER BY markdown_files.vault_scope, markdown_files.relative_path;
        "#,
    )?;
    let records = statement
        .query_map(params![normalized], markdown_file_metadata_from_row)?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(records)
}

pub fn search_markdown_file_index(
    database_path: &Path,
    query: &str,
) -> Result<Vec<MarkdownFileSearchResult>, SqliteIndexError> {
    apply_sqlite_index_migrations(database_path)?;
    let connection = Connection::open(database_path).map_err(|error| {
        SqliteIndexError::Sqlite(format!(
            "failed to open sqlite index database {}: {error}",
            database_path.display()
        ))
    })?;
    search_markdown_file_index_from_connection(&connection, query)
}

pub fn search_markdown_file_index_from_connection(
    connection: &Connection,
    query: &str,
) -> Result<Vec<MarkdownFileSearchResult>, SqliteIndexError> {
    let trimmed_query = query.trim();
    if trimmed_query.is_empty() {
        return Ok(Vec::new());
    }

    let mut statement = connection.prepare(
        r#"
        SELECT path, title, relative_path
        FROM markdown_file_search
        WHERE markdown_file_search MATCH ?1
        ORDER BY rank, relative_path;
        "#,
    )?;
    let records = statement
        .query_map(params![trimmed_query], |row| {
            Ok(MarkdownFileSearchResult {
                path: PathBuf::from(row.get::<_, String>(0)?),
                title: row.get(1)?,
                relative_path: PathBuf::from(row.get::<_, String>(2)?),
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(records)
}

pub fn markdown_file_backlink_records(
    database_path: &Path,
    path: &Path,
) -> Result<Vec<MarkdownFileBacklinkRecord>, SqliteIndexError> {
    apply_sqlite_index_migrations(database_path)?;
    let connection = Connection::open(database_path).map_err(|error| {
        SqliteIndexError::Sqlite(format!(
            "failed to open sqlite index database {}: {error}",
            database_path.display()
        ))
    })?;
    markdown_file_backlink_records_from_connection(&connection, path)
}

pub fn markdown_file_backlink_records_from_connection(
    connection: &Connection,
    path: &Path,
) -> Result<Vec<MarkdownFileBacklinkRecord>, SqliteIndexError> {
    let mut statement = connection.prepare(
        r#"
        SELECT
            path,
            backlink_source_path,
            link_id,
            link_kind,
            raw_target,
            normalized_target,
            link_text,
            target_anchor,
            byte_start,
            byte_end,
            line_number,
            link_position,
            indexed_at_unix_seconds
        FROM markdown_file_backlinks
        WHERE path = ?1
        ORDER BY backlink_source_path, link_position, link_id;
        "#,
    )?;
    let backlinks = statement
        .query_map(params![path_sql_value(path)], |row| {
            let link_kind = markdown_file_link_kind_from_sql_value(&row.get::<_, String>(3)?)?;
            Ok(MarkdownFileBacklinkRecord {
                path: PathBuf::from(row.get::<_, String>(0)?),
                backlink_source_path: PathBuf::from(row.get::<_, String>(1)?),
                link_id: row.get(2)?,
                link_kind,
                raw_target: row.get(4)?,
                normalized_target: row.get(5)?,
                link_text: row.get(6)?,
                target_anchor: row.get(7)?,
                byte_start: row.get(8)?,
                byte_end: row.get(9)?,
                line_number: row.get(10)?,
                link_position: row.get(11)?,
                indexed_at_unix_seconds: row.get(12)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;

    Ok(backlinks)
}

pub fn remove_markdown_file_index_record_to_connection(
    connection: &mut Connection,
    path: &Path,
) -> Result<MarkdownFileIndexRemovalReport, SqliteIndexError> {
    connection.execute_batch("PRAGMA foreign_keys = ON;")?;
    let transaction = connection.transaction()?;

    let updated_link_targets = clear_markdown_link_targets(&transaction, path)?;
    let removed_markdown_files = delete_markdown_file_index_state(&transaction, path)?;
    clear_orphan_tags(&transaction)?;

    transaction.commit()?;

    Ok(MarkdownFileIndexRemovalReport {
        database_path: None,
        removed_markdown_files,
        updated_link_targets,
    })
}

fn clear_markdown_index_state(transaction: &Transaction<'_>) -> Result<usize, SqliteIndexError> {
    let cleared_markdown_files =
        transaction.query_row("SELECT COUNT(*) FROM markdown_files;", [], |row| {
            row.get::<_, usize>(0)
        })?;
    transaction.execute("DELETE FROM markdown_file_search;", [])?;
    transaction.execute("DELETE FROM markdown_file_links;", [])?;
    transaction.execute("DELETE FROM markdown_file_tags;", [])?;
    transaction.execute("DELETE FROM tags;", [])?;
    transaction.execute("DELETE FROM markdown_file_frontmatter;", [])?;
    transaction.execute("DELETE FROM markdown_files;", [])?;
    Ok(cleared_markdown_files)
}

fn clear_markdown_file_index_state(
    transaction: &Transaction<'_>,
    path: &Path,
) -> Result<(), SqliteIndexError> {
    transaction.execute(
        "DELETE FROM markdown_file_search WHERE path = ?1;",
        params![path_sql_value(path)],
    )?;
    transaction.execute(
        "DELETE FROM markdown_file_links WHERE source_path = ?1;",
        params![path_sql_value(path)],
    )?;
    transaction.execute(
        "DELETE FROM markdown_file_tags WHERE path = ?1;",
        params![path_sql_value(path)],
    )?;
    transaction.execute(
        "DELETE FROM markdown_file_frontmatter WHERE path = ?1;",
        params![path_sql_value(path)],
    )?;
    Ok(())
}

fn delete_markdown_file_index_state(
    transaction: &Transaction<'_>,
    path: &Path,
) -> Result<usize, SqliteIndexError> {
    clear_markdown_file_index_state(transaction, path)?;
    let removed = transaction.execute(
        "DELETE FROM markdown_files WHERE path = ?1;",
        params![path_sql_value(path)],
    )?;
    Ok(removed)
}

fn update_markdown_link_targets(
    transaction: &Transaction<'_>,
    previous_path: &Path,
    new_path: &Path,
) -> Result<usize, SqliteIndexError> {
    let updated = transaction.execute(
        "UPDATE markdown_file_links SET target_path = ?1 WHERE target_path = ?2;",
        params![path_sql_value(new_path), path_sql_value(previous_path)],
    )?;
    Ok(updated)
}

fn clear_markdown_link_targets(
    transaction: &Transaction<'_>,
    path: &Path,
) -> Result<usize, SqliteIndexError> {
    let updated = transaction.execute(
        r#"
        UPDATE markdown_file_links
        SET target_path = NULL,
            resolved_status = 'unresolved'
        WHERE target_path = ?1;
        "#,
        params![path_sql_value(path)],
    )?;
    Ok(updated)
}

fn clear_orphan_tags(transaction: &Transaction<'_>) -> Result<(), SqliteIndexError> {
    transaction.execute(
        r#"
        DELETE FROM tags
        WHERE tag_id NOT IN (
            SELECT DISTINCT tag_id FROM markdown_file_tags
        );
        "#,
        [],
    )?;
    Ok(())
}

fn upsert_markdown_file_record(
    transaction: &Transaction<'_>,
    record: &MarkdownFileIndexRecord,
) -> Result<(), SqliteIndexError> {
    transaction.execute(
        r#"
        INSERT INTO markdown_files (
            path,
            context_id,
            title,
            vault_scope,
            relative_path,
            folder_path,
            file_name,
            classification,
            import_classification_suggestion,
            inferred_classification,
            llm_classification_status,
            file_created_at_unix_seconds,
            file_modified_at_unix_seconds,
            indexed_at_unix_seconds,
            content_hash,
            indexing_status,
            last_index_error,
            import_source,
            import_source_type
        ) VALUES (
            ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10,
            ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19
        )
        ON CONFLICT(path) DO UPDATE SET
            context_id = excluded.context_id,
            title = excluded.title,
            vault_scope = excluded.vault_scope,
            relative_path = excluded.relative_path,
            folder_path = excluded.folder_path,
            file_name = excluded.file_name,
            classification = excluded.classification,
            import_classification_suggestion = excluded.import_classification_suggestion,
            inferred_classification = excluded.inferred_classification,
            llm_classification_status = excluded.llm_classification_status,
            file_created_at_unix_seconds = excluded.file_created_at_unix_seconds,
            file_modified_at_unix_seconds = excluded.file_modified_at_unix_seconds,
            indexed_at_unix_seconds = excluded.indexed_at_unix_seconds,
            content_hash = excluded.content_hash,
            indexing_status = excluded.indexing_status,
            last_index_error = excluded.last_index_error,
            import_source = excluded.import_source,
            import_source_type = excluded.import_source_type;
        "#,
        params![
            path_sql_value(&record.path),
            &record.context_id,
            &record.title,
            vault_scope_sql_value(record.vault_scope),
            path_sql_value(&record.relative_path),
            path_sql_value(&record.folder_path),
            &record.file_name,
            classification_sql_value(record.classification),
            record
                .import_classification_suggestion
                .map(classification_sql_value),
            record.inferred_classification.map(classification_sql_value),
            classification_status_sql_value(record.llm_classification_status),
            record.file_created_at_unix_seconds,
            record.file_modified_at_unix_seconds,
            record.indexed_at_unix_seconds,
            &record.content_hash,
            record.indexing_status.as_sql_value(),
            record.last_index_error.as_deref(),
            record
                .import_source
                .as_ref()
                .map(|path| path_sql_value(path)),
            record.import_source_type.map(import_source_type_sql_value),
        ],
    )?;
    transaction.execute(
        r#"
        INSERT INTO markdown_file_search (path, title, relative_path, content)
        VALUES (?1, ?2, ?3, ?4);
        "#,
        params![
            path_sql_value(&record.path),
            &record.title,
            path_sql_value(&record.relative_path),
            &record.content,
        ],
    )?;
    Ok(())
}

fn upsert_frontmatter_record(
    transaction: &Transaction<'_>,
    frontmatter: &ParsedFrontmatterMetadata,
) -> Result<(), SqliteIndexError> {
    transaction.execute(
        r#"
        INSERT INTO markdown_file_frontmatter (
            path,
            frontmatter_format,
            frontmatter_raw,
            frontmatter_json,
            frontmatter_title,
            frontmatter_tags_json,
            frontmatter_classification,
            parse_status,
            parse_error,
            parsed_at_unix_seconds
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
        ON CONFLICT(path) DO UPDATE SET
            frontmatter_format = excluded.frontmatter_format,
            frontmatter_raw = excluded.frontmatter_raw,
            frontmatter_json = excluded.frontmatter_json,
            frontmatter_title = excluded.frontmatter_title,
            frontmatter_tags_json = excluded.frontmatter_tags_json,
            frontmatter_classification = excluded.frontmatter_classification,
            parse_status = excluded.parse_status,
            parse_error = excluded.parse_error,
            parsed_at_unix_seconds = excluded.parsed_at_unix_seconds;
        "#,
        params![
            path_sql_value(&frontmatter.path),
            frontmatter.frontmatter_format.as_sql_value(),
            frontmatter.frontmatter_raw.as_deref(),
            &frontmatter.frontmatter_json,
            frontmatter.frontmatter_title.as_deref(),
            serde_json::to_string(&frontmatter.frontmatter_tags).map_err(|error| {
                SqliteIndexError::Sqlite(format!("failed to serialize frontmatter tags: {error}"))
            })?,
            frontmatter
                .frontmatter_classification
                .map(classification_sql_value),
            frontmatter.parse_status.as_sql_value(),
            frontmatter.parse_error,
            frontmatter.parsed_at_unix_seconds,
        ],
    )?;
    Ok(())
}

fn upsert_tag_for_file(
    transaction: &Transaction<'_>,
    tag: &MarkdownFileTagRecord,
) -> Result<(), SqliteIndexError> {
    let normalized_name = normalize_tag_id(&tag.tag_id);
    if normalized_name.is_empty() {
        return Ok(());
    }
    transaction.execute(
        r#"
        INSERT INTO tags (tag_id, display_name, normalized_name, created_at_unix_seconds)
        VALUES (?1, ?2, ?3, ?4)
        ON CONFLICT(normalized_name) DO UPDATE SET
            display_name = excluded.display_name;
        "#,
        params![
            &normalized_name,
            tag.tag_id.trim().trim_start_matches('#'),
            &normalized_name,
            tag.indexed_at_unix_seconds,
        ],
    )?;
    transaction.execute(
        r#"
        INSERT INTO markdown_file_tags (
            path,
            tag_id,
            tag_source,
            tag_position,
            indexed_at_unix_seconds
        ) VALUES (?1, ?2, ?3, ?4, ?5)
        ON CONFLICT(path, tag_id) DO UPDATE SET
            tag_source = excluded.tag_source,
            tag_position = excluded.tag_position,
            indexed_at_unix_seconds = excluded.indexed_at_unix_seconds;
        "#,
        params![
            path_sql_value(&tag.path),
            &normalized_name,
            tag.tag_source.as_sql_value(),
            tag.tag_position,
            tag.indexed_at_unix_seconds,
        ],
    )?;
    Ok(())
}

fn upsert_link_for_file(
    transaction: &Transaction<'_>,
    link: &MarkdownFileLinkRecord,
) -> Result<(), SqliteIndexError> {
    transaction.execute(
        r#"
        INSERT INTO markdown_file_links (
            link_id,
            source_path,
            link_kind,
            raw_target,
            normalized_target,
            link_text,
            target_path,
            target_anchor,
            target_url,
            resolved_status,
            byte_start,
            byte_end,
            line_number,
            link_position,
            indexed_at_unix_seconds
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)
        ON CONFLICT(link_id) DO UPDATE SET
            source_path = excluded.source_path,
            link_kind = excluded.link_kind,
            raw_target = excluded.raw_target,
            normalized_target = excluded.normalized_target,
            link_text = excluded.link_text,
            target_path = excluded.target_path,
            target_anchor = excluded.target_anchor,
            target_url = excluded.target_url,
            resolved_status = excluded.resolved_status,
            byte_start = excluded.byte_start,
            byte_end = excluded.byte_end,
            line_number = excluded.line_number,
            link_position = excluded.link_position,
            indexed_at_unix_seconds = excluded.indexed_at_unix_seconds;
        "#,
        params![
            &link.link_id,
            path_sql_value(&link.source_path),
            link.link_kind.as_sql_value(),
            &link.raw_target,
            &link.normalized_target,
            link.link_text.as_deref(),
            link.target_path.as_ref().map(|path| path_sql_value(path)),
            link.target_anchor.as_deref(),
            link.target_url.as_deref(),
            link.resolved_status.as_sql_value(),
            link.byte_start,
            link.byte_end,
            link.line_number,
            link.link_position,
            link.indexed_at_unix_seconds,
        ],
    )?;
    Ok(())
}

fn markdown_file_metadata_from_row(
    row: &rusqlite::Row<'_>,
) -> Result<MarkdownFileMetadataRecord, rusqlite::Error> {
    Ok(MarkdownFileMetadataRecord {
        path: PathBuf::from(row.get::<_, String>(0)?),
        context_id: row.get(1)?,
        title: row.get(2)?,
        vault_scope: vault_scope_from_sql_value(&row.get::<_, String>(3)?)?,
        relative_path: PathBuf::from(row.get::<_, String>(4)?),
        folder_path: PathBuf::from(row.get::<_, String>(5)?),
        file_name: row.get(6)?,
        classification: classification_from_sql_value(&row.get::<_, String>(7)?)?,
        import_classification_suggestion: optional_classification_from_sql_value(row.get(8)?)?,
        inferred_classification: optional_classification_from_sql_value(row.get(9)?)?,
        llm_classification_status: classification_status_from_sql_value(
            &row.get::<_, String>(10)?,
        )?,
        file_created_at_unix_seconds: row.get(11)?,
        file_modified_at_unix_seconds: row.get(12)?,
        indexed_at_unix_seconds: row.get(13)?,
        content_hash: row.get(14)?,
        indexing_status: markdown_file_indexing_status_from_sql_value(&row.get::<_, String>(15)?)?,
        last_index_error: row.get(16)?,
        import_source: row.get::<_, Option<String>>(17)?.map(PathBuf::from),
        import_source_type: optional_import_source_type_from_sql_value(row.get(18)?)?,
    })
}

fn markdown_file_frontmatter_record_from_connection(
    connection: &Connection,
    path: &Path,
) -> Result<Option<ParsedFrontmatterMetadata>, SqliteIndexError> {
    connection
        .query_row(
            r#"
            SELECT
                path,
                frontmatter_format,
                frontmatter_raw,
                frontmatter_json,
                frontmatter_title,
                frontmatter_tags_json,
                frontmatter_classification,
                parse_status,
                parse_error,
                parsed_at_unix_seconds
            FROM markdown_file_frontmatter
            WHERE path = ?1;
            "#,
            params![path_sql_value(path)],
            |row| {
                let tags_json: String = row.get(5)?;
                let frontmatter_tags =
                    serde_json::from_str(&tags_json).map_err(|_| rusqlite::Error::InvalidQuery)?;
                Ok(ParsedFrontmatterMetadata {
                    path: PathBuf::from(row.get::<_, String>(0)?),
                    frontmatter_format: frontmatter_format_from_sql_value(
                        &row.get::<_, String>(1)?,
                    )?,
                    frontmatter_raw: row.get(2)?,
                    frontmatter_json: row.get(3)?,
                    frontmatter_title: row.get(4)?,
                    frontmatter_tags,
                    frontmatter_classification: optional_classification_from_sql_value(
                        row.get(6)?,
                    )?,
                    parse_status: frontmatter_parse_status_from_sql_value(
                        &row.get::<_, String>(7)?,
                    )?,
                    parse_error: row.get(8)?,
                    parsed_at_unix_seconds: row.get(9)?,
                })
            },
        )
        .optional()
        .map_err(SqliteIndexError::from)
}

fn markdown_file_tag_from_row(
    row: &rusqlite::Row<'_>,
) -> Result<MarkdownFileTagRecord, rusqlite::Error> {
    Ok(MarkdownFileTagRecord {
        path: PathBuf::from(row.get::<_, String>(0)?),
        tag_id: row.get(1)?,
        tag_source: markdown_file_tag_source_from_sql_value(&row.get::<_, String>(2)?)?,
        tag_position: row.get(3)?,
        indexed_at_unix_seconds: row.get(4)?,
    })
}

fn markdown_file_link_from_row(
    row: &rusqlite::Row<'_>,
) -> Result<MarkdownFileLinkRecord, rusqlite::Error> {
    Ok(MarkdownFileLinkRecord {
        link_id: row.get(0)?,
        source_path: PathBuf::from(row.get::<_, String>(1)?),
        link_kind: markdown_file_link_kind_from_sql_value(&row.get::<_, String>(2)?)?,
        raw_target: row.get(3)?,
        normalized_target: row.get(4)?,
        link_text: row.get(5)?,
        target_path: row.get::<_, Option<String>>(6)?.map(PathBuf::from),
        target_anchor: row.get(7)?,
        target_url: row.get(8)?,
        resolved_status: markdown_file_link_resolved_status_from_sql_value(
            &row.get::<_, String>(9)?,
        )?,
        byte_start: row.get(10)?,
        byte_end: row.get(11)?,
        line_number: row.get(12)?,
        link_position: row.get(13)?,
        indexed_at_unix_seconds: row.get(14)?,
    })
}

fn path_sql_value(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn vault_scope_from_sql_value(value: &str) -> Result<VaultScope, rusqlite::Error> {
    match value {
        "global" => Ok(VaultScope::Global),
        "local" => Ok(VaultScope::Local),
        _ => Err(rusqlite::Error::InvalidQuery),
    }
}

fn classification_from_sql_value(value: &str) -> Result<Classification, rusqlite::Error> {
    match value {
        "main-agent" => Ok(Classification::MainAgent),
        "subagent" => Ok(Classification::Subagent),
        "shared" => Ok(Classification::Shared),
        _ => Err(rusqlite::Error::InvalidQuery),
    }
}

fn optional_classification_from_sql_value(
    value: Option<String>,
) -> Result<Option<Classification>, rusqlite::Error> {
    value
        .as_deref()
        .map(classification_from_sql_value)
        .transpose()
}

fn classification_status_from_sql_value(
    value: &str,
) -> Result<crate::ClassificationStatus, rusqlite::Error> {
    match value {
        "pending" => Ok(crate::ClassificationStatus::Pending),
        "classified" => Ok(crate::ClassificationStatus::Classified),
        "reviewed" => Ok(crate::ClassificationStatus::Reviewed),
        "modified" => Ok(crate::ClassificationStatus::Modified),
        _ => Err(rusqlite::Error::InvalidQuery),
    }
}

fn import_source_type_from_sql_value(
    value: &str,
) -> Result<crate::ImportSourceType, rusqlite::Error> {
    match value {
        "context-markdown" => Ok(crate::ImportSourceType::ContextMarkdown),
        "claude-markdown" => Ok(crate::ImportSourceType::ClaudeMarkdown),
        "codex-agents" => Ok(crate::ImportSourceType::CodexAgents),
        "agent-markdown" => Ok(crate::ImportSourceType::AgentMarkdown),
        "agents-manifest" => Ok(crate::ImportSourceType::AgentsManifest),
        "skill-markdown" => Ok(crate::ImportSourceType::SkillMarkdown),
        "skill-manifest" => Ok(crate::ImportSourceType::SkillManifest),
        "subagent-markdown" => Ok(crate::ImportSourceType::SubagentMarkdown),
        _ => Err(rusqlite::Error::InvalidQuery),
    }
}

fn optional_import_source_type_from_sql_value(
    value: Option<String>,
) -> Result<Option<crate::ImportSourceType>, rusqlite::Error> {
    value
        .as_deref()
        .map(import_source_type_from_sql_value)
        .transpose()
}

fn markdown_file_indexing_status_from_sql_value(
    value: &str,
) -> Result<MarkdownFileIndexingStatus, rusqlite::Error> {
    match value {
        "pending" => Ok(MarkdownFileIndexingStatus::Pending),
        "indexed" => Ok(MarkdownFileIndexingStatus::Indexed),
        "stale" => Ok(MarkdownFileIndexingStatus::Stale),
        "failed" => Ok(MarkdownFileIndexingStatus::Failed),
        _ => Err(rusqlite::Error::InvalidQuery),
    }
}

fn frontmatter_format_from_sql_value(value: &str) -> Result<FrontmatterFormat, rusqlite::Error> {
    match value {
        "none" => Ok(FrontmatterFormat::None),
        "yaml" => Ok(FrontmatterFormat::Yaml),
        "toml" => Ok(FrontmatterFormat::Toml),
        "json" => Ok(FrontmatterFormat::Json),
        "unknown" => Ok(FrontmatterFormat::Unknown),
        _ => Err(rusqlite::Error::InvalidQuery),
    }
}

fn frontmatter_parse_status_from_sql_value(
    value: &str,
) -> Result<FrontmatterParseStatus, rusqlite::Error> {
    match value {
        "absent" => Ok(FrontmatterParseStatus::Absent),
        "parsed" => Ok(FrontmatterParseStatus::Parsed),
        "unsupported" => Ok(FrontmatterParseStatus::Unsupported),
        "failed" => Ok(FrontmatterParseStatus::Failed),
        _ => Err(rusqlite::Error::InvalidQuery),
    }
}

fn markdown_file_tag_source_from_sql_value(
    value: &str,
) -> Result<MarkdownFileTagSource, rusqlite::Error> {
    match value {
        "frontmatter" => Ok(MarkdownFileTagSource::Frontmatter),
        "body" => Ok(MarkdownFileTagSource::Body),
        "llm-classification" => Ok(MarkdownFileTagSource::LlmClassification),
        "manual" => Ok(MarkdownFileTagSource::Manual),
        "import" => Ok(MarkdownFileTagSource::Import),
        _ => Err(rusqlite::Error::InvalidQuery),
    }
}

fn markdown_file_link_kind_from_sql_value(
    value: &str,
) -> Result<MarkdownFileLinkKind, rusqlite::Error> {
    match value {
        "wikilink" => Ok(MarkdownFileLinkKind::Wikilink),
        "markdown" => Ok(MarkdownFileLinkKind::Markdown),
        _ => Err(rusqlite::Error::InvalidQuery),
    }
}

fn markdown_file_link_resolved_status_from_sql_value(
    value: &str,
) -> Result<MarkdownFileLinkResolvedStatus, rusqlite::Error> {
    match value {
        "unresolved" => Ok(MarkdownFileLinkResolvedStatus::Unresolved),
        "resolved" => Ok(MarkdownFileLinkResolvedStatus::Resolved),
        "external" => Ok(MarkdownFileLinkResolvedStatus::External),
        "ambiguous" => Ok(MarkdownFileLinkResolvedStatus::Ambiguous),
        _ => Err(rusqlite::Error::InvalidQuery),
    }
}

fn normalize_tag_id(tag: &str) -> String {
    tag.trim()
        .trim_start_matches('#')
        .to_ascii_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join("-")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};

    #[test]
    fn markdown_files_table_captures_required_index_fields() {
        let schema = CREATE_MARKDOWN_FILES_TABLE_SQL;

        for field in [
            "path TEXT PRIMARY KEY NOT NULL",
            "context_id TEXT NOT NULL",
            "title TEXT NOT NULL",
            "folder_path TEXT NOT NULL DEFAULT ''",
            "classification TEXT NOT NULL",
            "import_classification_suggestion TEXT",
            "inferred_classification TEXT",
            "llm_classification_status TEXT NOT NULL",
            "file_created_at_unix_seconds INTEGER NOT NULL",
            "file_modified_at_unix_seconds INTEGER NOT NULL",
            "indexed_at_unix_seconds INTEGER",
            "content_hash TEXT NOT NULL",
            "indexing_status TEXT NOT NULL",
            "import_source_type TEXT",
        ] {
            assert!(
                schema.contains(field),
                "markdown_files schema should include {field}"
            );
        }
    }

    #[test]
    fn markdown_files_schema_includes_status_and_lookup_indexes() {
        assert!(SQLITE_INDEX_SCHEMA_STATEMENTS.contains(&CREATE_MARKDOWN_FILES_TABLE_SQL));
        assert!(
            SQLITE_INDEX_SCHEMA_STATEMENTS.contains(&CREATE_MARKDOWN_FILE_FRONTMATTER_TABLE_SQL)
        );
        assert!(SQLITE_INDEX_SCHEMA_STATEMENTS.contains(&CREATE_TAGS_TABLE_SQL));
        assert!(SQLITE_INDEX_SCHEMA_STATEMENTS.contains(&CREATE_MARKDOWN_FILE_TAGS_TABLE_SQL));
        assert!(SQLITE_INDEX_SCHEMA_STATEMENTS.contains(&CREATE_MARKDOWN_FILE_LINKS_TABLE_SQL));
        assert!(SQLITE_INDEX_SCHEMA_STATEMENTS.contains(&CREATE_MARKDOWN_FILE_BACKLINKS_VIEW_SQL));
        assert!(CREATE_MARKDOWN_FILES_INDEXING_STATUS_INDEX_SQL
            .contains("ON markdown_files (indexing_status)"));
        assert!(CREATE_MARKDOWN_FILES_CONTENT_HASH_INDEX_SQL
            .contains("ON markdown_files (content_hash)"));
        assert!(CREATE_MARKDOWN_FILES_VAULT_SCOPE_INDEX_SQL
            .contains("ON markdown_files (vault_scope, relative_path)"));
        assert!(CREATE_MARKDOWN_FILE_FRONTMATTER_PARSE_STATUS_INDEX_SQL
            .contains("ON markdown_file_frontmatter (parse_status)"));
        assert!(CREATE_MARKDOWN_FILE_FRONTMATTER_TITLE_INDEX_SQL
            .contains("ON markdown_file_frontmatter (frontmatter_title)"));
        assert!(
            CREATE_MARKDOWN_FILE_TAGS_TAG_ID_INDEX_SQL.contains("ON markdown_file_tags (tag_id)")
        );
        assert!(CREATE_MARKDOWN_FILE_TAGS_SOURCE_INDEX_SQL
            .contains("ON markdown_file_tags (tag_source)"));
        assert!(CREATE_MARKDOWN_FILE_TAGS_PATH_POSITION_INDEX_SQL
            .contains("ON markdown_file_tags (path, tag_position)"));
        assert!(CREATE_MARKDOWN_FILE_LINKS_SOURCE_INDEX_SQL
            .contains("ON markdown_file_links (source_path, link_position)"));
        assert!(CREATE_MARKDOWN_FILE_LINKS_TARGET_INDEX_SQL
            .contains("ON markdown_file_links (target_path)"));
        assert!(CREATE_MARKDOWN_FILE_LINKS_NORMALIZED_TARGET_INDEX_SQL
            .contains("ON markdown_file_links (normalized_target)"));
        assert!(CREATE_MARKDOWN_FILE_LINKS_KIND_STATUS_INDEX_SQL
            .contains("ON markdown_file_links (link_kind, resolved_status)"));
    }

    #[test]
    fn indexing_status_values_match_sql_check_constraint() {
        let schema = CREATE_MARKDOWN_FILES_TABLE_SQL;

        for status in [
            MarkdownFileIndexingStatus::Pending,
            MarkdownFileIndexingStatus::Indexed,
            MarkdownFileIndexingStatus::Stale,
            MarkdownFileIndexingStatus::Failed,
        ] {
            assert!(schema.contains(status.as_sql_value()));
        }
    }

    #[test]
    fn markdown_files_metadata_values_match_sql_check_constraints() {
        let schema = CREATE_MARKDOWN_FILES_TABLE_SQL;

        for classification in [
            Classification::MainAgent,
            Classification::Subagent,
            Classification::Shared,
        ] {
            assert!(schema.contains(classification_sql_value(classification)));
        }

        for status in [
            crate::ClassificationStatus::Pending,
            crate::ClassificationStatus::Classified,
            crate::ClassificationStatus::Reviewed,
            crate::ClassificationStatus::Modified,
        ] {
            assert!(schema.contains(classification_status_sql_value(status)));
        }
    }

    #[test]
    fn frontmatter_schema_is_keyed_to_indexed_markdown_files() {
        let schema = CREATE_MARKDOWN_FILE_FRONTMATTER_TABLE_SQL;

        for field in [
            "path TEXT PRIMARY KEY NOT NULL REFERENCES markdown_files(path) ON DELETE CASCADE",
            "frontmatter_format TEXT NOT NULL",
            "frontmatter_raw TEXT",
            "frontmatter_json TEXT NOT NULL DEFAULT '{}'",
            "frontmatter_title TEXT",
            "frontmatter_tags_json TEXT NOT NULL DEFAULT '[]'",
            "frontmatter_classification TEXT",
            "parse_status TEXT NOT NULL",
            "parse_error TEXT",
            "parsed_at_unix_seconds INTEGER",
        ] {
            assert!(
                schema.contains(field),
                "frontmatter schema should include {field}"
            );
        }
    }

    #[test]
    fn frontmatter_enum_values_match_sql_check_constraints() {
        let schema = CREATE_MARKDOWN_FILE_FRONTMATTER_TABLE_SQL;

        for format in [
            FrontmatterFormat::None,
            FrontmatterFormat::Yaml,
            FrontmatterFormat::Toml,
            FrontmatterFormat::Json,
            FrontmatterFormat::Unknown,
        ] {
            assert!(schema.contains(format.as_sql_value()));
        }

        for status in [
            FrontmatterParseStatus::Absent,
            FrontmatterParseStatus::Parsed,
            FrontmatterParseStatus::Unsupported,
            FrontmatterParseStatus::Failed,
        ] {
            assert!(schema.contains(status.as_sql_value()));
        }
    }

    #[test]
    fn tags_table_stores_normalized_unique_tags() {
        let schema = CREATE_TAGS_TABLE_SQL;

        for field in [
            "tag_id TEXT PRIMARY KEY NOT NULL",
            "display_name TEXT NOT NULL",
            "normalized_name TEXT NOT NULL UNIQUE",
            "created_at_unix_seconds INTEGER",
            "CHECK (length(trim(normalized_name)) > 0)",
            "CHECK (normalized_name = lower(normalized_name))",
        ] {
            assert!(schema.contains(field), "tags schema should include {field}");
        }
    }

    #[test]
    fn markdown_file_tags_models_file_to_tag_relationships() {
        let schema = CREATE_MARKDOWN_FILE_TAGS_TABLE_SQL;

        for field in [
            "path TEXT NOT NULL REFERENCES markdown_files(path) ON DELETE CASCADE",
            "tag_id TEXT NOT NULL REFERENCES tags(tag_id) ON DELETE CASCADE",
            "tag_source TEXT NOT NULL",
            "tag_position INTEGER NOT NULL DEFAULT 0",
            "indexed_at_unix_seconds INTEGER",
            "PRIMARY KEY (path, tag_id)",
        ] {
            assert!(
                schema.contains(field),
                "markdown_file_tags schema should include {field}"
            );
        }
    }

    #[test]
    fn markdown_file_tag_source_values_match_sql_check_constraint() {
        let schema = CREATE_MARKDOWN_FILE_TAGS_TABLE_SQL;

        for source in [
            MarkdownFileTagSource::Frontmatter,
            MarkdownFileTagSource::Body,
            MarkdownFileTagSource::LlmClassification,
            MarkdownFileTagSource::Manual,
            MarkdownFileTagSource::Import,
        ] {
            assert!(schema.contains(source.as_sql_value()));
        }
    }

    #[test]
    fn markdown_file_links_store_normalized_outbound_edges() {
        let schema = CREATE_MARKDOWN_FILE_LINKS_TABLE_SQL;

        for field in [
            "link_id TEXT PRIMARY KEY NOT NULL",
            "source_path TEXT NOT NULL REFERENCES markdown_files(path) ON DELETE CASCADE",
            "link_kind TEXT NOT NULL",
            "raw_target TEXT NOT NULL",
            "normalized_target TEXT NOT NULL",
            "link_text TEXT",
            "target_path TEXT REFERENCES markdown_files(path) ON DELETE SET NULL",
            "target_anchor TEXT",
            "target_url TEXT",
            "resolved_status TEXT NOT NULL",
            "byte_start INTEGER",
            "byte_end INTEGER",
            "line_number INTEGER",
            "link_position INTEGER NOT NULL DEFAULT 0",
            "indexed_at_unix_seconds INTEGER",
        ] {
            assert!(
                schema.contains(field),
                "markdown_file_links schema should include {field}"
            );
        }
    }

    #[test]
    fn markdown_file_links_support_wikilinks_backlinks_and_markdown_links() {
        let schema = CREATE_MARKDOWN_FILE_LINKS_TABLE_SQL;

        assert!(schema.contains(MarkdownFileLinkKind::Wikilink.as_sql_value()));
        assert!(schema.contains(MarkdownFileLinkKind::Markdown.as_sql_value()));
        assert!(schema.contains("target_path TEXT REFERENCES markdown_files(path)"));
        assert!(CREATE_MARKDOWN_FILE_LINKS_TARGET_INDEX_SQL
            .contains("ON markdown_file_links (target_path)"));
        assert!(CREATE_MARKDOWN_FILE_LINKS_SOURCE_INDEX_SQL
            .contains("ON markdown_file_links (source_path, link_position)"));
        assert!(schema.contains("target_url TEXT"));
    }

    #[test]
    fn markdown_file_backlinks_view_normalizes_incoming_links() {
        let view = CREATE_MARKDOWN_FILE_BACKLINKS_VIEW_SQL;

        for field in [
            "CREATE VIEW IF NOT EXISTS markdown_file_backlinks",
            "target_path AS path",
            "source_path AS backlink_source_path",
            "FROM markdown_file_links",
            "WHERE target_path IS NOT NULL",
            "resolved_status = 'resolved'",
        ] {
            assert!(
                view.contains(field),
                "backlinks view should include {field}"
            );
        }
    }

    #[test]
    fn markdown_file_backlink_records_returns_resolved_inbound_links() {
        let mut connection = Connection::open_in_memory().expect("in-memory sqlite should open");
        apply_sqlite_index_migrations_to_connection(&mut connection)
            .expect("schema migrations should apply");

        let target = markdown_index_record(
            "/vault/contexts/notes/target.md",
            "notes/target.md",
            "target",
        );
        let mut source = markdown_index_record(
            "/vault/contexts/agents/source.md",
            "agents/source.md",
            "source",
        );
        source.links.push(MarkdownFileLinkRecord {
            link_id: "source-target".to_string(),
            source_path: source.path.clone(),
            link_kind: MarkdownFileLinkKind::Wikilink,
            raw_target: "notes/target#intro".to_string(),
            normalized_target: "notes/target".to_string(),
            link_text: Some("target alias".to_string()),
            target_path: Some(target.path.clone()),
            target_anchor: Some("intro".to_string()),
            target_url: None,
            resolved_status: MarkdownFileLinkResolvedStatus::Resolved,
            byte_start: Some(18),
            byte_end: Some(48),
            line_number: Some(3),
            link_position: 0,
            indexed_at_unix_seconds: Some(30),
        });

        full_reindex_markdown_files_to_connection(
            &mut connection,
            &[target.clone(), source.clone()],
        )
        .expect("full reindex should write linked records");

        let backlinks = markdown_file_backlink_records_from_connection(&connection, &target.path)
            .expect("backlink records should query");

        assert_eq!(backlinks.len(), 1);
        assert_eq!(backlinks[0].path, target.path);
        assert_eq!(backlinks[0].backlink_source_path, source.path);
        assert_eq!(backlinks[0].raw_target, "notes/target#intro");
        assert_eq!(backlinks[0].link_text.as_deref(), Some("target alias"));
        assert_eq!(backlinks[0].target_anchor.as_deref(), Some("intro"));
        assert_eq!(backlinks[0].line_number, Some(3));
    }

    #[test]
    fn markdown_file_index_lookup_returns_metadata_tags_links_and_backlinks() {
        let mut connection = Connection::open_in_memory().expect("in-memory sqlite should open");
        apply_sqlite_index_migrations_to_connection(&mut connection)
            .expect("schema migrations should apply");

        let target = markdown_index_record(
            "/vault/contexts/notes/target.md",
            "notes/target.md",
            "target",
        );
        let mut source = markdown_index_record(
            "/vault/contexts/agents/source.md",
            "agents/source.md",
            "source",
        );
        source.tags.push(MarkdownFileTagRecord {
            path: source.path.clone(),
            tag_id: "Review".to_string(),
            tag_source: MarkdownFileTagSource::Import,
            tag_position: 0,
            indexed_at_unix_seconds: Some(30),
        });
        source.links.push(MarkdownFileLinkRecord {
            link_id: "source-target".to_string(),
            source_path: source.path.clone(),
            link_kind: MarkdownFileLinkKind::Wikilink,
            raw_target: "target".to_string(),
            normalized_target: "target".to_string(),
            link_text: None,
            target_path: Some(target.path.clone()),
            target_anchor: None,
            target_url: None,
            resolved_status: MarkdownFileLinkResolvedStatus::Resolved,
            byte_start: Some(10),
            byte_end: Some(20),
            line_number: Some(2),
            link_position: 0,
            indexed_at_unix_seconds: Some(30),
        });

        full_reindex_markdown_files_to_connection(
            &mut connection,
            &[target.clone(), source.clone()],
        )
        .expect("full reindex should write lookup fixture");

        let source_lookup = markdown_file_index_lookup_from_connection(&connection, &source.path)
            .expect("index lookup should query")
            .expect("source should have an index record");
        assert_eq!(source_lookup.metadata.title, "source");
        assert_eq!(source_lookup.tags[0].tag_id, "review");
        assert_eq!(source_lookup.links[0].raw_target, "target");
        assert!(source_lookup.backlinks.is_empty());

        let target_lookup = markdown_file_index_lookup_from_connection(&connection, &target.path)
            .expect("index lookup should query")
            .expect("target should have an index record");
        assert_eq!(target_lookup.backlinks.len(), 1);
        assert_eq!(target_lookup.backlinks[0].backlink_source_path, source.path);

        let tagged = markdown_file_metadata_records_by_tag_from_connection(&connection, "#review")
            .expect("tag lookup should query");
        assert_eq!(tagged.len(), 1);
        assert_eq!(tagged[0].path, source.path);
    }

    #[test]
    fn incremental_upsert_synchronizes_metadata_changes_into_query_results() {
        let mut connection = Connection::open_in_memory().expect("in-memory sqlite should open");
        apply_sqlite_index_migrations_to_connection(&mut connection)
            .expect("schema migrations should apply");

        let old_target = markdown_index_record(
            "/vault/contexts/notes/old-target.md",
            "notes/old-target.md",
            "old-target",
        );
        let new_target = markdown_index_record(
            "/vault/contexts/notes/new-target.md",
            "notes/new-target.md",
            "new-target",
        );
        let mut changed = markdown_index_record(
            "/vault/contexts/agents/changed.md",
            "agents/changed.md",
            "changed",
        );
        changed.tags.push(MarkdownFileTagRecord {
            path: changed.path.clone(),
            tag_id: "old-tag".to_string(),
            tag_source: MarkdownFileTagSource::Manual,
            tag_position: 0,
            indexed_at_unix_seconds: Some(20),
        });
        changed.links.push(MarkdownFileLinkRecord {
            link_id: "changed-old-target".to_string(),
            source_path: changed.path.clone(),
            link_kind: MarkdownFileLinkKind::Wikilink,
            raw_target: "Old Target".to_string(),
            normalized_target: "old target".to_string(),
            link_text: Some("old target".to_string()),
            target_path: Some(old_target.path.clone()),
            target_anchor: None,
            target_url: None,
            resolved_status: MarkdownFileLinkResolvedStatus::Resolved,
            byte_start: Some(12),
            byte_end: Some(26),
            line_number: Some(3),
            link_position: 0,
            indexed_at_unix_seconds: Some(20),
        });

        full_reindex_markdown_files_to_connection(
            &mut connection,
            &[old_target.clone(), new_target.clone(), changed.clone()],
        )
        .expect("initial reindex should seed metadata fixture");

        changed.relative_path = PathBuf::from("skills/changed.md");
        changed.folder_path = PathBuf::from("skills");
        changed.file_name = "changed.md".to_string();
        changed.content_hash = "changed-metadata-updated-hash".to_string();
        changed.tags = vec![
            MarkdownFileTagRecord {
                path: changed.path.clone(),
                tag_id: "new-tag".to_string(),
                tag_source: MarkdownFileTagSource::Manual,
                tag_position: 0,
                indexed_at_unix_seconds: Some(30),
            },
            MarkdownFileTagRecord {
                path: changed.path.clone(),
                tag_id: "folder-tag".to_string(),
                tag_source: MarkdownFileTagSource::Frontmatter,
                tag_position: 1,
                indexed_at_unix_seconds: Some(30),
            },
        ];
        changed.links = vec![MarkdownFileLinkRecord {
            link_id: "changed-new-target".to_string(),
            source_path: changed.path.clone(),
            link_kind: MarkdownFileLinkKind::Wikilink,
            raw_target: "New Target#usage".to_string(),
            normalized_target: "new target".to_string(),
            link_text: Some("new target".to_string()),
            target_path: Some(new_target.path.clone()),
            target_anchor: Some("usage".to_string()),
            target_url: None,
            resolved_status: MarkdownFileLinkResolvedStatus::Resolved,
            byte_start: Some(30),
            byte_end: Some(51),
            line_number: Some(5),
            link_position: 0,
            indexed_at_unix_seconds: Some(30),
        }];

        upsert_markdown_file_index_record_to_connection(&mut connection, &changed)
            .expect("incremental upsert should synchronize metadata changes");

        let changed_lookup = markdown_file_index_lookup_from_connection(&connection, &changed.path)
            .expect("changed lookup should query")
            .expect("changed file should remain indexed");
        assert_eq!(
            changed_lookup.metadata.relative_path,
            PathBuf::from("skills/changed.md")
        );
        assert_eq!(changed_lookup.metadata.folder_path, PathBuf::from("skills"));
        assert_eq!(
            changed_lookup
                .tags
                .iter()
                .map(|tag| tag.tag_id.as_str())
                .collect::<Vec<_>>(),
            vec!["new-tag", "folder-tag"]
        );
        assert_eq!(changed_lookup.links.len(), 1);
        assert_eq!(changed_lookup.links[0].raw_target, "New Target#usage");
        assert_eq!(
            changed_lookup.links[0].target_path,
            Some(new_target.path.clone())
        );

        let old_tag_matches =
            markdown_file_metadata_records_by_tag_from_connection(&connection, "old-tag")
                .expect("old tag lookup should query");
        assert!(old_tag_matches.is_empty());

        let new_tag_matches =
            markdown_file_metadata_records_by_tag_from_connection(&connection, "#new-tag")
                .expect("new tag lookup should query");
        assert_eq!(new_tag_matches.len(), 1);
        assert_eq!(new_tag_matches[0].path, changed.path);
        assert_eq!(new_tag_matches[0].folder_path, PathBuf::from("skills"));

        let old_target_backlinks =
            markdown_file_backlink_records_from_connection(&connection, &old_target.path)
                .expect("old target backlinks should query");
        assert!(old_target_backlinks.is_empty());

        let new_target_backlinks =
            markdown_file_backlink_records_from_connection(&connection, &new_target.path)
                .expect("new target backlinks should query");
        assert_eq!(new_target_backlinks.len(), 1);
        assert_eq!(new_target_backlinks[0].backlink_source_path, changed.path);
        assert_eq!(new_target_backlinks[0].raw_target, "New Target#usage");
        assert_eq!(
            new_target_backlinks[0].target_anchor.as_deref(),
            Some("usage")
        );
    }

    #[test]
    fn markdown_file_link_status_values_match_sql_check_constraint() {
        let schema = CREATE_MARKDOWN_FILE_LINKS_TABLE_SQL;

        for status in [
            MarkdownFileLinkResolvedStatus::Unresolved,
            MarkdownFileLinkResolvedStatus::Resolved,
            MarkdownFileLinkResolvedStatus::External,
            MarkdownFileLinkResolvedStatus::Ambiguous,
        ] {
            assert!(schema.contains(status.as_sql_value()));
        }
    }

    #[test]
    fn migrations_apply_schema_and_set_user_version() {
        let mut connection = Connection::open_in_memory().expect("in-memory sqlite should open");

        let report = apply_sqlite_index_migrations_to_connection(&mut connection)
            .expect("schema migrations should apply");

        assert_eq!(report.previous_schema_version, 0);
        assert_eq!(report.applied_schema_version, CTX_INDEX_SCHEMA_VERSION);
        assert_eq!(
            sqlite_user_version(&connection).expect("user version should be readable"),
            CTX_INDEX_SCHEMA_VERSION
        );

        for table_name in [
            MARKDOWN_FILES_TABLE_NAME,
            MARKDOWN_FILE_SEARCH_TABLE_NAME,
            MARKDOWN_FILE_FRONTMATTER_TABLE_NAME,
            TAGS_TABLE_NAME,
            MARKDOWN_FILE_TAGS_TABLE_NAME,
            MARKDOWN_FILE_LINKS_TABLE_NAME,
        ] {
            assert!(
                sqlite_object_exists(&connection, "table", table_name),
                "{table_name} should exist"
            );
        }
        assert!(sqlite_object_exists(
            &connection,
            "view",
            MARKDOWN_FILE_BACKLINKS_VIEW_NAME
        ));
    }

    #[test]
    fn migrations_upgrade_existing_v4_markdown_files_table() {
        let mut connection = Connection::open_in_memory().expect("in-memory sqlite should open");
        connection
            .execute_batch(
                r#"
                CREATE TABLE markdown_files (
                    path TEXT PRIMARY KEY NOT NULL,
                    vault_scope TEXT NOT NULL CHECK (vault_scope IN ('global', 'local')),
                    relative_path TEXT NOT NULL,
                    file_name TEXT NOT NULL,
                    file_created_at_unix_seconds INTEGER NOT NULL,
                    file_modified_at_unix_seconds INTEGER NOT NULL,
                    indexed_at_unix_seconds INTEGER,
                    content_hash TEXT NOT NULL,
                    indexing_status TEXT NOT NULL CHECK (
                        indexing_status IN ('pending', 'indexed', 'stale', 'failed')
                    ),
                    last_index_error TEXT,
                    import_source TEXT,
                    UNIQUE (vault_scope, relative_path)
                );
                PRAGMA user_version = 4;
                "#,
            )
            .expect("v4 schema fixture should apply");

        let report = apply_sqlite_index_migrations_to_connection(&mut connection)
            .expect("v4 schema should migrate");
        let columns = table_columns(&connection, MARKDOWN_FILES_TABLE_NAME)
            .expect("migrated columns should be readable");

        assert_eq!(report.previous_schema_version, 4);
        assert_eq!(report.applied_schema_version, CTX_INDEX_SCHEMA_VERSION);
        for (column_name, _) in MARKDOWN_FILES_COLUMN_MIGRATIONS {
            assert!(
                columns.contains(*column_name),
                "migrated markdown_files should include {column_name}"
            );
        }
    }

    #[test]
    fn full_reindex_clears_stale_rows_and_upserts_discovered_markdown_metadata() {
        let mut connection = Connection::open_in_memory().expect("in-memory sqlite should open");
        apply_sqlite_index_migrations_to_connection(&mut connection)
            .expect("schema migrations should apply");

        let stale = markdown_index_record("/vault/contexts/stale.md", "stale.md", "stale");
        let first_report = full_reindex_markdown_files_to_connection(&mut connection, &[stale])
            .expect("initial reindex should write stale fixture");
        assert_eq!(first_report.cleared_markdown_files, 0);
        assert_eq!(first_report.indexed_markdown_files, 1);

        let mut current = markdown_index_record(
            "/vault/contexts/agents/reviewer.md",
            "agents/reviewer.md",
            "reviewer",
        );
        current.classification = Classification::Shared;
        current.import_classification_suggestion = Some(Classification::Subagent);
        current.tags.push(MarkdownFileTagRecord {
            path: current.path.clone(),
            tag_id: "review".to_string(),
            tag_source: MarkdownFileTagSource::Import,
            tag_position: 0,
            indexed_at_unix_seconds: Some(20),
        });
        current.links.push(MarkdownFileLinkRecord {
            link_id: "reviewer-style-guide".to_string(),
            source_path: current.path.clone(),
            link_kind: MarkdownFileLinkKind::Wikilink,
            raw_target: "Style Guide".to_string(),
            normalized_target: "style guide".to_string(),
            link_text: None,
            target_path: None,
            target_anchor: None,
            target_url: None,
            resolved_status: MarkdownFileLinkResolvedStatus::Unresolved,
            byte_start: None,
            byte_end: None,
            line_number: None,
            link_position: 0,
            indexed_at_unix_seconds: Some(20),
        });

        let second_report = full_reindex_markdown_files_to_connection(&mut connection, &[current])
            .expect("full reindex should clear stale state and write current metadata");

        assert_eq!(second_report.cleared_markdown_files, 1);
        assert_eq!(second_report.indexed_markdown_files, 1);
        assert_eq!(second_report.indexed_tags, 1);
        assert_eq!(second_report.indexed_links, 1);
        assert_eq!(table_count(&connection, MARKDOWN_FILES_TABLE_NAME), 1);
        assert_eq!(table_count(&connection, MARKDOWN_FILE_TAGS_TABLE_NAME), 1);
        assert_eq!(table_count(&connection, MARKDOWN_FILE_LINKS_TABLE_NAME), 1);
        assert_eq!(
            connection
                .query_row(
                    "SELECT relative_path FROM markdown_files WHERE path = ?1;",
                    params!["/vault/contexts/agents/reviewer.md"],
                    |row| row.get::<_, String>(0),
                )
                .expect("current file should be indexed"),
            "agents/reviewer.md"
        );
        assert_eq!(
            connection
                .query_row(
                    "SELECT classification, import_classification_suggestion FROM markdown_files WHERE path = ?1;",
                    params!["/vault/contexts/agents/reviewer.md"],
                    |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
                )
                .expect("classification fields should be indexed separately"),
            ("shared".to_string(), "subagent".to_string())
        );
        assert_eq!(
            connection
                .query_row(
                    "SELECT COUNT(*) FROM markdown_files WHERE path = ?1;",
                    params!["/vault/contexts/stale.md"],
                    |row| row.get::<_, usize>(0),
                )
                .expect("stale count should be readable"),
            0
        );
    }

    #[test]
    fn incremental_upsert_updates_one_markdown_file_without_clearing_other_rows() {
        let mut connection = Connection::open_in_memory().expect("in-memory sqlite should open");
        apply_sqlite_index_migrations_to_connection(&mut connection)
            .expect("schema migrations should apply");

        let mut stable = markdown_index_record("/vault/contexts/stable.md", "stable.md", "stable");
        stable.tags.push(MarkdownFileTagRecord {
            path: stable.path.clone(),
            tag_id: "stable".to_string(),
            tag_source: MarkdownFileTagSource::Import,
            tag_position: 0,
            indexed_at_unix_seconds: Some(20),
        });
        let mut changed =
            markdown_index_record("/vault/contexts/changed.md", "changed.md", "changed");
        changed.tags.push(MarkdownFileTagRecord {
            path: changed.path.clone(),
            tag_id: "old".to_string(),
            tag_source: MarkdownFileTagSource::Import,
            tag_position: 0,
            indexed_at_unix_seconds: Some(20),
        });

        full_reindex_markdown_files_to_connection(&mut connection, &[stable, changed.clone()])
            .expect("initial reindex should seed fixtures");

        changed.title = "Changed Title".to_string();
        changed.content_hash = "new-hash".to_string();
        changed.tags = vec![MarkdownFileTagRecord {
            path: changed.path.clone(),
            tag_id: "new".to_string(),
            tag_source: MarkdownFileTagSource::Import,
            tag_position: 0,
            indexed_at_unix_seconds: Some(30),
        }];
        changed.links = vec![MarkdownFileLinkRecord {
            link_id: "changed-target".to_string(),
            source_path: changed.path.clone(),
            link_kind: MarkdownFileLinkKind::Wikilink,
            raw_target: "Target".to_string(),
            normalized_target: "target".to_string(),
            link_text: None,
            target_path: None,
            target_anchor: None,
            target_url: None,
            resolved_status: MarkdownFileLinkResolvedStatus::Unresolved,
            byte_start: None,
            byte_end: None,
            line_number: None,
            link_position: 0,
            indexed_at_unix_seconds: Some(30),
        }];

        let report = upsert_markdown_file_index_record_to_connection(&mut connection, &changed)
            .expect("incremental upsert should update one file");

        assert_eq!(report.indexed_markdown_files, 1);
        assert_eq!(report.indexed_tags, 1);
        assert_eq!(report.indexed_links, 1);
        assert_eq!(table_count(&connection, MARKDOWN_FILES_TABLE_NAME), 2);
        assert_eq!(
            connection
                .query_row(
                    "SELECT title, content_hash FROM markdown_files WHERE path = ?1;",
                    params!["/vault/contexts/changed.md"],
                    |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
                )
                .expect("changed file should be readable"),
            ("Changed Title".to_string(), "new-hash".to_string())
        );
        assert_eq!(
            connection
                .query_row(
                    "SELECT COUNT(*) FROM markdown_file_tags WHERE path = ?1 AND tag_id = 'old';",
                    params!["/vault/contexts/changed.md"],
                    |row| row.get::<_, usize>(0),
                )
                .expect("old tags should be queryable"),
            0
        );
        assert_eq!(
            connection
                .query_row(
                    "SELECT COUNT(*) FROM markdown_file_tags WHERE path = ?1 AND tag_id = 'stable';",
                    params!["/vault/contexts/stable.md"],
                    |row| row.get::<_, usize>(0),
                )
                .expect("stable tags should be queryable"),
            1
        );
    }

    #[test]
    fn full_reindex_adds_created_markdown_content_to_search_results() {
        let mut connection = Connection::open_in_memory().expect("in-memory sqlite should open");
        apply_sqlite_index_migrations_to_connection(&mut connection)
            .expect("schema migrations should apply");

        let mut created = markdown_index_record(
            "/vault/contexts/agents/reviewer.md",
            "agents/reviewer.md",
            "Reviewer",
        );
        created.content =
            "# Reviewer\n\nVerify lifecycle cleanup and prompt injection behavior.".to_string();

        full_reindex_markdown_files_to_connection(&mut connection, &[created])
            .expect("full reindex should write search content");

        let results = search_markdown_file_index_from_connection(&connection, "lifecycle")
            .expect("search query should run");

        assert_eq!(
            results,
            vec![MarkdownFileSearchResult {
                path: PathBuf::from("/vault/contexts/agents/reviewer.md"),
                title: "Reviewer".to_string(),
                relative_path: PathBuf::from("agents/reviewer.md"),
            }]
        );
    }

    #[test]
    fn incremental_upsert_replaces_updated_markdown_content_in_search_results() {
        let mut connection = Connection::open_in_memory().expect("in-memory sqlite should open");
        apply_sqlite_index_migrations_to_connection(&mut connection)
            .expect("schema migrations should apply");

        let mut changed =
            markdown_index_record("/vault/contexts/changed.md", "changed.md", "Changed");
        changed.content = "# Changed\n\nLegacy alpha guidance.".to_string();
        full_reindex_markdown_files_to_connection(&mut connection, &[changed.clone()])
            .expect("initial reindex should seed search fixture");

        assert_eq!(
            search_markdown_file_index_from_connection(&connection, "alpha")
                .expect("old term should query")
                .len(),
            1
        );

        changed.title = "Changed Updated".to_string();
        changed.content_hash = "changed-updated-hash".to_string();
        changed.content = "# Changed Updated\n\nFresh beta guidance.".to_string();
        upsert_markdown_file_index_record_to_connection(&mut connection, &changed)
            .expect("incremental upsert should refresh search content");

        assert!(
            search_markdown_file_index_from_connection(&connection, "alpha")
                .expect("old term should query")
                .is_empty(),
            "stale content should be removed from full-text results"
        );
        assert_eq!(
            search_markdown_file_index_from_connection(&connection, "beta")
                .expect("new term should query"),
            vec![MarkdownFileSearchResult {
                path: PathBuf::from("/vault/contexts/changed.md"),
                title: "Changed Updated".to_string(),
                relative_path: PathBuf::from("changed.md"),
            }]
        );
    }

    fn sqlite_object_exists(connection: &Connection, object_type: &str, name: &str) -> bool {
        connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = ?1 AND name = ?2);",
                rusqlite::params![object_type, name],
                |row| row.get::<_, i64>(0),
            )
            .map(|exists| exists == 1)
            .unwrap_or(false)
    }

    fn table_count(connection: &Connection, table_name: &str) -> usize {
        connection
            .query_row(&format!("SELECT COUNT(*) FROM {table_name};"), [], |row| {
                row.get::<_, usize>(0)
            })
            .expect("table count should be readable")
    }

    fn markdown_index_record(
        path: &str,
        relative_path: &str,
        title: &str,
    ) -> MarkdownFileIndexRecord {
        MarkdownFileIndexRecord {
            path: PathBuf::from(path),
            context_id: format!("{title}-context-id"),
            title: title.to_string(),
            vault_scope: VaultScope::Local,
            relative_path: PathBuf::from(relative_path),
            folder_path: PathBuf::from(relative_path)
                .parent()
                .map(Path::to_path_buf)
                .unwrap_or_default(),
            file_name: PathBuf::from(relative_path)
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or(relative_path)
                .to_string(),
            classification: Classification::Shared,
            import_classification_suggestion: Some(Classification::Shared),
            inferred_classification: Some(Classification::Shared),
            llm_classification_status: crate::ClassificationStatus::Classified,
            file_created_at_unix_seconds: 10,
            file_modified_at_unix_seconds: 20,
            indexed_at_unix_seconds: Some(20),
            content_hash: format!("{title}-hash"),
            content: format!("# {title}\n\n{title} context body."),
            indexing_status: MarkdownFileIndexingStatus::Indexed,
            last_index_error: None,
            import_source: None,
            import_source_type: Some(crate::ImportSourceType::ContextMarkdown),
            frontmatter: Some(ParsedFrontmatterMetadata {
                path: PathBuf::from(path),
                frontmatter_format: FrontmatterFormat::None,
                frontmatter_raw: None,
                frontmatter_json: "{}".to_string(),
                frontmatter_title: None,
                frontmatter_tags: Vec::new(),
                frontmatter_classification: None,
                parse_status: FrontmatterParseStatus::Absent,
                parse_error: None,
                parsed_at_unix_seconds: Some(20),
            }),
            tags: Vec::new(),
            links: Vec::new(),
        }
    }
}
