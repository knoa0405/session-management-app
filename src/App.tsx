import {
  AlertTriangle,
  CheckCircle2,
  Clipboard,
  Clock3,
  Database,
  FileDown,
  LoaderCircle,
  MessageSquareText,
  Rocket,
  RefreshCw,
  Search,
  Sparkles
} from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import {
  listSavedSessionHandoffContexts,
  openSavedSessionHandoffContext,
  saveAgentSessionHandoffContext,
  type CoreContextFragment,
  type SavedSessionHandoffContext,
  type SessionClassificationMetadata,
  type SessionClassificationStatus,
  type SessionHandoffContext
} from "./data/sessionHandoffContexts";

type AgentSessionSummary = {
  provider: "codex" | "claude" | string;
  sessionId: string;
  title: string;
  updatedAt: string | null;
  cwd: string | null;
  filePath: string;
  messageCount: number;
  lastUserMessage: string | null;
  classificationMetadata?: SessionClassificationMetadata | null;
};

type AgentSessionMessage = {
  role: string;
  timestamp: string | null;
  content: string;
};

type AgentSessionDetail = {
  summary: AgentSessionSummary;
  messages: AgentSessionMessage[];
  distilledMarkdown: string;
  classificationMetadata: SessionClassificationMetadata;
};

type WorkContextCategory =
  | "implementation"
  | "debugging"
  | "review"
  | "planning"
  | "refactor"
  | "research"
  | "verification"
  | "launch"
  | "general";

type LoadState =
  | { state: "idle" }
  | { state: "loading" }
  | { state: "success"; message: string }
  | { state: "error"; message: string };

type SessionDetailState =
  | { state: "idle" }
  | { state: "loading"; session: AgentSessionSummary }
  | { state: "ready"; detail: AgentSessionDetail }
  | { state: "error"; session: AgentSessionSummary; message: string };

type ActionState =
  | { state: "idle" }
  | { state: "running"; message: string }
  | { state: "success"; message: string }
  | { state: "error"; message: string };

type ResolvedLaunchContextPayload = {
  fragment: CoreContextFragment;
  handoff: SessionHandoffContext;
  handoffMarkdown: string;
  contextFilePath: string;
  launchTarget: string;
  injectionMethod: string;
};

type LaunchFlowState =
  | {
      state: "empty";
      selectedHandoffId: null;
      resolvedContextPayload: null;
    }
  | {
      state: "ready";
      selectedHandoffId: string;
      resolvedContextPayload: ResolvedLaunchContextPayload;
    };

declare global {
  interface Window {
    __TAURI_INTERNALS__?: {
      invoke?: unknown;
    };
  }
}

function isDesktopBridgeAvailable() {
  return (
    typeof window !== "undefined" &&
    typeof window.__TAURI_INTERNALS__?.invoke === "function"
  );
}

async function invokeDesktop<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  if (!isDesktopBridgeAvailable()) {
    throw new Error("로컬 세션 로그는 데스크탑 앱에서만 읽을 수 있습니다. `npm run dev:desktop`으로 실행하세요.");
  }

  return invoke<T>(command, args);
}

function formatError(error: unknown) {
  if (error instanceof Error) {
    return error.message;
  }

  if (typeof error === "string") {
    return error;
  }

  return "요청을 처리하지 못했습니다.";
}

function isSessionContext(context: CoreContextFragment) {
  const path = context.file_path.toLowerCase();
  const folder = context.folder_path.toLowerCase();
  const tags = context.tags.map((tag) => tag.toLowerCase());

  return (
    folder.includes("session-history") ||
    path.includes("/session-history/") ||
    tags.includes("session-history") ||
    tags.includes("resume-context")
  );
}

function formatProvider(provider: string) {
  if (provider === "codex") {
    return "Codex";
  }

  if (provider === "claude") {
    return "Claude";
  }

  return provider;
}

function formatWorkContextCategory(category: string) {
  const labels: Record<string, string> = {
    implementation: "구현",
    debugging: "디버깅",
    review: "리뷰",
    planning: "계획",
    refactor: "리팩터",
    research: "리서치",
    verification: "검증",
    launch: "실행",
    general: "일반"
  };

  return labels[category] ?? category;
}

function formatClassificationStatus(status: SessionClassificationStatus) {
  const labels: Record<SessionClassificationStatus, string> = {
    pending: "대기",
    classified: "분류됨",
    reviewed: "검토됨",
    modified: "수정됨"
  };

  return labels[status];
}

function formatBoolean(value: boolean) {
  return value ? "true" : "false";
}

function formatNullable(value: string | null | undefined) {
  return value && value.trim() ? value : "없음";
}

function formatListValue(values: string[]) {
  return values.length > 0 ? values.join(", ") : "없음";
}

function formatInjectionMethod(method: string) {
  const labels: Record<string, string> = {
    "append-system-prompt-file": "Claude temporary prompt file",
    "agents-md-section-marker-merge": "Codex AGENTS.md managed block"
  };

  return labels[method] ?? method;
}

