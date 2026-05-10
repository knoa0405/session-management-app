use crate::settings::{load_configured_scan_roots, load_configured_skill_scan_roots};
use crate::vault::{managed_contexts_dir, VaultRoots};
use crate::VaultScope;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    fmt, fs,
    path::{Path, PathBuf},
    sync::mpsc,
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

const WATCH_EXCLUDED_DIRECTORY_NAMES: &[&str] = &[
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

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Eq, PartialEq, Ord, PartialOrd)]
#[serde(rename_all = "kebab-case")]
pub enum ContextWatchRootKind {
    ManagedVault,
    ConfiguredScanRoot,
    ConfiguredSkillRoot,
}

#[derive(Debug, Clone, Serialize, Deserialize, Eq, PartialEq, Ord, PartialOrd)]
pub struct ContextWatchRoot {
    pub path: PathBuf,
    pub vault_scope: VaultScope,
    pub root_kind: ContextWatchRootKind,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Eq, PartialEq, Ord, PartialOrd)]
#[serde(rename_all = "kebab-case")]
pub enum ContextFileChangeKind {
    Create,
    Update,
    Move,
    Delete,
}

#[derive(Debug, Clone, Serialize, Deserialize, Eq, PartialEq)]
pub struct ContextFileChangeEvent {
    pub kind: ContextFileChangeKind,
    pub vault_scope: VaultScope,
    pub root_kind: ContextWatchRootKind,
    pub root_path: PathBuf,
    pub path: PathBuf,
    pub relative_path: PathBuf,
    #[serde(default)]
    pub previous_path: Option<PathBuf>,
    #[serde(default)]
    pub previous_relative_path: Option<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Eq, PartialEq)]
pub struct ContextFileSnapshotEntry {
    pub vault_scope: VaultScope,
    pub root_kind: ContextWatchRootKind,
    pub root_path: PathBuf,
    pub path: PathBuf,
    pub relative_path: PathBuf,
    pub content_fingerprint: String,
    pub byte_len: u64,
    pub modified_unix_seconds: i64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, Eq, PartialEq)]
pub struct ContextFileSnapshot {
    pub files: BTreeMap<PathBuf, ContextFileSnapshotEntry>,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub enum ContextWatchError {
    Settings(String),
    Io(String),
}

impl fmt::Display for ContextWatchError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Settings(message) => write!(formatter, "{message}"),
            Self::Io(message) => write!(formatter, "{message}"),
        }
    }
}

impl std::error::Error for ContextWatchError {}

#[derive(Debug, Clone)]
pub struct ContextDirectoryWatcher {
    roots: Vec<ContextWatchRoot>,
    previous_snapshot: ContextFileSnapshot,
}

impl ContextDirectoryWatcher {
    pub fn new(working_dir: &Path) -> Result<Self, ContextWatchError> {
        let roots = configured_context_watch_roots(working_dir)?;
        Self::from_roots(roots)
    }

    pub fn from_roots(roots: Vec<ContextWatchRoot>) -> Result<Self, ContextWatchError> {
        let previous_snapshot = snapshot_context_directories(&roots)?;
        Ok(Self {
            roots,
            previous_snapshot,
        })
    }

    pub fn roots(&self) -> &[ContextWatchRoot] {
        &self.roots
    }

    pub fn poll(&mut self) -> Result<Vec<ContextFileChangeEvent>, ContextWatchError> {
        let current_snapshot = snapshot_context_directories(&self.roots)?;
        let events = diff_context_file_snapshots(&self.previous_snapshot, &current_snapshot);
        self.previous_snapshot = current_snapshot;
        Ok(events)
    }
}

pub fn watch_context_directories<F>(
    working_dir: &Path,
    poll_interval: Duration,
    stop_rx: mpsc::Receiver<()>,
    mut emit: F,
) -> Result<(), ContextWatchError>
where
    F: FnMut(ContextFileChangeEvent),
{
    let mut watcher = ContextDirectoryWatcher::new(working_dir)?;

    loop {
        match stop_rx.try_recv() {
            Ok(()) | Err(mpsc::TryRecvError::Disconnected) => return Ok(()),
            Err(mpsc::TryRecvError::Empty) => {}
        }

        for event in watcher.poll()? {
            emit(event);
        }

        thread::sleep(poll_interval);
    }
}

