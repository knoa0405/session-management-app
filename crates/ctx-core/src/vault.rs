use super::{
    classify_discovered_context, classify_import_markdown_content, Classification,
    ClassificationStatus, ContextDiscoveryMetadata, ContextDiscoveryResult, ContextFragment,
    DiscoveredContextClassificationMetadata, ImportSourceType, ImportTimeClassificationRequest,
    VaultEntryKey, VaultScope, MAIN_AGENT_DIRECTORY_PATTERNS, MAIN_AGENT_FILE_NAMES,
    SKILL_DIRECTORY_PATTERNS, SUBAGENT_DIRECTORY_PATTERNS,
};
use crate::settings::{load_configured_scan_roots, load_configured_skill_scan_roots};
use crate::sqlite_index::{
    apply_sqlite_index_migrations, full_reindex_markdown_files, markdown_file_index_lookup,
    markdown_file_metadata_records_by_tag, move_markdown_file_index_record,
    remove_markdown_file_index_record, sqlite_index_path, upsert_markdown_file_index_record,
    FrontmatterFormat, FrontmatterParseStatus, FullMarkdownReindexReport,
    IncrementalMarkdownIndexReport, MarkdownFileIndexLookup, MarkdownFileIndexRecord,
    MarkdownFileIndexingStatus, MarkdownFileLinkKind, MarkdownFileLinkRecord,
    MarkdownFileLinkResolvedStatus, MarkdownFileMetadataRecord, MarkdownFileTagRecord,
    MarkdownFileTagSource, ParsedFrontmatterMetadata, SqliteIndexMigrationReport,
};
use crate::watch::{ContextFileChangeEvent, ContextFileChangeKind, ContextWatchRootKind};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, HashSet},
    ffi::OsString,
    fmt,
    fs::{self, OpenOptions},
    io::{Read, Write},
    path::{Component, Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};
use uuid::Uuid;

pub const CTX_HOME_DIR: &str = ".ctx";
pub const GLOBAL_VAULT_DIR: &str = "vault";
pub const MANAGED_CONTEXTS_DIR: &str = "contexts";
const IMPORT_METADATA_FILE_NAME: &str = "import-metadata.json";
const DISCOVERY_EXCLUDED_DIRECTORY_NAMES: &[&str] = &[
    ".git",
    ".hg",
    ".svn",
    ".next",
    ".nuxt",
    ".turbo",
    ".vercel",
    "build",
    "coverage",
    "dist",
    "node_modules",
    "out",
    "target",
    "vendor",
];

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct ImportMetadataIndex {
    #[serde(default)]
    imports: Vec<ImportMetadataRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ImportMetadataRecord {
    context_relative_path: PathBuf,
    import_source: PathBuf,
    source_type: ImportSourceType,
    #[serde(default)]
    classification: Option<Classification>,
    #[serde(default)]
    import_classification_suggestion: Option<Classification>,
    #[serde(default)]
    inferred_classification: Option<Classification>,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default)]
    llm_classification_status: Option<ClassificationStatus>,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct GlobalVaultInitialization {
    pub global_root: PathBuf,
    pub contexts_dir: PathBuf,
    pub sqlite_index_path: PathBuf,
    pub sqlite_migration: SqliteIndexMigrationReport,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ProjectLocalVaultInitialization {
    pub local_root: PathBuf,
    pub contexts_dir: PathBuf,
    pub sqlite_index_path: PathBuf,
    pub sqlite_migration: SqliteIndexMigrationReport,
}

#[derive(Debug, Clone)]
pub struct VaultRoots {
    pub global_root: PathBuf,
    pub local_root: Option<PathBuf>,
}

#[derive(Debug, Clone)]
pub struct ResolvedOverlayVault {
    pub roots: VaultRoots,
    pub contexts: Vec<ContextFragment>,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct VaultReindexReport {
    pub global: FullMarkdownReindexReport,
    pub local: Option<FullMarkdownReindexReport>,
    pub discovered_markdown_files: usize,
}

#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct OverlayMarkdownIndexLookup {
    pub lookup: MarkdownFileIndexLookup,
    pub sqlite_index_path: PathBuf,
}

impl VaultRoots {
    pub fn discover(working_dir: &Path) -> Self {
        let global_root = discover_global_vault_path(working_dir);
        let local_root = Some(discover_project_local_vault_path(working_dir));

        Self {
            global_root,
            local_root,
        }
    }
}

pub fn discover_global_vault_path(working_dir: &Path) -> PathBuf {
    global_vault_path_from_home(std::env::var_os("HOME"), working_dir)
}

pub fn discover_project_local_vault_path(working_dir: &Path) -> PathBuf {
    working_dir.join(CTX_HOME_DIR).join(GLOBAL_VAULT_DIR)
}

pub fn initialize_global_vault(
    working_dir: &Path,
) -> Result<GlobalVaultInitialization, VaultError> {
    initialize_global_vault_path_from_home(std::env::var_os("HOME"), working_dir)
}

pub fn initialize_project_local_vault(
    working_dir: &Path,
) -> Result<ProjectLocalVaultInitialization, VaultError> {
    let local_root = discover_project_local_vault_path(working_dir);
    let contexts_dir = managed_contexts_dir(&local_root);

    fs::create_dir_all(&contexts_dir).map_err(|error| {
        VaultError::Io(format!(
            "failed to initialize project-local vault directory {}: {error}",
            contexts_dir.display()
        ))
    })?;
    let sqlite_index_path = sqlite_index_path(&local_root);
    let sqlite_migration = apply_sqlite_index_migrations(&sqlite_index_path).map_err(|error| {
        VaultError::Index(format!(
            "failed to apply sqlite index migrations for project-local vault {}: {error}",
            sqlite_index_path.display()
        ))
    })?;

    Ok(ProjectLocalVaultInitialization {
        local_root,
        contexts_dir,
        sqlite_index_path,
        sqlite_migration,
    })
}

fn global_vault_path_from_home(home: Option<OsString>, fallback_dir: &Path) -> PathBuf {
    home.map(PathBuf::from)
        .unwrap_or_else(|| fallback_dir.to_path_buf())
        .join(CTX_HOME_DIR)
        .join(GLOBAL_VAULT_DIR)
}

fn initialize_global_vault_path_from_home(
    home: Option<OsString>,
    fallback_dir: &Path,
) -> Result<GlobalVaultInitialization, VaultError> {
    let global_root = global_vault_path_from_home(home, fallback_dir);
    let contexts_dir = managed_contexts_dir(&global_root);

    fs::create_dir_all(&contexts_dir).map_err(|error| {
        VaultError::Io(format!(
            "failed to initialize global vault directory {}: {error}",
            contexts_dir.display()
        ))
    })?;
    let sqlite_index_path = sqlite_index_path(&global_root);
    let sqlite_migration = apply_sqlite_index_migrations(&sqlite_index_path).map_err(|error| {
        VaultError::Index(format!(
            "failed to apply sqlite index migrations for global vault {}: {error}",
            sqlite_index_path.display()
        ))
    })?;

    Ok(GlobalVaultInitialization {
        global_root,
        contexts_dir,
        sqlite_index_path,
        sqlite_migration,
    })
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub enum VaultError {
    MissingLocalVault,
    EmptyFileName,
    InvalidFileName(String),
    InvalidExtension(String),
    InvalidFolderPath(String),
    DuplicateContext(PathBuf),
    MissingContext(PathBuf),
    Io(String),
    Index(String),
}

impl fmt::Display for VaultError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingLocalVault => write!(formatter, "local vault root is not configured"),
            Self::EmptyFileName => write!(formatter, "context filename cannot be empty"),
            Self::InvalidFileName(message) => write!(formatter, "{message}"),
            Self::InvalidExtension(file_name) => {
                write!(
                    formatter,
                    "context filename must use the .md extension: {file_name}"
                )
            }
            Self::InvalidFolderPath(message) => write!(formatter, "{message}"),
            Self::DuplicateContext(path) => {
                write!(formatter, "context file already exists: {}", path.display())
            }
            Self::MissingContext(path) => {
                write!(
                    formatter,
                    "context file does not exist or is not a file: {}",
                    path.display()
                )
            }
            Self::Io(message) => write!(formatter, "{message}"),
            Self::Index(message) => write!(formatter, "{message}"),
        }
    }
}

impl std::error::Error for VaultError {}

pub fn managed_contexts_dir(root: &Path) -> PathBuf {
    root.join(MANAGED_CONTEXTS_DIR)
}

pub fn create_context_file(
    roots: &VaultRoots,
    scope: VaultScope,
    folder_path: impl AsRef<Path>,
    file_name: &str,
    content: &str,
) -> Result<ContextFragment, VaultError> {
    validate_context_file_name(file_name)?;

    let folder_path = normalize_context_folder(folder_path.as_ref())?;
    let root = match scope {
        VaultScope::Global => roots.global_root.clone(),
        VaultScope::Local => roots
            .local_root
            .clone()
            .ok_or(VaultError::MissingLocalVault)?,
    };

    let contexts_dir = managed_contexts_dir(&root);
    let target_dir = contexts_dir.join(&folder_path);
    let target_path = target_dir.join(file_name);

    if target_path.exists() {
        return Err(VaultError::DuplicateContext(target_path));
    }

    fs::create_dir_all(&target_dir).map_err(|error| {
        VaultError::Io(format!(
            "failed to create context directory {}: {error}",
            target_dir.display()
        ))
    })?;

    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&target_path)
        .map_err(|error| {
            if error.kind() == std::io::ErrorKind::AlreadyExists {
                VaultError::DuplicateContext(target_path.clone())
            } else {
                VaultError::Io(format!(
                    "failed to create context file {}: {error}",
                    target_path.display()
                ))
            }
        })?;

    file.write_all(content.as_bytes()).map_err(|error| {
        VaultError::Io(format!(
            "failed to write context file {}: {error}",
            target_path.display()
        ))
    })?;

    Ok(ContextFragment {
        context_id: Uuid::new_v4(),
        title: context_title(file_name),
        content: content.to_string(),
        file_path: target_path,
        vault_scope: scope,
        classification: Classification::Shared,
        import_classification_suggestion: None,
        inferred_classification: None,
        tags: Vec::new(),
        folder_path,
        wikilinks: extract_wikilinks(content),
        backlinks: Vec::new(),
        import_source: None,
        import_source_type: None,
        llm_classification_status: ClassificationStatus::Pending,
    })
}

pub fn read_markdown_context_file(path: &Path) -> Result<String, VaultError> {
    if !path.is_file() {
        return Err(VaultError::Io(format!(
            "context file does not exist or is not a file: {}",
            path.display()
        )));
    }

    if !is_markdown_file(path) {
        return Err(VaultError::InvalidExtension(path.display().to_string()));
    }

    fs::read_to_string(path).map_err(|error| {
        VaultError::Io(format!(
            "failed to read context file {}: {error}",
            path.display()
        ))
    })
}

pub fn update_markdown_context_file(path: &Path, content: &str) -> Result<String, VaultError> {
    if !path.is_file() {
        return Err(VaultError::MissingContext(path.to_path_buf()));
    }

    if !is_markdown_file(path) {
        return Err(VaultError::InvalidExtension(path.display().to_string()));
    }

    fs::write(path, content).map_err(|error| {
        VaultError::Io(format!(
            "failed to write context file {}: {error}",
            path.display()
        ))
    })?;

    Ok(content.to_string())
}

pub fn delete_markdown_context_file(path: &Path) -> Result<PathBuf, VaultError> {
    if !path.is_file() {
        return Err(VaultError::MissingContext(path.to_path_buf()));
    }

    if !is_markdown_file(path) {
        return Err(VaultError::InvalidExtension(path.display().to_string()));
    }

    fs::remove_file(path).map_err(|error| {
        VaultError::Io(format!(
            "failed to delete context file {}: {error}",
            path.display()
        ))
    })?;

    Ok(path.to_path_buf())
}

pub fn list_context_files(roots: &VaultRoots) -> Result<Vec<ContextFragment>, VaultError> {
    let mut contexts = Vec::new();

    collect_context_files(
        &managed_contexts_dir(&roots.global_root),
        VaultScope::Global,
        &mut contexts,
    )?;

    if let Some(local_root) = &roots.local_root {
        collect_context_files(
            &managed_contexts_dir(local_root),
            VaultScope::Local,
            &mut contexts,
        )?;
    }

    contexts.sort_by(|left, right| {
        left.vault_scope
            .cmp(&right.vault_scope)
            .then_with(|| left.folder_path.cmp(&right.folder_path))
            .then_with(|| left.title.cmp(&right.title))
    });

    let mut contexts = resolve_overlay(contexts);
    hydrate_backlinks(&mut contexts);

    Ok(contexts)
}

pub fn resolve_overlay_vault(working_dir: &Path) -> Result<ResolvedOverlayVault, VaultError> {
    let roots = VaultRoots::discover(working_dir);
    let contexts = list_context_files(&roots)?;

    Ok(ResolvedOverlayVault { roots, contexts })
}

pub fn discover_existing_context_files(
    working_dir: &Path,
) -> Result<Vec<ContextFragment>, VaultError> {
    let mut contexts = Vec::new();
    for result in discover_existing_context_file_results(working_dir)? {
        contexts.push(context_from_discovery_result(result)?);
    }
    hydrate_backlinks(&mut contexts);
    Ok(contexts)
}

pub fn discover_existing_context_file_results(
    working_dir: &Path,
) -> Result<Vec<ContextDiscoveryResult>, VaultError> {
    let roots = VaultRoots::discover(working_dir);
    let mut discovered = Vec::new();
    let mut seen_paths = HashSet::new();
    let reserved_entry_keys = existing_materialized_entry_keys(&roots)?;
    let imported_source_entries = existing_import_source_entries(&roots)?;

    for root in managed_context_roots(&roots) {
        if root.exists() {
            collect_seen_markdown_paths(&root, &mut seen_paths)?;
        }
    }

    for scan_root in discovery_scan_roots(&roots, working_dir)? {
        match scan_root.kind {
            DiscoveryScanRootKind::Context {
                recursive_named_contexts,
            } => {
                collect_named_context_candidates(
                    &scan_root.path,
                    &scan_root.path,
                    scan_root.scope,
                    recursive_named_contexts,
                    &mut seen_paths,
                    &mut discovered,
                )?;
                collect_skill_context_candidates(
                    &scan_root.path,
                    &scan_root.path,
                    scan_root.scope,
                    recursive_named_contexts,
                    &mut seen_paths,
                    &mut discovered,
                )?;
                collect_agent_context_candidates(
                    &scan_root.path,
                    &scan_root.path,
                    scan_root.scope,
                    &mut seen_paths,
                    &mut discovered,
                )?;
            }
            DiscoveryScanRootKind::Skill => collect_configured_skill_files_in_dir(
                &scan_root.path,
                &scan_root.path,
                scan_root.scope,
                &mut seen_paths,
                &mut discovered,
            )?,
        }
    }

    resolve_discovery_entry_conflicts(
        &mut discovered,
        reserved_entry_keys,
        &imported_source_entries,
    )?;

    discovered.sort_by(|left, right| {
        left.metadata
            .vault_scope
            .cmp(&right.metadata.vault_scope)
            .then_with(|| left.metadata.folder_path.cmp(&right.metadata.folder_path))
            .then_with(|| left.metadata.title.cmp(&right.metadata.title))
    });

    Ok(discovered)
}

#[derive(Debug, Clone, Eq, PartialEq)]
struct DiscoveryScanRoot {
    path: PathBuf,
    scope: VaultScope,
    kind: DiscoveryScanRootKind,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
enum DiscoveryScanRootKind {
    Context { recursive_named_contexts: bool },
    Skill,
}

fn discovery_scan_roots(
    roots: &VaultRoots,
    working_dir: &Path,
) -> Result<Vec<DiscoveryScanRoot>, VaultError> {
    let configured_scan_roots =
        load_configured_scan_roots(roots, working_dir).map_err(|error| {
            VaultError::Io(format!("failed to load configured scan roots: {error}"))
        })?;
    let configured_skill_scan_roots = load_configured_skill_scan_roots(roots, working_dir)
        .map_err(|error| {
            VaultError::Io(format!(
                "failed to load configured skill scan roots: {error}"
            ))
        })?;

    if !configured_scan_roots.is_empty() {
        let mut scan_roots = configured_scan_roots
            .into_iter()
            .map(|root| DiscoveryScanRoot {
                path: root.path,
                scope: root.scope,
                kind: DiscoveryScanRootKind::Context {
                    recursive_named_contexts: true,
                },
            })
            .collect::<Vec<_>>();
        scan_roots.extend(
            configured_skill_scan_roots
                .into_iter()
                .map(|root| DiscoveryScanRoot {
                    path: root.path,
                    scope: root.scope,
                    kind: DiscoveryScanRootKind::Skill,
                }),
        );
        return Ok(scan_roots);
    }

    let mut scan_roots = vec![DiscoveryScanRoot {
        path: working_dir.to_path_buf(),
        scope: VaultScope::Local,
        kind: DiscoveryScanRootKind::Context {
            recursive_named_contexts: false,
        },
    }];

    if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
        scan_roots.push(DiscoveryScanRoot {
            path: home,
            scope: VaultScope::Global,
            kind: DiscoveryScanRootKind::Context {
                recursive_named_contexts: false,
            },
        });
    }

    scan_roots.extend(
        configured_skill_scan_roots
            .into_iter()
            .map(|root| DiscoveryScanRoot {
                path: root.path,
                scope: root.scope,
                kind: DiscoveryScanRootKind::Skill,
            }),
    );

    Ok(scan_roots)
}

pub fn list_context_files_with_discovered(
    working_dir: &Path,
) -> Result<Vec<ContextFragment>, VaultError> {
    let roots = VaultRoots::discover(working_dir);
    let mut contexts = list_context_files(&roots)?;
    let mut seen_paths: HashSet<PathBuf> = contexts
        .iter()
        .map(|context| context.file_path.clone())
        .collect();

    for context in discover_existing_context_files(working_dir)? {
        if seen_paths.insert(context.file_path.clone()) {
            contexts.push(context);
        }
    }

    contexts.sort_by(|left, right| {
        left.vault_scope
            .cmp(&right.vault_scope)
            .then_with(|| left.folder_path.cmp(&right.folder_path))
            .then_with(|| left.title.cmp(&right.title))
    });
    let mut contexts = resolve_overlay(contexts);
    hydrate_backlinks(&mut contexts);

    Ok(contexts)
}

pub fn materialize_discovered_context_files(
    working_dir: &Path,
) -> Result<Vec<ContextFragment>, VaultError> {
    let roots = VaultRoots::discover(working_dir);
    let discovered = discover_existing_context_file_results(working_dir)?;
    let mut materialized = Vec::new();

    for result in discovered {
        materialized.push(materialize_discovered_context_file(&roots, result)?);
    }

    Ok(materialized)
}

pub fn reindex_markdown_contexts(working_dir: &Path) -> Result<VaultReindexReport, VaultError> {
    let global = initialize_global_vault(working_dir)?;
    let local = initialize_project_local_vault(working_dir)?;
    let roots = VaultRoots {
        global_root: global.global_root.clone(),
        local_root: Some(local.local_root.clone()),
    };
    let mut contexts = Vec::new();

    collect_context_files(
        &managed_contexts_dir(&roots.global_root),
        VaultScope::Global,
        &mut contexts,
    )?;
    if let Some(local_root) = &roots.local_root {
        collect_context_files(
            &managed_contexts_dir(local_root),
            VaultScope::Local,
            &mut contexts,
        )?;
    }

    let discovered = discover_existing_context_files(working_dir)?;
    let discovered_markdown_files = discovered.len();
    contexts.extend(discovered);
    hydrate_backlinks(&mut contexts);

    let now = unix_timestamp_now();
    let global_records =
        markdown_index_records_for_scope(&contexts, VaultScope::Global, &roots.global_root, now)?;
    let local_records = markdown_index_records_for_scope(
        &contexts,
        VaultScope::Local,
        roots
            .local_root
            .as_ref()
            .ok_or(VaultError::MissingLocalVault)?,
        now,
    )?;

    let global_report = full_reindex_markdown_files(&global.sqlite_index_path, &global_records)
        .map_err(|error| {
            VaultError::Index(format!(
                "failed to reindex global vault {}: {error}",
                global.sqlite_index_path.display()
            ))
        })?;
    let local_report = full_reindex_markdown_files(&local.sqlite_index_path, &local_records)
        .map_err(|error| {
            VaultError::Index(format!(
                "failed to reindex project-local vault {}: {error}",
                local.sqlite_index_path.display()
            ))
        })?;

    Ok(VaultReindexReport {
        global: global_report,
        local: Some(local_report),
        discovered_markdown_files,
    })
}

pub fn sync_markdown_context_index_event(
    working_dir: &Path,
    event: &ContextFileChangeEvent,
) -> Result<Option<IncrementalMarkdownIndexReport>, VaultError> {
    let (vault_root, database_path) = initialize_index_for_scope(working_dir, event.vault_scope)?;

    match event.kind {
        ContextFileChangeKind::Create | ContextFileChangeKind::Update => {
            let context = context_fragment_from_change_event(event)?;
            let now = unix_timestamp_now();
            let title_targets =
                title_target_paths_for_scope(&vault_root, event.vault_scope, Some(&context))?;
            let record =
                markdown_index_record_from_context(&context, &vault_root, &title_targets, now)?;
            let report =
                upsert_markdown_file_index_record(&database_path, &record).map_err(|error| {
                    VaultError::Index(format!(
                        "failed to incrementally sync markdown index for {}: {error}",
                        event.path.display()
                    ))
                })?;
            Ok(Some(report))
        }
        ContextFileChangeKind::Move => {
            update_managed_import_metadata_for_move(&vault_root, event)?;
            let previous_path = event.previous_path.as_ref().ok_or_else(|| {
                VaultError::Io(format!(
                    "move event for {} did not include the previous path",
                    event.path.display()
                ))
            })?;
            let context = context_fragment_from_change_event(event)?;
            let now = unix_timestamp_now();
            let title_targets =
                title_target_paths_for_scope(&vault_root, event.vault_scope, Some(&context))?;
            let record =
                markdown_index_record_from_context(&context, &vault_root, &title_targets, now)?;
            let report = move_markdown_file_index_record(&database_path, previous_path, &record)
                .map_err(|error| {
                    VaultError::Index(format!(
                        "failed to move markdown index row from {} to {}: {error}",
                        previous_path.display(),
                        event.path.display()
                    ))
                })?;
            Ok(Some(report))
        }
        ContextFileChangeKind::Delete => {
            cleanup_managed_import_metadata_for_delete(&vault_root, event)?;
            remove_markdown_file_index_record(&database_path, &event.path).map_err(|error| {
                VaultError::Index(format!(
                    "failed to remove stale markdown index row for {}: {error}",
                    event.path.display()
                ))
            })?;
            Ok(Some(IncrementalMarkdownIndexReport {
                database_path: Some(database_path),
                indexed_markdown_files: 0,
                indexed_tags: 0,
                indexed_links: 0,
            }))
        }
    }
}

pub fn sync_markdown_context_index_events(
    working_dir: &Path,
    events: &[ContextFileChangeEvent],
) -> Result<Vec<IncrementalMarkdownIndexReport>, VaultError> {
    let mut reports = Vec::new();
    for event in events {
        if let Some(report) = sync_markdown_context_index_event(working_dir, event)? {
            reports.push(report);
        }
    }
    Ok(reports)
}

pub fn lookup_markdown_context_index(
    working_dir: &Path,
    path: &Path,
) -> Result<Option<OverlayMarkdownIndexLookup>, VaultError> {
    let roots = VaultRoots::discover(working_dir);
    let path = resolve_context_path_from_overlay(working_dir, path)
        .map(|context| context.file_path)
        .unwrap_or_else(|_| path.to_path_buf());

    for (vault_root, database_path) in index_lookup_candidates(&roots, working_dir)? {
        if let Some(lookup) =
            markdown_file_index_lookup(&database_path, &path).map_err(|error| {
                VaultError::Index(format!(
                    "failed to query markdown index {} for {}: {error}",
                    database_path.display(),
                    path.display()
                ))
            })?
        {
            return Ok(Some(OverlayMarkdownIndexLookup {
                lookup,
                sqlite_index_path: database_path,
            }));
        }

        if path.starts_with(managed_contexts_dir(&vault_root)) {
            break;
        }
    }

    Ok(None)
}

pub fn lookup_markdown_contexts_by_tag(
    working_dir: &Path,
    tag: &str,
) -> Result<Vec<MarkdownFileMetadataRecord>, VaultError> {
    let roots = VaultRoots::discover(working_dir);
    let mut records = Vec::new();

    for (_, database_path) in index_lookup_candidates(&roots, working_dir)? {
        records.extend(
            markdown_file_metadata_records_by_tag(&database_path, tag).map_err(|error| {
                VaultError::Index(format!(
                    "failed to query markdown index {} by tag {tag}: {error}",
                    database_path.display()
                ))
            })?,
        );
    }

    records.sort_by(|left, right| {
        left.vault_scope
            .cmp(&right.vault_scope)
            .then_with(|| left.relative_path.cmp(&right.relative_path))
    });
    records.dedup_by(|left, right| left.path == right.path);
    Ok(records)
}

fn index_lookup_candidates(
    roots: &VaultRoots,
    working_dir: &Path,
) -> Result<Vec<(PathBuf, PathBuf)>, VaultError> {
    let global = initialize_global_vault(working_dir)?;
    let mut candidates = vec![(global.global_root, global.sqlite_index_path)];

    if roots.local_root.is_some() {
        let local = initialize_project_local_vault(working_dir)?;
        candidates.push((local.local_root, local.sqlite_index_path));
    }

    Ok(candidates)
}

pub fn materialize_discovered_context_file(
    roots: &VaultRoots,
    result: ContextDiscoveryResult,
) -> Result<ContextFragment, VaultError> {
    let content = read_markdown_context_file(&result.file_path)?;
    let result = analyze_discovery_result_before_import(result, &content)?;
    let root = match result.metadata.vault_scope {
        VaultScope::Global => roots.global_root.clone(),
        VaultScope::Local => roots
            .local_root
            .clone()
            .ok_or(VaultError::MissingLocalVault)?,
    };
    let folder_path = normalize_context_folder(&result.metadata.folder_path)?;
    let contexts_dir = managed_contexts_dir(&root);
    let target_dir = contexts_dir.join(&folder_path);
    let file_name = normalized_materialized_file_name(&result.file_name)?;

    let import_metadata = load_import_metadata_index(&root)?;
    if let Some(target_path) = materialized_target_path_for_import_source(
        &import_metadata,
        &contexts_dir,
        &result.file_path,
    )? {
        write_materialized_context_if_changed(&target_path, &content)?;
        let persisted_metadata = persist_import_metadata(
            &root,
            &contexts_dir,
            &target_path,
            &result.file_path,
            result.source_type,
            &result.metadata,
        )?;
        let result = discovery_result_with_persisted_import_metadata(result, &persisted_metadata);
        let folder_path = materialized_folder_path(&contexts_dir, &target_path)?;
        return Ok(context_fragment_from_materialized_file(
            target_path,
            folder_path,
            content,
            result,
        ));
    }

    if let Some(target_path) = materialized_target_path_for_moved_import_source(
        &import_metadata,
        &contexts_dir,
        &result.file_path,
        result.source_type,
        &content,
    )? {
        write_materialized_context_if_changed(&target_path, &content)?;
        let persisted_metadata = persist_import_metadata(
            &root,
            &contexts_dir,
            &target_path,
            &result.file_path,
            result.source_type,
            &result.metadata,
        )?;
        let result = discovery_result_with_persisted_import_metadata(result, &persisted_metadata);
        let folder_path = materialized_folder_path(&contexts_dir, &target_path)?;
        return Ok(context_fragment_from_materialized_file(
            target_path,
            folder_path,
            content,
            result,
        ));
    }

    let target_path = materialized_target_path(&target_dir, &file_name, &content)?;

    if target_path.is_file()
        && fs::read_to_string(&target_path).map_err(|error| {
            VaultError::Io(format!(
                "failed to read existing materialized context file {}: {error}",
                target_path.display()
            ))
        })? == content
    {
        let persisted_metadata = persist_import_metadata(
            &root,
            &contexts_dir,
            &target_path,
            &result.file_path,
            result.source_type,
            &result.metadata,
        )?;
        let result = discovery_result_with_persisted_import_metadata(result, &persisted_metadata);
        return Ok(context_fragment_from_materialized_file(
            target_path,
            folder_path,
            content,
            result,
        ));
    }

    fs::create_dir_all(&target_dir).map_err(|error| {
        VaultError::Io(format!(
            "failed to create materialized context directory {}: {error}",
            target_dir.display()
        ))
    })?;

    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&target_path)
        .map_err(|error| {
            if error.kind() == std::io::ErrorKind::AlreadyExists {
                VaultError::DuplicateContext(target_path.clone())
            } else {
                VaultError::Io(format!(
                    "failed to create materialized context file {}: {error}",
                    target_path.display()
                ))
            }
        })?;

    file.write_all(content.as_bytes()).map_err(|error| {
        VaultError::Io(format!(
            "failed to write materialized context file {}: {error}",
            target_path.display()
        ))
    })?;
    let persisted_metadata = persist_import_metadata(
        &root,
        &contexts_dir,
        &target_path,
        &result.file_path,
        result.source_type,
        &result.metadata,
    )?;
    let result = discovery_result_with_persisted_import_metadata(result, &persisted_metadata);

    Ok(context_fragment_from_materialized_file(
        target_path,
        folder_path,
        content,
        result,
    ))
}