function formatLaunchSelectLabel(context: SavedSessionHandoffContext) {
  return `Launch ${formatProvider(context.handoff.launch_target)} 선택`;
}

function buildResolvedLaunchContextPayload(
  context: SavedSessionHandoffContext
): ResolvedLaunchContextPayload {
  return {
    fragment: context.fragment,
    handoff: context.handoff,
    handoffMarkdown: context.handoff.handoff_markdown,
    contextFilePath: context.fragment.file_path,
    launchTarget: context.handoff.launch_target,
    injectionMethod: context.handoff.injection_method
  };
}

const WORK_CONTEXT_CATEGORIES: WorkContextCategory[] = [
  "implementation",
  "debugging",
  "review",
  "planning",
  "refactor",
  "research",
  "verification",
  "launch",
  "general"
];

const CLASSIFICATION_STATUSES: SessionClassificationStatus[] = [
  "classified",
  "reviewed",
  "modified",
  "pending"
];

function sessionHasCategory(session: AgentSessionSummary, category: WorkContextCategory) {
  const metadata = session.classificationMetadata;
  if (!metadata) {
    return false;
  }

  return (
    metadata.workContextCategory === category ||
    metadata.workContextCategories.includes(category)
  );
}

export function App() {
  const [sessions, setSessions] = useState<AgentSessionSummary[]>([]);
  const [savedContexts, setSavedContexts] = useState<SavedSessionHandoffContext[]>([]);
  const [selectedSavedContext, setSelectedSavedContext] = useState<SavedSessionHandoffContext | null>(null);
  const [activeSessionId, setActiveSessionId] = useState("");
  const [searchTerm, setSearchTerm] = useState("");
  const [providerFilter, setProviderFilter] = useState<"all" | "codex" | "claude">("all");
  const [categoryFilter, setCategoryFilter] = useState<"all" | WorkContextCategory>("all");
  const [statusFilter, setStatusFilter] = useState<"all" | SessionClassificationStatus>("all");
  const [sessionLoadState, setSessionLoadState] = useState<LoadState>({ state: "idle" });
  const [contextLoadState, setContextLoadState] = useState<LoadState>({ state: "idle" });
  const [detailState, setDetailState] = useState<SessionDetailState>({ state: "idle" });
  const [draftContent, setDraftContent] = useState("");
  const [saveState, setSaveState] = useState<ActionState>({ state: "idle" });
  const [copyState, setCopyState] = useState<ActionState>({ state: "idle" });
  const [selectionState, setSelectionState] = useState<ActionState>({ state: "idle" });
  const [launchFlowState, setLaunchFlowState] = useState<LaunchFlowState>({
    state: "empty",
    selectedHandoffId: null,
    resolvedContextPayload: null
  });
  const [refineState, setRefineState] = useState<ActionState>({ state: "idle" });
  const [refinerTarget, setRefinerTarget] = useState<"claude" | "codex">("claude");

  useEffect(() => {
    void refreshSessions();
    void refreshSavedContexts();
  }, []);

  useEffect(() => {
    if (!selectedSavedContext) {
      setSelectionState({ state: "idle" });
      setLaunchFlowState({
        state: "empty",
        selectedHandoffId: null,
        resolvedContextPayload: null
      });
      return;
    }

    setLaunchFlowState({
      state: "ready",
      selectedHandoffId: selectedSavedContext.fragment.context_id,
      resolvedContextPayload: buildResolvedLaunchContextPayload(selectedSavedContext)
    });
    setSelectionState({
      state: "success",
      message: `${selectedSavedContext.handoff.title}이(가) 새 세션 실행 대상으로 선택되었습니다.`
    });
  }, [selectedSavedContext]);

  const filteredSessions = useMemo(() => {
    const normalized = searchTerm.trim().toLowerCase();
    return sessions.filter((session) => {
      if (providerFilter !== "all" && session.provider !== providerFilter) {
        return false;
      }

      if (categoryFilter !== "all" && !sessionHasCategory(session, categoryFilter)) {
        return false;
      }

      if (
        statusFilter !== "all" &&
        session.classificationMetadata?.workContextClassificationStatus !== statusFilter
      ) {
        return false;
      }

      if (!normalized) {
        return true;
      }

      return [
        session.provider,
        session.title,
        session.sessionId,
        session.cwd ?? "",
        session.filePath,
        session.lastUserMessage ?? "",
        session.classificationMetadata?.workContextCategory ?? "",
        ...(session.classificationMetadata?.workContextCategories ?? [])
      ]
        .join(" ")
        .toLowerCase()
        .includes(normalized);
    });
  }, [categoryFilter, providerFilter, searchTerm, sessions, statusFilter]);

  const activeSession =
    filteredSessions.find((session) => session.sessionId === activeSessionId) ?? filteredSessions[0];

  useEffect(() => {
    if (!activeSession) {
      setDetailState({ state: "idle" });
      setDraftContent("");
      return;
    }

    let isMounted = true;
    setDetailState({ state: "loading", session: activeSession });
    setSaveState({ state: "idle" });
    setCopyState({ state: "idle" });
    setRefineState({ state: "idle" });

    invokeDesktop<AgentSessionDetail>("read_agent_session", {
      request: {
        provider: activeSession.provider,
        filePath: activeSession.filePath
      }
    })
      .then((detail) => {
        if (!isMounted) {
          return;
        }

        setDetailState({ state: "ready", detail });
        setDraftContent(detail.distilledMarkdown);
      })
      .catch((error: unknown) => {
        if (!isMounted) {
          return;
        }

        setDetailState({
          state: "error",
          session: activeSession,
          message: formatError(error)
        });
        setDraftContent("");
      });

    return () => {
      isMounted = false;
    };
  }, [activeSession?.sessionId, activeSession?.filePath]);

  async function refreshSessions() {
    setSessionLoadState({ state: "loading" });

    try {
      const nextSessions = await invokeDesktop<AgentSessionSummary[]>("list_agent_sessions");
      setSessions(nextSessions);
      setActiveSessionId((currentId) =>
        nextSessions.some((session) => session.sessionId === currentId)
          ? currentId
          : nextSessions[0]?.sessionId ?? ""
      );
      setSessionLoadState({
        state: "success",
        message: `${nextSessions.length}개의 이전 작업 세션을 찾았습니다.`
      });
    } catch (error: unknown) {
      setSessions([]);
      setActiveSessionId("");
      setSessionLoadState({ state: "error", message: formatError(error) });
    }
  }

  async function refreshSavedContexts() {
    setContextLoadState({ state: "loading" });

    try {
      const sessionContexts = await listSavedSessionHandoffContexts(invokeDesktop);
      setSavedContexts(sessionContexts);
      setSelectedSavedContext((current) => {
        if (!current) {
          return sessionContexts[0] ?? null;
        }

        return (
          sessionContexts.find(
            (context) => context.fragment.context_id === current.fragment.context_id
          ) ?? sessionContexts[0] ?? null
        );
      });
      setContextLoadState({
        state: "success",
        message: `${sessionContexts.length}개의 저장된 세션 컨텍스트가 있습니다.`
      });
    } catch (error: unknown) {
      setSavedContexts([]);
      setSelectedSavedContext(null);
      setContextLoadState({ state: "error", message: formatError(error) });
    }
  }

  async function copyDraft() {
    if (!draftContent.trim()) {
      return;
    }

    setCopyState({ state: "running", message: "복사 중" });

    try {
      await navigator.clipboard.writeText(draftContent);
      setCopyState({ state: "success", message: "새 세션 시작 프롬프트로 복사했습니다." });
    } catch (error: unknown) {
      setCopyState({ state: "error", message: formatError(error) });
    }
  }

  async function refineDraft() {
    if (detailState.state !== "ready" || !draftContent.trim()) {
      return;
    }

    setRefineState({ state: "running", message: "AI가 핵심 맥락을 정리하는 중" });
    setSaveState({ state: "idle" });

    try {
      const refined = await invokeDesktop<string>("refine_session_context", {
        request: {
          draftContent,
          targetCli: refinerTarget
        }
      });
      setDraftContent(refined);
      await persistSessionDraft(refined);
      setRefineState({
        state: "success",
        message: "목표, 변경사항, 검증, 남은 작업 중심으로 정리하고 저장했습니다."
      });
    } catch (error: unknown) {
      setSaveState({ state: "error", message: `저장하지 않았습니다: ${formatError(error)}` });
      setRefineState({ state: "error", message: formatError(error) });
    }
  }

  async function copySavedContext(context: SavedSessionHandoffContext) {
    setCopyState({ state: "running", message: "복사 중" });

    try {
      await navigator.clipboard.writeText(context.handoff.handoff_markdown);
      setCopyState({ state: "success", message: "저장된 세션 컨텍스트를 복사했습니다." });
    } catch (error: unknown) {
      setCopyState({ state: "error", message: formatError(error) });
    }
  }

  function selectSavedContextForDownstreamUse(context: SavedSessionHandoffContext) {
    setSelectedSavedContext(context);
    setSelectionState({
      state: "success",
      message: `${context.handoff.title}이(가) 새 세션 실행 대상으로 선택되었습니다.`
    });
  }

  async function openSavedContext(context: SavedSessionHandoffContext) {
    selectSavedContextForDownstreamUse(context);
    setDetailState({ state: "idle" });
    setSaveState({ state: "idle" });

    try {
      const detail = await openSavedSessionHandoffContext(invokeDesktop, context.fragment.file_path);
      selectSavedContextForDownstreamUse(detail);
      setDraftContent(detail.handoff.handoff_markdown);
      setCopyState({
        state: "success",
        message: `${detail.handoff.title}을(를) 편집 초안으로 열었습니다.`
      });
    } catch (error: unknown) {
      setDraftContent(context.handoff.handoff_markdown);
      setCopyState({
        state: "error",
        message: formatError(error)
      });
    }
  }

  async function saveDraftContext() {
    if (detailState.state !== "ready" || !draftContent.trim()) {
      return;
    }

    setSaveState({ state: "running", message: "저장 중" });

    try {
      await persistSessionDraft(draftContent);
    } catch (error: unknown) {
      setSaveState({ state: "error", message: `저장하지 않았습니다: ${formatError(error)}` });
    }
  }

  async function persistSessionDraft(content: string) {
    if (detailState.state !== "ready") {
      throw new Error("저장할 이전 세션이 선택되지 않았습니다.");
    }

    const summary = detailState.detail.summary;
    setSaveState({ state: "running", message: "검증 후 저장 중" });

    await saveAgentSessionHandoffContext(invokeDesktop, {
      provider: summary.provider,
      filePath: summary.filePath,
      content
    });
    setSaveState({
      state: "success",
      message: "검증된 세션 컨텍스트를 저장했습니다."
    });
    await refreshSavedContexts();
  }

  const stats = {
    sessionCount: sessions.length,
    savedContextCount: savedContexts.length,
    codexCount: sessions.filter((session) => session.provider === "codex").length,
    claudeCount: sessions.filter((session) => session.provider === "claude").length
  };

  return (
    <main className="session-app">
      <header className="session-hero">
        <div>
          <p className="eyebrow">Session Context Reuse</p>
          <h1>이전 작업을 새 세션에 바로 이어붙이기</h1>
          <p>
            끝난 Codex/Claude 작업 세션을 고르면 새 세션에 붙여넣기 좋은 맥락 초안을 만듭니다.
            필요 없는 부분만 덜어내고 복사하거나 저장하세요.
          </p>
        </div>
        <button type="button" onClick={() => void refreshSessions()}>
          <RefreshCw aria-hidden="true" />
          세션 새로고침
        </button>
      </header>

      <section className="workflow-guide" aria-label="사용 흐름">
        <article>
          <strong>1. 이전 세션 선택</strong>
          <p>프로젝트명, 요청 내용, 작업 폴더로 찾습니다.</p>
        </article>
        <article>
          <strong>2. 초안 확인</strong>
          <p>목표, 결정, 변경 내용이 들어간 새 세션용 문맥을 다듬습니다.</p>
        </article>
        <article>
          <strong>3. 새 세션에 사용</strong>
          <p>복사해서 붙여넣거나 프로젝트에 저장해 다시 꺼냅니다.</p>
        </article>
      </section>

      <section className="summary-grid session-summary-grid" aria-label="세션 요약">
        <Metric icon={<MessageSquareText aria-hidden="true" />} label="발견한 세션" value={stats.sessionCount} />
        <Metric icon={<Database aria-hidden="true" />} label="저장한 컨텍스트" value={stats.savedContextCount} />
        <Metric icon={<Clock3 aria-hidden="true" />} label="Codex 세션" value={stats.codexCount} />
        <Metric icon={<Clock3 aria-hidden="true" />} label="Claude 세션" value={stats.claudeCount} />
      </section>

      <section className="session-primary-grid">
        <section className="panel session-picker-panel" aria-labelledby="session-list-heading">
          <div className="panel-heading">
            <div>
              <p className="eyebrow">1. 찾기</p>
              <h2 id="session-list-heading">이전 작업 세션</h2>
            </div>
          </div>
          <label className="search-field session-search-field">
            <Search aria-hidden="true" />
            <span className="sr-only">세션 검색</span>
            <input
              value={searchTerm}
              placeholder="프로젝트, 요청, 세션 ID 검색"
              onChange={(event) => setSearchTerm(event.target.value)}
            />
          </label>
          <div className="filter-pills" aria-label="세션 종류 필터">
            {(["all", "codex", "claude"] as const).map((provider) => (
              <button
                type="button"
                className={providerFilter === provider ? "filter-pill active" : "filter-pill"}
                key={provider}
                onClick={() => setProviderFilter(provider)}
              >
                {provider === "all" ? "전체" : formatProvider(provider)}
              </button>
            ))}
          </div>
          <div className="classification-filter-grid" aria-label="세션 분류 필터">
            <label>
              <span>작업 유형</span>
              <select
                value={categoryFilter}
                onChange={(event) => setCategoryFilter(event.target.value as "all" | WorkContextCategory)}
              >
                <option value="all">전체</option>
                {WORK_CONTEXT_CATEGORIES.map((category) => (
                  <option value={category} key={category}>
                    {formatWorkContextCategory(category)}
                  </option>
                ))}
              </select>
            </label>
            <label>
              <span>분류 상태</span>
              <select
                value={statusFilter}
                onChange={(event) => setStatusFilter(event.target.value as "all" | SessionClassificationStatus)}
              >
                <option value="all">전체</option>
                {CLASSIFICATION_STATUSES.map((status) => (
                  <option value={status} key={status}>
                    {formatClassificationStatus(status)}
                  </option>
                ))}
              </select>
            </label>
          </div>
          <StatusLine state={sessionLoadState} />
          {activeSession ? <ActiveSessionCard session={activeSession} /> : null}
          <div className="session-list">
            {filteredSessions.map((session) => (
              <button
                type="button"
                className={session.sessionId === activeSession?.sessionId ? "session-row active" : "session-row"}
                key={`${session.provider}:${session.filePath}`}
                onClick={() => setActiveSessionId(session.sessionId)}
              >
                <MessageSquareText aria-hidden="true" />
                <span>
                  <strong>{session.title}</strong>
                  <small>{session.cwd ?? session.filePath}</small>
                  {session.lastUserMessage ? <small>{session.lastUserMessage}</small> : null}
                  <SessionRowClassification metadata={session.classificationMetadata ?? null} />
                </span>
                <em>{formatProvider(session.provider)}</em>
              </button>
            ))}
            {filteredSessions.length === 0 && sessionLoadState.state !== "loading" ? (
              <p className="empty-state">조건에 맞는 이전 세션이 없습니다.</p>
            ) : null}
          </div>
        </section>

        <section className="panel session-draft-panel" aria-labelledby="session-draft-heading">
          <div className="panel-heading">
            <div>
              <p className="eyebrow">2. 다듬기</p>
              <h2 id="session-draft-heading">새 세션 시작 컨텍스트</h2>
            </div>
          </div>
          <SessionDraftHeader detailState={detailState} />
          <div className="draft-stage-card">
            <div>
              <strong>1단계: 빠른 추출</strong>
              <p>세션 로그에서 사용자/어시스턴트 메시지를 로컬에서 즉시 가져옵니다.</p>
            </div>
            <div>
              <strong>2단계: AI 핵심 정리</strong>
              <p>원하면 로컬 Claude/Codex CLI로 목표, 결정, 변경 파일, 검증, 남은 작업만 압축합니다.</p>
            </div>
          </div>
          <textarea
            className="markdown-editor session-context-draft"
            aria-label="새 세션에 넣을 컨텍스트 초안"
            value={draftContent}
            disabled={detailState.state === "loading"}
            placeholder="이전 세션을 선택하면 새 작업 세션에 넣을 요약 초안이 여기에 생성됩니다."
            onChange={(event) => setDraftContent(event.target.value)}
            spellCheck="false"
          />
          <div className="session-action-bar">
            <ActionFeedback state={copyState} />
            <ActionFeedback state={saveState} />
            <ActionFeedback state={refineState} />
            <div className="session-action-buttons">
              <label className="refiner-select">
                <span>정리 엔진</span>
                <select
                  value={refinerTarget}
                  disabled={refineState.state === "running"}
                  onChange={(event) => setRefinerTarget(event.target.value as "claude" | "codex")}
                >
                  <option value="claude">Claude</option>
                  <option value="codex">Codex</option>
                </select>
              </label>
              <button
                type="button"
                className="secondary-button"
                disabled={
                  detailState.state !== "ready" ||
                  !draftContent.trim() ||
                  refineState.state === "running"
                }
                onClick={() => void refineDraft()}
              >
                <Sparkles aria-hidden="true" />
                AI로 핵심 정리
              </button>
              <button
                type="button"
                className="secondary-button"
                disabled={!draftContent.trim() || copyState.state === "running" || refineState.state === "running"}
                onClick={() => void copyDraft()}
              >
                <Clipboard aria-hidden="true" />
                새 세션에 붙여넣기
              </button>
              <button
                type="button"
                disabled={
                  detailState.state !== "ready" ||
                  !draftContent.trim() ||
                  saveState.state === "running" ||
                  refineState.state === "running"
                }
                onClick={() => void saveDraftContext()}
              >
                <FileDown aria-hidden="true" />
                나중에 쓰도록 저장
              </button>
            </div>
          </div>
        </section>
      </section>

      <section className="panel saved-session-panel" aria-labelledby="saved-contexts-heading">
        <div className="panel-heading">
          <div>
            <p className="eyebrow">3. 재사용</p>
            <h2 id="saved-contexts-heading">저장된 세션 컨텍스트</h2>
          </div>
          <button type="button" className="secondary-button" onClick={() => void refreshSavedContexts()}>
            <RefreshCw aria-hidden="true" />
            다시 읽기
          </button>
        </div>
        <StatusLine state={contextLoadState} />
        <ActionFeedback state={selectionState} />
        <div
          className="saved-context-list"
          role="listbox"
          aria-label="저장된 세션 컨텍스트 선택 목록"
          aria-activedescendant={
            selectedSavedContext ? `saved-context-${selectedSavedContext.fragment.context_id}` : undefined
          }
        >
          {savedContexts.map((context) => (
            <article
              aria-selected={selectedSavedContext?.fragment.context_id === context.fragment.context_id}
              className={
                selectedSavedContext?.fragment.context_id === context.fragment.context_id
                  ? "saved-context-row active"
                  : "saved-context-row"
              }
              id={`saved-context-${context.fragment.context_id}`}
              key={context.fragment.context_id}
              role="option"
              tabIndex={0}
              onClick={() => selectSavedContextForDownstreamUse(context)}
              onKeyDown={(event) => {
                if (event.target !== event.currentTarget) {
                  return;
                }

                if (event.key === "Enter" || event.key === " ") {
                  event.preventDefault();
                  selectSavedContextForDownstreamUse(context);
                }
              }}
            >
              <div className="saved-context-summary">
                <strong>{context.handoff.title}</strong>
                <small>{context.fragment.file_path}</small>
                <div className="saved-context-launch-summary" aria-label="실행 주입 방식">
                  <span>{formatProvider(context.handoff.launch_target)}</span>
                  <small>{formatInjectionMethod(context.handoff.injection_method)}</small>
                </div>
                <SavedHandoffMetadataGrid handoff={context.handoff} compact={true} />
                <SavedHandoffDistilledGrid handoff={context.handoff} compact={true} />
              </div>
              <div className="saved-context-actions">
                <em>{context.fragment.vault_scope}</em>
                <button
                  type="button"
                  className="launch-select-button"
                  aria-pressed={selectedSavedContext?.fragment.context_id === context.fragment.context_id}
                  aria-label={`${context.handoff.title}을(를) ${formatProvider(
                    context.handoff.launch_target
                  )} 새 세션 실행 대상으로 선택`}
                  onClick={(event) => {
                    event.stopPropagation();
                    selectSavedContextForDownstreamUse(context);
                  }}
                >
                  {selectedSavedContext?.fragment.context_id === context.fragment.context_id ? (
                    <CheckCircle2 aria-hidden="true" />
                  ) : (
                    <Rocket aria-hidden="true" />
                  )}
                  {selectedSavedContext?.fragment.context_id === context.fragment.context_id
                    ? "실행 대상으로 선택됨"
                    : formatLaunchSelectLabel(context)}
                </button>
                <button
                  type="button"
                  className="secondary-button"
                  onClick={(event) => {
                    event.stopPropagation();
                    void openSavedContext(context);
                  }}
                >
                  초안으로 열기
                </button>
                <button
                  type="button"
                  onClick={(event) => {
                    event.stopPropagation();
                    void copySavedContext(context);
                  }}
                >
                  <Clipboard aria-hidden="true" />
                  복사
                </button>
              </div>
            </article>
          ))}
          {savedContexts.length === 0 && contextLoadState.state !== "loading" ? (
            <p className="empty-state">
              아직 저장된 세션 컨텍스트가 없습니다. 위 초안을 저장하면 여기에 쌓입니다.
            </p>
          ) : null}
        </div>
        {selectedSavedContext ? (
          <>
            <SelectedSavedContextForUse
              context={selectedSavedContext}
              launchFlowState={launchFlowState}
            />
            <SavedSessionDetail context={selectedSavedContext} />
          </>
        ) : null}
      </section>
    </main>
  );
}

