import {
  CheckCircle2,
  FileText,
  ListOrdered,
  LoaderCircle,
  Save,
  Settings2,
  Terminal
} from "lucide-react";
import { useEffect, useMemo, useState } from "react";

import type {
  ContextClassification,
  ContextFragment,
  Preset,
  SubagentManifest,
  PresetContextSelectionKind
} from "../data/mockData";

type PresetPanelProps = {
  contexts: ContextFragment[];
  presets: Preset[];
  saveState: PresetExecutionSaveState;
  onSaveExecutionSettings: (preset: Preset, values: PresetExecutionFormValues) => void;
};

type PresetExecutionSaveState =
  | { state: "idle" }
  | { state: "saving"; presetId: string }
  | { state: "success"; presetId: string; message: string }
  | { state: "error"; presetId: string; message: string };

type PresetExecutionFormValues = {
  targetCli: Preset["targetCli"];
  workingDir: string;
  model: string | null;
  passthroughArgs: string[];
};

type PresetExecutionDraft = {
  targetCli: Preset["targetCli"];
  workingDir: string;
  model: string;
  passthroughArgs: string;
};

const selectionKinds: Array<{ value: PresetContextSelectionKind; label: string }> = [
  { value: "whole-file", label: "전체 파일" },
  { value: "heading", label: "제목" },
  { value: "line-range", label: "줄 범위" },
  { value: "anchor", label: "앵커" }
];

function formatClassification(classification: ContextClassification) {
  const labels: Record<ContextClassification, string> = {
    "main-agent": "메인 에이전트",
    subagent: "서브에이전트",
    shared: "공유"
  };

  return labels[classification];
}

