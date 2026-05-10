import { FileText, FolderTree } from "lucide-react";

import type { ContextClassification, ContextFragment } from "../data/mockData";

type ContextListProps = {
  contexts: ContextFragment[];
  activeContextId: string;
  isLoading: boolean;
  emptyMessage?: string;
  onSelectContext: (contextId: string) => void;
};

function formatClassification(classification: ContextClassification) {
  const labels: Record<ContextClassification, string> = {
    "main-agent": "메인 에이전트",
    subagent: "서브에이전트",
    shared: "공유"
  };

  return labels[classification];
}

function formatImportSourceType(sourceType: ContextFragment["importSourceType"]) {
  if (!sourceType) {
    return "직접 생성";
  }

  const labels: Record<NonNullable<ContextFragment["importSourceType"]>, string> = {
    "context-markdown": "일반 마크다운",
    "claude-markdown": "Claude 세션",
    "codex-agents": "Codex AGENTS",
    "agent-markdown": "에이전트 파일",
    "agents-manifest": "에이전트 매니페스트",
    "skill-markdown": "스킬 마크다운",
    "skill-manifest": "스킬 매니페스트",
    "subagent-markdown": "서브에이전트"
  };

  return labels[sourceType];
}

type ContextGroup = {
  id: string;
  title: string;
  description: string;
  contexts: ContextFragment[];
};

type SkillContextGroup = {
  id: string;
  title: string;
  sourceRoot: string;
  contexts: ContextFragment[];
};

function isSkillContext(context: ContextFragment) {
  return (
    context.importSourceType === "skill-markdown" ||
    context.importSourceType === "skill-manifest" ||
    context.tags.some((tag) => tag.toLowerCase() === "skills") ||
    normalizePath(context.importSource ?? context.path).toLowerCase().includes("/skills/") ||
    normalizePath(context.importSource ?? context.path).toLowerCase().endsWith("/skill.md")
  );
}

function isSessionContext(context: ContextFragment) {
  return (
    context.classification === "main-agent" ||
    context.importSourceType === "claude-markdown" ||
    context.importSourceType === "codex-agents" ||
    context.importSourceType === "agent-markdown" ||
    context.importSourceType === "agents-manifest"
  );
}

function buildContextGroups(contexts: ContextFragment[]): ContextGroup[] {
  const sessionContexts: ContextFragment[] = [];
  const subagentContexts: ContextFragment[] = [];
  const sharedContexts: ContextFragment[] = [];

  for (const context of contexts) {
    if (isSkillContext(context)) {
      continue;
    }

    if (isSessionContext(context)) {
      sessionContexts.push(context);
      continue;
    }

    if (context.classification === "subagent" || context.importSourceType === "subagent-markdown") {
      subagentContexts.push(context);
      continue;
    }

    sharedContexts.push(context);
  }

  return [
    {
      id: "session",
      title: "세션/메인 에이전트",
      description: "CLAUDE.md, AGENTS.md, agent.md처럼 세션 시작 시 직접 주입되는 파일",
      contexts: sessionContexts
    },
    {
      id: "subagents",
      title: "서브에이전트",
      description: "역할별 위임 에이전트와 관련된 컨텍스트",
      contexts: subagentContexts
    },
    {
      id: "shared",
      title: "기타 공유 컨텍스트",
      description: "스킬이나 세션 파일이 아닌 재사용 마크다운",
      contexts: sharedContexts
    }
  ].filter((group) => group.contexts.length > 0);
}

function buildSkillContextGroups(contexts: ContextFragment[]): SkillContextGroup[] {
  const groups = new Map<string, SkillContextGroup>();

  for (const context of contexts.filter(isSkillContext)) {
    const hierarchy = skillHierarchyForContext(context);
    const id = hierarchy.parts.join("/");
    const existingGroup = groups.get(id);

    if (existingGroup) {
      existingGroup.contexts.push(context);
      continue;
    }

    groups.set(id, {
      id,
      title: hierarchy.parts.join(" / "),
      sourceRoot: hierarchy.sourceRoot,
      contexts: [context]
    });
  }

  return Array.from(groups.values()).sort((left, right) => left.title.localeCompare(right.title));
}