function SelectedSavedContextForUse({
  context,
  launchFlowState
}: {
  context: SavedSessionHandoffContext;
  launchFlowState: LaunchFlowState;
}) {
  const handoff = context.handoff;
  const payload =
    launchFlowState.state === "ready" &&
    launchFlowState.selectedHandoffId === context.fragment.context_id
      ? launchFlowState.resolvedContextPayload
      : null;

  return (
    <article className="selected-saved-context" aria-label="다운스트림 사용 대상으로 선택된 세션 컨텍스트">
      <div>
        <p className="eyebrow">Selected for downstream use</p>
        <h3>{handoff.title}</h3>
        <small>{context.fragment.file_path}</small>
      </div>
      <dl>
        <div>
          <dt>Selected handoff ID</dt>
          <dd>{launchFlowState.selectedHandoffId ?? "없음"}</dd>
        </div>
        <div>
          <dt>Launch target</dt>
          <dd>{formatProvider(payload?.launchTarget ?? handoff.launch_target)}</dd>
        </div>
        <div>
          <dt>Injection method</dt>
          <dd>{formatInjectionMethod(payload?.injectionMethod ?? handoff.injection_method)}</dd>
        </div>
        <div>
          <dt>Source session</dt>
          <dd>{payload?.handoff.source_session_ref ?? handoff.source_session_ref}</dd>
        </div>
        <div>
          <dt>Payload file</dt>
          <dd>{payload?.contextFilePath ?? context.fragment.file_path}</dd>
        </div>
        <div>
          <dt>Handoff markdown</dt>
          <dd>{(payload?.handoffMarkdown ?? handoff.handoff_markdown).trim() ? "준비됨" : "비어 있음"}</dd>
        </div>
      </dl>
    </article>
  );
}

