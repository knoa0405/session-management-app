# session-management-app

CTX Context Manager is a Tauri v2 + React desktop scaffold with a bundled Rust
CLI sidecar named `ctx`. The Phase 1 target is a local markdown vault manager for
AI agent contexts, presets, and launch-time injection into Claude and Codex CLI
sessions.

## Prerequisites

- Node.js 20 or newer
- npm 10 or newer
- Rust stable toolchain with Cargo
- Tauri v2 system prerequisites for your OS

The project stores dependencies through npm and Cargo. Install them from the repo
root:

```sh
npm run install:deps
```

## Development

Run the React-only frontend during UI work:

```sh
npm run dev:react
```

Run the full desktop app with the Tauri backend and bundled `ctx` sidecar:

```sh
npm run dev:desktop
```

`npm run dev:desktop` uses the Tauri `beforeDevCommand`, which builds the `ctx`
sidecar into `src-tauri/bin/ctx-<target-triple>` before starting Vite and Tauri.
To prepare only the sidecar, run:

```sh
npm run build:sidecar
```

The CLI can also be run directly from Cargo while developing:

```sh
cargo run -p ctx-cli -- init --global
cargo run -p ctx-cli -- init --local
cargo run -p ctx-cli -- status
cargo run -p ctx-cli -- cleanup
cargo run -p ctx-cli -- list
cargo run -p ctx-cli -- classify --file ./AGENTS.md
cargo run -p ctx-cli -- import
cargo run -p ctx-cli -- launch claude
cargo run -p ctx-cli -- launch codex
```

`ctx classify` is the import-time classification API for automation and review
flows. It accepts markdown from `--file <path>` or stdin and returns a suggested
category (`main-agent`, `subagent`, or `shared`), confidence score, and rationale.

`ctx launch claude` assembles a temporary CTX prompt file and invokes Claude with
`--append-system-prompt-file`. `ctx launch codex` creates or updates the managed
`AGENTS.md` block between `<!-- [ctx:start] -->` and `<!-- [ctx:end] -->`,
launches Codex, and removes the managed block after the child process exits.
If a wrapper process is terminated abnormally, `ctx cleanup` removes stale
temporary prompt files, stale managed `AGENTS.md` marker blocks in the current
directory, and transient wrapper state for child processes that are no longer
active.

## Preset execution schema

Preset files live under `presets/*.json` inside the resolved global or local
vault. The file stem is the launch reference used by `ctx launch --preset`; if
the same stem exists in both vaults, the project-local `.ctx/presets/*.json`
file overrides the global `~/.ctx/presets/*.json` file. Phase 1 supports legacy
top-level execution keys and the explicit nested schema below:

```json
{
  "preset_id": "b7bd9b77-90e4-438d-b85e-2f504ad680c2",
  "preset_name": "Implementation",
  "preset_description": "Context pack for implementation sessions.",
  "preset_tags": ["implementation", "rust"],
  "preset_folder_path": "workflows",
  "preset_contexts": [
    "agents/rules.md",
    {
      "context_ref": "shared/rust-patterns.md",
      "order": 20,
      "required": true
    }
  ],
  "cli_execution_settings": {
    "target_cli": "codex",
    "working_dir": "/path/to/project",
    "model": "codex",
    "passthrough_args": ["--sandbox", "workspace-write"]
  },
  "wrapper_behavior": {
    "injection_strategy": "agents-md-section-marker-merge",
    "cleanup_on_exit": true,
    "cleanup_stale_on_launch": true,
    "state_dir": "/tmp/ctx/wrapper-sessions"
  },
  "subagent_manifest": {
    "manifest_version": "1",
    "roles": [
      {
        "role_id": "reviewer",
        "role_name": "Reviewer",
        "description": "Find correctness risks before handoff.",
        "assigned_contexts": ["agents/reviewer.md"],
        "spawn_instructions": ["Review the active patch and return findings."],
        "spawn_guidance": {
          "select_when": [
            "Use after implementation changes are ready for independent review."
          ],
          "avoid_when": [
            "Avoid when the task still needs code edits or repository exploration."
          ],
          "delegation_prompt": "Review changed files and return findings first."
        },
        "handoff_targets": ["implementer"],
        "model": "gpt-5.3-codex"
      }
    ],
    "handoff_constraints": {
      "require_summary": true,
      "require_changed_files": true,
      "require_open_questions": true,
      "max_parallel_subagents": 2,
      "allowed_handoff_targets": ["implementer"],
      "blocked_handoff_targets": [],
      "handoff_prompt_template": "Summarize work, changed files, and open questions."
    }
  }
}
```