function skillHierarchyForContext(context: ContextFragment) {
  const sourcePath = normalizePath(context.importSource ?? context.path);
  const segments = sourcePath.split("/").filter(Boolean);
  const lowerSegments = segments.map((segment) => segment.toLowerCase());
  const skillFileIndex = findLastIndex(lowerSegments, (segment) => segment === "skill.md");
  const skillsDirectoryIndex = findLastIndex(lowerSegments, (segment) => segment === "skills");

  if (skillsDirectoryIndex >= 0 && skillFileIndex > skillsDirectoryIndex) {
    const parts = segments.slice(skillsDirectoryIndex + 1, skillFileIndex);
    return {
      parts: parts.length > 0 ? parts : [segments[skillFileIndex - 1] ?? "SKILL.md"],
      sourceRoot: segments.slice(0, skillsDirectoryIndex + 1).join("/")
    };
  }

  if (skillFileIndex > 0) {
    return {
      parts: [segments[skillFileIndex - 1]],
      sourceRoot: segments.slice(0, skillFileIndex).join("/")
    };
  }

  if (skillsDirectoryIndex >= 0) {
    const parts = segments.slice(skillsDirectoryIndex + 1, -1);
    return {
      parts: parts.length > 0 ? parts : ["skills"],
      sourceRoot: segments.slice(0, skillsDirectoryIndex + 1).join("/")
    };
  }

  return {
    parts: [context.folder || "스킬"],
    sourceRoot: sourcePath
  };
}

function findLastIndex<T>(items: T[], predicate: (item: T) => boolean) {
  for (let index = items.length - 1; index >= 0; index -= 1) {
    if (predicate(items[index])) {
      return index;
    }
  }

  return -1;
}

function normalizePath(path: string) {
  return path.replace(/\\/g, "/");
}

export function ContextList({
  contexts,
  activeContextId,
  isLoading,
  emptyMessage = "발견된 마크다운 컨텍스트가 없습니다.",
  onSelectContext
}: ContextListProps) {
  if (isLoading) {
    return <p className="empty-state">보관함과 마크다운 컨텍스트를 스캔하는 중...</p>;
  }

  if (contexts.length === 0) {
    return <p className="empty-state">{emptyMessage}</p>;
  }

  const skillGroups = buildSkillContextGroups(contexts);
  const contextGroups = buildContextGroups(contexts);

  return (
    <div className="context-list">
      {contextGroups.map((group) => (
        <section className="context-group" key={group.id}>
          <ContextGroupHeader
            title={group.title}
            description={group.description}
            count={group.contexts.length}
          />
          <div className="context-group-body">
            {group.contexts.map((context) => (
              <ContextRow
                context={context}
                isActive={context.id === activeContextId}
                key={context.id}
                onSelectContext={onSelectContext}
              />
            ))}
          </div>
        </section>
      ))}

      {skillGroups.length > 0 ? (
        <section className="context-group skill-context-group">
          <ContextGroupHeader
            title="스킬 컨텍스트"
            description="SKILL.md와 skills/ 아래 마크다운을 스킬 패키지 경로별로 묶었습니다."
            count={skillGroups.reduce((total, group) => total + group.contexts.length, 0)}
          />
          <div className="skill-tree">
            {skillGroups.map((group) => (
              <section className="skill-tree-group" key={group.id}>
                <div className="skill-tree-heading">
                  <FolderTree aria-hidden="true" />
                  <span>
                    <strong>{group.title}</strong>
                    <small>{group.sourceRoot}</small>
                  </span>
                  <em>{group.contexts.length}개</em>
                </div>
                <div className="context-group-body skill-tree-children">
                  {group.contexts.map((context) => (
                    <ContextRow
                      context={context}
                      isActive={context.id === activeContextId}
                      key={context.id}
                      onSelectContext={onSelectContext}
                    />
                  ))}
                </div>
              </section>
            ))}
          </div>
        </section>
      ) : null}
    </div>
  );
}

function ContextGroupHeader({
  title,
  description,
  count
}: {
  title: string;
  description: string;
  count: number;
}) {
  return (
    <div className="context-group-header">
      <span>
        <strong>{title}</strong>
        <small>{description}</small>
      </span>
      <em>{count}개</em>
    </div>
  );
}

function ContextRow({
  context,
  isActive,
  onSelectContext
}: {
  context: ContextFragment;
  isActive: boolean;
  onSelectContext: (contextId: string) => void;
}) {
  return (
    <button
      type="button"
      className={isActive ? "context-row active" : "context-row"}
      onClick={() => onSelectContext(context.id)}
    >
      <FileText aria-hidden="true" />
      <span>
        <strong>{context.title}</strong>
        <small>{context.path}</small>
        {context.tags.length > 0 ? (
          <small className="context-row-tags">{context.tags.join(", ")}</small>
        ) : null}
      </span>
      <span className="context-row-badges">
        <em>{context.importSource ? formatImportSourceType(context.importSourceType) : context.scope}</em>
        <small className="classification-chip">
          추천:{" "}
          {formatClassification(
            context.importClassificationSuggestion ??
              context.inferredClassification ??
              context.classification
          )}
        </small>
      </span>
    </button>
  );
}