function Metric({
  icon,
  label,
  value
}: {
  icon: React.ReactNode;
  label: string;
  value: number;
}) {
  return (
    <article className="metric">
      {icon}
      <div>
        <span>{value}</span>
        <p>{label}</p>
      </div>
    </article>
  );
}

function ActiveSessionCard({ session }: { session: AgentSessionSummary }) {
  return (
    <article className="active-session-card" aria-label="현재 선택된 세션">
      <div>
        <p className="eyebrow">선택됨</p>
        <strong>{session.title}</strong>
        <small>{session.cwd ?? session.filePath}</small>
      </div>
      <dl>
        <div>
          <dt>종류</dt>
          <dd>{formatProvider(session.provider)}</dd>
        </div>
        <div>
          <dt>메시지</dt>
          <dd>{session.messageCount}개</dd>
        </div>
      </dl>
      {session.classificationMetadata ? (
        <SessionClassificationSummary metadata={session.classificationMetadata} compact={true} />
      ) : null}
      {session.lastUserMessage ? <p>{session.lastUserMessage}</p> : null}
    </article>
  );
}

function SessionRowClassification({
  metadata
}: {
  metadata: SessionClassificationMetadata | null;
}) {
  if (!metadata) {
    return <small className="session-row-classification pending">분류 정보 없음</small>;
  }

  return (
    <small className="session-row-classification">
      {formatWorkContextCategory(metadata.workContextCategory)} ·{" "}
      {formatClassificationStatus(metadata.workContextClassificationStatus)} ·{" "}
      {metadata.workContextConfidenceScore}%
    </small>
  );
}

