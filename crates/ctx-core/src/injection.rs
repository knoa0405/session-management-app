use super::models::{
    CliTarget, ContextFragment, InjectionMarkers, Preset, PresetContextComposition,
    PresetContextSelection, PresetContextSelectionKind, ResolvedContextItem, SubagentManifest,
};
use serde::{Deserialize, Serialize};
use std::{
    cmp::Ordering,
    collections::HashMap,
    error::Error,
    fmt,
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};
use uuid::Uuid;

pub const CTX_START_MARKER: &str = "<!-- [ctx:start] -->";
pub const CTX_END_MARKER: &str = "<!-- [ctx:end] -->";
pub const AGENTS_MD_FILE_NAME: &str = "AGENTS.md";
pub const WRAPPER_STATE_DIR_NAME: &str = "wrapper-sessions";
pub const COMBINED_CONTEXT_ITEM_SEPARATOR: &str = "\n\n---\n\n";

pub fn build_markers(preset_name: impl Into<String>, session_id: Uuid) -> InjectionMarkers {
    InjectionMarkers {
        preset_name: preset_name.into(),
        session_id,
        start_marker: CTX_START_MARKER.to_string(),
        end_marker: CTX_END_MARKER.to_string(),
    }
}

pub fn injection_strategy(target: CliTarget) -> &'static str {
    match target {
        CliTarget::Claude => "append-system-prompt-file",
        CliTarget::Codex => "agents-md-section-marker-merge",
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub enum SectionReplaceError {
    MissingStartMarker,
    MissingEndMarker,
    MultipleManagedSections,
    EndMarkerBeforeStartMarker,
}

impl fmt::Display for SectionReplaceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingStartMarker => write!(
                formatter,
                "managed ctx block is malformed: found end marker without start marker"
            ),
            Self::MissingEndMarker => write!(
                formatter,
                "managed ctx block is malformed: found start marker without end marker"
            ),
            Self::MultipleManagedSections => {
                write!(
                    formatter,
                    "managed ctx block is malformed: found multiple sections"
                )
            }
            Self::EndMarkerBeforeStartMarker => write!(
                formatter,
                "managed ctx block is malformed: end marker appears before start marker"
            ),
        }
    }
}

impl Error for SectionReplaceError {}

pub fn build_agents_md_managed_section(content: &str) -> String {
    let trimmed_content = content.trim_matches('\n');

    if trimmed_content.is_empty() {
        format!("{CTX_START_MARKER}\n{CTX_END_MARKER}")
    } else {
        format!("{CTX_START_MARKER}\n{trimmed_content}\n{CTX_END_MARKER}")
    }
}

pub fn replace_agents_md_managed_section(
    agents_md_content: &str,
    managed_content: &str,
) -> Result<String, SectionReplaceError> {
    replace_or_remove_agents_md_managed_section(agents_md_content, Some(managed_content))
}

pub fn remove_agents_md_managed_section(
    agents_md_content: &str,
) -> Result<String, SectionReplaceError> {
    replace_or_remove_agents_md_managed_section(agents_md_content, None)
}

fn replace_or_remove_agents_md_managed_section(
    agents_md_content: &str,
    managed_content: Option<&str>,
) -> Result<String, SectionReplaceError> {
    let section_bounds = find_managed_section_bounds(agents_md_content)?;
    let replacement = managed_content.map(build_agents_md_managed_section);

    match (section_bounds, replacement) {
        (Some((start, end)), Some(section)) => {
            let mut next =
                String::with_capacity(agents_md_content.len() - (end - start) + section.len() + 2);
            next.push_str(&agents_md_content[..start]);
            next.push_str(&section);
            next.push_str(&agents_md_content[end..]);
            Ok(next)
        }
        (Some((start, end)), None) => {
            let mut next = String::with_capacity(agents_md_content.len() - (end - start));
            next.push_str(&agents_md_content[..start]);
            next.push_str(&agents_md_content[end..]);
            Ok(next)
        }
        (None, Some(section)) => {
            let mut next = String::with_capacity(agents_md_content.len() + section.len() + 2);
            next.push_str(agents_md_content.trim_end_matches('\n'));
            append_with_boundary_newlines(&mut next, &section);
            Ok(next)
        }
        (None, None) => Ok(agents_md_content.to_string()),
    }
}

fn append_with_boundary_newlines(output: &mut String, section: &str) {
    if !output.is_empty() {
        output.push_str("\n\n");
    }
    output.push_str(section);
    output.push('\n');
}

fn find_managed_section_bounds(
    agents_md_content: &str,
) -> Result<Option<(usize, usize)>, SectionReplaceError> {
    let start_matches: Vec<_> = agents_md_content.match_indices(CTX_START_MARKER).collect();
    let end_matches: Vec<_> = agents_md_content.match_indices(CTX_END_MARKER).collect();

    match (start_matches.len(), end_matches.len()) {
        (0, 0) => Ok(None),
        (0, _) => Err(SectionReplaceError::MissingStartMarker),
        (_, 0) => Err(SectionReplaceError::MissingEndMarker),
        (1, 1) => {
            let (start_index, _) = start_matches[0];
            let (end_marker_index, end_marker) = end_matches[0];

            if end_marker_index < start_index {
                return Err(SectionReplaceError::EndMarkerBeforeStartMarker);
            }

            Ok(Some((start_index, end_marker_index + end_marker.len())))
        }
        _ => Err(SectionReplaceError::MultipleManagedSections),
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ClaudePromptFile {
    pub path: PathBuf,
    pub selected_context_ids: Vec<Uuid>,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct CodexAgentsMdInjection {
    pub path: PathBuf,
    pub selected_context_ids: Vec<Uuid>,
    pub managed_content: String,
    pub had_existing_file: bool,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct CodexResidualMarkers {
    pub path: PathBuf,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Eq, PartialEq)]
pub struct TransientWrapperState {
    pub session_id: Uuid,
    pub preset_id: Uuid,
    pub target: CliTarget,
    pub child_pid: u32,
    pub working_dir: PathBuf,
    pub claude_prompt_file: Option<PathBuf>,
    pub codex_agents_md_path: Option<PathBuf>,
    pub codex_had_existing_agents_md: bool,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct WrapperStateCleanupReport {
    pub scanned: usize,
    pub cleaned: usize,
    pub skipped_active: usize,
    pub errors: Vec<String>,
}

impl WrapperStateCleanupReport {
    fn empty() -> Self {
        Self {
            scanned: 0,
            cleaned: 0,
            skipped_active: 0,
            errors: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub enum PromptAssemblyError {
    UnsupportedTarget(CliTarget),
    MissingContext(Uuid),
    InvalidSelection(String),
    Io(String),
}

impl fmt::Display for PromptAssemblyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedTarget(target) => write!(
                formatter,
                "Claude prompt files can only be assembled for Claude presets, got {target:?}"
            ),
            Self::MissingContext(context_id) => {
                write!(formatter, "preset references missing context: {context_id}")
            }
            Self::InvalidSelection(message) => write!(formatter, "{message}"),
            Self::Io(message) => write!(
                formatter,
                "failed to assemble Claude prompt file: {message}"
            ),
        }
    }
}

impl Error for PromptAssemblyError {}

#[derive(Debug, Clone, Eq, PartialEq)]
pub enum CodexInjectionError {
    UnsupportedTarget(CliTarget),
    MissingContext(Uuid),
    InvalidSelection(String),
    SectionReplace(SectionReplaceError),
    Io(String),
}

impl fmt::Display for CodexInjectionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedTarget(target) => write!(
                formatter,
                "Codex AGENTS.md injection can only be assembled for Codex presets, got {target:?}"
            ),
            Self::MissingContext(context_id) => {
                write!(formatter, "preset references missing context: {context_id}")
            }
            Self::InvalidSelection(message) => write!(formatter, "{message}"),
            Self::SectionReplace(error) => write!(formatter, "{error}"),
            Self::Io(message) => write!(formatter, "{message}"),
        }
    }
}

impl Error for CodexInjectionError {}

#[derive(Debug, Clone, Eq, PartialEq)]
pub enum WrapperStateError {
    Io(String),
    Json(String),
}

impl fmt::Display for WrapperStateError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(message) => write!(formatter, "{message}"),
            Self::Json(message) => write!(formatter, "{message}"),
        }
    }
}

impl Error for WrapperStateError {}

#[derive(Debug, Clone, Eq, PartialEq)]
pub enum ContextItemResolveError {
    MissingContext(Uuid),
    InvalidSelection {
        context_id: Uuid,
        source_ref: String,
        reason: String,
    },
}

impl fmt::Display for ContextItemResolveError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingContext(context_id) => {
                write!(formatter, "preset references missing context: {context_id}")
            }
            Self::InvalidSelection {
                context_id,
                source_ref,
                reason,
            } => write!(
                formatter,
                "invalid context selection for {source_ref} ({context_id}): {reason}"
            ),
        }
    }
}

impl Error for ContextItemResolveError {}

#[derive(Debug, Clone, Eq, PartialEq)]
pub enum SubagentContextResolveError {
    MissingManifest,
    MissingRole(String),
    MissingAssignedContext {
        role_id: String,
        context_ref: String,
    },
    ContextItem(ContextItemResolveError),
}

impl fmt::Display for SubagentContextResolveError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingManifest => write!(
                formatter,
                "preset does not define a subagent_manifest for subagent context rendering"
            ),
            Self::MissingRole(role_id) => write!(
                formatter,
                "subagent_manifest does not define role: {role_id}"
            ),
            Self::MissingAssignedContext {
                role_id,
                context_ref,
            } => write!(
                formatter,
                "subagent role {role_id} references missing assigned context: {context_ref}"
            ),
            Self::ContextItem(error) => write!(formatter, "{error}"),
        }
    }
}

impl Error for SubagentContextResolveError {}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct ContextRenderOptions {
    pub include_subagent_manifest: bool,
}

impl ContextRenderOptions {
    pub const MAIN_AGENT: Self = Self {
        include_subagent_manifest: true,
    };

    pub const SUBAGENT: Self = Self {
        include_subagent_manifest: false,
    };
}

pub fn resolve_preset_context_items(
    preset: &Preset,
    contexts: &[ContextFragment],
) -> Result<Vec<ResolvedContextItem>, ContextItemResolveError> {
    let contexts_by_id: HashMap<Uuid, &ContextFragment> = contexts
        .iter()
        .map(|context| (context.context_id, context))
        .collect();
    let compositions = resolved_compositions(preset);
    let mut items = Vec::with_capacity(compositions.len());

    for composition in compositions {
        let context = contexts_by_id.get(&composition.context_id).ok_or(
            ContextItemResolveError::MissingContext(composition.context_id),
        )?;
        let content = select_context_content(context, &composition).map_err(|reason| {
            ContextItemResolveError::InvalidSelection {
                context_id: composition.context_id,
                source_ref: composition.source_ref.clone(),
                reason,
            }
        })?;

        items.push(ResolvedContextItem {
            context_id: context.context_id,
            title: context.title.clone(),
            source_ref: composition.source_ref,
            file_path: context.file_path.clone(),
            vault_scope: context.vault_scope,
            selection: composition.selection,
            content,
        });
    }

    Ok(items)
}