pub fn configured_context_watch_roots(
    working_dir: &Path,
) -> Result<Vec<ContextWatchRoot>, ContextWatchError> {
    let roots = VaultRoots::discover(working_dir);
    let mut watch_roots = Vec::new();
    let mut seen = HashSet::new();

    push_watch_root(
        &mut watch_roots,
        &mut seen,
        managed_contexts_dir(&roots.global_root),
        VaultScope::Global,
        ContextWatchRootKind::ManagedVault,
    );

    if let Some(local_root) = &roots.local_root {
        push_watch_root(
            &mut watch_roots,
            &mut seen,
            managed_contexts_dir(local_root),
            VaultScope::Local,
            ContextWatchRootKind::ManagedVault,
        );
    }

    let configured_scan_roots =
        load_configured_scan_roots(&roots, working_dir).map_err(|error| {
            ContextWatchError::Settings(format!("failed to load context watch roots: {error}"))
        })?;
    for root in configured_scan_roots {
        push_watch_root(
            &mut watch_roots,
            &mut seen,
            root.path,
            root.scope,
            ContextWatchRootKind::ConfiguredScanRoot,
        );
    }

    let configured_skill_roots =
        load_configured_skill_scan_roots(&roots, working_dir).map_err(|error| {
            ContextWatchError::Settings(format!("failed to load skill watch roots: {error}"))
        })?;
    for root in configured_skill_roots {
        push_watch_root(
            &mut watch_roots,
            &mut seen,
            root.path,
            root.scope,
            ContextWatchRootKind::ConfiguredSkillRoot,
        );
    }

    watch_roots.sort();
    Ok(watch_roots)
}

pub fn snapshot_context_directories(
    roots: &[ContextWatchRoot],
) -> Result<ContextFileSnapshot, ContextWatchError> {
    let mut snapshot = ContextFileSnapshot::default();

    for root in roots {
        if !root.path.is_dir() {
            continue;
        }
        collect_markdown_snapshot_entries(root, &root.path, &mut snapshot)?;
    }

    Ok(snapshot)
}

pub fn diff_context_file_snapshots(
    previous: &ContextFileSnapshot,
    current: &ContextFileSnapshot,
) -> Vec<ContextFileChangeEvent> {
    let mut events = Vec::new();
    let mut removed = Vec::new();
    let mut created = Vec::new();

    for (path, previous_entry) in &previous.files {
        match current.files.get(path) {
            Some(current_entry) if snapshot_entry_changed(previous_entry, current_entry) => {
                events.push(event_from_entry(
                    ContextFileChangeKind::Update,
                    current_entry,
                    None,
                ));
            }
            Some(_) => {}
            None => removed.push(previous_entry.clone()),
        }
    }

    for (path, current_entry) in &current.files {
        if !previous.files.contains_key(path) {
            created.push(current_entry.clone());
        }
    }

    let mut moved_created_indexes = HashSet::new();
    let mut created_by_fingerprint: HashMap<String, Vec<usize>> = HashMap::new();
    for (index, entry) in created.iter().enumerate() {
        created_by_fingerprint
            .entry(move_fingerprint_key(entry))
            .or_default()
            .push(index);
    }

    for removed_entry in &removed {
        let key = move_fingerprint_key(removed_entry);
        let Some(indexes) = created_by_fingerprint.get(&key) else {
            events.push(event_from_entry(
                ContextFileChangeKind::Delete,
                removed_entry,
                None,
            ));
            continue;
        };
        let Some(created_index) = indexes
            .iter()
            .copied()
            .find(|index| !moved_created_indexes.contains(index))
        else {
            events.push(event_from_entry(
                ContextFileChangeKind::Delete,
                removed_entry,
                None,
            ));
            continue;
        };

        moved_created_indexes.insert(created_index);
        events.push(event_from_entry(
            ContextFileChangeKind::Move,
            &created[created_index],
            Some(removed_entry),
        ));
    }

    for (index, created_entry) in created.iter().enumerate() {
        if !moved_created_indexes.contains(&index) {
            events.push(event_from_entry(
                ContextFileChangeKind::Create,
                created_entry,
                None,
            ));
        }
    }

    events.sort_by(|left, right| {
        left.vault_scope
            .cmp(&right.vault_scope)
            .then_with(|| left.root_kind.cmp(&right.root_kind))
            .then_with(|| left.relative_path.cmp(&right.relative_path))
            .then_with(|| left.kind.cmp(&right.kind))
    });
    events
}