function StatusLine({ state }: { state: LoadState }) {
  if (state.state === "idle") {
    return null;
  }

  if (state.state === "loading") {
    return (
      <p className="content-status">
        <LoaderCircle aria-hidden="true" className="spin-icon" />
        읽는 중
      </p>
    );
  }

  if (state.state === "error") {
    return (
      <p className="content-status error">
        <AlertTriangle aria-hidden="true" />
        {state.message}
      </p>
    );
  }

  return (
    <p className="content-status success">
      <CheckCircle2 aria-hidden="true" />
      {state.message}
    </p>
  );
}

function SessionDraftHeader({ detailState }: { detailState: SessionDetailState }) {
  if (detailState.state === "idle") {
    return <p className="empty-state">왼쪽에서 이전 세션을 선택하세요.</p>;
  }

  if (detailState.state === "loading") {
    return (
      <p className="content-status">
        <LoaderCircle aria-hidden="true" className="spin-icon" />
        {formatProvider(detailState.session.provider)} 세션을 읽고 있습니다.
      </p>
    );
  }

  if (detailState.state === "error") {
    return (
      <p className="content-status error">
        <AlertTriangle aria-hidden="true" />
        {detailState.message}
      </p>
    );
  }

  return (
    <div className="session-detail-block">
      <div className="session-detail-meta">
        <span>
          <MessageSquareText aria-hidden="true" />
          {formatProvider(detailState.detail.summary.provider)}
        </span>
        <span>
          <Clock3 aria-hidden="true" />
          {detailState.detail.summary.messageCount}개 메시지
        </span>
        {detailState.detail.summary.cwd ? <span>{detailState.detail.summary.cwd}</span> : null}
      </div>
      <SessionClassificationSummary metadata={detailState.detail.classificationMetadata} />
    </div>
  );
}