pub fn read_resolved_context_markdown(
    working_dir: &Path,
    path: &Path,
) -> Result<String, VaultError> {
    let context = resolve_context_path_from_overlay(working_dir, path)?;
    read_markdown_context_file(&context.file_path)
}

pub fn update_resolved_context_markdown(
    working_dir: &Path,
    path: &Path,
    content: &str,
) -> Result<String, VaultError> {
    let context = resolve_context_path_from_overlay(working_dir, path)?;
    update_markdown_context_file(&context.file_path, content)
}

pub fn delete_resolved_context_markdown(
    working_dir: &Path,
    path: &Path,
) -> Result<PathBuf, VaultError> {
    let context = resolve_context_path_from_overlay(working_dir, path)?;
    delete_markdown_context_file(&context.file_path)
}

pub fn canonical_vault_entry_key(context: &ContextFragment) -> VaultEntryKey {
    let mut parts = canonical_path_parts(&context.folder_path);

    if let Some(file_name) = context.file_path.file_name().and_then(|name| name.to_str()) {
        parts.push(canonical_path_segment(file_name));
    } else {
        parts.push(canonical_path_segment(&format!("{}.md", context.title)));
    }

    VaultEntryKey {
        relative_path: parts.join("/"),
    }
}

pub fn resolve_overlay(contexts: Vec<ContextFragment>) -> Vec<ContextFragment> {
    let mut resolved: Vec<ContextFragment> = Vec::new();
    let mut index_by_key: BTreeMap<VaultEntryKey, usize> = BTreeMap::new();

    for context in contexts {
        let key = canonical_vault_entry_key(&context);
        match index_by_key.get(&key).copied() {
            Some(existing_index) if resolved[existing_index].vault_scope == VaultScope::Local => {}
            Some(existing_index) if context.vault_scope == VaultScope::Local => {
                resolved[existing_index] = context;
            }
            Some(_) => {}
            None => {
                let next_index = resolved.len();
                resolved.push(context);
                index_by_key.insert(key, next_index);
            }
        }
    }

    resolved
}

fn resolve_context_path_from_overlay(
    working_dir: &Path,
    path: &Path,
) -> Result<ContextFragment, VaultError> {
    let contexts = list_context_files_with_discovered(working_dir)?;
    let requested_path = path.to_path_buf();

    contexts
        .into_iter()
        .find(|context| context.file_path == requested_path)
        .ok_or_else(|| {
            VaultError::Io(format!(
                "context file is not part of the resolved vault overlay: {}",
                path.display()
            ))
        })
}

fn collect_context_files(
    contexts_dir: &Path,
    scope: VaultScope,
    contexts: &mut Vec<ContextFragment>,
) -> Result<(), VaultError> {
    if !contexts_dir.exists() {
        return Ok(());
    }

    let import_metadata =
        load_import_metadata_index(contexts_dir.parent().unwrap_or_else(|| Path::new(".")))?;
    collect_context_files_in_dir(
        contexts_dir,
        contexts_dir,
        scope,
        &import_metadata,
        contexts,
    )
}

fn title_target_paths_for_scope(
    vault_root: &Path,
    scope: VaultScope,
    current_context: Option<&ContextFragment>,
) -> Result<BTreeMap<String, PathBuf>, VaultError> {
    let mut contexts = Vec::new();
    collect_context_files(&managed_contexts_dir(vault_root), scope, &mut contexts)?;

    if let Some(current_context) = current_context {
        if let Some(existing) = contexts
            .iter_mut()
            .find(|context| context.file_path == current_context.file_path)
        {
            *existing = current_context.clone();
        } else {
            contexts.push(current_context.clone());
        }
    }

    let context_refs = contexts.iter().collect::<Vec<_>>();
    Ok(title_target_paths(&context_refs))
}

fn initialize_index_for_scope(
    working_dir: &Path,
    scope: VaultScope,
) -> Result<(PathBuf, PathBuf), VaultError> {
    match scope {
        VaultScope::Global => {
            let initialized = initialize_global_vault(working_dir)?;
            Ok((initialized.global_root, initialized.sqlite_index_path))
        }
        VaultScope::Local => {
            let initialized = initialize_project_local_vault(working_dir)?;
            Ok((initialized.local_root, initialized.sqlite_index_path))
        }
    }
}

fn context_fragment_from_change_event(
    event: &ContextFileChangeEvent,
) -> Result<ContextFragment, VaultError> {
    match event.root_kind {
        ContextWatchRootKind::ManagedVault => managed_context_fragment_from_change_event(event),
        ContextWatchRootKind::ConfiguredScanRoot => {
            discovered_context_fragment_from_change_event(event, None, Vec::new())
        }
        ContextWatchRootKind::ConfiguredSkillRoot => discovered_context_fragment_from_change_event(
            event,
            Some(Classification::Shared),
            vec!["skills".to_string()],
        ),
    }
}

fn managed_context_fragment_from_change_event(
    event: &ContextFileChangeEvent,
) -> Result<ContextFragment, VaultError> {
    let content = read_markdown_context_file(&event.path)?;
    let file_name = event
        .path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("context.md");
    let folder_path = event
        .relative_path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .map(Path::to_path_buf)
        .unwrap_or_default();
    let vault_root = event.root_path.parent().unwrap_or(&event.root_path);
    let import_metadata = load_import_metadata_index(vault_root)?;
    let import_record = import_metadata_record_for(&import_metadata, &event.root_path, &event.path);
    let classification = import_record
        .and_then(|record| record.classification)
        .unwrap_or(Classification::Shared);
    let inferred_classification =
        import_record.and_then(|record| record.inferred_classification.or(record.classification));
    let import_classification_suggestion = import_record.and_then(|record| {
        record
            .import_classification_suggestion
            .or(record.inferred_classification)
    });
    let tags = import_record
        .map(|record| record.tags.clone())
        .unwrap_or_default();
    let llm_classification_status = import_record
        .and_then(|record| record.llm_classification_status)
        .unwrap_or(ClassificationStatus::Pending);

    Ok(ContextFragment {
        context_id: Uuid::new_v4(),
        title: context_title(file_name),
        content: content.clone(),
        file_path: event.path.clone(),
        vault_scope: event.vault_scope,
        classification,
        import_classification_suggestion,
        inferred_classification,
        tags,
        folder_path,
        wikilinks: extract_wikilinks(&content),
        backlinks: Vec::new(),
        import_source: import_record.map(|record| record.import_source.clone()),
        import_source_type: import_record.map(|record| record.source_type),
        llm_classification_status,
    })
}

fn discovered_context_fragment_from_change_event(
    event: &ContextFileChangeEvent,
    classification_override: Option<Classification>,
    classification_tags: Vec<String>,
) -> Result<ContextFragment, VaultError> {
    let mut seen_paths = HashSet::new();
    let mut discovered = Vec::new();
    collect_candidate_file_with_override(
        &event.path,
        &event.root_path,
        event.vault_scope,
        classification_override,
        classification_tags,
        &mut seen_paths,
        &mut discovered,
    )?;
    let result = discovered
        .into_iter()
        .next()
        .ok_or_else(|| VaultError::MissingContext(event.path.clone()))?;
    context_from_discovery_result(result)
}

fn collect_context_files_in_dir(
    contexts_dir: &Path,
    current_dir: &Path,
    scope: VaultScope,
    import_metadata: &ImportMetadataIndex,
    contexts: &mut Vec<ContextFragment>,
) -> Result<(), VaultError> {
    let entries = fs::read_dir(current_dir).map_err(|error| {
        VaultError::Io(format!(
            "failed to read context directory {}: {error}",
            current_dir.display()
        ))
    })?;

    for entry in entries {
        let entry = entry.map_err(|error| {
            VaultError::Io(format!(
                "failed to read context directory entry in {}: {error}",
                current_dir.display()
            ))
        })?;
        let path = entry.path();
        let metadata = entry.metadata().map_err(|error| {
            VaultError::Io(format!(
                "failed to read context path metadata {}: {error}",
                path.display()
            ))
        })?;

        if metadata.is_dir() {
            collect_context_files_in_dir(contexts_dir, &path, scope, import_metadata, contexts)?;
            continue;
        }

        if !metadata.is_file()
            || path.extension().and_then(|extension| extension.to_str()) != Some("md")
        {
            continue;
        }

        let mut content = String::new();
        fs::File::open(&path)
            .and_then(|mut file| file.read_to_string(&mut content))
            .map_err(|error| {
                VaultError::Io(format!(
                    "failed to read context file {}: {error}",
                    path.display()
                ))
            })?;

        let folder_path = path
            .parent()
            .and_then(|parent| parent.strip_prefix(contexts_dir).ok())
            .filter(|relative| !relative.as_os_str().is_empty())
            .map(Path::to_path_buf)
            .unwrap_or_default();
        let file_name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default();
        let import_record = import_metadata_record_for(import_metadata, contexts_dir, &path);
        let classification = import_record
            .and_then(|record| record.classification)
            .unwrap_or(Classification::Shared);
        let inferred_classification = import_record
            .and_then(|record| record.inferred_classification.or(record.classification));
        let import_classification_suggestion = import_record.and_then(|record| {
            record
                .import_classification_suggestion
                .or(record.inferred_classification)
        });
        let tags = import_record
            .map(|record| record.tags.clone())
            .unwrap_or_default();
        let llm_classification_status = import_record
            .and_then(|record| record.llm_classification_status)
            .unwrap_or(ClassificationStatus::Pending);

        contexts.push(ContextFragment {
            context_id: Uuid::new_v4(),
            title: context_title(file_name),
            content: content.clone(),
            file_path: path,
            vault_scope: scope,
            classification,
            import_classification_suggestion,
            inferred_classification,
            tags,
            folder_path,
            wikilinks: extract_wikilinks(&content),
            backlinks: Vec::new(),
            import_source: import_record.map(|record| record.import_source.clone()),
            import_source_type: import_record.map(|record| record.source_type),
            llm_classification_status,
        });
    }

    Ok(())
}

fn managed_context_roots(roots: &VaultRoots) -> Vec<PathBuf> {
    let mut managed_roots = vec![managed_contexts_dir(&roots.global_root)];
    if let Some(local_root) = &roots.local_root {
        managed_roots.push(managed_contexts_dir(local_root));
    }
    managed_roots
}

fn collect_seen_markdown_paths(
    current_dir: &Path,
    seen_paths: &mut HashSet<PathBuf>,
) -> Result<(), VaultError> {
    let entries = fs::read_dir(current_dir).map_err(|error| {
        VaultError::Io(format!(
            "failed to read managed context directory {}: {error}",
            current_dir.display()
        ))
    })?;

    for entry in entries {
        let entry = entry.map_err(|error| {
            VaultError::Io(format!(
                "failed to read managed context directory entry in {}: {error}",
                current_dir.display()
            ))
        })?;
        let path = entry.path();
        let metadata = entry.metadata().map_err(|error| {
            VaultError::Io(format!(
                "failed to read managed context path metadata {}: {error}",
                path.display()
            ))
        })?;

        if metadata.is_dir() {
            collect_seen_markdown_paths(&path, seen_paths)?;
        } else if metadata.is_file() && is_markdown_file(&path) {
            seen_paths.insert(path.canonicalize().unwrap_or(path));
        }
    }

    Ok(())
}

#[derive(Debug, Clone, Eq, PartialEq, Ord, PartialOrd)]
struct MaterializedEntryKey {
    scope: VaultScope,
    relative_path: String,
}

#[derive(Debug, Clone, Eq, PartialEq, Ord, PartialOrd)]
struct ImportedSourceKey {
    scope: VaultScope,
    import_source: PathBuf,
}

#[derive(Debug, Clone, Eq, PartialEq)]
struct ImportedSourceEntry {
    context_relative_path: PathBuf,
}

fn existing_materialized_entry_keys(
    roots: &VaultRoots,
) -> Result<BTreeSet<MaterializedEntryKey>, VaultError> {
    let mut keys = BTreeSet::new();
    collect_existing_materialized_entry_keys(
        &managed_contexts_dir(&roots.global_root),
        &managed_contexts_dir(&roots.global_root),
        VaultScope::Global,
        &mut keys,
    )?;
    if let Some(local_root) = &roots.local_root {
        collect_existing_materialized_entry_keys(
            &managed_contexts_dir(local_root),
            &managed_contexts_dir(local_root),
            VaultScope::Local,
            &mut keys,
        )?;
    }
    Ok(keys)
}

fn existing_import_source_entries(
    roots: &VaultRoots,
) -> Result<BTreeMap<ImportedSourceKey, ImportedSourceEntry>, VaultError> {
    let mut entries = BTreeMap::new();
    collect_existing_import_source_entries(&roots.global_root, VaultScope::Global, &mut entries)?;
    if let Some(local_root) = &roots.local_root {
        collect_existing_import_source_entries(local_root, VaultScope::Local, &mut entries)?;
    }
    Ok(entries)
}

fn collect_existing_import_source_entries(
    root: &Path,
    scope: VaultScope,
    entries: &mut BTreeMap<ImportedSourceKey, ImportedSourceEntry>,
) -> Result<(), VaultError> {
    let index = load_import_metadata_index(root)?;

    for record in index.imports {
        entries.insert(
            ImportedSourceKey {
                scope,
                import_source: normalize_import_source_path(&record.import_source),
            },
            ImportedSourceEntry {
                context_relative_path: record.context_relative_path,
            },
        );
    }

    Ok(())
}

fn collect_existing_materialized_entry_keys(
    contexts_dir: &Path,
    current_dir: &Path,
    scope: VaultScope,
    keys: &mut BTreeSet<MaterializedEntryKey>,
) -> Result<(), VaultError> {
    if !current_dir.exists() {
        return Ok(());
    }

    let entries = fs::read_dir(current_dir).map_err(|error| {
        VaultError::Io(format!(
            "failed to read managed context directory {}: {error}",
            current_dir.display()
        ))
    })?;

    for entry in entries {
        let entry = entry.map_err(|error| {
            VaultError::Io(format!(
                "failed to read managed context directory entry in {}: {error}",
                current_dir.display()
            ))
        })?;
        let path = entry.path();
        let metadata = entry.metadata().map_err(|error| {
            VaultError::Io(format!(
                "failed to read managed context path metadata {}: {error}",
                path.display()
            ))
        })?;

        if metadata.is_dir() {
            collect_existing_materialized_entry_keys(contexts_dir, &path, scope, keys)?;
        } else if metadata.is_file() && is_markdown_file(&path) {
            let relative_path = path.strip_prefix(contexts_dir).map_err(|_| {
                VaultError::Io(format!(
                    "managed context path {} is outside contexts directory {}",
                    path.display(),
                    contexts_dir.display()
                ))
            })?;
            keys.insert(MaterializedEntryKey {
                scope,
                relative_path: canonical_path_parts(relative_path).join("/"),
            });
        }
    }

    Ok(())
}

fn resolve_discovery_entry_conflicts(
    discovered: &mut [ContextDiscoveryResult],
    mut used_keys: BTreeSet<MaterializedEntryKey>,
    imported_source_entries: &BTreeMap<ImportedSourceKey, ImportedSourceEntry>,
) -> Result<(), VaultError> {
    discovered.sort_by(|left, right| {
        left.metadata
            .vault_scope
            .cmp(&right.metadata.vault_scope)
            .then_with(|| left.metadata.folder_path.cmp(&right.metadata.folder_path))
            .then_with(|| left.file_name.cmp(&right.file_name))
            .then_with(|| left.file_path.cmp(&right.file_path))
    });

    for result in discovered {
        if let Some(imported_entry) = imported_source_entries.get(&ImportedSourceKey {
            scope: result.metadata.vault_scope,
            import_source: normalize_import_source_path(&result.file_path),
        }) {
            apply_imported_source_entry(result, imported_entry)?;
        }

        let original_file_name = normalized_materialized_file_name(&result.file_name)?;
        let had_conflict = used_keys.contains(&discovery_entry_key(
            result.metadata.vault_scope,
            &result.metadata.folder_path,
            &original_file_name,
        ));
        if imported_source_entries.contains_key(&ImportedSourceKey {
            scope: result.metadata.vault_scope,
            import_source: normalize_import_source_path(&result.file_path),
        }) {
            used_keys.remove(&discovery_entry_key(
                result.metadata.vault_scope,
                &result.metadata.folder_path,
                &original_file_name,
            ));
        }
        let resolved_file_name = unique_discovery_file_name(
            result.metadata.vault_scope,
            &result.metadata.folder_path,
            &original_file_name,
            &mut used_keys,
        )?;

        if resolved_file_name != result.file_name {
            result.file_name = resolved_file_name;
            result.metadata.title = context_title(&result.file_name);
            if had_conflict
                && !result
                    .metadata
                    .tags
                    .iter()
                    .any(|tag| tag == "name-conflict")
            {
                result.metadata.tags.push("name-conflict".to_string());
            }
        }
    }

    Ok(())
}

fn apply_imported_source_entry(
    result: &mut ContextDiscoveryResult,
    imported_entry: &ImportedSourceEntry,
) -> Result<(), VaultError> {
    let file_name = imported_entry
        .context_relative_path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| {
            VaultError::InvalidFileName(format!(
                "import metadata context path is invalid: {}",
                imported_entry.context_relative_path.display()
            ))
        })?;
    validate_context_file_name(file_name)?;
    let folder_path = imported_entry
        .context_relative_path
        .parent()
        .map(normalize_context_folder)
        .transpose()?
        .unwrap_or_default();

    result.file_name = file_name.to_string();
    result.metadata.title = context_title(file_name);
    result.metadata.folder_path = folder_path;

    Ok(())
}

fn unique_discovery_file_name(
    scope: VaultScope,
    folder_path: &Path,
    file_name: &str,
    used_keys: &mut BTreeSet<MaterializedEntryKey>,
) -> Result<String, VaultError> {
    let stem = Path::new(file_name)
        .file_stem()
        .and_then(|stem| stem.to_str())
        .ok_or_else(|| {
            VaultError::InvalidFileName(format!("context filename is invalid: {file_name}"))
        })?;

    for suffix in 1.. {
        let candidate = if suffix == 1 {
            file_name.to_string()
        } else {
            format!("{stem}-{suffix}.md")
        };
        validate_context_file_name(&candidate)?;
        let key = discovery_entry_key(scope, folder_path, &candidate);
        if used_keys.insert(key) {
            return Ok(candidate);
        }
    }

    unreachable!("unbounded suffix search should always return a discovery file name")
}

fn discovery_entry_key(
    scope: VaultScope,
    folder_path: &Path,
    file_name: &str,
) -> MaterializedEntryKey {
    let mut parts = canonical_path_parts(folder_path);
    parts.push(canonical_path_segment(file_name));
    MaterializedEntryKey {
        scope,
        relative_path: parts.join("/"),
    }
}

fn collect_named_context_candidates(
    scan_root: &Path,
    base_dir: &Path,
    scope: VaultScope,
    recursive: bool,
    seen_paths: &mut HashSet<PathBuf>,
    contexts: &mut Vec<ContextDiscoveryResult>,
) -> Result<(), VaultError> {
    if recursive {
        return collect_known_named_context_files_in_dir(
            scan_root, base_dir, scope, seen_paths, contexts,
        );
    }

    for relative_dir in MAIN_AGENT_DIRECTORY_PATTERNS {
        for file_name in MAIN_AGENT_FILE_NAMES {
            let path = scan_root.join(relative_dir).join(file_name);
            collect_candidate_file(&path, base_dir, scope, seen_paths, contexts)?;
        }
    }

    Ok(())
}

fn collect_known_named_context_files_in_dir(
    current_dir: &Path,
    base_dir: &Path,
    scope: VaultScope,
    seen_paths: &mut HashSet<PathBuf>,
    contexts: &mut Vec<ContextDiscoveryResult>,
) -> Result<(), VaultError> {
    for path in discover_markdown_files_recursive(current_dir, "configured scan root")? {
        if is_main_agent_file_name(&path) {
            collect_candidate_file(&path, base_dir, scope, seen_paths, contexts)?;
        }
    }

    Ok(())
}

fn collect_skill_context_candidates(
    scan_root: &Path,
    base_dir: &Path,
    scope: VaultScope,
    recursive: bool,
    seen_paths: &mut HashSet<PathBuf>,
    contexts: &mut Vec<ContextDiscoveryResult>,
) -> Result<(), VaultError> {
    if recursive {
        return collect_skill_context_candidates_in_tree(
            scan_root, base_dir, scope, seen_paths, contexts,
        );
    }

    for relative_dir in SKILL_DIRECTORY_PATTERNS {
        let dir = scan_root.join(relative_dir);
        if dir.is_dir() {
            collect_skill_files_in_dir(&dir, base_dir, scope, seen_paths, contexts)?;
        }
    }

    Ok(())
}

fn collect_skill_context_candidates_in_tree(
    current_dir: &Path,
    base_dir: &Path,
    scope: VaultScope,
    seen_paths: &mut HashSet<PathBuf>,
    contexts: &mut Vec<ContextDiscoveryResult>,
) -> Result<(), VaultError> {
    for path in discover_markdown_files_recursive(current_dir, "configured scan root")? {
        if is_skill_file_name(&path) || path_has_skill_segment_under_root(&path, current_dir) {
            collect_candidate_file_as_skill(&path, base_dir, scope, seen_paths, contexts)?;
        }
    }

    Ok(())
}

fn collect_agent_context_candidates(
    scan_root: &Path,
    base_dir: &Path,
    scope: VaultScope,
    seen_paths: &mut HashSet<PathBuf>,
    contexts: &mut Vec<ContextDiscoveryResult>,
) -> Result<(), VaultError> {
    for relative_dir in SUBAGENT_DIRECTORY_PATTERNS {
        let dir = scan_root.join(relative_dir);
        if dir.is_dir() {
            collect_markdown_context_files_in_dir(&dir, base_dir, scope, seen_paths, contexts)?;
        }
    }

    Ok(())
}

fn collect_skill_files_in_dir(
    current_dir: &Path,
    base_dir: &Path,
    scope: VaultScope,
    seen_paths: &mut HashSet<PathBuf>,
    contexts: &mut Vec<ContextDiscoveryResult>,
) -> Result<(), VaultError> {
    for path in discover_markdown_files_recursive(current_dir, "skills directory")? {
        collect_candidate_file(&path, base_dir, scope, seen_paths, contexts)?;
    }

    Ok(())
}

fn collect_configured_skill_files_in_dir(
    current_dir: &Path,
    base_dir: &Path,
    scope: VaultScope,
    seen_paths: &mut HashSet<PathBuf>,
    contexts: &mut Vec<ContextDiscoveryResult>,
) -> Result<(), VaultError> {
    for path in discover_markdown_files_recursive(current_dir, "configured skill scan root")? {
        collect_candidate_file_as_skill(&path, base_dir, scope, seen_paths, contexts)?;
    }

    Ok(())
}

fn collect_markdown_context_files_in_dir(
    current_dir: &Path,
    base_dir: &Path,
    scope: VaultScope,
    seen_paths: &mut HashSet<PathBuf>,
    contexts: &mut Vec<ContextDiscoveryResult>,
) -> Result<(), VaultError> {
    for path in discover_markdown_files_recursive(current_dir, "context directory")? {
        collect_candidate_file(&path, base_dir, scope, seen_paths, contexts)?;
    }

    Ok(())
}

fn discover_markdown_files_recursive(
    root: &Path,
    root_label: &str,
) -> Result<Vec<PathBuf>, VaultError> {
    let mut files = Vec::new();
    collect_markdown_files_recursive(root, root_label, &mut files)?;
    files.sort();
    Ok(files)
}

fn collect_markdown_files_recursive(
    current_dir: &Path,
    root_label: &str,
    files: &mut Vec<PathBuf>,
) -> Result<(), VaultError> {
    let entries = fs::read_dir(current_dir).map_err(|error| {
        VaultError::Io(format!(
            "failed to read {root_label} {}: {error}",
            current_dir.display()
        ))
    })?;

    for entry in entries {
        let entry = entry.map_err(|error| {
            VaultError::Io(format!(
                "failed to read {root_label} entry in {}: {error}",
                current_dir.display()
            ))
        })?;
        let path = entry.path();
        let metadata = entry.metadata().map_err(|error| {
            VaultError::Io(format!(
                "failed to read {root_label} path metadata {}: {error}",
                path.display()
            ))
        })?;

        if metadata.is_dir() {
            if should_exclude_discovery_directory(&path) {
                continue;
            }
            collect_markdown_files_recursive(&path, root_label, files)?;
        } else if metadata.is_file() && is_markdown_file(&path) {
            files.push(path);
        }
    }

    Ok(())
}

fn collect_candidate_file(
    path: &Path,
    base_dir: &Path,
    scope: VaultScope,
    seen_paths: &mut HashSet<PathBuf>,
    contexts: &mut Vec<ContextDiscoveryResult>,
) -> Result<(), VaultError> {
    collect_candidate_file_with_override(
        path,
        base_dir,
        scope,
        None,
        Vec::new(),
        seen_paths,
        contexts,
    )
}

fn collect_candidate_file_as_skill(
    path: &Path,
    base_dir: &Path,
    scope: VaultScope,
    seen_paths: &mut HashSet<PathBuf>,
    contexts: &mut Vec<ContextDiscoveryResult>,
) -> Result<(), VaultError> {
    collect_candidate_file_with_override(
        path,
        base_dir,
        scope,
        Some(Classification::Shared),
        vec!["skills".to_string()],
        seen_paths,
        contexts,
    )
}

fn collect_candidate_file_with_override(
    path: &Path,
    base_dir: &Path,
    scope: VaultScope,
    classification_override: Option<Classification>,
    classification_tags: Vec<String>,
    seen_paths: &mut HashSet<PathBuf>,
    contexts: &mut Vec<ContextDiscoveryResult>,
) -> Result<(), VaultError> {
    if !path.is_file() || !is_markdown_file(path) {
        return Ok(());
    }

    let normalized_path = path.canonicalize().map_err(|error| {
        VaultError::Io(format!(
            "failed to normalize discovered context file path {}: {error}",
            path.display()
        ))
    })?;
    if !seen_paths.insert(normalized_path.clone()) {
        return Ok(());
    }

    let mut content = String::new();
    fs::File::open(path)
        .and_then(|mut file| file.read_to_string(&mut content))
        .map_err(|error| {
            VaultError::Io(format!(
                "failed to read discovered context file {}: {error}",
                path.display()
            ))
        })?;

    let classification = classify_discovered_context(
        &normalized_path,
        &DiscoveredContextClassificationMetadata {
            root_source: Some(base_dir.to_path_buf()),
            tags: classification_tags,
            ..DiscoveredContextClassificationMetadata::default()
        },
    );

    let inferred_classification = classification.classification;
    let classification_kind = classification_override.unwrap_or(inferred_classification);
    let initial_source_type = detect_import_source_type(
        &normalized_path,
        classification_kind,
        classification_override,
    );
    let mut tags = normalized_discovery_tags(classification.tags);
    if classification_override == Some(Classification::Shared)
        && !tags.iter().any(|tag| tag == "skills")
    {
        tags.push("skills".to_string());
    }
    let import_classification =
        classify_import_markdown_content(&ImportTimeClassificationRequest {
            content: content.clone(),
            file_name: normalized_path
                .file_name()
                .and_then(|name| name.to_str())
                .map(str::to_string),
            folder_path: Some(classification.folder_path.clone()),
            import_source_type: Some(initial_source_type),
            existing_tags: tags.clone(),
        });
    let classification_kind =
        classification_override.unwrap_or(import_classification.classification);
    let source_type = detect_import_source_type(
        &normalized_path,
        classification_kind,
        classification_override,
    );
    tags = normalized_discovery_tags(import_classification.suggested_tags);
    if classification_override == Some(Classification::Shared)
        && !tags.iter().any(|tag| tag == "skills")
    {
        tags.push("skills".to_string());
    }

    contexts.push(ContextDiscoveryResult {
        file_path: normalized_path,
        file_name: classification.file_name.clone(),
        root_source: base_dir.to_path_buf(),
        source_type,
        metadata: ContextDiscoveryMetadata {
            title: context_title(&classification.file_name),
            vault_scope: scope,
            classification: classification_kind,
            import_classification_suggestion: Some(import_classification.classification),
            inferred_classification: Some(import_classification.classification),
            tags,
            folder_path: classification.folder_path,
            wikilinks: extract_wikilinks(&content),
            llm_classification_status: import_classification.status,
        },
    });

    Ok(())
}