pub fn resolve_subagent_context_items(
    preset: &Preset,
    contexts: &[ContextFragment],
    role_id: &str,
) -> Result<Vec<ResolvedContextItem>, SubagentContextResolveError> {
    let manifest = preset
        .subagent_manifest
        .as_ref()
        .ok_or(SubagentContextResolveError::MissingManifest)?;
    let role = manifest
        .roles
        .iter()
        .find(|role| role.role_id == role_id)
        .ok_or_else(|| SubagentContextResolveError::MissingRole(role_id.to_string()))?;
    let resolved_items = resolve_preset_context_items(preset, contexts)
        .map_err(SubagentContextResolveError::ContextItem)?;
    let mut assigned_items = Vec::with_capacity(role.assigned_contexts.len());

    for context_ref in &role.assigned_contexts {
        let item = resolved_items
            .iter()
            .find(|item| context_refs_match(&item.source_ref, context_ref))
            .ok_or_else(|| SubagentContextResolveError::MissingAssignedContext {
                role_id: role_id.to_string(),
                context_ref: context_ref.clone(),
            })?;
        assigned_items.push(item.clone());
    }

    Ok(assigned_items)
}

pub fn assemble_combined_context_output(
    title: &str,
    preset: &Preset,
    resolved_items: &[ResolvedContextItem],
) -> String {
    assemble_context_output_with_options(
        title,
        preset,
        resolved_items,
        ContextRenderOptions::MAIN_AGENT,
    )
}

pub fn assemble_subagent_context_output(
    title: &str,
    preset: &Preset,
    resolved_items: &[ResolvedContextItem],
) -> String {
    assemble_context_output_with_options(
        title,
        preset,
        resolved_items,
        ContextRenderOptions::SUBAGENT,
    )
}

pub fn assemble_context_output_with_options(
    title: &str,
    preset: &Preset,
    resolved_items: &[ResolvedContextItem],
    options: ContextRenderOptions,
) -> String {
    let mut output = String::new();
    output.push_str(&format!("# {title}\n\n"));
    output.push_str(&format!("Preset: {}\n", preset.preset_name));
    output.push_str(&format!("Preset ID: {}\n", preset.preset_id));
    output.push_str(&format!(
        "Target CLI: {}\n",
        cli_target_label(preset.preset_target_cli)
    ));
    output.push_str(&format!(
        "Working Directory: {}\n",
        preset.preset_working_dir.display()
    ));
    if let Some(model) = &preset.preset_model {
        output.push_str(&format!("Model: {model}\n"));
    }
    if options.include_subagent_manifest {
        if let Some(manifest) = &preset.subagent_manifest {
            append_subagent_manifest_output(&mut output, manifest);
        }
    }

    for item in resolved_items {
        output.push_str(COMBINED_CONTEXT_ITEM_SEPARATOR);
        append_resolved_item_output(&mut output, item);
    }

    output.push('\n');
    output
}

fn context_refs_match(left: &str, right: &str) -> bool {
    normalize_context_ref(left) == normalize_context_ref(right)
}

fn normalize_context_ref(value: &str) -> String {
    value.trim().replace('\\', "/").to_ascii_lowercase()
}

fn append_subagent_manifest_output(output: &mut String, manifest: &SubagentManifest) {
    output.push_str("\n## Delegation Manifest\n\n");
    output.push_str("```ctx-subagent-manifest\n");
    output.push_str(
        &serde_json::to_string_pretty(manifest)
            .expect("validated SubagentManifest should serialize for composition output"),
    );
    output.push_str("\n```\n");
}

fn append_resolved_item_output(output: &mut String, item: &ResolvedContextItem) {
    output.push_str(&format!("## {}\n\n", item.title));
    output.push_str("```ctx-metadata\n");
    output.push_str(&format!("context_id: {}\n", item.context_id));
    output.push_str(&format!("source_ref: {}\n", item.source_ref));
    output.push_str(&format!("file_path: {}\n", item.file_path.display()));
    output.push_str(&format!(
        "vault_scope: {}\n",
        vault_scope_label(item.vault_scope)
    ));
    output.push_str(&format!(
        "selection: {}\n",
        selection_label(&item.selection)
    ));
    output.push_str("```\n\n");
    output.push_str(item.content.trim_end_matches('\n'));
    output.push('\n');
}

fn cli_target_label(target: CliTarget) -> &'static str {
    match target {
        CliTarget::Claude => "claude",
        CliTarget::Codex => "codex",
    }
}

fn vault_scope_label(scope: super::models::VaultScope) -> &'static str {
    match scope {
        super::models::VaultScope::Global => "global",
        super::models::VaultScope::Local => "local",
    }
}

fn selection_label(selection: &PresetContextSelection) -> String {
    match selection.selection_kind {
        PresetContextSelectionKind::WholeFile => "whole-file".to_string(),
        PresetContextSelectionKind::Heading => {
            format!(
                "heading:{}:include_children={}",
                selection.heading.as_deref().unwrap_or_default(),
                selection.include_children
            )
        }
        PresetContextSelectionKind::LineRange => {
            format!(
                "lines:{}-{}",
                selection
                    .line_start
                    .map(|value| value.to_string())
                    .unwrap_or_default(),
                selection
                    .line_end
                    .map(|value| value.to_string())
                    .unwrap_or_default()
            )
        }
        PresetContextSelectionKind::Anchor => {
            format!(
                "anchor:{}:include_children={}",
                selection.anchor.as_deref().unwrap_or_default(),
                selection.include_children
            )
        }
    }
}

fn resolved_compositions(preset: &Preset) -> Vec<PresetContextComposition> {
    if !preset.preset_context_composition.is_empty() {
        let mut compositions = preset.preset_context_composition.clone();
        compositions.sort_by(compare_compositions_by_resolved_item_order);
        return compositions;
    }

    let mut compositions: Vec<_> = preset
        .preset_contexts
        .iter()
        .enumerate()
        .map(|(order, context_id)| PresetContextComposition {
            context_id: *context_id,
            order,
            source_ref: context_id.to_string(),
            required: true,
            selection: PresetContextSelection::default(),
        })
        .collect();
    compositions.sort_by(compare_compositions_by_resolved_item_order);
    compositions
}

fn compare_compositions_by_resolved_item_order(
    left: &PresetContextComposition,
    right: &PresetContextComposition,
) -> Ordering {
    left.order
        .cmp(&right.order)
        .then_with(|| compare_source_refs(&left.source_ref, &right.source_ref))
        .then_with(|| left.context_id.cmp(&right.context_id))
        .then_with(|| compare_selections(&left.selection, &right.selection))
        .then_with(|| left.required.cmp(&right.required))
}

fn compare_source_refs(left: &str, right: &str) -> Ordering {
    left.trim()
        .replace('\\', "/")
        .to_ascii_lowercase()
        .cmp(&right.trim().replace('\\', "/").to_ascii_lowercase())
        .then_with(|| left.cmp(right))
}

fn compare_selections(left: &PresetContextSelection, right: &PresetContextSelection) -> Ordering {
    selection_kind_order(left.selection_kind)
        .cmp(&selection_kind_order(right.selection_kind))
        .then_with(|| left.heading.cmp(&right.heading))
        .then_with(|| left.anchor.cmp(&right.anchor))
        .then_with(|| left.line_start.cmp(&right.line_start))
        .then_with(|| left.line_end.cmp(&right.line_end))
        .then_with(|| left.include_children.cmp(&right.include_children))
}

fn selection_kind_order(selection_kind: PresetContextSelectionKind) -> u8 {
    match selection_kind {
        PresetContextSelectionKind::WholeFile => 0,
        PresetContextSelectionKind::Heading => 1,
        PresetContextSelectionKind::LineRange => 2,
        PresetContextSelectionKind::Anchor => 3,
    }
}

fn select_context_content(
    context: &ContextFragment,
    composition: &PresetContextComposition,
) -> Result<String, String> {
    match composition.selection.selection_kind {
        PresetContextSelectionKind::WholeFile => Ok(context.content.clone()),
        PresetContextSelectionKind::LineRange => {
            select_line_range(&context.content, &composition.selection)
        }
        PresetContextSelectionKind::Heading => {
            select_heading(&context.content, &composition.selection)
        }
        PresetContextSelectionKind::Anchor => {
            select_anchor(&context.content, &composition.selection)
        }
    }
}

fn select_line_range(content: &str, selection: &PresetContextSelection) -> Result<String, String> {
    let lines: Vec<&str> = content.split_inclusive('\n').collect();
    let line_count = lines.len();
    let start = selection
        .line_start
        .ok_or_else(|| "line-range selection requires line_start".to_string())?;
    let end = selection
        .line_end
        .ok_or_else(|| "line-range selection requires line_end".to_string())?;

    if start == 0 {
        return Err("line_start is 1-based and must be greater than zero".to_string());
    }
    if end < start {
        return Err("line_end must be greater than or equal to line_start".to_string());
    }
    if end as usize > line_count {
        return Err(format!(
            "line_end {end} exceeds context length of {line_count} lines"
        ));
    }

    Ok(lines[(start as usize - 1)..end as usize].concat())
}

fn select_heading(content: &str, selection: &PresetContextSelection) -> Result<String, String> {
    let heading = selection
        .heading
        .as_deref()
        .ok_or_else(|| "heading selection requires heading".to_string())?;
    let lines: Vec<&str> = content.split_inclusive('\n').collect();
    let Some((start_index, level)) = lines.iter().enumerate().find_map(|(index, line)| {
        parse_markdown_heading(line)
            .and_then(|(level, title)| titles_match(title, heading).then_some((index, level)))
    }) else {
        return Err(format!("heading not found: {heading}"));
    };

    let end_index = lines
        .iter()
        .enumerate()
        .skip(start_index + 1)
        .find_map(|(index, line)| {
            parse_markdown_heading(line).and_then(|(next_level, _)| {
                let boundary = if selection.include_children {
                    next_level <= level
                } else {
                    true
                };
                boundary.then_some(index)
            })
        })
        .unwrap_or(lines.len());

    Ok(lines[start_index..end_index].concat())
}

fn select_anchor(content: &str, selection: &PresetContextSelection) -> Result<String, String> {
    let anchor = selection
        .anchor
        .as_deref()
        .ok_or_else(|| "anchor selection requires anchor".to_string())?;
    let lines: Vec<&str> = content.split_inclusive('\n').collect();

    for (index, line) in lines.iter().enumerate() {
        if let Some((_, title)) = parse_markdown_heading(line) {
            if slugify_heading(title) == slugify_heading(anchor) {
                let heading_selection = PresetContextSelection {
                    selection_kind: PresetContextSelectionKind::Heading,
                    heading: Some(title.to_string()),
                    anchor: None,
                    line_start: None,
                    line_end: None,
                    include_children: selection.include_children,
                };
                return select_heading(content, &heading_selection);
            }
        }

        if line_has_anchor_marker(line, anchor) {
            let start_index = index + 1;
            let end_index = lines
                .iter()
                .enumerate()
                .skip(start_index)
                .find_map(|(boundary_index, candidate)| {
                    (is_any_anchor_marker(candidate) || parse_markdown_heading(candidate).is_some())
                        .then_some(boundary_index)
                })
                .unwrap_or(lines.len());
            return Ok(lines[start_index..end_index].concat());
        }
    }

    Err(format!("anchor not found: {anchor}"))
}

