use ctx_core::{
    app_status, assemble_claude_prompt_file, assemble_subagent_context_output,
    classify_import_markdown_content, classify_work_context_detail, cleanup_codex_agents_md,
    cleanup_residual_codex_agents_md_markers, cleanup_stale_wrapper_state,
    create_session_handoff_context_file, default_wrapper_state_dir,
    discover_existing_context_file_results, filter_work_relevant_content, initialize_global_vault,
    initialize_project_local_vault, inject_codex_agents_md, list_context_files_with_discovered,
    load_preset_from_resolved_overlay, lookup_markdown_context_index,
    lookup_markdown_contexts_by_tag, materialize_discovered_context_files, new_empty_preset,
    parse_claude_session_log_detail, parse_codex_session_log_detail,
    read_resolved_session_handoff_context, reindex_markdown_contexts,
    remove_transient_wrapper_state_file, replace_agents_md_managed_section,
    resolve_claude_session_log_roots, resolve_codex_session_log_roots, resolve_overlay_vault,
    resolve_subagent_context_items, snapshot_context_directories, watch_context_directories,
    write_transient_wrapper_state, Classification, ClaudeSessionLogScanner, CliTarget,
    CodexAgentsMdInjection, CodexSessionLogScanner, ContextFileSnapshot, ContextFragment,
    ImportTimeClassificationRequest, Preset, PresetContextComposition, PresetContextSelection,
    PresetLoadError, ResolvedContextItem, SessionHandoffClassificationMetadata,
    SessionHandoffContext, SessionLogDetail as AgentSessionDetail,
    SessionLogMetadata as AgentSessionSummary, SessionLogProvider, SessionLogScanRequest,
    SessionLogScanner, SubagentManifest, TransientWrapperState, VaultScope, WorkContextCategory,
    WorkContextClassificationResult, WorkContextRefineMode, WorkContextSignalSet,
};
use std::{
    env,
    ffi::OsString,
    fs,
    io::{self, Read, Write},
    path::{Path, PathBuf},
    process::{self, Child, Command, ExitStatus, Stdio},
    sync::mpsc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use uuid::Uuid;

const CTX_CLAUDE_BIN_ENV: &str = "CTX_CLAUDE_BIN";
const CTX_CODEX_BIN_ENV: &str = "CTX_CODEX_BIN";

fn main() {
    let mut args = env::args().skip(1);

    match args.next().as_deref() {
        Some("init") => init_vault(args.next().as_deref()).unwrap_or_else(|message| {
            eprintln!("{message}");
            process::exit(1);
        }),
        Some("status") => print_status(),
        Some("cleanup") => cleanup_command().unwrap_or_else(|message| {
            eprintln!("{message}");
            process::exit(1);
        }),
        Some("launch") => {
            let target = parse_target(args.next().as_deref()).unwrap_or_else(|message| {
                eprintln!("{message}");
                process::exit(2);
            });
            let launch_args = parse_launch_args(args.collect()).unwrap_or_else(|message| {
                eprintln!("{message}");
                process::exit(2);
            });
            let exit_code = match target {
                CliTarget::Claude => launch_claude(launch_args),
                CliTarget::Codex => launch_codex(launch_args),
            }
            .unwrap_or_else(|message| {
                eprintln!("{message}");
                1
            });

            process::exit(exit_code);
        }
        Some("list") | Some("scan") => list_sessions().unwrap_or_else(|message| {
            eprintln!("{message}");
            process::exit(1);
        }),
        Some("classify") => classify_session(args.collect()).unwrap_or_else(|message| {
            eprintln!("{message}");
            process::exit(1);
        }),
        Some("distill") => distill_session(args.collect()).unwrap_or_else(|message| {
            eprintln!("{message}");
            process::exit(1);
        }),
        Some("saved") => list_saved_session_contexts().unwrap_or_else(|message| {
            eprintln!("{message}");
            process::exit(1);
        }),
        Some("context") => context_command(args.collect()).unwrap_or_else(|message| {
            eprintln!("{message}");
            process::exit(1);
        }),
        Some("import") | Some("reindex") | Some("lookup") | Some("watch") => {
            eprintln!(
                "markdown context commands moved under 'ctx context'. Run 'ctx context --help'."
            );
            process::exit(2);
        }
        Some("-h") | Some("--help") | None => print_help(),
        Some(command) => {
            eprintln!("unknown ctx command: {command}");
            print_help();
            process::exit(2);
        }
    }
}

#[derive(Debug)]
struct ClaudeLaunchPlan {
    session_id: Uuid,
    program: String,
    args: Vec<String>,
    working_dir: PathBuf,
    preset_id: Uuid,
    state_dir: PathBuf,
    prompt_file: TemporaryPromptFile,
    embedded_manifest: Option<ResolvedEmbeddedManifest>,
}

#[derive(Debug)]
struct CodexLaunchPlan {
    session_id: Uuid,
    program: String,
    args: Vec<String>,
    working_dir: PathBuf,
    preset_id: Uuid,
    state_dir: PathBuf,
    injection: ManagedAgentsMdBlock,
    embedded_manifest: Option<ResolvedEmbeddedManifest>,
}

#[derive(Debug, Clone, Eq, PartialEq)]
struct LaunchArgs {
    preset_ref: Option<String>,
    session_ref: Option<String>,
    handoff_ref: Option<String>,
    passthrough_args: Vec<String>,
}

#[derive(Debug, Clone, Eq, PartialEq)]
struct WatchArgs {
    once: bool,
    interval_ms: u64,
}

#[derive(Debug)]
struct LoadedLaunchPreset {
    preset: Preset,
    contexts: Vec<ContextFragment>,
    passthrough_args: Vec<String>,
}

#[derive(Debug)]
struct WrapperStartupOrchestration {
    preset: Preset,
    contexts: Vec<ContextFragment>,
    passthrough_args: Vec<String>,
    embedded_manifest: Option<ResolvedEmbeddedManifest>,
}

#[derive(Debug, Clone)]
struct ResolvedEmbeddedManifest {
    manifest: SubagentManifest,
    role_contexts: Vec<ResolvedSubagentRoleContexts>,
}

#[derive(Debug, Clone)]
struct ResolvedSubagentRoleContexts {
    role_id: String,
    contexts: Vec<ResolvedContextItem>,
}

#[derive(Debug)]
struct TemporaryPromptFile {
    path: PathBuf,
}

impl TemporaryPromptFile {
    fn new(path: PathBuf) -> Self {
        Self { path }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TemporaryPromptFile {
    fn drop(&mut self) {
        cleanup_prompt_file(&self.path);
    }
}

#[derive(Debug)]
struct ManagedAgentsMdBlock {
    injection: CodexAgentsMdInjection,
}

impl ManagedAgentsMdBlock {
    fn new(injection: CodexAgentsMdInjection) -> Self {
        Self { injection }
    }
}

impl Drop for ManagedAgentsMdBlock {
    fn drop(&mut self) {
        let _ = cleanup_codex_agents_md(&self.injection);
    }
}

fn launch_claude(launch_args: LaunchArgs) -> Result<i32, String> {
    let working_dir = env::current_dir()
        .map_err(|error| format!("failed to resolve current working directory: {error}"))?;
    let orchestration = orchestrate_wrapper_startup(CliTarget::Claude, &working_dir, launch_args)?;
    let mut plan = build_claude_launch_plan(orchestration)?;
    plan.program = resolve_claude_command()?;
    run_wrapped_claude_session(plan)
}

fn launch_codex(launch_args: LaunchArgs) -> Result<i32, String> {
    let working_dir = env::current_dir()
        .map_err(|error| format!("failed to resolve current working directory: {error}"))?;
    let orchestration = orchestrate_wrapper_startup(CliTarget::Codex, &working_dir, launch_args)?;
    let program = resolve_codex_command()?;
    let mut plan = build_codex_launch_plan(orchestration)?;
    plan.program = program;
    run_wrapped_codex_session(plan)
}

fn orchestrate_wrapper_startup(
    target: CliTarget,
    working_dir: &Path,
    launch_args: LaunchArgs,
) -> Result<WrapperStartupOrchestration, String> {
    cleanup_stale_state_before_launch();
    let loaded = load_launch_preset(target, working_dir, launch_args.preset_ref)?;
    if launch_args.session_ref.is_some() && launch_args.handoff_ref.is_some() {
        return Err("--session and --handoff cannot be supplied together".to_string());
    }

    let selected_context = match (launch_args.session_ref, launch_args.handoff_ref) {
        (Some(session_ref), None) => Some(resolve_session_context_fragment(&session_ref)?),
        (None, Some(handoff_ref)) => Some(resolve_saved_handoff_context_fragment(
            working_dir,
            &handoff_ref,
        )?),
        (None, None) => None,
        (Some(_), Some(_)) => unreachable!("checked above"),
    };
    let mut preset = loaded.preset;
    let mut contexts = loaded.contexts;
    if let Some(selected_context) = selected_context {
        prepend_session_context_to_launch_selection(&mut preset, &selected_context);
        contexts.insert(0, selected_context);
    }
    let passthrough_args =
        merge_launch_passthrough_args(loaded.passthrough_args, launch_args.passthrough_args);
    let embedded_manifest = resolve_embedded_launch_manifest(&preset, &contexts)?;

    Ok(WrapperStartupOrchestration {
        preset,
        contexts,
        passthrough_args,
        embedded_manifest,
    })
}

fn prepend_session_context_to_launch_selection(preset: &mut Preset, context: &ContextFragment) {
    preset
        .preset_contexts
        .retain(|context_id| context_id != &context.context_id);
    preset.preset_contexts.insert(0, context.context_id);

    if preset.preset_context_composition.is_empty() {
        return;
    }

    for composition in &mut preset.preset_context_composition {
        composition.order = composition.order.saturating_add(1);
    }

    let source_ref = context
        .session_handoff_classification
        .as_ref()
        .map(|metadata| {
            format!(
                "session:{}:{}",
                metadata.source_tool, metadata.source_session_ref
            )
        })
        .unwrap_or_else(|| format!("session:{}", context.context_id));

    preset
        .preset_context_composition
        .retain(|composition| composition.context_id != context.context_id);
    preset
        .preset_context_composition
        .push(PresetContextComposition {
            context_id: context.context_id,
            order: 0,
            source_ref,
            required: true,
            selection: PresetContextSelection::default(),
        });
}

fn load_launch_preset(
    target: CliTarget,
    working_dir: &Path,
    preset_ref: Option<String>,
) -> Result<LoadedLaunchPreset, String> {
    let Some(preset_ref) = preset_ref else {
        let name = match target {
            CliTarget::Claude => "Default Claude Session",
            CliTarget::Codex => "Default Codex Session",
        };
        return Ok(LoadedLaunchPreset {
            preset: new_empty_preset(name, target, working_dir.to_path_buf()),
            contexts: Vec::new(),
            passthrough_args: Vec::new(),
        });
    };

    let vault = resolve_overlay_vault(working_dir)
        .map_err(|error| format!("failed to resolve vault overlay for launch: {error}"))?;
    let loaded = load_preset_from_resolved_overlay(
        &vault.roots,
        &vault.contexts,
        &preset_ref,
        target,
        working_dir,
    )
    .map_err(|error| format_launch_preset_load_error(target, &preset_ref, error))?;

    Ok(LoadedLaunchPreset {
        preset: loaded.preset,
        contexts: loaded.contexts,
        passthrough_args: loaded.passthrough_args,
    })
}

fn format_launch_preset_load_error(
    target: CliTarget,
    preset_ref: &str,
    error: PresetLoadError,
) -> String {
    let target_name = match target {
        CliTarget::Claude => "claude",
        CliTarget::Codex => "codex",
    };
    let detail = error.to_string();

    match error {
        PresetLoadError::NotFound { .. } => {
            format!("Cannot launch {target_name} with --preset '{preset_ref}': {detail}")
        }
        PresetLoadError::AmbiguousPresetName { .. } => {
            format!("Cannot launch {target_name} because --preset '{preset_ref}' is ambiguous: {detail}")
        }
        PresetLoadError::TargetMismatch { .. }
        | PresetLoadError::MissingContext { .. }
        | PresetLoadError::Validation(_)
        | PresetLoadError::Parse(_) => {
            format!(
                "Cannot launch {target_name} because preset '{preset_ref}' is invalid: {detail}"
            )
        }
        PresetLoadError::Io(_) => {
            format!("Cannot launch {target_name} because preset '{preset_ref}' could not be read: {detail}")
        }
    }
}

fn resolve_embedded_launch_manifest(
    preset: &Preset,
    contexts: &[ContextFragment],
) -> Result<Option<ResolvedEmbeddedManifest>, String> {
    let Some(manifest) = &preset.subagent_manifest else {
        return Ok(None);
    };

    let mut role_contexts = Vec::with_capacity(manifest.roles.len());
    for role in &manifest.roles {
        let contexts =
            resolve_subagent_context_items(preset, contexts, &role.role_id).map_err(|error| {
                format!(
                    "failed to resolve embedded subagent manifest for preset {}: {error}",
                    preset.preset_name
                )
            })?;
        role_contexts.push(ResolvedSubagentRoleContexts {
            role_id: role.role_id.clone(),
            contexts,
        });
    }

    Ok(Some(ResolvedEmbeddedManifest {
        manifest: manifest.clone(),
        role_contexts,
    }))
}

fn merge_launch_passthrough_args(
    mut preset_passthrough_args: Vec<String>,
    cli_passthrough_args: Vec<String>,
) -> Vec<String> {
    preset_passthrough_args.extend(cli_passthrough_args);
    preset_passthrough_args
}

fn build_claude_launch_plan(
    orchestration: WrapperStartupOrchestration,
) -> Result<ClaudeLaunchPlan, String> {
    let prompt_file = assemble_claude_prompt_file(&orchestration.preset, &orchestration.contexts)
        .map_err(|error| {
        format!(
            "Cannot launch Claude: failed to prepare the temporary --append-system-prompt-file payload.\n\
             Detail: {error}\n\
             Next step: verify the selected preset contexts still exist and are readable, then retry."
        )
    })?;
    let temporary_prompt_file = TemporaryPromptFile::new(prompt_file.path);
    append_embedded_manifest_to_claude_prompt_file(
        temporary_prompt_file.path(),
        &orchestration.preset,
        orchestration.embedded_manifest.as_ref(),
    )?;

    let mut args = vec![
        "--append-system-prompt-file".to_string(),
        temporary_prompt_file.path().display().to_string(),
    ];
    append_model_arg(&mut args, orchestration.preset.preset_model.as_deref());
    args.extend(orchestration.passthrough_args);

    Ok(ClaudeLaunchPlan {
        session_id: Uuid::new_v4(),
        program: "claude".to_string(),
        args,
        working_dir: orchestration.preset.preset_working_dir.clone(),
        preset_id: orchestration.preset.preset_id,
        state_dir: default_wrapper_state_dir(),
        prompt_file: temporary_prompt_file,
        embedded_manifest: orchestration.embedded_manifest,
    })
}

fn resolve_claude_command() -> Result<String, String> {
    resolve_command_from_path_or_env(
        "Claude CLI",
        "claude",
        CTX_CLAUDE_BIN_ENV,
        env::var_os(CTX_CLAUDE_BIN_ENV),
        env::var_os("PATH"),
    )
}

fn resolve_codex_command() -> Result<String, String> {
    resolve_command_from_path_or_env(
        "Codex CLI",
        "codex",
        CTX_CODEX_BIN_ENV,
        env::var_os(CTX_CODEX_BIN_ENV),
        env::var_os("PATH"),
    )
}

fn resolve_command_from_path_or_env(
    label: &str,
    default_name: &str,
    env_var_name: &str,
    configured_command: Option<OsString>,
    path_env: Option<OsString>,
) -> Result<String, String> {
    if let Some(command) = configured_command {
        let command = command.to_string_lossy().trim().to_string();
        if command.is_empty() {
            return Err(format!(
                "{env_var_name} is set but empty; set it to the {label} executable path or unset it"
            ));
        }

        return resolve_configured_command(label, env_var_name, &command, path_env.as_ref());
    }

    find_executable_on_path(default_name, path_env.as_ref())
        .map(|path| path.display().to_string())
        .ok_or_else(|| {
            format!(
                "failed to resolve {label}: executable '{default_name}' was not found on PATH. Install {label}, add it to PATH, or set {env_var_name} to its executable path."
            )
        })
}

fn resolve_configured_command(
    label: &str,
    env_var_name: &str,
    command: &str,
    path_env: Option<&OsString>,
) -> Result<String, String> {
    let path = PathBuf::from(command);
    if command_has_path_separator(command) || path.is_absolute() {
        if is_executable_file(&path) {
            return Ok(path.display().to_string());
        }

        return Err(format!(
            "{env_var_name} points to {}, but that file is not executable or does not exist",
            path.display()
        ));
    }

    find_executable_on_path(command, path_env)
        .map(|path| path.display().to_string())
        .ok_or_else(|| {
            format!("failed to resolve {label}: {env_var_name}='{command}' was not found on PATH")
        })
}

fn command_has_path_separator(command: &str) -> bool {
    command.contains(std::path::MAIN_SEPARATOR) || command.contains('/') || command.contains('\\')
}

fn find_executable_on_path(command: &str, path_env: Option<&OsString>) -> Option<PathBuf> {
    let path_env = path_env?;
    env::split_paths(path_env)
        .map(|dir| dir.join(command))
        .find(|candidate| is_executable_file(candidate))
}

fn is_executable_file(path: &Path) -> bool {
    if !path.is_file() {
        return false;
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::metadata(path)
            .map(|metadata| metadata.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
    }

    #[cfg(not(unix))]
    {
        true
    }
}

fn append_embedded_manifest_to_claude_prompt_file(
    prompt_file: &Path,
    preset: &Preset,
    embedded_manifest: Option<&ResolvedEmbeddedManifest>,
) -> Result<(), String> {
    let Some(embedded_manifest) = embedded_manifest else {
        return Ok(());
    };

    let output = build_embedded_manifest_startup_payload("Claude", preset, embedded_manifest)?;

    let mut file = fs::OpenOptions::new()
        .append(true)
        .open(prompt_file)
        .map_err(|error| {
            format!(
                "failed to open Claude prompt file {} for embedded manifest injection: {error}",
                prompt_file.display()
            )
        })?;
    file.write_all(output.as_bytes()).map_err(|error| {
        format!(
            "failed to inject embedded manifest into Claude prompt file {}: {error}",
            prompt_file.display()
        )
    })
}

fn build_embedded_manifest_startup_payload(
    target_label: &str,
    preset: &Preset,
    embedded_manifest: &ResolvedEmbeddedManifest,
) -> Result<String, String> {
    let mut output = String::new();
    output.push_str("\n\n---\n\n");
    output.push_str("# CTX Wrapper Embedded Subagent Payload\n\n");
    output.push_str(&format!(
        "This payload was resolved by the ctx wrapper before {target_label} CLI startup. Use it as the authoritative subagent manifest and per-role context bundle for this launched session.\n",
    ));
    output.push_str("\n```ctx-subagent-manifest\n");
    output.push_str(
        &serde_json::to_string_pretty(&embedded_manifest.manifest)
            .map_err(|error| format!("failed to serialize embedded subagent manifest: {error}"))?,
    );
    output.push_str("\n```\n");

    for role_contexts in &embedded_manifest.role_contexts {
        output.push_str("\n\n---\n\n");
        output.push_str(&assemble_subagent_context_output(
            &format!("CTX Embedded Subagent Context: {}", role_contexts.role_id),
            preset,
            &role_contexts.contexts,
        ));
    }

    Ok(output)
}

fn build_codex_launch_plan(
    orchestration: WrapperStartupOrchestration,
) -> Result<CodexLaunchPlan, String> {
    let injection = inject_codex_agents_md(&orchestration.preset, &orchestration.contexts)
        .map_err(|error| {
            format!(
                "Cannot launch Codex: failed to prepare the managed AGENTS.md context block.\n\
                 Detail: {error}\n\
                 Safety: AGENTS.md was not overwritten.\n\
                 Next step: fix malformed or duplicate ctx managed markers in AGENTS.md, or verify the selected preset contexts still exist and are readable."
            )
        })?;
    let mut managed_block = ManagedAgentsMdBlock::new(injection);
    append_embedded_manifest_to_codex_agents_md(
        &mut managed_block.injection,
        &orchestration.preset,
        orchestration.embedded_manifest.as_ref(),
    )?;

    Ok(CodexLaunchPlan {
        session_id: Uuid::new_v4(),
        program: "codex".to_string(),
        args: launch_args_with_model(
            orchestration.preset.preset_model.as_deref(),
            orchestration.passthrough_args,
        ),
        working_dir: orchestration.preset.preset_working_dir.clone(),
        preset_id: orchestration.preset.preset_id,
        state_dir: default_wrapper_state_dir(),
        injection: managed_block,
        embedded_manifest: orchestration.embedded_manifest,
    })
}

fn append_embedded_manifest_to_codex_agents_md(
    injection: &mut CodexAgentsMdInjection,
    preset: &Preset,
    embedded_manifest: Option<&ResolvedEmbeddedManifest>,
) -> Result<(), String> {
    let Some(embedded_manifest) = embedded_manifest else {
        return Ok(());
    };

    let payload = build_embedded_manifest_startup_payload("Codex", preset, embedded_manifest)?;
    injection.managed_content.push_str(&payload);

    let existing_content = fs::read_to_string(&injection.path).map_err(|error| {
        format!(
            "failed to read Codex AGENTS.md {} before embedded manifest injection: {error}",
            injection.path.display()
        )
    })?;
    let replacement =
        replace_agents_md_managed_section(&existing_content, &injection.managed_content);
    let next_content = match replacement {
        Ok(next_content) => next_content,
        Err(error) => {
            return Err(format!(
                "failed to merge embedded manifest into managed Codex AGENTS.md block {}: {error}",
                injection.path.display()
            ));
        }
    };

    fs::write(&injection.path, next_content).map_err(|error| {
        format!(
            "failed to inject embedded manifest into Codex AGENTS.md {}: {error}",
            injection.path.display()
        )
    })
}

fn launch_args_with_model(model: Option<&str>, passthrough_args: Vec<String>) -> Vec<String> {
    let mut args = Vec::new();
    append_model_arg(&mut args, model);
    args.extend(passthrough_args);
    args
}

fn append_model_arg(args: &mut Vec<String>, model: Option<&str>) {
    let Some(model) = model.map(str::trim).filter(|model| !model.is_empty()) else {
        return;
    };

    args.push("--model".to_string());
    args.push(model.to_string());
}

fn run_wrapped_claude_session(plan: ClaudeLaunchPlan) -> Result<i32, String> {
    let status = run_claude_launch_plan(&plan);
    drop(plan);
    status.map(exit_code_from_status)
}

fn run_wrapped_codex_session(plan: CodexLaunchPlan) -> Result<i32, String> {
    let status = run_codex_launch_plan(&plan).map(propagate_codex_child_exit_status);
    drop(plan);
    status
}

fn run_claude_launch_plan(plan: &ClaudeLaunchPlan) -> Result<ExitStatus, String> {
    let mut child = spawn_claude_child(plan)?;
    let state_file = match record_claude_wrapper_state(plan, child.id()) {
        Ok(state_file) => state_file,
        Err(error) => {
            stop_child_after_state_failure(&mut child);
            return Err(error);
        }
    };
    let status = child.wait().map_err(|error| {
        format!("failed while waiting for Claude CLI wrapped session to exit: {error}")
    });
    cleanup_prompt_file(plan.prompt_file.path());
    drop(state_file);
    status
}

fn spawn_claude_child(plan: &ClaudeLaunchPlan) -> Result<Child, String> {
    let mut command = Command::new(&plan.program);
    command.args(&plan.args).current_dir(&plan.working_dir);
    configure_interactive_stdio(&mut command);
    command
        .spawn()
        .map_err(|error| format_claude_launch_spawn_error(plan, &error))
}

fn run_codex_launch_plan(plan: &CodexLaunchPlan) -> Result<ExitStatus, String> {
    let mut child = spawn_codex_child(plan)?;
    let _signal_guard = ParentInteractiveSignalGuard::install()?;
    let state_file = match record_codex_wrapper_state(plan, child.id()) {
        Ok(state_file) => state_file,
        Err(error) => {
            stop_child_after_state_failure(&mut child);
            return Err(error);
        }
    };
    let status = child.wait().map_err(|error| {
        format!("failed while waiting for Codex CLI wrapped session to exit: {error}")
    });
    let _ = cleanup_codex_agents_md(&plan.injection.injection);
    drop(state_file);
    status
}

fn propagate_codex_child_exit_status(status: ExitStatus) -> i32 {
    exit_code_from_status(status)
}

fn spawn_codex_child(plan: &CodexLaunchPlan) -> Result<Child, String> {
    let mut command = Command::new(&plan.program);
    command.args(&plan.args).current_dir(&plan.working_dir);
    configure_interactive_stdio(&mut command);
    configure_child_terminal_signal_defaults(&mut command);
    command
        .spawn()
        .map_err(|error| format_codex_launch_spawn_error(plan, &error))
}

fn format_claude_launch_spawn_error(plan: &ClaudeLaunchPlan, error: &io::Error) -> String {
    format!(
        "Launch failed: Claude CLI did not start.\n\
         Injection method: temporary prompt file via --append-system-prompt-file.\n\
         Command: {}\n\
         Working directory: {}\n\
         Prompt file: {}\n\
         Detail: {error}\n\
         Cleanup: the temporary prompt file will be removed automatically.\n\
         Next step: verify Claude is installed and executable, or set {CTX_CLAUDE_BIN_ENV} to the Claude CLI path.",
        plan.program,
        plan.working_dir.display(),
        plan.prompt_file.path().display()
    )
}

fn format_codex_launch_spawn_error(plan: &CodexLaunchPlan, error: &io::Error) -> String {
    format!(
        "Launch failed: Codex CLI did not start.\n\
         Injection method: managed ctx block in AGENTS.md.\n\
         Command: {}\n\
         Working directory: {}\n\
         AGENTS.md: {}\n\
         Detail: {error}\n\
         Cleanup: the managed ctx block will be removed automatically; existing user content is preserved.\n\
         Next step: verify Codex is installed and executable, or set {CTX_CODEX_BIN_ENV} to the Codex CLI path.",
        plan.program,
        plan.working_dir.display(),
        plan.injection.injection.path.display()
    )
}

fn configure_interactive_stdio(command: &mut Command) -> &mut Command {
    command
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
}

#[cfg(unix)]
fn configure_child_terminal_signal_defaults(command: &mut Command) {
    use std::os::unix::process::CommandExt;

    unsafe {
        command.pre_exec(|| {
            for signal in [libc::SIGINT, libc::SIGTERM, libc::SIGQUIT] {
                if libc::signal(signal, libc::SIG_DFL) == libc::SIG_ERR {
                    return Err(io::Error::last_os_error());
                }
            }
            Ok(())
        });
    }
}

#[cfg(not(unix))]
fn configure_child_terminal_signal_defaults(_command: &mut Command) {}

#[cfg(unix)]
#[derive(Debug)]
struct ParentInteractiveSignalGuard {
    previous_handlers: Vec<(libc::c_int, libc::sighandler_t)>,
}

#[cfg(unix)]
impl ParentInteractiveSignalGuard {
    fn install() -> Result<Self, String> {
        let mut previous_handlers = Vec::new();
        for signal in [libc::SIGINT, libc::SIGQUIT] {
            // Keep the wrapper alive during interactive child termination so Drop/final cleanup
            // can remove managed Codex AGENTS.md markers after terminal interrupts.
            let previous = unsafe { libc::signal(signal, libc::SIG_IGN) };
            if previous == libc::SIG_ERR {
                for (restored_signal, restored_handler) in previous_handlers.into_iter().rev() {
                    unsafe {
                        libc::signal(restored_signal, restored_handler);
                    }
                }
                return Err(format!(
                    "failed to configure parent signal handling for interactive Codex session: signal {signal}"
                ));
            }
            previous_handlers.push((signal, previous));
        }

        Ok(Self { previous_handlers })
    }
}

#[cfg(unix)]
impl Drop for ParentInteractiveSignalGuard {
    fn drop(&mut self) {
        for (signal, handler) in self.previous_handlers.iter().rev() {
            unsafe {
                libc::signal(*signal, *handler);
            }
        }
    }
}

#[cfg(not(unix))]
#[derive(Debug)]
struct ParentInteractiveSignalGuard;

#[cfg(not(unix))]
impl ParentInteractiveSignalGuard {
    fn install() -> Result<Self, String> {
        Ok(Self)
    }
}

fn cleanup_prompt_file(prompt_file: &Path) {
    let _ = fs::remove_file(prompt_file);
}

#[derive(Debug)]
struct TransientWrapperStateFile {
    path: PathBuf,
}

impl TransientWrapperStateFile {
    fn new(path: PathBuf) -> Self {
        Self { path }
    }
}

impl Drop for TransientWrapperStateFile {
    fn drop(&mut self) {
        let _ = remove_transient_wrapper_state_file(&self.path);
    }
}

fn record_claude_wrapper_state(
    plan: &ClaudeLaunchPlan,
    child_pid: u32,
) -> Result<TransientWrapperStateFile, String> {
    let state = TransientWrapperState {
        session_id: plan.session_id,
        preset_id: plan.preset_id,
        target: CliTarget::Claude,
        child_pid,
        working_dir: plan.working_dir.clone(),
        claude_prompt_file: Some(plan.prompt_file.path().to_path_buf()),
        codex_agents_md_path: None,
        codex_had_existing_agents_md: false,
    };
    write_transient_wrapper_state(&plan.state_dir, &state)
        .map(TransientWrapperStateFile::new)
        .map_err(|error| error.to_string())
}

fn record_codex_wrapper_state(
    plan: &CodexLaunchPlan,
    child_pid: u32,
) -> Result<TransientWrapperStateFile, String> {
    let state = TransientWrapperState {
        session_id: plan.session_id,
        preset_id: plan.preset_id,
        target: CliTarget::Codex,
        child_pid,
        working_dir: plan.working_dir.clone(),
        claude_prompt_file: None,
        codex_agents_md_path: Some(plan.injection.injection.path.clone()),
        codex_had_existing_agents_md: plan.injection.injection.had_existing_file,
    };
    write_transient_wrapper_state(&plan.state_dir, &state)
        .map(TransientWrapperStateFile::new)
        .map_err(|error| error.to_string())
}

fn stop_child_after_state_failure(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}

fn cleanup_stale_state_before_launch() {
    let _ = cleanup_stale_wrapper_state(&default_wrapper_state_dir(), process_is_active);
}

fn cleanup_command() -> Result<(), String> {
    let state_report = cleanup_stale_wrapper_state(&default_wrapper_state_dir(), process_is_active);
    let working_dir = env::current_dir()
        .map_err(|error| format!("failed to resolve current working directory: {error}"))?;
    let cleaned_cwd_markers = if state_report.skipped_active == 0 {
        cleanup_residual_codex_agents_md_markers(&working_dir)
            .map_err(|error| format!("failed to clean residual Codex markers: {error}"))?
    } else {
        false
    };

    println!("wrapper_state_scanned={}", state_report.scanned);
    println!("wrapper_state_cleaned={}", state_report.cleaned);
    println!(
        "wrapper_state_skipped_active={}",
        state_report.skipped_active
    );
    println!("cwd_codex_markers_cleaned={cleaned_cwd_markers}");
    if state_report.skipped_active > 0 {
        eprintln!("cleanup warning: skipped current-directory marker cleanup while wrapped sessions are still active");
    }
    for error in state_report.errors {
        eprintln!("cleanup warning: {error}");
    }

    Ok(())
}

#[cfg(unix)]
fn process_is_active(pid: u32) -> bool {
    let pid = pid.to_string();
    Command::new("kill")
        .args(["-0", pid.as_str()])
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

#[cfg(windows)]
fn process_is_active(pid: u32) -> bool {
    let pid_filter = format!("PID eq {pid}");
    Command::new("tasklist")
        .args(["/FI", pid_filter.as_str()])
        .output()
        .map(|output| {
            output.status.success()
                && String::from_utf8_lossy(&output.stdout).contains(&pid.to_string())
        })
        .unwrap_or(false)
}

fn exit_code_from_status(status: ExitStatus) -> i32 {
    if let Some(code) = status.code() {
        return code;
    }

    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;

        if let Some(signal) = status.signal() {
            return 128 + signal;
        }
    }

    1
}

fn print_status() {
    let status = app_status();
    println!("ctx {}", status.version);
    println!("vault_ready={}", status.vault_ready);
    println!("sqlite_index_ready={}", status.sqlite_index_ready);
    println!("wrapper_ready={}", status.wrapper_ready);
}

fn init_vault(scope: Option<&str>) -> Result<(), String> {
    match scope {
        Some("--global") | Some("global") | None => init_global_vault(),
        Some("--local") | Some("local") => init_project_local_vault(),
        Some(other) => Err(format!(
            "unsupported init scope: {other}. Expected --global or --local."
        )),
    }
}

fn init_global_vault() -> Result<(), String> {
    let working_dir = env::current_dir()
        .map_err(|error| format!("failed to resolve current working directory: {error}"))?;
    let initialized = initialize_global_vault(&working_dir)
        .map_err(|error| format!("failed to initialize global vault: {error}"))?;

    println!(
        "initialized global vault: {}",
        initialized.global_root.display()
    );
    println!(
        "managed contexts directory: {}",
        initialized.contexts_dir.display()
    );
    println!("sqlite index: {}", initialized.sqlite_index_path.display());
    println!(
        "sqlite schema_version: {} -> {}",
        initialized.sqlite_migration.previous_schema_version,
        initialized.sqlite_migration.applied_schema_version
    );

    Ok(())
}

fn init_project_local_vault() -> Result<(), String> {
    let working_dir = env::current_dir()
        .map_err(|error| format!("failed to resolve current working directory: {error}"))?;
    let initialized = initialize_project_local_vault(&working_dir)
        .map_err(|error| format!("failed to initialize project-local vault: {error}"))?;

    println!(
        "initialized project-local vault: {}",
        initialized.local_root.display()
    );
    println!(
        "managed contexts directory: {}",
        initialized.contexts_dir.display()
    );
    println!("sqlite index: {}", initialized.sqlite_index_path.display());
    println!(
        "sqlite schema_version: {} -> {}",
        initialized.sqlite_migration.previous_schema_version,
        initialized.sqlite_migration.applied_schema_version
    );

    Ok(())
}

fn list_contexts() -> Result<(), String> {
    let working_dir = env::current_dir()
        .map_err(|error| format!("failed to resolve current working directory: {error}"))?;
    let contexts = list_context_files_with_discovered(&working_dir)
        .map_err(|error| format!("failed to list context files: {error}"))?;

    if contexts.is_empty() {
        println!("No markdown contexts found.");
        return Ok(());
    }

    for context in contexts {
        println!("{}", format_context_list_row(&context));
    }

    Ok(())
}

fn scan_import_candidates() -> Result<(), String> {
    let working_dir = env::current_dir()
        .map_err(|error| format!("failed to resolve current working directory: {error}"))?;
    let contexts = discover_existing_context_file_results(&working_dir)
        .map_err(|error| format!("failed to scan existing markdown context files: {error}"))?;

    if contexts.is_empty() {
        println!("No existing markdown context files found for import.");
        return Ok(());
    }

    println!("Discovered markdown context files:");
    for context in contexts {
        println!("{}", format_import_candidate_row(&context));
    }

    Ok(())
}

fn import_discovered_contexts() -> Result<(), String> {
    let working_dir = env::current_dir()
        .map_err(|error| format!("failed to resolve current working directory: {error}"))?;
    let contexts = materialize_discovered_context_files(&working_dir)
        .map_err(|error| format!("failed to import discovered markdown context files: {error}"))?;

    if contexts.is_empty() {
        println!("No existing markdown context files found for import.");
        return Ok(());
    }

    println!("Imported markdown context files:");
    for context in contexts {
        println!("{}", format_context_list_row(&context));
    }

    Ok(())
}

fn reindex_contexts() -> Result<(), String> {
    let working_dir = env::current_dir()
        .map_err(|error| format!("failed to resolve current working directory: {error}"))?;
    let report = reindex_markdown_contexts(&working_dir)
        .map_err(|error| format!("failed to reindex markdown contexts: {error}"))?;

    println!(
        "global_indexed_markdown_files={}",
        report.global.indexed_markdown_files
    );
    println!(
        "global_cleared_markdown_files={}",
        report.global.cleared_markdown_files
    );
    if let Some(local) = report.local {
        println!(
            "local_indexed_markdown_files={}",
            local.indexed_markdown_files
        );
        println!(
            "local_cleared_markdown_files={}",
            local.cleared_markdown_files
        );
    }
    println!(
        "discovered_markdown_files={}",
        report.discovered_markdown_files
    );

    Ok(())
}

fn context_command(args: Vec<String>) -> Result<(), String> {
    let mut iter = args.into_iter();
    match iter.next().as_deref() {
        Some("list") => list_contexts(),
        Some("scan") => scan_import_candidates(),
        Some("import") => import_discovered_contexts(),
        Some("classify") => classify_import_content(iter.collect()),
        Some("reindex") => reindex_contexts(),
        Some("lookup") => lookup_index(iter.collect()),
        Some("watch") => watch_contexts(iter.collect()),
        Some("-h") | Some("--help") | None => {
            print_context_help();
            Ok(())
        }
        Some(command) => Err(format!(
            "unknown ctx context command: {command}. Run 'ctx context --help'."
        )),
    }
}

fn list_sessions() -> Result<(), String> {
    let sessions = discover_agent_sessions()?;

    if sessions.is_empty() {
        println!("No Codex or Claude resume sessions found.");
        return Ok(());
    }

    println!("provider\tupdated_at\tmessages\tsession_id\tcwd\ttitle");
    for session in sessions {
        println!("{}", format_session_row(&session));
    }

    Ok(())
}

fn classify_session(args: Vec<String>) -> Result<(), String> {
    let session_ref = parse_optional_session_ref(args)?;
    let detail = resolve_session_detail(session_ref.as_deref().unwrap_or("latest"))?;
    let classification = classify_work_context_detail(&detail)
        .map_err(|error| format!("failed to classify session context: {error}"))?;

    println!(
        "{}\t{}\t{}\t{}",
        classification.category.as_str(),
        classification.confidence_score,
        work_context_category_list(&classification.categories),
        classification.rationale
    );
    Ok(())
}

fn distill_session(args: Vec<String>) -> Result<(), String> {
    let options = parse_distill_args(args)?;
    let detail = resolve_session_detail(options.session_ref.as_deref().unwrap_or("latest"))?;
    let filtered = filter_work_relevant_content(&detail)
        .map_err(|error| format!("failed to filter session work context: {error}"))?;
    let output = if let Some(refiner) = options.refiner {
        refine_distilled_session_context(refiner, &filtered.handoff_markdown)?
    } else {
        filtered.handoff_markdown
    };
    let classification = classify_work_context_detail(&detail)
        .map_err(|error| format!("failed to classify session context: {error}"))?;

    if options.save {
        let refine_mode = if options.refiner.is_some() {
            WorkContextRefineMode::Refined
        } else {
            WorkContextRefineMode::Raw
        };
        let launch_target = options
            .launch_target
            .unwrap_or_else(|| default_launch_target_for_session_detail(&detail));
        let context = save_session_context(
            &detail,
            &output,
            &classification,
            launch_target,
            refine_mode,
        )?;
        println!("{}", context.file_path.display());
        return Ok(());
    }

    print!("{output}");
    Ok(())
}

fn list_saved_session_contexts() -> Result<(), String> {
    let working_dir = env::current_dir()
        .map_err(|error| format!("failed to resolve current working directory: {error}"))?;
    let vault = resolve_overlay_vault(&working_dir)
        .map_err(|error| format!("failed to resolve saved session contexts: {error}"))?;
    let contexts = vault
        .contexts
        .into_iter()
        .filter(is_saved_session_context)
        .collect::<Vec<_>>();

    if contexts.is_empty() {
        println!("No saved session contexts found.");
        return Ok(());
    }

    println!("scope\tcategories\ttitle\tpath");
    for context in contexts {
        println!(
            "{}\t{}\t{}\t{}",
            scope_label(context.vault_scope),
            saved_context_category_list(&context),
            context.title.replace('\n', " "),
            context.file_path.display()
        );
    }

    Ok(())
}

fn is_saved_session_context(context: &ContextFragment) -> bool {
    let path = context.file_path.display().to_string().to_lowercase();
    let folder = context.folder_path.display().to_string().to_lowercase();
    let tags = context
        .tags
        .iter()
        .map(|tag| tag.to_lowercase())
        .collect::<Vec<_>>();

    folder.contains("session-history")
        || path.contains("/session-history/")
        || tags
            .iter()
            .any(|tag| tag == "session-history" || tag == "resume-context")
}

fn parse_optional_session_ref(args: Vec<String>) -> Result<Option<String>, String> {
    match args.as_slice() {
        [] => Ok(Some("latest".to_string())),
        [value] if !value.starts_with('-') => Ok(Some(value.to_string())),
        [flag, value] if flag == "--session" || flag == "-s" => Ok(Some(value.to_string())),
        [flag] if flag.starts_with("--session=") => Ok(Some(
            flag.strip_prefix("--session=")
                .unwrap_or_default()
                .to_string(),
        )),
        _ => Err("usage: ctx classify [latest|<session-id>]".to_string()),
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
struct DistillArgs {
    session_ref: Option<String>,
    save: bool,
    refiner: Option<CliTarget>,
    launch_target: Option<CliTarget>,
}

fn parse_distill_args(args: Vec<String>) -> Result<DistillArgs, String> {
    let mut session_ref = None;
    let mut save = false;
    let mut refiner = None;
    let mut launch_target = None;
    let mut iter = args.into_iter();

    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--save" => save = true,
            "--refine" => refiner = Some(CliTarget::Claude),
            "--refiner" => {
                let value = iter
                    .next()
                    .ok_or_else(|| "--refiner requires claude or codex".to_string())?;
                refiner = Some(parse_target(Some(&value))?);
            }
            value if value.starts_with("--refiner=") => {
                refiner = Some(parse_target(Some(
                    value.strip_prefix("--refiner=").unwrap_or_default(),
                ))?);
            }
            "--launch-target" => {
                let value = iter
                    .next()
                    .ok_or_else(|| "--launch-target requires claude or codex".to_string())?;
                launch_target = Some(parse_target(Some(&value))?);
            }
            value if value.starts_with("--launch-target=") => {
                launch_target = Some(parse_target(Some(
                    value.strip_prefix("--launch-target=").unwrap_or_default(),
                ))?);
            }
            "--session" | "-s" => {
                let value = iter
                    .next()
                    .ok_or_else(|| "--session requires latest or a session id".to_string())?;
                session_ref = Some(value);
            }
            value if value.starts_with("--session=") => {
                session_ref = Some(
                    value
                        .strip_prefix("--session=")
                        .unwrap_or_default()
                        .to_string(),
                );
            }
            value if !value.starts_with('-') && session_ref.is_none() => {
                session_ref = Some(value.to_string());
            }
            "-h" | "--help" => {
                return Err(
                    "usage: ctx distill [latest|<session-id>] [--refine] [--refiner <claude|codex>] [--save] [--launch-target <claude|codex>]"
                        .to_string(),
                )
            }
            other => return Err(format!("unknown ctx distill option: {other}")),
        }
    }

    Ok(DistillArgs {
        session_ref,
        save,
        refiner,
        launch_target,
    })
}

fn lookup_index(args: Vec<String>) -> Result<(), String> {
    let lookup = parse_lookup_args(&args)?;
    let working_dir = env::current_dir()
        .map_err(|error| format!("failed to resolve current working directory: {error}"))?;

    match lookup {
        LookupArgs::Path(path) => {
            let lookup = lookup_markdown_context_index(&working_dir, &path)
                .map_err(|error| format!("failed to lookup markdown index: {error}"))?
                .ok_or_else(|| format!("no markdown index record found for {}", path.display()))?;
            println!(
                "{}",
                serde_json::to_string_pretty(&lookup)
                    .map_err(|error| format!("failed to serialize index lookup: {error}"))?
            );
        }
        LookupArgs::Tag(tag) => {
            let records = lookup_markdown_contexts_by_tag(&working_dir, &tag)
                .map_err(|error| format!("failed to lookup markdown index by tag: {error}"))?;
            println!(
                "{}",
                serde_json::to_string_pretty(&records)
                    .map_err(|error| format!("failed to serialize tag lookup: {error}"))?
            );
        }
    }

    Ok(())
}

fn watch_contexts(args: Vec<String>) -> Result<(), String> {
    let options = parse_watch_args(args)?;
    let working_dir = env::current_dir()
        .map_err(|error| format!("failed to resolve current working directory: {error}"))?;

    if options.once {
        let roots = ctx_core::configured_context_watch_roots(&working_dir)
            .map_err(|error| format!("failed to resolve context watch roots: {error}"))?;
        let snapshot = snapshot_context_directories(&roots)
            .map_err(|error| format!("failed to snapshot watched context directories: {error}"))?;
        for event in
            ctx_core::diff_context_file_snapshots(&ContextFileSnapshot::default(), &snapshot)
        {
            println!(
                "{}",
                serde_json::to_string(&event)
                    .map_err(|error| format!("failed to serialize watch event: {error}"))?
            );
        }
        return Ok(());
    }

    let (_stop_tx, stop_rx) = mpsc::channel();
    watch_context_directories(
        &working_dir,
        Duration::from_millis(options.interval_ms),
        stop_rx,
        |event| match serde_json::to_string(&event) {
            Ok(json) => println!("{json}"),
            Err(error) => eprintln!("failed to serialize watch event: {error}"),
        },
    )
    .map_err(|error| format!("context watch failed: {error}"))
}

#[derive(Debug, Clone, Eq, PartialEq)]
enum LookupArgs {
    Path(PathBuf),
    Tag(String),
}

fn parse_lookup_args(args: &[String]) -> Result<LookupArgs, String> {
    match args {
        [flag, value] if flag == "--path" || flag == "-p" => {
            Ok(LookupArgs::Path(PathBuf::from(value)))
        }
        [flag, value] if flag == "--tag" || flag == "-t" => Ok(LookupArgs::Tag(value.to_string())),
        [path] if !path.starts_with('-') => Ok(LookupArgs::Path(PathBuf::from(path))),
        _ => Err("usage: ctx lookup (--path <markdown-path>|--tag <tag>)".to_string()),
    }
}

fn classify_import_content(args: Vec<String>) -> Result<(), String> {
    let file_path = parse_classify_file_arg(&args)?;
    let (content, file_name, folder_path) = match file_path {
        Some(path) => {
            let content = fs::read_to_string(&path).map_err(|error| {
                format!(
                    "failed to read markdown content from {}: {error}",
                    path.display()
                )
            })?;
            let file_name = path
                .file_name()
                .and_then(|name| name.to_str())
                .map(str::to_string);
            let folder_path = path.parent().map(Path::to_path_buf);
            (content, file_name, folder_path)
        }
        None => {
            let mut content = String::new();
            io::stdin()
                .read_to_string(&mut content)
                .map_err(|error| format!("failed to read markdown content from stdin: {error}"))?;
            (content, None, None)
        }
    };

    if content.trim().is_empty() {
        return Err("markdown content cannot be empty for import classification".to_string());
    }

    let suggestion = classify_import_markdown_content(&ImportTimeClassificationRequest {
        content,
        file_name,
        folder_path,
        import_source_type: None,
        existing_tags: Vec::new(),
    });

    println!(
        "{}\t{}\t{}",
        classification_label(suggestion.classification),
        suggestion.confidence_score,
        suggestion.rationale
    );
    Ok(())
}

fn parse_classify_file_arg(args: &[String]) -> Result<Option<PathBuf>, String> {
    match args {
        [] => Ok(None),
        [flag, path] if flag == "--file" || flag == "-f" => Ok(Some(PathBuf::from(path))),
        [path] if !path.starts_with('-') => Ok(Some(PathBuf::from(path))),
        _ => Err("usage: ctx classify [--file <markdown-path>] < markdown".to_string()),
    }
}

fn parse_watch_args(args: Vec<String>) -> Result<WatchArgs, String> {
    let mut once = false;
    let mut interval_ms = 1000;
    let mut iter = args.into_iter();

    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--once" => once = true,
            "--interval-ms" => {
                let value = iter
                    .next()
                    .ok_or_else(|| "--interval-ms requires a positive integer".to_string())?;
                interval_ms = parse_watch_interval_ms(&value)?;
            }
            value if value.starts_with("--interval-ms=") => {
                interval_ms = parse_watch_interval_ms(
                    value.strip_prefix("--interval-ms=").unwrap_or_default(),
                )?;
            }
            "-h" | "--help" => {
                return Err("usage: ctx watch [--once] [--interval-ms <milliseconds>]".to_string())
            }
            other => return Err(format!("unknown ctx watch option: {other}")),
        }
    }

    Ok(WatchArgs { once, interval_ms })
}

fn parse_watch_interval_ms(value: &str) -> Result<u64, String> {
    let parsed = value
        .parse::<u64>()
        .map_err(|_| "--interval-ms requires a positive integer".to_string())?;
    if parsed == 0 {
        return Err("--interval-ms must be greater than 0".to_string());
    }
    Ok(parsed)
}

fn discover_agent_sessions() -> Result<Vec<AgentSessionSummary>, String> {
    let home = home_dir()?;
    let working_dir = env::current_dir()
        .map_err(|error| format!("failed to resolve current working directory: {error}"))?;
    let mut sessions = Vec::new();
    sessions.extend(list_codex_sessions(&home, &working_dir)?);
    sessions.extend(list_claude_sessions(&home, &working_dir)?);
    sessions.sort_by(|left, right| right.updated_at.cmp(&left.updated_at));
    Ok(sessions)
}

fn resolve_session_detail(session_ref: &str) -> Result<AgentSessionDetail, String> {
    let sessions = discover_agent_sessions()?;
    let session = if session_ref == "latest" {
        sessions
            .first()
            .cloned()
            .ok_or_else(|| "No Codex or Claude resume sessions found.".to_string())?
    } else {
        sessions
            .iter()
            .find(|session| {
                session.session_id == session_ref
                    || session.session_id.starts_with(session_ref)
                    || session.file_path.display().to_string() == session_ref
            })
            .cloned()
            .ok_or_else(|| format!("No session matched '{session_ref}'."))?
    };

    match session.provider.as_str() {
        "codex" => parse_codex_session_log_detail(&session.file_path, None)
            .map_err(|error| error.to_string()),
        "claude" => {
            parse_claude_session_log_detail(&session.file_path).map_err(|error| error.to_string())
        }
        other => Err(format!("unsupported session provider: {other}")),
    }
}

fn resolve_session_context_fragment(session_ref: &str) -> Result<ContextFragment, String> {
    let detail = resolve_session_detail(session_ref)?;
    let classification = classify_work_context_detail(&detail)
        .map_err(|error| format!("failed to classify selected session context: {error}"))?;
    Ok(session_detail_to_context_fragment(&detail, &classification))
}

fn resolve_saved_handoff_context_fragment(
    working_dir: &Path,
    handoff_ref: &str,
) -> Result<ContextFragment, String> {
    let saved = read_resolved_session_handoff_context(working_dir, &PathBuf::from(handoff_ref))
        .map_err(|error| {
            format!("failed to read selected saved session handoff '{handoff_ref}': {error}")
        })?;
    let mut fragment = saved.fragment;
    fragment.title = saved.handoff.title.clone();
    fragment.content = saved.handoff.handoff_markdown.clone();
    fragment.session_handoff_classification =
        Some(session_handoff_metadata_from_handoff(&saved.handoff));
    Ok(fragment)
}

fn save_session_context(
    detail: &AgentSessionDetail,
    content: &str,
    classification: &WorkContextClassificationResult,
    launch_target: CliTarget,
    refine_mode: WorkContextRefineMode,
) -> Result<ContextFragment, String> {
    let working_dir = env::current_dir()
        .map_err(|error| format!("failed to resolve current working directory: {error}"))?;
    let roots = ctx_core::VaultRoots::discover(&working_dir);
    let file_name = format!(
        "{}-{}.md",
        sanitize_file_name(&detail.summary.provider),
        sanitize_file_name(&detail.summary.session_id)
    );
    let signal_set = WorkContextSignalSet::from_session_detail(detail)
        .map_err(|error| format!("failed to normalize session handoff context: {error}"))?;
    let handoff = SessionHandoffContext::from_classified_signals(
        &signal_set,
        classification,
        current_timestamp_string()?,
        content,
        launch_target,
        refine_mode,
    )
    .map_err(|error| format!("failed to extract classified session handoff fields: {error}"))?;
    create_session_handoff_context_file(
        &roots,
        VaultScope::Local,
        PathBuf::from("session-history"),
        &file_name,
        &handoff,
    )
    .map(|saved| saved.fragment)
    .map_err(|error| format!("failed to save distilled session context: {error}"))
}

fn default_launch_target_for_session_detail(detail: &AgentSessionDetail) -> CliTarget {
    match detail.summary.provider_kind() {
        Some(SessionLogProvider::Claude) => CliTarget::Claude,
        Some(SessionLogProvider::Codex) | None => CliTarget::Codex,
    }
}

fn current_timestamp_string() -> Result<String, String> {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| format!("failed to resolve current timestamp: {error}"))?
        .as_secs();
    Ok(format!("unix-seconds:{seconds}"))
}

fn add_session_category_frontmatter(
    content: &str,
    classification: &WorkContextClassificationResult,
) -> String {
    let body = strip_existing_frontmatter(content).trim_start();
    let tags = session_context_tags(classification);

    format!(
        "---\nclassification: shared\ntags: [{}]\nsource_tool: {}\nsource_session_ref: {}\nsource_working_directory: {}\nsource_log_path: {}\nwork_context_category: {}\nwork_context_categories: [{}]\nwork_context_classification_status: {}\nwork_context_confidence_score: {}\nwork_context_rationale: {}\ndistillation_focus: [{}]\n---\n\n{}",
        tags.join(", "),
        classification.source_tool.as_str(),
        yaml_quoted(&classification.source_session_ref),
        yaml_quoted(&classification.source_working_directory),
        yaml_quoted(&classification.source_log_path),
        classification.category.as_str(),
        work_context_category_list(&classification.categories),
        classification_status_label(classification.status),
        classification.confidence_score,
        yaml_quoted(&classification.rationale),
        classification
            .distillation_focus
            .iter()
            .map(|focus| yaml_quoted(focus))
            .collect::<Vec<_>>()
            .join(", "),
        body
    )
}

fn yaml_quoted(value: &str) -> String {
    format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
}

fn classification_status_label(status: ctx_core::ClassificationStatus) -> &'static str {
    match status {
        ctx_core::ClassificationStatus::Pending => "pending",
        ctx_core::ClassificationStatus::Classified => "classified",
        ctx_core::ClassificationStatus::Reviewed => "reviewed",
        ctx_core::ClassificationStatus::Modified => "modified",
    }
}

