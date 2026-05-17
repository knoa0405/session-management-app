use ctx_core::{
    create_context_file, create_session_handoff_context_file, managed_presets_dir,
    ClassificationStatus, CliTarget, InjectionStrategy, SessionHandoffContext, SessionLogProvider,
    VaultRoots, VaultScope, WorkContextCategory, WorkContextRefineMode,
};
use std::{
    env, fs,
    path::{Path, PathBuf},
    process::Command,
};
use uuid::Uuid;

const CTX_CODEX_BIN_ENV: &str = "CTX_CODEX_BIN";
const AGENTS_MD_FILE_NAME: &str = "AGENTS.md";
const CTX_START_MARKER: &str = "<!-- [ctx:start] -->";
const CTX_END_MARKER: &str = "<!-- [ctx:end] -->";

#[test]
fn ctx_launch_codex_uses_mocked_executable_injects_agents_md_and_cleans_up() {
    let workspace = temp_workspace();
    let home = temp_workspace();
    let roots = VaultRoots {
        global_root: home.join(".ctx").join("vault"),
        local_root: Some(workspace.join(".ctx").join("vault")),
    };
    let context = create_context_file(
        &roots,
        VaultScope::Local,
        "agents",
        "codex-command-launch.md",
        "# Codex Command Launch Rules\n\nVisible to the mocked Codex command.",
    )
    .expect("local Codex context should be created");
    let local_root = roots
        .local_root
        .as_ref()
        .expect("test roots should include a local vault");
    let presets_dir = managed_presets_dir(local_root);
    fs::create_dir_all(&presets_dir).expect("local presets dir should be created");
    fs::write(
        presets_dir.join("codex-command-wrapper.json"),
        r#"{
            "preset_name": "Codex Command Wrapper",
            "preset_target_cli": "codex",
            "preset_model": "gpt-5.3-codex",
            "preset_contexts": ["agents/codex-command-launch.md"]
        }"#,
    )
    .expect("Codex preset should be writable");

    let args_log = workspace.join("mock-codex-args.log");
    let agents_snapshot = workspace.join("mock-codex-agents-snapshot.md");
    let fake_codex = workspace.join("codex");
    write_executable_script(
        &fake_codex,
        &format!(
            "#!/bin/sh\nprintf '%s\\n' \"$@\" > '{}'\n[ -f AGENTS.md ] || exit 40\ngrep -q 'Codex Command Launch Rules' AGENTS.md || exit 41\ngrep -q 'Visible to the mocked Codex command.' AGENTS.md || exit 42\ncat AGENTS.md > '{}'\nexit 7\n",
            args_log.display(),
            agents_snapshot.display()
        ),
    );

    let status = Command::new(env!("CARGO_BIN_EXE_ctx"))
        .args([
            "launch",
            "codex",
            "--preset",
            "codex-command-wrapper",
            "--",
            "--sandbox",
            "workspace-write",
        ])
        .current_dir(&workspace)
        .env("HOME", &home)
        .env(CTX_CODEX_BIN_ENV, &fake_codex)
        .status()
        .expect("ctx launch codex command should execute");

    let forwarded_args = fs::read_to_string(&args_log).expect("mock Codex should record argv");
    let child_visible_agents =
        fs::read_to_string(&agents_snapshot).expect("mock Codex should snapshot AGENTS.md");

    assert_eq!(status.code(), Some(7));
    assert_eq!(
        forwarded_args.lines().collect::<Vec<_>>(),
        vec!["--model", "gpt-5.3-codex", "--sandbox", "workspace-write"]
    );
    assert!(child_visible_agents.contains(CTX_START_MARKER));
    assert!(child_visible_agents.contains(CTX_END_MARKER));
    assert!(child_visible_agents.contains("# CTX Codex Session Context"));
    assert!(child_visible_agents.contains("Preset: Codex Command Wrapper"));
    assert!(child_visible_agents.contains(&context.file_path.display().to_string()));
    assert!(child_visible_agents.contains("Visible to the mocked Codex command."));
    assert!(
        !workspace.join(AGENTS_MD_FILE_NAME).exists(),
        "ctx launch codex should clean the managed AGENTS.md file after the mock exits"
    );
}

#[test]
fn ctx_launch_codex_cleans_agents_md_when_child_spawn_fails() {
    let workspace = temp_workspace();
    let home = temp_workspace();
    let roots = VaultRoots {
        global_root: home.join(".ctx").join("vault"),
        local_root: Some(workspace.join(".ctx").join("vault")),
    };
    create_context_file(
        &roots,
        VaultScope::Local,
        "agents",
        "codex-failed-spawn.md",
        "# Codex Failed Spawn\n\nThis managed block must not be left behind.",
    )
    .expect("local Codex context should be created");
    let local_root = roots
        .local_root
        .as_ref()
        .expect("test roots should include a local vault");
    let presets_dir = managed_presets_dir(local_root);
    fs::create_dir_all(&presets_dir).expect("local presets dir should be created");
    fs::write(
        presets_dir.join("codex-failed-spawn.json"),
        r#"{
            "preset_name": "Codex Failed Spawn",
            "preset_target_cli": "codex",
            "preset_contexts": ["agents/codex-failed-spawn.md"]
        }"#,
    )
    .expect("Codex preset should be writable");

    let fake_codex = workspace.join("codex");
    write_executable_script(
        &fake_codex,
        "#!/definitely/missing/ctx-test-shell\nexit 99\n",
    );

    let status = Command::new(env!("CARGO_BIN_EXE_ctx"))
        .args(["launch", "codex", "--preset", "codex-failed-spawn"])
        .current_dir(&workspace)
        .env("HOME", &home)
        .env(CTX_CODEX_BIN_ENV, &fake_codex)
        .status()
        .expect("ctx launch codex command should execute");

    assert_eq!(status.code(), Some(1));
    assert!(
        !workspace.join(AGENTS_MD_FILE_NAME).exists(),
        "ctx launch codex should remove temporary AGENTS.md when the child process fails to spawn"
    );
}

