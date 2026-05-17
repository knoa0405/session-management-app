import { invoke } from "@tauri-apps/api/core";

export type VaultScope = "global" | "local";

export type CoreContextFragment = {
  context_id: string;
  title: string;
  content: string;
  file_path: string;
  vault_scope: VaultScope;
  tags: string[];
  folder_path: string;
  session_handoff_classification?: SessionClassificationMetadata | null;
};

export type SessionClassificationStatus = "pending" | "classified" | "reviewed" | "modified";

export type SessionClassificationMetadata = {
  sourceTool: string;
  sourceSessionRef: string;
  sourceWorkingDirectory: string;
  sourceLogPath: string;
  workContextCategory: string;
  workContextCategories: string[];
  workContextClassificationStatus: SessionClassificationStatus;
  workContextConfidenceScore: number;
  workContextRationale: string;
  distillationFocus: string[];
};

export type SessionHandoffContext = {
  source_tool: "codex" | "claude" | string;
  source_session_ref: string;
  source_working_directory: string;
  source_log_path: string;
  source_updated_at: string | null;
  title: string;
  category: string;
  categories: string[];
  classification_status: SessionClassificationStatus;
  classification_confidence_score: number;
  classification_rationale: string;
  goals: string[];
  summary: string;
  key_changed_files: string[];
  commands: string[];
  decisions: string[];
  verification_results: string[];
  remaining_work: string[];
  created_at: string;
  handoff_markdown: string;
  tags: string[];
  cleanup_applied: boolean;
  refine_mode: "raw" | "refined" | string;
  launch_target: "codex" | "claude" | string;
  injection_method: "append-system-prompt-file" | "agents-md-section-marker-merge" | string;
};

export type SavedSessionHandoffContext = {
  fragment: CoreContextFragment;
  handoff: SessionHandoffContext;
};

export type DesktopInvoker = <T>(command: string, args?: Record<string, unknown>) => Promise<T>;

export async function listSavedSessionHandoffContexts(
  invokeDesktop: DesktopInvoker,
  workingDir?: string
) {
  const request = workingDir ? { workingDir } : null;

  return invokeDesktop<SavedSessionHandoffContext[]>("list_saved_session_contexts", {
    request
  });
}

export async function openSavedSessionHandoffContext(
  invokeDesktop: DesktopInvoker,
  filePath: string,
  workingDir?: string
) {
  return invokeDesktop<SavedSessionHandoffContext>("open_saved_session_context", {
    request: {
      filePath,
      workingDir
    }
  });
}

export async function saveAgentSessionHandoffContext(
  invokeDesktop: DesktopInvoker,
  request: {
    provider: string;
    filePath: string;
    content: string;
    workingDir?: string;
  }
) {
  return invokeDesktop<CoreContextFragment>("save_agent_session_context", {
    request
  });
}

export async function listSavedSessionHandoffContextsFromPersistentStorage(
  workingDir?: string
) {
  return listSavedSessionHandoffContexts(invoke, workingDir);
}