Preset validation rules:

- Preset files must be valid JSON objects with a `.json` extension. Other files
  in `presets/` are ignored.
- `preset_id` is optional but, when supplied, must be a UUID. If omitted, CTX
  generates an in-memory UUID while loading the preset.
- `preset_name` is optional. If omitted, CTX derives it from the file stem by
  replacing `-` and `_` with spaces.
- `preset_contexts` is optional and defaults to an empty list. Each entry may be
  a string context ref or an object with `context_ref`, `order`, and `required`.
  Context refs match resolved vault paths case-insensitively, with `/` path
  separators, or an explicit context UUID. Missing refs make preset loading fail.
- Context composition is sorted by `order`; entries with the same order keep
  their source-array order. String entries default to their array index and
  `required: true`.
- `target_cli` accepts only `claude` or `codex`. `injection_strategy` accepts
  only `append-system-prompt-file` or `agents-md-section-marker-merge`.
- Nested `cli_execution_settings` values override legacy top-level
  `preset_target_cli`, `preset_working_dir`, and `preset_model` fields.
- `subagent_manifest.roles` entries require `role_id`, `role_name`, at least
  one `assigned_contexts` ref, at least one `spawn_instructions` entry, and
  `spawn_guidance.select_when` plus `spawn_guidance.avoid_when` rules that
  explain when to select or avoid the delegated role.
  `role_id` and handoff target refs may only contain letters, numbers, `-`,
  and `_`. Assigned context refs must be relative vault refs, not absolute paths
  or `..` traversal paths. Handoff booleans default to `true`.
- Invalid subagent delegation manifests are rejected with grouped,
  field-specific errors under `invalid subagent_manifest:` so the desktop UI and
  CLI can show all review fixes at once.

Preset defaults:

- Listing presets defaults `target_cli` to `codex`; launching defaults it to the
  requested target and rejects a preset whose explicit target does not match the
  launch target.
- `working_dir` defaults to the current/default launch directory. `model` is
  optional. `passthrough_args` defaults to an empty list.
- Claude presets default to `append-system-prompt-file`, create temporary prompt
  files under the CTX Claude prompt directory, and do not set an `AGENTS.md`
  path.
- Codex presets default to `agents-md-section-marker-merge`, set
  `agents_md_path` to `<working_dir>/AGENTS.md`, and use `<!-- [ctx:start] -->`
  and `<!-- [ctx:end] -->` markers.
- `cleanup_on_exit` and `cleanup_stale_on_launch` default to `true`.
  `state_dir` defaults to CTX wrapper session state.

Versioning strategy:

- The Phase 1 preset schema is versionless at the top level. Unknown JSON fields
  are tolerated so newer CTX versions can add optional metadata without breaking
  older presets.
- Backward compatibility is maintained for the legacy top-level execution fields
  `preset_target_cli`, `preset_working_dir`, and `preset_model`; new presets
  should prefer `cli_execution_settings`.
- Any future breaking preset change must introduce an explicit top-level
  `schema_version` and keep a migration path for current versionless presets.
- `subagent_manifest` has its own optional `manifest_version`. Phase 1 writes
  `"1"` when present; missing manifest versions are accepted for local drafts.

`subagent_manifest` is optional, but when present it defines delegated roles,
their assigned context refs, spawn instructions, selection/avoidance guidance,
allowed handoff targets, and handoff constraints the wrapper/UI can enforce
during session orchestration.
The desktop backend persists this schema through `save_preset_subagent_manifest`,
which writes the manifest into the preset JSON, preserves existing preset fields,
normalizes blank/whitespace-only strings, defaults `manifest_version` to `"1"`,
and rejects duplicate or empty `role_id` values, unsupported manifest versions,
unsafe context refs, empty role instructions, invalid handoff target refs,
missing selection/avoidance guidance, zero `max_parallel_subagents`, and targets
that are both allowed and blocked.

## Context discovery rules

