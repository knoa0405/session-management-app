use ctx_core::{create_context_file, managed_presets_dir, VaultRoots, VaultScope};
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