fn push_watch_root(
    watch_roots: &mut Vec<ContextWatchRoot>,
    seen: &mut HashSet<PathBuf>,
    path: PathBuf,
    vault_scope: VaultScope,
    root_kind: ContextWatchRootKind,
) {
    let normalized = normalize_watch_root_path(&path);
    if seen.insert(normalized.clone()) {
        watch_roots.push(ContextWatchRoot {
            path: normalized,
            vault_scope,
            root_kind,
        });
    }
}

fn collect_markdown_snapshot_entries(
    root: &ContextWatchRoot,
    dir: &Path,
    snapshot: &mut ContextFileSnapshot,
) -> Result<(), ContextWatchError> {
    let entries = fs::read_dir(dir).map_err(|error| {
        ContextWatchError::Io(format!(
            "failed to read context watch directory {}: {error}",
            dir.display()
        ))
    })?;

    for entry in entries {
        let entry = entry.map_err(|error| {
            ContextWatchError::Io(format!(
                "failed to inspect context watch directory entry in {}: {error}",
                dir.display()
            ))
        })?;
        let path = entry.path();
        let metadata = entry.metadata().map_err(|error| {
            ContextWatchError::Io(format!(
                "failed to inspect context watch path {}: {error}",
                path.display()
            ))
        })?;

        if metadata.is_dir() {
            if !is_excluded_watch_dir(&path) {
                collect_markdown_snapshot_entries(root, &path, snapshot)?;
            }
            continue;
        }

        if metadata.is_file() && is_markdown_path(&path) {
            let content = fs::read(&path).map_err(|error| {
                ContextWatchError::Io(format!(
                    "failed to read watched markdown file {}: {error}",
                    path.display()
                ))
            })?;
            let relative_path = path
                .strip_prefix(&root.path)
                .map(Path::to_path_buf)
                .unwrap_or_else(|_| path.clone());
            snapshot.files.insert(
                path.clone(),
                ContextFileSnapshotEntry {
                    vault_scope: root.vault_scope,
                    root_kind: root.root_kind,
                    root_path: root.path.clone(),
                    path,
                    relative_path,
                    content_fingerprint: content_fingerprint(&content),
                    byte_len: metadata.len(),
                    modified_unix_seconds: metadata
                        .modified()
                        .map(system_time_to_unix_seconds)
                        .unwrap_or(0),
                },
            );
        }
    }

    Ok(())
}

fn normalize_watch_root_path(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

fn is_excluded_watch_dir(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .map(|name| WATCH_EXCLUDED_DIRECTORY_NAMES.contains(&name))
        .unwrap_or(false)
}

fn is_markdown_path(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| extension.eq_ignore_ascii_case("md"))
        .unwrap_or(false)
}

fn snapshot_entry_changed(
    previous: &ContextFileSnapshotEntry,
    current: &ContextFileSnapshotEntry,
) -> bool {
    previous.content_fingerprint != current.content_fingerprint
        || previous.byte_len != current.byte_len
        || previous.vault_scope != current.vault_scope
        || previous.root_kind != current.root_kind
        || previous.root_path != current.root_path
}

fn event_from_entry(
    kind: ContextFileChangeKind,
    entry: &ContextFileSnapshotEntry,
    previous: Option<&ContextFileSnapshotEntry>,
) -> ContextFileChangeEvent {
    ContextFileChangeEvent {
        kind,
        vault_scope: entry.vault_scope,
        root_kind: entry.root_kind,
        root_path: entry.root_path.clone(),
        path: entry.path.clone(),
        relative_path: entry.relative_path.clone(),
        previous_path: previous.map(|entry| entry.path.clone()),
        previous_relative_path: previous.map(|entry| entry.relative_path.clone()),
    }
}

