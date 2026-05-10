#!/usr/bin/env node
import { existsSync, readFileSync, readdirSync, statSync } from "node:fs";
import path from "node:path";

const root = process.cwd();
const mode = process.argv[2] ?? "all";
const allowedModes = new Set(["all", "lint", "test"]);

if (!allowedModes.has(mode)) {
  fail(`Unknown frontend verification mode "${mode}". Expected one of: ${[...allowedModes].join(", ")}`);
}

const sourceExtensions = new Set([".css", ".html", ".js", ".json", ".md", ".ts", ".tsx"]);
const ignoredDirectories = new Set([".git", "dist", "node_modules", "target", "src-tauri/target"]);

const requiredPackageScripts = [
  "lint:frontend",
  "typecheck",
  "test:frontend",
  "check:frontend",
  "format:rust:check",
  "lint:rust",
  "test:rust",
  "check:rust",
  "check:tauri",
  "verify",
  "check"
];

const requiredScaffoldFiles = [
  "src/App.tsx",
  "src/main.tsx",
  "src/data/mockData.ts",
  "vite.config.ts",
  "tsconfig.json",
  "tsconfig.node.json",
  "src-tauri/Cargo.toml",
  "src-tauri/tauri.conf.json",
  "src-tauri/src/lib.rs",
  "crates/ctx-core/Cargo.toml",
  "crates/ctx-cli/Cargo.toml",
  "crates/ctx-cli/src/bin/ctx.rs"
];

const failures = [];

if (mode === "all" || mode === "lint") {
  runLint();
}

if (mode === "all" || mode === "test") {
  runSmokeTests();
}

if (failures.length > 0) {
  for (const failure of failures) {
    console.error(`- ${failure}`);
  }
  process.exit(1);
}

console.log(`frontend ${mode} verification passed`);

function runLint() {
  for (const filePath of walk(root)) {
    const relativePath = path.relative(root, filePath);
    const extension = path.extname(filePath);

    if (!sourceExtensions.has(extension)) {
      continue;
    }

    const contents = readFileSync(filePath, "utf8");
    const lines = contents.split(/\n/);

    lines.forEach((line, index) => {
      if (/[ \t]$/.test(line)) {
        failures.push(`${relativePath}:${index + 1} has trailing whitespace`);
      }
    });

    if (!contents.endsWith("\n")) {
      failures.push(`${relativePath} must end with a newline`);
    }

    if ((extension === ".ts" || extension === ".tsx") && /\bdebugger\b/.test(contents)) {
      failures.push(`${relativePath} contains a debugger statement`);
    }

    if ((extension === ".ts" || extension === ".tsx") && /\bconsole\.(log|debug|info)\b/.test(contents)) {
      failures.push(`${relativePath} contains console logging`);
    }
  }

  const packageJson = readJson("package.json");
  for (const scriptName of requiredPackageScripts) {
    if (!packageJson.scripts?.[scriptName]) {
      failures.push(`package.json is missing script "${scriptName}"`);
    }
  }
}