fn analyze_discovery_result_before_import(
    mut result: ContextDiscoveryResult,
    content: &str,
) -> Result<ContextDiscoveryResult, VaultError> {
    let file_name = result
        .file_path
        .file_name()
        .and_then(|name| name.to_str())
        .map(str::to_string)
        .unwrap_or_else(|| result.file_name.clone());
    let classification = classify_import_markdown_content(&ImportTimeClassificationRequest {
        content: content.to_string(),
        file_name: Some(file_name),
        folder_path: Some(result.metadata.folder_path.clone()),
        import_source_type: Some(result.source_type),
        existing_tags: result.metadata.tags.clone(),
    });

    result.metadata.classification = classification.classification;
    result.metadata.import_classification_suggestion = Some(classification.classification);
    result.metadata.inferred_classification = Some(classification.classification);
    result.metadata.tags = normalized_discovery_tags(classification.suggested_tags);
    result.metadata.wikilinks = extract_wikilinks(content);
    result.metadata.llm_classification_status = classification.status;
    result.source_type =
        detect_import_source_type(&result.file_path, result.metadata.classification, None);

    Ok(result)
}

fn normalized_discovery_tags(tags: Vec<String>) -> Vec<String> {
    let mut normalized = Vec::new();

    for tag in tags {
        let tag = normalize_tag_token(&tag);
        if !tag.is_empty() && !normalized.iter().any(|existing| existing == &tag) {
            normalized.push(tag);
        }
    }

    normalized
}

fn context_from_discovery_result(
    result: ContextDiscoveryResult,
) -> Result<ContextFragment, VaultError> {
    let content = read_markdown_context_file(&result.file_path)?;

    Ok(ContextFragment {
        context_id: Uuid::new_v4(),
        title: result.metadata.title,
        content,
        file_path: result.file_path.clone(),
        vault_scope: result.metadata.vault_scope,
        classification: result.metadata.classification,
        import_classification_suggestion: result.metadata.import_classification_suggestion,
        inferred_classification: result.metadata.inferred_classification,
        tags: result.metadata.tags,
        folder_path: result.metadata.folder_path,
        wikilinks: result.metadata.wikilinks,
        backlinks: Vec::new(),
        import_source: Some(result.file_path),
        import_source_type: Some(result.source_type),
        llm_classification_status: result.metadata.llm_classification_status,
    })
}

fn normalized_materialized_file_name(file_name: &str) -> Result<String, VaultError> {
    let path = Path::new(file_name);
    let stem = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .ok_or_else(|| {
            VaultError::InvalidFileName(format!("context filename is invalid: {file_name}"))
        })?;
    let normalized = format!("{stem}.md");
    validate_context_file_name(&normalized)?;
    Ok(normalized)
}

fn materialized_target_path(
    target_dir: &Path,
    file_name: &str,
    content: &str,
) -> Result<PathBuf, VaultError> {
    let path = target_dir.join(file_name);
    if !path.exists() {
        return Ok(path);
    }
    if path.is_file()
        && fs::read_to_string(&path).map_err(|error| {
            VaultError::Io(format!(
                "failed to read existing materialized context file {}: {error}",
                path.display()
            ))
        })? == content
    {
        return Ok(path);
    }

    let stem = Path::new(file_name)
        .file_stem()
        .and_then(|stem| stem.to_str())
        .ok_or_else(|| {
            VaultError::InvalidFileName(format!("context filename is invalid: {file_name}"))
        })?;

    for suffix in 2.. {
        let candidate = target_dir.join(format!("{stem}-{suffix}.md"));
        if !candidate.exists() {
            return Ok(candidate);
        }
        if candidate.is_file()
            && fs::read_to_string(&candidate).map_err(|error| {
                VaultError::Io(format!(
                    "failed to read existing materialized context file {}: {error}",
                    candidate.display()
                ))
            })? == content
        {
            return Ok(candidate);
        }
    }

    unreachable!("unbounded suffix search should always return a materialization target")
}

fn materialized_target_path_for_import_source(
    index: &ImportMetadataIndex,
    contexts_dir: &Path,
    import_source: &Path,
) -> Result<Option<PathBuf>, VaultError> {
    let normalized_import_source = normalize_import_source_path(import_source);

    for record in &index.imports {
        if normalize_import_source_path(&record.import_source) == normalized_import_source {
            return Ok(Some(import_metadata_record_context_path(
                contexts_dir,
                record,
            )?));
        }
    }

    Ok(None)
}

fn materialized_target_path_for_moved_import_source(
    index: &ImportMetadataIndex,
    contexts_dir: &Path,
    import_source: &Path,
    source_type: ImportSourceType,
    content: &str,
) -> Result<Option<PathBuf>, VaultError> {
    let source_file_name = import_source
        .file_name()
        .and_then(|name| name.to_str())
        .map(normalized_materialized_file_name)
        .transpose()?;

    for record in &index.imports {
        if record.import_source.is_file() {
            continue;
        }
        if record.source_type != source_type {
            continue;
        }
        let target_path = import_metadata_record_context_path(contexts_dir, record)?;
        if !target_path.is_file() {
            continue;
        }
        let target_file_name = target_path.file_name().and_then(|name| name.to_str());
        let content_matches = fs::read_to_string(&target_path).map_err(|error| {
            VaultError::Io(format!(
                "failed to read existing materialized context file {}: {error}",
                target_path.display()
            ))
        })? == content;
        let file_name_matches = source_file_name.as_deref() == target_file_name;

        if content_matches || file_name_matches {
            return Ok(Some(target_path));
        }
    }

    Ok(None)
}

fn normalize_import_source_path(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

fn import_metadata_record_context_path(
    contexts_dir: &Path,
    record: &ImportMetadataRecord,
) -> Result<PathBuf, VaultError> {
    let file_name = record
        .context_relative_path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| {
            VaultError::InvalidFileName(format!(
                "import metadata context path is invalid: {}",
                record.context_relative_path.display()
            ))
        })?;
    validate_context_file_name(file_name)?;
    let folder_path = record
        .context_relative_path
        .parent()
        .map(normalize_context_folder)
        .transpose()?
        .unwrap_or_default();

    Ok(contexts_dir.join(folder_path).join(file_name))
}

fn write_materialized_context_if_changed(path: &Path, content: &str) -> Result<(), VaultError> {
    if path.is_file() {
        let current_content = fs::read_to_string(path).map_err(|error| {
            VaultError::Io(format!(
                "failed to read existing materialized context file {}: {error}",
                path.display()
            ))
        })?;
        if current_content == content {
            return Ok(());
        }
    }

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| {
            VaultError::Io(format!(
                "failed to create materialized context directory {}: {error}",
                parent.display()
            ))
        })?;
    }

    fs::write(path, content).map_err(|error| {
        VaultError::Io(format!(
            "failed to write materialized context file {}: {error}",
            path.display()
        ))
    })
}

fn materialized_folder_path(
    contexts_dir: &Path,
    target_path: &Path,
) -> Result<PathBuf, VaultError> {
    let parent = target_path.parent().unwrap_or(contexts_dir);
    parent
        .strip_prefix(contexts_dir)
        .map(Path::to_path_buf)
        .map_err(|_| {
            VaultError::Io(format!(
                "materialized context path {} is outside contexts directory {}",
                target_path.display(),
                contexts_dir.display()
            ))
        })
}

fn context_fragment_from_materialized_file(
    target_path: PathBuf,
    folder_path: PathBuf,
    content: String,
    result: ContextDiscoveryResult,
) -> ContextFragment {
    let ContextDiscoveryResult {
        file_path,
        source_type,
        metadata,
        ..
    } = result;
    let title = target_path
        .file_name()
        .and_then(|name| name.to_str())
        .map(context_title)
        .unwrap_or(metadata.title);

    ContextFragment {
        context_id: Uuid::new_v4(),
        title,
        content,
        file_path: target_path,
        vault_scope: metadata.vault_scope,
        classification: metadata.classification,
        import_classification_suggestion: metadata.import_classification_suggestion,
        inferred_classification: metadata.inferred_classification,
        tags: metadata.tags,
        folder_path,
        wikilinks: metadata.wikilinks,
        backlinks: Vec::new(),
        import_source: Some(file_path),
        import_source_type: Some(source_type),
        llm_classification_status: metadata.llm_classification_status,
    }
}

fn import_metadata_path(root: &Path) -> PathBuf {
    root.join(IMPORT_METADATA_FILE_NAME)
}

fn load_import_metadata_index(root: &Path) -> Result<ImportMetadataIndex, VaultError> {
    let path = import_metadata_path(root);
    if !path.is_file() {
        return Ok(ImportMetadataIndex::default());
    }

    let content = fs::read_to_string(&path).map_err(|error| {
        VaultError::Io(format!(
            "failed to read import metadata file {}: {error}",
            path.display()
        ))
    })?;
    serde_json::from_str(&content).map_err(|error| {
        VaultError::Io(format!(
            "failed to parse import metadata file {}: {error}",
            path.display()
        ))
    })
}

fn save_import_metadata_index(root: &Path, index: &ImportMetadataIndex) -> Result<(), VaultError> {
    let path = import_metadata_path(root);
    let content = serde_json::to_string_pretty(index).map_err(|error| {
        VaultError::Io(format!(
            "failed to serialize import metadata file {}: {error}",
            path.display()
        ))
    })?;
    fs::write(&path, format!("{content}\n")).map_err(|error| {
        VaultError::Io(format!(
            "failed to write import metadata file {}: {error}",
            path.display()
        ))
    })
}

fn persist_import_metadata(
    root: &Path,
    contexts_dir: &Path,
    context_file_path: &Path,
    import_source: &Path,
    source_type: ImportSourceType,
    metadata: &ContextDiscoveryMetadata,
) -> Result<ImportMetadataRecord, VaultError> {
    let context_relative_path = context_file_path
        .strip_prefix(contexts_dir)
        .map_err(|_| {
            VaultError::Io(format!(
                "materialized context path {} is outside contexts directory {}",
                context_file_path.display(),
                contexts_dir.display()
            ))
        })?
        .to_path_buf();
    let mut index = load_import_metadata_index(root)?;
    let normalized_import_source = normalize_import_source_path(import_source);
    let previous_record = index
        .imports
        .iter()
        .find(|existing| {
            existing.context_relative_path == context_relative_path
                || normalize_import_source_path(&existing.import_source) == normalized_import_source
        })
        .cloned();
    let previous_confirmed_status = previous_record
        .as_ref()
        .and_then(|record| record.llm_classification_status)
        .filter(|status| is_confirmed_classification_status(*status));
    let classification = previous_confirmed_status
        .and_then(|_| {
            previous_record
                .as_ref()
                .and_then(|record| record.classification)
        })
        .unwrap_or(metadata.classification);
    let llm_classification_status =
        previous_confirmed_status.unwrap_or(metadata.llm_classification_status);
    let record = ImportMetadataRecord {
        context_relative_path: context_relative_path.clone(),
        import_source: import_source.to_path_buf(),
        source_type,
        classification: Some(classification),
        import_classification_suggestion: metadata.import_classification_suggestion,
        inferred_classification: metadata.inferred_classification,
        tags: metadata.tags.clone(),
        llm_classification_status: Some(llm_classification_status),
    };

    index.imports.retain(|existing| {
        existing.context_relative_path == context_relative_path
            || normalize_import_source_path(&existing.import_source) != normalized_import_source
    });

    match index.imports.iter_mut().find(|existing| {
        existing.context_relative_path == context_relative_path
            || normalize_import_source_path(&existing.import_source) == normalized_import_source
    }) {
        Some(existing) => *existing = record.clone(),
        None => index.imports.push(record.clone()),
    }
    index.imports.sort_by(|left, right| {
        left.context_relative_path
            .cmp(&right.context_relative_path)
            .then_with(|| left.import_source.cmp(&right.import_source))
    });

    save_import_metadata_index(root, &index)?;
    Ok(record)
}

fn is_confirmed_classification_status(status: ClassificationStatus) -> bool {
    matches!(
        status,
        ClassificationStatus::Reviewed | ClassificationStatus::Modified
    )
}

fn discovery_result_with_persisted_import_metadata(
    mut result: ContextDiscoveryResult,
    record: &ImportMetadataRecord,
) -> ContextDiscoveryResult {
    if let Some(classification) = record.classification {
        result.metadata.classification = classification;
    }
    result.metadata.import_classification_suggestion = record.import_classification_suggestion;
    result.metadata.inferred_classification = record.inferred_classification;
    result.metadata.tags = record.tags.clone();
    if let Some(status) = record.llm_classification_status {
        result.metadata.llm_classification_status = status;
    }
    result.source_type = record.source_type;
    result
}

pub fn review_import_classification(
    working_dir: &Path,
    path: &Path,
    classification: Classification,
) -> Result<ContextFragment, VaultError> {
    let context = resolve_context_path_from_overlay(working_dir, path)?;
    let roots = VaultRoots::discover(working_dir);
    let root = match context.vault_scope {
        VaultScope::Global => roots.global_root,
        VaultScope::Local => roots.local_root.ok_or(VaultError::MissingLocalVault)?,
    };
    let contexts_dir = managed_contexts_dir(&root);
    let relative_path = context
        .file_path
        .strip_prefix(&contexts_dir)
        .map_err(|_| {
            VaultError::Io(format!(
                "context file is not managed by the {:?} vault: {}",
                context.vault_scope,
                context.file_path.display()
            ))
        })?
        .to_path_buf();
    let mut index = load_import_metadata_index(&root)?;
    let record = index
        .imports
        .iter_mut()
        .find(|record| record.context_relative_path == relative_path)
        .ok_or_else(|| {
            VaultError::Io(format!(
                "context file has no import metadata to review: {}",
                context.file_path.display()
            ))
        })?;
    let suggested_classification = record
        .import_classification_suggestion
        .or(record.inferred_classification)
        .or(record.classification);
    let status = if suggested_classification == Some(classification) {
        ClassificationStatus::Reviewed
    } else {
        ClassificationStatus::Modified
    };

    record.classification = Some(classification);
    record.llm_classification_status = Some(status);
    save_import_metadata_index(&root, &index)?;

    let refreshed = resolve_context_path_from_overlay(working_dir, path)?;
    sync_reviewed_classification_index(working_dir, &refreshed)?;
    Ok(refreshed)
}

fn sync_reviewed_classification_index(
    working_dir: &Path,
    context: &ContextFragment,
) -> Result<(), VaultError> {
    let (vault_root, database_path) = initialize_index_for_scope(working_dir, context.vault_scope)?;
    let now = unix_timestamp_now();
    let title_targets =
        title_target_paths_for_scope(&vault_root, context.vault_scope, Some(context))?;
    let record = markdown_index_record_from_context(context, &vault_root, &title_targets, now)?;

    upsert_markdown_file_index_record(&database_path, &record).map_err(|error| {
        VaultError::Index(format!(
            "failed to persist reviewed classification for {} to sqlite index: {error}",
            context.file_path.display()
        ))
    })?;

    Ok(())
}

fn import_metadata_record_for<'a>(
    index: &'a ImportMetadataIndex,
    contexts_dir: &Path,
    context_file_path: &Path,
) -> Option<&'a ImportMetadataRecord> {
    let context_relative_path = context_file_path.strip_prefix(contexts_dir).ok()?;
    index
        .imports
        .iter()
        .find(|record| record.context_relative_path == context_relative_path)
}

fn update_managed_import_metadata_for_move(
    vault_root: &Path,
    event: &ContextFileChangeEvent,
) -> Result<(), VaultError> {
    if event.root_kind != ContextWatchRootKind::ManagedVault {
        return Ok(());
    }

    let Some(previous_relative_path) = &event.previous_relative_path else {
        return Ok(());
    };

    let mut index = load_import_metadata_index(vault_root)?;
    let mut changed = false;
    for record in &mut index.imports {
        if record.context_relative_path == *previous_relative_path {
            record.context_relative_path = event.relative_path.clone();
            changed = true;
        }
    }

    if changed {
        index.imports.sort_by(|left, right| {
            left.context_relative_path
                .cmp(&right.context_relative_path)
                .then_with(|| left.import_source.cmp(&right.import_source))
        });
        save_import_metadata_index(vault_root, &index)?;
    }

    Ok(())
}

fn cleanup_managed_import_metadata_for_delete(
    vault_root: &Path,
    event: &ContextFileChangeEvent,
) -> Result<(), VaultError> {
    if event.root_kind != ContextWatchRootKind::ManagedVault {
        return Ok(());
    }

    let mut index = load_import_metadata_index(vault_root)?;
    let original_len = index.imports.len();
    index
        .imports
        .retain(|record| record.context_relative_path != event.relative_path);

    if index.imports.len() != original_len {
        save_import_metadata_index(vault_root, &index)?;
    }

    Ok(())
}

fn detect_import_source_type(
    path: &Path,
    classification: Classification,
    classification_override: Option<Classification>,
) -> ImportSourceType {
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default();

    if file_name.eq_ignore_ascii_case("SKILL.md") {
        return ImportSourceType::SkillManifest;
    }
    if classification_override == Some(Classification::Shared) || path_has_skill_segment(path) {
        return ImportSourceType::SkillMarkdown;
    }
    if file_name.eq_ignore_ascii_case("CLAUDE.md") || file_name.eq_ignore_ascii_case("claude.md") {
        return ImportSourceType::ClaudeMarkdown;
    }
    if file_name.eq_ignore_ascii_case("AGENTS.md") {
        return ImportSourceType::CodexAgents;
    }
    if file_name.eq_ignore_ascii_case("agent.md") {
        return ImportSourceType::AgentMarkdown;
    }
    if file_name.eq_ignore_ascii_case("agents.md") {
        return ImportSourceType::AgentsManifest;
    }
    if classification == Classification::Subagent {
        return ImportSourceType::SubagentMarkdown;
    }

    ImportSourceType::ContextMarkdown
}

fn path_has_skill_segment(path: &Path) -> bool {
    path.components().any(|component| {
        component
            .as_os_str()
            .to_str()
            .map(|segment| segment.eq_ignore_ascii_case("skills"))
            .unwrap_or(false)
    })
}

fn path_has_skill_segment_under_root(path: &Path, root: &Path) -> bool {
    path.strip_prefix(root)
        .ok()
        .map(path_has_skill_segment)
        .unwrap_or(false)
}

fn is_markdown_file(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| extension.eq_ignore_ascii_case("md"))
        .unwrap_or(false)
}

fn should_exclude_discovery_directory(path: &Path) -> bool {
    if is_ctx_vault_directory(path) {
        return true;
    }

    path.file_name()
        .and_then(|name| name.to_str())
        .map(|file_name| {
            DISCOVERY_EXCLUDED_DIRECTORY_NAMES
                .iter()
                .any(|excluded| file_name.eq_ignore_ascii_case(excluded))
        })
        .unwrap_or(false)
}

fn is_ctx_vault_directory(path: &Path) -> bool {
    let is_vault_dir = path
        .file_name()
        .and_then(|name| name.to_str())
        .map(|file_name| file_name.eq_ignore_ascii_case(GLOBAL_VAULT_DIR))
        .unwrap_or(false);
    let parent_is_ctx_dir = path
        .parent()
        .and_then(|parent| parent.file_name())
        .and_then(|name| name.to_str())
        .map(|file_name| file_name.eq_ignore_ascii_case(CTX_HOME_DIR))
        .unwrap_or(false);

    is_vault_dir && parent_is_ctx_dir
}

fn is_main_agent_file_name(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .map(|file_name| {
            MAIN_AGENT_FILE_NAMES
                .iter()
                .any(|candidate| file_name.eq_ignore_ascii_case(candidate))
        })
        .unwrap_or(false)
}

fn is_skill_file_name(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .map(|file_name| file_name.eq_ignore_ascii_case("SKILL.md"))
        .unwrap_or(false)
}

fn hydrate_backlinks(contexts: &mut [ContextFragment]) {
    let context_refs = contexts.iter().collect::<Vec<_>>();
    let title_targets = title_target_candidates(&context_refs);
    let mut backlinks_by_path: BTreeMap<PathBuf, BTreeSet<String>> = BTreeMap::new();

    for source in contexts.iter() {
        for link in &source.wikilinks {
            let normalized_target = normalize_link_target(link);
            let Some(target_path) = unique_title_target_path(&title_targets, &normalized_target)
            else {
                continue;
            };
            if target_path == source.file_path {
                continue;
            }

            backlinks_by_path
                .entry(target_path)
                .or_default()
                .insert(source.title.clone());
        }
    }

    for context in contexts {
        context.backlinks = backlinks_by_path
            .remove(&context.file_path)
            .unwrap_or_default()
            .into_iter()
            .collect();
    }
}

fn validate_context_file_name(file_name: &str) -> Result<(), VaultError> {
    let trimmed = file_name.trim();
    if trimmed.is_empty() {
        return Err(VaultError::EmptyFileName);
    }

    if trimmed != file_name {
        return Err(VaultError::InvalidFileName(
            "context filename cannot contain leading or trailing whitespace".to_string(),
        ));
    }

    if file_name.contains('/') || file_name.contains('\\') || file_name.contains('\0') {
        return Err(VaultError::InvalidFileName(
            "context filename must be a single file name without path separators".to_string(),
        ));
    }

    let path = Path::new(file_name);
    if path.components().count() != 1
        || path.file_name().and_then(|name| name.to_str()) != Some(file_name)
    {
        return Err(VaultError::InvalidFileName(
            "context filename must be a single file name without path separators".to_string(),
        ));
    }

    if matches!(file_name, "." | "..") || file_name.starts_with('.') {
        return Err(VaultError::InvalidFileName(
            "context filename cannot be hidden, current directory, or parent directory".to_string(),
        ));
    }

    if path.extension().and_then(|extension| extension.to_str()) != Some("md") {
        return Err(VaultError::InvalidExtension(file_name.to_string()));
    }

    Ok(())
}

fn normalize_context_folder(folder_path: &Path) -> Result<PathBuf, VaultError> {
    let mut normalized = PathBuf::new();

    for component in folder_path.components() {
        match component {
            Component::CurDir => {}
            Component::Normal(segment) => normalized.push(segment),
            Component::Prefix(_) | Component::RootDir | Component::ParentDir => {
                return Err(VaultError::InvalidFolderPath(
                    "context folder path must be relative and stay inside the managed contexts directory"
                        .to_string(),
                ));
            }
        }
    }

    Ok(normalized)
}

fn canonical_path_parts(path: &Path) -> Vec<String> {
    path.components()
        .filter_map(|component| match component {
            Component::CurDir => None,
            Component::Normal(segment) => segment.to_str().map(canonical_path_segment),
            Component::Prefix(_) | Component::RootDir | Component::ParentDir => None,
        })
        .collect()
}

fn canonical_path_segment(segment: &str) -> String {
    segment.trim().to_lowercase()
}

fn context_title(file_name: &str) -> String {
    Path::new(file_name)
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or(file_name)
        .replace('-', " ")
        .replace('_', " ")
}

fn extract_wikilinks(content: &str) -> Vec<String> {
    let mut links = Vec::new();
    let mut remaining = content;

    while let Some(start) = remaining.find("[[") {
        let after_start = &remaining[start + 2..];
        let Some(end) = after_start.find("]]") else {
            break;
        };

        let link = after_start[..end].trim();
        if !link.is_empty() && !links.iter().any(|existing| existing == link) {
            links.push(link.to_string());
        }
        remaining = &after_start[end + 2..];
    }

    links
}

fn markdown_index_records_for_scope(
    contexts: &[ContextFragment],
    scope: VaultScope,
    vault_root: &Path,
    indexed_at_unix_seconds: i64,
) -> Result<Vec<MarkdownFileIndexRecord>, VaultError> {
    let scoped_contexts = contexts
        .iter()
        .filter(|context| context.vault_scope == scope)
        .collect::<Vec<_>>();
    let title_targets = title_target_paths(&scoped_contexts);
    let mut records = Vec::new();

    for context in scoped_contexts {
        records.push(markdown_index_record_from_context(
            context,
            vault_root,
            &title_targets,
            indexed_at_unix_seconds,
        )?);
    }

    records.sort_by(|left, right| {
        left.relative_path
            .cmp(&right.relative_path)
            .then_with(|| left.path.cmp(&right.path))
    });
    Ok(records)
}

fn markdown_index_record_from_context(
    context: &ContextFragment,
    vault_root: &Path,
    title_targets: &BTreeMap<String, PathBuf>,
    indexed_at_unix_seconds: i64,
) -> Result<MarkdownFileIndexRecord, VaultError> {
    let metadata = fs::metadata(&context.file_path).map_err(|error| {
        VaultError::Io(format!(
            "failed to read markdown file metadata {}: {error}",
            context.file_path.display()
        ))
    })?;
    let file_name = context
        .file_path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("context.md")
        .to_string();
    let relative_path = indexed_relative_path(context, vault_root, &file_name);
    let frontmatter = parse_frontmatter_for_index(
        &context.file_path,
        &context.content,
        indexed_at_unix_seconds,
    );
    let inline_metadata =
        parse_inline_metadata_fields(markdown_body_without_frontmatter(&context.content));
    let tags = markdown_tag_records_for_context(
        context,
        &frontmatter,
        &inline_metadata,
        indexed_at_unix_seconds,
    );
    let links = markdown_link_records_for_context(context, title_targets, indexed_at_unix_seconds);

    Ok(MarkdownFileIndexRecord {
        path: context.file_path.clone(),
        context_id: context.context_id.to_string(),
        title: frontmatter
            .frontmatter_title
            .clone()
            .or(inline_metadata.title.clone())
            .unwrap_or_else(|| context.title.clone()),
        vault_scope: context.vault_scope,
        relative_path,
        folder_path: context.folder_path.clone(),
        file_name,
        classification: frontmatter
            .frontmatter_classification
            .or(inline_metadata.classification)
            .unwrap_or(context.classification),
        import_classification_suggestion: context.import_classification_suggestion,
        inferred_classification: context.inferred_classification,
        llm_classification_status: context.llm_classification_status,
        file_created_at_unix_seconds: system_time_to_unix_seconds(
            metadata.created().unwrap_or_else(|_| SystemTime::now()),
        ),
        file_modified_at_unix_seconds: system_time_to_unix_seconds(
            metadata.modified().unwrap_or_else(|_| SystemTime::now()),
        ),
        indexed_at_unix_seconds: Some(indexed_at_unix_seconds),
        content_hash: content_hash(&context.content),
        content: context.content.clone(),
        indexing_status: MarkdownFileIndexingStatus::Indexed,
        last_index_error: None,
        import_source: context.import_source.clone(),
        import_source_type: context.import_source_type,
        frontmatter: Some(frontmatter),
        tags,
        links,
    })
}

fn indexed_relative_path(context: &ContextFragment, vault_root: &Path, file_name: &str) -> PathBuf {
    let managed_dir = managed_contexts_dir(vault_root);
    if let Ok(relative_path) = context.file_path.strip_prefix(&managed_dir) {
        return relative_path.to_path_buf();
    }

    let mut relative_path = context.folder_path.clone();
    relative_path.push(file_name);
    relative_path
}

fn parse_frontmatter_for_index(
    path: &Path,
    content: &str,
    indexed_at_unix_seconds: i64,
) -> ParsedFrontmatterMetadata {
    let Some((format, raw, body_start)) = split_frontmatter(content) else {
        return ParsedFrontmatterMetadata {
            path: path.to_path_buf(),
            frontmatter_format: FrontmatterFormat::None,
            frontmatter_raw: None,
            frontmatter_json: "{}".to_string(),
            frontmatter_title: None,
            frontmatter_tags: Vec::new(),
            frontmatter_classification: None,
            parse_status: FrontmatterParseStatus::Absent,
            parse_error: None,
            parsed_at_unix_seconds: Some(indexed_at_unix_seconds),
        };
    };
    let parsed = parse_simple_frontmatter_fields(format, raw);
    let frontmatter_json = serde_json::json!({
        "title": parsed.title.clone(),
        "tags": parsed.tags.clone(),
        "classification": parsed.classification.map(classification_label),
    })
    .to_string();

    ParsedFrontmatterMetadata {
        path: path.to_path_buf(),
        frontmatter_format: format,
        frontmatter_raw: Some(content[..body_start].to_string()),
        frontmatter_json,
        frontmatter_title: parsed.title,
        frontmatter_tags: parsed.tags,
        frontmatter_classification: parsed.classification,
        parse_status: FrontmatterParseStatus::Parsed,
        parse_error: None,
        parsed_at_unix_seconds: Some(indexed_at_unix_seconds),
    }
}