fn parse_markdown_heading(line: &str) -> Option<(usize, &str)> {
    let trimmed = line.trim_start();
    let hashes = trimmed
        .chars()
        .take_while(|character| *character == '#')
        .count();
    if !(1..=6).contains(&hashes) {
        return None;
    }
    let after_hashes = trimmed.get(hashes..)?;
    if !after_hashes
        .chars()
        .next()
        .map(|character| character.is_whitespace())
        .unwrap_or(false)
    {
        return None;
    }
    let title = after_hashes.trim().trim_end_matches('#').trim();
    (!title.is_empty()).then_some((hashes, title))
}

fn titles_match(left: &str, right: &str) -> bool {
    left.trim().eq_ignore_ascii_case(right.trim())
}

fn slugify_heading(value: &str) -> String {
    let mut slug = String::new();
    let mut previous_dash = false;

    for character in value.trim().chars().flat_map(char::to_lowercase) {
        if character.is_ascii_alphanumeric() {
            slug.push(character);
            previous_dash = false;
        } else if !previous_dash && !slug.is_empty() {
            slug.push('-');
            previous_dash = true;
        }
    }

    slug.trim_matches('-').to_string()
}

fn line_has_anchor_marker(line: &str, anchor: &str) -> bool {
    let normalized = normalize_anchor_name(anchor);
    anchor_marker_value(line)
        .map(|value| normalize_anchor_name(value) == normalized)
        .unwrap_or(false)
}

fn is_any_anchor_marker(line: &str) -> bool {
    anchor_marker_value(line).is_some()
}

fn anchor_marker_value(line: &str) -> Option<&str> {
    let trimmed = line.trim();
    if trimmed.starts_with("<!--") && trimmed.ends_with("-->") {
        let inner = trimmed
            .trim_start_matches("<!--")
            .trim_end_matches("-->")
            .trim();
        for prefix in ["ctx:anchor", "anchor"] {
            if let Some(value) = inner.strip_prefix(prefix) {
                return Some(value.trim_start_matches(':').trim());
            }
        }
    }

    for prefix in ["<a id=\"", "<a name=\""] {
        if let Some(rest) = trimmed.strip_prefix(prefix) {
            if let Some((value, _)) = rest.split_once('"') {
                return Some(value);
            }
        }
    }

    None
}

fn normalize_anchor_name(value: &str) -> String {
    value.trim().to_ascii_lowercase()
}

pub fn detect_residual_codex_agents_md_markers(
    working_dir: &Path,
) -> Result<Option<CodexResidualMarkers>, CodexInjectionError> {
    let agents_md_path = working_dir.join(AGENTS_MD_FILE_NAME);
    let existing_content = match fs::read_to_string(&agents_md_path) {
        Ok(content) => content,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(CodexInjectionError::Io(format!(
                "failed to read {} before Codex residual marker detection: {error}",
                agents_md_path.display()
            )));
        }
    };

    match find_managed_section_bounds(&existing_content) {
        Ok(Some(_)) => Ok(Some(CodexResidualMarkers {
            path: agents_md_path,
            reason: "found existing managed ctx marker section".to_string(),
        })),
        Ok(None) => Ok(None),
        Err(error) => Ok(Some(CodexResidualMarkers {
            path: agents_md_path,
            reason: error.to_string(),
        })),
    }
}

pub fn assemble_claude_prompt_file(
    preset: &Preset,
    contexts: &[ContextFragment],
) -> Result<ClaudePromptFile, PromptAssemblyError> {
    if preset.preset_target_cli != CliTarget::Claude {
        return Err(PromptAssemblyError::UnsupportedTarget(
            preset.preset_target_cli,
        ));
    }

    let resolved_items =
        resolve_preset_context_items(preset, contexts).map_err(|error| match error {
            ContextItemResolveError::MissingContext(context_id) => {
                PromptAssemblyError::MissingContext(context_id)
            }
            ContextItemResolveError::InvalidSelection { .. } => {
                PromptAssemblyError::InvalidSelection(error.to_string())
            }
        })?;

    let prompt =
        assemble_combined_context_output("CTX Claude Session Context", preset, &resolved_items);

    let prompt_dir = std::env::temp_dir().join("ctx").join("claude-prompts");
    fs::create_dir_all(&prompt_dir).map_err(|error| PromptAssemblyError::Io(error.to_string()))?;

    let prompt_path = prompt_dir.join(format!(
        "{}-{}.md",
        sanitize_filename(&preset.preset_name),
        Uuid::new_v4()
    ));

    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&prompt_path)
        .map_err(|error| PromptAssemblyError::Io(error.to_string()))?;

    file.write_all(prompt.as_bytes())
        .map_err(|error| PromptAssemblyError::Io(error.to_string()))?;

    Ok(ClaudePromptFile {
        path: prompt_path,
        selected_context_ids: resolved_items.iter().map(|item| item.context_id).collect(),
    })
}

pub fn assemble_codex_agents_md_payload(
    preset: &Preset,
    contexts: &[ContextFragment],
) -> Result<String, CodexInjectionError> {
    if preset.preset_target_cli != CliTarget::Codex {
        return Err(CodexInjectionError::UnsupportedTarget(
            preset.preset_target_cli,
        ));
    }

    let resolved_items =
        resolve_preset_context_items(preset, contexts).map_err(|error| match error {
            ContextItemResolveError::MissingContext(context_id) => {
                CodexInjectionError::MissingContext(context_id)
            }
            ContextItemResolveError::InvalidSelection { .. } => {
                CodexInjectionError::InvalidSelection(error.to_string())
            }
        })?;

    Ok(assemble_combined_context_output(
        "CTX Codex Session Context",
        preset,
        &resolved_items,
    ))
}

pub fn inject_codex_agents_md(
    preset: &Preset,
    contexts: &[ContextFragment],
) -> Result<CodexAgentsMdInjection, CodexInjectionError> {
    if preset.preset_target_cli != CliTarget::Codex {
        return Err(CodexInjectionError::UnsupportedTarget(
            preset.preset_target_cli,
        ));
    }

    let selected_context_ids = resolve_preset_context_items(preset, contexts)
        .map_err(|error| match error {
            ContextItemResolveError::MissingContext(context_id) => {
                CodexInjectionError::MissingContext(context_id)
            }
            ContextItemResolveError::InvalidSelection { .. } => {
                CodexInjectionError::InvalidSelection(error.to_string())
            }
        })?
        .into_iter()
        .map(|item| item.context_id)
        .collect();
    let managed_content = assemble_codex_agents_md_payload(preset, contexts)?;
    let agents_md_path = preset.preset_working_dir.join(AGENTS_MD_FILE_NAME);
    let had_existing_file = agents_md_path.exists();
    let existing_content = if had_existing_file {
        fs::read_to_string(&agents_md_path).map_err(|error| {
            CodexInjectionError::Io(format!(
                "failed to read {} before Codex injection: {error}",
                agents_md_path.display()
            ))
        })?
    } else {
        String::new()
    };

    let next_content = replace_agents_md_managed_section(&existing_content, &managed_content)
        .map_err(CodexInjectionError::SectionReplace)?;

    fs::write(&agents_md_path, next_content).map_err(|error| {
        CodexInjectionError::Io(format!(
            "failed to write managed ctx block to {}: {error}",
            agents_md_path.display()
        ))
    })?;

    Ok(CodexAgentsMdInjection {
        path: agents_md_path,
        selected_context_ids,
        managed_content,
        had_existing_file,
    })
}

pub fn cleanup_codex_agents_md(
    injection: &CodexAgentsMdInjection,
) -> Result<(), CodexInjectionError> {
    let existing_content = match fs::read_to_string(&injection.path) {
        Ok(content) => content,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(CodexInjectionError::Io(format!(
                "failed to read {} before Codex cleanup: {error}",
                injection.path.display()
            )));
        }
    };

    let cleaned_content = remove_agents_md_managed_section(&existing_content)
        .map_err(CodexInjectionError::SectionReplace)?;

    if !injection.had_existing_file && cleaned_content.trim().is_empty() {
        fs::remove_file(&injection.path).map_err(|error| {
            CodexInjectionError::Io(format!(
                "failed to remove temporary {} after Codex cleanup: {error}",
                injection.path.display()
            ))
        })?;
    } else {
        fs::write(&injection.path, cleaned_content).map_err(|error| {
            CodexInjectionError::Io(format!(
                "failed to remove managed ctx block from {}: {error}",
                injection.path.display()
            ))
        })?;
    }

    Ok(())
}

pub fn default_wrapper_state_dir() -> PathBuf {
    std::env::temp_dir()
        .join("ctx")
        .join(WRAPPER_STATE_DIR_NAME)
}

pub fn wrapper_state_path(state_dir: &Path, session_id: Uuid) -> PathBuf {
    state_dir.join(format!("{session_id}.json"))
}

pub fn write_transient_wrapper_state(
    state_dir: &Path,
    state: &TransientWrapperState,
) -> Result<PathBuf, WrapperStateError> {
    fs::create_dir_all(state_dir).map_err(|error| {
        WrapperStateError::Io(format!(
            "failed to create ctx wrapper state directory {}: {error}",
            state_dir.display()
        ))
    })?;

    let state_path = wrapper_state_path(state_dir, state.session_id);
    let encoded = serde_json::to_string_pretty(state).map_err(|error| {
        WrapperStateError::Json(format!(
            "failed to serialize ctx wrapper state {}: {error}",
            state_path.display()
        ))
    })?;

    fs::write(&state_path, encoded).map_err(|error| {
        WrapperStateError::Io(format!(
            "failed to write ctx wrapper state {}: {error}",
            state_path.display()
        ))
    })?;

    Ok(state_path)
}

pub fn remove_transient_wrapper_state_file(state_path: &Path) -> Result<(), WrapperStateError> {
    match fs::remove_file(state_path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(WrapperStateError::Io(format!(
            "failed to remove ctx wrapper state {}: {error}",
            state_path.display()
        ))),
    }
}

pub fn cleanup_stale_wrapper_state<F>(
    state_dir: &Path,
    is_process_active: F,
) -> WrapperStateCleanupReport
where
    F: Fn(u32) -> bool,
{
    let mut report = WrapperStateCleanupReport::empty();
    let entries = match fs::read_dir(state_dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return report,
        Err(error) => {
            report.errors.push(format!(
                "failed to read ctx wrapper state directory {}: {error}",
                state_dir.display()
            ));
            return report;
        }
    };

    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                report
                    .errors
                    .push(format!("failed to read ctx wrapper state entry: {error}"));
                continue;
            }
        };
        let state_path = entry.path();

        if state_path
            .extension()
            .and_then(|extension| extension.to_str())
            != Some("json")
        {
            continue;
        }

        report.scanned += 1;
        let state_content = match fs::read_to_string(&state_path) {
            Ok(content) => content,
            Err(error) => {
                report.errors.push(format!(
                    "failed to read ctx wrapper state {}: {error}",
                    state_path.display()
                ));
                continue;
            }
        };
        let state: TransientWrapperState = match serde_json::from_str(&state_content) {
            Ok(state) => state,
            Err(error) => {
                report.errors.push(format!(
                    "failed to parse ctx wrapper state {}: {error}",
                    state_path.display()
                ));
                continue;
            }
        };

        if is_process_active(state.child_pid) {
            report.skipped_active += 1;
            continue;
        }

        if let Err(error) = cleanup_transient_wrapper_artifacts(&state) {
            report.errors.push(error);
            continue;
        }

        if let Err(error) = remove_transient_wrapper_state_file(&state_path) {
            report.errors.push(error.to_string());
            continue;
        }

        report.cleaned += 1;
    }

    report
}