function SessionClassificationSummary({
  metadata,
  compact = false
}: {
  metadata: SessionClassificationMetadata;
  compact?: boolean;
}) {
  const categories =
    metadata.workContextCategories.length > 0
      ? metadata.workContextCategories
      : [metadata.workContextCategory];

  return (
    <div className={compact ? "session-classification compact" : "session-classification"}>
      <div className="session-classification-chips" aria-label="세션 분류 메타데이터">
        <span>{formatWorkContextCategory(metadata.workContextCategory)}</span>
        <span>{formatClassificationStatus(metadata.workContextClassificationStatus)}</span>
        <span>{metadata.workContextConfidenceScore}%</span>
        <span>{formatProvider(metadata.sourceTool)}</span>
      </div>
      {!compact ? (
        <>
          <p>{metadata.workContextRationale}</p>
          <small>
            범주: {categories.map(formatWorkContextCategory).join(", ")}
            {metadata.distillationFocus.length > 0
              ? ` · 정리 초점: ${metadata.distillationFocus.join(", ")}`
              : ""}
          </small>
        </>
      ) : (
        <small>
          {formatWorkContextCategory(metadata.workContextCategory)} ·{" "}
          {formatClassificationStatus(metadata.workContextClassificationStatus)} ·{" "}
          {metadata.workContextConfidenceScore}%
        </small>
      )}
    </div>
  );
}