fn move_fingerprint_key(entry: &ContextFileSnapshotEntry) -> String {
    format!(
        "{:?}:{:?}:{}:{}",
        entry.vault_scope, entry.root_kind, entry.byte_len, entry.content_fingerprint
    )
}

fn content_fingerprint(content: &[u8]) -> String {
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in content {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

fn system_time_to_unix_seconds(time: SystemTime) -> i64 {
    time.duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::{
        configured_context_watch_roots, diff_context_file_snapshots, snapshot_context_directories,
        ContextDirectoryWatcher, ContextFileChangeKind, ContextWatchRoot, ContextWatchRootKind,
    };
    use crate::{managed_contexts_dir, vault_settings_path, VaultRoots, VaultScope};
    use std::{fs, path::PathBuf};
    use uuid::Uuid;

    fn temp_base(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!("ctx-watch-{label}-{}", Uuid::new_v4()))
    }

    fn watch_root(path: PathBuf) -> ContextWatchRoot {
        ContextWatchRoot {
            path,
            vault_scope: VaultScope::Local,
            root_kind: ContextWatchRootKind::ManagedVault,
        }
    }

    #[test]
    fn diff_emits_create_update_move_and_delete_events() {
        let base = temp_base("diff-events");
        let root = base.join("contexts");
        fs::create_dir_all(&root).expect("watch root should be created");
        let roots = vec![watch_root(root.clone())];

        let original = root.join("agent.md");
        fs::write(&original, "# Agent\n").expect("initial markdown should be writable");
        let first = snapshot_context_directories(&roots).expect("initial snapshot should pass");

        let created = root.join("new.md");
        fs::write(&created, "# New\n").expect("created markdown should be writable");
        let second = snapshot_context_directories(&roots).expect("second snapshot should pass");
        let events = diff_context_file_snapshots(&first, &second);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].kind, ContextFileChangeKind::Create);
        assert_eq!(events[0].relative_path, PathBuf::from("new.md"));

        fs::write(&created, "# New\n\nUpdated.\n").expect("markdown should update");
        let third = snapshot_context_directories(&roots).expect("third snapshot should pass");
        let events = diff_context_file_snapshots(&second, &third);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].kind, ContextFileChangeKind::Update);

        let moved = root.join("renamed.md");
        fs::rename(&created, &moved).expect("markdown should move");
        let fourth = snapshot_context_directories(&roots).expect("fourth snapshot should pass");
        let events = diff_context_file_snapshots(&third, &fourth);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].kind, ContextFileChangeKind::Move);
        assert_eq!(events[0].relative_path, PathBuf::from("renamed.md"));
        assert_eq!(
            events[0].previous_relative_path,
            Some(PathBuf::from("new.md"))
        );

        fs::remove_file(&moved).expect("markdown should delete");
        let fifth = snapshot_context_directories(&roots).expect("fifth snapshot should pass");
        let events = diff_context_file_snapshots(&fourth, &fifth);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].kind, ContextFileChangeKind::Delete);

        fs::remove_dir_all(base).ok();
    }

    #[test]
    fn watcher_poll_uses_previous_snapshot_and_advances_after_events() {
        let base = temp_base("poll");
        let root = base.join("contexts");
        fs::create_dir_all(&root).expect("watch root should be created");
        let roots = vec![watch_root(root.clone())];
        let mut watcher =
            ContextDirectoryWatcher::from_roots(roots).expect("watcher should initialize");

        fs::write(root.join("agent.md"), "# Agent\n").expect("markdown should be writable");
        let events = watcher.poll().expect("poll should detect create");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].kind, ContextFileChangeKind::Create);

        let events = watcher.poll().expect("second poll should advance baseline");
        assert!(events.is_empty());

        fs::remove_dir_all(base).ok();
    }

    #[test]
    fn diff_coalesces_create_then_update_between_polls_into_create() {
        let base = temp_base("coalesce-create-update");
        let root = base.join("contexts");
        fs::create_dir_all(&root).expect("watch root should be created");
        let roots = vec![watch_root(root.clone())];
        let before = snapshot_context_directories(&roots).expect("initial snapshot should pass");

        let created = root.join("agent.md");
        fs::write(&created, "# Agent\n").expect("created markdown should be writable");
        fs::write(&created, "# Agent\n\nFinal content.\n")
            .expect("updated markdown should be writable");

        let after = snapshot_context_directories(&roots).expect("final snapshot should pass");
        let events = diff_context_file_snapshots(&before, &after);

        assert_eq!(events.len(), 1);
        assert_eq!(events[0].kind, ContextFileChangeKind::Create);
        assert_eq!(events[0].relative_path, PathBuf::from("agent.md"));

        fs::remove_dir_all(base).ok();
    }

    #[test]
    fn diff_coalesces_repeated_updates_between_polls_into_single_update() {
        let base = temp_base("coalesce-updates");
        let root = base.join("contexts");
        fs::create_dir_all(&root).expect("watch root should be created");
        let roots = vec![watch_root(root.clone())];

        let path = root.join("agent.md");
        fs::write(&path, "# Agent\n").expect("initial markdown should be writable");
        let before = snapshot_context_directories(&roots).expect("initial snapshot should pass");

        fs::write(&path, "# Agent\n\nDraft one.\n").expect("first update should be writable");
        fs::write(&path, "# Agent\n\nDraft two.\n").expect("second update should be writable");
        fs::write(&path, "# Agent\n\nFinal content.\n").expect("final update should be writable");

        let after = snapshot_context_directories(&roots).expect("final snapshot should pass");
        let events = diff_context_file_snapshots(&before, &after);

        assert_eq!(events.len(), 1);
        assert_eq!(events[0].kind, ContextFileChangeKind::Update);
        assert_eq!(events[0].relative_path, PathBuf::from("agent.md"));

        fs::remove_dir_all(base).ok();
    }

    #[test]
    fn diff_debounces_transient_create_delete_between_polls() {
        let base = temp_base("debounce-create-delete");
        let root = base.join("contexts");
        fs::create_dir_all(&root).expect("watch root should be created");
        let roots = vec![watch_root(root.clone())];
        let before = snapshot_context_directories(&roots).expect("initial snapshot should pass");

        let transient = root.join("scratch.md");
        fs::write(&transient, "# Scratch\n").expect("transient markdown should be writable");
        fs::remove_file(&transient).expect("transient markdown should be removable");

        let after = snapshot_context_directories(&roots).expect("final snapshot should pass");
        let events = diff_context_file_snapshots(&before, &after);

        assert!(events.is_empty());

        fs::remove_dir_all(base).ok();
    }

    #[test]
    fn configured_roots_include_managed_vault_and_configured_scan_directories() {
        let base = temp_base("configured-roots");
        let working_dir = base.join("project");
        let roots = VaultRoots {
            global_root: base.join("home").join(".ctx").join("vault"),
            local_root: Some(working_dir.join(".ctx").join("vault")),
        };
        let local_contexts = managed_contexts_dir(roots.local_root.as_ref().unwrap());
        let configured = working_dir.join("extra-contexts");
        let skills = working_dir.join("skills");

        fs::create_dir_all(&local_contexts).expect("local contexts should be created");
        fs::create_dir_all(&configured).expect("configured root should be created");
        fs::create_dir_all(&skills).expect("skill root should be created");
        fs::write(
            vault_settings_path(roots.local_root.as_ref().unwrap()),
            r#"{
  "scan_roots": [{"path": "extra-contexts", "scope": "local"}],
  "skill_scan_roots": [{"path": "skills", "scope": "local"}]
}"#,
        )
        .expect("settings should be writable");

        let discovered =
            configured_context_watch_roots(&working_dir).expect("watch roots should resolve");

        let local_contexts = local_contexts
            .canonicalize()
            .expect("local contexts should canonicalize");

        assert!(discovered.iter().any(|root| {
            root.path == local_contexts
                && root.vault_scope == VaultScope::Local
                && root.root_kind == ContextWatchRootKind::ManagedVault
        }));
        assert!(discovered.iter().any(|root| {
            root.path == configured.canonicalize().unwrap()
                && root.root_kind == ContextWatchRootKind::ConfiguredScanRoot
        }));
        assert!(discovered.iter().any(|root| {
            root.path == skills.canonicalize().unwrap()
                && root.root_kind == ContextWatchRootKind::ConfiguredSkillRoot
        }));

        fs::remove_dir_all(base).ok();
    }
}