export function PresetPanel({
  contexts,
  presets,
  saveState,
  onSaveExecutionSettings
}: PresetPanelProps) {
  const firstContext = contexts[0];
  const [activePresetId, setActivePresetId] = useState(presets[0]?.id ?? "");
  const activePreset = presets.find((preset) => preset.id === activePresetId) ?? presets[0];
  const [executionDraft, setExecutionDraft] = useState<PresetExecutionDraft>(() =>
    presetToDraft(activePreset)
  );
  const isSavingActivePreset =
    saveState.state === "saving" && saveState.presetId === activePreset?.id;
  const isActiveSaveSuccess =
    saveState.state === "success" && saveState.presetId === activePreset?.id;
  const isActiveSaveError = saveState.state === "error" && saveState.presetId === activePreset?.id;
  const hasExecutionChanges = useMemo(() => {
    if (!activePreset) {
      return false;
    }

    const persistedDraft = presetToDraft(activePreset);
    return (
      persistedDraft.targetCli !== executionDraft.targetCli ||
      persistedDraft.workingDir !== executionDraft.workingDir ||
      persistedDraft.model !== executionDraft.model ||
      persistedDraft.passthroughArgs !== executionDraft.passthroughArgs
    );
  }, [activePreset, executionDraft]);

  useEffect(() => {
    if (!activePreset && presets[0]) {
      setActivePresetId(presets[0].id);
      return;
    }

    if (activePreset) {
      setExecutionDraft(presetToDraft(activePreset));
    }
  }, [
    activePreset?.id,
    activePreset?.targetCli,
    activePreset?.workingDir,
    activePreset?.model,
    activePreset?.passthroughArgs.join(" "),
    presets
  ]);

  return (
    <div className="preset-panel">
      <h3>실행 프리셋</h3>
      {presets.length === 0 ? (
        <p className="empty-state">해결된 보관함 오버레이에서 실행 프리셋을 찾지 못했습니다.</p>
      ) : null}
      <div className="preset-list">
        {presets.map((preset) => (
          <button
            type="button"
            key={preset.id}
            className={`preset-row${preset.id === activePreset?.id ? " active" : ""}`}
            onClick={() => setActivePresetId(preset.id)}
          >
            <Terminal aria-hidden="true" />
            <span>
              <strong>{preset.name}</strong>
              <small>
                {preset.targetCli} · {preset.workingDir} · 컨텍스트 {preset.contextCount}개
              </small>
            </span>
            <em>선택 {preset.contextComposition.length}개</em>
          </button>
        ))}
      </div>
      {activePreset ? (
        <form
          className="preset-execution-form"
          aria-label={`${activePreset.name} 실행 설정`}
          onSubmit={(event) => {
            event.preventDefault();
            onSaveExecutionSettings(activePreset, draftToValues(executionDraft));
          }}
        >
          <div className="preset-composer-heading">
            <Settings2 aria-hidden="true" />
            <strong>실행 설정</strong>
          </div>
          <div className="preset-execution-grid">
            <label>
              <span>대상 CLI</span>
              <select
                value={executionDraft.targetCli}
                onChange={(event) =>
                  setExecutionDraft((currentDraft) => ({
                    ...currentDraft,
                    targetCli: event.target.value as Preset["targetCli"]
                  }))
                }
              >
                <option value="claude">Claude</option>
                <option value="codex">Codex</option>
              </select>
            </label>
            <label>
              <span>작업 디렉터리</span>
              <input
                value={executionDraft.workingDir}
                placeholder="/workspace/project"
                onChange={(event) =>
                  setExecutionDraft((currentDraft) => ({
                    ...currentDraft,
                    workingDir: event.target.value
                  }))
                }
              />
            </label>
            <label>
              <span>모델</span>
              <input
                value={executionDraft.model}
                placeholder={executionDraft.targetCli === "claude" ? "claude-sonnet" : "codex"}
                onChange={(event) =>
                  setExecutionDraft((currentDraft) => ({
                    ...currentDraft,
                    model: event.target.value
                  }))
                }
              />
            </label>
            <label>
              <span>추가 인자</span>
              <input
                value={executionDraft.passthroughArgs}
                placeholder="--sandbox workspace-write"
                onChange={(event) =>
                  setExecutionDraft((currentDraft) => ({
                    ...currentDraft,
                    passthroughArgs: event.target.value
                  }))
                }
              />
            </label>
          </div>
          <div className="preset-execution-footer">
            <PresetExecutionFeedback
              hasChanges={hasExecutionChanges}
              isError={isActiveSaveError}
              isSaving={isSavingActivePreset}
              isSuccess={isActiveSaveSuccess}
              message={
                isActiveSaveError || isActiveSaveSuccess
                  ? saveState.message
                  : `${activePreset.injectionStrategy}; cleanup ${
                      activePreset.cleanupOnExit ? "활성" : "비활성"
                    }`
              }
            />
            <button
              type="submit"
              disabled={
                !executionDraft.workingDir.trim() || !hasExecutionChanges || isSavingActivePreset
              }
            >
              {isSavingActivePreset ? (
                <LoaderCircle aria-hidden="true" />
              ) : (
                <Save aria-hidden="true" />
              )}
              설정 저장
            </button>
          </div>
        </form>
      ) : null}
      <div className="preset-composer" aria-label="프리셋 컨텍스트 선택 입력">
        <div className="preset-composer-heading">
          <FileText aria-hidden="true" />
          <strong>컨텍스트 선택</strong>
        </div>
        <div className="preset-selection-grid">
          <label>
            <span>컨텍스트 파일</span>
            <select defaultValue={firstContext?.id ?? ""} disabled={contexts.length === 0}>
              {contexts.length === 0 ? <option value="">색인된 컨텍스트 없음</option> : null}
              {contexts.map((context) => (
                <option key={context.id} value={context.id}>
                  {context.folder}/{context.title} · 추론{" "}
                  {formatClassification(
                    context.importClassificationSuggestion ??
                      context.inferredClassification ??
                      context.classification
                  )}
                </option>
              ))}
            </select>
          </label>
          <label>
            <span>조각</span>
            <select defaultValue="whole-file">
              {selectionKinds.map((kind) => (
                <option key={kind.value} value={kind.value}>
                  {kind.label}
                </option>
              ))}
            </select>
          </label>
          <label>
            <span>제목 또는 앵커</span>
            <input placeholder="선택 조각 대상" />
          </label>
          <label>
            <span>줄</span>
            <input inputMode="numeric" placeholder="시작-끝" />
          </label>
          <label className="checkbox-field">
            <input type="checkbox" defaultChecked />
            <span>필수</span>
          </label>
          <label className="checkbox-field">
            <input type="checkbox" />
            <span>하위 제목 포함</span>
          </label>
        </div>
        <div className="preset-selection-order">
          <ListOrdered aria-hidden="true" />
          <span>순서: 0, 1, 2</span>
        </div>
      </div>
      {activePreset ? <SubagentManifestPreview manifest={activePreset.subagentManifest} /> : null}
    </div>
  );
}