function SavedHandoffMetadataGrid({
  handoff,
  compact = false
}: {
  handoff: SessionHandoffContext;
  compact?: boolean;
}) {
  const metadataRows = [
    ["Source tool", formatProvider(handoff.source_tool)],
    ["Source session", handoff.source_session_ref],
    ["Working directory", formatNullable(handoff.source_working_directory)],
    ["Source log", formatNullable(handoff.source_log_path)],
    ["Source updated", formatNullable(handoff.source_updated_at)],
    ["Created", formatNullable(handoff.created_at)],
    ["Category", formatWorkContextCategory(handoff.category)],
    ["Categories", formatListValue(handoff.categories.map(formatWorkContextCategory))],
    ["Status", formatClassificationStatus(handoff.classification_status)],
    ["Confidence", `${handoff.classification_confidence_score}%`],
    ["Launch target", formatProvider(handoff.launch_target)],
    ["Injection", formatInjectionMethod(handoff.injection_method)],
    ["Cleanup applied", formatBoolean(handoff.cleanup_applied)],
    ["Refine mode", handoff.refine_mode],
    ["Tags", formatListValue(handoff.tags)]
  ];

  return (
    <dl className={compact ? "saved-handoff-grid compact" : "saved-handoff-grid"}>
      {metadataRows.map(([label, value]) => (
        <div key={label}>
          <dt>{label}</dt>
          <dd>{value}</dd>
        </div>
      ))}
    </dl>
  );
}