function runSmokeTests() {
  for (const requiredFile of requiredScaffoldFiles) {
    if (!existsSync(path.join(root, requiredFile))) {
      failures.push(`missing scaffold file: ${requiredFile}`);
    }
  }

  const packageJson = readJson("package.json");
  assertScriptIncludes(packageJson, "check:frontend", "lint:frontend");
  assertScriptIncludes(packageJson, "check:frontend", "typecheck");
  assertScriptIncludes(packageJson, "check:frontend", "test:frontend");
  assertScriptIncludes(packageJson, "check:rust", "cargo check --workspace");
  assertScriptIncludes(packageJson, "check:rust", "lint:rust");
  assertScriptIncludes(packageJson, "check:rust", "test:rust");
  assertScriptIncludes(packageJson, "check:tauri", "ctx-desktop");

  const tauriConfig = readJson("src-tauri/tauri.conf.json");
  if (!tauriConfig.bundle?.externalBin?.includes("bin/ctx")) {
    failures.push("src-tauri/tauri.conf.json must bundle the ctx sidecar as bin/ctx");
  }

  const appSource = readText("src/App.tsx");
  if (!appSource.includes('invoke<CtxIntegrationProbe>("probe_ctx_integration")')) {
    failures.push("src/App.tsx must probe the Tauri ctx integration command");
  }
  if (!appSource.includes('invoke<CoreContextFragment[]>("discover_markdown_contexts"')) {
    failures.push("src/App.tsx must discover markdown contexts through the Tauri command");
  }
  if (
    !appSource.includes('invoke<string>("open_markdown_context"') ||
    !appSource.includes('invoke<string>("save_markdown_context"') ||
    !appSource.includes("Markdown context editor")
  ) {
    failures.push("src/App.tsx must open selected markdown contexts in a saveable editor");
  }
  if (
    !appSource.includes('invoke<string>("delete_markdown_context"') ||
    !appSource.includes("loadVaultLists") ||
    !appSource.includes("applyVaultLists") ||
    !appSource.includes("Refreshed vaults after deleting") ||
    !appSource.includes("Delete failed. Details:")
  ) {
    failures.push("src/App.tsx must refresh vault lists and show success/failure feedback after deletion");
  }
  for (const saveStateLabel of ["Dirty", "Saving", "Saved", "Save error"]) {
    if (!appSource.includes(saveStateLabel)) {
      failures.push(`src/App.tsx must show the "${saveStateLabel}" save state in the editor`);
    }
  }
  for (const reviewFlowText of [
    "Inspect suggestion",
    "Confirm suggestion",
    "Confirm change",
    "Discard suggestion",
    "Confirmed categories",
    "Available categories",
    "Remove ${formatClassification(classification)} category"
  ]) {
    if (!appSource.includes(reviewFlowText)) {
      failures.push(`src/App.tsx must expose classification review flow text "${reviewFlowText}"`);
    }
  }
  for (const reviewDraftHelper of [
    "ClassificationReviewDraft",
    "addReviewCategory",
    "removeReviewCategory"
  ]) {
    if (!appSource.includes(reviewDraftHelper)) {
      failures.push(`src/App.tsx must support editable classification review categories with ${reviewDraftHelper}`);
    }
  }

  const stylesSource = readText("src/styles.css");
  for (const saveStateClass of [
    ".save-state.dirty",
    ".save-state.saving",
    ".save-state.saved",
    ".save-state.error"
  ]) {
    if (!stylesSource.includes(saveStateClass)) {
      failures.push(`src/styles.css must style ${saveStateClass}`);
    }
  }
  for (const reviewFlowClass of [
    ".review-suggestion-details",
    ".review-confirmation-preview",
    ".confirmed-review-summary",
    ".review-category-editor",
    ".category-chip-button"
  ]) {
    if (!stylesSource.includes(reviewFlowClass)) {
      failures.push(`src/styles.css must style ${reviewFlowClass}`);
    }
  }

  const contextListSource = readText("src/components/ContextList.tsx");
  if (
    !contextListSource.includes("contexts.map((context)") ||
    !contextListSource.includes("context.importSource")
  ) {
    failures.push("ContextList must render discovered markdown context rows with import-source state");
  }

  const cliSource = readText("crates/ctx-cli/src/bin/ctx.rs");
  for (const command of ["status", "launch", "list", "import"]) {
    if (!cliSource.includes(`"${command}"`)) {
      failures.push(`ctx CLI scaffold must expose the "${command}" command`);
    }
  }
}

function* walk(directory) {
  const relativeDirectory = path.relative(root, directory);
  if (relativeDirectory && ignoredDirectories.has(relativeDirectory)) {
    return;
  }

  for (const entry of readdirSync(directory)) {
    const filePath = path.join(directory, entry);
    const relativePath = path.relative(root, filePath);

    if ([...ignoredDirectories].some((ignored) => relativePath === ignored || relativePath.startsWith(`${ignored}${path.sep}`))) {
      continue;
    }

    const stats = statSync(filePath);
    if (stats.isDirectory()) {
      yield* walk(filePath);
    } else if (stats.isFile()) {
      yield filePath;
    }
  }
}

function readJson(relativePath) {
  return JSON.parse(readText(relativePath));
}

function readText(relativePath) {
  return readFileSync(path.join(root, relativePath), "utf8");
}

function assertScriptIncludes(packageJson, scriptName, expectedText) {
  const script = packageJson.scripts?.[scriptName];
  if (!script?.includes(expectedText)) {
    failures.push(`package.json script "${scriptName}" must include "${expectedText}"`);
  }
}

function fail(message) {
  console.error(message);
  process.exit(2);
}