`ctx scan` and vault rescans emit each discovered context with a derived
classification type and `classified` review status before LLM review.
`ctx import` materializes those discovered files into the managed vault
convention for their scope: `.ctx/vault/contexts/` for project-local context
and `~/.ctx/vault/contexts/` for global context.
Each imported item also records provenance in the vault-local
`import-metadata.json`, including the original source path and detected source
type (for example `codex-agents`, `claude-markdown`, `skill-markdown`,
`subagent-markdown`, or `context-markdown`) so later vault listings can
distinguish imported contexts, skills, and hand-created managed files.
When multiple discovered files would map to the same vault entry in the same
scope, scan assigns deterministic suffixes such as `AGENTS-2.md` before import.
Existing managed entries and prior import metadata reserve their names so
repeated scans keep imported sources stable.

- Configured scan roots are walked recursively for known context filenames:
  `CLAUDE.md`, `claude.md`, `AGENTS.md`, `agent.md`, and `agents.md`.
  These map to `main-agent` context, even when nested under `.claude`,
  `.codex`, or `agents`.
- Configured scan roots also recurse for skill material: markdown inside any
  nested `skills/` directory and standalone `SKILL.md` files are discovered as
  `shared` skill context.
- Configured skill scan roots (`skill_scan_roots` in vault settings) are walked
  recursively for markdown files and force those files to `shared` skill
  context. They support the same string and `{ "path", "scope" }` forms as
  `scan_roots`.
- Markdown under `agents/`, `subagents/`, `.claude/agents/`,
  `.claude/subagents/`, `.codex/agents/`, or `.agents/` maps to `subagent`
  context. For example, `agents/reviewer.md` maps to `subagent`.
- Markdown under `skills/`, `.claude/skills/`, `.codex/skills/`,
  `.agents/skills/`, or `.ctx/skills/` maps to `shared` context.
- Markdown files whose stem contains `agent` map to `subagent` unless they
  match the canonical main-agent filenames or live under a skills directory.
- Other discovered markdown defaults to `shared`.

## Build

Build the web frontend:

```sh
npm run build
```

Build the desktop bundle:

```sh
npm run build:desktop
```

The Tauri production build runs `npm run build:tauri-assets` first. That command
builds a release `ctx` sidecar, copies it to `src-tauri/bin/ctx-<target-triple>`,
and then builds the Vite frontend assets for packaging.

## Verification

Run the complete scaffold verification suite:

```sh
npm run verify
```

`npm run check` is an alias for the same command. The full suite runs frontend
lint/smoke checks, TypeScript checks, Rust formatting checks, workspace checks,
Clippy with warnings denied, Rust tests, and the Tauri backend check.

Focused verification commands:

```sh
npm run check:frontend
npm run check:rust
npm run check:tauri
npm run verify:rust
```

Lower-level commands are also available when isolating a failure:

```sh
npm run lint:frontend
npm run typecheck
npm run test:frontend
npm run format:rust:check
npm run lint:rust
npm run test:rust
cargo check --workspace
cargo check -p ctx-desktop
```

## Root scripts

- `npm run install:deps` installs Node dependencies and prefetches Rust crates.
- `npm run dev:react` runs the React app with Vite.
- `npm run dev:desktop` runs the Tauri desktop app in development.
- `npm run build:sidecar` builds the debug `ctx` CLI sidecar for Tauri.
- `npm run build:sidecar:release` builds the release `ctx` CLI sidecar for Tauri.
- `npm run build` builds the React frontend.
- `npm run build:desktop` builds the bundled desktop app.
- `npm run build:tauri-assets` prepares release sidecar and frontend assets for Tauri packaging.
- `npm run lint:frontend` runs dependency-free scaffold lint checks for frontend files.
- `npm run typecheck` runs TypeScript checks for the React app and Vite config.
- `npm run test:frontend` runs dependency-free frontend scaffold smoke tests.
- `npm run check:frontend` runs frontend lint, typecheck, and smoke tests.
- `npm run format:rust:check` checks Rust formatting.
- `npm run lint:rust` runs Clippy across the Rust workspace with warnings denied.
- `npm run test:rust` runs Rust workspace tests.
- `npm run check:rust` runs Rust formatting, workspace checks, clippy, and tests.
- `npm run check:tauri` validates the Tauri backend crate.
- `npm run verify:rust` runs the standalone Rust verification script.
- `npm run verify` runs the full scaffold verification suite.
- `npm run check` is an alias for `npm run verify`.
- `npm run preview` serves the built frontend locally.
- `npm run tauri`, `npm run tauri:dev`, and `npm run tauri:build` proxy Tauri CLI commands.