function SavedHandoffDistilledGrid({
  handoff,
  compact = false
}: {
  handoff: SessionHandoffContext;
  compact?: boolean;
}) {
  const rows = [
    ["Summary", formatNullable(handoff.summary)],
    ["Goals", formatListValue(handoff.goals)],
    ["Key changed files", formatListValue(handoff.key_changed_files)],
    ["Commands", formatListValue(handoff.commands)],
    ["Decisions", formatListValue(handoff.decisions)],
    ["Verification results", formatListValue(handoff.verification_results)],
    ["Remaining work", formatListValue(handoff.remaining_work)]
  ];

  return (
    <dl className={compact ? "saved-handoff-distilled compact" : "saved-handoff-distilled"}>
      {rows.map(([label, value]) => (
        <div key={label}>
          <dt>{label}</dt>
          <dd>{value}</dd>
        </div>
      ))}
    </dl>
  );
}

function SavedHandoffFieldList({
  title,
  values
}: {
  title: string;
  values: string[];
}) {
  return (
    <section className="saved-handoff-field">
      <h4>{title}</h4>
      {values.length > 0 ? (
        <ul>
          {values.map((value, index) => (
            <li key={`${title}:${index}`}>{value}</li>
          ))}
        </ul>
      ) : (
        <p>없음</p>
      )}
    </section>
  );
}

function SavedSessionDetail({ context }: { context: SavedSessionHandoffContext }) {
  const handoff = context.handoff;

  return (
    <article className="saved-session-detail" aria-label="저장된 세션 컨텍스트 상세">
      <div className="saved-session-detail-heading">
        <div>
          <p className="eyebrow">Saved handoff detail</p>
          <h3>{handoff.title}</h3>
        </div>
        <em>{context.fragment.vault_scope}</em>
      </div>
      <SavedHandoffMetadataGrid handoff={handoff} />
      <SavedHandoffDistilledGrid handoff={handoff} />
      <section className="saved-handoff-rationale">
        <h4>Classification rationale</h4>
        <p>{formatNullable(handoff.classification_rationale)}</p>
      </section>
      <section className="saved-handoff-rationale">
        <h4>Summary</h4>
        <p>{formatNullable(handoff.summary)}</p>
      </section>
      <div className="saved-handoff-fields">
        <SavedHandoffFieldList title="Goals" values={handoff.goals} />
        <SavedHandoffFieldList title="Key changed files" values={handoff.key_changed_files} />
        <SavedHandoffFieldList title="Commands" values={handoff.commands} />
        <SavedHandoffFieldList title="Decisions" values={handoff.decisions} />
        <SavedHandoffFieldList title="Verification results" values={handoff.verification_results} />
        <SavedHandoffFieldList title="Remaining work" values={handoff.remaining_work} />
      </div>
      <section className="saved-handoff-markdown">
        <h4>Handoff markdown</h4>
        <pre>{handoff.handoff_markdown || "없음"}</pre>
      </section>
    </article>
  );
}

function ActionFeedback({ state }: { state: ActionState }) {
  if (state.state === "idle") {
    return null;
  }

  if (state.state === "running") {
    return (
      <p className="save-state saving" role="status">
        <LoaderCircle aria-hidden="true" />
        {state.message}
      </p>
    );
  }

  if (state.state === "error") {
    return (
      <p className="save-state error" role="alert">
        <AlertTriangle aria-hidden="true" />
        {state.message}
      </p>
    );
  }

  return (
    <p className="save-state saved" role="status">
      <CheckCircle2 aria-hidden="true" />
      {state.message}
    </p>
  );
}