pub fn cleanup_transient_wrapper_artifacts(state: &TransientWrapperState) -> Result<(), String> {
    if let Some(prompt_file) = &state.claude_prompt_file {
        match fs::remove_file(prompt_file) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(format!(
                    "failed to remove stale Claude prompt file {}: {error}",
                    prompt_file.display()
                ));
            }
        }
    }

    if let Some(agents_md_path) = &state.codex_agents_md_path {
        let injection = CodexAgentsMdInjection {
            path: agents_md_path.clone(),
            selected_context_ids: Vec::new(),
            managed_content: String::new(),
            had_existing_file: state.codex_had_existing_agents_md,
        };
        cleanup_codex_agents_md(&injection).map_err(|error| error.to_string())?;
    }

    Ok(())
}

pub fn cleanup_residual_codex_agents_md_markers(
    working_dir: &Path,
) -> Result<bool, CodexInjectionError> {
    let agents_md_path = working_dir.join(AGENTS_MD_FILE_NAME);
    let existing_content = match fs::read_to_string(&agents_md_path) {
        Ok(content) => content,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => {
            return Err(CodexInjectionError::Io(format!(
                "failed to read {} before Codex residual marker cleanup: {error}",
                agents_md_path.display()
            )));
        }
    };

    if find_managed_section_bounds(&existing_content)
        .map_err(CodexInjectionError::SectionReplace)?
        .is_none()
    {
        return Ok(false);
    }

    let cleaned_content = remove_agents_md_managed_section(&existing_content)
        .map_err(CodexInjectionError::SectionReplace)?;

    if cleaned_content.trim().is_empty() {
        fs::remove_file(&agents_md_path).map_err(|error| {
            CodexInjectionError::Io(format!(
                "failed to remove stale temporary {} after marker cleanup: {error}",
                agents_md_path.display()
            ))
        })?;
    } else {
        fs::write(&agents_md_path, cleaned_content).map_err(|error| {
            CodexInjectionError::Io(format!(
                "failed to clean stale managed ctx block from {}: {error}",
                agents_md_path.display()
            ))
        })?;
    }

    Ok(true)
}

