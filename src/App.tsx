import {
  AlertTriangle,
  CheckCircle2,
  Clipboard,
  Clock3,
  Database,
  FileDown,
  LoaderCircle,
  MessageSquareText,
  RefreshCw,
  Search
} from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

type VaultScope = "global" | "local";

type CoreContextFragment = {
  context_id: string;
  title: string;
  content: string;
  file_path: string;
  vault_scope: VaultScope;
  tags: string[];
  folder_path: string;
};

type AgentSessionSummary = {
  provider: "codex" | "claude" | string;
  sessionId: string;
  title: string;
  updatedAt: string | null;
  cwd: string | null;
  filePath: string;
  messageCount: number;
  lastUserMessage: string | null;
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
};

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

function sanitizeFileName(value: string) {
  return value.replace(/[^a-zA-Z0-9._-]+/g, "-").replace(/^-+|-+$/g, "").slice(0, 120);
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

export function App() {
  const [sessions, setSessions] = useState<AgentSessionSummary[]>([]);
  const [savedContexts, setSavedContexts] = useState<CoreContextFragment[]>([]);
  const [activeSessionId, setActiveSessionId] = useState("");
  const [searchTerm, setSearchTerm] = useState("");
  const [sessionLoadState, setSessionLoadState] = useState<LoadState>({ state: "idle" });
  const [contextLoadState, setContextLoadState] = useState<LoadState>({ state: "idle" });
  const [detailState, setDetailState] = useState<SessionDetailState>({ state: "idle" });
  const [draftContent, setDraftContent] = useState("");
  const [saveState, setSaveState] = useState<ActionState>({ state: "idle" });
  const [copyState, setCopyState] = useState<ActionState>({ state: "idle" });

  useEffect(() => {
    void refreshSessions();
    void refreshSavedContexts();
  }, []);

  const filteredSessions = useMemo(() => {
    const normalized = searchTerm.trim().toLowerCase();
    if (!normalized) {
      return sessions;
    }

    return sessions.filter((session) =>
      [
        session.provider,
        session.title,
        session.sessionId,
        session.cwd ?? "",
        session.filePath,
        session.lastUserMessage ?? ""
      ]
        .join(" ")
        .toLowerCase()
        .includes(normalized)
    );
  }, [searchTerm, sessions]);

  const activeSession =
    sessions.find((session) => session.sessionId === activeSessionId) ?? sessions[0];

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
      const contexts = await invokeDesktop<CoreContextFragment[]>("discover_markdown_contexts", {
        request: null
      });
      const sessionContexts = contexts.filter(isSessionContext);
      setSavedContexts(sessionContexts);
      setContextLoadState({
        state: "success",
        message: `${sessionContexts.length}개의 저장된 세션 컨텍스트가 있습니다.`
      });
    } catch (error: unknown) {
      setSavedContexts([]);
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

  async function saveDraftContext() {
    if (detailState.state !== "ready" || !draftContent.trim()) {
      return;
    }

    const summary = detailState.detail.summary;
    setSaveState({ state: "running", message: "저장 중" });

    try {
      const fileName = `${sanitizeFileName(summary.provider)}-${sanitizeFileName(
        summary.sessionId
      )}.md`;
      await invokeDesktop<CoreContextFragment>("create_markdown_context", {
        request: {
          fileName,
          folderPath: "session-history",
          vaultScope: "local",
          content: draftContent
        }
      });
      setSaveState({
        state: "success",
        message: "이전 세션을 다음 작업용 컨텍스트로 저장했습니다."
      });
      await refreshSavedContexts();
    } catch (error: unknown) {
      setSaveState({ state: "error", message: formatError(error) });
    }
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
          <h1>끝난 작업 세션을 새 작업의 출발점으로</h1>
          <p>
            Codex/Claude resume 로그에서 필요한 맥락만 뽑아 새 세션에 바로 붙여 넣거나
            프로젝트 로컬 컨텍스트로 저장합니다.
          </p>
        </div>
        <button type="button" onClick={() => void refreshSessions()}>
          <RefreshCw aria-hidden="true" />
          세션 새로고침
        </button>
      </header>

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
          <StatusLine state={sessionLoadState} />
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
            <div className="session-action-buttons">
              <button
                type="button"
                className="secondary-button"
                disabled={!draftContent.trim() || copyState.state === "running"}
                onClick={() => void copyDraft()}
              >
                <Clipboard aria-hidden="true" />
                복사
              </button>
              <button
                type="button"
                disabled={
                  detailState.state !== "ready" ||
                  !draftContent.trim() ||
                  saveState.state === "running"
                }
                onClick={() => void saveDraftContext()}
              >
                <FileDown aria-hidden="true" />
                프로젝트에 저장
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
        <div className="saved-context-list">
          {savedContexts.map((context) => (
            <article className="saved-context-row" key={context.context_id}>
              <div>
                <strong>{context.title}</strong>
                <small>{context.file_path}</small>
              </div>
              <em>{context.vault_scope}</em>
            </article>
          ))}
          {savedContexts.length === 0 && contextLoadState.state !== "loading" ? (
            <p className="empty-state">
              아직 저장된 세션 컨텍스트가 없습니다. 위 초안을 저장하면 여기에 쌓입니다.
            </p>
          ) : null}
        </div>
      </section>
    </main>
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