#[test]
fn ctx_launch_codex_handoff_resolves_saved_entry_markdown_for_agents_md() {
    let workspace = temp_workspace();
    let home = temp_workspace();
    let roots = VaultRoots {
        global_root: home.join(".ctx").join("vault"),
        local_root: Some(workspace.join(".ctx").join("vault")),
    };
    let handoff_markdown = "# Previous Session Context\n\n## Handoff Summary\n\nSaved Codex handoff body is resolved from disk and injected into AGENTS.md.\n\n### Goals\n\n- Launch a saved Codex session handoff.\n\n### Key changed files\n\n- crates/ctx-cli/tests/codex_command_launch.rs\n\n### Commands\n\n- cargo test -p ctx-cli --test codex_command_launch\n\n### Decisions\n\n- Resolve the selected saved handoff entry before launching Codex.\n\n### Verification results\n\n- Mock Codex observed the saved handoff markdown in AGENTS.md.\n\n### Remaining work\n\n- Continue launch cleanup coverage.";
    let saved = create_session_handoff_context_file(
        &roots,
        VaultScope::Local,
        "session-history",
        "codex-selected-handoff.md",
        &saved_handoff_context(handoff_markdown),
    )
    .expect("saved Codex handoff should be created");
    let handoff_path = fs::canonicalize(&saved.fragment.file_path)
        .expect("saved handoff path should canonicalize for CLI resolution");

    let agents_snapshot = workspace.join("mock-codex-handoff-agents-snapshot.md");
    let fake_codex = workspace.join("codex");
    write_executable_script(
        &fake_codex,
        &format!(
            "#!/bin/sh\n[ -f AGENTS.md ] || exit 40\ngrep -q 'Saved Codex handoff body is resolved from disk' AGENTS.md || exit 41\ngrep -q 'Resolve the selected saved handoff entry before launching Codex.' AGENTS.md || exit 42\nif grep -q 'session_handoff_format_version' AGENTS.md; then exit 43; fi\ncat AGENTS.md > '{}'\nexit 6\n",
            agents_snapshot.display()
        ),
    );

    let status = Command::new(env!("CARGO_BIN_EXE_ctx"))
        .args([
            "launch",
            "codex",
            "--handoff",
            &handoff_path.display().to_string(),
        ])
        .current_dir(&workspace)
        .env("HOME", &home)
        .env(CTX_CODEX_BIN_ENV, &fake_codex)
        .status()
        .expect("ctx launch codex --handoff command should execute");

    let child_visible_agents =
        fs::read_to_string(&agents_snapshot).expect("mock Codex should snapshot AGENTS.md");

    assert_eq!(status.code(), Some(6));
    assert!(child_visible_agents.contains(CTX_START_MARKER));
    assert!(child_visible_agents.contains(CTX_END_MARKER));
    assert!(child_visible_agents.contains("# CTX Codex Session Context"));
    assert!(child_visible_agents.contains("# Previous Session Context"));
    assert!(child_visible_agents.contains("Saved Codex handoff body is resolved from disk"));
    assert!(
        !child_visible_agents.contains("session_handoff_format_version"),
        "Codex launch should expose the saved handoff body, not persisted frontmatter"
    );
    assert!(
        !workspace.join(AGENTS_MD_FILE_NAME).exists(),
        "ctx launch codex --handoff should clean the managed AGENTS.md file after the mock exits"
    );
}

fn saved_handoff_context(handoff_markdown: &str) -> SessionHandoffContext {
    SessionHandoffContext {
        source_tool: SessionLogProvider::Codex,
        source_session_ref: "codex-selected-handoff".to_string(),
        source_working_directory: "/tmp/project".to_string(),
        source_log_path: "/tmp/project/session.jsonl".to_string(),
        source_updated_at: Some("2026-05-11T00:00:00Z".to_string()),
        title: "Codex selected handoff".to_string(),
        category: WorkContextCategory::Launch,
        categories: vec![WorkContextCategory::Launch],
        classification_status: ClassificationStatus::Reviewed,
        classification_confidence_score: 95,
        classification_rationale: "Saved handoff is ready for Codex launch.".to_string(),
        goals: vec!["Launch a saved Codex session handoff.".to_string()],
        summary: "Saved Codex handoff body is resolved from disk and injected into AGENTS.md."
            .to_string(),
        key_changed_files: vec!["crates/ctx-cli/tests/codex_command_launch.rs".to_string()],
        commands: vec!["cargo test -p ctx-cli --test codex_command_launch".to_string()],
        decisions: vec![
            "Resolve the selected saved handoff entry before launching Codex.".to_string(),
        ],
        verification_results: vec![
            "Mock Codex observed the saved handoff markdown in AGENTS.md.".to_string(),
        ],
        remaining_work: vec!["Continue launch cleanup coverage.".to_string()],
        created_at: "2026-05-11T00:01:00Z".to_string(),
        handoff_markdown: handoff_markdown.to_string(),
        tags: vec!["session-history".to_string(), "codex".to_string()],
        cleanup_applied: true,
        refine_mode: WorkContextRefineMode::Refined,
        launch_target: CliTarget::Codex,
        injection_method: InjectionStrategy::AgentsMdSectionMarkerMerge,
    }
}

fn temp_workspace() -> PathBuf {
    let workspace = env::temp_dir().join(format!("ctx-codex-command-test-{}", Uuid::new_v4()));
    fs::create_dir_all(&workspace).expect("test workspace should be created");
    workspace
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