fn sanitize_filename(value: &str) -> String {
    let sanitized: String = value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_') {
                character.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();

    let trimmed = sanitized.trim_matches('-');
    if trimmed.is_empty() {
        "preset".to_string()
    } else {
        trimmed.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{
        Classification, ClassificationStatus, HandoffConstraints, SubagentRole,
        SubagentSpawnGuidance, VaultScope,
    };
    use crate::presets::validate_subagent_manifest;
    use std::{
        path::{Path, PathBuf},
        time::{SystemTime, UNIX_EPOCH},
    };

    #[test]
    fn assemble_claude_prompt_file_writes_selected_contexts_in_preset_order() {
        let first = context("First", "first body");
        let second = context("Second", "second body");
        let ignored = context("Ignored", "ignored body");
        let preset = Preset {
            preset_id: Uuid::new_v4(),
            preset_name: "Daily Driver".to_string(),
            preset_contexts: vec![second.context_id, first.context_id],
            preset_metadata: Default::default(),
            preset_context_composition: Vec::new(),
            preset_target_cli: CliTarget::Claude,
            preset_working_dir: PathBuf::from("/workspace"),
            preset_model: None,
            subagent_manifest: None,
        };

        let prompt_file =
            assemble_claude_prompt_file(&preset, &[first.clone(), ignored, second.clone()])
                .expect("prompt file should be assembled");
        let prompt = fs::read_to_string(&prompt_file.path).expect("prompt file should be readable");

        assert_eq!(
            prompt_file.selected_context_ids,
            vec![second.context_id, first.context_id]
        );
        assert!(prompt.contains("# CTX Claude Session Context"));
        assert!(prompt.contains("Preset: Daily Driver"));
        assert!(prompt.contains("## Second"));
        assert!(prompt.contains("second body"));
        assert!(prompt.contains("## First"));
        assert!(prompt.contains("first body"));
        assert!(!prompt.contains("ignored body"));
        assert!(
            prompt.find("## Second").expect("second section exists")
                < prompt.find("## First").expect("first section exists")
        );

        fs::remove_file(prompt_file.path).expect("test prompt file should be removable");
    }

    #[test]
    fn assemble_claude_prompt_file_includes_delegation_manifest_in_main_context() {
        let main_context = context("Main Agent Notes", "Use the preset delegation rules.\n");
        let mut preset = Preset {
            preset_id: Uuid::new_v4(),
            preset_name: "Delegated Main Agent".to_string(),
            preset_contexts: vec![main_context.context_id],
            preset_metadata: Default::default(),
            preset_context_composition: Vec::new(),
            preset_target_cli: CliTarget::Claude,
            preset_working_dir: PathBuf::from("/workspace"),
            preset_model: Some("claude-sonnet".to_string()),
            subagent_manifest: None,
        };
        preset.subagent_manifest = Some(
            validate_subagent_manifest(SubagentManifest {
                manifest_version: Some("1".to_string()),
                roles: vec![SubagentRole {
                    role_id: "reviewer".to_string(),
                    role_name: "Reviewer".to_string(),
                    role: "Code review subagent".to_string(),
                    capabilities: vec!["review changed files".to_string()],
                    constraints: vec!["Report findings before summary.".to_string()],
                    metadata: Default::default(),
                    description: Some("Reviews implementation risk.".to_string()),
                    assigned_contexts: vec!["subagents/reviewer.md".to_string()],
                    spawn_instructions: vec!["Inspect the main agent changes.".to_string()],
                    spawn_guidance: SubagentSpawnGuidance {
                        select_when: vec![
                            "Use after implementation changes are ready for review.".to_string()
                        ],
                        avoid_when: vec!["Avoid when no code or test files changed.".to_string()],
                        delegation_prompt: Some(
                            "Review changed files and return findings first.".to_string(),
                        ),
                    },
                    handoff_targets: Vec::new(),
                    model: Some("gpt-5.3-codex".to_string()),
                }],
                handoff_constraints: HandoffConstraints {
                    require_summary: true,
                    require_changed_files: true,
                    require_open_questions: true,
                    max_parallel_subagents: Some(1),
                    allowed_handoff_targets: Vec::new(),
                    blocked_handoff_targets: Vec::new(),
                    handoff_prompt_template: Some(
                        "Return findings before implementation notes.".to_string(),
                    ),
                },
            })
            .expect("manifest fixture should satisfy delegation validation"),
        );

        let prompt_file = assemble_claude_prompt_file(&preset, &[main_context])
            .expect("prompt file should include the main agent context");
        let prompt = fs::read_to_string(&prompt_file.path).expect("prompt file should be readable");

        assert!(prompt.contains("# CTX Claude Session Context"));
        assert!(prompt.contains("Preset: Delegated Main Agent"));
        assert!(prompt.contains("## Delegation Manifest\n"));
        assert!(prompt.contains("```ctx-subagent-manifest\n"));
        assert!(prompt.contains("\"id\": \"reviewer\""));
        assert!(prompt.contains("\"assigned_contexts\": ["));
        assert!(prompt.contains("\"subagents/reviewer.md\""));
        assert!(
            prompt
                .find("## Delegation Manifest")
                .expect("delegation manifest should render")
                < prompt
                    .find("## Main Agent Notes")
                    .expect("main context should render")
        );

        fs::remove_file(prompt_file.path).expect("test prompt file should be removable");
    }

    #[test]
    fn assemble_claude_prompt_file_rejects_codex_presets() {
        let preset = Preset {
            preset_id: Uuid::new_v4(),
            preset_name: "Codex".to_string(),
            preset_contexts: Vec::new(),
            preset_metadata: Default::default(),
            preset_context_composition: Vec::new(),
            preset_target_cli: CliTarget::Codex,
            preset_working_dir: PathBuf::from("/workspace"),
            preset_model: None,
            subagent_manifest: None,
        };

        assert_eq!(
            assemble_claude_prompt_file(&preset, &[]),
            Err(PromptAssemblyError::UnsupportedTarget(CliTarget::Codex))
        );
    }

    #[test]
    fn assemble_claude_prompt_file_reports_missing_contexts() {
        let missing_context_id = Uuid::new_v4();
        let preset = Preset {
            preset_id: Uuid::new_v4(),
            preset_name: "Missing".to_string(),
            preset_contexts: vec![missing_context_id],
            preset_metadata: Default::default(),
            preset_context_composition: Vec::new(),
            preset_target_cli: CliTarget::Claude,
            preset_working_dir: PathBuf::from("/workspace"),
            preset_model: None,
            subagent_manifest: None,
        };

        assert_eq!(
            assemble_claude_prompt_file(&preset, &[]),
            Err(PromptAssemblyError::MissingContext(missing_context_id))
        );
    }

    #[test]
    fn exposes_agents_md_section_marker_contract() {
        assert_eq!(AGENTS_MD_FILE_NAME, "AGENTS.md");
        assert_eq!(CTX_START_MARKER, "<!-- [ctx:start] -->");
        assert_eq!(CTX_END_MARKER, "<!-- [ctx:end] -->");
    }

    #[test]
    fn builds_canonical_agents_md_managed_section() {
        assert_eq!(
            build_agents_md_managed_section("\n# Preset\n\nUse shared rules.\n"),
            "<!-- [ctx:start] -->\n# Preset\n\nUse shared rules.\n<!-- [ctx:end] -->"
        );
    }

    #[test]
    fn managed_section_wraps_injected_context_block_with_markers() {
        let injected_context_block = "# CTX Codex Session Context\n\nPreset: Review\n\n## Shared Rules\n\nUse repo conventions.\n\n## Agent Notes\n\nCheck focused tests.\n";

        let section = build_agents_md_managed_section(injected_context_block);

        assert_eq!(section.matches(CTX_START_MARKER).count(), 1);
        assert_eq!(section.matches(CTX_END_MARKER).count(), 1);
        assert!(section.starts_with(&format!("{CTX_START_MARKER}\n")));
        assert!(section.ends_with(&format!("\n{CTX_END_MARKER}")));
        assert_eq!(
            section,
            format!(
                "{CTX_START_MARKER}\n{}\n{CTX_END_MARKER}",
                injected_context_block.trim_matches('\n')
            )
        );
    }

    #[test]
    fn appends_agents_md_section_when_no_managed_block_exists() {
        let actual = replace_agents_md_managed_section("# Project\n\nKeep this.", "Injected")
            .expect("section should append");

        assert_eq!(
            actual,
            "# Project\n\nKeep this.\n\n<!-- [ctx:start] -->\nInjected\n<!-- [ctx:end] -->\n"
        );
    }

    #[test]
    fn replaces_agents_md_section_and_preserves_outer_content() {
        let existing = "# Project\n\nKeep this.\n\n<!-- [ctx:start] -->\nOld\n<!-- [ctx:end] -->\n\n## Notes\nDo not touch.";
        let actual = replace_agents_md_managed_section(existing, "New\n\nContext")
            .expect("section should replace");

        assert_eq!(
            actual,
            "# Project\n\nKeep this.\n\n<!-- [ctx:start] -->\nNew\n\nContext\n<!-- [ctx:end] -->\n\n## Notes\nDo not touch."
        );
    }

    #[test]
    fn preserves_exact_agents_md_prefix_and_suffix_bytes() {
        let existing = "Before\n\n\n<!-- [ctx:start] -->\nOld\n<!-- [ctx:end] -->\n\n\nAfter\n";
        let actual =
            replace_agents_md_managed_section(existing, "New").expect("section should replace");

        assert_eq!(
            actual,
            "Before\n\n\n<!-- [ctx:start] -->\nNew\n<!-- [ctx:end] -->\n\n\nAfter\n"
        );
    }

    #[test]
    fn agents_md_replacement_is_idempotent_for_same_content() {
        let once = replace_agents_md_managed_section("Intro", "Injected")
            .expect("first replacement should succeed");
        let twice = replace_agents_md_managed_section(&once, "Injected")
            .expect("second replacement should succeed");

        assert_eq!(twice, once);
    }

    #[test]
    fn removes_agents_md_section_and_preserves_outer_content() {
        let existing = "Before\n\n<!-- [ctx:start] -->\nInjected\n<!-- [ctx:end] -->\n\nAfter";
        let actual =
            remove_agents_md_managed_section(existing).expect("managed section should remove");

        assert_eq!(actual, "Before\n\n\n\nAfter");
    }

    #[test]
    fn removes_only_managed_agents_md_section_and_keeps_user_content_exact() {
        let existing = concat!(
            "# Project Agents\n",
            "\n",
            "Manual rule before ctx block.\n",
            "This user note mentions ctx:start but is not a marker.\n",
            "\n",
            "<!-- [ctx:start] -->\n",
            "# CTX Codex Session Context\n",
            "\n",
            "Preset: Temporary\n",
            "\n",
            "## Injected Context\n",
            "\n",
            "Remove this managed payload.\n",
            "<!-- [ctx:end] -->",
            "\n",
            "\n",
            "Manual rule after ctx block.\n",
            "Keep this trailing user content.\n",
        );

        let actual =
            remove_agents_md_managed_section(existing).expect("managed section should remove");

        assert_eq!(
            actual,
            concat!(
                "# Project Agents\n",
                "\n",
                "Manual rule before ctx block.\n",
                "This user note mentions ctx:start but is not a marker.\n",
                "\n",
                "\n",
                "\n",
                "Manual rule after ctx block.\n",
                "Keep this trailing user content.\n",
            )
        );
        assert!(!actual.contains("# CTX Codex Session Context"));
        assert!(!actual.contains("Remove this managed payload."));
        assert!(!actual.contains(CTX_START_MARKER));
        assert!(!actual.contains(CTX_END_MARKER));
    }

    #[test]
    fn rejects_malformed_agents_md_marker_pairs() {
        assert_eq!(
            replace_agents_md_managed_section("Before\n<!-- [ctx:start] -->\nMissing end", "New")
                .unwrap_err(),
            SectionReplaceError::MissingEndMarker
        );
        assert_eq!(
            replace_agents_md_managed_section("Before\n<!-- [ctx:end] -->", "New").unwrap_err(),
            SectionReplaceError::MissingStartMarker
        );
        assert_eq!(
            replace_agents_md_managed_section(
                "<!-- [ctx:end] -->\nMiddle\n<!-- [ctx:start] -->",
                "New"
            )
            .unwrap_err(),
            SectionReplaceError::EndMarkerBeforeStartMarker
        );
    }

    #[test]
    fn rejects_multiple_agents_md_managed_sections() {
        let existing = "<!-- [ctx:start] -->\nOne\n<!-- [ctx:end] -->\n<!-- [ctx:start] -->\nTwo\n<!-- [ctx:end] -->";

        assert_eq!(
            replace_agents_md_managed_section(existing, "New").unwrap_err(),
            SectionReplaceError::MultipleManagedSections
        );
    }

    #[test]
    fn residual_marker_detection_reports_existing_managed_section() {
        let working_dir = test_working_dir("detects-residual-section");
        fs::write(
            working_dir.join(AGENTS_MD_FILE_NAME),
            "# Project\n\n<!-- [ctx:start] -->\nOld session\n<!-- [ctx:end] -->\n",
        )
        .expect("fixture should be written");

        let residual = detect_residual_codex_agents_md_markers(&working_dir)
            .expect("detection should succeed")
            .expect("residual marker should be reported");

        assert_eq!(residual.path, working_dir.join(AGENTS_MD_FILE_NAME));
        assert_eq!(residual.reason, "found existing managed ctx marker section");

        remove_test_working_dir(working_dir);
    }

    #[test]
    fn residual_marker_detection_ignores_plain_agents_md() {
        let working_dir = test_working_dir("ignores-plain-agents-md");
        fs::write(
            working_dir.join(AGENTS_MD_FILE_NAME),
            "# Project\n\nManual project guidance.\n",
        )
        .expect("fixture should be written");

        assert_eq!(
            detect_residual_codex_agents_md_markers(&working_dir)
                .expect("detection should succeed"),
            None
        );

        remove_test_working_dir(working_dir);
    }

    #[test]
    fn residual_marker_detection_reports_malformed_markers() {
        let working_dir = test_working_dir("detects-malformed-residual-markers");
        fs::write(
            working_dir.join(AGENTS_MD_FILE_NAME),
            "# Project\n\n<!-- [ctx:start] -->\nOld session\n",
        )
        .expect("fixture should be written");

        let residual = detect_residual_codex_agents_md_markers(&working_dir)
            .expect("detection should succeed")
            .expect("malformed residual marker should be reported");

        assert_eq!(
            residual.reason,
            "managed ctx block is malformed: found start marker without end marker"
        );

        remove_test_working_dir(working_dir);
    }

    #[test]
    fn inject_codex_agents_md_creates_new_agents_md_file() {
        let working_dir = test_working_dir("creates-new-agents-md-file");
        let context = context("Shared Rules", "Use repo conventions.");
        let preset = codex_preset(&working_dir, "New File", vec![context.context_id]);

        let injection = inject_codex_agents_md(&preset, &[context.clone()])
            .expect("Codex AGENTS.md injection should succeed");
        let agents_md =
            fs::read_to_string(working_dir.join(AGENTS_MD_FILE_NAME)).expect("file should exist");

        assert_eq!(injection.path, working_dir.join(AGENTS_MD_FILE_NAME));
        assert!(!injection.had_existing_file);
        assert_eq!(injection.selected_context_ids, vec![context.context_id]);
        assert!(agents_md.contains(CTX_START_MARKER));
        assert!(agents_md.contains("# CTX Codex Session Context"));
        assert!(agents_md.contains("Preset: New File"));
        assert!(agents_md.contains("## Shared Rules"));
        assert!(agents_md.contains("Use repo conventions."));
        assert!(agents_md.contains(CTX_END_MARKER));

        cleanup_codex_agents_md(&injection).expect("cleanup should remove temporary file");
        assert!(!working_dir.join(AGENTS_MD_FILE_NAME).exists());
        remove_test_working_dir(working_dir);
    }

    #[test]
    fn inject_codex_agents_md_replaces_existing_managed_block() {
        let working_dir = test_working_dir("replaces-existing-managed-block");
        let agents_md_path = working_dir.join(AGENTS_MD_FILE_NAME);
        fs::write(
            &agents_md_path,
            "# Project\n\n<!-- [ctx:start] -->\nOld managed context\n<!-- [ctx:end] -->\n\nManual note\n",
        )
        .expect("fixture should be written");
        let context = context("Fresh Context", "New managed context");
        let preset = codex_preset(&working_dir, "Replacement", vec![context.context_id]);

        let injection =
            inject_codex_agents_md(&preset, &[context]).expect("managed block should replace");
        let agents_md = fs::read_to_string(&agents_md_path).expect("file should be readable");

        assert!(injection.had_existing_file);
        assert!(agents_md.contains("# Project"));
        assert!(agents_md.contains("Manual note"));
        assert!(agents_md.contains("Preset: Replacement"));
        assert!(agents_md.contains("New managed context"));
        assert!(!agents_md.contains("Old managed context"));
        assert_eq!(agents_md.matches(CTX_START_MARKER).count(), 1);
        assert_eq!(agents_md.matches(CTX_END_MARKER).count(), 1);

        cleanup_codex_agents_md(&injection).expect("cleanup should remove managed section");
        remove_test_working_dir(working_dir);
    }

    #[test]
    fn inject_codex_agents_md_duplicate_runs_do_not_duplicate_managed_block() {
        let working_dir = test_working_dir("duplicate-runs");
        let context = context("Repeatable", "Repeatable context");
        let preset = codex_preset(&working_dir, "Repeat", vec![context.context_id]);

        let first = inject_codex_agents_md(&preset, std::slice::from_ref(&context))
            .expect("first injection should succeed");
        let after_first =
            fs::read_to_string(working_dir.join(AGENTS_MD_FILE_NAME)).expect("file should exist");
        let second = inject_codex_agents_md(&preset, &[context])
            .expect("second injection should replace same managed block");
        let after_second =
            fs::read_to_string(working_dir.join(AGENTS_MD_FILE_NAME)).expect("file should exist");

        assert_eq!(after_second, after_first);
        assert_eq!(after_second.matches(CTX_START_MARKER).count(), 1);
        assert_eq!(after_second.matches(CTX_END_MARKER).count(), 1);
        assert!(!first.had_existing_file);
        assert!(second.had_existing_file);

        cleanup_codex_agents_md(&second).expect("cleanup should remove managed section");
        remove_test_working_dir(working_dir);
    }

    #[test]
    fn cleanup_codex_agents_md_preserves_unrelated_user_content() {
        let working_dir = test_working_dir("preserves-user-content");
        let agents_md_path = working_dir.join(AGENTS_MD_FILE_NAME);
        let original_user_content =
            "# Project Agents\n\nManual instruction.\n\n## Local Notes\nKeep this.\n";
        fs::write(&agents_md_path, original_user_content).expect("fixture should be written");
        let context = context("Session Context", "Temporary injected context");
        let preset = codex_preset(&working_dir, "Preserve Manual", vec![context.context_id]);

        let injection =
            inject_codex_agents_md(&preset, &[context]).expect("injection should append block");
        let injected = fs::read_to_string(&agents_md_path).expect("file should be readable");
        assert!(injected.contains(original_user_content.trim_end()));
        assert!(injected.contains("Temporary injected context"));

        cleanup_codex_agents_md(&injection).expect("cleanup should preserve user content");
        let cleaned = fs::read_to_string(&agents_md_path).expect("file should remain");
        assert_eq!(
            cleaned,
            original_user_content.trim_end_matches('\n').to_string() + "\n\n"
        );

        remove_test_working_dir(working_dir);
    }

    #[test]
    fn cleanup_codex_agents_md_removes_only_managed_marker_block() {
        let working_dir = test_working_dir("selective-managed-block-cleanup");
        let agents_md_path = working_dir.join(AGENTS_MD_FILE_NAME);
        fs::write(
            &agents_md_path,
            concat!(
                "# Existing Rules\n",
                "\n",
                "Manual instruction before launch.\n",
                "\n",
                "<!-- [ctx:start] -->\n",
                "# CTX Codex Session Context\n",
                "\n",
                "Preset: Cleanup Test\n",
                "\n",
                "Temporary managed context.\n",
                "<!-- [ctx:end] -->\n",
                "\n",
                "Manual instruction added during session.\n",
                "Do not remove this line.\n",
            ),
        )
        .expect("AGENTS.md fixture should be written");
        let injection = CodexAgentsMdInjection {
            path: agents_md_path.clone(),
            selected_context_ids: Vec::new(),
            managed_content: "Temporary managed context.".to_string(),
            had_existing_file: true,
        };

        cleanup_codex_agents_md(&injection).expect("cleanup should remove only managed block");
        let cleaned = fs::read_to_string(&agents_md_path).expect("AGENTS.md should remain");

        assert_eq!(
            cleaned,
            concat!(
                "# Existing Rules\n",
                "\n",
                "Manual instruction before launch.\n",
                "\n",
                "\n",
                "\n",
                "Manual instruction added during session.\n",
                "Do not remove this line.\n",
            )
        );
        assert!(!cleaned.contains("Temporary managed context."));
        assert!(!cleaned.contains(CTX_START_MARKER));
        assert!(!cleaned.contains(CTX_END_MARKER));

        remove_test_working_dir(working_dir);
    }

    #[test]
    fn stale_wrapper_state_cleanup_removes_prompt_file_agents_block_and_state_file() {
        let working_dir = test_working_dir("stale-wrapper-state-cleanup");
        let state_dir = test_working_dir("stale-wrapper-state-dir");
        let prompt_file = working_dir.join("ctx-prompt.md");
        let agents_md_path = working_dir.join(AGENTS_MD_FILE_NAME);
        fs::write(&prompt_file, "stale prompt").expect("prompt fixture should be written");
        fs::write(
            &agents_md_path,
            "# Project\n\n<!-- [ctx:start] -->\nstale context\n<!-- [ctx:end] -->\n\nManual rule\n",
        )
        .expect("AGENTS.md fixture should be written");

        let state = TransientWrapperState {
            session_id: Uuid::new_v4(),
            preset_id: Uuid::new_v4(),
            target: CliTarget::Codex,
            child_pid: 4242,
            working_dir: working_dir.clone(),
            claude_prompt_file: Some(prompt_file.clone()),
            codex_agents_md_path: Some(agents_md_path.clone()),
            codex_had_existing_agents_md: true,
        };
        let state_path =
            write_transient_wrapper_state(&state_dir, &state).expect("state should be written");

        let report = cleanup_stale_wrapper_state(&state_dir, |_| false);
        let agents_content =
            fs::read_to_string(&agents_md_path).expect("AGENTS.md should remain readable");

        assert_eq!(report.scanned, 1);
        assert_eq!(report.cleaned, 1);
        assert_eq!(report.skipped_active, 0);
        assert!(report.errors.is_empty());
        assert!(!prompt_file.exists());
        assert!(!state_path.exists());
        assert!(agents_content.contains("# Project"));
        assert!(agents_content.contains("Manual rule"));
        assert!(!agents_content.contains("stale context"));
        assert!(!agents_content.contains(CTX_START_MARKER));

        remove_test_working_dir(working_dir);
        remove_test_working_dir(state_dir);
    }

    #[test]
    fn stale_wrapper_state_cleanup_skips_active_child_processes() {
        let working_dir = test_working_dir("active-wrapper-state-cleanup");
        let state_dir = test_working_dir("active-wrapper-state-dir");
        let prompt_file = working_dir.join("ctx-prompt.md");
        fs::write(&prompt_file, "active prompt").expect("prompt fixture should be written");

        let state = TransientWrapperState {
            session_id: Uuid::new_v4(),
            preset_id: Uuid::new_v4(),
            target: CliTarget::Claude,
            child_pid: 4343,
            working_dir: working_dir.clone(),
            claude_prompt_file: Some(prompt_file.clone()),
            codex_agents_md_path: None,
            codex_had_existing_agents_md: false,
        };
        let state_path =
            write_transient_wrapper_state(&state_dir, &state).expect("state should be written");

        let report = cleanup_stale_wrapper_state(&state_dir, |pid| pid == 4343);

        assert_eq!(report.scanned, 1);
        assert_eq!(report.cleaned, 0);
        assert_eq!(report.skipped_active, 1);
        assert!(report.errors.is_empty());
        assert!(prompt_file.exists());
        assert!(state_path.exists());

        remove_test_working_dir(working_dir);
        remove_test_working_dir(state_dir);
    }

    #[test]
    fn stale_wrapper_state_cleanup_reports_malformed_agents_md_and_keeps_state_for_recovery() {
        let working_dir = test_working_dir("malformed-wrapper-state-cleanup");
        let state_dir = test_working_dir("malformed-wrapper-state-dir");
        let agents_md_path = working_dir.join(AGENTS_MD_FILE_NAME);
        fs::write(
            &agents_md_path,
            "# Project\n\n<!-- [ctx:start] -->\npartial stale context\n",
        )
        .expect("malformed AGENTS.md fixture should be written");

        let state = TransientWrapperState {
            session_id: Uuid::new_v4(),
            preset_id: Uuid::new_v4(),
            target: CliTarget::Codex,
            child_pid: 4444,
            working_dir: working_dir.clone(),
            claude_prompt_file: None,
            codex_agents_md_path: Some(agents_md_path.clone()),
            codex_had_existing_agents_md: true,
        };
        let state_path =
            write_transient_wrapper_state(&state_dir, &state).expect("state should be written");

        let report = cleanup_stale_wrapper_state(&state_dir, |_| false);
        let agents_content =
            fs::read_to_string(&agents_md_path).expect("AGENTS.md should remain readable");

        assert_eq!(report.scanned, 1);
        assert_eq!(report.cleaned, 0);
        assert_eq!(report.skipped_active, 0);
        assert_eq!(report.errors.len(), 1);
        assert!(report.errors[0].contains("found start marker without end marker"));
        assert!(state_path.exists());
        assert!(agents_content.contains("partial stale context"));
        assert!(agents_content.contains(CTX_START_MARKER));
        assert!(!agents_content.contains(CTX_END_MARKER));

        remove_test_working_dir(working_dir);
        remove_test_working_dir(state_dir);
    }

    #[test]
    fn residual_codex_marker_cleanup_removes_orphaned_managed_block() {
        let working_dir = test_working_dir("residual-marker-cleanup");
        let agents_md_path = working_dir.join(AGENTS_MD_FILE_NAME);
        fs::write(
            &agents_md_path,
            "# Existing Project Rules\n\n<!-- [ctx:start] -->\norphaned context\n<!-- [ctx:end] -->\n\nManual rule\n",
        )
        .expect("AGENTS.md fixture should be written");

        let cleaned = cleanup_residual_codex_agents_md_markers(&working_dir)
            .expect("residual marker cleanup should succeed");
        let agents_content =
            fs::read_to_string(&agents_md_path).expect("AGENTS.md should remain readable");

        assert!(cleaned);
        assert!(agents_content.contains("# Existing Project Rules"));
        assert!(agents_content.contains("Manual rule"));
        assert!(!agents_content.contains("orphaned context"));
        assert!(!agents_content.contains(CTX_START_MARKER));

        remove_test_working_dir(working_dir);
    }

    #[test]
    fn residual_codex_marker_cleanup_preserves_user_content_around_orphaned_block() {
        let working_dir = test_working_dir("selective-residual-marker-cleanup");
        let agents_md_path = working_dir.join(AGENTS_MD_FILE_NAME);
        fs::write(
            &agents_md_path,
            concat!(
                "# Existing Project Rules\n",
                "\n",
                "Manual rule before orphaned ctx block.\n",
                "\n",
                "<!-- [ctx:start] -->\n",
                "orphaned managed context\n",
                "<!-- [ctx:end] -->\n",
                "\n",
                "Manual rule after orphaned ctx block.\n",
            ),
        )
        .expect("AGENTS.md fixture should be written");

        let cleaned = cleanup_residual_codex_agents_md_markers(&working_dir)
            .expect("residual marker cleanup should succeed");
        let agents_content =
            fs::read_to_string(&agents_md_path).expect("AGENTS.md should remain readable");

        assert!(cleaned);
        assert_eq!(
            agents_content,
            concat!(
                "# Existing Project Rules\n",
                "\n",
                "Manual rule before orphaned ctx block.\n",
                "\n",
                "\n",
                "\n",
                "Manual rule after orphaned ctx block.\n",
            )
        );
        assert!(!agents_content.contains("orphaned managed context"));
        assert!(!agents_content.contains(CTX_START_MARKER));
        assert!(!agents_content.contains(CTX_END_MARKER));

        remove_test_working_dir(working_dir);
    }

    #[test]
    fn resolves_preset_context_items_for_whole_file_heading_line_range_and_anchor() {
        let context = context(
            "Guide",
            concat!(
                "# Guide\n",
                "Intro\n",
                "\n",
                "## Setup\n",
                "Install tools.\n",
                "### Details\n",
                "Keep nested details.\n",
                "\n",
                "## Usage\n",
                "Run ctx.\n",
                "<!-- ctx:anchor:tips -->\n",
                "Tip one.\n",
                "Tip two.\n",
                "## Next\n",
                "Stop here.\n",
            ),
        );
        let mut preset = codex_preset(
            Path::new("/workspace"),
            "Fragments",
            vec![context.context_id],
        );
        preset.preset_context_composition = vec![
            composition(
                context.context_id,
                0,
                "guide.md",
                PresetContextSelection::default(),
            ),
            composition(
                context.context_id,
                1,
                "guide.md#setup",
                PresetContextSelection {
                    selection_kind: PresetContextSelectionKind::Heading,
                    heading: Some("Setup".to_string()),
                    anchor: None,
                    line_start: None,
                    line_end: None,
                    include_children: true,
                },
            ),
            composition(
                context.context_id,
                2,
                "guide.md:L9-L10",
                PresetContextSelection {
                    selection_kind: PresetContextSelectionKind::LineRange,
                    heading: None,
                    anchor: None,
                    line_start: Some(9),
                    line_end: Some(10),
                    include_children: false,
                },
            ),
            composition(
                context.context_id,
                3,
                "guide.md#tips",
                PresetContextSelection {
                    selection_kind: PresetContextSelectionKind::Anchor,
                    heading: None,
                    anchor: Some("tips".to_string()),
                    line_start: None,
                    line_end: None,
                    include_children: false,
                },
            ),
        ];

        let items = resolve_preset_context_items(&preset, &[context])
            .expect("context selections should resolve to concrete items");

        assert_eq!(items.len(), 4);
        assert!(items[0].content.contains("# Guide"));
        assert_eq!(
            items[1].content,
            "## Setup\nInstall tools.\n### Details\nKeep nested details.\n\n"
        );
        assert_eq!(items[2].content, "## Usage\nRun ctx.\n");
        assert_eq!(items[3].content, "Tip one.\nTip two.\n");
    }

    #[test]
    fn codex_payload_uses_selected_fragment_content_in_composition_order() {
        let context = context(
            "Fragments",
            concat!(
                "# Fragments\n",
                "Ignore root.\n",
                "## First\n",
                "First body.\n",
                "## Second\n",
                "Second body.\n",
            ),
        );
        let mut preset = codex_preset(Path::new("/workspace"), "Ordered", vec![context.context_id]);
        preset.preset_context_composition = vec![
            composition(
                context.context_id,
                20,
                "fragments.md#second",
                PresetContextSelection {
                    selection_kind: PresetContextSelectionKind::Heading,
                    heading: Some("Second".to_string()),
                    anchor: None,
                    line_start: None,
                    line_end: None,
                    include_children: true,
                },
            ),
            composition(
                context.context_id,
                10,
                "fragments.md#first",
                PresetContextSelection {
                    selection_kind: PresetContextSelectionKind::Heading,
                    heading: Some("First".to_string()),
                    anchor: None,
                    line_start: None,
                    line_end: None,
                    include_children: true,
                },
            ),
        ];

        let payload = assemble_codex_agents_md_payload(&preset, &[context])
            .expect("payload should be assembled from selected fragments");

        assert!(payload.contains("## First"));
        assert!(payload.contains("First body."));
        assert!(payload.contains("## Second"));
        assert!(payload.contains("Second body."));
        assert!(payload.contains("```ctx-metadata\n"));
        assert!(!payload.contains("Ignore root."));
        assert!(
            payload
                .find("First body.")
                .expect("first body should exist")
                < payload
                    .find("Second body.")
                    .expect("second body should exist")
        );
    }

    #[test]
    fn combined_context_output_merges_resolved_items_with_stable_separators_and_metadata() {
        let first = context("Alpha", "alpha body\n");
        let second = context("Beta", "# Beta\n\n## Notes\nbeta body\n");
        let mut preset = codex_preset(
            Path::new("/workspace/project"),
            "Stable Merge",
            vec![first.context_id, second.context_id],
        );
        preset.preset_model = Some("codex".to_string());
        preset.preset_context_composition = vec![
            composition(
                second.context_id,
                20,
                "contexts/beta.md#notes",
                PresetContextSelection {
                    selection_kind: PresetContextSelectionKind::Heading,
                    heading: Some("Notes".to_string()),
                    anchor: None,
                    line_start: None,
                    line_end: None,
                    include_children: true,
                },
            ),
            composition(
                first.context_id,
                10,
                "contexts/alpha.md",
                PresetContextSelection::default(),
            ),
        ];

        let items = resolve_preset_context_items(&preset, &[second, first])
            .expect("items should resolve in preset order");
        let output = assemble_combined_context_output("CTX Test Context", &preset, &items);

        assert!(output.starts_with("# CTX Test Context\n\n"));
        assert!(output.contains("Preset: Stable Merge\n"));
        assert!(output.contains(&format!("Preset ID: {}\n", preset.preset_id)));
        assert!(output.contains("Target CLI: codex\n"));
        assert!(output.contains("Working Directory: /workspace/project\n"));
        assert!(output.contains("Model: codex\n"));
        assert_eq!(output.matches(COMBINED_CONTEXT_ITEM_SEPARATOR).count(), 2);
        assert!(
            output
                .find("## Alpha")
                .expect("first resolved item should exist")
                < output
                    .find("## Beta")
                    .expect("second resolved item should exist")
        );
        assert!(output.contains("```ctx-metadata\n"));
        assert!(output.contains("source_ref: contexts/alpha.md\n"));
        assert!(output.contains("selection: whole-file\n"));
        assert!(output.contains("source_ref: contexts/beta.md#notes\n"));
        assert!(output.contains("selection: heading:Notes:include_children=true\n"));
        assert!(output.contains("vault_scope: local\n"));
        assert!(output.contains("alpha body\n"));
        assert!(output.contains("beta body\n"));
    }

    #[test]
    fn combined_context_output_includes_validated_subagent_manifest_before_context_items() {
        let context = context("Implementation Notes", "ship the narrow patch\n");
        let mut preset = codex_preset(
            Path::new("/workspace/project"),
            "Delegated Implementation",
            vec![context.context_id],
        );
        let manifest = validate_subagent_manifest(SubagentManifest {
            manifest_version: Some("1".to_string()),
            roles: vec![SubagentRole {
                role_id: "reviewer".to_string(),
                role_name: "Reviewer".to_string(),
                role: "Code review subagent".to_string(),
                capabilities: vec!["correctness review".to_string()],
                constraints: vec!["Stay within assigned context.".to_string()],
                metadata: std::collections::BTreeMap::from([(
                    "owner".to_string(),
                    "quality".to_string(),
                )]),
                description: Some("Review implementation changes".to_string()),
                assigned_contexts: vec!["contexts/reviewer.md".to_string()],
                spawn_instructions: vec!["Inspect changed files".to_string()],
                spawn_guidance: SubagentSpawnGuidance {
                    select_when: vec![
                        "Use after code changes are complete and ready for review.".to_string()
                    ],
                    avoid_when: vec![
                        "Avoid for broad repository research or direct implementation.".to_string(),
                    ],
                    delegation_prompt: Some(
                        "Inspect changed files and return findings first.".to_string(),
                    ),
                },
                handoff_targets: vec!["implementer".to_string()],
                model: Some("gpt-5.3-codex".to_string()),
            }],
            handoff_constraints: HandoffConstraints {
                require_summary: true,
                require_changed_files: true,
                require_open_questions: true,
                max_parallel_subagents: Some(2),
                allowed_handoff_targets: vec!["implementer".to_string()],
                blocked_handoff_targets: Vec::new(),
                handoff_prompt_template: Some("Return findings first.".to_string()),
            },
        })
        .expect("manifest fixture should satisfy delegation validation");
        preset.subagent_manifest = Some(manifest);

        let items = resolve_preset_context_items(&preset, &[context])
            .expect("items should resolve in preset order");
        let output = assemble_combined_context_output("CTX Test Context", &preset, &items);

        assert!(output.contains("## Delegation Manifest\n"));
        assert!(output.contains("```ctx-subagent-manifest\n"));
        assert!(output.contains("\"manifest_version\": \"1\""));
        assert!(output.contains("\"id\": \"reviewer\""));
        assert!(output.contains("\"role\": \"Code review subagent\""));
        assert!(output.contains("\"capabilities\": ["));
        assert!(output.contains("\"constraints\": ["));
        assert!(output.contains("\"metadata\": {"));
        assert!(output.contains("\"assigned_contexts\": ["));
        assert!(output.contains("\"contexts/reviewer.md\""));
        assert!(output.contains("\"max_parallel_subagents\": 2"));
        assert!(
            output
                .find("## Delegation Manifest")
                .expect("manifest section should exist")
                < output
                    .find("## Implementation Notes")
                    .expect("context item should exist")
        );
    }

    #[test]
    fn subagent_context_output_renders_assigned_contexts_without_delegation_manifest() {
        let shared = context("Shared Rules", "Keep the whole team aligned.\n");
        let reviewer = context("Reviewer Notes", "Return findings first.\n");
        let implementer = context("Implementer Notes", "Ship the scoped patch.\n");
        let mut preset = codex_preset(
            Path::new("/workspace/project"),
            "Delegated Implementation",
            vec![
                shared.context_id,
                reviewer.context_id,
                implementer.context_id,
            ],
        );
        preset.preset_context_composition = vec![
            composition(
                shared.context_id,
                0,
                "shared/rules.md",
                PresetContextSelection::default(),
            ),
            composition(
                reviewer.context_id,
                10,
                "subagents/reviewer.md",
                PresetContextSelection::default(),
            ),
            composition(
                implementer.context_id,
                20,
                "subagents/implementer.md",
                PresetContextSelection::default(),
            ),
        ];
        preset.subagent_manifest = Some(
            validate_subagent_manifest(SubagentManifest {
                manifest_version: Some("1".to_string()),
                roles: vec![SubagentRole {
                    role_id: "reviewer".to_string(),
                    role_name: "Reviewer".to_string(),
                    role: "Code review subagent".to_string(),
                    capabilities: vec!["correctness review".to_string()],
                    constraints: vec!["Stay within assigned context.".to_string()],
                    metadata: Default::default(),
                    description: Some("Reviews implementation changes.".to_string()),
                    assigned_contexts: vec!["subagents/reviewer.md".to_string()],
                    spawn_instructions: vec!["Inspect changed files.".to_string()],
                    spawn_guidance: SubagentSpawnGuidance {
                        select_when: vec!["Use after implementation changes.".to_string()],
                        avoid_when: vec!["Avoid before code changes exist.".to_string()],
                        delegation_prompt: Some("Return findings first.".to_string()),
                    },
                    handoff_targets: Vec::new(),
                    model: Some("gpt-5.3-codex".to_string()),
                }],
                handoff_constraints: HandoffConstraints {
                    require_summary: true,
                    require_changed_files: true,
                    require_open_questions: true,
                    max_parallel_subagents: Some(1),
                    allowed_handoff_targets: Vec::new(),
                    blocked_handoff_targets: Vec::new(),
                    handoff_prompt_template: None,
                },
            })
            .expect("manifest fixture should satisfy delegation validation"),
        );

        let items =
            resolve_subagent_context_items(&preset, &[shared, reviewer, implementer], "reviewer")
                .expect("reviewer assigned contexts should resolve");
        let output = assemble_subagent_context_output("CTX Reviewer Context", &preset, &items);

        assert_eq!(items.len(), 1);
        assert!(output.starts_with("# CTX Reviewer Context\n\n"));
        assert!(output.contains("Preset: Delegated Implementation\n"));
        assert!(output.contains("## Reviewer Notes\n"));
        assert!(output.contains("Return findings first.\n"));
        assert!(output.contains("source_ref: subagents/reviewer.md\n"));
        assert!(!output.contains("## Delegation Manifest\n"));
        assert!(!output.contains("```ctx-subagent-manifest\n"));
        assert!(!output.contains("\"assigned_contexts\""));
        assert!(!output.contains("## Shared Rules\n"));
        assert!(!output.contains("## Implementer Notes\n"));
    }

    #[test]
    fn delegation_manifest_renders_only_for_main_agent_context_output() {
        let main = context("Main Runbook", "Coordinate delegated work.\n");
        let reviewer = context("Reviewer Notes", "Review only the assigned patch.\n");
        let mut preset = codex_preset(
            Path::new("/workspace/project"),
            "Delegated Implementation",
            vec![main.context_id, reviewer.context_id],
        );
        preset.preset_context_composition = vec![
            composition(
                main.context_id,
                0,
                "main/runbook.md",
                PresetContextSelection::default(),
            ),
            composition(
                reviewer.context_id,
                10,
                "subagents/reviewer.md",
                PresetContextSelection::default(),
            ),
        ];
        preset.subagent_manifest = Some(
            validate_subagent_manifest(SubagentManifest {
                manifest_version: Some("1".to_string()),
                roles: vec![SubagentRole {
                    role_id: "reviewer".to_string(),
                    role_name: "Reviewer".to_string(),
                    role: "Code review subagent".to_string(),
                    capabilities: vec!["correctness review".to_string()],
                    constraints: vec!["Stay within assigned context.".to_string()],
                    metadata: Default::default(),
                    description: Some("Reviews implementation changes.".to_string()),
                    assigned_contexts: vec!["subagents/reviewer.md".to_string()],
                    spawn_instructions: vec!["Inspect changed files.".to_string()],
                    spawn_guidance: SubagentSpawnGuidance {
                        select_when: vec!["Use after implementation changes.".to_string()],
                        avoid_when: vec!["Avoid before code changes exist.".to_string()],
                        delegation_prompt: Some("Return findings first.".to_string()),
                    },
                    handoff_targets: Vec::new(),
                    model: Some("gpt-5.3-codex".to_string()),
                }],
                handoff_constraints: HandoffConstraints {
                    require_summary: true,
                    require_changed_files: true,
                    require_open_questions: true,
                    max_parallel_subagents: Some(1),
                    allowed_handoff_targets: Vec::new(),
                    blocked_handoff_targets: Vec::new(),
                    handoff_prompt_template: None,
                },
            })
            .expect("manifest fixture should satisfy delegation validation"),
        );

        let main_items = resolve_preset_context_items(&preset, &[main.clone(), reviewer.clone()])
            .expect("main agent should resolve all preset contexts");
        let subagent_items = resolve_subagent_context_items(&preset, &[main, reviewer], "reviewer")
            .expect("subagent should resolve assigned context only");
        let main_output =
            assemble_combined_context_output("CTX Main Agent Context", &preset, &main_items);
        let subagent_output =
            assemble_subagent_context_output("CTX Reviewer Context", &preset, &subagent_items);

        assert!(main_output.contains("## Delegation Manifest\n"));
        assert!(main_output.contains("```ctx-subagent-manifest\n"));
        assert!(main_output.contains("\"id\": \"reviewer\""));
        assert!(main_output.contains("\"assigned_contexts\": ["));
        assert!(main_output.contains("\"subagents/reviewer.md\""));
        assert!(main_output.contains("## Main Runbook\n"));
        assert!(main_output.contains("## Reviewer Notes\n"));

        assert_eq!(subagent_items.len(), 1);
        assert!(subagent_output.contains("## Reviewer Notes\n"));
        assert!(subagent_output.contains("Review only the assigned patch.\n"));
        assert!(!subagent_output.contains("## Delegation Manifest\n"));
        assert!(!subagent_output.contains("```ctx-subagent-manifest\n"));
        assert!(!subagent_output.contains("\"id\": \"reviewer\""));
        assert!(!subagent_output.contains("\"assigned_contexts\""));
        assert!(!subagent_output.contains("## Main Runbook\n"));
    }

    #[test]
    fn generated_main_agent_context_embeds_expected_manifest_content() {
        let main = context("Main Runbook", "Coordinate delegated work.\n");
        let reviewer = context("Reviewer Notes", "Review only the assigned patch.\n");
        let mut preset = codex_preset(
            Path::new("/workspace/project"),
            "Delegated Implementation",
            vec![main.context_id, reviewer.context_id],
        );
        preset.preset_context_composition = vec![
            composition(
                main.context_id,
                0,
                "main/runbook.md",
                PresetContextSelection::default(),
            ),
            composition(
                reviewer.context_id,
                10,
                "subagents/reviewer.md",
                PresetContextSelection::default(),
            ),
        ];
        preset.subagent_manifest = Some(
            validate_subagent_manifest(SubagentManifest {
                manifest_version: Some("1".to_string()),
                roles: vec![SubagentRole {
                    role_id: "reviewer".to_string(),
                    role_name: "Reviewer".to_string(),
                    role: "Code review subagent".to_string(),
                    capabilities: vec!["correctness review".to_string()],
                    constraints: vec!["Stay within assigned context.".to_string()],
                    metadata: Default::default(),
                    description: Some("Reviews implementation changes.".to_string()),
                    assigned_contexts: vec!["subagents/reviewer.md".to_string()],
                    spawn_instructions: vec!["Inspect changed files.".to_string()],
                    spawn_guidance: SubagentSpawnGuidance {
                        select_when: vec!["Use after implementation changes.".to_string()],
                        avoid_when: vec!["Avoid before code changes exist.".to_string()],
                        delegation_prompt: Some("Return findings first.".to_string()),
                    },
                    handoff_targets: Vec::new(),
                    model: Some("gpt-5.3-codex".to_string()),
                }],
                handoff_constraints: HandoffConstraints {
                    require_summary: true,
                    require_changed_files: true,
                    require_open_questions: true,
                    max_parallel_subagents: Some(1),
                    allowed_handoff_targets: Vec::new(),
                    blocked_handoff_targets: Vec::new(),
                    handoff_prompt_template: None,
                },
            })
            .expect("manifest fixture should satisfy delegation validation"),
        );

        let main_items = resolve_preset_context_items(&preset, &[main, reviewer])
            .expect("main agent should resolve all preset contexts");
        let main_output =
            assemble_combined_context_output("CTX Main Agent Context", &preset, &main_items);
        let manifest_json = extract_manifest_fixture(&main_output);
        let manifest: serde_json::Value =
            serde_json::from_str(manifest_json).expect("manifest fixture should be valid JSON");

        assert_eq!(manifest["manifest_version"], "1");
        assert_eq!(manifest["roles"][0]["id"], "reviewer");
        assert_eq!(manifest["roles"][0]["name"], "Reviewer");
        assert_eq!(manifest["roles"][0]["role"], "Code review subagent");
        assert_eq!(
            manifest["roles"][0]["assigned_contexts"][0],
            "subagents/reviewer.md"
        );
        assert_eq!(
            manifest["roles"][0]["spawn_guidance"]["delegation_prompt"],
            "Return findings first."
        );
        assert_eq!(manifest["roles"][0]["model"], "gpt-5.3-codex");
        assert_eq!(manifest["handoff_constraints"]["max_parallel_subagents"], 1);
        assert!(main_output.contains("## Main Runbook\n"));
        assert!(main_output.contains("## Reviewer Notes\n"));
    }

    #[test]
    fn resolves_context_items_with_deterministic_tie_breakers_for_duplicate_orders() {
        let beta = context("Beta", "beta body");
        let alpha = context("Alpha", "alpha body");
        let gamma = context("Gamma", "gamma body");
        let mut preset = codex_preset(
            Path::new("/workspace"),
            "Duplicate Orders",
            vec![beta.context_id, alpha.context_id, gamma.context_id],
        );
        preset.preset_context_composition = vec![
            composition(
                beta.context_id,
                10,
                "contexts/beta.md",
                PresetContextSelection::default(),
            ),
            composition(
                gamma.context_id,
                20,
                "contexts/gamma.md",
                PresetContextSelection::default(),
            ),
            composition(
                alpha.context_id,
                10,
                "contexts/alpha.md",
                PresetContextSelection::default(),
            ),
        ];

        let items = resolve_preset_context_items(&preset, &[gamma, beta, alpha])
            .expect("items should resolve in deterministic order");

        assert_eq!(
            items
                .iter()
                .map(|item| item.source_ref.as_str())
                .collect::<Vec<_>>(),
            vec!["contexts/alpha.md", "contexts/beta.md", "contexts/gamma.md"]
        );
        assert_eq!(
            items
                .iter()
                .map(|item| item.content.as_str())
                .collect::<Vec<_>>(),
            vec!["alpha body", "beta body", "gamma body"]
        );
    }

    #[test]
    fn resolving_context_items_reports_invalid_missing_heading_selection() {
        let context = context("Guide", "# Guide\nBody\n");
        let mut preset = codex_preset(Path::new("/workspace"), "Missing", vec![context.context_id]);
        preset.preset_context_composition = vec![composition(
            context.context_id,
            0,
            "guide.md#missing",
            PresetContextSelection {
                selection_kind: PresetContextSelectionKind::Heading,
                heading: Some("Missing".to_string()),
                anchor: None,
                line_start: None,
                line_end: None,
                include_children: true,
            },
        )];

        let error = resolve_preset_context_items(&preset, &[context])
            .expect_err("missing heading should fail");

        assert!(error.to_string().contains("heading not found: Missing"));
    }

    fn context(title: &str, content: &str) -> ContextFragment {
        ContextFragment {
            context_id: Uuid::new_v4(),
            title: title.to_string(),
            content: content.to_string(),
            file_path: PathBuf::from(format!("/vault/{title}.md")),
            vault_scope: VaultScope::Local,
            classification: Classification::Shared,
            import_classification_suggestion: Some(Classification::Shared),
            inferred_classification: Some(Classification::Shared),
            tags: Vec::new(),
            folder_path: PathBuf::from("contexts"),
            wikilinks: Vec::new(),
            backlinks: Vec::new(),
            import_source: None,
            import_source_type: None,
            llm_classification_status: ClassificationStatus::Reviewed,
        }
    }

    fn codex_preset(
        working_dir: &std::path::Path,
        preset_name: &str,
        context_ids: Vec<Uuid>,
    ) -> Preset {
        Preset {
            preset_id: Uuid::new_v4(),
            preset_name: preset_name.to_string(),
            preset_contexts: context_ids,
            preset_metadata: Default::default(),
            preset_context_composition: Vec::new(),
            preset_target_cli: CliTarget::Codex,
            preset_working_dir: working_dir.to_path_buf(),
            preset_model: None,
            subagent_manifest: None,
        }
    }

    fn composition(
        context_id: Uuid,
        order: usize,
        source_ref: &str,
        selection: PresetContextSelection,
    ) -> PresetContextComposition {
        PresetContextComposition {
            context_id,
            order,
            source_ref: source_ref.to_string(),
            required: true,
            selection,
        }
    }

    fn extract_manifest_fixture(output: &str) -> &str {
        let start_marker = "```ctx-subagent-manifest\n";
        let start = output
            .find(start_marker)
            .expect("main agent output should include manifest fence")
            + start_marker.len();
        let rest = &output[start..];
        let end = rest
            .find("\n```")
            .expect("main agent output should close manifest fence");
        &rest[..end]
    }

    fn test_working_dir(name: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be after epoch")
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "ctx-core-injection-test-{name}-{}-{nanos}",
            std::process::id()
        ));
        fs::create_dir_all(&path).expect("test working dir should be created");
        path
    }

    fn remove_test_working_dir(path: PathBuf) {
        fs::remove_dir_all(path).expect("test working dir should be removable");
    }
}