#[derive(Debug, Default)]
struct SimpleFrontmatterFields {
    title: Option<String>,
    tags: Vec<String>,
    classification: Option<Classification>,
}

#[derive(Debug, Default)]
struct InlineMetadataFields {
    title: Option<String>,
    tags: Vec<String>,
    classification: Option<Classification>,
}

fn split_frontmatter(content: &str) -> Option<(FrontmatterFormat, &str, usize)> {
    if let Some(rest) = content.strip_prefix("---\n") {
        let end = rest.find("\n---\n")?;
        let body_start = "---\n".len() + end + "\n---\n".len();
        return Some((FrontmatterFormat::Yaml, &rest[..end], body_start));
    }
    if let Some(rest) = content.strip_prefix("+++\n") {
        let end = rest.find("\n+++\n")?;
        let body_start = "+++\n".len() + end + "\n+++\n".len();
        return Some((FrontmatterFormat::Toml, &rest[..end], body_start));
    }
    None
}

fn parse_simple_frontmatter_fields(
    format: FrontmatterFormat,
    raw: &str,
) -> SimpleFrontmatterFields {
    let mut fields = SimpleFrontmatterFields::default();
    let mut active_list_key: Option<String> = None;

    for line in raw.lines() {
        let trimmed_line = line.trim();
        if trimmed_line.is_empty() || trimmed_line.starts_with('#') {
            continue;
        }

        if let Some(key) = active_list_key.as_deref() {
            if let Some(item) = trimmed_line.strip_prefix("- ") {
                if key.eq_ignore_ascii_case("tags") {
                    fields.tags.extend(parse_tag_list(item));
                }
                continue;
            }
            active_list_key = None;
        }

        let Some((key, value)) = line.split_once(':').or_else(|| line.split_once('=')) else {
            continue;
        };
        let key = key.trim();
        let value = value
            .trim()
            .trim_end_matches(',')
            .trim_matches('"')
            .trim_matches('\'');
        if key.eq_ignore_ascii_case("title") {
            if !value.is_empty() {
                fields.title = Some(value.to_string());
            }
        } else if key.eq_ignore_ascii_case("tags") {
            if value.is_empty() && format == FrontmatterFormat::Yaml {
                active_list_key = Some(key.to_string());
            } else {
                fields.tags.extend(parse_tag_list(value));
            }
        } else if key.eq_ignore_ascii_case("classification") {
            fields.classification = parse_classification(value);
        }
    }

    fields.tags = normalized_discovery_tags(fields.tags);
    fields
}

fn parse_tag_list(value: &str) -> Vec<String> {
    let trimmed = value.trim();
    let trimmed = trimmed
        .strip_prefix('[')
        .and_then(|value| value.strip_suffix(']'))
        .unwrap_or(trimmed);
    if is_quoted_scalar(trimmed) {
        let tag = normalize_metadata_token(trimmed);
        return (!tag.is_empty()).then_some(tag).into_iter().collect();
    }
    let separators: &[_] = if trimmed.contains(',') {
        &[',']
    } else {
        &[' ', '\t']
    };
    trimmed
        .split(separators)
        .map(normalize_metadata_token)
        .filter(|tag| !tag.is_empty())
        .collect()
}

fn is_quoted_scalar(value: &str) -> bool {
    (value.starts_with('"') && value.ends_with('"'))
        || (value.starts_with('\'') && value.ends_with('\''))
}

fn normalize_metadata_token(value: &str) -> String {
    value
        .trim()
        .trim_matches('"')
        .trim_matches('\'')
        .trim_start_matches('#')
        .trim_end_matches(',')
        .trim()
        .to_string()
}

fn normalize_tag_token(value: &str) -> String {
    value
        .trim()
        .trim_matches('"')
        .trim_matches('\'')
        .trim_start_matches('#')
        .trim_matches(|character: char| {
            character == ','
                || character == '.'
                || character == ';'
                || character == ':'
                || character == ')'
                || character == ']'
                || character == '}'
                || character == '('
                || character == '['
                || character == '{'
        })
        .trim()
        .to_ascii_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join("-")
}

fn parse_classification(value: &str) -> Option<Classification> {
    match value.trim().to_ascii_lowercase().as_str() {
        "main-agent" | "main_agent" | "main" => Some(Classification::MainAgent),
        "subagent" | "sub-agent" | "sub_agent" => Some(Classification::Subagent),
        "shared" => Some(Classification::Shared),
        _ => None,
    }
}

fn classification_label(classification: Classification) -> &'static str {
    match classification {
        Classification::MainAgent => "main-agent",
        Classification::Subagent => "subagent",
        Classification::Shared => "shared",
    }
}

fn markdown_body_without_frontmatter(content: &str) -> &str {
    split_frontmatter(content)
        .map(|(_, _, body_start)| &content[body_start..])
        .unwrap_or(content)
}

fn parse_inline_metadata_fields(body: &str) -> InlineMetadataFields {
    let mut fields = InlineMetadataFields::default();

    for line in body.lines() {
        if let Some((key, value)) = line.split_once("::") {
            let key = key.trim();
            let value = value.trim();
            if key.eq_ignore_ascii_case("title") && fields.title.is_none() {
                let title = normalize_inline_scalar(value);
                if !title.is_empty() {
                    fields.title = Some(title);
                }
            } else if key.eq_ignore_ascii_case("tags") || key.eq_ignore_ascii_case("tag") {
                fields.tags.extend(parse_tag_list(value));
            } else if key.eq_ignore_ascii_case("classification")
                || key.eq_ignore_ascii_case("class")
            {
                fields.classification = parse_classification(value);
            }
        }

        fields.tags.extend(extract_inline_hash_tags(line));
    }

    fields.tags = normalized_discovery_tags(fields.tags);
    fields
}

fn normalize_inline_scalar(value: &str) -> String {
    value
        .trim()
        .trim_matches('"')
        .trim_matches('\'')
        .trim()
        .to_string()
}

fn extract_inline_hash_tags(line: &str) -> Vec<String> {
    let trimmed = line.trim_start();
    if trimmed.starts_with('#') && trimmed.chars().nth(1).is_some_and(char::is_whitespace) {
        return Vec::new();
    }

    let mut tags = Vec::new();
    for token in line.split_whitespace() {
        let Some(hash_index) = token.find('#') else {
            continue;
        };
        if hash_index > 0 {
            let previous = token.as_bytes()[hash_index - 1] as char;
            if previous.is_ascii_alphanumeric() || previous == '/' || previous == ':' {
                continue;
            }
        }
        let candidate = token[hash_index + 1..]
            .trim_matches(|character: char| {
                character == ','
                    || character == '.'
                    || character == ';'
                    || character == ':'
                    || character == ')'
                    || character == ']'
                    || character == '}'
                    || character == '"'
                    || character == '\''
            })
            .trim();
        if candidate.chars().all(|character| {
            character.is_ascii_alphanumeric()
                || character == '-'
                || character == '_'
                || character == '/'
        }) && candidate
            .chars()
            .any(|character| character.is_ascii_alphanumeric())
        {
            tags.push(candidate.to_string());
        }
    }
    tags
}

fn markdown_tag_records_for_context(
    context: &ContextFragment,
    frontmatter: &ParsedFrontmatterMetadata,
    inline_metadata: &InlineMetadataFields,
    indexed_at_unix_seconds: i64,
) -> Vec<MarkdownFileTagRecord> {
    let mut records = Vec::new();
    let mut seen = HashSet::new();

    for tag in &frontmatter.frontmatter_tags {
        push_markdown_tag_record(
            &mut records,
            &mut seen,
            &context.file_path,
            tag,
            MarkdownFileTagSource::Frontmatter,
            indexed_at_unix_seconds,
        );
    }
    for tag in &context.tags {
        push_markdown_tag_record(
            &mut records,
            &mut seen,
            &context.file_path,
            tag,
            MarkdownFileTagSource::Import,
            indexed_at_unix_seconds,
        );
    }
    for tag in &inline_metadata.tags {
        push_markdown_tag_record(
            &mut records,
            &mut seen,
            &context.file_path,
            tag,
            MarkdownFileTagSource::Body,
            indexed_at_unix_seconds,
        );
    }

    records
}

fn push_markdown_tag_record(
    records: &mut Vec<MarkdownFileTagRecord>,
    seen: &mut HashSet<String>,
    path: &Path,
    tag: &str,
    tag_source: MarkdownFileTagSource,
    indexed_at_unix_seconds: i64,
) {
    let normalized = normalize_tag_token(tag);
    if normalized.is_empty() || !seen.insert(normalized.clone()) {
        return;
    }
    records.push(MarkdownFileTagRecord {
        path: path.to_path_buf(),
        tag_id: normalized,
        tag_source,
        tag_position: records.len() as i64,
        indexed_at_unix_seconds: Some(indexed_at_unix_seconds),
    });
}

fn markdown_link_records_for_context(
    context: &ContextFragment,
    title_targets: &BTreeMap<String, PathBuf>,
    indexed_at_unix_seconds: i64,
) -> Vec<MarkdownFileLinkRecord> {
    extract_markdown_link_occurrences(markdown_body_without_frontmatter(&context.content))
        .into_iter()
        .enumerate()
        .map(|(index, occurrence)| {
            let normalized_target = normalize_link_target(&occurrence.raw_target);
            let target_path = title_targets.get(&normalized_target).cloned();
            let target_url = external_link_url(&occurrence.raw_target);
            let resolved_status = if target_url.is_some() {
                MarkdownFileLinkResolvedStatus::External
            } else if target_path.is_some() {
                MarkdownFileLinkResolvedStatus::Resolved
            } else {
                MarkdownFileLinkResolvedStatus::Unresolved
            };

            MarkdownFileLinkRecord {
                link_id: deterministic_id(&format!(
                    "{}:{index}:{}:{}:{}",
                    context.file_path.display(),
                    occurrence.kind.as_sql_value(),
                    occurrence.byte_start,
                    occurrence.raw_target
                )),
                source_path: context.file_path.clone(),
                link_kind: occurrence.kind,
                raw_target: occurrence.raw_target,
                normalized_target,
                link_text: occurrence.link_text,
                target_path,
                target_anchor: occurrence.target_anchor,
                target_url,
                resolved_status,
                byte_start: Some(occurrence.byte_start as i64),
                byte_end: Some(occurrence.byte_end as i64),
                line_number: Some(occurrence.line_number as i64),
                link_position: index as i64,
                indexed_at_unix_seconds: Some(indexed_at_unix_seconds),
            }
        })
        .collect()
}

fn title_target_paths(contexts: &[&ContextFragment]) -> BTreeMap<String, PathBuf> {
    title_target_candidates(contexts)
        .into_iter()
        .filter_map(|(target, paths)| {
            let mut paths = paths.into_iter();
            let path = paths.next()?;
            paths.next().is_none().then_some((target, path))
        })
        .collect()
}

fn title_target_candidates(contexts: &[&ContextFragment]) -> BTreeMap<String, BTreeSet<PathBuf>> {
    let mut targets = BTreeMap::new();
    for context in contexts {
        insert_title_target(&mut targets, &context.title, &context.file_path);
        if let Some(stem) = context.file_path.file_stem().and_then(|stem| stem.to_str()) {
            insert_title_target(&mut targets, stem, &context.file_path);
        }
        if let Some(file_name) = context.file_path.file_name().and_then(|name| name.to_str()) {
            let relative_path = context.folder_path.join(file_name);
            insert_title_target(
                &mut targets,
                &relative_path.to_string_lossy(),
                &context.file_path,
            );
            if let Some(stem) = Path::new(file_name)
                .file_stem()
                .and_then(|stem| stem.to_str())
            {
                let relative_stem_path = context.folder_path.join(stem);
                insert_title_target(
                    &mut targets,
                    &relative_stem_path.to_string_lossy(),
                    &context.file_path,
                );
            }
        }
    }
    targets
}

fn insert_title_target(
    targets: &mut BTreeMap<String, BTreeSet<PathBuf>>,
    target: &str,
    path: &Path,
) {
    let normalized = normalize_link_target(target);
    if normalized.is_empty() {
        return;
    }
    targets
        .entry(normalized)
        .or_default()
        .insert(path.to_path_buf());
}

fn unique_title_target_path(
    title_targets: &BTreeMap<String, BTreeSet<PathBuf>>,
    normalized_target: &str,
) -> Option<PathBuf> {
    let mut paths = title_targets.get(normalized_target)?.iter();
    let path = paths.next()?.clone();
    paths.next().is_none().then_some(path)
}

fn normalize_link_target(target: &str) -> String {
    let target_without_alias = target.split('|').next().unwrap_or(target).trim();
    let normalized_base = target_without_alias
        .split('#')
        .next()
        .unwrap_or(target_without_alias)
        .trim()
        .trim_matches('<')
        .trim_matches('>')
        .replace('\\', "/")
        .to_ascii_lowercase();
    let normalized = if normalized_base.is_empty() {
        target_without_alias
            .trim()
            .trim_matches('<')
            .trim_matches('>')
            .replace('\\', "/")
            .to_ascii_lowercase()
    } else {
        normalized_base
    };
    normalized
        .strip_suffix(".md")
        .unwrap_or(&normalized)
        .to_string()
}

#[derive(Debug, Clone, Eq, PartialEq)]
struct MarkdownLinkOccurrence {
    kind: MarkdownFileLinkKind,
    raw_target: String,
    link_text: Option<String>,
    target_anchor: Option<String>,
    byte_start: usize,
    byte_end: usize,
    line_number: usize,
}

fn extract_markdown_link_occurrences(body: &str) -> Vec<MarkdownLinkOccurrence> {
    let mut links = Vec::new();
    let bytes = body.as_bytes();
    let mut index = 0;
    let mut line_number = 1;

    while index < bytes.len() {
        if !body.is_char_boundary(index) {
            index += 1;
            continue;
        }
        match bytes[index] {
            b'\n' => {
                line_number += 1;
                index += 1;
            }
            b'[' if body[index..].starts_with("[[") => {
                let start_line = line_number;
                if let Some((occurrence, next_index)) =
                    parse_wikilink_occurrence(body, index, start_line)
                {
                    links.push(occurrence);
                    line_number += body[index..next_index]
                        .bytes()
                        .filter(|byte| *byte == b'\n')
                        .count();
                    index = next_index;
                } else {
                    index += 1;
                }
            }
            b'[' => {
                let start_line = line_number;
                if let Some((occurrence, next_index)) =
                    parse_markdown_link_occurrence(body, index, start_line)
                {
                    links.push(occurrence);
                    line_number += body[index..next_index]
                        .bytes()
                        .filter(|byte| *byte == b'\n')
                        .count();
                    index = next_index;
                } else {
                    index += 1;
                }
            }
            _ => {
                index += 1;
            }
        }
    }

    links
}

fn parse_wikilink_occurrence(
    body: &str,
    byte_start: usize,
    line_number: usize,
) -> Option<(MarkdownLinkOccurrence, usize)> {
    let after_start = byte_start + 2;
    let end_offset = body[after_start..].find("]]")?;
    let byte_end = after_start + end_offset + 2;
    let inner = body[after_start..after_start + end_offset].trim();
    let (raw_target, link_text) = split_wikilink_target_and_alias(inner)?;
    let target_anchor = link_anchor(&raw_target);

    Some((
        MarkdownLinkOccurrence {
            kind: MarkdownFileLinkKind::Wikilink,
            raw_target,
            link_text,
            target_anchor,
            byte_start,
            byte_end,
            line_number,
        },
        byte_end,
    ))
}

fn parse_markdown_link_occurrence(
    body: &str,
    byte_start: usize,
    line_number: usize,
) -> Option<(MarkdownLinkOccurrence, usize)> {
    if byte_start > 0 && body.as_bytes()[byte_start - 1] == b'!' {
        return None;
    }

    let text_end = find_closing_markdown_bracket(body, byte_start)?;
    let open_paren = text_end + 1;
    if body.as_bytes().get(open_paren) != Some(&b'(') {
        return None;
    }
    let target_end = find_closing_markdown_paren(body, open_paren)?;
    let link_text = body[byte_start + 1..text_end].trim();
    let raw_target = normalize_markdown_link_destination(&body[open_paren + 1..target_end])?;
    let target_anchor = link_anchor(&raw_target);

    Some((
        MarkdownLinkOccurrence {
            kind: MarkdownFileLinkKind::Markdown,
            raw_target,
            link_text: (!link_text.is_empty()).then(|| link_text.to_string()),
            target_anchor,
            byte_start,
            byte_end: target_end + 1,
            line_number,
        },
        target_end + 1,
    ))
}

fn split_wikilink_target_and_alias(inner: &str) -> Option<(String, Option<String>)> {
    let (target, alias) = inner
        .split_once('|')
        .map(|(target, alias)| (target.trim(), Some(alias.trim())))
        .unwrap_or((inner.trim(), None));
    if target.is_empty() {
        return None;
    }
    Some((
        target.to_string(),
        alias
            .filter(|value| !value.is_empty())
            .map(ToString::to_string),
    ))
}

fn normalize_markdown_link_destination(raw_destination: &str) -> Option<String> {
    let mut destination = raw_destination.trim();
    if destination.is_empty() {
        return None;
    }
    if let Some(title_start) = destination.find(char::is_whitespace) {
        destination = destination[..title_start].trim();
    }
    destination = destination
        .trim_matches('<')
        .trim_matches('>')
        .trim_matches('"')
        .trim_matches('\'');
    (!destination.is_empty()).then(|| destination.to_string())
}

fn find_closing_markdown_bracket(body: &str, open_bracket: usize) -> Option<usize> {
    let bytes = body.as_bytes();
    let mut index = open_bracket + 1;
    while index < bytes.len() {
        match bytes[index] {
            b'\\' => index += 2,
            b']' => return Some(index),
            b'\n' => return None,
            _ => index += 1,
        }
    }
    None
}

fn find_closing_markdown_paren(body: &str, open_paren: usize) -> Option<usize> {
    let bytes = body.as_bytes();
    let mut index = open_paren + 1;
    let mut nested = 0usize;
    while index < bytes.len() {
        match bytes[index] {
            b'\\' => index += 2,
            b'(' => {
                nested += 1;
                index += 1;
            }
            b')' if nested == 0 => return Some(index),
            b')' => {
                nested -= 1;
                index += 1;
            }
            b'\n' => return None,
            _ => index += 1,
        }
    }
    None
}

fn link_anchor(target: &str) -> Option<String> {
    target
        .split_once('#')
        .map(|(_, anchor)| anchor.trim())
        .filter(|anchor| !anchor.is_empty())
        .map(ToString::to_string)
}

fn external_link_url(target: &str) -> Option<String> {
    let normalized = target.trim().to_ascii_lowercase();
    (normalized.starts_with("http://")
        || normalized.starts_with("https://")
        || normalized.starts_with("mailto:")
        || normalized.starts_with("file://"))
    .then(|| target.trim().to_string())
}

fn unix_timestamp_now() -> i64 {
    system_time_to_unix_seconds(SystemTime::now())
}

fn system_time_to_unix_seconds(time: SystemTime) -> i64 {
    time.duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0)
}

fn content_hash(content: &str) -> String {
    deterministic_id(content)
}

