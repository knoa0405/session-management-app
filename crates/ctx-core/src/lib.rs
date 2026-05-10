//! Shared CTX domain library used by both the Tauri backend and bundled `ctx`
//! command-line wrapper.
//!
//! The crate is intentionally scaffolded around Phase 1 boundaries:
//! vault discovery and overlay rules, preset composition, classification
//! suggestions, and CLI injection primitives.

pub mod classification;
pub mod injection;
pub mod models;
pub mod presets;
pub mod settings;
pub mod sqlite_index;
pub mod vault;
pub mod watch;

use std::path::Path;

pub use classification::{
    build_headless_classification_prompt, classification_rule_for, classify_discovered_context,
    classify_import_markdown_content, deterministic_classification, deterministic_tags,
    noninteractive_cli_args, noninteractive_cli_program, parse_headless_classification_cli_output,
    parse_headless_classification_text, run_noninteractive_cli_process, ClassificationSuggestion,
    ContextClassificationRule, DiscoveredContextClassification,
    DiscoveredContextClassificationMetadata, HeadlessClassificationAdapter,
    HeadlessClassificationAdapterKind, HeadlessClassificationError, HeadlessClassificationRequest,
    HeadlessClassificationResult, ImportTimeClassificationRequest, ImportTimeClassificationResult,
    LocalHeadlessCliClassificationAdapter, NoninteractiveCliProcessError,
    NoninteractiveCliProcessOutput, NoninteractiveCliProcessRequest, CONTEXT_CLASSIFICATION_RULES,
    MAIN_AGENT_DIRECTORY_PATTERNS, MAIN_AGENT_FILE_NAMES, SKILL_DIRECTORY_PATTERNS,
    SKILL_DIRECTORY_SEGMENTS, SUBAGENT_DIRECTORY_PATTERNS, SUBAGENT_DIRECTORY_SEGMENTS,
    SUBAGENT_FILE_STEM_TOKENS,
};
pub use injection::{
    assemble_claude_prompt_file, assemble_codex_agents_md_payload,
    assemble_combined_context_output, assemble_context_output_with_options,
    assemble_subagent_context_output, build_agents_md_managed_section, build_markers,
    cleanup_codex_agents_md, cleanup_residual_codex_agents_md_markers, cleanup_stale_wrapper_state,
    cleanup_transient_wrapper_artifacts, default_wrapper_state_dir,
    detect_residual_codex_agents_md_markers, inject_codex_agents_md, injection_strategy,
    remove_agents_md_managed_section, remove_transient_wrapper_state_file,
    replace_agents_md_managed_section, resolve_preset_context_items,
    resolve_subagent_context_items, wrapper_state_path, write_transient_wrapper_state,
    ClaudePromptFile, CodexAgentsMdInjection, CodexInjectionError, CodexResidualMarkers,
    ContextItemResolveError, ContextRenderOptions, PromptAssemblyError, SectionReplaceError,
    SubagentContextResolveError, TransientWrapperState, WrapperStateCleanupReport,
    WrapperStateError, AGENTS_MD_FILE_NAME, COMBINED_CONTEXT_ITEM_SEPARATOR, CTX_END_MARKER,
    CTX_START_MARKER, WRAPPER_STATE_DIR_NAME,
};
pub use models::{
    AppStatus, Classification, ClassificationStatus, CliExecutionSettings, CliTarget,
    ContextDiscoveryMetadata, ContextDiscoveryResult, ContextFragment, HandoffConstraints,
    ImportSourceType, InjectionMarkers, InjectionStrategy, Preset, PresetContextComposition,
    PresetContextSelection, PresetContextSelectionInput, PresetContextSelectionKind,
    PresetExecutionSettingsUpdate, PresetMetadata, ResolvedContextItem, SessionRecord,
    SessionStatus, SubagentManifest, SubagentManifestUpdate, SubagentRole, SubagentSpawnGuidance,
    VaultEntryKey, VaultScope, WrapperBehavior,
};
pub use presets::{
    list_presets_from_resolved_overlay, load_preset_from_resolved_overlay, managed_presets_dir,
    new_empty_preset, save_preset_execution_settings, save_preset_subagent_manifest,
    validate_cli_execution_settings, validate_subagent_manifest, LoadedPreset, PresetLoadError,
    PresetSummary, MANAGED_PRESETS_DIR, MAX_SUBAGENT_MANIFEST_JSON_BYTES,
};
pub use settings::{
    load_configured_scan_roots, load_configured_skill_scan_roots, load_vault_settings_overlay,
    vault_settings_path, ConfiguredScanRoot, ResolvedVaultSettings, ScanRootConfig, VaultSettings,
    VaultSettingsError, VaultSettingsSource, VAULT_SETTINGS_FILE_NAME,
};
pub use sqlite_index::{
    apply_sqlite_index_migrations, apply_sqlite_index_migrations_to_connection,
    full_reindex_markdown_files, full_reindex_markdown_files_to_connection,
    markdown_file_backlink_records, markdown_file_backlink_records_from_connection,
    markdown_file_index_lookup, markdown_file_index_lookup_from_connection,
    markdown_file_link_records, markdown_file_link_records_from_connection,
    markdown_file_metadata_record, markdown_file_metadata_record_from_connection,
    markdown_file_metadata_records_by_tag, markdown_file_metadata_records_by_tag_from_connection,
    markdown_file_tag_records, markdown_file_tag_records_from_connection,
    move_markdown_file_index_record, move_markdown_file_index_record_to_connection,
    remove_markdown_file_index_record, remove_markdown_file_index_record_to_connection,
    search_markdown_file_index, search_markdown_file_index_from_connection, sqlite_index_path,
    upsert_markdown_file_index_record, upsert_markdown_file_index_record_to_connection,
    FrontmatterFormat, FrontmatterParseStatus, FullMarkdownReindexReport,
    IncrementalMarkdownIndexReport, MarkdownFileBacklinkRecord, MarkdownFileIndexLookup,
    MarkdownFileIndexRecord, MarkdownFileIndexRemovalReport, MarkdownFileIndexingStatus,
    MarkdownFileLinkKind, MarkdownFileLinkRecord, MarkdownFileLinkResolvedStatus,
    MarkdownFileMetadataRecord, MarkdownFileSearchResult, MarkdownFileTagRecord,
    MarkdownFileTagSource, NormalizedTagRecord, ParsedFrontmatterMetadata, SqliteIndexError,
    SqliteIndexMigrationReport, CREATE_MARKDOWN_FILES_CONTENT_HASH_INDEX_SQL,
    CREATE_MARKDOWN_FILES_INDEXING_STATUS_INDEX_SQL, CREATE_MARKDOWN_FILES_TABLE_SQL,
    CREATE_MARKDOWN_FILES_VAULT_SCOPE_INDEX_SQL, CREATE_MARKDOWN_FILE_BACKLINKS_VIEW_SQL,
    CREATE_MARKDOWN_FILE_FRONTMATTER_PARSE_STATUS_INDEX_SQL,
    CREATE_MARKDOWN_FILE_FRONTMATTER_TABLE_SQL, CREATE_MARKDOWN_FILE_FRONTMATTER_TITLE_INDEX_SQL,
    CREATE_MARKDOWN_FILE_LINKS_KIND_STATUS_INDEX_SQL,
    CREATE_MARKDOWN_FILE_LINKS_NORMALIZED_TARGET_INDEX_SQL,
    CREATE_MARKDOWN_FILE_LINKS_SOURCE_INDEX_SQL, CREATE_MARKDOWN_FILE_LINKS_TABLE_SQL,
    CREATE_MARKDOWN_FILE_LINKS_TARGET_INDEX_SQL, CREATE_MARKDOWN_FILE_SEARCH_TABLE_SQL,
    CREATE_MARKDOWN_FILE_TAGS_PATH_POSITION_INDEX_SQL, CREATE_MARKDOWN_FILE_TAGS_SOURCE_INDEX_SQL,
    CREATE_MARKDOWN_FILE_TAGS_TABLE_SQL, CREATE_MARKDOWN_FILE_TAGS_TAG_ID_INDEX_SQL,
    CREATE_TAGS_TABLE_SQL, CTX_INDEX_DATABASE_FILE_NAME, CTX_INDEX_SCHEMA_VERSION,
    MARKDOWN_FILES_COLUMN_MIGRATIONS, MARKDOWN_FILES_TABLE_NAME, MARKDOWN_FILE_BACKLINKS_VIEW_NAME,
    MARKDOWN_FILE_FRONTMATTER_TABLE_NAME, MARKDOWN_FILE_LINKS_TABLE_NAME,
    MARKDOWN_FILE_SEARCH_TABLE_NAME, MARKDOWN_FILE_TAGS_TABLE_NAME, SQLITE_INDEX_SCHEMA_STATEMENTS,
    TAGS_TABLE_NAME,
};
pub use vault::{
    canonical_vault_entry_key, create_context_file, delete_markdown_context_file,
    delete_resolved_context_markdown, discover_existing_context_file_results,
    discover_existing_context_files, discover_global_vault_path, discover_project_local_vault_path,
    initialize_global_vault, initialize_project_local_vault, list_context_files,
    list_context_files_with_discovered, lookup_markdown_context_index,
    lookup_markdown_contexts_by_tag, managed_contexts_dir, materialize_discovered_context_file,
    materialize_discovered_context_files, read_markdown_context_file,
    read_resolved_context_markdown, reindex_markdown_contexts, resolve_overlay,
    resolve_overlay_vault, review_import_classification, sync_markdown_context_index_event,
    sync_markdown_context_index_events, update_markdown_context_file,
    update_resolved_context_markdown, GlobalVaultInitialization, OverlayMarkdownIndexLookup,
    ProjectLocalVaultInitialization, ResolvedOverlayVault, VaultError, VaultReindexReport,
    VaultRoots, CTX_HOME_DIR, GLOBAL_VAULT_DIR,
};
pub use watch::{
    configured_context_watch_roots, diff_context_file_snapshots, snapshot_context_directories,
    watch_context_directories, ContextDirectoryWatcher, ContextFileChangeEvent,
    ContextFileChangeKind, ContextFileSnapshot, ContextFileSnapshotEntry, ContextWatchError,
    ContextWatchRoot, ContextWatchRootKind,
};

pub const APP_NAME: &str = "ctx";

#[derive(Debug, Clone, Copy, Default)]
pub struct CtxCore;

impl CtxCore {
    pub fn new() -> Self {
        Self
    }

    pub fn status(&self) -> AppStatus {
        app_status()
    }

    pub fn vault_roots(&self, working_dir: &Path) -> VaultRoots {
        VaultRoots::discover(working_dir)
    }

    pub fn initialize_global_vault(
        &self,
        working_dir: &Path,
    ) -> Result<GlobalVaultInitialization, VaultError> {
        initialize_global_vault(working_dir)
    }

    pub fn initialize_project_local_vault(
        &self,
        working_dir: &Path,
    ) -> Result<ProjectLocalVaultInitialization, VaultError> {
        initialize_project_local_vault(working_dir)
    }

    pub fn injection_strategy(&self, target: CliTarget) -> &'static str {
        injection_strategy(target)
    }
}

pub fn app_status() -> AppStatus {
    AppStatus {
        name: APP_NAME.to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        vault_ready: true,
        sqlite_index_ready: true,
        wrapper_ready: false,
    }
}