function presetToDraft(preset: Preset | undefined): PresetExecutionDraft {
  return {
    targetCli: preset?.targetCli ?? "codex",
    workingDir: preset?.workingDir ?? "",
    model: preset?.model ?? "",
    passthroughArgs: preset?.passthroughArgs.join(" ") ?? ""
  };
}

function draftToValues(draft: PresetExecutionDraft): PresetExecutionFormValues {
  return {
    targetCli: draft.targetCli,
    workingDir: draft.workingDir.trim(),
    model: draft.model.trim() || null,
    passthroughArgs: draft.passthroughArgs
      .split(/\s+/)
      .map((arg) => arg.trim())
      .filter(Boolean)
  };
}

function PresetExecutionFeedback({
  hasChanges,
  isError,
  isSaving,
  isSuccess,
  message
}: {
  hasChanges: boolean;
  isError: boolean;
  isSaving: boolean;
  isSuccess: boolean;
  message: string;
}) {
  if (isSaving) {
    return (
      <p className="preset-execution-status saving" role="status" aria-live="polite">
        <LoaderCircle aria-hidden="true" />
        실행 설정 저장 중
      </p>
    );
  }

  if (isError) {
    return (
      <p className="preset-execution-status error" role="alert">
        {message}
      </p>
    );
  }

  if (hasChanges) {
    return <p className="preset-execution-status dirty">저장되지 않은 실행 설정 변경</p>;
  }

  return (
    <p className={`preset-execution-status${isSuccess ? " saved" : ""}`} role="status">
      {isSuccess ? <CheckCircle2 aria-hidden="true" /> : null}
      {message}
    </p>
  );
}

function SubagentManifestPreview({ manifest }: { manifest: SubagentManifest | null }) {
  if (!manifest) {
    return (
      <div className="subagent-manifest-preview">
        <div className="preset-composer-heading">
          <FileText aria-hidden="true" />
          <strong>서브에이전트 매니페스트</strong>
        </div>
        <p className="empty-state">이 프리셋에는 위임 서브에이전트가 설정되어 있지 않습니다.</p>
      </div>
    );
  }

  return (
    <div className="subagent-manifest-preview" aria-label="Subagent manifest">
      <div className="preset-composer-heading">
        <FileText aria-hidden="true" />
        <strong>서브에이전트 매니페스트</strong>
      </div>
      <div className="subagent-role-list">
        {manifest.roles.map((role) => (
          <article className="subagent-role-card" key={role.id}>
            <div>
              <strong>{role.name}</strong>
              <span>{role.role}</span>
            </div>
            {role.description ? <p>{role.description}</p> : null}
            <div className="subagent-spawn-guidance">
              <section>
                <span>선택 조건</span>
                <ul>
                  {role.spawnGuidance.selectWhen.map((rule) => (
                    <li key={rule}>{rule}</li>
                  ))}
                </ul>
              </section>
              <section>
                <span>피할 조건</span>
                <ul>
                  {role.spawnGuidance.avoidWhen.map((rule) => (
                    <li key={rule}>{rule}</li>
                  ))}
                </ul>
              </section>
              {role.spawnGuidance.delegationPrompt ? (
                <p>{role.spawnGuidance.delegationPrompt}</p>
              ) : null}
            </div>
            <ul>
              {role.capabilities.map((capability) => (
                <li key={capability}>{capability}</li>
              ))}
            </ul>
          </article>
        ))}
      </div>
      <p className="subagent-handoff-summary">
        최대 병렬: {manifest.handoffConstraints.maxParallelSubagents ?? "제한 없음"} · 허용된
        핸드오프:{" "}
        {manifest.handoffConstraints.allowedHandoffTargets.length > 0
          ? manifest.handoffConstraints.allowedHandoffTargets.join(", ")
          : "모두"}
      </p>
    </div>
  );
}