fn deterministic_id(value: &str) -> String {
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in value.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

#[cfg(test)]
mod tests {
    use super::{
        canonical_vault_entry_key, create_context_file, delete_markdown_context_file,
        delete_resolved_context_markdown, discover_existing_context_file_results,
        discover_existing_context_files, discover_global_vault_path,
        discover_project_local_vault_path, global_vault_path_from_home, import_metadata_path,
        initialize_global_vault_path_from_home, initialize_project_local_vault, list_context_files,
        list_context_files_with_discovered, load_import_metadata_index, managed_contexts_dir,
        materialize_discovered_context_files, normalize_import_source_path,
        read_markdown_context_file, read_resolved_context_markdown, reindex_markdown_contexts,
        resolve_overlay, resolve_overlay_vault, review_import_classification,
        sync_markdown_context_index_event, sync_markdown_context_index_events,
        update_markdown_context_file, VaultError, VaultRoots, CTX_HOME_DIR, GLOBAL_VAULT_DIR,
        MANAGED_CONTEXTS_DIR,
    };
    use crate::sqlite_index::{
        search_markdown_file_index_from_connection, sqlite_index_path, MARKDOWN_FILES_TABLE_NAME,
        MARKDOWN_FILE_BACKLINKS_VIEW_NAME, MARKDOWN_FILE_FRONTMATTER_TABLE_NAME,
        MARKDOWN_FILE_LINKS_TABLE_NAME, MARKDOWN_FILE_SEARCH_TABLE_NAME,
        MARKDOWN_FILE_TAGS_TABLE_NAME,
    };
    use crate::{
        vault_settings_path, Classification, ClassificationStatus, ContextFileChangeEvent,
        ContextFileChangeKind, ContextFragment, ContextWatchRootKind, ImportSourceType, VaultScope,
    };
    use rusqlite::{params, Connection};
    use std::{
        collections::HashSet,
        ffi::OsString,
        fs,
        path::{Path, PathBuf},
        sync::Mutex,
    };
    use uuid::Uuid;

    static HOME_ENV_LOCK: Mutex<()> = Mutex::new(());

    fn test_roots() -> (VaultRoots, PathBuf) {
        let base = std::env::temp_dir().join(format!("ctx-vault-test-{}", Uuid::new_v4()));
        let roots = VaultRoots {
            global_root: base.join("global"),
            local_root: Some(base.join("project").join(".ctx").join("vault")),
        };

        (roots, base)
    }

    fn context_by_entry_key<'a>(
        contexts: &'a [ContextFragment],
        relative_path: &str,
    ) -> &'a ContextFragment {
        contexts
            .iter()
            .find(|context| canonical_vault_entry_key(context).relative_path == relative_path)
            .unwrap_or_else(|| panic!("expected context with vault entry key {relative_path}"))
    }

    #[test]
    fn discovers_global_vault_under_ctx_home_without_creating_it() {
        let base = std::env::temp_dir().join(format!("ctx-vault-discovery-{}", Uuid::new_v4()));
        let home = base.join("home");

        let discovered = global_vault_path_from_home(Some(OsString::from(&home)), &base);

        assert_eq!(discovered, home.join(CTX_HOME_DIR).join(GLOBAL_VAULT_DIR));
        assert!(
            !home.exists(),
            "global vault discovery must not create ~/.ctx/vault or its parents"
        );
    }

    #[test]
    fn vault_roots_discover_uses_global_vault_directory() {
        let working_dir =
            std::env::temp_dir().join(format!("ctx-vault-discover-roots-{}", Uuid::new_v4()));
        let roots = VaultRoots::discover(&working_dir);

        assert!(roots
            .global_root
            .ends_with(PathBuf::from(CTX_HOME_DIR).join(GLOBAL_VAULT_DIR)));
        assert_eq!(
            roots.local_root,
            Some(working_dir.join(CTX_HOME_DIR).join(GLOBAL_VAULT_DIR))
        );
    }

    #[test]
    fn discovers_project_local_vault_relative_to_working_dir_without_creating_it() {
        let working_dir =
            std::env::temp_dir().join(format!("ctx-local-vault-discovery-{}", Uuid::new_v4()));

        let discovered = discover_project_local_vault_path(&working_dir);

        assert_eq!(
            discovered,
            working_dir.join(CTX_HOME_DIR).join(GLOBAL_VAULT_DIR)
        );
        assert!(
            !working_dir.exists(),
            "project-local vault discovery must not create .ctx/vault or its parents"
        );
    }

    #[test]
    fn global_vault_discovery_has_no_filesystem_side_effects() {
        let fallback =
            std::env::temp_dir().join(format!("ctx-vault-discover-fallback-{}", Uuid::new_v4()));

        let discovered = discover_global_vault_path(&fallback);

        assert!(discovered.ends_with(PathBuf::from(CTX_HOME_DIR).join(GLOBAL_VAULT_DIR)));
        assert!(
            !fallback.exists(),
            "global vault path discovery must not create fallback directories"
        );
    }

    #[test]
    fn initializes_global_vault_directory_structure_when_missing() {
        let base = std::env::temp_dir().join(format!("ctx-vault-init-{}", Uuid::new_v4()));
        let home = base.join("home");

        let initialized =
            initialize_global_vault_path_from_home(Some(OsString::from(&home)), &base)
                .expect("global vault initialization should create missing directories");

        assert_eq!(
            initialized.global_root,
            home.join(CTX_HOME_DIR).join(GLOBAL_VAULT_DIR)
        );
        assert_eq!(
            initialized.contexts_dir,
            home.join(CTX_HOME_DIR)
                .join(GLOBAL_VAULT_DIR)
                .join(MANAGED_CONTEXTS_DIR)
        );
        assert!(initialized.global_root.is_dir());
        assert!(initialized.contexts_dir.is_dir());

        fs::remove_dir_all(base).ok();
    }

    #[test]
    fn global_vault_initialization_is_idempotent() {
        let base =
            std::env::temp_dir().join(format!("ctx-vault-init-idempotent-{}", Uuid::new_v4()));
        let home = base.join("home");

        initialize_global_vault_path_from_home(Some(OsString::from(&home)), &base)
            .expect("initial global vault initialization should pass");
        let initialized =
            initialize_global_vault_path_from_home(Some(OsString::from(&home)), &base)
                .expect("repeat global vault initialization should pass");

        assert!(initialized.contexts_dir.is_dir());
        fs::remove_dir_all(base).ok();
    }

    #[test]
    fn initializes_project_local_vault_directory_structure_when_missing() {
        let working_dir =
            std::env::temp_dir().join(format!("ctx-local-vault-init-{}", Uuid::new_v4()));

        let initialized = initialize_project_local_vault(&working_dir)
            .expect("project-local vault initialization should create missing directories");

        assert_eq!(
            initialized.local_root,
            working_dir.join(CTX_HOME_DIR).join(GLOBAL_VAULT_DIR)
        );
        assert_eq!(
            initialized.contexts_dir,
            working_dir
                .join(CTX_HOME_DIR)
                .join(GLOBAL_VAULT_DIR)
                .join(MANAGED_CONTEXTS_DIR)
        );
        assert!(initialized.local_root.is_dir());
        assert!(initialized.contexts_dir.is_dir());

        fs::remove_dir_all(working_dir).ok();
    }

    #[test]
    fn project_local_vault_initialization_is_idempotent() {
        let working_dir = std::env::temp_dir().join(format!(
            "ctx-local-vault-init-idempotent-{}",
            Uuid::new_v4()
        ));

        initialize_project_local_vault(&working_dir)
            .expect("initial project-local vault initialization should pass");
        let initialized = initialize_project_local_vault(&working_dir)
            .expect("repeat project-local vault initialization should pass");

        assert!(initialized.contexts_dir.is_dir());
        fs::remove_dir_all(working_dir).ok();
    }

    #[test]
    fn creates_markdown_context_in_managed_directory() {
        let (roots, base) = test_roots();
        let context = create_context_file(
            &roots,
            VaultScope::Local,
            "agents",
            "main-agent.md",
            "# Main\n\nSee [[Shared Rust Patterns]].",
        )
        .expect("context file should be created");

        let expected_path = managed_contexts_dir(roots.local_root.as_ref().unwrap())
            .join("agents")
            .join("main-agent.md");
        assert_eq!(context.file_path, expected_path);
        assert_eq!(context.folder_path, PathBuf::from("agents"));
        assert_eq!(context.title, "main agent");
        assert_eq!(context.wikilinks, vec!["Shared Rust Patterns"]);
        assert_eq!(
            fs::read_to_string(&context.file_path).expect("created markdown should be readable"),
            "# Main\n\nSee [[Shared Rust Patterns]]."
        );

        fs::remove_dir_all(base).ok();
    }

    #[test]
    fn lists_context_files_from_vault_roots_after_creation() {
        let (roots, base) = test_roots();
        let created = create_context_file(
            &roots,
            VaultScope::Local,
            "agents",
            "main-agent.md",
            "# Main\n\nSee [[Shared Rules]].",
        )
        .expect("context file should be created");

        let contexts = list_context_files(&roots).expect("context files should be listed");
        let listed = contexts
            .iter()
            .find(|context| context.file_path == created.file_path)
            .expect("created context should be present in refreshed file list");

        assert_eq!(listed.title, "main agent");
        assert_eq!(listed.folder_path, PathBuf::from("agents"));
        assert_eq!(listed.wikilinks, vec!["Shared Rules"]);
        fs::remove_dir_all(base).ok();
    }

    #[test]
    fn reads_selected_markdown_context_file_contents() {
        let base = std::env::temp_dir().join(format!("ctx-read-context-{}", Uuid::new_v4()));
        fs::create_dir_all(&base).expect("test directory should be created");
        let file_path = base.join("agent.md");
        fs::write(&file_path, "# Agent\n\nUse [[Shared Context]].")
            .expect("selected context should be writable");

        let content = read_markdown_context_file(&file_path)
            .expect("selected markdown context should be readable");

        assert_eq!(content, "# Agent\n\nUse [[Shared Context]].");
        fs::remove_dir_all(base).ok();
    }

    #[test]
    fn updates_selected_markdown_context_file_contents() {
        let base = std::env::temp_dir().join(format!("ctx-update-context-{}", Uuid::new_v4()));
        fs::create_dir_all(&base).expect("test directory should be created");
        let file_path = base.join("agent.md");
        fs::write(&file_path, "# Agent").expect("selected context should be writable");

        let content = update_markdown_context_file(&file_path, "# Agent\n\nUpdated [[Rules]].")
            .expect("selected markdown context should be writable");

        assert_eq!(content, "# Agent\n\nUpdated [[Rules]].");
        assert_eq!(
            fs::read_to_string(&file_path).expect("updated context should be readable"),
            "# Agent\n\nUpdated [[Rules]]."
        );
        fs::remove_dir_all(base).ok();
    }

    #[test]
    fn deletes_selected_markdown_context_file() {
        let base = std::env::temp_dir().join(format!("ctx-delete-context-{}", Uuid::new_v4()));
        fs::create_dir_all(&base).expect("test directory should be created");
        let file_path = base.join("agent.md");
        fs::write(&file_path, "# Agent").expect("selected context should be writable");

        let deleted_path = delete_markdown_context_file(&file_path)
            .expect("selected markdown context should be deletable");

        assert_eq!(deleted_path, file_path);
        assert!(!deleted_path.exists());
        fs::remove_dir_all(base).ok();
    }

    #[test]
    fn delete_rejects_missing_and_non_markdown_context_paths() {
        let base =
            std::env::temp_dir().join(format!("ctx-delete-invalid-context-{}", Uuid::new_v4()));
        fs::create_dir_all(&base).expect("test directory should be created");
        let missing = base.join("missing.md");
        let non_markdown = base.join("notes.txt");
        fs::write(&non_markdown, "not markdown").expect("non-markdown file should be writable");

        let missing_error = delete_markdown_context_file(&missing)
            .expect_err("missing context paths should be rejected");
        let extension_error = delete_markdown_context_file(&non_markdown)
            .expect_err("non-markdown context paths should be rejected");

        assert!(matches!(missing_error, VaultError::MissingContext(_)));
        assert!(matches!(extension_error, VaultError::InvalidExtension(_)));
        assert!(non_markdown.exists());
        fs::remove_dir_all(base).ok();
    }

    #[test]
    fn list_context_files_prefers_local_override_for_matching_relative_path() {
        let (roots, base) = test_roots();
        let global = create_context_file(
            &roots,
            VaultScope::Global,
            "agents",
            "shared-rules.md",
            "# Global Shared Rules",
        )
        .expect("global context should be created");
        let local = create_context_file(
            &roots,
            VaultScope::Local,
            "agents",
            "shared-rules.md",
            "# Local Shared Rules",
        )
        .expect("local override should be created");

        let contexts = list_context_files(&roots).expect("contexts should be listed");

        assert_eq!(contexts.len(), 1);
        let resolved = context_by_entry_key(&contexts, "agents/shared-rules.md");
        assert_eq!(resolved.vault_scope, VaultScope::Local);
        assert_eq!(resolved.file_path, local.file_path);
        assert_eq!(resolved.content, "# Local Shared Rules");
        assert!(!contexts
            .iter()
            .any(|context| context.file_path == global.file_path));

        fs::remove_dir_all(base).ok();
    }

    #[test]
    fn list_context_files_derives_backlinks_from_resolved_wikilink_targets() {
        let (roots, base) = test_roots();
        create_context_file(
            &roots,
            VaultScope::Local,
            "notes",
            "target.md",
            "# Target\n\nLinked target.",
        )
        .expect("target context should be created");
        create_context_file(
            &roots,
            VaultScope::Local,
            "agents",
            "source.md",
            "# Source\n\nUses [[notes/target#intro|target alias]] and [[missing]].",
        )
        .expect("source context should be created");

        let contexts = list_context_files(&roots).expect("contexts should list");
        let target = contexts
            .iter()
            .find(|context| context.title == "target")
            .expect("target context should be listed");
        let source = contexts
            .iter()
            .find(|context| context.title == "source")
            .expect("source context should be listed");

        assert_eq!(target.backlinks, vec!["source"]);
        assert!(source.backlinks.is_empty());

        fs::remove_dir_all(base).ok();
    }

    #[test]
    fn delete_resolved_context_removes_only_active_overlay_path() {
        let (roots, base) = test_roots();
        let global = create_context_file(
            &roots,
            VaultScope::Global,
            "agents",
            "shared-rules.md",
            "# Global Shared Rules",
        )
        .expect("global context should be created");
        let local = create_context_file(
            &roots,
            VaultScope::Local,
            "agents",
            "shared-rules.md",
            "# Local Shared Rules",
        )
        .expect("local override should be created");
        let working_dir = base.join("project");

        let deleted_path = delete_resolved_context_markdown(&working_dir, &local.file_path)
            .expect("active local overlay path should be deletable");

        assert_eq!(deleted_path, local.file_path);
        assert!(!local.file_path.exists());
        assert!(
            global.file_path.exists(),
            "deleting the local override must not remove the shadowed global context"
        );

        fs::remove_dir_all(base).ok();
    }

    #[test]
    fn delete_resolved_context_rejects_shadowed_global_overlay_path() {
        let (roots, base) = test_roots();
        let global = create_context_file(
            &roots,
            VaultScope::Global,
            "agents",
            "shared-rules.md",
            "# Global Shared Rules",
        )
        .expect("global context should be created");
        create_context_file(
            &roots,
            VaultScope::Local,
            "agents",
            "shared-rules.md",
            "# Local Shared Rules",
        )
        .expect("local override should be created");
        let working_dir = base.join("project");

        let error = delete_resolved_context_markdown(&working_dir, &global.file_path)
            .expect_err("shadowed global overlay path should be rejected");

        assert!(error
            .to_string()
            .contains("context file is not part of the resolved vault overlay"));
        assert!(global.file_path.exists());
        fs::remove_dir_all(base).ok();
    }

    #[test]
    fn read_resolved_context_accepts_global_only_context_without_local_match() {
        let base = std::env::temp_dir().join(format!("ctx-global-only-resolve-{}", Uuid::new_v4()));
        let home = base.join("home");
        let working_dir = base.join("project");
        let roots = VaultRoots {
            global_root: home.join(CTX_HOME_DIR).join(GLOBAL_VAULT_DIR),
            local_root: Some(working_dir.join(CTX_HOME_DIR).join(GLOBAL_VAULT_DIR)),
        };
        let global = create_context_file(
            &roots,
            VaultScope::Global,
            "agents",
            "global-only.md",
            "# Global Only\n\nShared guidance.",
        )
        .expect("global-only context should be created");

        with_home(&home, || {
            let content = read_resolved_context_markdown(&working_dir, &global.file_path)
                .expect("global-only context should resolve without a local match");

            assert_eq!(content, "# Global Only\n\nShared guidance.");
        });

        fs::remove_dir_all(base).ok();
    }

    #[test]
    fn read_resolved_context_accepts_local_only_context_without_global_match() {
        let base = std::env::temp_dir().join(format!("ctx-local-only-resolve-{}", Uuid::new_v4()));
        let home = base.join("home");
        let working_dir = base.join("project");
        let roots = VaultRoots {
            global_root: home.join(CTX_HOME_DIR).join(GLOBAL_VAULT_DIR),
            local_root: Some(working_dir.join(CTX_HOME_DIR).join(GLOBAL_VAULT_DIR)),
        };
        let local = create_context_file(
            &roots,
            VaultScope::Local,
            "skills",
            "local-only.md",
            "# Local Only\n\nProject-specific guidance.",
        )
        .expect("local-only context should be created");

        with_home(&home, || {
            let content = read_resolved_context_markdown(&working_dir, &local.file_path)
                .expect("local-only context should resolve without a global match");

            assert_eq!(content, "# Local Only\n\nProject-specific guidance.");
        });

        fs::remove_dir_all(base).ok();
    }

    #[test]
    fn list_context_files_preserves_global_only_contexts() {
        let (roots, base) = test_roots();
        let global = create_context_file(
            &roots,
            VaultScope::Global,
            "agents",
            "global-only.md",
            "# Global Only",
        )
        .expect("global-only context should be created");

        let contexts = list_context_files(&roots).expect("contexts should be listed");

        assert_eq!(contexts.len(), 1);
        let listed = context_by_entry_key(&contexts, "agents/global-only.md");
        assert_eq!(listed.vault_scope, VaultScope::Global);
        assert_eq!(listed.file_path, global.file_path);
        assert_eq!(listed.content, "# Global Only");

        fs::remove_dir_all(base).ok();
    }

    #[test]
    fn list_context_files_preserves_local_only_contexts() {
        let (roots, base) = test_roots();
        let local = create_context_file(
            &roots,
            VaultScope::Local,
            "skills",
            "local-only.md",
            "# Local Only",
        )
        .expect("local-only context should be created");

        let contexts = list_context_files(&roots).expect("contexts should be listed");

        assert_eq!(contexts.len(), 1);
        let listed = context_by_entry_key(&contexts, "skills/local-only.md");
        assert_eq!(listed.vault_scope, VaultScope::Local);
        assert_eq!(listed.file_path, local.file_path);
        assert_eq!(listed.content, "# Local Only");

        fs::remove_dir_all(base).ok();
    }

    #[test]
    fn list_context_files_resolves_mixed_overlay_cases() {
        let (roots, base) = test_roots();
        let global_only = create_context_file(
            &roots,
            VaultScope::Global,
            "agents",
            "global-only.md",
            "# Global Only",
        )
        .expect("global-only context should be created");
        let global_shadowed = create_context_file(
            &roots,
            VaultScope::Global,
            "agents",
            "shared-rules.md",
            "# Global Shared",
        )
        .expect("global context should be created");
        let local_override = create_context_file(
            &roots,
            VaultScope::Local,
            "agents",
            "shared-rules.md",
            "# Local Shared",
        )
        .expect("local override should be created");
        let local_only = create_context_file(
            &roots,
            VaultScope::Local,
            "skills",
            "local-only.md",
            "# Local Only",
        )
        .expect("local-only context should be created");

        let contexts = list_context_files(&roots).expect("contexts should be listed");

        assert_eq!(contexts.len(), 3);

        let listed_global_only = context_by_entry_key(&contexts, "agents/global-only.md");
        assert_eq!(listed_global_only.vault_scope, VaultScope::Global);
        assert_eq!(listed_global_only.file_path, global_only.file_path);

        let listed_override = context_by_entry_key(&contexts, "agents/shared-rules.md");
        assert_eq!(listed_override.vault_scope, VaultScope::Local);
        assert_eq!(listed_override.file_path, local_override.file_path);
        assert_eq!(listed_override.content, "# Local Shared");

        let listed_local_only = context_by_entry_key(&contexts, "skills/local-only.md");
        assert_eq!(listed_local_only.vault_scope, VaultScope::Local);
        assert_eq!(listed_local_only.file_path, local_only.file_path);

        assert!(!contexts
            .iter()
            .any(|context| context.file_path == global_shadowed.file_path));

        fs::remove_dir_all(base).ok();
    }

    #[test]
    fn resolve_overlay_vault_merges_global_and_local_vaults_with_local_overrides() {
        let base = std::env::temp_dir().join(format!("ctx-resolved-overlay-{}", Uuid::new_v4()));
        let home = base.join("home");
        let working_dir = base.join("project");
        let roots = VaultRoots {
            global_root: home.join(CTX_HOME_DIR).join(GLOBAL_VAULT_DIR),
            local_root: Some(working_dir.join(CTX_HOME_DIR).join(GLOBAL_VAULT_DIR)),
        };
        let global_only = create_context_file(
            &roots,
            VaultScope::Global,
            "agents",
            "global-only.md",
            "# Global Only",
        )
        .expect("global-only context should be created");
        let global_shadowed = create_context_file(
            &roots,
            VaultScope::Global,
            "agents",
            "shared-rules.md",
            "# Global Shared",
        )
        .expect("global shared context should be created");
        let local_override = create_context_file(
            &roots,
            VaultScope::Local,
            "agents",
            "shared-rules.md",
            "# Local Shared",
        )
        .expect("local override should be created");
        let local_only = create_context_file(
            &roots,
            VaultScope::Local,
            "skills",
            "local-only.md",
            "# Local Only",
        )
        .expect("local-only context should be created");

        with_home(&home, || {
            let resolved =
                resolve_overlay_vault(&working_dir).expect("vault overlay should resolve");

            assert_eq!(resolved.roots.global_root, roots.global_root);
            assert_eq!(resolved.roots.local_root, roots.local_root);
            assert_eq!(resolved.contexts.len(), 3);

            let listed_global_only =
                context_by_entry_key(&resolved.contexts, "agents/global-only.md");
            assert_eq!(listed_global_only.vault_scope, VaultScope::Global);
            assert_eq!(listed_global_only.file_path, global_only.file_path);

            let listed_override =
                context_by_entry_key(&resolved.contexts, "agents/shared-rules.md");
            assert_eq!(listed_override.vault_scope, VaultScope::Local);
            assert_eq!(listed_override.file_path, local_override.file_path);
            assert_eq!(listed_override.content, "# Local Shared");

            let listed_local_only =
                context_by_entry_key(&resolved.contexts, "skills/local-only.md");
            assert_eq!(listed_local_only.vault_scope, VaultScope::Local);
            assert_eq!(listed_local_only.file_path, local_only.file_path);

            assert!(!resolved
                .contexts
                .iter()
                .any(|context| context.file_path == global_shadowed.file_path));
        });

        fs::remove_dir_all(base).ok();
    }

    #[test]
    fn canonical_vault_entry_key_matches_same_relative_path_across_scopes() {
        let (roots, base) = test_roots();
        let global = create_context_file(
            &roots,
            VaultScope::Global,
            "agents",
            "shared-rules.md",
            "# Global",
        )
        .expect("global context should be created");
        let local = create_context_file(
            &roots,
            VaultScope::Local,
            "agents",
            "shared-rules.md",
            "# Local",
        )
        .expect("local context should be created");

        assert_eq!(
            canonical_vault_entry_key(&global),
            canonical_vault_entry_key(&local)
        );
        assert_eq!(
            canonical_vault_entry_key(&local).relative_path,
            "agents/shared-rules.md"
        );

        fs::remove_dir_all(base).ok();
    }

    #[test]
    fn overlay_resolution_uses_canonical_entry_key_instead_of_title() {
        let (roots, base) = test_roots();
        let global = create_context_file(
            &roots,
            VaultScope::Global,
            "agents",
            "main-agent.md",
            "# Global",
        )
        .expect("global context should be created");
        let local_same_title = create_context_file(
            &roots,
            VaultScope::Local,
            "agents",
            "main_agent.md",
            "# Local",
        )
        .expect("local context with same derived title should be created");

        assert_eq!(global.title, local_same_title.title);
        assert_ne!(
            canonical_vault_entry_key(&global),
            canonical_vault_entry_key(&local_same_title)
        );

        let resolved = resolve_overlay(vec![global.clone(), local_same_title.clone()]);

        assert_eq!(resolved.len(), 2);
        assert!(resolved
            .iter()
            .any(|context| context.file_path == global.file_path));
        assert!(resolved
            .iter()
            .any(|context| context.file_path == local_same_title.file_path));

        fs::remove_dir_all(base).ok();
    }

    #[test]
    fn overlay_resolution_prefers_local_context_with_same_canonical_entry_key() {
        let (roots, base) = test_roots();
        let global = create_context_file(
            &roots,
            VaultScope::Global,
            "agents",
            "shared-rules.md",
            "# Global",
        )
        .expect("global context should be created");
        let local = create_context_file(
            &roots,
            VaultScope::Local,
            "agents",
            "shared-rules.md",
            "# Local",
        )
        .expect("local context should be created");

        let resolved = resolve_overlay(vec![global.clone(), local.clone()]);

        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].file_path, local.file_path);

        fs::remove_dir_all(base).ok();
    }

    #[test]
    fn overlay_resolution_keeps_unmatched_entries_from_both_vaults() {
        let (roots, base) = test_roots();
        let global_only = create_context_file(
            &roots,
            VaultScope::Global,
            "agents",
            "global-only.md",
            "# Global Only",
        )
        .expect("global-only context should be created");
        let global_shadowed = create_context_file(
            &roots,
            VaultScope::Global,
            "agents",
            "shared-rules.md",
            "# Global Shared",
        )
        .expect("global shared context should be created");
        let local_shadow = create_context_file(
            &roots,
            VaultScope::Local,
            "agents",
            "shared-rules.md",
            "# Local Shared",
        )
        .expect("local shared context should be created");
        let local_only = create_context_file(
            &roots,
            VaultScope::Local,
            "skills",
            "local-only.md",
            "# Local Only",
        )
        .expect("local-only context should be created");

        let resolved = resolve_overlay(vec![
            global_only.clone(),
            global_shadowed.clone(),
            local_shadow.clone(),
            local_only.clone(),
        ]);

        assert_eq!(resolved.len(), 3);
        assert!(resolved
            .iter()
            .any(|context| context.file_path == global_only.file_path));
        assert!(resolved
            .iter()
            .any(|context| context.file_path == local_shadow.file_path));
        assert!(resolved
            .iter()
            .any(|context| context.file_path == local_only.file_path));
        assert!(!resolved
            .iter()
            .any(|context| context.file_path == global_shadowed.file_path));

        fs::remove_dir_all(base).ok();
    }

    #[test]
    fn overlay_resolution_replaces_matching_global_when_local_is_seen_first() {
        let (roots, base) = test_roots();
        let global = create_context_file(
            &roots,
            VaultScope::Global,
            "agents",
            "shared-rules.md",
            "# Global",
        )
        .expect("global context should be created");
        let local = create_context_file(
            &roots,
            VaultScope::Local,
            "agents",
            "shared-rules.md",
            "# Local",
        )
        .expect("local context should be created");

        let resolved = resolve_overlay(vec![local.clone(), global.clone()]);

        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].file_path, local.file_path);

        fs::remove_dir_all(base).ok();
    }

    #[test]
    fn discovers_existing_agent_contexts_and_skill_markdown() {
        let base = std::env::temp_dir().join(format!("ctx-discover-existing-{}", Uuid::new_v4()));
        fs::create_dir_all(base.join(".claude").join("skills"))
            .expect("skills directory should be created");
        fs::create_dir_all(base.join("agents")).expect("agents directory should be created");
        fs::create_dir_all(base.join(".agents")).expect(".agents directory should be created");
        fs::write(base.join("AGENTS.md"), "# Codex\n\nSee [[Skill One]].")
            .expect("AGENTS.md should be writable");
        fs::write(base.join("agent.md"), "# Agent").expect("agent.md should be writable");
        fs::write(base.join(".agents").join("agents.md"), "# Agent manifest")
            .expect("agents.md should be writable");
        fs::write(base.join("agents").join("reviewer.md"), "# Reviewer")
            .expect("subagent context should be writable");
        fs::write(
            base.join(".claude").join("skills").join("skill-one.md"),
            "# Skill One",
        )
        .expect("skill markdown should be writable");

        let contexts =
            discover_existing_context_files(&base).expect("existing context files should scan");

        assert!(contexts.iter().any(|context| {
            context.file_path == base.join("AGENTS.md")
                && context.tags.contains(&"codex".to_string())
                && context.import_source == Some(base.join("AGENTS.md"))
                && context.import_source_type == Some(ImportSourceType::CodexAgents)
        }));
        assert!(contexts.iter().any(|context| {
            context.file_path == base.join("agent.md")
                && context.classification == Classification::MainAgent
                && context.llm_classification_status == ClassificationStatus::Classified
        }));
        assert!(contexts.iter().any(|context| {
            context.file_path == base.join(".agents").join("agents.md")
                && context.classification == Classification::MainAgent
        }));
        assert!(contexts.iter().any(|context| {
            context.file_path == base.join("agents").join("reviewer.md")
                && context.classification == Classification::Subagent
                && context.tags.contains(&"agents".to_string())
        }));
        assert!(contexts.iter().any(|context| {
            context.file_path == base.join(".claude").join("skills").join("skill-one.md")
                && context.tags.contains(&"skills".to_string())
        }));

        fs::remove_dir_all(base).ok();
    }

    #[test]
    fn discovery_results_are_normalized_with_root_and_metadata() {
        let base = std::env::temp_dir().join(format!("ctx-discovery-results-{}", Uuid::new_v4()));
        fs::create_dir_all(base.join(".claude").join("skills"))
            .expect("skills directory should be created");
        fs::write(
            base.join(".claude").join("skills").join("review.md"),
            "# Review\n\nSee [[Checklist]].",
        )
        .expect("skill markdown should be writable");

        let results = discover_existing_context_file_results(&base)
            .expect("existing context discovery results should scan");
        let result = results
            .iter()
            .find(|result| result.file_name == "review.md")
            .expect("review skill should be discovered");

        assert_eq!(
            result.file_path,
            base.join(".claude").join("skills").join("review.md")
        );
        assert_eq!(result.root_source, base);
        assert_eq!(result.metadata.title, "Review");
        assert_eq!(result.metadata.vault_scope, VaultScope::Local);
        assert_eq!(result.metadata.classification, Classification::Shared);
        assert_eq!(
            result.metadata.inferred_classification,
            Some(Classification::Shared)
        );
        assert_eq!(
            result.metadata.llm_classification_status,
            ClassificationStatus::Classified
        );
        assert_eq!(result.source_type, ImportSourceType::SkillMarkdown);
        assert_eq!(result.metadata.folder_path, PathBuf::from(".claude/skills"));
        assert!(result.metadata.tags.contains(&"skills".to_string()));
        assert_eq!(result.metadata.wikilinks, vec!["Checklist"]);

        fs::remove_dir_all(base).ok();
    }

    #[test]
    fn discovery_results_apply_import_time_classification_before_review() {
        let base =
            std::env::temp_dir().join(format!("ctx-discovery-classified-{}", Uuid::new_v4()));
        fs::create_dir_all(&base).expect("base directory should be created");
        fs::write(
            base.join("AGENTS.md"),
            "---\nclassification: subagent\n---\n# Reviewer\nUse [[Shared Style Guide]].",
        )
        .expect("AGENTS.md should be writable");

        let results = discover_existing_context_file_results(&base)
            .expect("existing context discovery results should scan");
        let result = results
            .iter()
            .find(|result| result.file_name == "AGENTS.md")
            .expect("AGENTS.md should be discovered");

        assert_eq!(result.metadata.classification, Classification::Subagent);
        assert_eq!(
            result.metadata.inferred_classification,
            Some(Classification::Subagent)
        );
        assert_eq!(
            result.metadata.llm_classification_status,
            ClassificationStatus::Classified
        );
        assert!(result.metadata.tags.contains(&"subagent".to_string()));
        assert_eq!(result.metadata.wikilinks, vec!["Shared Style Guide"]);

        fs::remove_dir_all(base).ok();
    }

    #[test]
    fn materializes_discovered_contexts_into_managed_vault_conventions() {
        let base = std::env::temp_dir().join(format!("ctx-materialize-{}", Uuid::new_v4()));
        fs::create_dir_all(base.join(".claude").join("skills"))
            .expect("skills directory should be created");
        fs::create_dir_all(base.join("agents")).expect("agents directory should be created");
        fs::write(base.join("AGENTS.md"), "# Codex\n\nUse [[Review]].")
            .expect("root AGENTS.md should be writable");
        fs::write(base.join("agents").join("reviewer.md"), "# Reviewer")
            .expect("subagent context should be writable");
        fs::write(
            base.join(".claude").join("skills").join("review.md"),
            "# Review",
        )
        .expect("skill markdown should be writable");

        let imported = materialize_discovered_context_files(&base)
            .expect("discovered contexts should materialize into the vault");
        let imported_paths = imported
            .iter()
            .map(|context| context.file_path.clone())
            .collect::<HashSet<_>>();

        assert!(imported_paths.contains(
            &base
                .join(CTX_HOME_DIR)
                .join(GLOBAL_VAULT_DIR)
                .join(MANAGED_CONTEXTS_DIR)
                .join("AGENTS.md")
        ));
        assert!(imported_paths.contains(
            &base
                .join(CTX_HOME_DIR)
                .join(GLOBAL_VAULT_DIR)
                .join(MANAGED_CONTEXTS_DIR)
                .join("agents")
                .join("reviewer.md")
        ));
        assert!(imported_paths.contains(
            &base
                .join(CTX_HOME_DIR)
                .join(GLOBAL_VAULT_DIR)
                .join(MANAGED_CONTEXTS_DIR)
                .join(".claude")
                .join("skills")
                .join("review.md")
        ));
        assert!(imported.iter().any(|context| {
            context.import_source == Some(base.join("AGENTS.md"))
                && context.import_source_type == Some(ImportSourceType::CodexAgents)
                && context.classification == Classification::MainAgent
                && context.llm_classification_status == ClassificationStatus::Classified
        }));
        let listed = list_context_files(&VaultRoots::discover(&base))
            .expect("managed vault listing should keep import metadata");
        assert!(listed.iter().any(|context| {
            context.file_path
                == base
                    .join(CTX_HOME_DIR)
                    .join(GLOBAL_VAULT_DIR)
                    .join(MANAGED_CONTEXTS_DIR)
                    .join("AGENTS.md")
                && context.import_source == Some(base.join("AGENTS.md"))
                && context.import_source_type == Some(ImportSourceType::CodexAgents)
                && context.classification == Classification::MainAgent
                && context.inferred_classification == Some(Classification::MainAgent)
                && context.tags.contains(&"codex".to_string())
                && context.llm_classification_status == ClassificationStatus::Classified
        }));
        assert!(listed.iter().any(|context| {
            context.file_path
                == base
                    .join(CTX_HOME_DIR)
                    .join(GLOBAL_VAULT_DIR)
                    .join(MANAGED_CONTEXTS_DIR)
                    .join(".claude")
                    .join("skills")
                    .join("review.md")
                && context.import_source
                    == Some(base.join(".claude").join("skills").join("review.md"))
                && context.import_source_type == Some(ImportSourceType::SkillMarkdown)
                && context.classification == Classification::Shared
                && context.inferred_classification == Some(Classification::Shared)
                && context.tags.contains(&"skills".to_string())
                && context.llm_classification_status == ClassificationStatus::Classified
        }));
        assert!(
            import_metadata_path(&base.join(CTX_HOME_DIR).join(GLOBAL_VAULT_DIR)).is_file(),
            "import metadata index should be persisted in the vault root"
        );
        assert_eq!(
            fs::read_to_string(
                base.join(CTX_HOME_DIR)
                    .join(GLOBAL_VAULT_DIR)
                    .join(MANAGED_CONTEXTS_DIR)
                    .join("AGENTS.md")
            )
            .expect("materialized AGENTS.md should be readable"),
            "# Codex\n\nUse [[Review]]."
        );

        fs::remove_dir_all(base).ok();
    }

    #[test]
    fn materialized_import_reanalyzes_discovered_context_before_saving() {
        let base =
            std::env::temp_dir().join(format!("ctx-materialize-classified-{}", Uuid::new_v4()));
        fs::create_dir_all(&base).expect("base directory should be created");
        fs::write(
            base.join("AGENTS.md"),
            "---\nclassification: subagent\n---\n# Reviewer\nUse [[Shared Style Guide]].",
        )
        .expect("AGENTS.md should be writable");

        let imported = materialize_discovered_context_files(&base)
            .expect("discovered contexts should materialize into the vault");
        let context = imported
            .iter()
            .find(|context| context.file_path.ends_with("AGENTS.md"))
            .expect("AGENTS.md should be imported");
        let listed = list_context_files(&VaultRoots::discover(&base))
            .expect("managed vault listing should keep import metadata");
        let persisted = listed
            .iter()
            .find(|context| context.file_path.ends_with("AGENTS.md"))
            .expect("materialized AGENTS.md should be listed");

        assert_eq!(context.classification, Classification::Subagent);
        assert_eq!(
            context.import_classification_suggestion,
            Some(Classification::Subagent)
        );
        assert_eq!(
            context.inferred_classification,
            Some(Classification::Subagent)
        );
        assert_eq!(
            context.llm_classification_status,
            ClassificationStatus::Classified
        );
        assert_eq!(persisted.classification, Classification::Subagent);
        assert_eq!(
            persisted.llm_classification_status,
            ClassificationStatus::Classified
        );
        assert!(persisted.tags.contains(&"subagent".to_string()));

        fs::remove_dir_all(base).ok();
    }

    #[test]
    fn import_pipeline_persists_classification_metadata_for_review() {
        let base =
            std::env::temp_dir().join(format!("ctx-import-classification-{}", Uuid::new_v4()));
        fs::create_dir_all(base.join("agents")).expect("agents directory should be created");
        fs::write(
            base.join("agents").join("reviewer.md"),
            "# Review Agent\nThis delegated subagent reviews pull requests and uses [[Review Checklist]].",
        )
        .expect("subagent context should be writable");

        let imported = materialize_discovered_context_files(&base)
            .expect("discovered subagent should materialize into the local vault");
        let context = imported
            .iter()
            .find(|context| context.file_path.ends_with("reviewer.md"))
            .expect("reviewer context should be imported");

        assert_eq!(context.classification, Classification::Subagent);
        assert_eq!(
            context.inferred_classification,
            Some(Classification::Subagent)
        );
        assert_eq!(
            context.llm_classification_status,
            ClassificationStatus::Classified
        );
        assert_eq!(
            context.import_source,
            Some(base.join("agents").join("reviewer.md"))
        );
        assert_eq!(
            context.import_source_type,
            Some(ImportSourceType::SubagentMarkdown)
        );
        assert!(context.tags.contains(&"subagent".to_string()));
        assert_eq!(context.wikilinks, vec!["Review Checklist"]);

        let listed = list_context_files(&VaultRoots::discover(&base))
            .expect("managed listing should hydrate persisted import metadata");
        let persisted = listed
            .iter()
            .find(|context| context.file_path.ends_with("reviewer.md"))
            .expect("persisted reviewer context should be listed");

        assert_eq!(persisted.classification, Classification::Subagent);
        assert_eq!(
            persisted.llm_classification_status,
            ClassificationStatus::Classified
        );
        assert_eq!(
            persisted.import_source,
            Some(base.join("agents").join("reviewer.md"))
        );
        assert_eq!(
            persisted.import_source_type,
            Some(ImportSourceType::SubagentMarkdown)
        );
        assert!(persisted.tags.contains(&"subagent".to_string()));

        fs::remove_dir_all(base).ok();
    }

    #[test]
    fn review_import_classification_marks_matching_suggestion_reviewed() {
        let base =
            std::env::temp_dir().join(format!("ctx-review-classification-{}", Uuid::new_v4()));
        fs::create_dir_all(base.join("agents")).expect("agents directory should be created");
        fs::write(base.join("agents").join("reviewer.md"), "# Review Agent")
            .expect("subagent context should be writable");

        let imported = materialize_discovered_context_files(&base)
            .expect("discovered subagent should materialize into the local vault");
        let context = imported
            .iter()
            .find(|context| context.file_path.ends_with("reviewer.md"))
            .expect("reviewer context should be imported");

        let reviewed =
            review_import_classification(&base, &context.file_path, Classification::Subagent)
                .expect("matching review should be persisted");

        assert_eq!(reviewed.classification, Classification::Subagent);
        assert_eq!(
            reviewed.inferred_classification,
            Some(Classification::Subagent)
        );
        assert_eq!(
            reviewed.llm_classification_status,
            ClassificationStatus::Reviewed
        );

        let listed = list_context_files(&VaultRoots::discover(&base))
            .expect("managed listing should reflect reviewed status");
        let persisted = listed
            .iter()
            .find(|context| context.file_path == reviewed.file_path)
            .expect("reviewed context should remain listed");
        assert_eq!(
            persisted.llm_classification_status,
            ClassificationStatus::Reviewed
        );

        fs::remove_dir_all(base).ok();
    }

    #[test]
    fn review_import_classification_marks_user_override_modified() {
        let base = std::env::temp_dir().join(format!("ctx-review-override-{}", Uuid::new_v4()));
        fs::create_dir_all(&base).expect("base directory should be created");
        fs::write(base.join("AGENTS.md"), "# Codex\nPrimary instructions.")
            .expect("main agent context should be writable");

        let imported = materialize_discovered_context_files(&base)
            .expect("discovered main agent should materialize into the local vault");
        let context = imported
            .iter()
            .find(|context| context.file_path.ends_with("AGENTS.md"))
            .expect("AGENTS.md should be imported");

        let modified =
            review_import_classification(&base, &context.file_path, Classification::Shared)
                .expect("changed review should be persisted as a user override");

        assert_eq!(modified.classification, Classification::Shared);
        assert_eq!(
            modified.import_classification_suggestion,
            Some(Classification::MainAgent)
        );
        assert_eq!(
            modified.inferred_classification,
            Some(Classification::MainAgent)
        );
        assert_eq!(
            modified.llm_classification_status,
            ClassificationStatus::Modified
        );

        let listed = list_context_files(&VaultRoots::discover(&base))
            .expect("managed listing should reflect modified classification");
        let persisted = listed
            .iter()
            .find(|context| context.file_path == modified.file_path)
            .expect("modified context should remain listed");
        assert_eq!(persisted.classification, Classification::Shared);
        assert_eq!(
            persisted.import_classification_suggestion,
            Some(Classification::MainAgent)
        );
        assert_eq!(
            persisted.llm_classification_status,
            ClassificationStatus::Modified
        );

        fs::remove_dir_all(base).ok();
    }

    #[test]
    fn review_import_classification_persists_final_category_to_sqlite_index() {
        let base = std::env::temp_dir().join(format!("ctx-review-sqlite-{}", Uuid::new_v4()));
        fs::create_dir_all(&base).expect("base directory should be created");
        fs::write(base.join("AGENTS.md"), "# Codex\nPrimary instructions.")
            .expect("main agent context should be writable");

        let imported = materialize_discovered_context_files(&base)
            .expect("discovered context should materialize into the local vault");
        let context = imported
            .iter()
            .find(|context| context.file_path.ends_with("AGENTS.md"))
            .expect("AGENTS.md should be imported");
        reindex_markdown_contexts(&base).expect("initial sqlite index should be populated");

        let reviewed =
            review_import_classification(&base, &context.file_path, Classification::Shared)
                .expect("confirmed override should persist");
        let roots = VaultRoots::discover(&base);
        let connection = Connection::open(sqlite_index_path(roots.local_root.as_ref().unwrap()))
            .expect("local sqlite index should open");
        let relative_path = reviewed
            .file_path
            .strip_prefix(managed_contexts_dir(roots.local_root.as_ref().unwrap()))
            .expect("reviewed context should be local-managed")
            .to_string_lossy()
            .replace('\\', "/");

        assert_eq!(
            markdown_file_classification(&connection, &relative_path),
            "shared"
        );
        assert_eq!(
            markdown_file_classification_status(&connection, &relative_path),
            "modified"
        );

        fs::remove_dir_all(base).ok();
    }

    #[test]
    fn reimport_refreshes_suggestion_without_mutating_confirmed_classification() {
        let base = std::env::temp_dir().join(format!("ctx-reimport-confirmed-{}", Uuid::new_v4()));
        fs::create_dir_all(&base).expect("base directory should be created");
        let source_path = base.join("AGENTS.md");
        fs::write(&source_path, "# Codex\nPrimary instructions.")
            .expect("main agent context should be writable");

        let imported = materialize_discovered_context_files(&base)
            .expect("discovered main agent should materialize into the local vault");
        let context = imported
            .iter()
            .find(|context| context.file_path.ends_with("AGENTS.md"))
            .expect("AGENTS.md should be imported");
        let modified =
            review_import_classification(&base, &context.file_path, Classification::Shared)
                .expect("user override should be persisted");
        assert_eq!(modified.classification, Classification::Shared);
        assert_eq!(
            modified.import_classification_suggestion,
            Some(Classification::MainAgent)
        );
        assert_eq!(
            modified.llm_classification_status,
            ClassificationStatus::Modified
        );

        fs::write(
            &source_path,
            "---\nclassification: subagent\n---\n# Reviewer\nDelegated review specialist.",
        )
        .expect("source context should be updatable");
        let reimported = materialize_discovered_context_files(&base)
            .expect("reimport should update suggestions for existing materialized file");
        let reimported_context = reimported
            .iter()
            .find(|context| context.file_path == modified.file_path)
            .expect("existing materialized context should be reused");

        assert_eq!(reimported_context.classification, Classification::Shared);
        assert_eq!(
            reimported_context.import_classification_suggestion,
            Some(Classification::Subagent)
        );
        assert_eq!(
            reimported_context.inferred_classification,
            Some(Classification::Subagent)
        );
        assert_eq!(
            reimported_context.llm_classification_status,
            ClassificationStatus::Modified
        );

        let listed = list_context_files(&VaultRoots::discover(&base))
            .expect("managed listing should keep confirmed classification");
        let persisted = listed
            .iter()
            .find(|context| context.file_path == modified.file_path)
            .expect("reimported context should remain listed");
        assert_eq!(persisted.classification, Classification::Shared);
        assert_eq!(
            persisted.import_classification_suggestion,
            Some(Classification::Subagent)
        );
        assert_eq!(
            persisted.llm_classification_status,
            ClassificationStatus::Modified
        );

        fs::remove_dir_all(base).ok();
    }

    #[test]
    fn materialized_import_is_idempotent_and_keeps_conflicting_sources() {
        let base =
            std::env::temp_dir().join(format!("ctx-materialize-conflict-{}", Uuid::new_v4()));
        let working_dir = base.join("project");
        let first_root = base.join("first");
        let second_root = base.join("second");
        let local_vault = working_dir.join(CTX_HOME_DIR).join(GLOBAL_VAULT_DIR);
        fs::create_dir_all(&first_root).expect("first scan root should be created");
        fs::create_dir_all(&second_root).expect("second scan root should be created");
        fs::create_dir_all(&local_vault).expect("local vault should be created");
        fs::write(first_root.join("AGENTS.md"), "# First").expect("first context should write");
        fs::write(second_root.join("AGENTS.md"), "# Second").expect("second context should write");
        fs::write(
            vault_settings_path(&local_vault),
            r#"{"scan_roots":[{"path":"../first","scope":"local"},{"path":"../second","scope":"local"}]}"#,
        )
        .expect("local settings should be writable");

        let first_import = materialize_discovered_context_files(&working_dir)
            .expect("first import should materialize both sources");
        let second_import = materialize_discovered_context_files(&working_dir)
            .expect("second import should reuse existing materialized files");
        let contexts_dir = managed_contexts_dir(&local_vault);

        assert!(contexts_dir.join("AGENTS.md").is_file());
        assert!(contexts_dir.join("AGENTS-2.md").is_file());
        assert_eq!(first_import.len(), 2);
        assert_eq!(second_import.len(), 2);
        assert_eq!(
            fs::read_dir(&contexts_dir)
                .expect("contexts dir should be readable")
                .filter_map(Result::ok)
                .filter(|entry| entry.path().extension().and_then(|ext| ext.to_str()) == Some("md"))
                .count(),
            2
        );

        fs::remove_dir_all(base).ok();
    }

    #[test]
    fn scan_conflicts_get_unique_materialized_names_before_import() {
        let base = std::env::temp_dir().join(format!(
            "ctx-scan-conflicting-discovery-names-{}",
            Uuid::new_v4()
        ));
        let working_dir = base.join("project");
        let first_root = base.join("first");
        let second_root = base.join("second");
        let local_vault = working_dir.join(CTX_HOME_DIR).join(GLOBAL_VAULT_DIR);
        fs::create_dir_all(&first_root).expect("first scan root should be created");
        fs::create_dir_all(&second_root).expect("second scan root should be created");
        fs::create_dir_all(&local_vault).expect("local vault should be created");
        fs::write(first_root.join("AGENTS.md"), "# First").expect("first context should write");
        fs::write(second_root.join("AGENTS.md"), "# Second").expect("second context should write");
        fs::write(
            vault_settings_path(&local_vault),
            r#"{"scan_roots":[{"path":"../first","scope":"local"},{"path":"../second","scope":"local"}]}"#,
        )
        .expect("local settings should be writable");

        let results = discover_existing_context_file_results(&working_dir)
            .expect("conflicting discoveries should scan with unique names");
        let first = results
            .iter()
            .find(|result| result.file_path == first_root.join("AGENTS.md").canonicalize().unwrap())
            .expect("first source should be discovered");
        let second = results
            .iter()
            .find(|result| {
                result.file_path == second_root.join("AGENTS.md").canonicalize().unwrap()
            })
            .expect("second source should be discovered");

        assert_eq!(first.file_name, "AGENTS.md");
        assert_eq!(first.metadata.title, "AGENTS");
        assert!(!first.metadata.tags.contains(&"name-conflict".to_string()));
        assert_eq!(second.file_name, "AGENTS-2.md");
        assert_eq!(second.metadata.title, "AGENTS 2");
        assert!(second.metadata.tags.contains(&"name-conflict".to_string()));

        let imported = materialize_discovered_context_files(&working_dir)
            .expect("import should reuse scan conflict names");
        let imported_paths = imported
            .iter()
            .map(|context| context.file_path.clone())
            .collect::<HashSet<_>>();
        let contexts_dir = managed_contexts_dir(&local_vault);

        assert!(imported_paths.contains(&contexts_dir.join("AGENTS.md")));
        assert!(imported_paths.contains(&contexts_dir.join("AGENTS-2.md")));

        let rescanned = discover_existing_context_file_results(&working_dir)
            .expect("imported source names should stay stable on later scans");
        let rescanned_first = rescanned
            .iter()
            .find(|result| result.file_path == first.file_path)
            .expect("first source should remain discoverable");
        let rescanned_second = rescanned
            .iter()
            .find(|result| result.file_path == second.file_path)
            .expect("second source should remain discoverable");

        assert_eq!(rescanned_first.file_name, "AGENTS.md");
        assert_eq!(rescanned_second.file_name, "AGENTS-2.md");

        fs::remove_dir_all(base).ok();
    }

    #[test]
    fn scan_conflicts_with_existing_vault_entries_use_next_available_name() {
        let base = std::env::temp_dir().join(format!(
            "ctx-scan-conflict-existing-vault-entry-{}",
            Uuid::new_v4()
        ));
        let working_dir = base.join("project");
        let scan_root = base.join("configured-contexts");
        let local_vault = working_dir.join(CTX_HOME_DIR).join(GLOBAL_VAULT_DIR);
        let contexts_dir = managed_contexts_dir(&local_vault);
        fs::create_dir_all(&scan_root).expect("scan root should be created");
        fs::create_dir_all(&contexts_dir).expect("contexts directory should be created");
        fs::write(contexts_dir.join("AGENTS.md"), "# Managed")
            .expect("managed context should be writable");
        fs::write(scan_root.join("AGENTS.md"), "# Discovered")
            .expect("discovered context should be writable");
        fs::write(
            vault_settings_path(&local_vault),
            r#"{"scan_roots":[{"path":"../configured-contexts","scope":"local"}]}"#,
        )
        .expect("local settings should be writable");

        let results = discover_existing_context_file_results(&working_dir)
            .expect("existing managed entry should reserve its scan identity");
        let result = results
            .iter()
            .find(|result| result.file_path == scan_root.join("AGENTS.md").canonicalize().unwrap())
            .expect("discovered context should be returned");

        assert_eq!(result.file_name, "AGENTS-2.md");
        assert_eq!(result.metadata.title, "AGENTS 2");
        assert!(result.metadata.tags.contains(&"name-conflict".to_string()));

        fs::remove_dir_all(base).ok();
    }

    #[test]
    fn materialized_import_updates_existing_entry_for_changed_source() {
        let base = std::env::temp_dir().join(format!(
            "ctx-materialize-update-existing-source-{}",
            Uuid::new_v4()
        ));
        let working_dir = base.join("project");
        let scan_root = base.join("configured-contexts");
        let local_vault = working_dir.join(CTX_HOME_DIR).join(GLOBAL_VAULT_DIR);
        fs::create_dir_all(&scan_root).expect("scan root should be created");
        fs::create_dir_all(&local_vault).expect("local vault should be created");
        fs::write(scan_root.join("AGENTS.md"), "# First")
            .expect("initial source context should be writable");
        fs::write(
            vault_settings_path(&local_vault),
            r#"{"scan_roots":[{"path":"../configured-contexts","scope":"local"}]}"#,
        )
        .expect("local settings should be writable");

        let first_import = materialize_discovered_context_files(&working_dir)
            .expect("first import should materialize the source");
        fs::write(scan_root.join("AGENTS.md"), "# Second")
            .expect("changed source context should be writable");
        let second_import = materialize_discovered_context_files(&working_dir)
            .expect("second import should update the existing materialized file");
        let contexts_dir = managed_contexts_dir(&local_vault);
        let materialized_path = contexts_dir.join("AGENTS.md");

        assert_eq!(first_import.len(), 1);
        assert_eq!(second_import.len(), 1);
        assert_eq!(first_import[0].file_path, materialized_path);
        assert_eq!(second_import[0].file_path, materialized_path);
        assert_eq!(
            fs::read_to_string(&materialized_path)
                .expect("updated materialized context should be readable"),
            "# Second"
        );
        assert_eq!(
            fs::read_dir(&contexts_dir)
                .expect("contexts dir should be readable")
                .filter_map(Result::ok)
                .filter(|entry| entry.path().extension().and_then(|ext| ext.to_str()) == Some("md"))
                .count(),
            1
        );

        fs::remove_dir_all(base).ok();
    }

    #[test]
    fn repeated_scans_and_imports_do_not_create_redundant_vault_entries() {
        let base = std::env::temp_dir().join(format!(
            "ctx-repeated-scan-no-duplicates-{}",
            Uuid::new_v4()
        ));
        let working_dir = base.join("project");
        let scan_root = base.join("configured-contexts");
        let local_vault = working_dir.join(CTX_HOME_DIR).join(GLOBAL_VAULT_DIR);
        fs::create_dir_all(&scan_root).expect("scan root should be created");
        fs::create_dir_all(&local_vault).expect("local vault should be created");
        fs::write(scan_root.join("AGENTS.md"), "# Stable")
            .expect("source context should be writable");
        fs::write(
            vault_settings_path(&local_vault),
            r#"{"scan_roots":[{"path":"../configured-contexts","scope":"local"}]}"#,
        )
        .expect("local settings should be writable");

        let first_scan = discover_existing_context_file_results(&working_dir)
            .expect("first scan should discover source context");
        let second_scan = discover_existing_context_file_results(&working_dir)
            .expect("second scan should discover the same source context");
        let first_import = materialize_discovered_context_files(&working_dir)
            .expect("first import should materialize source context");
        let second_import = materialize_discovered_context_files(&working_dir)
            .expect("second import should reuse materialized source context");
        let contexts_dir = managed_contexts_dir(&local_vault);
        let metadata = load_import_metadata_index(&local_vault)
            .expect("import metadata should remain readable");

        assert_eq!(first_scan.len(), 1);
        assert_eq!(second_scan.len(), 1);
        assert_eq!(first_scan[0].file_path, second_scan[0].file_path);
        assert_eq!(first_import.len(), 1);
        assert_eq!(second_import.len(), 1);
        assert_eq!(first_import[0].file_path, second_import[0].file_path);
        assert_eq!(
            fs::read_dir(&contexts_dir)
                .expect("contexts dir should be readable")
                .filter_map(Result::ok)
                .filter(|entry| entry.path().extension().and_then(|ext| ext.to_str()) == Some("md"))
                .count(),
            1
        );
        assert_eq!(metadata.imports.len(), 1);

        fs::remove_dir_all(base).ok();
    }

    #[test]
    fn moved_import_source_reuses_existing_materialized_entry() {
        let base = std::env::temp_dir().join(format!(
            "ctx-moved-import-source-no-duplicate-{}",
            Uuid::new_v4()
        ));
        let working_dir = base.join("project");
        let scan_root = base.join("configured-contexts");
        let original_dir = scan_root.join("original");
        let moved_dir = scan_root.join("moved");
        let local_vault = working_dir.join(CTX_HOME_DIR).join(GLOBAL_VAULT_DIR);
        fs::create_dir_all(&original_dir).expect("original scan dir should be created");
        fs::create_dir_all(&local_vault).expect("local vault should be created");
        fs::write(original_dir.join("AGENTS.md"), "# Portable")
            .expect("original source context should be writable");
        fs::write(
            vault_settings_path(&local_vault),
            r#"{"scan_roots":[{"path":"../configured-contexts","scope":"local"}]}"#,
        )
        .expect("local settings should be writable");

        let first_import = materialize_discovered_context_files(&working_dir)
            .expect("first import should materialize original source");
        fs::create_dir_all(&moved_dir).expect("moved scan dir should be created");
        fs::rename(original_dir.join("AGENTS.md"), moved_dir.join("AGENTS.md"))
            .expect("source context should move");
        fs::write(
            moved_dir.join("AGENTS.md"),
            "# Portable\n\nUpdated after move.",
        )
        .expect("moved source context should be writable");
        fs::remove_dir_all(&original_dir).expect("old scan dir should be removed");
        let moved_import = materialize_discovered_context_files(&working_dir)
            .expect("moved source import should reuse existing materialized context");
        let contexts_dir = managed_contexts_dir(&local_vault);
        let metadata = load_import_metadata_index(&local_vault)
            .expect("import metadata should remain readable");

        assert_eq!(first_import.len(), 1);
        assert_eq!(moved_import.len(), 1);
        assert_eq!(first_import[0].file_path, contexts_dir.join("AGENTS.md"));
        assert_eq!(moved_import[0].file_path, contexts_dir.join("AGENTS.md"));
        assert_eq!(
            moved_import[0].import_source,
            Some(moved_dir.join("AGENTS.md").canonicalize().unwrap())
        );
        assert_eq!(
            fs::read_to_string(contexts_dir.join("AGENTS.md"))
                .expect("materialized context should be readable"),
            "# Portable\n\nUpdated after move."
        );
        assert_eq!(
            fs::read_dir(&contexts_dir)
                .expect("contexts dir should be readable")
                .filter_map(Result::ok)
                .filter(|entry| entry.path().extension().and_then(|ext| ext.to_str()) == Some("md"))
                .count(),
            1
        );
        assert_eq!(metadata.imports.len(), 1);
        assert_eq!(
            normalize_import_source_path(&metadata.imports[0].import_source),
            moved_dir.join("AGENTS.md").canonicalize().unwrap()
        );

        fs::remove_dir_all(base).ok();
    }

    #[test]
    fn conflicting_scan_results_remain_distinct_without_extra_entries_on_reimport() {
        let base = std::env::temp_dir().join(format!(
            "ctx-conflicting-scan-no-extra-entries-{}",
            Uuid::new_v4()
        ));
        let working_dir = base.join("project");
        let first_root = base.join("first");
        let second_root = base.join("second");
        let local_vault = working_dir.join(CTX_HOME_DIR).join(GLOBAL_VAULT_DIR);
        fs::create_dir_all(&first_root).expect("first scan root should be created");
        fs::create_dir_all(&second_root).expect("second scan root should be created");
        fs::create_dir_all(&local_vault).expect("local vault should be created");
        fs::write(first_root.join("AGENTS.md"), "# First").expect("first context should write");
        fs::write(second_root.join("AGENTS.md"), "# Second").expect("second context should write");
        fs::write(
            vault_settings_path(&local_vault),
            r#"{"scan_roots":[{"path":"../first","scope":"local"},{"path":"../second","scope":"local"}]}"#,
        )
        .expect("local settings should be writable");

        materialize_discovered_context_files(&working_dir)
            .expect("first import should materialize conflicting scan results");
        let reimport = materialize_discovered_context_files(&working_dir)
            .expect("reimport should reuse both conflicting materialized entries");
        let contexts_dir = managed_contexts_dir(&local_vault);
        let metadata = load_import_metadata_index(&local_vault)
            .expect("import metadata should remain readable");

        assert_eq!(reimport.len(), 2);
        assert!(contexts_dir.join("AGENTS.md").is_file());
        assert!(contexts_dir.join("AGENTS-2.md").is_file());
        assert_eq!(
            fs::read_dir(&contexts_dir)
                .expect("contexts dir should be readable")
                .filter_map(Result::ok)
                .filter(|entry| entry.path().extension().and_then(|ext| ext.to_str()) == Some("md"))
                .count(),
            2
        );
        assert_eq!(metadata.imports.len(), 2);

        fs::remove_dir_all(base).ok();
    }

    #[test]
    fn materialized_import_keeps_distinct_sources_with_same_name_stable_after_changes() {
        let base = std::env::temp_dir().join(format!(
            "ctx-materialize-stable-distinct-sources-{}",
            Uuid::new_v4()
        ));
        let working_dir = base.join("project");
        let first_root = base.join("first");
        let second_root = base.join("second");
        let local_vault = working_dir.join(CTX_HOME_DIR).join(GLOBAL_VAULT_DIR);
        fs::create_dir_all(&first_root).expect("first scan root should be created");
        fs::create_dir_all(&second_root).expect("second scan root should be created");
        fs::create_dir_all(&local_vault).expect("local vault should be created");
        fs::write(first_root.join("AGENTS.md"), "# First").expect("first context should write");
        fs::write(second_root.join("AGENTS.md"), "# Second").expect("second context should write");
        fs::write(
            vault_settings_path(&local_vault),
            r#"{"scan_roots":[{"path":"../first","scope":"local"},{"path":"../second","scope":"local"}]}"#,
        )
        .expect("local settings should be writable");

        materialize_discovered_context_files(&working_dir)
            .expect("first import should materialize both sources");
        fs::write(second_root.join("AGENTS.md"), "# Second changed")
            .expect("second context update should write");
        let second_import = materialize_discovered_context_files(&working_dir)
            .expect("second import should update the existing second target");
        let contexts_dir = managed_contexts_dir(&local_vault);

        assert_eq!(second_import.len(), 2);
        assert_eq!(
            fs::read_to_string(contexts_dir.join("AGENTS.md"))
                .expect("first materialized context should be readable"),
            "# First"
        );
        assert_eq!(
            fs::read_to_string(contexts_dir.join("AGENTS-2.md"))
                .expect("second materialized context should be readable"),
            "# Second changed"
        );
        assert_eq!(
            fs::read_dir(&contexts_dir)
                .expect("contexts dir should be readable")
                .filter_map(Result::ok)
                .filter(|entry| entry.path().extension().and_then(|ext| ext.to_str()) == Some("md"))
                .count(),
            2
        );

        fs::remove_dir_all(base).ok();
    }

    #[test]
    fn default_discovery_matches_known_context_locations() {
        let base = std::env::temp_dir().join(format!(
            "ctx-discovery-known-context-locations-{}",
            Uuid::new_v4()
        ));
        fs::create_dir_all(base.join(".claude")).expect(".claude directory should be created");
        fs::create_dir_all(base.join(".codex")).expect(".codex directory should be created");
        fs::create_dir_all(base.join(".agents")).expect(".agents directory should be created");
        fs::write(base.join("agent.md"), "# Root agent").expect("root agent should be writable");
        fs::write(base.join(".claude").join("CLAUDE.md"), "# Claude")
            .expect("claude context should be writable");
        fs::write(base.join(".codex").join("AGENTS.md"), "# Codex")
            .expect("codex context should be writable");
        fs::write(base.join(".agents").join("agents.md"), "# Agents")
            .expect("agents context should be writable");

        let contexts =
            discover_existing_context_files(&base).expect("known context locations should scan");
        let discovered_paths = contexts
            .iter()
            .map(|context| context.file_path.clone())
            .collect::<HashSet<_>>();

        assert!(discovered_paths.contains(&base.join("agent.md")));
        assert!(discovered_paths.contains(&base.join(".claude").join("CLAUDE.md")));
        assert!(discovered_paths.contains(&base.join(".codex").join("AGENTS.md")));
        assert!(discovered_paths.contains(&base.join(".agents").join("agents.md")));

        fs::remove_dir_all(base).ok();
    }

    #[test]
    fn default_discovery_ignores_non_context_files() {
        let base = std::env::temp_dir().join(format!(
            "ctx-discovery-ignored-non-contexts-{}",
            Uuid::new_v4()
        ));
        fs::create_dir_all(base.join("docs")).expect("docs directory should be created");
        fs::create_dir_all(base.join("skills")).expect("skills directory should be created");
        fs::create_dir_all(base.join("agents")).expect("agents directory should be created");
        fs::write(base.join("AGENTS.md"), "# Codex").expect("context file should be writable");
        fs::write(base.join("README.md"), "# Project").expect("root readme should be writable");
        fs::write(base.join("docs").join("claude.md"), "# Nested docs")
            .expect("nested docs context-like file should be writable");
        fs::write(base.join("skills").join("notes.txt"), "not markdown")
            .expect("skill text file should be writable");
        fs::write(base.join("agents").join("notes.txt"), "not markdown")
            .expect("agent text file should be writable");

        let contexts =
            discover_existing_context_files(&base).expect("ignored files should not fail scanning");
        let discovered_paths = contexts
            .iter()
            .map(|context| context.file_path.clone())
            .collect::<HashSet<_>>();

        assert!(discovered_paths.contains(&base.join("AGENTS.md")));
        assert!(!discovered_paths.contains(&base.join("README.md")));
        assert!(!discovered_paths.contains(&base.join("docs").join("claude.md")));
        assert!(!discovered_paths.contains(&base.join("skills").join("notes.txt")));
        assert!(!discovered_paths.contains(&base.join("agents").join("notes.txt")));

        fs::remove_dir_all(base).ok();
    }

    #[test]
    fn configured_scan_roots_drive_existing_context_discovery() {
        let base =
            std::env::temp_dir().join(format!("ctx-configured-scan-roots-{}", Uuid::new_v4()));
        let working_dir = base.join("project");
        let local_vault = working_dir.join(".ctx").join("vault");
        let configured_scan_root = base.join("configured-contexts");
        fs::create_dir_all(&local_vault).expect("local vault should be created");
        fs::create_dir_all(&configured_scan_root).expect("configured scan root should be created");
        fs::write(working_dir.join("agent.md"), "# Default root")
            .expect("default root file should be writable");
        fs::write(configured_scan_root.join("CLAUDE.md"), "# Configured root")
            .expect("configured root file should be writable");
        fs::write(
            vault_settings_path(&local_vault),
            r#"{"scan_roots":[{"path":"../configured-contexts","scope":"local"}]}"#,
        )
        .expect("local settings should be writable");

        let contexts =
            discover_existing_context_files(&working_dir).expect("configured roots should scan");
        let configured_file = configured_scan_root
            .join("CLAUDE.md")
            .canonicalize()
            .expect("configured context file should canonicalize");

        assert!(contexts
            .iter()
            .any(|context| context.file_path == configured_file
                && context.vault_scope == VaultScope::Local));
        assert!(!contexts
            .iter()
            .any(|context| context.file_path == working_dir.join("agent.md")));

        fs::remove_dir_all(base).ok();
    }

    #[test]
    fn configured_scan_roots_scan_multiple_global_and_local_roots() {
        let base = std::env::temp_dir().join(format!(
            "ctx-configured-multiple-scan-roots-{}",
            Uuid::new_v4()
        ));
        let home = base.join("home");
        let working_dir = base.join("project");
        let global_vault = home.join(".ctx").join("vault");
        let local_vault = working_dir.join(".ctx").join("vault");
        let global_scan_root = home.join("global-contexts");
        let local_scan_root = working_dir.join("project-contexts");

        fs::create_dir_all(&global_vault).expect("global vault should be created");
        fs::create_dir_all(&local_vault).expect("local vault should be created");
        fs::create_dir_all(&global_scan_root).expect("global scan root should be created");
        fs::create_dir_all(&local_scan_root).expect("local scan root should be created");
        fs::write(global_scan_root.join("CLAUDE.md"), "# Global Claude")
            .expect("global context should be writable");
        fs::write(local_scan_root.join("AGENTS.md"), "# Local Codex")
            .expect("local context should be writable");
        fs::write(
            vault_settings_path(&global_vault),
            r#"{"scan_roots":["global-contexts"]}"#,
        )
        .expect("global settings should be writable");
        fs::write(
            vault_settings_path(&local_vault),
            r#"{"scan_roots":[{"path":"project-contexts","scope":"local"}]}"#,
        )
        .expect("local settings should be writable");

        with_home(&home, || {
            let contexts = discover_existing_context_files(&working_dir)
                .expect("multiple configured roots should scan");

            assert!(contexts.iter().any(|context| {
                context.file_path
                    == global_scan_root
                        .join("CLAUDE.md")
                        .canonicalize()
                        .expect("global context should canonicalize")
                    && context.vault_scope == VaultScope::Global
            }));
            assert!(contexts.iter().any(|context| {
                context.file_path
                    == local_scan_root
                        .join("AGENTS.md")
                        .canonicalize()
                        .expect("local context should canonicalize")
                    && context.vault_scope == VaultScope::Local
            }));
        });

        fs::remove_dir_all(base).ok();
    }

    #[test]
    fn configured_scan_roots_recursively_discover_known_context_filenames() {
        let base = std::env::temp_dir().join(format!(
            "ctx-configured-recursive-scan-roots-{}",
            Uuid::new_v4()
        ));
        let working_dir = base.join("project");
        let local_vault = working_dir.join(".ctx").join("vault");
        let configured_scan_root = base.join("configured-contexts");
        fs::create_dir_all(&local_vault).expect("local vault should be created");
        fs::create_dir_all(configured_scan_root.join("teams").join("review"))
            .expect("nested review directory should be created");
        fs::create_dir_all(configured_scan_root.join("tools").join("codex"))
            .expect("nested codex directory should be created");
        fs::create_dir_all(configured_scan_root.join("docs").join("claude"))
            .expect("nested claude directory should be created");
        fs::write(
            configured_scan_root
                .join("teams")
                .join("review")
                .join("agent.md"),
            "# Review agent",
        )
        .expect("nested agent.md should be writable");
        fs::write(
            configured_scan_root
                .join("tools")
                .join("codex")
                .join("AGENTS.md"),
            "# Codex instructions",
        )
        .expect("nested AGENTS.md should be writable");
        fs::write(
            configured_scan_root
                .join("docs")
                .join("claude")
                .join("claude.md"),
            "# Claude instructions",
        )
        .expect("nested claude.md should be writable");
        fs::write(
            configured_scan_root.join("docs").join("notes.md"),
            "# Notes",
        )
        .expect("non-known markdown should be writable");
        fs::write(
            vault_settings_path(&local_vault),
            r#"{"scan_roots":[{"path":"../configured-contexts","scope":"local"}]}"#,
        )
        .expect("local settings should be writable");

        let contexts =
            discover_existing_context_files(&working_dir).expect("configured roots should scan");
        let discovered_paths = contexts
            .iter()
            .map(|context| context.file_path.clone())
            .collect::<HashSet<_>>();

        assert!(discovered_paths.contains(
            &configured_scan_root
                .join("teams")
                .join("review")
                .join("agent.md")
                .canonicalize()
                .expect("nested agent.md should canonicalize")
        ));
        assert!(discovered_paths.contains(
            &configured_scan_root
                .join("tools")
                .join("codex")
                .join("AGENTS.md")
                .canonicalize()
                .expect("nested AGENTS.md should canonicalize")
        ));
        assert!(discovered_paths.contains(
            &configured_scan_root
                .join("docs")
                .join("claude")
                .join("claude.md")
                .canonicalize()
                .expect("nested claude.md should canonicalize")
        ));
        assert!(!discovered_paths.contains(
            &configured_scan_root
                .join("docs")
                .join("notes.md")
                .canonicalize()
                .expect("non-known markdown should canonicalize")
        ));

        fs::remove_dir_all(base).ok();
    }

    #[test]
    fn configured_scan_roots_skip_excluded_directories_during_recursive_discovery() {
        let base = std::env::temp_dir().join(format!(
            "ctx-configured-recursive-exclusions-{}",
            Uuid::new_v4()
        ));
        let working_dir = base.join("project");
        let local_vault = working_dir.join(".ctx").join("vault");
        let configured_scan_root = base.join("configured-contexts");

        fs::create_dir_all(&local_vault).expect("local vault should be created");
        fs::create_dir_all(configured_scan_root.join("team"))
            .expect("valid nested directory should be created");
        fs::create_dir_all(configured_scan_root.join("node_modules").join("pkg"))
            .expect("excluded node_modules directory should be created");
        fs::create_dir_all(configured_scan_root.join(".git").join("hooks"))
            .expect("excluded git directory should be created");
        fs::create_dir_all(configured_scan_root.join("target").join("debug"))
            .expect("excluded target directory should be created");
        fs::create_dir_all(
            configured_scan_root
                .join(".ctx")
                .join("vault")
                .join("contexts"),
        )
        .expect("excluded managed vault directory should be created");

        fs::write(
            configured_scan_root.join("team").join("AGENTS.md"),
            "# Valid",
        )
        .expect("valid nested context should be writable");
        fs::write(
            configured_scan_root
                .join("node_modules")
                .join("pkg")
                .join("AGENTS.md"),
            "# Dependency instructions",
        )
        .expect("excluded dependency context should be writable");
        fs::write(
            configured_scan_root
                .join(".git")
                .join("hooks")
                .join("agent.md"),
            "# Git hook instructions",
        )
        .expect("excluded git context should be writable");
        fs::write(
            configured_scan_root
                .join("target")
                .join("debug")
                .join("CLAUDE.md"),
            "# Build output instructions",
        )
        .expect("excluded build context should be writable");
        fs::write(
            configured_scan_root
                .join(".ctx")
                .join("vault")
                .join("contexts")
                .join("agent.md"),
            "# Managed copy",
        )
        .expect("excluded managed context should be writable");
        fs::write(
            vault_settings_path(&local_vault),
            r#"{"scan_roots":[{"path":"../configured-contexts","scope":"local"}]}"#,
        )
        .expect("local settings should be writable");

        let contexts =
            discover_existing_context_files(&working_dir).expect("configured roots should scan");
        let discovered_paths = contexts
            .iter()
            .map(|context| context.file_path.clone())
            .collect::<HashSet<_>>();

        assert!(discovered_paths.contains(
            &configured_scan_root
                .join("team")
                .join("AGENTS.md")
                .canonicalize()
                .expect("valid nested context should canonicalize")
        ));
        for excluded in [
            configured_scan_root
                .join("node_modules")
                .join("pkg")
                .join("AGENTS.md"),
            configured_scan_root
                .join(".git")
                .join("hooks")
                .join("agent.md"),
            configured_scan_root
                .join("target")
                .join("debug")
                .join("CLAUDE.md"),
            configured_scan_root
                .join(".ctx")
                .join("vault")
                .join("contexts")
                .join("agent.md"),
        ] {
            assert!(!discovered_paths.contains(
                &excluded
                    .canonicalize()
                    .expect("excluded context should canonicalize")
            ));
        }

        fs::remove_dir_all(base).ok();
    }

    #[test]
    fn configured_scan_roots_discover_nested_skill_directories_and_skill_files() {
        let base = std::env::temp_dir().join(format!(
            "ctx-configured-scan-roots-skills-{}",
            Uuid::new_v4()
        ));
        let working_dir = base.join("project");
        let local_vault = working_dir.join(".ctx").join("vault");
        let configured_scan_root = base.join("configured-contexts");
        fs::create_dir_all(&local_vault).expect("local vault should be created");
        fs::create_dir_all(
            configured_scan_root
                .join("teams")
                .join("platform")
                .join("skills")
                .join("review"),
        )
        .expect("nested skill directory should be created");
        fs::create_dir_all(configured_scan_root.join("standalone-skill"))
            .expect("standalone skill directory should be created");
        fs::write(
            configured_scan_root
                .join("teams")
                .join("platform")
                .join("skills")
                .join("review")
                .join("reviewer-agent.md"),
            "# Review skill",
        )
        .expect("nested skill markdown should be writable");
        fs::write(
            configured_scan_root
                .join("standalone-skill")
                .join("SKILL.md"),
            "# Standalone skill",
        )
        .expect("standalone SKILL.md should be writable");
        fs::write(configured_scan_root.join("notes.md"), "# General note")
            .expect("general markdown should be writable");
        fs::write(
            vault_settings_path(&local_vault),
            r#"{"scan_roots":[{"path":"../configured-contexts","scope":"local"}]}"#,
        )
        .expect("local settings should be writable");

        let contexts =
            discover_existing_context_files(&working_dir).expect("configured roots should scan");
        let skill_dir_path = configured_scan_root
            .join("teams")
            .join("platform")
            .join("skills")
            .join("review")
            .join("reviewer-agent.md")
            .canonicalize()
            .expect("nested skill markdown should canonicalize");
        let skill_file_path = configured_scan_root
            .join("standalone-skill")
            .join("SKILL.md")
            .canonicalize()
            .expect("standalone SKILL.md should canonicalize");

        for skill_path in [skill_dir_path, skill_file_path] {
            let skill = contexts
                .iter()
                .find(|context| context.file_path == skill_path)
                .expect("configured scan root skill should be discovered");

            assert_eq!(skill.classification, Classification::Shared);
            assert_eq!(skill.vault_scope, VaultScope::Local);
            assert!(skill.tags.contains(&"skills".to_string()));
        }
        assert!(!contexts.iter().any(|context| {
            context.file_path
                == configured_scan_root
                    .join("notes.md")
                    .canonicalize()
                    .expect("general markdown should canonicalize")
        }));

        fs::remove_dir_all(base).ok();
    }

    #[test]
    fn configured_skill_scan_roots_discover_markdown_as_shared_skills() {
        let base = std::env::temp_dir().join(format!(
            "ctx-configured-skill-scan-roots-{}",
            Uuid::new_v4()
        ));
        let working_dir = base.join("project");
        let local_vault = working_dir.join(".ctx").join("vault");
        let skill_scan_root = base.join("shared-prompts");
        fs::create_dir_all(&local_vault).expect("local vault should be created");
        fs::create_dir_all(skill_scan_root.join("review"))
            .expect("nested skill directory should be created");
        fs::write(
            skill_scan_root.join("review").join("reviewer-agent.md"),
            "# Review skill",
        )
        .expect("configured skill markdown should be writable");
        fs::write(skill_scan_root.join("notes.txt"), "not markdown")
            .expect("non-markdown file should be writable");
        fs::write(
            vault_settings_path(&local_vault),
            r#"{"skill_scan_roots":[{"path":"../shared-prompts","scope":"local"}]}"#,
        )
        .expect("local settings should be writable");

        let contexts =
            discover_existing_context_files(&working_dir).expect("configured skill roots scan");
        let skill_path = skill_scan_root
            .join("review")
            .join("reviewer-agent.md")
            .canonicalize()
            .expect("configured skill file should canonicalize");
        let skill = contexts
            .iter()
            .find(|context| context.file_path == skill_path)
            .expect("configured skill markdown should be discovered");

        assert_eq!(skill.classification, Classification::Shared);
        assert_eq!(skill.vault_scope, VaultScope::Local);
        assert!(skill.tags.contains(&"skills".to_string()));
        assert!(!contexts
            .iter()
            .any(|context| context.file_path == skill_scan_root.join("notes.txt")));

        fs::remove_dir_all(base).ok();
    }

    #[test]
    fn configured_skill_scan_roots_force_agent_named_markdown_to_shared_skill() {
        let base = std::env::temp_dir().join(format!(
            "ctx-configured-skill-agent-name-{}",
            Uuid::new_v4()
        ));
        let working_dir = base.join("project");
        let local_vault = working_dir.join(".ctx").join("vault");
        let skill_scan_root = base.join("shared-prompts");
        fs::create_dir_all(&local_vault).expect("local vault should be created");
        fs::create_dir_all(&skill_scan_root).expect("skill scan root should be created");
        fs::write(skill_scan_root.join("reviewer-agent.MD"), "# Review skill")
            .expect("uppercase markdown skill should be writable");
        fs::write(
            vault_settings_path(&local_vault),
            r#"{"skill_scan_roots":[{"path":"../shared-prompts","scope":"local"}]}"#,
        )
        .expect("local settings should be writable");

        let results = discover_existing_context_file_results(&working_dir)
            .expect("configured skill roots should scan");
        let skill_path = skill_scan_root
            .join("reviewer-agent.MD")
            .canonicalize()
            .expect("configured skill file should canonicalize");
        let skill = results
            .iter()
            .find(|result| result.file_path == skill_path)
            .expect("agent-named configured skill markdown should be discovered");

        assert_eq!(skill.metadata.classification, Classification::Shared);
        assert_eq!(
            skill.metadata.inferred_classification,
            Some(Classification::Shared)
        );
        assert_eq!(
            skill.metadata.llm_classification_status,
            ClassificationStatus::Classified
        );
        assert!(skill.metadata.tags.contains(&"skills".to_string()));

        fs::remove_dir_all(base).ok();
    }

    #[test]
    fn overlapping_context_and_skill_scan_roots_deduplicate_skill_paths() {
        let base = std::env::temp_dir().join(format!(
            "ctx-overlapping-skill-scan-roots-{}",
            Uuid::new_v4()
        ));
        let working_dir = base.join("project");
        let local_vault = working_dir.join(".ctx").join("vault");
        let configured_scan_root = base.join("configured-contexts");
        let nested_skill_dir = configured_scan_root
            .join("teams")
            .join("platform")
            .join("skills");
        fs::create_dir_all(&local_vault).expect("local vault should be created");
        fs::create_dir_all(&nested_skill_dir).expect("nested skill directory should be created");
        fs::write(nested_skill_dir.join("review.md"), "# Review skill")
            .expect("skill markdown should be writable");
        fs::write(
            vault_settings_path(&local_vault),
            r#"{
  "scan_roots":[{"path":"../configured-contexts","scope":"local"}],
  "skill_scan_roots":[{"path":"../configured-contexts/teams/platform/skills","scope":"local"}]
}"#,
        )
        .expect("local settings should be writable");

        let results = discover_existing_context_file_results(&working_dir)
            .expect("overlapping context and skill roots should scan");
        let skill_path = nested_skill_dir
            .join("review.md")
            .canonicalize()
            .expect("skill markdown should canonicalize");
        let matching_results = results
            .iter()
            .filter(|result| result.file_path == skill_path)
            .collect::<Vec<_>>();

        assert_eq!(matching_results.len(), 1);
        assert_eq!(
            matching_results[0].metadata.classification,
            Classification::Shared
        );
        assert!(matching_results[0]
            .metadata
            .tags
            .contains(&"skills".to_string()));

        fs::remove_dir_all(base).ok();
    }

    #[test]
    fn configured_roots_scan_multiple_sources_and_mixed_skill_layouts() {
        let base = std::env::temp_dir().join(format!(
            "ctx-multiple-roots-mixed-skills-{}",
            Uuid::new_v4()
        ));
        let home = base.join("home");
        let working_dir = base.join("project");
        let global_vault = home.join(".ctx").join("vault");
        let local_vault = working_dir.join(".ctx").join("vault");
        let global_scan_root = home.join("global-contexts");
        let local_scan_root = working_dir.join("project-contexts");
        let shared_skill_root = base.join("shared-skills");

        fs::create_dir_all(&global_vault).expect("global vault should be created");
        fs::create_dir_all(&local_vault).expect("local vault should be created");
        fs::create_dir_all(global_scan_root.join("teams").join("skills").join("review"))
            .expect("global nested skills directory should be created");
        fs::create_dir_all(local_scan_root.join("bundle"))
            .expect("local standalone skill directory should be created");
        fs::create_dir_all(shared_skill_root.join("ops"))
            .expect("configured skill root directory should be created");

        fs::write(global_scan_root.join("CLAUDE.md"), "# Global Claude")
            .expect("global named context should be writable");
        fs::write(
            global_scan_root
                .join("teams")
                .join("skills")
                .join("review")
                .join("review.md"),
            "# Review skill",
        )
        .expect("global nested skill should be writable");
        fs::write(
            local_scan_root.join("bundle").join("SKILL.md"),
            "# Project bundle skill",
        )
        .expect("local standalone SKILL.md should be writable");
        fs::write(
            shared_skill_root.join("ops").join("runbook.md"),
            "# Ops skill",
        )
        .expect("configured skill root markdown should be writable");
        fs::write(shared_skill_root.join("notes.txt"), "not markdown")
            .expect("non-markdown skill root file should be writable");

        fs::write(
            vault_settings_path(&global_vault),
            r#"{"scan_roots":["global-contexts"]}"#,
        )
        .expect("global settings should be writable");
        fs::write(
            vault_settings_path(&local_vault),
            r#"{
  "scan_roots":[{"path":"project-contexts","scope":"local"}],
  "skill_scan_roots":[{"path":"../shared-skills","scope":"local"}]
}"#,
        )
        .expect("local settings should be writable");

        with_home(&home, || {
            let contexts = discover_existing_context_files(&working_dir)
                .expect("multiple configured roots should scan");
            let discovered_paths = contexts
                .iter()
                .map(|context| context.file_path.clone())
                .collect::<HashSet<_>>();

            assert!(discovered_paths.contains(
                &global_scan_root
                    .join("CLAUDE.md")
                    .canonicalize()
                    .expect("global context should canonicalize")
            ));
            assert!(discovered_paths.contains(
                &global_scan_root
                    .join("teams")
                    .join("skills")
                    .join("review")
                    .join("review.md")
                    .canonicalize()
                    .expect("global skill should canonicalize")
            ));
            assert!(discovered_paths.contains(
                &local_scan_root
                    .join("bundle")
                    .join("SKILL.md")
                    .canonicalize()
                    .expect("local standalone skill should canonicalize")
            ));
            assert!(discovered_paths.contains(
                &shared_skill_root
                    .join("ops")
                    .join("runbook.md")
                    .canonicalize()
                    .expect("configured skill root file should canonicalize")
            ));
            assert!(!discovered_paths.contains(
                &shared_skill_root
                    .join("notes.txt")
                    .canonicalize()
                    .expect("non-markdown skill root file should canonicalize")
            ));

            for skill_path in [
                global_scan_root
                    .join("teams")
                    .join("skills")
                    .join("review")
                    .join("review.md")
                    .canonicalize()
                    .expect("global skill should canonicalize"),
                local_scan_root
                    .join("bundle")
                    .join("SKILL.md")
                    .canonicalize()
                    .expect("local standalone skill should canonicalize"),
                shared_skill_root
                    .join("ops")
                    .join("runbook.md")
                    .canonicalize()
                    .expect("configured skill root file should canonicalize"),
            ] {
                let skill = contexts
                    .iter()
                    .find(|context| context.file_path == skill_path)
                    .expect("skill should be discovered");

                assert_eq!(skill.classification, Classification::Shared);
                assert!(skill.tags.contains(&"skills".to_string()));
            }
        });

        fs::remove_dir_all(base).ok();
    }

    #[test]
    fn duplicate_configured_roots_and_overlapping_skill_roots_are_deduplicated() {
        let base =
            std::env::temp_dir().join(format!("ctx-duplicate-configured-roots-{}", Uuid::new_v4()));
        let working_dir = base.join("project");
        let local_vault = working_dir.join(".ctx").join("vault");
        let scan_root = base.join("configured-contexts");
        let skill_dir = scan_root.join("teams").join("skills");

        fs::create_dir_all(&local_vault).expect("local vault should be created");
        fs::create_dir_all(&skill_dir).expect("skill directory should be created");
        fs::write(scan_root.join("AGENTS.md"), "# Codex")
            .expect("named context should be writable");
        fs::write(skill_dir.join("review.md"), "# Review skill")
            .expect("skill context should be writable");
        fs::write(
            vault_settings_path(&local_vault),
            r#"{
  "scan_roots":[
    {"path":"../configured-contexts","scope":"local"},
    {"path":"../configured-contexts","scope":"local"}
  ],
  "skill_scan_roots":[
    {"path":"../configured-contexts/teams/skills","scope":"local"},
    {"path":"../configured-contexts/teams/skills","scope":"local"}
  ]
}"#,
        )
        .expect("local settings should be writable");

        let results = discover_existing_context_file_results(&working_dir)
            .expect("duplicate configured roots should scan once");
        let named_context = scan_root
            .join("AGENTS.md")
            .canonicalize()
            .expect("named context should canonicalize");
        let skill_context = skill_dir
            .join("review.md")
            .canonicalize()
            .expect("skill context should canonicalize");

        assert_eq!(
            results
                .iter()
                .filter(|result| result.file_path == named_context)
                .count(),
            1
        );
        assert_eq!(
            results
                .iter()
                .filter(|result| result.file_path == skill_context)
                .count(),
            1
        );

        fs::remove_dir_all(base).ok();
    }

    #[test]
    fn missing_configured_skill_scan_root_blocks_discovery_with_clear_error() {
        let base = std::env::temp_dir().join(format!(
            "ctx-missing-configured-skill-root-{}",
            Uuid::new_v4()
        ));
        let working_dir = base.join("project");
        let local_vault = working_dir.join(".ctx").join("vault");
        fs::create_dir_all(&local_vault).expect("local vault should be created");
        fs::write(
            vault_settings_path(&local_vault),
            r#"{"skill_scan_roots":[{"path":"../missing-skills","scope":"local"}]}"#,
        )
        .expect("local settings should be writable");

        let error = discover_existing_context_files(&working_dir)
            .expect_err("missing configured skill root should fail discovery")
            .to_string();

        assert!(error.contains("failed to load configured skill scan roots"));
        assert!(error.contains("configured skill scan root"));
        assert!(error.contains("missing-skills"));

        fs::remove_dir_all(base).ok();
    }

    #[cfg(unix)]
    #[test]
    fn inaccessible_configured_scan_root_reports_clear_error() {
        use std::os::unix::fs::PermissionsExt;

        let base = std::env::temp_dir().join(format!(
            "ctx-inaccessible-configured-scan-root-{}",
            Uuid::new_v4()
        ));
        let working_dir = base.join("project");
        let local_vault = working_dir.join(".ctx").join("vault");
        let scan_root = base.join("inaccessible-contexts");
        fs::create_dir_all(&local_vault).expect("local vault should be created");
        fs::create_dir_all(&scan_root).expect("scan root should be created");
        fs::write(
            vault_settings_path(&local_vault),
            r#"{"scan_roots":[{"path":"../inaccessible-contexts","scope":"local"}]}"#,
        )
        .expect("local settings should be writable");

        let original_permissions = fs::metadata(&scan_root)
            .expect("scan root metadata should be readable")
            .permissions();
        fs::set_permissions(&scan_root, fs::Permissions::from_mode(0o000))
            .expect("scan root should be made inaccessible");

        let result = discover_existing_context_files(&working_dir);

        fs::set_permissions(&scan_root, original_permissions)
            .expect("scan root permissions should be restored");

        let error = result
            .expect_err("inaccessible configured scan root should fail discovery")
            .to_string();

        assert!(error.contains("failed to read configured scan root"));
        assert!(error.contains("inaccessible-contexts"));

        fs::remove_dir_all(base).ok();
    }

    #[test]
    fn invalid_configured_scan_root_blocks_discovery_with_clear_error() {
        let base = std::env::temp_dir().join(format!(
            "ctx-invalid-configured-scan-root-{}",
            Uuid::new_v4()
        ));
        let working_dir = base.join("project");
        let local_vault = working_dir.join(".ctx").join("vault");
        fs::create_dir_all(&local_vault).expect("local vault should be created");
        fs::write(
            vault_settings_path(&local_vault),
            r#"{"scan_roots":["missing-contexts"]}"#,
        )
        .expect("local settings should be writable");

        let error = discover_existing_context_files(&working_dir)
            .expect_err("invalid configured scan root should fail discovery")
            .to_string();

        assert!(error.contains("failed to load configured scan roots"));
        assert!(error.contains("configured scan root"));

        fs::remove_dir_all(base).ok();
    }

    #[test]
    fn discovered_contexts_exclude_managed_vault_files() {
        let (roots, base) = test_roots();
        let working_dir = base.join("project");
        fs::create_dir_all(&working_dir).expect("working directory should be created");
        create_context_file(&roots, VaultScope::Local, "", "agent.md", "# Managed")
            .expect("managed context should be created");
        fs::write(working_dir.join("agent.md"), "# Existing")
            .expect("existing context should be writable");

        let contexts = list_context_files_with_discovered(&working_dir)
            .expect("vault and discovered context files should be listed");

        assert!(contexts.iter().any(|context| {
            context.file_path
                == managed_contexts_dir(roots.local_root.as_ref().unwrap()).join("agent.md")
                && context.import_source.is_none()
        }));
        assert!(contexts.iter().any(|context| {
            context.file_path == working_dir.join("agent.md") && context.import_source.is_some()
        }));

        fs::remove_dir_all(base).ok();
    }

    #[test]
    fn reindex_discovers_markdown_and_replaces_stale_sqlite_rows_on_filesystem() {
        let base = std::env::temp_dir().join(format!("ctx-reindex-filesystem-{}", Uuid::new_v4()));
        let home = base.join("home");
        let working_dir = base.join("project");
        let roots = VaultRoots {
            global_root: home.join(CTX_HOME_DIR).join(GLOBAL_VAULT_DIR),
            local_root: Some(working_dir.join(CTX_HOME_DIR).join(GLOBAL_VAULT_DIR)),
        };
        fs::create_dir_all(&working_dir).expect("working directory should be created");
        fs::create_dir_all(&home).expect("home directory should be created");

        let stale = create_context_file(
            &roots,
            VaultScope::Local,
            "archive",
            "stale.md",
            "# Stale\n\nThis row should disappear after full reindex.",
        )
        .expect("stale managed context should be created");

        with_home(&home, || {
            let first_report =
                reindex_markdown_contexts(&working_dir).expect("initial reindex should pass");
            assert_eq!(
                first_report.local.as_ref().unwrap().indexed_markdown_files,
                1
            );
            assert_eq!(first_report.discovered_markdown_files, 0);

            fs::remove_file(&stale.file_path).expect("stale markdown should be removable");
            create_context_file(
                &roots,
                VaultScope::Global,
                "shared",
                "style-guide.md",
                "# Style Guide\n\nShared conventions.",
            )
            .expect("global managed context should be created");
            create_context_file(
                &roots,
                VaultScope::Local,
                "agents",
                "reviewer.md",
                "---\ntitle: Reviewer\ntags: [review, qa]\nclassification: subagent\n---\n# Reviewer\n\nUse [[AGENTS]].",
            )
            .expect("local managed context should be created");
            fs::write(
                working_dir.join("AGENTS.md"),
                "# AGENTS\n\nProject context for [[Reviewer]].",
            )
            .expect("local discovered AGENTS.md should be writable");
            fs::write(
                home.join("CLAUDE.md"),
                "# Claude\n\nGlobal discovered context.",
            )
            .expect("global discovered CLAUDE.md should be writable");

            let second_report =
                reindex_markdown_contexts(&working_dir).expect("second reindex should pass");

            assert_eq!(second_report.global.indexed_markdown_files, 2);
            assert_eq!(
                second_report.local.as_ref().unwrap().cleared_markdown_files,
                1
            );
            assert_eq!(
                second_report.local.as_ref().unwrap().indexed_markdown_files,
                2
            );
            assert_eq!(second_report.discovered_markdown_files, 2);
            assert!(second_report.local.as_ref().unwrap().indexed_tags >= 2);
            assert_eq!(second_report.local.as_ref().unwrap().indexed_links, 2);

            let local_connection =
                Connection::open(sqlite_index_path(roots.local_root.as_ref().unwrap()))
                    .expect("local sqlite index should open");
            let global_connection = Connection::open(sqlite_index_path(&roots.global_root))
                .expect("global sqlite index should open");

            assert_eq!(
                sqlite_table_count(&local_connection, MARKDOWN_FILES_TABLE_NAME),
                2
            );
            assert_eq!(
                sqlite_table_count(&global_connection, MARKDOWN_FILES_TABLE_NAME),
                2
            );
            assert_eq!(
                sqlite_table_count(&local_connection, MARKDOWN_FILE_FRONTMATTER_TABLE_NAME),
                2
            );
            assert!(sqlite_table_count(&local_connection, MARKDOWN_FILE_TAGS_TABLE_NAME) >= 2);
            assert_eq!(
                sqlite_table_count(&local_connection, MARKDOWN_FILE_LINKS_TABLE_NAME),
                2
            );
            assert_eq!(
                sqlite_table_count(&local_connection, MARKDOWN_FILE_BACKLINKS_VIEW_NAME),
                2
            );

            assert_eq!(
                sqlite_path_count(&local_connection, &stale.file_path),
                0,
                "full reindex should clear rows for deleted markdown files"
            );
            assert_eq!(
                markdown_file_classification(&local_connection, "agents/reviewer.md"),
                "subagent"
            );
            assert_eq!(
                markdown_file_title(&local_connection, "agents/reviewer.md"),
                "Reviewer"
            );
            assert_eq!(
                markdown_file_import_source_type(&local_connection, "AGENTS.md"),
                Some("codex-agents".to_string())
            );
            assert_eq!(
                markdown_file_import_source_type(&global_connection, "CLAUDE.md"),
                Some("claude-markdown".to_string())
            );
        });

        fs::remove_dir_all(base).ok();
    }

    #[test]
    fn incremental_sync_create_and_update_events_upsert_markdown_index_without_full_rescan() {
        let base = std::env::temp_dir().join(format!("ctx-incremental-sync-{}", Uuid::new_v4()));
        let home = base.join("home");
        let working_dir = base.join("project");
        let roots = VaultRoots {
            global_root: home.join(CTX_HOME_DIR).join(GLOBAL_VAULT_DIR),
            local_root: Some(working_dir.join(CTX_HOME_DIR).join(GLOBAL_VAULT_DIR)),
        };
        let local_contexts_dir = managed_contexts_dir(roots.local_root.as_ref().unwrap());
        fs::create_dir_all(&local_contexts_dir).expect("local contexts directory should exist");
        fs::create_dir_all(&working_dir).expect("working directory should exist");
        fs::create_dir_all(&home).expect("home directory should exist");

        let stable = create_context_file(
            &roots,
            VaultScope::Local,
            "stable",
            "stable.md",
            "# Stable\n\nThis row should remain indexed.",
        )
        .expect("stable context should be created");

        with_home(&home, || {
            reindex_markdown_contexts(&working_dir)
                .expect("initial full reindex should seed sqlite");

            let created_path = local_contexts_dir.join("agents").join("reviewer.md");
            fs::create_dir_all(created_path.parent().unwrap())
                .expect("created parent directory should exist");
            fs::write(
                &created_path,
                "---\ntitle: Reviewer\ntags: [review]\nclassification: subagent\n---\n# Reviewer\n\nUse [[Stable]].",
            )
            .expect("created markdown should be writable");

            let create_event = ContextFileChangeEvent {
                kind: ContextFileChangeKind::Create,
                vault_scope: VaultScope::Local,
                root_kind: ContextWatchRootKind::ManagedVault,
                root_path: local_contexts_dir.clone(),
                path: created_path.clone(),
                relative_path: PathBuf::from("agents/reviewer.md"),
                previous_path: None,
                previous_relative_path: None,
            };
            let create_report = sync_markdown_context_index_event(&working_dir, &create_event)
                .expect("create event should sync")
                .expect("create event should produce an index report");

            assert_eq!(create_report.indexed_markdown_files, 1);

            let connection =
                Connection::open(sqlite_index_path(roots.local_root.as_ref().unwrap()))
                    .expect("local sqlite index should open after create");
            assert_eq!(
                search_markdown_file_index_from_connection(&connection, "Stable")
                    .expect("created file content should be searchable")
                    .iter()
                    .filter(|result| result.relative_path == PathBuf::from("agents/reviewer.md"))
                    .count(),
                1,
                "create sync should add new file content to full-text search"
            );

            fs::write(
                &created_path,
                "---\ntitle: Reviewer Updated\ntags: [updated]\nclassification: main-agent\n---\n# Reviewer\n",
            )
            .expect("updated markdown should be writable");
            let mut update_event = create_event.clone();
            update_event.kind = ContextFileChangeKind::Update;
            let update_reports = sync_markdown_context_index_events(&working_dir, &[update_event])
                .expect("update event should sync");

            assert_eq!(update_reports.len(), 1);

            let connection =
                Connection::open(sqlite_index_path(roots.local_root.as_ref().unwrap()))
                    .expect("local sqlite index should open");
            assert_eq!(
                sqlite_table_count(&connection, MARKDOWN_FILES_TABLE_NAME),
                2,
                "incremental sync should not clear unrelated indexed rows"
            );
            assert_eq!(
                sqlite_path_count(&connection, &stable.file_path),
                1,
                "stable row should survive incremental create/update"
            );
            assert_eq!(
                markdown_file_title(&connection, "agents/reviewer.md"),
                "Reviewer Updated"
            );
            assert_eq!(
                markdown_file_classification(&connection, "agents/reviewer.md"),
                "main-agent"
            );
            assert!(
                search_markdown_file_index_from_connection(&connection, "Stable")
                    .expect("stale created content should be searchable")
                    .iter()
                    .all(|result| result.relative_path != PathBuf::from("agents/reviewer.md")),
                "update sync should remove stale full-text content for the updated file"
            );
            assert_eq!(
                search_markdown_file_index_from_connection(&connection, "Updated")
                    .expect("updated file content should be searchable")
                    .iter()
                    .filter(|result| result.relative_path == PathBuf::from("agents/reviewer.md"))
                    .count(),
                1,
                "update sync should add updated title/content to full-text search"
            );
            assert_eq!(
                connection
                    .query_row(
                        "SELECT COUNT(*) FROM markdown_file_tags WHERE path = ?1 AND tag_id = 'review';",
                        params![created_path.to_string_lossy().replace('\\', "/")],
                        |row| row.get::<_, usize>(0),
                    )
                    .expect("old tag count should be readable"),
                0,
                "update sync should remove stale per-file tag rows"
            );
        });

        fs::remove_dir_all(base).ok();
    }

    #[test]
    fn markdown_index_normalizes_frontmatter_and_inline_metadata() {
        let base = std::env::temp_dir().join(format!("ctx-metadata-index-{}", Uuid::new_v4()));
        let home = base.join("home");
        let working_dir = base.join("project");
        let roots = VaultRoots {
            global_root: home.join(CTX_HOME_DIR).join(GLOBAL_VAULT_DIR),
            local_root: Some(working_dir.join(CTX_HOME_DIR).join(GLOBAL_VAULT_DIR)),
        };
        fs::create_dir_all(&working_dir).expect("working directory should exist");
        fs::create_dir_all(&home).expect("home directory should exist");

        create_context_file(
            &roots,
            VaultScope::Local,
            "agents",
            "reviewer.md",
            "---\ntitle: Reviewer\nclassification: subagent\ntags:\n  - Review\n  - \"Quality Review\"\n  - QA\n---\n# Reviewer\n\nBody tags:: #Rust, implementation\n\nUse #Codex, #Review and [[Stable]].",
        )
        .expect("frontmatter context should be created");
        create_context_file(
            &roots,
            VaultScope::Local,
            "notes",
            "inline.md",
            "# Inline\n\ntitle:: Inline Metadata Title\nclassification:: main-agent\ntags:: #Planner, shared, \"Launch Plan\"\n\nCoordinate #Launch.",
        )
        .expect("inline metadata context should be created");

        with_home(&home, || {
            reindex_markdown_contexts(&working_dir).expect("metadata reindex should pass");
            let connection =
                Connection::open(sqlite_index_path(roots.local_root.as_ref().unwrap()))
                    .expect("local sqlite index should open");

            assert_eq!(
                markdown_file_title(&connection, "agents/reviewer.md"),
                "Reviewer"
            );
            assert_eq!(
                markdown_file_classification(&connection, "agents/reviewer.md"),
                "subagent"
            );
            assert_eq!(
                markdown_file_title(&connection, "notes/inline.md"),
                "Inline Metadata Title"
            );
            assert_eq!(
                markdown_file_classification(&connection, "notes/inline.md"),
                "main-agent"
            );
            assert_eq!(
                markdown_file_tag_source(&connection, "agents/reviewer.md", "review"),
                "frontmatter"
            );
            assert_eq!(
                markdown_file_tag_source(&connection, "agents/reviewer.md", "quality-review"),
                "frontmatter"
            );
            assert_eq!(
                markdown_file_tag_source(&connection, "agents/reviewer.md", "qa"),
                "frontmatter"
            );
            assert_eq!(
                markdown_file_tag_source(&connection, "agents/reviewer.md", "rust"),
                "body"
            );
            assert_eq!(
                markdown_file_tag_source(&connection, "agents/reviewer.md", "codex"),
                "body"
            );
            assert_eq!(
                markdown_file_tag_source(&connection, "notes/inline.md", "planner"),
                "body"
            );
            assert_eq!(
                markdown_file_tag_source(&connection, "notes/inline.md", "launch-plan"),
                "body"
            );
            assert_eq!(
                markdown_file_tag_ids(&connection, "agents/reviewer.md"),
                vec![
                    "review".to_string(),
                    "quality-review".to_string(),
                    "qa".to_string(),
                    "rust".to_string(),
                    "implementation".to_string(),
                    "codex".to_string()
                ]
            );
        });

        fs::remove_dir_all(base).ok();
    }

    #[test]
    fn markdown_index_extracts_body_wikilinks_and_markdown_links_as_link_records() {
        let base = std::env::temp_dir().join(format!("ctx-link-index-{}", Uuid::new_v4()));
        let home = base.join("home");
        let working_dir = base.join("project");
        let roots = VaultRoots {
            global_root: home.join(CTX_HOME_DIR).join(GLOBAL_VAULT_DIR),
            local_root: Some(working_dir.join(CTX_HOME_DIR).join(GLOBAL_VAULT_DIR)),
        };
        fs::create_dir_all(&working_dir).expect("working directory should exist");
        fs::create_dir_all(&home).expect("home directory should exist");

        create_context_file(
            &roots,
            VaultScope::Local,
            "notes",
            "target.md",
            "# Target\n\nLinked target.",
        )
        .expect("target context should be created");
        create_context_file(
            &roots,
            VaultScope::Local,
            "notes",
            "source.md",
            "---\ntitle: Source\n---\n# Source\n\nUse [[Target#intro|target alias]] and [target md](notes/target.md#details).\nExternal [site](https://example.com/docs).",
        )
        .expect("source context should be created");

        with_home(&home, || {
            let report = reindex_markdown_contexts(&working_dir)
                .expect("link reindex should extract body links");
            assert_eq!(report.local.as_ref().unwrap().indexed_links, 3);

            let connection =
                Connection::open(sqlite_index_path(roots.local_root.as_ref().unwrap()))
                    .expect("local sqlite index should open");
            let links = markdown_file_links(&connection, "notes/source.md");

            assert_eq!(
                links,
                vec![
                    (
                        "wikilink".to_string(),
                        "Target#intro".to_string(),
                        Some("target alias".to_string()),
                        Some("intro".to_string()),
                        "resolved".to_string(),
                    ),
                    (
                        "markdown".to_string(),
                        "notes/target.md#details".to_string(),
                        Some("target md".to_string()),
                        Some("details".to_string()),
                        "resolved".to_string(),
                    ),
                    (
                        "markdown".to_string(),
                        "https://example.com/docs".to_string(),
                        Some("site".to_string()),
                        None,
                        "external".to_string(),
                    ),
                ]
            );
        });

        fs::remove_dir_all(base).ok();
    }

    #[test]
    fn incremental_sync_resolves_new_source_links_into_backlink_records() {
        let base = std::env::temp_dir().join(format!(
            "ctx-incremental-backlink-resolution-{}",
            Uuid::new_v4()
        ));
        let home = base.join("home");
        let working_dir = base.join("project");
        let roots = VaultRoots {
            global_root: home.join(CTX_HOME_DIR).join(GLOBAL_VAULT_DIR),
            local_root: Some(working_dir.join(CTX_HOME_DIR).join(GLOBAL_VAULT_DIR)),
        };
        let local_contexts_dir = managed_contexts_dir(roots.local_root.as_ref().unwrap());
        fs::create_dir_all(&local_contexts_dir).expect("local contexts directory should exist");
        fs::create_dir_all(&working_dir).expect("working directory should exist");
        fs::create_dir_all(&home).expect("home directory should exist");

        create_context_file(
            &roots,
            VaultScope::Local,
            "notes",
            "target.md",
            "# Target\n\nOriginal target.",
        )
        .expect("target context should be created");

        with_home(&home, || {
            reindex_markdown_contexts(&working_dir).expect("initial target should be indexed");

            let source = create_context_file(
                &roots,
                VaultScope::Local,
                "agents",
                "source.md",
                "# Source\n\nLinks to [[notes/target#usage|target]].",
            )
            .expect("source context should be created");
            let source_event = ContextFileChangeEvent {
                kind: ContextFileChangeKind::Create,
                vault_scope: VaultScope::Local,
                root_kind: ContextWatchRootKind::ManagedVault,
                root_path: local_contexts_dir.clone(),
                path: source.file_path.clone(),
                relative_path: PathBuf::from("agents/source.md"),
                previous_path: None,
                previous_relative_path: None,
            };

            sync_markdown_context_index_event(&working_dir, &source_event)
                .expect("source create event should sync")
                .expect("source create should produce an index report");

            let database_path = sqlite_index_path(roots.local_root.as_ref().unwrap());
            let target_path = local_contexts_dir.join("notes").join("target.md");
            let backlinks =
                crate::sqlite_index::markdown_file_backlink_records(&database_path, &target_path)
                    .expect("backlink records should query");

            assert_eq!(backlinks.len(), 1);
            assert_eq!(backlinks[0].backlink_source_path, source.file_path);
            assert_eq!(backlinks[0].raw_target, "notes/target#usage");
            assert_eq!(backlinks[0].target_anchor.as_deref(), Some("usage"));
        });

        fs::remove_dir_all(base).ok();
    }

    #[test]
    fn incremental_sync_move_and_delete_events_update_paths_and_clear_stale_index_rows() {
        let base = std::env::temp_dir().join(format!(
            "ctx-incremental-sync-move-delete-{}",
            Uuid::new_v4()
        ));
        let home = base.join("home");
        let working_dir = base.join("project");
        let roots = VaultRoots {
            global_root: home.join(CTX_HOME_DIR).join(GLOBAL_VAULT_DIR),
            local_root: Some(working_dir.join(CTX_HOME_DIR).join(GLOBAL_VAULT_DIR)),
        };
        let local_contexts_dir = managed_contexts_dir(roots.local_root.as_ref().unwrap());
        fs::create_dir_all(&local_contexts_dir).expect("local contexts directory should exist");
        fs::create_dir_all(&working_dir).expect("working directory should exist");
        fs::create_dir_all(&home).expect("home directory should exist");

        let target = create_context_file(
            &roots,
            VaultScope::Local,
            "",
            "target.md",
            "---\ntitle: Target\ntags: [move-delete]\nclassification: shared\n---\n# Target\n\nOriginal target contains rename_delete_probe.",
        )
        .expect("target context should be created");
        create_context_file(
            &roots,
            VaultScope::Local,
            "",
            "zz-source.md",
            "# Zz Source\n\nLinks to [[Target]].",
        )
        .expect("source context should be created");

        with_home(&home, || {
            reindex_markdown_contexts(&working_dir)
                .expect("initial full reindex should seed sqlite");

            let moved_path = local_contexts_dir.join("archive").join("target.md");
            fs::create_dir_all(moved_path.parent().unwrap())
                .expect("moved parent directory should exist");
            fs::rename(&target.file_path, &moved_path).expect("target markdown should move");

            let move_event = ContextFileChangeEvent {
                kind: ContextFileChangeKind::Move,
                vault_scope: VaultScope::Local,
                root_kind: ContextWatchRootKind::ManagedVault,
                root_path: local_contexts_dir.clone(),
                path: moved_path.clone(),
                relative_path: PathBuf::from("archive/target.md"),
                previous_path: Some(target.file_path.clone()),
                previous_relative_path: Some(PathBuf::from("target.md")),
            };
            let move_report = sync_markdown_context_index_event(&working_dir, &move_event)
                .expect("move event should sync")
                .expect("move event should produce an index report");

            assert_eq!(move_report.indexed_markdown_files, 1);

            let connection =
                Connection::open(sqlite_index_path(roots.local_root.as_ref().unwrap()))
                    .expect("local sqlite index should open");
            assert_eq!(
                sqlite_path_count(&connection, &target.file_path),
                0,
                "move sync should remove the old markdown_files row"
            );
            assert_eq!(
                sqlite_path_count_in_table(
                    &connection,
                    MARKDOWN_FILE_SEARCH_TABLE_NAME,
                    &target.file_path
                ),
                0,
                "move sync should remove stale full-text search rows for the old path"
            );
            assert_eq!(
                sqlite_path_count_in_table(
                    &connection,
                    MARKDOWN_FILE_FRONTMATTER_TABLE_NAME,
                    &target.file_path
                ),
                0,
                "move sync should remove stale frontmatter metadata rows for the old path"
            );
            assert_eq!(
                sqlite_path_count_in_table(
                    &connection,
                    MARKDOWN_FILE_TAGS_TABLE_NAME,
                    &target.file_path
                ),
                0,
                "move sync should remove stale tag metadata rows for the old path"
            );
            assert_eq!(
                sqlite_path_count(&connection, &moved_path),
                1,
                "move sync should insert the moved markdown_files row"
            );
            assert_eq!(
                search_markdown_file_index_from_connection(&connection, "rename_delete_probe")
                    .expect("moved file content should remain searchable"),
                vec![crate::sqlite_index::MarkdownFileSearchResult {
                    path: moved_path.clone(),
                    title: "Target".to_string(),
                    relative_path: PathBuf::from("archive/target.md"),
                }],
                "move sync should keep only the new path in full-text search"
            );
            assert_eq!(
                connection
                    .query_row(
                        "SELECT COUNT(*) FROM markdown_file_backlinks WHERE path = ?1;",
                        params![moved_path.to_string_lossy().replace('\\', "/")],
                        |row| row.get::<_, usize>(0),
                    )
                    .expect("backlink count should be readable"),
                1,
                "move sync should retarget existing backlink references to the new path"
            );

            fs::remove_file(&moved_path).expect("moved markdown should delete");
            let delete_event = ContextFileChangeEvent {
                kind: ContextFileChangeKind::Delete,
                vault_scope: VaultScope::Local,
                root_kind: ContextWatchRootKind::ManagedVault,
                root_path: local_contexts_dir.clone(),
                path: moved_path.clone(),
                relative_path: PathBuf::from("archive/target.md"),
                previous_path: None,
                previous_relative_path: None,
            };
            let delete_report = sync_markdown_context_index_event(&working_dir, &delete_event)
                .expect("delete event should sync")
                .expect("delete event should produce a cleanup report");

            assert_eq!(delete_report.indexed_markdown_files, 0);
            assert_eq!(
                sqlite_path_count(&connection, &moved_path),
                0,
                "delete sync should remove the stale moved markdown_files row"
            );
            assert!(
                search_markdown_file_index_from_connection(&connection, "rename_delete_probe")
                    .expect("deleted file content should query")
                    .is_empty(),
                "delete sync should remove deleted file content from full-text search"
            );
            assert_eq!(
                sqlite_path_count_in_table(
                    &connection,
                    MARKDOWN_FILE_SEARCH_TABLE_NAME,
                    &moved_path
                ),
                0,
                "delete sync should remove stale search rows for the deleted path"
            );
            assert_eq!(
                sqlite_path_count_in_table(
                    &connection,
                    MARKDOWN_FILE_FRONTMATTER_TABLE_NAME,
                    &moved_path
                ),
                0,
                "delete sync should remove stale frontmatter metadata rows for the deleted path"
            );
            assert_eq!(
                sqlite_path_count_in_table(&connection, MARKDOWN_FILE_TAGS_TABLE_NAME, &moved_path),
                0,
                "delete sync should remove stale tag metadata rows for the deleted path"
            );
            assert_eq!(
                connection
                    .query_row(
                        "SELECT COUNT(*) FROM markdown_file_links WHERE target_path IS NULL AND resolved_status = 'unresolved';",
                        [],
                        |row| row.get::<_, usize>(0),
                    )
                    .expect("unresolved link count should be readable"),
                1,
                "delete sync should clear stale target_path references"
            );
        });

        fs::remove_dir_all(base).ok();
    }

    #[test]
    fn rejects_duplicate_context_path() {
        let (roots, base) = test_roots();
        create_context_file(&roots, VaultScope::Global, "", "shared.md", "first")
            .expect("initial context should be created");

        let error = create_context_file(&roots, VaultScope::Global, "", "shared.md", "second")
            .expect_err("duplicate context path should fail");

        assert!(matches!(error, VaultError::DuplicateContext(_)));
        fs::remove_dir_all(base).ok();
    }

    #[test]
    fn validates_filename_and_extension() {
        let (roots, base) = test_roots();

        let path_error =
            create_context_file(&roots, VaultScope::Local, "", "../escape.md", "content")
                .expect_err("path separators should be rejected");
        assert!(matches!(path_error, VaultError::InvalidFileName(_)));

        let extension_error =
            create_context_file(&roots, VaultScope::Local, "", "notes.txt", "content")
                .expect_err("non-markdown extension should be rejected");
        assert!(matches!(extension_error, VaultError::InvalidExtension(_)));

        fs::remove_dir_all(base).ok();
    }

    #[test]
    fn rejects_folder_traversal() {
        let (roots, base) = test_roots();
        let error = create_context_file(
            &roots,
            VaultScope::Local,
            "../outside",
            "safe.md",
            "content",
        )
        .expect_err("folder traversal should be rejected");

        assert!(matches!(error, VaultError::InvalidFolderPath(_)));
        fs::remove_dir_all(base).ok();
    }

    fn sqlite_table_count(connection: &Connection, table_name: &str) -> usize {
        connection
            .query_row(&format!("SELECT COUNT(*) FROM {table_name};"), [], |row| {
                row.get::<_, usize>(0)
            })
            .expect("table count should be readable")
    }

    fn sqlite_path_count(connection: &Connection, path: &Path) -> usize {
        connection
            .query_row(
                "SELECT COUNT(*) FROM markdown_files WHERE path = ?1;",
                params![path.to_string_lossy().replace('\\', "/")],
                |row| row.get::<_, usize>(0),
            )
            .expect("path count should be readable")
    }

    fn sqlite_path_count_in_table(connection: &Connection, table_name: &str, path: &Path) -> usize {
        connection
            .query_row(
                &format!("SELECT COUNT(*) FROM {table_name} WHERE path = ?1;"),
                params![path.to_string_lossy().replace('\\', "/")],
                |row| row.get::<_, usize>(0),
            )
            .expect("table path count should be readable")
    }

    fn markdown_file_classification(connection: &Connection, relative_path: &str) -> String {
        connection
            .query_row(
                "SELECT classification FROM markdown_files WHERE relative_path = ?1;",
                params![relative_path],
                |row| row.get::<_, String>(0),
            )
            .expect("classification should be indexed")
    }

    fn markdown_file_classification_status(connection: &Connection, relative_path: &str) -> String {
        connection
            .query_row(
                "SELECT llm_classification_status FROM markdown_files WHERE relative_path = ?1;",
                params![relative_path],
                |row| row.get::<_, String>(0),
            )
            .expect("classification status should be indexed")
    }

    fn markdown_file_title(connection: &Connection, relative_path: &str) -> String {
        connection
            .query_row(
                "SELECT title FROM markdown_files WHERE relative_path = ?1;",
                params![relative_path],
                |row| row.get::<_, String>(0),
            )
            .expect("title should be indexed")
    }

    fn markdown_file_import_source_type(
        connection: &Connection,
        relative_path: &str,
    ) -> Option<String> {
        connection
            .query_row(
                "SELECT import_source_type FROM markdown_files WHERE relative_path = ?1;",
                params![relative_path],
                |row| row.get::<_, Option<String>>(0),
            )
            .expect("import source type should be indexed")
    }

    fn markdown_file_tag_source(
        connection: &Connection,
        relative_path: &str,
        tag_id: &str,
    ) -> String {
        connection
            .query_row(
                r#"
                SELECT markdown_file_tags.tag_source
                FROM markdown_file_tags
                JOIN markdown_files ON markdown_file_tags.path = markdown_files.path
                WHERE markdown_files.relative_path = ?1
                  AND markdown_file_tags.tag_id = ?2;
                "#,
                params![relative_path, tag_id],
                |row| row.get::<_, String>(0),
            )
            .expect("tag source should be indexed")
    }

    fn markdown_file_tag_ids(connection: &Connection, relative_path: &str) -> Vec<String> {
        let mut statement = connection
            .prepare(
                r#"
                SELECT markdown_file_tags.tag_id
                FROM markdown_file_tags
                JOIN markdown_files ON markdown_file_tags.path = markdown_files.path
                WHERE markdown_files.relative_path = ?1
                ORDER BY markdown_file_tags.tag_position;
                "#,
            )
            .expect("tag query should prepare");

        statement
            .query_map(params![relative_path], |row| row.get::<_, String>(0))
            .expect("tag rows should query")
            .collect::<Result<Vec<_>, _>>()
            .expect("tag ids should collect")
    }

    fn markdown_file_links(
        connection: &Connection,
        relative_path: &str,
    ) -> Vec<(String, String, Option<String>, Option<String>, String)> {
        let mut statement = connection
            .prepare(
                r#"
                SELECT
                    markdown_file_links.link_kind,
                    markdown_file_links.raw_target,
                    markdown_file_links.link_text,
                    markdown_file_links.target_anchor,
                    markdown_file_links.resolved_status
                FROM markdown_file_links
                JOIN markdown_files
                  ON markdown_file_links.source_path = markdown_files.path
                WHERE markdown_files.relative_path = ?1
                ORDER BY markdown_file_links.link_position;
                "#,
            )
            .expect("link query should prepare");
        let rows = statement
            .query_map(params![relative_path], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, String>(4)?,
                ))
            })
            .expect("link rows should query");

        rows.map(|row| row.expect("link row should decode"))
            .collect()
    }

    fn with_home(home: &Path, test: impl FnOnce()) {
        let _guard = HOME_ENV_LOCK
            .lock()
            .expect("HOME env lock should not be poisoned");
        let previous_home = std::env::var_os("HOME");
        std::env::set_var("HOME", home);
        test();
        match previous_home {
            Some(value) => std::env::set_var("HOME", value),
            None => std::env::remove_var("HOME"),
        }
    }
}
