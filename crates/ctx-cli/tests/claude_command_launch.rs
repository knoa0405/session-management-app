use ctx_core::{create_context_file, managed_presets_dir, VaultRoots, VaultScope};
use std::{
    env, fs,
    path::{Path, PathBuf},
    process::Command,
};
use uuid::Uuid;

const CTX_CLAUDE_BIN_ENV: &str = "CTX_CLAUDE_BIN";
const CTX_PROMPT_DIR: &str = "ctx/claude-prompts";

#[test]
fn ctx_launch_claude_uses_mocked_executable_append_prompt_file_and_cleans_up() {
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
        "claude-command-launch.md",
        "# Claude Command Launch Rules\n\nVisible to the mocked Claude command.",
    )
    .expect("local Claude context should be created");
    let local_root = roots
        .local_root
        .as_ref()
        .expect("test roots should include a local vault");
    let presets_dir = managed_presets_dir(local_root);
    fs::create_dir_all(&presets_dir).expect("local presets dir should be created");
    fs::write(
        presets_dir.join("claude-command-wrapper.json"),
        r#"{
            "preset_name": "Claude Command Wrapper",
            "preset_target_cli": "claude",
            "preset_model": "claude-sonnet",
            "preset_contexts": ["agents/claude-command-launch.md"]
        }"#,
    )
    .expect("Claude preset should be writable");

    let args_log = workspace.join("mock-claude-args.log");
    let prompt_path_log = workspace.join("mock-claude-prompt-path.log");
    let prompt_snapshot = workspace.join("mock-claude-prompt-snapshot.md");
    let fake_claude = workspace.join("claude");
    write_executable_script(
        &fake_claude,
        &format!(
            "#!/bin/sh\nprintf '%s\\n' \"$@\" > '{}'\nprompt_file=''\nwhile [ \"$#\" -gt 0 ]; do\n  if [ \"$1\" = '--append-system-prompt-file' ]; then\n    shift\n    prompt_file=\"$1\"\n  fi\n  shift\ndone\n[ -n \"$prompt_file\" ] || exit 40\n[ -f \"$prompt_file\" ] || exit 41\ngrep -q 'Claude Command Launch Rules' \"$prompt_file\" || exit 42\ngrep -q 'Visible to the mocked Claude command.' \"$prompt_file\" || exit 43\nprintf '%s\\n' \"$prompt_file\" > '{}'\ncat \"$prompt_file\" > '{}'\nexit 5\n",
            args_log.display(),
            prompt_path_log.display(),
            prompt_snapshot.display()
        ),
    );

    let status = Command::new(env!("CARGO_BIN_EXE_ctx"))
        .args([
            "launch",
            "claude",
            "--preset",
            "claude-command-wrapper",
            "--",
            "--debug",
        ])
        .current_dir(&workspace)
        .env("HOME", &home)
        .env(CTX_CLAUDE_BIN_ENV, &fake_claude)
        .status()
        .expect("ctx launch claude command should execute");

    let prompt_file_path = PathBuf::from(
        fs::read_to_string(prompt_path_log)
            .expect("mock Claude should record prompt path")
            .trim(),
    );
    let forwarded_args = fs::read_to_string(args_log).expect("mock Claude should record argv");
    let child_visible_prompt =
        fs::read_to_string(prompt_snapshot).expect("mock Claude should snapshot prompt file");

    assert_eq!(status.code(), Some(5));
    assert_eq!(
        forwarded_args.lines().collect::<Vec<_>>(),
        vec![
            "--append-system-prompt-file",
            prompt_file_path
                .to_str()
                .expect("temporary prompt path should be valid UTF-8"),
            "--model",
            "claude-sonnet",
            "--debug"
        ]
    );
    assert!(child_visible_prompt.contains("# CTX Claude Session Context"));
    assert!(child_visible_prompt.contains("Preset: Claude Command Wrapper"));
    assert!(child_visible_prompt.contains(&context.file_path.display().to_string()));
    assert!(child_visible_prompt.contains("Visible to the mocked Claude command."));
    assert!(
        !prompt_file_path.exists(),
        "ctx launch claude should remove the temporary prompt file after the mock exits"
    );
}

#[test]
fn ctx_launch_claude_cleans_prompt_file_when_child_spawn_fails() {
    let workspace = temp_workspace();
    let home = temp_workspace();
    let temp_root = temp_workspace();
    let roots = VaultRoots {
        global_root: home.join(".ctx").join("vault"),
        local_root: Some(workspace.join(".ctx").join("vault")),
    };
    create_context_file(
        &roots,
        VaultScope::Local,
        "agents",
        "claude-failed-spawn.md",
        "# Claude Failed Spawn\n\nThis prompt must not be left behind.",
    )
    .expect("local Claude context should be created");
    let local_root = roots
        .local_root
        .as_ref()
        .expect("test roots should include a local vault");
    let presets_dir = managed_presets_dir(local_root);
    fs::create_dir_all(&presets_dir).expect("local presets dir should be created");
    fs::write(
        presets_dir.join("claude-failed-spawn.json"),
        r#"{
            "preset_name": "Claude Failed Spawn",
            "preset_target_cli": "claude",
            "preset_contexts": ["agents/claude-failed-spawn.md"]
        }"#,
    )
    .expect("Claude preset should be writable");

    let fake_claude = workspace.join("claude");
    write_executable_script(
        &fake_claude,
        "#!/definitely/missing/ctx-test-shell\nexit 99\n",
    );

    let status = Command::new(env!("CARGO_BIN_EXE_ctx"))
        .args(["launch", "claude", "--preset", "claude-failed-spawn"])
        .current_dir(&workspace)
        .env("HOME", &home)
        .env("TMPDIR", &temp_root)
        .env(CTX_CLAUDE_BIN_ENV, &fake_claude)
        .status()
        .expect("ctx launch claude command should execute");

    assert_eq!(status.code(), Some(1));
    assert!(
        prompt_dir_is_empty(&temp_root),
        "ctx launch claude should remove temporary prompt files when the child process fails to spawn"
    );
}

fn temp_workspace() -> PathBuf {
    let workspace = env::temp_dir().join(format!("ctx-claude-command-test-{}", Uuid::new_v4()));
    fs::create_dir_all(&workspace).expect("test workspace should be created");
    workspace
}

fn prompt_dir_is_empty(temp_root: &Path) -> bool {
    let prompt_dir = temp_root.join(CTX_PROMPT_DIR);
    if !prompt_dir.exists() {
        return true;
    }

    fs::read_dir(prompt_dir)
        .expect("prompt dir should be readable")
        .next()
        .is_none()
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