fn strip_existing_frontmatter(content: &str) -> &str {
    let Some(rest) = content.strip_prefix("---\n") else {
        return content;
    };
    let Some(end_index) = rest.find("\n---") else {
        return content;
    };
    let after_marker = &rest[end_index + "\n---".len()..];
    after_marker.strip_prefix('\n').unwrap_or(after_marker)
}

fn session_context_tags(classification: &WorkContextClassificationResult) -> Vec<String> {
    let mut tags = classification.suggested_tags.clone();
    for tag in [
        "session-history".to_string(),
        "resume-context".to_string(),
        classification.source_tool.as_str().to_string(),
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

fn work_context_category_list(categories: &[WorkContextCategory]) -> String {
    categories
        .iter()
        .map(|category| category.as_str())
        .collect::<Vec<_>>()
        .join(", ")
}

fn saved_context_category_list(context: &ContextFragment) -> String {
    let mut category_tags = context
        .tags
        .iter()
        .filter(|tag| tag_is_work_context_category(tag))
        .cloned()
        .collect::<Vec<_>>();
    if category_tags.is_empty() {
        category_tags = saved_context_categories_from_frontmatter(&context.content);
    }
    if category_tags.is_empty() {
        "-".to_string()
    } else {
        category_tags.join(", ")
    }
}

fn saved_context_categories_from_frontmatter(content: &str) -> Vec<String> {
    let Some(rest) = content.strip_prefix("---\n") else {
        return Vec::new();
    };
    let Some(end_index) = rest.find("\n---") else {
        return Vec::new();
    };
    let frontmatter = &rest[..end_index];
    let mut primary_category = None;
    let mut categories = Vec::new();

    for line in frontmatter.lines() {
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let key = key.trim();
        let value = value.trim();
        if key == "work_context_categories" {
            categories = parse_work_context_category_list(value);
        } else if key == "work_context_category" {
            primary_category = parse_work_context_category_token(value);
        }
    }

    if categories.is_empty() {
        if let Some(category) = primary_category {
            categories.push(category);
        }
    }
    categories
}

fn parse_work_context_category_list(value: &str) -> Vec<String> {
    let trimmed = value
        .trim()
        .strip_prefix('[')
        .and_then(|inner| inner.strip_suffix(']'))
        .unwrap_or(value);

    trimmed
        .split(',')
        .filter_map(parse_work_context_category_token)
        .collect()
}

fn parse_work_context_category_token(value: &str) -> Option<String> {
    let category = value.trim().trim_matches('"').trim_matches('\'');
    tag_is_work_context_category(category).then(|| category.to_string())
}

fn tag_is_work_context_category(tag: &str) -> bool {
    matches!(
        tag,
        "implementation"
            | "debugging"
            | "review"
            | "planning"
            | "refactor"
            | "research"
            | "verification"
            | "launch"
            | "general"
    )
}

fn refine_distilled_session_context(target: CliTarget, draft: &str) -> Result<String, String> {
    if draft.trim().is_empty() {
        return Err("cannot refine an empty session draft".to_string());
    }

    let prompt = build_session_refinement_prompt(draft);
    let output = match target {
        CliTarget::Claude => {
            let program = resolve_claude_command()?;
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
            let program = resolve_codex_command()?;
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
        return Err(format!(
            "refinement CLI exited with {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
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

fn build_session_refinement_prompt(draft: &str) -> String {
    format!(
        r#"You are preparing context for a new coding-agent session.

Rewrite the raw previous-session transcript into a concise Korean handoff note.
Do not invent facts. Preserve concrete filenames, commands, decisions, verification results, blockers, and remaining work when present.
Remove noisy tool output and repeated status chatter.

Return only markdown with this structure:

# 새 세션 시작 컨텍스트

## 목표

## 완료한 작업

## 변경한 파일

## 중요한 결정

## 검증 결과

## 남은 작업

## 주의사항

## 새 세션 시작 프롬프트

Raw previous-session draft:

```markdown
{draft}
```
"#
    )
}

fn session_detail_to_context_fragment(
    detail: &AgentSessionDetail,
    classification: &WorkContextClassificationResult,
) -> ContextFragment {
    let tags = session_context_tags(classification);
    let filtered = filter_work_relevant_content(detail).ok();
    let body = filtered
        .as_ref()
        .map(|content| content.handoff_markdown.as_str())
        .unwrap_or(detail.distilled_markdown.as_str());
    let content = add_session_category_frontmatter(body, classification);
    ContextFragment {
        context_id: Uuid::new_v4(),
        title: classification.title.clone(),
        content,
        file_path: detail.summary.file_path.clone(),
        vault_scope: VaultScope::Local,
        classification: Classification::Shared,
        import_classification_suggestion: Some(Classification::Shared),
        inferred_classification: Some(Classification::Shared),
        tags,
        folder_path: PathBuf::from("session-history"),
        wikilinks: Vec::new(),
        backlinks: Vec::new(),
        import_source: Some(detail.summary.file_path.clone()),
        import_source_type: None,
        llm_classification_status: classification.status,
        session_handoff_classification: Some(session_handoff_metadata_from_classification(
            classification,
        )),
    }
}

fn session_handoff_metadata_from_classification(
    classification: &WorkContextClassificationResult,
) -> SessionHandoffClassificationMetadata {
    SessionHandoffClassificationMetadata {
        source_tool: classification.source_tool.as_str().to_string(),
        source_session_ref: classification.source_session_ref.clone(),
        source_working_directory: classification.source_working_directory.clone(),
        source_log_path: classification.source_log_path.clone(),
        work_context_category: classification.category.as_str().to_string(),
        work_context_categories: classification
            .categories
            .iter()
            .map(|category| category.as_str().to_string())
            .collect(),
        work_context_classification_status: classification.status,
        work_context_confidence_score: classification.confidence_score,
        work_context_rationale: classification.rationale.clone(),
        distillation_focus: classification.distillation_focus.clone(),
    }
}

fn session_handoff_metadata_from_handoff(
    handoff: &SessionHandoffContext,
) -> SessionHandoffClassificationMetadata {
    SessionHandoffClassificationMetadata {
        source_tool: handoff.source_tool.as_str().to_string(),
        source_session_ref: handoff.source_session_ref.clone(),
        source_working_directory: handoff.source_working_directory.clone(),
        source_log_path: handoff.source_log_path.clone(),
        work_context_category: handoff.category.as_str().to_string(),
        work_context_categories: handoff
            .categories
            .iter()
            .map(|category| category.as_str().to_string())
            .collect(),
        work_context_classification_status: handoff.classification_status,
        work_context_confidence_score: handoff.classification_confidence_score,
        work_context_rationale: handoff.classification_rationale.clone(),
        distillation_focus: Vec::new(),
    }
}

fn home_dir() -> Result<PathBuf, String> {
    env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or_else(|| "HOME directory is not available".to_string())
}

fn list_codex_sessions(
    home_dir: &Path,
    working_dir: &Path,
) -> Result<Vec<AgentSessionSummary>, String> {
    let roots = ctx_core::VaultRoots::discover(working_dir);
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
    let roots = ctx_core::VaultRoots::discover(working_dir);
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

fn format_session_row(session: &AgentSessionSummary) -> String {
    format!(
        "{}\t{}\t{}\t{}\t{}\t{}",
        session.provider,
        session.updated_at.as_deref().unwrap_or("unknown"),
        session.message_count,
        session.session_id,
        session.cwd.as_deref().unwrap_or("-"),
        session.title.replace('\n', " ")
    )
}

fn sanitize_file_name(value: &str) -> String {
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

fn format_context_list_row(context: &ContextFragment) -> String {
    let source = context
        .import_source_type
        .map(import_source_type_label)
        .unwrap_or(if context.import_source.is_some() {
            "discovered"
        } else {
            "vault"
        });

    format!(
        "{}\t{}\t{}\t{}\t{}\t{}",
        source,
        scope_label(context.vault_scope),
        classification_label(context.classification),
        inferred_classification_label(context.inferred_classification),
        context.title,
        context.file_path.display()
    )
}

fn format_import_candidate_row(context: &ctx_core::ContextDiscoveryResult) -> String {
    format!(
        "{}\t{}\t{}\t{}\t{}\t{}\t{}",
        scope_label(context.metadata.vault_scope),
        classification_label(context.metadata.classification),
        inferred_classification_label(context.metadata.inferred_classification),
        import_source_type_label(context.source_type),
        context.file_name,
        context.root_source.display(),
        context.file_path.display()
    )
}

fn scope_label(scope: ctx_core::VaultScope) -> &'static str {
    match scope {
        ctx_core::VaultScope::Global => "global",
        ctx_core::VaultScope::Local => "local",
    }
}

fn classification_label(classification: ctx_core::Classification) -> &'static str {
    match classification {
        ctx_core::Classification::MainAgent => "main-agent",
        ctx_core::Classification::Subagent => "subagent",
        ctx_core::Classification::Shared => "shared",
    }
}

fn inferred_classification_label(classification: Option<ctx_core::Classification>) -> &'static str {
    classification
        .map(classification_label)
        .unwrap_or("unknown")
}

fn import_source_type_label(source_type: ctx_core::ImportSourceType) -> &'static str {
    match source_type {
        ctx_core::ImportSourceType::ContextMarkdown => "context-md",
        ctx_core::ImportSourceType::ClaudeMarkdown => "claude-md",
        ctx_core::ImportSourceType::CodexAgents => "codex-agents",
        ctx_core::ImportSourceType::AgentMarkdown => "agent-md",
        ctx_core::ImportSourceType::AgentsManifest => "agents-manifest",
        ctx_core::ImportSourceType::SkillMarkdown => "skill-md",
        ctx_core::ImportSourceType::SkillManifest => "skill-manifest",
        ctx_core::ImportSourceType::SubagentMarkdown => "subagent-md",
    }
}

fn parse_target(value: Option<&str>) -> Result<CliTarget, String> {
    match value {
        Some("claude") => Ok(CliTarget::Claude),
        Some("codex") => Ok(CliTarget::Codex),
        Some(other) => Err(format!(
            "unsupported launch target: {other}. Expected claude or codex."
        )),
        None => Err("missing launch target. Usage: ctx launch <claude|codex>".to_string()),
    }
}

fn parse_launch_args(args: Vec<String>) -> Result<LaunchArgs, String> {
    let mut preset_ref = None;
    let mut session_ref = None;
    let mut handoff_ref = None;
    let mut passthrough_args = Vec::new();
    let mut iter = args.into_iter();
    let mut passthrough_only = false;

    while let Some(arg) = iter.next() {
        if passthrough_only {
            passthrough_args.push(arg);
            continue;
        }

        match arg.as_str() {
            "--" => passthrough_only = true,
            "--preset" => {
                let value = iter.next().ok_or_else(|| {
                    "--preset requires a preset id, file stem, or name".to_string()
                })?;
                if value.trim().is_empty() {
                    return Err("--preset requires a preset id, file stem, or name".to_string());
                }
                if preset_ref.replace(value).is_some() {
                    return Err("--preset can only be supplied once".to_string());
                }
            }
            "--session" => {
                let value = iter
                    .next()
                    .ok_or_else(|| "--session requires latest or a session id".to_string())?;
                if value.trim().is_empty() {
                    return Err("--session requires latest or a session id".to_string());
                }
                if session_ref.replace(value).is_some() {
                    return Err("--session can only be supplied once".to_string());
                }
            }
            "--handoff" | "--saved" => {
                let value = iter.next().ok_or_else(|| {
                    "--handoff requires a saved session handoff markdown path".to_string()
                })?;
                if value.trim().is_empty() {
                    return Err(
                        "--handoff requires a saved session handoff markdown path".to_string()
                    );
                }
                if handoff_ref.replace(value).is_some() {
                    return Err("--handoff can only be supplied once".to_string());
                }
            }
            value if value.starts_with("--preset=") => {
                let value = value
                    .strip_prefix("--preset=")
                    .unwrap_or_default()
                    .to_string();
                if value.trim().is_empty() {
                    return Err("--preset requires a preset id, file stem, or name".to_string());
                }
                if preset_ref.replace(value).is_some() {
                    return Err("--preset can only be supplied once".to_string());
                }
            }
            value if value.starts_with("--session=") => {
                let value = value
                    .strip_prefix("--session=")
                    .unwrap_or_default()
                    .to_string();
                if value.trim().is_empty() {
                    return Err("--session requires latest or a session id".to_string());
                }
                if session_ref.replace(value).is_some() {
                    return Err("--session can only be supplied once".to_string());
                }
            }
            value if value.starts_with("--handoff=") || value.starts_with("--saved=") => {
                let value = value
                    .split_once('=')
                    .map(|(_, value)| value.to_string())
                    .unwrap_or_default();
                if value.trim().is_empty() {
                    return Err(
                        "--handoff requires a saved session handoff markdown path".to_string()
                    );
                }
                if handoff_ref.replace(value).is_some() {
                    return Err("--handoff can only be supplied once".to_string());
                }
            }
            _ => passthrough_args.push(arg),
        }
    }

    Ok(LaunchArgs {
        preset_ref,
        session_ref,
        handoff_ref,
        passthrough_args,
    })
}

fn print_help() {
    println!("ctx - reuse previous Codex/Claude sessions as new-session context");
    println!();
    println!("Usage:");
    println!("  ctx init [--global|--local]");
    println!("  ctx status");
    println!("  ctx cleanup");
    println!("  ctx list");
    println!("  ctx scan");
    println!("  ctx saved");
    println!("  ctx classify [latest|<session-id>]");
    println!("  ctx distill [latest|<session-id>] [--refine] [--refiner <claude|codex>] [--save]");
    println!("  ctx launch <claude|codex> [--session <latest|session-id>|--handoff <saved-md-path>] [--preset <id|file-stem|name>] [-- cli args...]");
    println!("  ctx context <list|scan|import|classify|reindex|lookup|watch>");
}

fn print_context_help() {
    println!("ctx context - manage markdown context vault files");
    println!();
    println!("Usage:");
    println!("  ctx context list");
    println!("  ctx context scan");
    println!("  ctx context import");
    println!("  ctx context classify [--file <markdown-path>] < markdown");
    println!("  ctx context reindex");
    println!("  ctx context lookup (--path <markdown-path>|--tag <tag>)");
    println!("  ctx context watch [--once] [--interval-ms <milliseconds>]");
}

#[cfg(test)]
mod tests {
    use super::*;
    use ctx_core::{
        create_context_file, list_context_files, managed_presets_dir, Classification,
        ClassificationStatus, InjectionStrategy, SessionLogMessage as AgentSessionMessage,
        VaultRoots, VaultScope, AGENTS_MD_FILE_NAME, CTX_END_MARKER, CTX_START_MARKER,
        MAX_SUBAGENT_MANIFEST_JSON_BYTES,
    };
    use std::sync::{Mutex, MutexGuard};
    use uuid::Uuid;

    static HOME_ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn saved_session_context_persists_categories_in_frontmatter_for_listing() {
        let workspace = temp_workspace();
        let roots = VaultRoots {
            global_root: workspace.join("global-vault"),
            local_root: Some(workspace.join(".ctx").join("vault")),
        };
        let detail = AgentSessionDetail {
            summary: AgentSessionSummary {
                provider: "codex".to_string(),
                session_id: "category-save-1".to_string(),
                title: "Implement launch handoff".to_string(),
                updated_at: Some("2026-05-11T00:00:00Z".to_string()),
                cwd: Some(workspace.display().to_string()),
                file_path: workspace.join("category-save-1.jsonl"),
                message_count: 1,
                last_user_message: Some("Implement launch handoff".to_string()),
            },
            messages: vec![AgentSessionMessage {
                role: "assistant".to_string(),
                timestamp: Some("2026-05-11T00:01:00Z".to_string()),
                content: "Summary: Implemented Codex launch handoff injection.\nVerified with cargo test -p ctx-core work_context::tests.".to_string(),
            }],
            events: Vec::new(),
            distilled_markdown:
                "---\ntags: [old]\n---\n\n# Previous Session Context\n\nLaunch work.".to_string(),
        };
        let classification =
            classify_work_context_detail(&detail).expect("session detail should classify");
        let expected_categories = work_context_category_list(&classification.categories);
        let content = add_session_category_frontmatter(&detail.distilled_markdown, &classification);

        assert!(content.contains(&format!(
            "work_context_category: {}",
            classification.category.as_str()
        )));
        assert!(content.contains(&format!("work_context_categories: [{expected_categories}]")));
        assert!(content.contains("source_tool: codex"));
        assert!(content.contains("source_session_ref: \"category-save-1\""));
        assert!(content.contains("work_context_classification_status: classified"));
        assert!(content.contains("work_context_confidence_score: "));
        assert!(content.contains("work_context_rationale: \""));
        assert!(content.contains("distillation_focus: ["));
        assert!(content.contains("tags: [session-history, resume-context, codex"));
        assert!(!content.contains("tags: [old]"));

        create_context_file(
            &roots,
            VaultScope::Local,
            "session-history",
            "codex-category-save-1.md",
            &content,
        )
        .expect("saved session context should be created");

        let contexts = list_context_files(&roots).expect("saved contexts should list");
        let saved = contexts
            .iter()
            .find(|context| context.file_path.ends_with("codex-category-save-1.md"))
            .expect("saved context should be listed after reload");

        assert_eq!(saved_context_category_list(saved), expected_categories);
        let metadata = saved
            .session_handoff_classification
            .as_ref()
            .expect("saved session context should reload handoff classification metadata");
        assert_eq!(metadata.source_tool, "codex");
        assert_eq!(metadata.source_session_ref, "category-save-1");
        assert_eq!(
            metadata.work_context_category,
            classification.category.as_str()
        );
        assert_eq!(
            metadata.work_context_categories,
            classification
                .categories
                .iter()
                .map(|category| category.as_str().to_string())
                .collect::<Vec<_>>()
        );
        assert_eq!(
            metadata.work_context_classification_status,
            classification.status
        );
        assert_eq!(
            metadata.work_context_confidence_score,
            classification.confidence_score
        );
        assert_eq!(metadata.work_context_rationale, classification.rationale);
        assert!(saved.content.contains("tags: [session-history"));
        assert!(saved.content.contains("work_context_categories: ["));
    }

    #[test]
    fn save_session_context_serializes_full_validated_handoff_entry() {
        let _guard = HOME_ENV_LOCK
            .lock()
            .expect("HOME env lock should not be poisoned");
        let workspace = temp_workspace();
        let previous_dir = env::current_dir().expect("current dir should resolve");
        env::set_current_dir(&workspace).expect("test workspace should become current dir");

        let detail = AgentSessionDetail {
            summary: AgentSessionSummary {
                provider: "claude".to_string(),
                session_id: "save-full-1".to_string(),
                title: "Save full handoff entry".to_string(),
                updated_at: Some("2026-05-11T00:00:00Z".to_string()),
                cwd: Some(workspace.display().to_string()),
                file_path: workspace.join("save-full-1.jsonl"),
                message_count: 1,
                last_user_message: Some("Save full handoff entry".to_string()),
            },
            messages: vec![AgentSessionMessage {
                role: "assistant".to_string(),
                timestamp: Some("2026-05-11T00:01:00Z".to_string()),
                content: "Summary: Saved complete reusable handoff metadata.\nChanged files: crates/ctx-cli/src/bin/ctx.rs.\nDecision: Validate before writing session handoffs.\nVerified with cargo test -p ctx-cli save_session_context_serializes_full_validated_handoff_entry.\nRemaining work: launch saved entries."
                    .to_string(),
            }],
            events: Vec::new(),
            distilled_markdown: "# Previous Session Context".to_string(),
        };
        let classification =
            classify_work_context_detail(&detail).expect("session detail should classify");

        let saved = save_session_context(
            &detail,
            "---\ntags: [stale]\n---\n\n# Previous Session Context\n\n## Handoff Summary\n\nSaved complete reusable handoff metadata.\n\n### Goals\n\n- Save full handoff entry\n\n### Key changed files\n\n- crates/ctx-cli/src/bin/ctx.rs\n\n### Decisions\n\n- Validate before writing session handoffs.\n\n### Verification results\n\n- Verified with cargo test -p ctx-cli save_session_context_serializes_full_validated_handoff_entry.\n\n### Remaining work\n\n- launch saved entries.",
            &classification,
            CliTarget::Claude,
            WorkContextRefineMode::Raw,
        )
        .expect("complete session handoff should save");

        env::set_current_dir(previous_dir).expect("test cwd should be restored");

        let content =
            fs::read_to_string(saved.file_path).expect("saved handoff markdown should be readable");
        assert!(content.contains("session_handoff_format_version: 1"));
        assert!(content.contains("source_tool: claude"));
        assert!(content.contains("source_session_ref: \"save-full-1\""));
        assert!(content.contains("summary: \"Saved complete reusable handoff metadata.\""));
        assert!(content.contains("key_changed_files: [\"crates/ctx-cli/src/bin/ctx.rs\"]"));
        assert!(content.contains("decisions: [\"Validate before writing session handoffs.\"]"));
        assert!(content.contains("verification_results: [\"Verified with cargo test -p ctx-cli save_session_context_serializes_full_validated_handoff_entry.\"]"));
        assert!(content.contains("remaining_work: [\"launch saved entries.\"]"));
        assert!(content.contains("launch_target: claude"));
        assert!(content.contains("injection_method: append-system-prompt-file"));
        assert!(content.ends_with("- launch saved entries."));
        assert!(!content.contains("tags: [stale]"));
    }

    #[test]
    fn save_session_context_preserves_unrelated_saved_entries() {
        let _guard = HOME_ENV_LOCK
            .lock()
            .expect("HOME env lock should not be poisoned");
        let workspace = temp_workspace();
        let roots = VaultRoots {
            global_root: workspace.join("global-vault"),
            local_root: Some(workspace.join(".ctx").join("vault")),
        };
        let unrelated_content = "---\nclassification: shared\ntags: [session-history]\n---\n\n# Existing Handoff\n\nKeep this entry.";
        let unrelated = create_context_file(
            &roots,
            VaultScope::Local,
            "session-history",
            "codex-existing-session.md",
            unrelated_content,
        )
        .expect("unrelated saved session entry should be created");

        let previous_dir = env::current_dir().expect("current dir should resolve");
        env::set_current_dir(&workspace).expect("test workspace should become current dir");
        let detail = AgentSessionDetail {
            summary: AgentSessionSummary {
                provider: "codex".to_string(),
                session_id: "new-save-1".to_string(),
                title: "Persist new handoff entry".to_string(),
                updated_at: Some("2026-05-11T00:00:00Z".to_string()),
                cwd: Some(workspace.display().to_string()),
                file_path: workspace.join("new-save-1.jsonl"),
                message_count: 1,
                last_user_message: Some("Persist new handoff entry".to_string()),
            },
            messages: vec![AgentSessionMessage {
                role: "assistant".to_string(),
                timestamp: Some("2026-05-11T00:01:00Z".to_string()),
                content: "Summary: Saved a new reusable handoff.\nDecision: Preserve existing saved entries.\nVerified with cargo test -p ctx-cli save_session_context_preserves_unrelated_saved_entries."
                    .to_string(),
            }],
            events: Vec::new(),
            distilled_markdown: "# Previous Session Context".to_string(),
        };
        let classification =
            classify_work_context_detail(&detail).expect("session detail should classify");

        let saved = save_session_context(
            &detail,
            "# Previous Session Context\n\n## Handoff Summary\n\nSaved a new reusable handoff.\n\n### Goals\n\n- Persist new handoff entry\n\n### Decisions\n\n- Preserve existing saved entries.\n\n### Verification results\n\n- Verified with cargo test -p ctx-cli save_session_context_preserves_unrelated_saved_entries.",
            &classification,
            CliTarget::Codex,
            WorkContextRefineMode::Raw,
        )
        .expect("new session handoff should save beside existing entries");

        env::set_current_dir(previous_dir).expect("test cwd should be restored");

        assert_ne!(saved.file_path, unrelated.file_path);
        assert_eq!(
            fs::read_to_string(&unrelated.file_path)
                .expect("unrelated saved entry should remain readable"),
            unrelated_content
        );
        assert!(fs::read_to_string(&saved.file_path)
            .expect("new saved handoff should be readable")
            .contains("source_session_ref: \"new-save-1\""));

        let contexts = list_context_files(&roots).expect("saved session entries should list");
        assert_eq!(
            contexts
                .iter()
                .filter(|context| context.folder_path == PathBuf::from("session-history"))
                .count(),
            2
        );
    }

    struct MockCommandLaunchHarness {
        _guard: MutexGuard<'static, ()>,
        workspace: PathBuf,
        home: PathBuf,
        bin_dir: PathBuf,
        claude_log: PathBuf,
        codex_log: PathBuf,
        previous_path: Option<OsString>,
        previous_home: Option<OsString>,
        previous_claude_bin: Option<OsString>,
        previous_codex_bin: Option<OsString>,
        previous_dir: PathBuf,
    }

    impl MockCommandLaunchHarness {
        fn new() -> Self {
            let guard = HOME_ENV_LOCK
                .lock()
                .expect("process env lock should not be poisoned");
            let workspace = temp_workspace();
            let home = temp_workspace();
            let bin_dir = workspace.join("bin");
            fs::create_dir_all(&bin_dir).expect("mock bin dir should be created");

            let claude_log = workspace.join("mock-claude-args.log");
            let codex_log = workspace.join("mock-codex-args.log");
            write_mock_child_executable(
                &bin_dir.join("claude"),
                &format!(
                    "#!/bin/sh\nprintf '%s\\n' \"$@\" > '{}'\nexit 0\n",
                    claude_log.display()
                ),
            );
            write_mock_child_executable(
                &bin_dir.join("codex"),
                &format!(
                    "#!/bin/sh\nprintf '%s\\n' \"$@\" > '{}'\n[ ! -f AGENTS.md ] || cp AGENTS.md '{}'\nexit 0\n",
                    codex_log.display(),
                    workspace.join("mock-codex-agents-snapshot.md").display()
                ),
            );

            let previous_path = env::var_os("PATH");
            let previous_home = env::var_os("HOME");
            let previous_claude_bin = env::var_os(CTX_CLAUDE_BIN_ENV);
            let previous_codex_bin = env::var_os(CTX_CODEX_BIN_ENV);
            let previous_dir = env::current_dir().expect("current dir should resolve");

            env::set_var("HOME", &home);
            env::remove_var(CTX_CLAUDE_BIN_ENV);
            env::remove_var(CTX_CODEX_BIN_ENV);
            env::set_var("PATH", prepend_path(&bin_dir, previous_path.as_ref()));
            env::set_current_dir(&workspace).expect("mock launch cwd should be set");

            Self {
                _guard: guard,
                workspace,
                home,
                bin_dir,
                claude_log,
                codex_log,
                previous_path,
                previous_home,
                previous_claude_bin,
                previous_codex_bin,
                previous_dir,
            }
        }

        fn workspace(&self) -> &Path {
            &self.workspace
        }

        fn home(&self) -> &Path {
            &self.home
        }

        fn bin_dir(&self) -> &Path {
            &self.bin_dir
        }

        fn claude_args(&self) -> Vec<String> {
            read_arg_log(&self.claude_log)
        }

        fn codex_args(&self) -> Vec<String> {
            read_arg_log(&self.codex_log)
        }
    }

    impl Drop for MockCommandLaunchHarness {
        fn drop(&mut self) {
            let _ = env::set_current_dir(&self.previous_dir);
            restore_env_var("PATH", self.previous_path.as_ref());
            restore_env_var("HOME", self.previous_home.as_ref());
            restore_env_var(CTX_CLAUDE_BIN_ENV, self.previous_claude_bin.as_ref());
            restore_env_var(CTX_CODEX_BIN_ENV, self.previous_codex_bin.as_ref());
        }
    }

    #[test]
    fn parse_launch_args_consumes_preset_and_preserves_passthrough_after_separator() {
        let parsed = parse_launch_args(vec![
            "--preset".to_string(),
            "daily".to_string(),
            "--".to_string(),
            "--model".to_string(),
            "sonnet".to_string(),
        ])
        .expect("launch args should parse");

        assert_eq!(
            parsed,
            LaunchArgs {
                preset_ref: Some("daily".to_string()),
                session_ref: None,
                handoff_ref: None,
                passthrough_args: vec!["--model".to_string(), "sonnet".to_string()]
            }
        );
    }

    #[test]
    fn parse_launch_args_accepts_saved_handoff_selection() {
        let parsed = parse_launch_args(vec![
            "--handoff".to_string(),
            ".ctx/vault/contexts/session-history/codex-1.md".to_string(),
            "--".to_string(),
            "--sandbox".to_string(),
            "workspace-write".to_string(),
        ])
        .expect("launch args should accept saved handoff paths");

        assert_eq!(
            parsed,
            LaunchArgs {
                preset_ref: None,
                session_ref: None,
                handoff_ref: Some(".ctx/vault/contexts/session-history/codex-1.md".to_string()),
                passthrough_args: vec!["--sandbox".to_string(), "workspace-write".to_string()]
            }
        );
    }

    #[test]
    fn parse_launch_args_accepts_preset_name_and_rejects_blank_values() {
        let parsed = parse_launch_args(vec![
            "--preset=Daily Driver".to_string(),
            "--".to_string(),
            "--sandbox".to_string(),
            "workspace-write".to_string(),
        ])
        .expect("launch args should accept configured preset names");

        assert_eq!(parsed.preset_ref.as_deref(), Some("Daily Driver"));
        assert_eq!(
            parsed.passthrough_args,
            vec!["--sandbox".to_string(), "workspace-write".to_string()]
        );

        let error = parse_launch_args(vec!["--preset".to_string(), " ".to_string()])
            .expect_err("blank preset refs should be rejected");
        assert!(error.contains("preset id, file stem, or name"));
    }

    #[test]
    fn resolve_claude_command_finds_executable_on_path_or_env_override() {
        let workspace = temp_workspace();
        let bin_dir = workspace.join("bin");
        fs::create_dir_all(&bin_dir).expect("bin dir should be created");
        let fake_claude = bin_dir.join("claude");
        write_executable_script(&fake_claude, "#!/bin/sh\nexit 0\n");
        let path_env = OsString::from(bin_dir.display().to_string());

        let resolved = resolve_command_from_path_or_env(
            "Claude CLI",
            "claude",
            CTX_CLAUDE_BIN_ENV,
            None,
            Some(path_env.clone()),
        )
        .expect("Claude should resolve from PATH");
        assert_eq!(PathBuf::from(resolved), fake_claude);

        let resolved_override = resolve_command_from_path_or_env(
            "Claude CLI",
            "claude",
            CTX_CLAUDE_BIN_ENV,
            Some(OsString::from(fake_claude.display().to_string())),
            Some(path_env),
        )
        .expect("Claude should resolve from CTX_CLAUDE_BIN override");
        assert_eq!(PathBuf::from(resolved_override), fake_claude);
    }

    #[test]
    fn resolve_claude_command_reports_missing_executable_clearly() {
        let workspace = temp_workspace();
        let error = resolve_command_from_path_or_env(
            "Claude CLI",
            "claude",
            CTX_CLAUDE_BIN_ENV,
            None,
            Some(OsString::from(workspace.display().to_string())),
        )
        .expect_err("missing Claude executable should fail before spawn");

        assert!(error.contains("failed to resolve Claude CLI"));
        assert!(error.contains("executable 'claude' was not found on PATH"));
        assert!(error.contains(CTX_CLAUDE_BIN_ENV));
    }

    #[test]
    fn resolve_codex_command_finds_executable_on_path_or_env_override() {
        let workspace = temp_workspace();
        let bin_dir = workspace.join("bin");
        fs::create_dir_all(&bin_dir).expect("bin dir should be created");
        let fake_codex = bin_dir.join("codex");
        write_executable_script(&fake_codex, "#!/bin/sh\nexit 0\n");
        let path_env = OsString::from(bin_dir.display().to_string());

        let resolved = resolve_command_from_path_or_env(
            "Codex CLI",
            "codex",
            CTX_CODEX_BIN_ENV,
            None,
            Some(path_env.clone()),
        )
        .expect("Codex should resolve from PATH");
        assert_eq!(PathBuf::from(resolved), fake_codex);

        let resolved_override = resolve_command_from_path_or_env(
            "Codex CLI",
            "codex",
            CTX_CODEX_BIN_ENV,
            Some(OsString::from(fake_codex.display().to_string())),
            Some(path_env),
        )
        .expect("Codex should resolve from CTX_CODEX_BIN override");
        assert_eq!(PathBuf::from(resolved_override), fake_codex);
    }

    #[test]
    fn resolve_codex_command_reports_missing_executable_clearly() {
        let workspace = temp_workspace();
        let error = resolve_command_from_path_or_env(
            "Codex CLI",
            "codex",
            CTX_CODEX_BIN_ENV,
            None,
            Some(OsString::from(workspace.display().to_string())),
        )
        .expect_err("missing Codex executable should fail before spawn");

        assert!(error.contains("failed to resolve Codex CLI"));
        assert!(error.contains("executable 'codex' was not found on PATH"));
        assert!(error.contains(CTX_CODEX_BIN_ENV));
    }

    #[test]
    fn list_codex_sessions_enumerates_default_session_log_path() {
        let home = temp_workspace();
        let workspace = temp_workspace();
        let codex_session_dir = home.join(".codex").join("sessions").join("2026").join("05");
        fs::create_dir_all(&codex_session_dir).expect("Codex session log dir should be created");
        let log_path = codex_session_dir.join("codex-session-123.jsonl");
        fs::write(
            &log_path,
            r#"{"type":"session_meta","timestamp":"2026-05-10T12:00:00Z","payload":{"id":"codex-session-123","cwd":"/workspace/app","timestamp":"2026-05-10T12:00:00Z"}}
{"type":"event_msg","timestamp":"2026-05-10T12:01:00Z","payload":{"type":"user_message","message":"Implement session handoff scanning"}}"#,
        )
        .expect("Codex session log should be writable");

        with_home(&home, || {
            let sessions = list_codex_sessions(&home, &workspace)
                .expect("readable Codex session logs should enumerate");

            assert_eq!(sessions.len(), 1);
            let session = &sessions[0];
            assert_eq!(session.provider, "codex");
            assert_eq!(session.session_id, "codex-session-123");
            assert_eq!(session.updated_at.as_deref(), Some("2026-05-10T12:00:00Z"));
            assert_eq!(session.cwd.as_deref(), Some("/workspace/app"));
            assert_eq!(
                session
                    .file_path
                    .canonicalize()
                    .expect("enumerated log path should canonicalize"),
                log_path
                    .canonicalize()
                    .expect("expected log path should canonicalize")
            );
            assert_eq!(session.message_count, 1);
            assert_eq!(session.title, "Implement session handoff scanning");
        });
    }

    #[cfg(unix)]
    #[test]
    fn list_codex_sessions_skips_missing_and_unreadable_log_locations() {
        use std::os::unix::fs::PermissionsExt;

        let home = temp_workspace();
        let workspace = temp_workspace();
        let vault_root = workspace.join(".ctx").join("vault");
        let codex_session_dir = home.join(".codex").join("sessions").join("2026").join("05");
        let unreadable_configured_root = workspace.join("unreadable-codex");
        let unreadable_nested_dir = home
            .join(".codex")
            .join("sessions")
            .join("unreadable-nested");
        fs::create_dir_all(&vault_root).expect("local vault root should be created");
        fs::create_dir_all(&codex_session_dir).expect("Codex session log dir should be created");
        fs::create_dir_all(&unreadable_configured_root)
            .expect("unreadable configured root should be created");
        fs::create_dir_all(&unreadable_nested_dir)
            .expect("unreadable nested dir should be created");
        fs::write(
            ctx_core::vault_settings_path(&vault_root),
            r#"{"codex_session_roots":["missing-codex","unreadable-codex"]}"#,
        )
        .expect("local settings should be writable");
        let log_path = codex_session_dir.join("codex-session-readable.jsonl");
        fs::write(
            &log_path,
            r#"{"type":"session_meta","timestamp":"2026-05-10T12:00:00Z","payload":{"id":"codex-session-readable","cwd":"/workspace/app","timestamp":"2026-05-10T12:00:00Z"}}"#,
        )
        .expect("readable Codex session log should be writable");

        let original_configured_permissions = fs::metadata(&unreadable_configured_root)
            .expect("configured root metadata should be readable")
            .permissions();
        let original_nested_permissions = fs::metadata(&unreadable_nested_dir)
            .expect("nested root metadata should be readable")
            .permissions();
        fs::set_permissions(
            &unreadable_configured_root,
            fs::Permissions::from_mode(0o000),
        )
        .expect("configured root should be made unreadable");
        fs::set_permissions(&unreadable_nested_dir, fs::Permissions::from_mode(0o000))
            .expect("nested root should be made unreadable");

        let sessions = with_home(&home, || {
            list_codex_sessions(&home, &workspace)
                .expect("missing and unreadable Codex log locations should be skipped")
        });

        fs::set_permissions(&unreadable_configured_root, original_configured_permissions)
            .expect("configured root permissions should be restored");
        fs::set_permissions(&unreadable_nested_dir, original_nested_permissions)
            .expect("nested root permissions should be restored");

        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].session_id, "codex-session-readable");
    }

    #[test]
    fn list_claude_sessions_enumerates_readable_log_files_with_basic_metadata() {
        let home = temp_workspace();
        let workspace = temp_workspace();
        let claude_project_dir = home.join(".claude").join("projects").join("workspace-app");
        fs::create_dir_all(&claude_project_dir).expect("Claude project log dir should be created");
        let log_path = claude_project_dir.join("metadata-only.jsonl");
        fs::write(
            &log_path,
            r#"{"type":"system","sessionId":"claude-session-123","cwd":"/workspace/app","timestamp":"2026-05-10T12:00:00Z"}"#,
        )
        .expect("Claude session log should be writable");

        with_home(&home, || {
            let sessions = list_claude_sessions(&home, &workspace)
                .expect("readable Claude session logs should enumerate");

            assert_eq!(sessions.len(), 1);
            let session = &sessions[0];
            assert_eq!(session.provider, "claude");
            assert_eq!(session.session_id, "claude-session-123");
            assert_eq!(session.updated_at.as_deref(), Some("2026-05-10T12:00:00Z"));
            assert_eq!(session.cwd.as_deref(), Some("/workspace/app"));
            assert_eq!(
                session
                    .file_path
                    .canonicalize()
                    .expect("enumerated log path should canonicalize"),
                log_path
                    .canonicalize()
                    .expect("expected log path should canonicalize")
            );
            assert_eq!(session.message_count, 0);
            assert_eq!(session.title, "Claude session");
        });
    }

    #[test]
    fn mock_command_launch_harness_provides_path_resolved_claude_and_codex_children() {
        let harness = MockCommandLaunchHarness::new();

        let resolved_claude = resolve_claude_command().expect("mock Claude should resolve on PATH");
        let resolved_codex = resolve_codex_command().expect("mock Codex should resolve on PATH");

        assert_eq!(
            PathBuf::from(resolved_claude),
            harness.bin_dir().join("claude")
        );
        assert_eq!(
            PathBuf::from(resolved_codex),
            harness.bin_dir().join("codex")
        );
        assert_eq!(
            env::var_os("HOME").as_deref(),
            Some(harness.home().as_os_str())
        );
        assert_eq!(
            env::current_dir()
                .expect("cwd should resolve")
                .canonicalize()
                .expect("cwd should canonicalize"),
            harness
                .workspace()
                .canonicalize()
                .expect("mock workspace should canonicalize")
        );

        let prompt_file = env::temp_dir().join(format!("ctx-claude-harness-{}.md", Uuid::new_v4()));
        fs::write(&prompt_file, "temporary ctx prompt")
            .expect("test prompt file should be writable");
        let claude_plan = ClaudeLaunchPlan {
            session_id: Uuid::new_v4(),
            program: "claude".to_string(),
            args: vec![
                "--append-system-prompt-file".to_string(),
                prompt_file.display().to_string(),
                "--model".to_string(),
                "claude-sonnet".to_string(),
            ],
            working_dir: harness.workspace().to_path_buf(),
            preset_id: Uuid::new_v4(),
            state_dir: temp_workspace(),
            prompt_file: TemporaryPromptFile::new(prompt_file.clone()),
            embedded_manifest: None,
        };

        let claude_exit =
            run_wrapped_claude_session(claude_plan).expect("mock Claude child should launch");

        let context = context(
            "Harness Codex Rules",
            "Visible through the mock Codex child.",
        );
        let preset = Preset {
            preset_id: Uuid::new_v4(),
            preset_name: "Harness Codex".to_string(),
            preset_contexts: vec![context.context_id],
            preset_metadata: Default::default(),
            preset_context_composition: Vec::new(),
            preset_target_cli: CliTarget::Codex,
            preset_working_dir: harness.workspace().to_path_buf(),
            preset_model: Some("gpt-5.3-codex".to_string()),
            subagent_manifest: None,
        };
        let codex_plan = build_codex_launch_plan(orchestration(
            preset,
            vec![context],
            vec!["--sandbox".to_string(), "workspace-write".to_string()],
        ))
        .expect("Codex launch plan should be created with mock workspace");

        let codex_exit =
            run_wrapped_codex_session(codex_plan).expect("mock Codex child should launch");

        assert_eq!(claude_exit, 0);
        assert_eq!(codex_exit, 0);
        assert_eq!(
            harness.claude_args(),
            vec![
                "--append-system-prompt-file".to_string(),
                prompt_file.display().to_string(),
                "--model".to_string(),
                "claude-sonnet".to_string(),
            ]
        );
        assert_eq!(
            harness.codex_args(),
            vec![
                "--model".to_string(),
                "gpt-5.3-codex".to_string(),
                "--sandbox".to_string(),
                "workspace-write".to_string(),
            ]
        );
        assert!(!prompt_file.exists());
        assert!(!harness.workspace().join(AGENTS_MD_FILE_NAME).exists());
    }

    #[test]
    fn parse_classify_file_arg_accepts_file_or_stdin_modes() {
        assert_eq!(
            parse_classify_file_arg(&[]).expect("stdin mode should parse"),
            None
        );
        assert_eq!(
            parse_classify_file_arg(&["--file".to_string(), "notes/reviewer.md".to_string()])
                .expect("file flag should parse"),
            Some(PathBuf::from("notes/reviewer.md"))
        );
        assert_eq!(
            parse_classify_file_arg(&["notes/reviewer.md".to_string()])
                .expect("single path should parse"),
            Some(PathBuf::from("notes/reviewer.md"))
        );
    }

    #[test]
    fn parse_classify_file_arg_rejects_unknown_flags() {
        let error = parse_classify_file_arg(&["--json".to_string()])
            .expect_err("unknown classify flags should fail");

        assert!(error.contains("ctx classify"));
    }

    #[test]
    fn parse_lookup_args_accepts_path_and_tag_modes() {
        assert_eq!(
            parse_lookup_args(&["--path".to_string(), "notes/target.md".to_string()])
                .expect("path lookup should parse"),
            LookupArgs::Path(PathBuf::from("notes/target.md"))
        );
        assert_eq!(
            parse_lookup_args(&["--tag".to_string(), "#review".to_string()])
                .expect("tag lookup should parse"),
            LookupArgs::Tag("#review".to_string())
        );
        assert_eq!(
            parse_lookup_args(&["notes/target.md".to_string()])
                .expect("bare path lookup should parse"),
            LookupArgs::Path(PathBuf::from("notes/target.md"))
        );
    }

    #[test]
    fn parse_watch_args_accepts_once_and_interval() {
        let parsed = parse_watch_args(vec!["--once".to_string(), "--interval-ms=250".to_string()])
            .expect("watch args should parse");

        assert_eq!(
            parsed,
            WatchArgs {
                once: true,
                interval_ms: 250
            }
        );
    }

    #[test]
    fn parse_watch_args_rejects_zero_interval() {
        let error = parse_watch_args(vec!["--interval-ms".to_string(), "0".to_string()])
            .expect_err("zero interval should be rejected");

        assert!(error.contains("greater than 0"));
    }

    #[test]
    fn context_list_row_includes_inferred_classification() {
        let mut context = context("Reviewer Agent", "Review the patch.");
        context.classification = Classification::Shared;
        context.inferred_classification = Some(Classification::Subagent);
        context.file_path = PathBuf::from("/workspace/.ctx/vault/agents/reviewer.md");
        context.import_source = Some(PathBuf::from("/workspace/agents/reviewer.md"));

        let row = format_context_list_row(&context);

        assert_eq!(
            row,
            "discovered\tglobal\tshared\tsubagent\tReviewer Agent\t/workspace/.ctx/vault/agents/reviewer.md"
        );
    }

    #[test]
    fn import_candidate_row_includes_inferred_classification_for_selection() {
        let candidate = ctx_core::ContextDiscoveryResult {
            file_path: PathBuf::from("/workspace/skills/rust.md"),
            file_name: "rust.md".to_string(),
            root_source: PathBuf::from("/workspace"),
            source_type: ctx_core::ImportSourceType::SkillMarkdown,
            metadata: ctx_core::ContextDiscoveryMetadata {
                title: "Rust".to_string(),
                vault_scope: VaultScope::Local,
                classification: Classification::Shared,
                import_classification_suggestion: Some(Classification::Shared),
                inferred_classification: Some(Classification::Shared),
                tags: vec!["skills".to_string()],
                folder_path: PathBuf::from("skills"),
                wikilinks: Vec::new(),
                llm_classification_status: ClassificationStatus::Classified,
            },
        };

        let row = format_import_candidate_row(&candidate);

        assert_eq!(
            row,
            "local\tshared\tshared\tskill-md\trust.md\t/workspace\t/workspace/skills/rust.md"
        );
    }

    #[test]
    fn load_launch_preset_reads_local_overlay_contexts_for_wrapper_injection() {
        let workspace = temp_workspace();
        let roots = VaultRoots::discover(&workspace);
        let created = create_context_file(
            &roots,
            VaultScope::Local,
            "agents",
            "rules.md",
            "Use project-local rules.",
        )
        .expect("local context should be created");
        let presets_dir = managed_presets_dir(roots.local_root.as_ref().unwrap());
        fs::create_dir_all(&presets_dir).expect("local presets dir should be created");
        fs::write(
            presets_dir.join("daily.json"),
            r#"{
                "preset_name": "Daily Codex",
                "preset_target_cli": "codex",
                "preset_contexts": ["agents/rules.md"]
            }"#,
        )
        .expect("preset should be writable");

        let loaded = load_launch_preset(CliTarget::Codex, &workspace, Some("daily".to_string()))
            .expect("launch preset should load from resolved overlay");

        assert_eq!(loaded.preset.preset_name, "Daily Codex");
        assert_eq!(loaded.contexts.len(), 1);
        assert_eq!(loaded.contexts[0].file_path, created.file_path);
        assert_eq!(
            loaded.preset.preset_contexts,
            vec![loaded.contexts[0].context_id]
        );
        assert!(loaded.passthrough_args.is_empty());
    }

    #[test]
    fn load_launch_preset_resolves_configured_preset_name_for_wrapper_injection() {
        let workspace = temp_workspace();
        let roots = VaultRoots::discover(&workspace);
        let created = create_context_file(
            &roots,
            VaultScope::Local,
            "agents",
            "rules.md",
            "Use preset-name rules.",
        )
        .expect("local context should be created");
        let presets_dir = managed_presets_dir(roots.local_root.as_ref().unwrap());
        fs::create_dir_all(&presets_dir).expect("local presets dir should be created");
        fs::write(
            presets_dir.join("daily-driver.json"),
            r#"{
                "preset_name": "Daily Driver",
                "preset_target_cli": "codex",
                "preset_contexts": ["agents/rules.md"]
            }"#,
        )
        .expect("preset should be writable");

        let loaded = load_launch_preset(
            CliTarget::Codex,
            &workspace,
            Some("Daily Driver".to_string()),
        )
        .expect("launch preset should resolve by configured preset name");

        assert_eq!(loaded.preset.preset_name, "Daily Driver");
        assert_eq!(loaded.contexts.len(), 1);
        assert_eq!(loaded.contexts[0].file_path, created.file_path);
    }

    #[test]
    fn load_launch_preset_prefers_local_overlay_preset_and_context_for_cli_reads() {
        let workspace = temp_workspace();
        let home = temp_workspace();
        with_home(&home, || {
            let roots = VaultRoots::discover(&workspace);
            let global = create_context_file(
                &roots,
                VaultScope::Global,
                "agents",
                "rules.md",
                "# Global CLI Rules",
            )
            .expect("global context should be created");
            let local = create_context_file(
                &roots,
                VaultScope::Local,
                "agents",
                "rules.md",
                "# Local CLI Rules",
            )
            .expect("local context should override global context");

            let global_presets = managed_presets_dir(&roots.global_root);
            let local_presets = managed_presets_dir(roots.local_root.as_ref().unwrap());
            fs::create_dir_all(&global_presets).expect("global preset dir should be created");
            fs::create_dir_all(&local_presets).expect("local preset dir should be created");
            fs::write(
                global_presets.join("daily.json"),
                r#"{"preset_name":"Global CLI Daily","preset_target_cli":"codex","preset_contexts":["agents/rules.md"]}"#,
            )
            .expect("global preset should be writable");
            fs::write(
                local_presets.join("daily.json"),
                r#"{"preset_name":"Local CLI Daily","preset_target_cli":"codex","preset_contexts":["agents/rules.md"]}"#,
            )
            .expect("local preset should be writable");

            let loaded =
                load_launch_preset(CliTarget::Codex, &workspace, Some("daily".to_string()))
                    .expect("CLI launch should resolve local overlay preset and context");

            assert_eq!(loaded.preset.preset_name, "Local CLI Daily");
            assert_eq!(loaded.contexts.len(), 1);
            assert_eq!(loaded.contexts[0].file_path, local.file_path);
            assert_eq!(loaded.contexts[0].content, "# Local CLI Rules");
            assert!(!loaded
                .contexts
                .iter()
                .any(|context| context.file_path == global.file_path));
        });
    }

    #[test]
    fn load_launch_preset_falls_back_to_global_preset_and_context_when_local_is_absent() {
        let workspace = temp_workspace();
        let home = temp_workspace();
        with_home(&home, || {
            let roots = VaultRoots::discover(&workspace);
            let global = create_context_file(
                &roots,
                VaultScope::Global,
                "shared",
                "fallback.md",
                "# Global Fallback",
            )
            .expect("global context should be created");

            let global_presets = managed_presets_dir(&roots.global_root);
            fs::create_dir_all(&global_presets).expect("global preset dir should be created");
            fs::write(
                global_presets.join("fallback.json"),
                r#"{"preset_name":"Global CLI Fallback","preset_target_cli":"claude","preset_contexts":["shared/fallback.md"]}"#,
            )
            .expect("global preset should be writable");

            let loaded =
                load_launch_preset(CliTarget::Claude, &workspace, Some("fallback".to_string()))
                    .expect("CLI launch should fall back to global preset and context");

            assert_eq!(loaded.preset.preset_name, "Global CLI Fallback");
            assert_eq!(loaded.contexts.len(), 1);
            assert_eq!(loaded.contexts[0].vault_scope, VaultScope::Global);
            assert_eq!(loaded.contexts[0].file_path, global.file_path);
            assert_eq!(loaded.contexts[0].content, "# Global Fallback");
        });
    }

    #[test]
    fn load_launch_preset_carries_saved_execution_settings_for_wrapper_args() {
        let workspace = temp_workspace();
        let roots = VaultRoots::discover(&workspace);
        let presets_dir = managed_presets_dir(roots.local_root.as_ref().unwrap());
        fs::create_dir_all(&presets_dir).expect("local presets dir should be created");
        fs::write(
            presets_dir.join("implementation.json"),
            format!(
                r#"{{
                    "preset_name": "Implementation",
                    "cli_execution_settings": {{
                        "target_cli": "codex",
                        "working_dir": "{}",
                        "model": "gpt-5.3-codex",
                        "passthrough_args": ["--sandbox", "workspace-write"]
                    }}
                }}"#,
                workspace.display()
            ),
        )
        .expect("preset should be writable");

        let loaded = load_launch_preset(
            CliTarget::Codex,
            &workspace,
            Some("implementation".to_string()),
        )
        .expect("launch preset should expose saved execution settings");

        assert_eq!(loaded.preset.preset_working_dir, workspace);
        assert_eq!(loaded.preset.preset_model.as_deref(), Some("gpt-5.3-codex"));
        assert_eq!(
            loaded.passthrough_args,
            vec!["--sandbox".to_string(), "workspace-write".to_string()]
        );
    }

    #[test]
    fn load_launch_preset_reports_missing_preset_with_available_choices() {
        let workspace = temp_workspace();
        let roots = VaultRoots::discover(&workspace);
        let presets_dir = managed_presets_dir(roots.local_root.as_ref().unwrap());
        fs::create_dir_all(&presets_dir).expect("local presets dir should be created");
        fs::write(
            presets_dir.join("daily.json"),
            r#"{"preset_name":"Daily","preset_target_cli":"codex"}"#,
        )
        .expect("preset should be writable");

        let error = load_launch_preset(CliTarget::Codex, &workspace, Some("missing".to_string()))
            .expect_err("missing launch preset should report a clear user-facing error");

        assert!(error.starts_with("Cannot launch codex with --preset 'missing':"));
        assert!(error.contains("preset 'missing' was not found"));
        assert!(error.contains("Searched:"));
        assert!(error.contains(".ctx/vault/presets"));
        assert!(error.contains("Available presets: daily"));
    }

    #[test]
    fn load_launch_preset_reports_invalid_preset_definition_before_launch() {
        let workspace = temp_workspace();
        let roots = VaultRoots::discover(&workspace);
        let presets_dir = managed_presets_dir(roots.local_root.as_ref().unwrap());
        fs::create_dir_all(&presets_dir).expect("local presets dir should be created");
        fs::write(
            presets_dir.join("bad.json"),
            r#"["preset definitions must be objects"]"#,
        )
        .expect("preset should be writable");

        let error = load_launch_preset(CliTarget::Codex, &workspace, Some("bad".to_string()))
            .expect_err("invalid launch preset should report a clear user-facing error");

        assert!(error.starts_with("Cannot launch codex because preset 'bad' is invalid:"));
        assert!(error.contains("invalid preset definition"));
        assert!(error.contains("top-level JSON value must be an object"));
        assert!(
            !workspace.join(AGENTS_MD_FILE_NAME).exists(),
            "invalid preset definitions must fail before Codex AGENTS.md injection"
        );
    }

    #[test]
    fn wrapper_startup_orchestration_resolves_embedded_manifest_before_launch() {
        let workspace = temp_workspace();
        let roots = VaultRoots::discover(&workspace);
        let reviewer = create_context_file(
            &roots,
            VaultScope::Local,
            "subagents",
            "reviewer.md",
            "# Reviewer\n\nFind correctness risks.",
        )
        .expect("subagent context should be created");
        let presets_dir = managed_presets_dir(roots.local_root.as_ref().unwrap());
        fs::create_dir_all(&presets_dir).expect("local presets dir should be created");
        fs::write(
            presets_dir.join("delegated-review.json"),
            r#"{
                "preset_name": "Delegated Review",
                "preset_target_cli": "codex",
                "preset_contexts": ["subagents/reviewer.md"],
                "subagent_manifest": {
                    "manifest_version": "1",
                    "roles": [
                        {
                            "role_id": "reviewer",
                            "role_name": "Reviewer",
                            "role": "Code review subagent",
                            "capabilities": ["correctness review"],
                            "constraints": ["Return findings first."],
                            "assigned_contexts": ["subagents/reviewer.md"],
                            "spawn_instructions": ["Review the active patch."],
                            "spawn_guidance": {
                                "select_when": ["Use after implementation."],
                                "avoid_when": ["Avoid before code exists."]
                            }
                        }
                    ]
                }
            }"#,
        )
        .expect("preset should be writable");

        let startup = orchestrate_wrapper_startup(
            CliTarget::Codex,
            &workspace,
            LaunchArgs {
                preset_ref: Some("delegated-review".to_string()),
                session_ref: None,
                handoff_ref: None,
                passthrough_args: vec!["--sandbox".to_string(), "workspace-write".to_string()],
            },
        )
        .expect("wrapper startup should resolve preset and manifest before launch planning");
        let manifest = startup
            .embedded_manifest
            .expect("embedded manifest should be resolved");

        assert_eq!(startup.preset.preset_name, "Delegated Review");
        assert_eq!(
            startup.passthrough_args,
            vec!["--sandbox".to_string(), "workspace-write".to_string()]
        );
        assert_eq!(manifest.manifest.roles[0].role_id, "reviewer");
        assert_eq!(manifest.role_contexts.len(), 1);
        assert_eq!(manifest.role_contexts[0].role_id, "reviewer");
        assert_eq!(manifest.role_contexts[0].contexts.len(), 1);
        assert_eq!(
            manifest.role_contexts[0].contexts[0].file_path,
            reviewer.file_path
        );
        assert_eq!(
            manifest.role_contexts[0].contexts[0].content,
            "# Reviewer\n\nFind correctness risks."
        );
    }

    #[test]
    fn claude_launch_plan_renders_contexts_loaded_from_preset_filesystem_refs() {
        let workspace = temp_workspace();
        let roots = VaultRoots::discover(&workspace);
        let selected = create_context_file(
            &roots,
            VaultScope::Local,
            "agents",
            "selected.md",
            "# Selected Launch Rules\n\nUse the selected preset context.",
        )
        .expect("selected context should be created");
        let ignored = create_context_file(
            &roots,
            VaultScope::Local,
            "agents",
            "ignored.md",
            "# Ignored Launch Rules\n\nThis file is not in the preset.",
        )
        .expect("ignored context should be created");
        let presets_dir = managed_presets_dir(roots.local_root.as_ref().unwrap());
        fs::create_dir_all(&presets_dir).expect("local presets dir should be created");
        fs::write(
            presets_dir.join("claude-selected.json"),
            r#"{
                "preset_name": "Claude Selected",
                "preset_target_cli": "claude",
                "preset_contexts": ["agents/selected.md"]
            }"#,
        )
        .expect("preset should be writable");

        let startup = orchestrate_wrapper_startup(
            CliTarget::Claude,
            &workspace,
            LaunchArgs {
                preset_ref: Some("claude-selected".to_string()),
                session_ref: None,
                handoff_ref: None,
                passthrough_args: Vec::new(),
            },
        )
        .expect("wrapper startup should load selected preset contexts from disk");
        let plan = build_claude_launch_plan(startup)
            .expect("Claude launch plan should render the loaded preset context");
        let prompt =
            fs::read_to_string(plan.prompt_file.path()).expect("prompt file should be readable");

        assert!(prompt.contains("# CTX Claude Session Context"));
        assert!(prompt.contains("Preset: Claude Selected"));
        assert!(prompt.contains("## selected"));
        assert!(prompt.contains("Use the selected preset context."));
        assert!(prompt.contains(&selected.file_path.display().to_string()));
        assert!(!prompt.contains("This file is not in the preset."));
        assert!(!prompt.contains(&ignored.file_path.display().to_string()));
    }

    #[test]
    fn launch_session_selection_preserves_classified_handoff_context() {
        let harness = MockCommandLaunchHarness::new();
        let session_id = "codex-classified-selection";
        let session_dir = harness
            .home()
            .join(".codex")
            .join("sessions")
            .join("2026")
            .join("05");
        fs::create_dir_all(&session_dir).expect("Codex session log dir should be created");
        fs::write(
            session_dir.join(format!("{session_id}.jsonl")),
            format!(
                r#"{{"type":"session_meta","timestamp":"2026-05-11T00:00:00Z","payload":{{"id":"{session_id}","cwd":"{}","timestamp":"2026-05-11T00:00:00Z"}}}}
{{"type":"event_msg","timestamp":"2026-05-11T00:01:00Z","payload":{{"type":"user_message","message":"Implement session launch handoff selection"}}}}
{{"type":"event_msg","timestamp":"2026-05-11T00:02:00Z","payload":{{"type":"agent_message","message":"Summary: Implemented launch handoff selection for prior sessions.\nDecision: Preserve work-context classification when a scanned session is selected for launch.\nChanged files: crates/ctx-cli/src/bin/ctx.rs.\nVerified with cargo test -p ctx-cli launch_session_selection_preserves_classified_handoff_context.\nRemaining work: run full CLI tests."}}}}"#,
                harness.workspace().display()
            ),
        )
        .expect("Codex session log should be writable");

        let startup = orchestrate_wrapper_startup(
            CliTarget::Claude,
            harness.workspace(),
            LaunchArgs {
                preset_ref: None,
                session_ref: Some(session_id.to_string()),
                handoff_ref: None,
                passthrough_args: Vec::new(),
            },
        )
        .expect("wrapper startup should classify and select the session handoff context");

        assert_eq!(startup.contexts.len(), 1);
        assert_eq!(
            startup.preset.preset_contexts,
            vec![startup.contexts[0].context_id]
        );
        let metadata = startup.contexts[0]
            .session_handoff_classification
            .as_ref()
            .expect("selected session should retain handoff classification metadata");
        assert_eq!(metadata.source_tool, "codex");
        assert_eq!(metadata.source_session_ref, session_id);
        assert_eq!(
            metadata.source_working_directory,
            harness.workspace().display().to_string()
        );
        assert_eq!(
            metadata.work_context_classification_status,
            ClassificationStatus::Classified
        );
        assert!(!metadata.work_context_categories.is_empty());

        let plan = build_claude_launch_plan(startup)
            .expect("Claude launch plan should include selected session context");
        let prompt =
            fs::read_to_string(plan.prompt_file.path()).expect("prompt file should be readable");

        assert!(prompt.contains("# CTX Claude Session Context"));
        assert!(prompt.contains("Implement session launch handoff selection"));
        assert!(prompt.contains("source_tool: codex"));
        assert!(prompt.contains(&format!("source_session_ref: \"{session_id}\"")));
        assert!(prompt.contains("work_context_category: "));
        assert!(prompt.contains("work_context_categories: ["));
        assert!(prompt.contains("work_context_classification_status: classified"));
        assert!(prompt.contains("Preserve work-context classification"));
        assert!(prompt.contains("crates/ctx-cli/src/bin/ctx.rs"));
    }

    #[test]
    fn claude_launch_plan_preserves_selected_saved_handoff_markdown() {
        let workspace = temp_workspace();
        let roots = VaultRoots::discover(&workspace);
        let body = "# Previous Session Context\n\n## Handoff Summary\n\nSaved handoff launch selection is preserved. Saved Claude handoff body must survive target launch selection.\n\n### Goals\n\n- Preserve selected saved handoff context.\n\n### Key changed files\n\n- crates/ctx-cli/src/bin/ctx.rs\n\n### Commands\n\n- cargo test -p ctx-cli\n\n### Decisions\n\n- Use the saved handoff markdown, not regenerated scan output.\n\n### Verification results\n\n- cargo test -p ctx-cli claude_launch_plan_preserves_selected_saved_handoff_markdown\n\n### Remaining work\n\n- Launch the selected saved context.";
        let handoff = saved_handoff_context(CliTarget::Claude, body);
        let saved = create_session_handoff_context_file(
            &roots,
            VaultScope::Local,
            PathBuf::from("session-history"),
            "claude-selected-handoff.md",
            &handoff,
        )
        .expect("saved Claude handoff should be persisted");

        let startup = orchestrate_wrapper_startup(
            CliTarget::Claude,
            &workspace,
            LaunchArgs {
                preset_ref: None,
                session_ref: None,
                handoff_ref: Some(saved.fragment.file_path.display().to_string()),
                passthrough_args: Vec::new(),
            },
        )
        .expect("wrapper startup should select the saved handoff context");
        assert_eq!(startup.contexts[0].content, body);

        let plan = build_claude_launch_plan(startup)
            .expect("Claude launch plan should include selected saved handoff");
        let prompt =
            fs::read_to_string(plan.prompt_file.path()).expect("prompt file should be readable");

        assert!(prompt.contains("# CTX Claude Session Context"));
        assert!(prompt.contains("Saved Claude handoff body must survive target launch selection."));
        assert!(prompt.contains("Use the saved handoff markdown, not regenerated scan output."));
        assert!(prompt.contains(&saved.fragment.file_path.display().to_string()));
        assert!(
            !prompt.contains("session_handoff_format_version"),
            "launch should inject the selected handoff body, not saved frontmatter"
        );
    }

    #[test]
    fn codex_launch_plan_preserves_selected_saved_handoff_markdown() {
        let workspace = temp_workspace();
        let roots = VaultRoots::discover(&workspace);
        let body = "# Previous Session Context\n\n## Handoff Summary\n\nSaved handoff launch selection is preserved. Saved Codex handoff body must be visible through AGENTS.md injection.\n\n### Goals\n\n- Preserve selected saved handoff context.\n\n### Key changed files\n\n- crates/ctx-cli/src/bin/ctx.rs\n\n### Commands\n\n- cargo test -p ctx-cli\n\n### Decisions\n\n- Use the saved handoff markdown, not regenerated scan output.\n- Inject the selected saved handoff as the managed block payload.\n\n### Verification results\n\n- cargo test -p ctx-cli codex_launch_plan_preserves_selected_saved_handoff_markdown\n\n### Remaining work\n\n- Launch the selected saved context.\n- Clean up the managed block after exit.";
        let handoff = saved_handoff_context(CliTarget::Codex, body);
        let saved = create_session_handoff_context_file(
            &roots,
            VaultScope::Local,
            PathBuf::from("session-history"),
            "codex-selected-handoff.md",
            &handoff,
        )
        .expect("saved Codex handoff should be persisted");

        let startup = orchestrate_wrapper_startup(
            CliTarget::Codex,
            &workspace,
            LaunchArgs {
                preset_ref: None,
                session_ref: None,
                handoff_ref: Some(saved.fragment.file_path.display().to_string()),
                passthrough_args: Vec::new(),
            },
        )
        .expect("wrapper startup should select the saved handoff context");
        assert_eq!(startup.contexts[0].content, body);

        let plan = build_codex_launch_plan(startup)
            .expect("Codex launch plan should inject selected saved handoff");
        let agents_md = fs::read_to_string(workspace.join(AGENTS_MD_FILE_NAME))
            .expect("AGENTS.md should be readable after injection");

        assert!(agents_md.contains(CTX_START_MARKER));
        assert!(agents_md.contains("# CTX Codex Session Context"));
        assert!(agents_md
            .contains("Saved Codex handoff body must be visible through AGENTS.md injection."));
        assert!(
            agents_md.contains("Inject the selected saved handoff as the managed block payload.")
        );
        assert!(agents_md.contains(&saved.fragment.file_path.display().to_string()));
        assert!(
            !agents_md.contains("session_handoff_format_version"),
            "launch should inject the selected handoff body, not saved frontmatter"
        );
        drop(plan);
        assert!(
            !workspace.join(AGENTS_MD_FILE_NAME).exists(),
            "dropping the plan should clean the temporary managed block"
        );
    }

    #[test]
    fn wrapper_startup_orchestration_rejects_oversized_manifest_before_launch_planning() {
        let workspace = temp_workspace();
        let roots = VaultRoots::discover(&workspace);
        let presets_dir = managed_presets_dir(roots.local_root.as_ref().unwrap());
        fs::create_dir_all(&presets_dir).expect("local presets dir should be created");
        let oversized_padding = "x".repeat(MAX_SUBAGENT_MANIFEST_JSON_BYTES + 1);
        fs::write(
            presets_dir.join("oversized-delegation.json"),
            format!(
                r#"{{
                    "preset_name": "Oversized Delegation",
                    "preset_target_cli": "codex",
                    "subagent_manifest": {{
                        "manifest_version": "1",
                        "padding": "{oversized_padding}"
                    }}
                }}"#
            ),
        )
        .expect("preset should be writable");

        let error = orchestrate_wrapper_startup(
            CliTarget::Codex,
            &workspace,
            LaunchArgs {
                preset_ref: Some("oversized-delegation".to_string()),
                session_ref: None,
                handoff_ref: None,
                passthrough_args: Vec::new(),
            },
        )
        .expect_err("oversized manifest should fail before launch planning or injection");

        assert!(
            error.contains("Cannot launch codex because preset 'oversized-delegation' is invalid")
        );
        assert!(error.contains("invalid subagent_manifest"));
        assert!(error.contains("byte launch limit"));
        assert!(
            !workspace.join(AGENTS_MD_FILE_NAME).exists(),
            "launch preflight must fail before Codex AGENTS.md injection"
        );
    }

    #[test]
    fn wrapper_startup_orchestration_rejects_manifest_with_unselected_assigned_context() {
        let selected = context("Selected Rules", "# Selected");
        let missing = Uuid::new_v4();
        let preset = Preset {
            preset_id: Uuid::new_v4(),
            preset_name: "Bad Manifest Assignment".to_string(),
            preset_contexts: vec![selected.context_id],
            preset_metadata: Default::default(),
            preset_context_composition: vec![ctx_core::PresetContextComposition {
                context_id: selected.context_id,
                order: 0,
                source_ref: "shared/selected.md".to_string(),
                required: true,
                selection: Default::default(),
            }],
            preset_target_cli: CliTarget::Codex,
            preset_working_dir: temp_workspace(),
            preset_model: None,
            subagent_manifest: Some(SubagentManifest {
                manifest_version: Some("1".to_string()),
                roles: vec![ctx_core::SubagentRole {
                    role_id: "reviewer".to_string(),
                    role_name: "Reviewer".to_string(),
                    role: "Code review subagent".to_string(),
                    capabilities: vec!["correctness review".to_string()],
                    constraints: vec!["Return findings first.".to_string()],
                    metadata: Default::default(),
                    description: None,
                    assigned_contexts: vec![missing.to_string()],
                    spawn_instructions: vec!["Review the active patch.".to_string()],
                    spawn_guidance: ctx_core::models::SubagentSpawnGuidance {
                        select_when: vec!["Use after implementation.".to_string()],
                        avoid_when: vec!["Avoid before code exists.".to_string()],
                        delegation_prompt: None,
                    },
                    handoff_targets: Vec::new(),
                    model: None,
                }],
                handoff_constraints: Default::default(),
            }),
        };

        let error = resolve_embedded_launch_manifest(&preset, &[selected])
            .expect_err("startup manifest resolution should reject missing role contexts");

        assert!(error.contains("failed to resolve embedded subagent manifest"));
        assert!(error.contains("reviewer"));
    }

    #[test]
    fn claude_launch_plan_uses_append_system_prompt_file_model_and_passthrough_args() {
        let context = ContextFragment {
            context_id: Uuid::new_v4(),
            title: "Shared Rules".to_string(),
            content: "Use the shared rules.".to_string(),
            file_path: PathBuf::from("/vault/contexts/shared-rules.md"),
            vault_scope: VaultScope::Global,
            classification: Classification::Shared,
            import_classification_suggestion: Some(Classification::Shared),
            inferred_classification: Some(Classification::Shared),
            tags: Vec::new(),
            folder_path: PathBuf::new(),
            wikilinks: Vec::new(),
            backlinks: Vec::new(),
            import_source: None,
            import_source_type: None,
            llm_classification_status: ClassificationStatus::Reviewed,
            session_handoff_classification: None,
        };
        let preset = Preset {
            preset_id: Uuid::new_v4(),
            preset_name: "Claude Test".to_string(),
            preset_contexts: vec![context.context_id],
            preset_metadata: Default::default(),
            preset_context_composition: Vec::new(),
            preset_target_cli: CliTarget::Claude,
            preset_working_dir: PathBuf::from("/workspace"),
            preset_model: Some("claude-sonnet".to_string()),
            subagent_manifest: None,
        };

        let plan = build_claude_launch_plan(orchestration(
            preset,
            vec![context],
            vec!["--model".to_string(), "sonnet".to_string()],
        ))
        .expect("Claude launch plan should be created");

        assert_eq!(plan.program, "claude");
        assert_eq!(plan.working_dir, PathBuf::from("/workspace"));
        assert_eq!(plan.args[0], "--append-system-prompt-file");
        assert_eq!(PathBuf::from(&plan.args[1]), plan.prompt_file.path());
        assert_eq!(plan.args[2], "--model");
        assert_eq!(plan.args[3], "claude-sonnet");
        assert_eq!(plan.args[4], "--model");
        assert_eq!(plan.args[5], "sonnet");
        assert!(plan.prompt_file.path().exists());
    }

    #[test]
    fn wrapped_claude_session_forwards_append_prompt_model_and_passthrough_args() {
        let workspace = temp_workspace();
        let args_log = workspace.join("claude-args.log");
        let fake_claude = workspace.join("claude");
        write_executable_script(
            &fake_claude,
            &format!(
                "#!/bin/sh\nprintf '%s\\n' \"$@\" > \"{}\"\nexit 7\n",
                args_log.display()
            ),
        );
        let prompt_file =
            env::temp_dir().join(format!("ctx-claude-forwarding-{}.md", Uuid::new_v4()));
        fs::write(&prompt_file, "temporary ctx prompt")
            .expect("test prompt file should be writable");
        let plan = ClaudeLaunchPlan {
            session_id: Uuid::new_v4(),
            program: fake_claude.display().to_string(),
            args: vec![
                "--append-system-prompt-file".to_string(),
                prompt_file.display().to_string(),
                "--model".to_string(),
                "claude-sonnet".to_string(),
                "--debug".to_string(),
                "--".to_string(),
                "literal user prompt".to_string(),
            ],
            working_dir: workspace.clone(),
            preset_id: Uuid::new_v4(),
            state_dir: temp_workspace(),
            prompt_file: TemporaryPromptFile::new(prompt_file.clone()),
            embedded_manifest: None,
        };

        let exit_code = run_wrapped_claude_session(plan).expect("fake Claude process should run");
        let forwarded_args = fs::read_to_string(args_log).expect("fake CLI should record argv");

        assert_eq!(exit_code, 7);
        assert_eq!(
            forwarded_args
                .lines()
                .map(str::to_string)
                .collect::<Vec<_>>(),
            vec![
                "--append-system-prompt-file".to_string(),
                prompt_file.display().to_string(),
                "--model".to_string(),
                "claude-sonnet".to_string(),
                "--debug".to_string(),
                "--".to_string(),
                "literal user prompt".to_string()
            ]
        );
        assert!(!prompt_file.exists());
    }

    #[test]
    fn wrapped_claude_session_materializes_prompt_before_child_launch() {
        let workspace = temp_workspace();
        let prompt_snapshot = workspace.join("claude-prompt-snapshot.md");
        let fake_claude = workspace.join("claude");
        write_executable_script(
            &fake_claude,
            &format!(
                "#!/bin/sh\nprompt_file=''\nwhile [ \"$#\" -gt 0 ]; do\n  if [ \"$1\" = '--append-system-prompt-file' ]; then\n    shift\n    prompt_file=\"$1\"\n  fi\n  shift\ndone\n[ -n \"$prompt_file\" ] || exit 41\n[ -f \"$prompt_file\" ] || exit 42\ncat \"$prompt_file\" > '{}'\nexit 0\n",
                prompt_snapshot.display()
            ),
        );
        let selected = context(
            "Claude Startup Rules",
            "# Startup Rules\n\nRead this before launch.",
        );
        let preset = Preset {
            preset_id: Uuid::new_v4(),
            preset_name: "Claude Prelaunch Injection".to_string(),
            preset_contexts: vec![selected.context_id],
            preset_metadata: Default::default(),
            preset_context_composition: Vec::new(),
            preset_target_cli: CliTarget::Claude,
            preset_working_dir: workspace.clone(),
            preset_model: Some("claude-sonnet".to_string()),
            subagent_manifest: None,
        };
        let mut plan = build_claude_launch_plan(orchestration(
            preset,
            vec![selected.clone()],
            vec!["--debug".to_string()],
        ))
        .expect("Claude launch plan should materialize prompt file");
        let prompt_file_path = plan.prompt_file.path().to_path_buf();
        plan.program = fake_claude.display().to_string();

        let exit_code = run_wrapped_claude_session(plan)
            .expect("fake Claude process should read prelaunch prompt file");
        let child_visible_prompt =
            fs::read_to_string(prompt_snapshot).expect("fake Claude should snapshot prompt");

        assert_eq!(exit_code, 0);
        assert!(child_visible_prompt.contains("# CTX Claude Session Context"));
        assert!(child_visible_prompt.contains("Preset: Claude Prelaunch Injection"));
        assert!(child_visible_prompt.contains("## Claude Startup Rules"));
        assert!(child_visible_prompt.contains("Read this before launch."));
        assert!(child_visible_prompt.contains(&selected.file_path.display().to_string()));
        assert!(!prompt_file_path.exists());
    }

    #[test]
    fn claude_launch_plan_injects_embedded_manifest_payload_into_startup_prompt() {
        let reviewer = context("Reviewer Notes", "# Reviewer\n\nCheck correctness risks.");
        let mut preset = Preset {
            preset_id: Uuid::new_v4(),
            preset_name: "Claude Delegated Review".to_string(),
            preset_contexts: vec![reviewer.context_id],
            preset_metadata: Default::default(),
            preset_context_composition: vec![ctx_core::PresetContextComposition {
                context_id: reviewer.context_id,
                order: 0,
                source_ref: "subagents/reviewer.md".to_string(),
                required: true,
                selection: Default::default(),
            }],
            preset_target_cli: CliTarget::Claude,
            preset_working_dir: PathBuf::from("/workspace"),
            preset_model: Some("claude-sonnet".to_string()),
            subagent_manifest: None,
        };
        preset.subagent_manifest = Some(SubagentManifest {
            manifest_version: Some("1".to_string()),
            roles: vec![ctx_core::SubagentRole {
                role_id: "reviewer".to_string(),
                role_name: "Reviewer".to_string(),
                role: "Code review subagent".to_string(),
                capabilities: vec!["correctness review".to_string()],
                constraints: vec!["Return findings first.".to_string()],
                metadata: Default::default(),
                description: Some("Reviews implementation risk.".to_string()),
                assigned_contexts: vec!["subagents/reviewer.md".to_string()],
                spawn_instructions: vec!["Review the launched Claude session.".to_string()],
                spawn_guidance: ctx_core::models::SubagentSpawnGuidance {
                    select_when: vec!["Use after implementation changes.".to_string()],
                    avoid_when: vec!["Avoid when no code changed.".to_string()],
                    delegation_prompt: Some("Return findings first.".to_string()),
                },
                handoff_targets: Vec::new(),
                model: Some("claude-sonnet".to_string()),
            }],
            handoff_constraints: Default::default(),
        });

        let plan = build_claude_launch_plan(orchestration(preset, vec![reviewer], Vec::new()))
            .expect("Claude launch plan should include embedded manifest startup payload");
        let prompt =
            fs::read_to_string(plan.prompt_file.path()).expect("prompt file should be readable");

        assert!(prompt.contains("# CTX Claude Session Context"));
        assert!(prompt.contains("# CTX Wrapper Embedded Subagent Payload"));
        assert!(prompt.contains("\"id\": \"reviewer\""));
        assert!(prompt.contains("# CTX Embedded Subagent Context: reviewer"));
        assert!(prompt.contains("source_ref: subagents/reviewer.md"));
        assert!(prompt.contains("Check correctness risks."));
        assert!(
            prompt
                .find("# CTX Wrapper Embedded Subagent Payload")
                .expect("embedded payload should be present")
                > prompt
                    .find("# CTX Claude Session Context")
                    .expect("main startup context should be present")
        );
    }

    #[test]
    fn wrapped_claude_session_removes_prompt_file_after_child_exits() {
        let prompt_file = env::temp_dir().join(format!("ctx-claude-cleanup-{}.md", Uuid::new_v4()));
        fs::write(&prompt_file, "temporary ctx prompt")
            .expect("test prompt file should be writable");
        let plan = ClaudeLaunchPlan {
            session_id: Uuid::new_v4(),
            program: "true".to_string(),
            args: Vec::new(),
            working_dir: env::current_dir().expect("current dir should resolve"),
            preset_id: Uuid::new_v4(),
            state_dir: temp_workspace(),
            prompt_file: TemporaryPromptFile::new(prompt_file.clone()),
            embedded_manifest: None,
        };

        let exit_code = run_wrapped_claude_session(plan).expect("wrapped child process should run");

        assert_eq!(exit_code, 0);
        assert!(!prompt_file.exists());
    }

    #[test]
    fn wrapped_claude_session_records_then_removes_state_and_prompt_after_child_exits() {
        let state_dir = temp_workspace();
        let prompt_file =
            env::temp_dir().join(format!("ctx-claude-state-cleanup-{}.md", Uuid::new_v4()));
        fs::write(&prompt_file, "temporary ctx prompt")
            .expect("test prompt file should be writable");
        let state_dir_arg = state_dir.display().to_string();
        let plan = ClaudeLaunchPlan {
            session_id: Uuid::new_v4(),
            program: "sh".to_string(),
            args: vec![
                "-c".to_string(),
                format!(
                    "i=0; while [ $i -lt 50 ]; do count=$(find '{}' -name '*.json' 2>/dev/null | wc -l | tr -d ' '); [ \"$count\" = 1 ] && [ -f '{}' ] && exit 0; i=$((i + 1)); sleep 0.02; done; exit 1",
                    state_dir_arg,
                    prompt_file.display()
                ),
            ],
            working_dir: env::current_dir().expect("current dir should resolve"),
            preset_id: Uuid::new_v4(),
            state_dir: state_dir.clone(),
            prompt_file: TemporaryPromptFile::new(prompt_file.clone()),
            embedded_manifest: None,
        };

        let exit_code = run_wrapped_claude_session(plan).expect("wrapped child process should run");
        let remaining_state_files = json_file_count(&state_dir);

        assert_eq!(exit_code, 0);
        assert_eq!(remaining_state_files, 0);
        assert!(!prompt_file.exists());
    }

    #[test]
    fn wrapped_claude_session_keeps_child_stdio_open_and_preserves_exit_code() {
        let state_dir = temp_workspace();
        let prompt_file =
            env::temp_dir().join(format!("ctx-claude-stdio-lifecycle-{}.md", Uuid::new_v4()));
        fs::write(&prompt_file, "temporary ctx prompt")
            .expect("test prompt file should be writable");
        let plan = ClaudeLaunchPlan {
            session_id: Uuid::new_v4(),
            program: "sh".to_string(),
            args: vec![
                "-c".to_string(),
                "printf 'ctx claude stdout passthrough\\n'; printf 'ctx claude stderr passthrough\\n' >&2; exit 9".to_string(),
            ],
            working_dir: env::current_dir().expect("current dir should resolve"),
            preset_id: Uuid::new_v4(),
            state_dir: state_dir.clone(),
            prompt_file: TemporaryPromptFile::new(prompt_file.clone()),
            embedded_manifest: None,
        };

        let exit_code = run_wrapped_claude_session(plan).expect("wrapped child process should run");
        let remaining_state_files = json_file_count(&state_dir);

        assert_eq!(exit_code, 9);
        assert_eq!(remaining_state_files, 0);
        assert!(!prompt_file.exists());
    }

    #[cfg(unix)]
    #[test]
    fn wrapped_claude_session_maps_signal_termination_to_shell_compatible_exit_code() {
        let state_dir = temp_workspace();
        let prompt_file =
            env::temp_dir().join(format!("ctx-claude-signal-exit-{}.md", Uuid::new_v4()));
        fs::write(&prompt_file, "temporary ctx prompt")
            .expect("test prompt file should be writable");
        let plan = ClaudeLaunchPlan {
            session_id: Uuid::new_v4(),
            program: "sh".to_string(),
            args: vec!["-c".to_string(), "kill -TERM $$".to_string()],
            working_dir: env::current_dir().expect("current dir should resolve"),
            preset_id: Uuid::new_v4(),
            state_dir: state_dir.clone(),
            prompt_file: TemporaryPromptFile::new(prompt_file.clone()),
            embedded_manifest: None,
        };

        let exit_code = run_wrapped_claude_session(plan).expect("wrapped child process should run");
        let remaining_state_files = json_file_count(&state_dir);

        assert_eq!(exit_code, 143);
        assert_eq!(remaining_state_files, 0);
        assert!(!prompt_file.exists());
    }

    #[test]
    fn wrapped_claude_session_reports_spawn_errors_with_command_and_working_dir() {
        let workspace = temp_workspace();
        let prompt_file =
            env::temp_dir().join(format!("ctx-claude-spawn-error-{}.md", Uuid::new_v4()));
        fs::write(&prompt_file, "temporary ctx prompt")
            .expect("test prompt file should be writable");
        let missing_program = workspace.join("missing-claude");
        let plan = ClaudeLaunchPlan {
            session_id: Uuid::new_v4(),
            program: missing_program.display().to_string(),
            args: vec![
                "--append-system-prompt-file".to_string(),
                prompt_file.display().to_string(),
            ],
            working_dir: workspace.clone(),
            preset_id: Uuid::new_v4(),
            state_dir: temp_workspace(),
            prompt_file: TemporaryPromptFile::new(prompt_file.clone()),
            embedded_manifest: None,
        };

        let error = run_wrapped_claude_session(plan)
            .expect_err("missing Claude executable should be reported");

        assert!(error.contains("Launch failed: Claude CLI did not start."));
        assert!(error.contains("Injection method: temporary prompt file"));
        assert!(error.contains("Next step: verify Claude is installed and executable"));
        assert!(error.contains(CTX_CLAUDE_BIN_ENV));
        assert!(error.contains("--append-system-prompt-file"));
        assert!(error.contains(&missing_program.display().to_string()));
        assert!(error.contains(&workspace.display().to_string()));
        assert!(!prompt_file.exists());
    }

    #[test]
    fn codex_launch_plan_writes_selected_contexts_to_managed_agents_md_block() {
        let workspace = temp_workspace();
        let agents_md = workspace.join(AGENTS_MD_FILE_NAME);
        fs::write(
            &agents_md,
            "# Existing Project Rules\n\nManual project rule.\n",
        )
        .expect("existing AGENTS.md should be writable");

        let first = context("First Rules", "Prefer small focused changes.");
        let second = context("Second Rules", "Run focused tests.");
        let ignored = context("Ignored Rules", "This should not be injected.");
        let preset = Preset {
            preset_id: Uuid::new_v4(),
            preset_name: "Codex Test".to_string(),
            preset_contexts: vec![second.context_id, first.context_id],
            preset_metadata: Default::default(),
            preset_context_composition: Vec::new(),
            preset_target_cli: CliTarget::Codex,
            preset_working_dir: workspace.clone(),
            preset_model: Some("gpt-5.3-codex".to_string()),
            subagent_manifest: None,
        };

        let plan = build_codex_launch_plan(orchestration(
            preset,
            vec![first.clone(), ignored, second.clone()],
            vec!["--sandbox".to_string(), "workspace-write".to_string()],
        ))
        .expect("Codex launch plan should be created");
        let agents_content =
            fs::read_to_string(&agents_md).expect("AGENTS.md should be readable after injection");

        assert_eq!(plan.program, "codex");
        assert_eq!(plan.working_dir, workspace);
        assert_eq!(
            plan.args,
            vec![
                "--model".to_string(),
                "gpt-5.3-codex".to_string(),
                "--sandbox".to_string(),
                "workspace-write".to_string()
            ]
        );
        assert!(agents_content.contains("# Existing Project Rules"));
        assert!(agents_content.contains("Manual project rule."));
        assert!(agents_content.contains("# CTX Codex Session Context"));
        assert!(agents_content.contains("Preset: Codex Test"));
        assert!(agents_content.contains("## Second Rules"));
        assert!(agents_content.contains(&second.content));
        assert!(agents_content.contains("## First Rules"));
        assert!(agents_content.contains(&first.content));
        assert!(!agents_content.contains("This should not be injected."));
        assert!(
            agents_content
                .find("## Second Rules")
                .expect("second section exists")
                < agents_content
                    .find("## First Rules")
                    .expect("first section exists")
        );

        drop(plan);
        let cleaned_content =
            fs::read_to_string(&agents_md).expect("AGENTS.md should remain after cleanup");
        assert!(cleaned_content.contains("# Existing Project Rules"));
        assert!(cleaned_content.contains("Manual project rule."));
        assert!(!cleaned_content.contains("<!-- [ctx:start] -->"));
        assert!(!cleaned_content.contains("Prefer small focused changes."));
    }

    #[test]
    fn codex_launch_plan_wraps_injected_contexts_between_managed_markers() {
        let workspace = temp_workspace();
        let agents_md = workspace.join(AGENTS_MD_FILE_NAME);
        fs::write(
            &agents_md,
            "# Existing Project Rules\n\nManual project rule.\n",
        )
        .expect("existing AGENTS.md should be writable");

        let selected = context("Selected Rules", "This belongs inside the managed block.");
        let ignored = context("Ignored Rules", "This must not be injected.");
        let preset = Preset {
            preset_id: Uuid::new_v4(),
            preset_name: "Marker Boundary".to_string(),
            preset_contexts: vec![selected.context_id],
            preset_metadata: Default::default(),
            preset_context_composition: Vec::new(),
            preset_target_cli: CliTarget::Codex,
            preset_working_dir: workspace,
            preset_model: None,
            subagent_manifest: None,
        };

        let plan =
            build_codex_launch_plan(orchestration(preset, vec![ignored, selected], Vec::new()))
                .expect("Codex launch plan should be created");
        let agents_content =
            fs::read_to_string(&agents_md).expect("AGENTS.md should be readable after injection");
        let start = agents_content
            .find(CTX_START_MARKER)
            .expect("start marker should be inserted");
        let end = agents_content
            .find(CTX_END_MARKER)
            .expect("end marker should be inserted");
        let managed_block = &agents_content[start..end + CTX_END_MARKER.len()];

        assert!(start < end);
        assert_eq!(agents_content.matches(CTX_START_MARKER).count(), 1);
        assert_eq!(agents_content.matches(CTX_END_MARKER).count(), 1);
        assert!(managed_block.starts_with(&format!("{CTX_START_MARKER}\n")));
        assert!(managed_block.ends_with(CTX_END_MARKER));
        assert!(managed_block.contains("# CTX Codex Session Context"));
        assert!(managed_block.contains("Preset: Marker Boundary"));
        assert!(managed_block.contains("## Selected Rules"));
        assert!(managed_block.contains("This belongs inside the managed block."));
        assert!(!managed_block.contains("This must not be injected."));
        assert!(agents_content[..start].contains("Manual project rule."));

        drop(plan);
    }

    #[test]
    fn codex_launch_plan_injects_embedded_manifest_payload_into_startup_agents_md() {
        let workspace = temp_workspace();
        let agents_md = workspace.join(AGENTS_MD_FILE_NAME);
        fs::write(&agents_md, "# Existing Project Rules\n\nManual rule.\n")
            .expect("existing AGENTS.md should be writable");

        let reviewer = context("Reviewer Notes", "# Reviewer\n\nCheck correctness risks.");
        let implementer = context("Implementer Notes", "# Implementer\n\nMake the patch.");
        let mut preset = Preset {
            preset_id: Uuid::new_v4(),
            preset_name: "Codex Delegated Review".to_string(),
            preset_contexts: vec![implementer.context_id, reviewer.context_id],
            preset_metadata: Default::default(),
            preset_context_composition: vec![
                ctx_core::PresetContextComposition {
                    context_id: implementer.context_id,
                    order: 0,
                    source_ref: "subagents/implementer.md".to_string(),
                    required: true,
                    selection: Default::default(),
                },
                ctx_core::PresetContextComposition {
                    context_id: reviewer.context_id,
                    order: 10,
                    source_ref: "subagents/reviewer.md".to_string(),
                    required: true,
                    selection: Default::default(),
                },
            ],
            preset_target_cli: CliTarget::Codex,
            preset_working_dir: workspace.clone(),
            preset_model: Some("gpt-5.3-codex".to_string()),
            subagent_manifest: None,
        };
        preset.subagent_manifest = Some(SubagentManifest {
            manifest_version: Some("1".to_string()),
            roles: vec![ctx_core::SubagentRole {
                role_id: "reviewer".to_string(),
                role_name: "Reviewer".to_string(),
                role: "Code review subagent".to_string(),
                capabilities: vec!["correctness review".to_string()],
                constraints: vec!["Return findings first.".to_string()],
                metadata: Default::default(),
                description: Some("Reviews implementation risk.".to_string()),
                assigned_contexts: vec!["subagents/reviewer.md".to_string()],
                spawn_instructions: vec!["Review the launched Codex session.".to_string()],
                spawn_guidance: ctx_core::models::SubagentSpawnGuidance {
                    select_when: vec!["Use after implementation changes.".to_string()],
                    avoid_when: vec!["Avoid when no code changed.".to_string()],
                    delegation_prompt: Some("Return findings first.".to_string()),
                },
                handoff_targets: Vec::new(),
                model: Some("gpt-5.3-codex".to_string()),
            }],
            handoff_constraints: Default::default(),
        });

        let mut plan = build_codex_launch_plan(orchestration(
            preset,
            vec![implementer.clone(), reviewer.clone()],
            Vec::new(),
        ))
        .expect("Codex launch plan should include embedded manifest startup payload");
        let agents_content =
            fs::read_to_string(&agents_md).expect("AGENTS.md should be readable after injection");
        let start = agents_content
            .find(CTX_START_MARKER)
            .expect("start marker should be inserted");
        let end = agents_content
            .find(CTX_END_MARKER)
            .expect("end marker should be inserted");
        let managed_block = &agents_content[start..end + CTX_END_MARKER.len()];

        assert!(plan.embedded_manifest.is_some());
        assert!(managed_block.contains("# CTX Codex Session Context"));
        assert!(managed_block.contains("# CTX Wrapper Embedded Subagent Payload"));
        assert!(managed_block.contains("before Codex CLI startup"));
        assert!(managed_block.contains("```ctx-subagent-manifest\n"));
        assert!(managed_block.contains("\"id\": \"reviewer\""));
        assert!(managed_block.contains("# CTX Embedded Subagent Context: reviewer"));
        assert!(managed_block.contains("source_ref: subagents/reviewer.md"));
        assert!(managed_block.contains("Check correctness risks."));
        assert!(
            managed_block
                .find("# CTX Wrapper Embedded Subagent Payload")
                .expect("embedded payload should be present")
                > managed_block
                    .find("# CTX Codex Session Context")
                    .expect("main startup context should be present")
        );
        assert!(agents_content[..start].contains("Manual rule."));

        plan.program = "sh".to_string();
        plan.args = vec![
            "-c".to_string(),
            "grep -q '# CTX Wrapper Embedded Subagent Payload' AGENTS.md && grep -q 'source_ref: subagents/reviewer.md' AGENTS.md && grep -q 'Check correctness risks.' AGENTS.md".to_string(),
        ];
        let exit_code =
            run_wrapped_codex_session(plan).expect("wrapped child should see startup manifest");

        assert_eq!(exit_code, 0);
        let cleaned_content =
            fs::read_to_string(&agents_md).expect("existing AGENTS.md should remain after cleanup");
        assert!(cleaned_content.contains("Manual rule."));
        assert!(!cleaned_content.contains("# CTX Wrapper Embedded Subagent Payload"));
        assert!(!cleaned_content.contains("Check correctness risks."));
    }

    #[test]
    fn wrapped_codex_session_keeps_subagent_spawn_guidance_referenceable_until_exit() {
        let workspace = temp_workspace();
        let agents_md = workspace.join(AGENTS_MD_FILE_NAME);
        let reviewer = context(
            "Reviewer Spawn Notes",
            "# Reviewer\n\nUse this context when spawned for review.",
        );
        let preset = Preset {
            preset_id: Uuid::new_v4(),
            preset_name: "Codex Spawn Guidance".to_string(),
            preset_contexts: vec![reviewer.context_id],
            preset_metadata: Default::default(),
            preset_context_composition: vec![ctx_core::PresetContextComposition {
                context_id: reviewer.context_id,
                order: 0,
                source_ref: "subagents/reviewer.md".to_string(),
                required: true,
                selection: Default::default(),
            }],
            preset_target_cli: CliTarget::Codex,
            preset_working_dir: workspace.clone(),
            preset_model: None,
            subagent_manifest: Some(SubagentManifest {
                manifest_version: Some("1".to_string()),
                roles: vec![ctx_core::SubagentRole {
                    role_id: "reviewer".to_string(),
                    role_name: "Reviewer".to_string(),
                    role: "Code review subagent".to_string(),
                    capabilities: vec!["correctness review".to_string()],
                    constraints: vec!["Return findings first.".to_string()],
                    metadata: Default::default(),
                    description: Some("Reviews implementation risk.".to_string()),
                    assigned_contexts: vec!["subagents/reviewer.md".to_string()],
                    spawn_instructions: vec![
                        "Read the referenced reviewer context before reporting.".to_string(),
                    ],
                    spawn_guidance: ctx_core::models::SubagentSpawnGuidance {
                        select_when: vec![
                            "Spawn after implementation changes need review.".to_string()
                        ],
                        avoid_when: vec!["Avoid before there is a patch to inspect.".to_string()],
                        delegation_prompt: Some(
                            "Review changed files and return findings first.".to_string(),
                        ),
                    },
                    handoff_targets: Vec::new(),
                    model: Some("gpt-5.3-codex".to_string()),
                }],
                handoff_constraints: Default::default(),
            }),
        };
        let mut plan = build_codex_launch_plan(orchestration(preset, vec![reviewer], Vec::new()))
            .expect("Codex launch plan should be created with embedded spawn guidance");
        plan.program = "sh".to_string();
        plan.args = vec![
            "-c".to_string(),
            concat!(
                "grep -Fq '\"spawn_guidance\"' AGENTS.md && ",
                "grep -Fq 'Spawn after implementation changes need review.' AGENTS.md && ",
                "grep -Fq 'Review changed files and return findings first.' AGENTS.md && ",
                "grep -Fq 'source_ref: subagents/reviewer.md' AGENTS.md && ",
                "grep -Fq 'Use this context when spawned for review.' AGENTS.md"
            )
            .to_string(),
        ];

        let exit_code = run_wrapped_codex_session(plan)
            .expect("wrapped child should read subagent spawn guidance during active session");

        assert_eq!(exit_code, 0);
        assert!(
            !agents_md.exists(),
            "managed AGENTS.md fixture should be removed after the active session exits"
        );
    }

    #[test]
    fn codex_launch_plan_replaces_existing_managed_agents_md_block_in_place() {
        let workspace = temp_workspace();
        let agents_md = workspace.join(AGENTS_MD_FILE_NAME);
        fs::write(
            &agents_md,
            "# Existing Project Rules\n\n<!-- [ctx:start] -->\nOld ctx block\n<!-- [ctx:end] -->\n\nManual project rule.\n",
        )
        .expect("existing AGENTS.md should be writable");

        let context = context("Fresh Rules", "Use fresh context.");
        let preset = Preset {
            preset_id: Uuid::new_v4(),
            preset_name: "Codex Residual Detection".to_string(),
            preset_contexts: vec![context.context_id],
            preset_metadata: Default::default(),
            preset_context_composition: Vec::new(),
            preset_target_cli: CliTarget::Codex,
            preset_working_dir: workspace,
            preset_model: None,
            subagent_manifest: None,
        };

        let plan = build_codex_launch_plan(orchestration(preset, vec![context], Vec::new()))
            .expect("valid existing managed block should be replaced during launch planning");
        let agents_content =
            fs::read_to_string(&agents_md).expect("AGENTS.md should contain fresh injection");
        let project_rules_index = agents_content
            .find("# Existing Project Rules")
            .expect("prefix should remain");
        let fresh_context_index = agents_content
            .find("Use fresh context.")
            .expect("fresh context should be injected");
        let manual_rule_index = agents_content
            .find("Manual project rule.")
            .expect("suffix should remain");

        assert!(agents_content.contains("# Existing Project Rules"));
        assert!(agents_content.contains("Manual project rule."));
        assert!(!agents_content.contains("Old ctx block"));
        assert!(agents_content.contains("Use fresh context."));
        assert!(project_rules_index < fresh_context_index);
        assert!(fresh_context_index < manual_rule_index);
        assert_eq!(agents_content.matches(CTX_START_MARKER).count(), 1);
        assert_eq!(agents_content.matches(CTX_END_MARKER).count(), 1);

        drop(plan);
    }

    #[test]
    fn codex_launch_plan_refuses_startup_when_managed_markers_are_malformed() {
        let workspace = temp_workspace();
        let agents_md = workspace.join(AGENTS_MD_FILE_NAME);
        fs::write(
            &agents_md,
            "# Existing Project Rules\n\n<!-- [ctx:start] -->\nMalformed stale ctx block\n",
        )
        .expect("existing AGENTS.md should be writable");

        let context = context("Fresh Rules", "Use fresh context.");
        let preset = Preset {
            preset_id: Uuid::new_v4(),
            preset_name: "Codex Residual Cleanup Failure".to_string(),
            preset_contexts: vec![context.context_id],
            preset_metadata: Default::default(),
            preset_context_composition: Vec::new(),
            preset_target_cli: CliTarget::Codex,
            preset_working_dir: workspace,
            preset_model: None,
            subagent_manifest: None,
        };

        let error = build_codex_launch_plan(orchestration(preset, vec![context], Vec::new()))
            .expect_err("malformed residual markers should gate Codex startup");
        let agents_content =
            fs::read_to_string(&agents_md).expect("AGENTS.md should remain readable");

        assert!(error.contains("Cannot launch Codex"));
        assert!(error.contains("managed AGENTS.md context block"));
        assert!(error.contains("Safety: AGENTS.md was not overwritten."));
        assert!(error.contains("Next step: fix malformed or duplicate ctx managed markers"));
        assert!(error.contains("found start marker without end marker"));
        assert!(agents_content.contains("Malformed stale ctx block"));
        assert!(!agents_content.contains("Use fresh context."));
    }

    #[test]
    fn codex_launch_plan_refuses_startup_when_multiple_residual_marker_blocks_exist() {
        let workspace = temp_workspace();
        let agents_md = workspace.join(AGENTS_MD_FILE_NAME);
        fs::write(
            &agents_md,
            concat!(
                "# Existing Project Rules\n\n",
                "<!-- [ctx:start] -->\n",
                "First stale ctx block\n",
                "<!-- [ctx:end] -->\n\n",
                "Manual project rule.\n\n",
                "<!-- [ctx:start] -->\n",
                "Second stale ctx block\n",
                "<!-- [ctx:end] -->\n",
            ),
        )
        .expect("existing AGENTS.md should be writable");

        let context = context("Fresh Rules", "Use fresh context.");
        let preset = Preset {
            preset_id: Uuid::new_v4(),
            preset_name: "Codex Multiple Residual Blocks".to_string(),
            preset_contexts: vec![context.context_id],
            preset_metadata: Default::default(),
            preset_context_composition: Vec::new(),
            preset_target_cli: CliTarget::Codex,
            preset_working_dir: workspace,
            preset_model: None,
            subagent_manifest: None,
        };

        let error = build_codex_launch_plan(orchestration(preset, vec![context], Vec::new()))
            .expect_err("ambiguous residual markers should gate Codex startup");
        let agents_content =
            fs::read_to_string(&agents_md).expect("AGENTS.md should remain readable");

        assert!(error.contains("Cannot launch Codex"));
        assert!(error.contains("managed AGENTS.md context block"));
        assert!(error.contains("Safety: AGENTS.md was not overwritten."));
        assert!(error.contains("Next step: fix malformed or duplicate ctx managed markers"));
        assert!(error.contains("found multiple sections"));
        assert!(agents_content.contains("First stale ctx block"));
        assert!(agents_content.contains("Second stale ctx block"));
        assert!(agents_content.contains("Manual project rule."));
        assert!(!agents_content.contains("Use fresh context."));
    }

    #[test]
    fn wrapped_codex_session_removes_new_agents_md_after_child_exits() {
        let workspace = temp_workspace();
        let context = context("Session Rules", "Clean up managed context.");
        let preset = Preset {
            preset_id: Uuid::new_v4(),
            preset_name: "Codex Cleanup".to_string(),
            preset_contexts: vec![context.context_id],
            preset_metadata: Default::default(),
            preset_context_composition: Vec::new(),
            preset_target_cli: CliTarget::Codex,
            preset_working_dir: workspace.clone(),
            preset_model: None,
            subagent_manifest: None,
        };
        let mut plan = build_codex_launch_plan(orchestration(preset, vec![context], Vec::new()))
            .expect("Codex launch plan should be created");
        plan.program = "true".to_string();

        let exit_code = run_wrapped_codex_session(plan).expect("wrapped child process should run");

        assert_eq!(exit_code, 0);
        assert!(!workspace.join(AGENTS_MD_FILE_NAME).exists());
    }

    #[test]
    fn wrapped_codex_session_forwards_model_and_passthrough_args() {
        let workspace = temp_workspace();
        let args_log = workspace.join("codex-args.log");
        let fake_codex = workspace.join("codex");
        write_executable_script(
            &fake_codex,
            &format!(
                "#!/bin/sh\nprintf '%s\\n' \"$@\" > '{}'\ngrep -q 'Codex Forwarding Rules' AGENTS.md || exit 43\nexit 6\n",
                args_log.display()
            ),
        );
        let context = context(
            "Codex Forwarding Rules",
            "This managed context must be visible before process startup.",
        );
        let preset = Preset {
            preset_id: Uuid::new_v4(),
            preset_name: "Codex Forwarding".to_string(),
            preset_contexts: vec![context.context_id],
            preset_metadata: Default::default(),
            preset_context_composition: Vec::new(),
            preset_target_cli: CliTarget::Codex,
            preset_working_dir: workspace.clone(),
            preset_model: Some("gpt-5.3-codex".to_string()),
            subagent_manifest: None,
        };
        let mut plan = build_codex_launch_plan(orchestration(
            preset,
            vec![context],
            vec![
                "--sandbox".to_string(),
                "workspace-write".to_string(),
                "--".to_string(),
                "literal user prompt".to_string(),
            ],
        ))
        .expect("Codex launch plan should be created");
        plan.program = fake_codex.display().to_string();

        let exit_code = run_wrapped_codex_session(plan).expect("fake Codex process should run");
        let forwarded_args = fs::read_to_string(args_log).expect("fake CLI should record argv");

        assert_eq!(exit_code, 6);
        assert_eq!(
            forwarded_args
                .lines()
                .map(str::to_string)
                .collect::<Vec<_>>(),
            vec![
                "--model".to_string(),
                "gpt-5.3-codex".to_string(),
                "--sandbox".to_string(),
                "workspace-write".to_string(),
                "--".to_string(),
                "literal user prompt".to_string()
            ]
        );
        assert!(
            !workspace.join(AGENTS_MD_FILE_NAME).exists(),
            "managed AGENTS.md should be removed after the child exits"
        );
    }

    #[test]
    fn launch_codex_resolves_cli_injects_agents_md_and_cleans_after_wrapped_session() {
        let _guard = HOME_ENV_LOCK
            .lock()
            .expect("process env lock should not be poisoned");
        let workspace = temp_workspace();
        let home = temp_workspace();
        let roots = VaultRoots::discover(&workspace);
        let context = create_context_file(
            &roots,
            VaultScope::Local,
            "agents",
            "codex-real-launch.md",
            "# Codex Real Launch Rules\n\nVisible through ctx launch codex.",
        )
        .expect("local Codex context should be created");
        let presets_dir = managed_presets_dir(roots.local_root.as_ref().unwrap());
        fs::create_dir_all(&presets_dir).expect("local presets dir should be created");
        fs::write(
            presets_dir.join("codex-real-wrapper.json"),
            r#"{
                "preset_name": "Codex Real Wrapper",
                "preset_target_cli": "codex",
                "preset_model": "gpt-5.3-codex",
                "preset_contexts": ["agents/codex-real-launch.md"]
            }"#,
        )
        .expect("Codex preset should be writable");

        let args_log = workspace.join("codex-launch-args.log");
        let agents_snapshot = workspace.join("codex-launch-agents-snapshot.md");
        let fake_codex = workspace.join("codex");
        write_executable_script(
            &fake_codex,
            &format!(
                "#!/bin/sh\nprintf '%s\\n' \"$@\" > '{}'\n[ -f AGENTS.md ] || exit 40\ngrep -q 'Codex Real Launch Rules' AGENTS.md || exit 41\ngrep -q 'Visible through ctx launch codex.' AGENTS.md || exit 42\ncat AGENTS.md > '{}'\nexit 5\n",
                args_log.display(),
                agents_snapshot.display()
            ),
        );

        let previous_home = env::var_os("HOME");
        let previous_codex_bin = env::var_os(CTX_CODEX_BIN_ENV);
        let previous_dir = env::current_dir().expect("current dir should resolve");
        env::set_var("HOME", &home);
        env::set_var(CTX_CODEX_BIN_ENV, &fake_codex);
        env::set_current_dir(&workspace).expect("test cwd should be set to workspace");

        let launch_result = launch_codex(LaunchArgs {
            preset_ref: Some("codex-real-wrapper".to_string()),
            session_ref: None,
            handoff_ref: None,
            passthrough_args: vec!["--sandbox".to_string(), "workspace-write".to_string()],
        });

        env::set_current_dir(previous_dir).expect("test cwd should be restored");
        match previous_home {
            Some(value) => env::set_var("HOME", value),
            None => env::remove_var("HOME"),
        }
        match previous_codex_bin {
            Some(value) => env::set_var(CTX_CODEX_BIN_ENV, value),
            None => env::remove_var(CTX_CODEX_BIN_ENV),
        }

        let exit_code = launch_result.expect("ctx launch codex should run fake Codex");
        let forwarded_args = fs::read_to_string(args_log).expect("fake Codex should record argv");
        let child_visible_agents =
            fs::read_to_string(agents_snapshot).expect("fake Codex should snapshot AGENTS.md");

        assert_eq!(exit_code, 5);
        assert_eq!(
            forwarded_args
                .lines()
                .map(str::to_string)
                .collect::<Vec<_>>(),
            vec![
                "--model".to_string(),
                "gpt-5.3-codex".to_string(),
                "--sandbox".to_string(),
                "workspace-write".to_string()
            ]
        );
        assert!(child_visible_agents.contains("# CTX Codex Session Context"));
        assert!(child_visible_agents.contains("Preset: Codex Real Wrapper"));
        assert!(child_visible_agents.contains(&context.file_path.display().to_string()));
        assert!(child_visible_agents.contains("Visible through ctx launch codex."));
        assert!(
            !workspace.join(AGENTS_MD_FILE_NAME).exists(),
            "ctx launch codex should clean the managed AGENTS.md file after child exit"
        );
    }

    #[test]
    fn launch_claude_resolves_cli_appends_prompt_file_and_cleans_after_wrapped_session() {
        let _guard = HOME_ENV_LOCK
            .lock()
            .expect("process env lock should not be poisoned");
        let workspace = temp_workspace();
        let home = temp_workspace();
        let roots = VaultRoots::discover(&workspace);
        let context = create_context_file(
            &roots,
            VaultScope::Local,
            "agents",
            "claude-real-launch.md",
            "# Claude Real Launch Rules\n\nVisible through ctx launch claude.",
        )
        .expect("local Claude context should be created");
        let presets_dir = managed_presets_dir(roots.local_root.as_ref().unwrap());
        fs::create_dir_all(&presets_dir).expect("local presets dir should be created");
        fs::write(
            presets_dir.join("claude-real-wrapper.json"),
            r#"{
                "preset_name": "Claude Real Wrapper",
                "preset_target_cli": "claude",
                "preset_model": "claude-sonnet",
                "preset_contexts": ["agents/claude-real-launch.md"]
            }"#,
        )
        .expect("Claude preset should be writable");

        let args_log = workspace.join("claude-launch-args.log");
        let prompt_path_log = workspace.join("claude-launch-prompt-path.log");
        let prompt_snapshot = workspace.join("claude-launch-prompt-snapshot.md");
        let fake_claude = workspace.join("claude");
        write_executable_script(
            &fake_claude,
            &format!(
                "#!/bin/sh\nprintf '%s\\n' \"$@\" > '{}'\nprompt_file=''\nwhile [ \"$#\" -gt 0 ]; do\n  if [ \"$1\" = '--append-system-prompt-file' ]; then\n    shift\n    prompt_file=\"$1\"\n  fi\n  shift\ndone\n[ -n \"$prompt_file\" ] || exit 40\n[ -f \"$prompt_file\" ] || exit 41\ngrep -q 'Claude Real Launch Rules' \"$prompt_file\" || exit 42\ngrep -q 'Visible through ctx launch claude.' \"$prompt_file\" || exit 43\nprintf '%s\\n' \"$prompt_file\" > '{}'\ncat \"$prompt_file\" > '{}'\nexit 5\n",
                args_log.display(),
                prompt_path_log.display(),
                prompt_snapshot.display()
            ),
        );

        let previous_home = env::var_os("HOME");
        let previous_claude_bin = env::var_os(CTX_CLAUDE_BIN_ENV);
        let previous_dir = env::current_dir().expect("current dir should resolve");
        env::set_var("HOME", &home);
        env::set_var(CTX_CLAUDE_BIN_ENV, &fake_claude);
        env::set_current_dir(&workspace).expect("test cwd should be set to workspace");

        let launch_result = launch_claude(LaunchArgs {
            preset_ref: Some("claude-real-wrapper".to_string()),
            session_ref: None,
            handoff_ref: None,
            passthrough_args: vec!["--debug".to_string(), "--print".to_string()],
        });

        env::set_current_dir(previous_dir).expect("test cwd should be restored");
        match previous_home {
            Some(value) => env::set_var("HOME", value),
            None => env::remove_var("HOME"),
        }
        match previous_claude_bin {
            Some(value) => env::set_var(CTX_CLAUDE_BIN_ENV, value),
            None => env::remove_var(CTX_CLAUDE_BIN_ENV),
        }

        let exit_code = launch_result.expect("ctx launch claude should run fake Claude");
        let forwarded_args = fs::read_to_string(args_log).expect("fake Claude should record argv");
        let child_visible_prompt =
            fs::read_to_string(prompt_snapshot).expect("fake Claude should snapshot prompt file");
        let prompt_file_path = PathBuf::from(
            fs::read_to_string(prompt_path_log)
                .expect("prompt path should be logged")
                .trim(),
        );

        assert_eq!(exit_code, 5);
        assert_eq!(
            forwarded_args
                .lines()
                .map(str::to_string)
                .collect::<Vec<_>>(),
            vec![
                "--append-system-prompt-file".to_string(),
                prompt_file_path.display().to_string(),
                "--model".to_string(),
                "claude-sonnet".to_string(),
                "--debug".to_string(),
                "--print".to_string()
            ]
        );
        assert!(child_visible_prompt.contains("# CTX Claude Session Context"));
        assert!(child_visible_prompt.contains("Preset: Claude Real Wrapper"));
        assert!(child_visible_prompt.contains(&context.file_path.display().to_string()));
        assert!(child_visible_prompt.contains("Visible through ctx launch claude."));
        assert!(
            !prompt_file_path.exists(),
            "ctx launch claude should remove the temporary prompt file after child exit"
        );
    }

    #[test]
    fn wrapped_codex_session_propagates_nonzero_child_exit_code() {
        let workspace = temp_workspace();
        let fake_codex = workspace.join("codex");
        write_executable_script(
            &fake_codex,
            "#!/bin/sh\ngrep -q 'Codex Failure Rules' AGENTS.md || exit 44\nprintf 'codex child failed\\n' >&2\nexit 27\n",
        );
        let context = context(
            "Codex Failure Rules",
            "This managed context must be visible before a failing child exits.",
        );
        let preset = Preset {
            preset_id: Uuid::new_v4(),
            preset_name: "Codex Failure Propagation".to_string(),
            preset_contexts: vec![context.context_id],
            preset_metadata: Default::default(),
            preset_context_composition: Vec::new(),
            preset_target_cli: CliTarget::Codex,
            preset_working_dir: workspace.clone(),
            preset_model: None,
            subagent_manifest: None,
        };
        let mut plan = build_codex_launch_plan(orchestration(preset, vec![context], Vec::new()))
            .expect("Codex launch plan should be created");
        plan.program = fake_codex.display().to_string();

        let exit_code = run_wrapped_codex_session(plan)
            .expect("nonzero Codex child exits should propagate as wrapper exit codes");

        assert_eq!(exit_code, 27);
        assert!(
            !workspace.join(AGENTS_MD_FILE_NAME).exists(),
            "managed AGENTS.md should be removed even when the child returns an error code"
        );
    }

    #[test]
    fn wrapped_codex_session_propagates_spawn_error_and_cleans_managed_agents_md() {
        let workspace = temp_workspace();
        let missing_codex = workspace.join("missing-codex");
        let context = context(
            "Codex Spawn Failure Rules",
            "Temporary managed context should be removed when spawn fails.",
        );
        let preset = Preset {
            preset_id: Uuid::new_v4(),
            preset_name: "Codex Spawn Failure".to_string(),
            preset_contexts: vec![context.context_id],
            preset_metadata: Default::default(),
            preset_context_composition: Vec::new(),
            preset_target_cli: CliTarget::Codex,
            preset_working_dir: workspace.clone(),
            preset_model: None,
            subagent_manifest: None,
        };
        let mut plan = build_codex_launch_plan(orchestration(preset, vec![context], Vec::new()))
            .expect("Codex launch plan should be created");
        plan.program = missing_codex.display().to_string();

        let error = run_wrapped_codex_session(plan)
            .expect_err("Codex child spawn errors should be returned to the wrapper caller");

        assert!(error.contains("Launch failed: Codex CLI did not start."));
        assert!(error.contains("Injection method: managed ctx block in AGENTS.md."));
        assert!(error.contains("Cleanup: the managed ctx block will be removed automatically"));
        assert!(error.contains("Next step: verify Codex is installed and executable"));
        assert!(error.contains(CTX_CODEX_BIN_ENV));
        assert!(error.contains(&missing_codex.display().to_string()));
        assert!(
            !workspace.join(AGENTS_MD_FILE_NAME).exists(),
            "managed AGENTS.md should be removed when the child cannot be spawned"
        );
    }

    #[test]
    fn wrapped_codex_session_records_then_removes_state_during_normal_startup() {
        let workspace = temp_workspace();
        let state_dir = temp_workspace();
        let context = context("Session Rules", "Record transient state.");
        let preset = Preset {
            preset_id: Uuid::new_v4(),
            preset_name: "Codex State Lifecycle".to_string(),
            preset_contexts: vec![context.context_id],
            preset_metadata: Default::default(),
            preset_context_composition: Vec::new(),
            preset_target_cli: CliTarget::Codex,
            preset_working_dir: workspace.clone(),
            preset_model: None,
            subagent_manifest: None,
        };
        let mut plan = build_codex_launch_plan(orchestration(preset, vec![context], Vec::new()))
            .expect("Codex launch plan should be created");
        let state_dir_arg = state_dir.display().to_string();
        plan.program = "sh".to_string();
        plan.args = vec![
            "-c".to_string(),
            format!(
                "i=0; while [ $i -lt 50 ]; do count=$(find '{}' -name '*.json' 2>/dev/null | wc -l | tr -d ' '); [ \"$count\" = 1 ] && exit 0; i=$((i + 1)); sleep 0.02; done; exit 1",
                state_dir_arg
            ),
        ];
        plan.state_dir = state_dir.clone();

        let exit_code = run_wrapped_codex_session(plan).expect("wrapped child process should run");
        let remaining_state_files = fs::read_dir(&state_dir)
            .expect("state directory should remain readable")
            .filter_map(Result::ok)
            .filter(|entry| {
                entry.path().extension().and_then(|value| value.to_str()) == Some("json")
            })
            .count();

        assert_eq!(exit_code, 0);
        assert_eq!(remaining_state_files, 0);
        assert!(!workspace.join(AGENTS_MD_FILE_NAME).exists());
    }

    #[cfg(unix)]
    #[test]
    fn wrapped_codex_session_maps_child_signal_and_still_cleans_managed_agents_md() {
        let workspace = temp_workspace();
        let state_dir = temp_workspace();
        let context = context(
            "Interrupt Rules",
            "This context should be available until the interrupted child exits.",
        );
        let preset = Preset {
            preset_id: Uuid::new_v4(),
            preset_name: "Codex Interrupt Cleanup".to_string(),
            preset_contexts: vec![context.context_id],
            preset_metadata: Default::default(),
            preset_context_composition: Vec::new(),
            preset_target_cli: CliTarget::Codex,
            preset_working_dir: workspace.clone(),
            preset_model: None,
            subagent_manifest: None,
        };
        let mut plan = build_codex_launch_plan(orchestration(preset, vec![context], Vec::new()))
            .expect("Codex launch plan should be created");
        let state_dir_arg = state_dir.display().to_string();
        plan.program = "sh".to_string();
        plan.args = vec![
            "-c".to_string(),
            format!(
                "i=0; while [ $i -lt 50 ]; do count=$(find '{}' -name '*.json' 2>/dev/null | wc -l | tr -d ' '); [ \"$count\" = 1 ] && break; i=$((i + 1)); sleep 0.02; done; grep -q 'Interrupt Rules' AGENTS.md || exit 44; kill -TERM $$",
                state_dir_arg
            ),
        ];
        plan.state_dir = state_dir.clone();

        let exit_code = run_wrapped_codex_session(plan)
            .expect("child interrupt should not prevent Codex wrapper cleanup");
        let remaining_state_files = json_file_count(&state_dir);

        assert_eq!(exit_code, 143);
        assert_eq!(remaining_state_files, 0);
        assert!(
            !workspace.join(AGENTS_MD_FILE_NAME).exists(),
            "managed AGENTS.md should be removed after an interrupted child exits"
        );
    }

    #[test]
    fn wrapped_codex_session_removes_only_managed_agents_md_section_after_child_exits() {
        let workspace = temp_workspace();
        let agents_md = workspace.join(AGENTS_MD_FILE_NAME);
        fs::write(
            &agents_md,
            "# Existing Project Rules\n\nManual rule before launch.\n",
        )
        .expect("existing AGENTS.md should be writable");

        let context = context("Session Rules", "Temporary managed context.");
        let preset = Preset {
            preset_id: Uuid::new_v4(),
            preset_name: "Codex Existing File Cleanup".to_string(),
            preset_contexts: vec![context.context_id],
            preset_metadata: Default::default(),
            preset_context_composition: Vec::new(),
            preset_target_cli: CliTarget::Codex,
            preset_working_dir: workspace.clone(),
            preset_model: None,
            subagent_manifest: None,
        };
        let mut plan = build_codex_launch_plan(orchestration(preset, vec![context], Vec::new()))
            .expect("Codex launch plan should be created");
        plan.program = "sh".to_string();
        plan.args = vec![
            "-c".to_string(),
            "printf '\\nManual rule added during session.\\n' >> AGENTS.md".to_string(),
        ];

        let exit_code = run_wrapped_codex_session(plan).expect("wrapped child process should run");
        let cleaned_content =
            fs::read_to_string(&agents_md).expect("AGENTS.md should remain after cleanup");

        assert_eq!(exit_code, 0);
        assert!(cleaned_content.contains("# Existing Project Rules"));
        assert!(cleaned_content.contains("Manual rule before launch."));
        assert!(cleaned_content.contains("Manual rule added during session."));
        assert!(!cleaned_content.contains("Temporary managed context."));
        assert!(!cleaned_content.contains("<!-- [ctx:start] -->"));
        assert!(!cleaned_content.contains("<!-- [ctx:end] -->"));
    }

    #[test]
    fn wrapped_codex_session_cleanup_failure_does_not_mask_child_exit_code() {
        let workspace = temp_workspace();
        let agents_md = workspace.join(AGENTS_MD_FILE_NAME);
        let context = context("Session Rules", "Temporary managed context.");
        let preset = Preset {
            preset_id: Uuid::new_v4(),
            preset_name: "Codex Cleanup Failure".to_string(),
            preset_contexts: vec![context.context_id],
            preset_metadata: Default::default(),
            preset_context_composition: Vec::new(),
            preset_target_cli: CliTarget::Codex,
            preset_working_dir: workspace.clone(),
            preset_model: None,
            subagent_manifest: None,
        };
        let mut plan = build_codex_launch_plan(orchestration(preset, vec![context], Vec::new()))
            .expect("Codex launch plan should be created");
        plan.program = "sh".to_string();
        plan.args = vec![
            "-c".to_string(),
            "printf '<!-- [ctx:start] -->\\ncorrupted during session\\n' > AGENTS.md".to_string(),
        ];

        let exit_code = run_wrapped_codex_session(plan).expect("wrapped child process should run");
        let remaining_content =
            fs::read_to_string(&agents_md).expect("AGENTS.md should remain for manual recovery");

        assert_eq!(exit_code, 0);
        assert!(remaining_content.contains("corrupted during session"));
        assert!(remaining_content.contains("<!-- [ctx:start] -->"));
        assert!(!remaining_content.contains("<!-- [ctx:end] -->"));
    }

    fn context(title: &str, content: &str) -> ContextFragment {
        ContextFragment {
            context_id: Uuid::new_v4(),
            title: title.to_string(),
            content: content.to_string(),
            file_path: PathBuf::from(format!("/vault/contexts/{title}.md")),
            vault_scope: VaultScope::Global,
            classification: Classification::Shared,
            import_classification_suggestion: Some(Classification::Shared),
            inferred_classification: Some(Classification::Shared),
            tags: Vec::new(),
            folder_path: PathBuf::new(),
            wikilinks: Vec::new(),
            backlinks: Vec::new(),
            import_source: None,
            import_source_type: None,
            llm_classification_status: ClassificationStatus::Reviewed,
            session_handoff_classification: None,
        }
    }

    fn saved_handoff_context(target: CliTarget, handoff_markdown: &str) -> SessionHandoffContext {
        let injection_method = match target {
            CliTarget::Claude => InjectionStrategy::AppendSystemPromptFile,
            CliTarget::Codex => InjectionStrategy::AgentsMdSectionMarkerMerge,
        };

        SessionHandoffContext {
            source_tool: SessionLogProvider::Codex,
            source_session_ref: "saved-launch-selection-1".to_string(),
            source_working_directory: "/workspace/saved-launch".to_string(),
            source_log_path: "/workspace/saved-launch/session.jsonl".to_string(),
            source_updated_at: Some("2026-05-11T00:00:00Z".to_string()),
            title: "Saved launch selection".to_string(),
            category: WorkContextCategory::Implementation,
            categories: vec![
                WorkContextCategory::Implementation,
                WorkContextCategory::Launch,
            ],
            classification_status: ClassificationStatus::Reviewed,
            classification_confidence_score: 92,
            classification_rationale: "Saved session handoff is ready for launch.".to_string(),
            goals: vec!["Preserve selected saved handoff context.".to_string()],
            summary: "Saved handoff launch selection is preserved.".to_string(),
            key_changed_files: vec!["crates/ctx-cli/src/bin/ctx.rs".to_string()],
            commands: vec!["cargo test -p ctx-cli".to_string()],
            decisions: vec![
                "Use the saved handoff markdown, not regenerated scan output.".to_string(),
            ],
            verification_results: vec!["cargo test -p ctx-cli".to_string()],
            remaining_work: vec!["Launch the selected saved context.".to_string()],
            created_at: "2026-05-11T00:01:00Z".to_string(),
            handoff_markdown: handoff_markdown.to_string(),
            tags: vec!["session-history".to_string(), "resume-context".to_string()],
            cleanup_applied: true,
            refine_mode: WorkContextRefineMode::Refined,
            launch_target: target,
            injection_method,
        }
    }

    fn temp_workspace() -> PathBuf {
        let workspace = env::temp_dir().join(format!("ctx-codex-test-{}", Uuid::new_v4()));
        fs::create_dir_all(&workspace).expect("test workspace should be created");
        workspace
    }

    fn with_home<T>(home: &Path, test: impl FnOnce() -> T) -> T {
        let _guard = HOME_ENV_LOCK
            .lock()
            .expect("HOME env lock should not be poisoned");
        let previous_home = env::var_os("HOME");
        env::set_var("HOME", home);
        let output = test();
        match previous_home {
            Some(value) => env::set_var("HOME", value),
            None => env::remove_var("HOME"),
        }
        output
    }

    fn json_file_count(path: &Path) -> usize {
        fs::read_dir(path)
            .expect("directory should remain readable")
            .filter_map(Result::ok)
            .filter(|entry| {
                entry.path().extension().and_then(|value| value.to_str()) == Some("json")
            })
            .count()
    }

    fn write_executable_script(path: &Path, content: &str) {
        fs::write(path, content).expect("script should be writable");

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut permissions = fs::metadata(path)
                .expect("script metadata should be readable")
                .permissions();
            permissions.set_mode(0o755);
            fs::set_permissions(path, permissions).expect("script should be executable");
        }
    }

    fn write_mock_child_executable(path: &Path, content: &str) {
        write_executable_script(path, content);
    }

    fn prepend_path(bin_dir: &Path, previous_path: Option<&OsString>) -> OsString {
        let mut entries = vec![bin_dir.to_path_buf()];
        if let Some(previous_path) = previous_path {
            entries.extend(env::split_paths(previous_path));
        }

        env::join_paths(entries).expect("mock PATH should be joinable")
    }

    fn restore_env_var(name: &str, previous: Option<&OsString>) {
        match previous {
            Some(value) => env::set_var(name, value),
            None => env::remove_var(name),
        }
    }

    fn read_arg_log(path: &Path) -> Vec<String> {
        fs::read_to_string(path)
            .expect("mock child should record argv")
            .lines()
            .map(str::to_string)
            .collect()
    }

    fn orchestration(
        preset: Preset,
        contexts: Vec<ContextFragment>,
        passthrough_args: Vec<String>,
    ) -> WrapperStartupOrchestration {
        let embedded_manifest = resolve_embedded_launch_manifest(&preset, &contexts)
            .expect("test launch manifest should resolve");

        WrapperStartupOrchestration {
            preset,
            contexts,
            passthrough_args,
            embedded_manifest,
        }
    }
}
