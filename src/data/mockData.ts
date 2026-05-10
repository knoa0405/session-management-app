export type VaultScope = "global" | "local";
export type ContextClassification = "main-agent" | "subagent" | "shared";
export type ImportSourceType =
  | "context-markdown"
  | "claude-markdown"
  | "codex-agents"
  | "agent-markdown"
  | "agents-manifest"
  | "skill-markdown"
  | "skill-manifest"
  | "subagent-markdown";

export type ContextFragment = {
  id: string;
  title: string;
  path: string;
  scope: VaultScope;
  importSource: string | null;
  importSourceType: ImportSourceType | null;
  classification: ContextClassification;
  importClassificationSuggestion: ContextClassification | null;
  inferredClassification: ContextClassification | null;
  tags: string[];
  folder: string;
  wikilinks: string[];
  reviewStatus: "pending" | "classified" | "reviewed" | "modified";
  content: string;
  excerpt: string;
};

export type PresetContextSelectionKind = "whole-file" | "heading" | "line-range" | "anchor";

export type PresetContextSelection = {
  selectionKind: PresetContextSelectionKind;
  heading: string | null;
  anchor: string | null;
  lineStart: number | null;
  lineEnd: number | null;
  includeChildren: boolean;
};

export type PresetContextComposition = {
  contextId: string;
  order: number;
  sourceRef: string;
  required: boolean;
  selection: PresetContextSelection;
};

export type SubagentManifestEntry = {
  id: string;
  name: string;
  role: string;
  capabilities: string[];
  constraints: string[];
  metadata: Record<string, string>;
  description: string | null;
  assignedContexts: string[];
  spawnInstructions: string[];
  spawnGuidance: {
    selectWhen: string[];
    avoidWhen: string[];
    delegationPrompt: string | null;
  };
  handoffTargets: string[];
  model: string | null;
};

export type HandoffConstraints = {
  requireSummary: boolean;
  requireChangedFiles: boolean;
  requireOpenQuestions: boolean;
  maxParallelSubagents: number | null;
  allowedHandoffTargets: string[];
  blockedHandoffTargets: string[];
  handoffPromptTemplate: string | null;
};

export type SubagentManifest = {
  manifestVersion: string | null;
  roles: SubagentManifestEntry[];
  handoffConstraints: HandoffConstraints;
};

export type Preset = {
  id: string;
  presetRef: string;
  name: string;
  description: string | null;
  tags: string[];
  folder: string;
  scope: VaultScope;
  filePath: string;
  targetCli: "claude" | "codex";
  workingDir: string;
  model: string | null;
  passthroughArgs: string[];
  injectionStrategy: "append-system-prompt-file" | "agents-md-section-marker-merge";
  cleanupOnExit: boolean;
  cleanupStaleOnLaunch: boolean;
  contextCount: number;
  contextComposition: PresetContextComposition[];
  subagentManifest: SubagentManifest | null;
};

export const contexts: ContextFragment[] = [
  {
    id: "ctx-main-agent",
    title: "메인 에이전트 운영 노트",
    path: ".ctx/agent.md",
    scope: "local",
    importSource: null,
    importSourceType: null,
    classification: "main-agent",
    importClassificationSuggestion: "main-agent",
    inferredClassification: "main-agent",
    tags: ["워크플로", "프로젝트"],
    folder: "agents",
    wikilinks: ["공유 Rust 패턴", "Codex 실행 프리셋"],
    reviewStatus: "reviewed",
    content:
      "주요 코딩 에이전트를 위한 프로젝트 로컬 운영 지침입니다. wrapper cleanup 기대 동작과 보관함 overlay 규칙을 포함합니다.",
    excerpt:
      "주요 코딩 에이전트를 위한 프로젝트 로컬 운영 지침입니다. wrapper cleanup 기대 동작과 보관함 overlay 규칙을 포함합니다."
  },
  {
    id: "ctx-rust-patterns",
    title: "공유 Rust 패턴",
    path: "~/.ctx/skills/rust.md",
    scope: "global",
    importSource: null,
    importSourceType: null,
    classification: "shared",
    importClassificationSuggestion: "shared",
    inferredClassification: "shared",
    tags: ["rust", "tauri"],
    folder: "skills",
    wikilinks: ["메인 에이전트 운영 노트"],
    reviewStatus: "classified",
    content:
      "Tauri backend와 번들 ctx CLI가 공유하는 Rust core module용 재사용 구현 메모입니다.",
    excerpt:
      "Tauri backend와 번들 ctx CLI가 공유하는 Rust core module용 재사용 구현 메모입니다."
  },
  {
    id: "ctx-codex-preset",
    title: "Codex 실행 프리셋",
    path: ".ctx/presets/codex.md",
    scope: "local",
    importSource: null,
    importSourceType: null,
    classification: "subagent",
    importClassificationSuggestion: "subagent",
    inferredClassification: "subagent",
    tags: ["codex", "실행"],
    folder: "presets",
    wikilinks: ["공유 Rust 패턴"],
    reviewStatus: "pending",
    content:
      "AGENTS.md marker injection과 프로세스 종료 후 자동 cleanup을 사용하는 Codex session용 프리셋 구성입니다.",
    excerpt:
      "AGENTS.md marker injection과 프로세스 종료 후 자동 cleanup을 사용하는 Codex session용 프리셋 구성입니다."
  },
  {
    id: "ctx-subagent-reviewer",
    title: "리뷰어 서브에이전트",
    path: ".ctx/vault/subagents/reviewer.md",
    scope: "local",
    importSource: null,
    importSourceType: null,
    classification: "subagent",
    importClassificationSuggestion: "subagent",
    inferredClassification: "subagent",
    tags: ["리뷰", "품질"],
    folder: "subagents",
    wikilinks: ["Codex 실행 프리셋"],
    reviewStatus: "reviewed",
    content:
      "Codex session을 계속하기 전에 정확성 리스크, 누락된 테스트, 위험한 handoff를 찾는 위임 리뷰 지침입니다.",
    excerpt:
      "Codex session을 계속하기 전에 정확성 리스크, 누락된 테스트, 위험한 handoff를 찾는 위임 리뷰 지침입니다."
  },
  {
    id: "ctx-subagent-implementer",
    title: "구현 서브에이전트",
    path: ".ctx/vault/subagents/implementer.md",
    scope: "local",
    importSource: null,
    importSourceType: null,
    classification: "subagent",
    importClassificationSuggestion: "subagent",
    inferredClassification: "subagent",
    tags: ["구현", "테스트"],
    folder: "subagents",
    wikilinks: ["공유 Rust 패턴"],
    reviewStatus: "reviewed",
    content:
      "현재 프로젝트 안에서 범위가 정해진 코드 변경, 집중 테스트, 통합 수정을 수행하는 위임 구현 지침입니다.",
    excerpt:
      "현재 프로젝트 안에서 범위가 정해진 코드 변경, 집중 테스트, 통합 수정을 수행하는 위임 구현 지침입니다."
  },
  {
    id: "ctx-subagent-researcher",
    title: "리서처 서브에이전트",
    path: ".ctx/vault/subagents/researcher.md",
    scope: "local",
    importSource: null,
    importSourceType: null,
    classification: "subagent",
    importClassificationSuggestion: "subagent",
    inferredClassification: "subagent",
    tags: ["리서치", "저장소"],
    folder: "subagents",
    wikilinks: ["메인 에이전트 운영 노트"],
    reviewStatus: "classified",
    content:
      "저장소 탐색, 의존성 확인, 구현을 막는 지점을 해소하는 간결한 답변을 위한 위임 리서치 지침입니다.",
    excerpt:
      "저장소 탐색, 의존성 확인, 구현을 막는 지점을 해소하는 간결한 답변을 위한 위임 리서치 지침입니다."
  }
];

export const presets: Preset[] = [
  {
    id: "preset-claude-default",
    presetRef: "preset-claude-default",
    name: "Claude 프로젝트 세션",
    description: "공유 조각과 로컬 조각을 조합한 프로젝트용 Claude 컨텍스트입니다.",
    tags: ["claude", "프로젝트"],
    folder: "presets",
    scope: "local",
    filePath: ".ctx/vault/presets/preset-claude-default.json",
    targetCli: "claude",
    workingDir: "/workspace",
    model: "claude-sonnet",
    passthroughArgs: [],
    injectionStrategy: "append-system-prompt-file",
    cleanupOnExit: true,
    cleanupStaleOnLaunch: true,
    contextCount: 2,
    subagentManifest: null,
    contextComposition: [
      {
        contextId: "ctx-main-agent",
        order: 0,
        sourceRef: "agents/agent.md",
        required: true,
        selection: {
          selectionKind: "whole-file",
          heading: null,
          anchor: null,
          lineStart: null,
          lineEnd: null,
          includeChildren: false
        }
      }
    ]
  },
  {
    id: "preset-codex-default",
    presetRef: "preset-codex-default",
    name: "Codex 구현 세션",
    description: "marker 기반 AGENTS.md injection을 사용하는 구현 작업용 Codex 프리셋입니다.",
    tags: ["codex", "구현"],
    folder: "presets",
    scope: "local",
    filePath: ".ctx/vault/presets/preset-codex-default.json",
    targetCli: "codex",
    workingDir: "/workspace",
    model: "codex",
    passthroughArgs: ["--sandbox", "workspace-write"],
    injectionStrategy: "agents-md-section-marker-merge",
    cleanupOnExit: true,
    cleanupStaleOnLaunch: true,
    contextCount: 6,
    subagentManifest: {
      manifestVersion: "1",
      roles: [
        {
          id: "reviewer",
          name: "리뷰어",
          role: "코드 리뷰 서브에이전트",
          capabilities: ["정확성 검토", "리스크 식별", "테스트 공백 탐지"],
          constraints: [
            "파일과 줄 번호를 포함해 발견 사항을 반환합니다.",
            "넓은 스타일 피드백보다 구체적인 동작 리스크를 우선합니다."
          ],
          metadata: { owner: "quality", phase: "phase-1" },
          description: "handoff 전에 정확성 리스크를 찾습니다.",
          assignedContexts: ["subagents/reviewer.md"],
          spawnInstructions: [
            "제어가 런처로 돌아가기 전에 변경 파일과 영향받는 테스트를 검사합니다.",
            "발견 사항을 먼저 보고하고, 그다음 열린 질문과 남은 리스크를 정리합니다."
          ],
          spawnGuidance: {
            selectWhen: [
              "구현 변경이 독립적인 정확성 검토를 받을 만큼 완료되었을 때 사용합니다.",
              "메인 에이전트가 리스크, 회귀, 테스트 공백 탐지를 필요로 할 때 사용합니다."
            ],
            avoidWhen: [
              "아직 넓은 코드 수정이나 구현 소유권이 필요한 작업에는 피합니다.",
              "검토할 패치 없이 저장소 사실 확인만 필요한 요청에는 피합니다."
            ],
            delegationPrompt:
              "변경 파일과 영향받는 테스트를 검토하고, 파일 참조와 함께 발견 사항을 먼저 반환하세요."
          },
          handoffTargets: ["implementer"],
          model: "gpt-5.3-codex"
        },
        {
          id: "implementer",
          name: "구현 담당",
          role: "범위 지정 구현 서브에이전트",
          capabilities: ["집중 코드 변경", "테스트 업데이트", "통합 수정"],
          constraints: [
            "수정 범위를 할당된 모듈이나 파일 집합 안에 유지합니다.",
            "관련 없는 사용자 변경을 되돌리지 않습니다."
          ],
          metadata: { owner: "engineering", phase: "phase-1" },
          description: "범위가 정해진 코드 변경과 대상 테스트 수정을 수행합니다.",
          assignedContexts: ["subagents/implementer.md", "skills/rust.md"],
          spawnInstructions: [
            "위임받은 변경만 구현합니다.",
            "handoff에 변경 파일과 검증 결과를 나열합니다."
          ],
          spawnGuidance: {
            selectWhen: [
              "파일 또는 모듈 소유권이 명확한 제한된 코드 변경에 사용합니다.",
              "구현이 메인 경로와 독립적으로 진행될 수 있을 때 사용합니다."
            ],
            avoidWhen: [
              "요구사항이 모호하거나 제품 결정이 더 필요할 때는 피합니다.",
              "읽기 전용 검토나 저장소 탐색 작업에는 피합니다."
            ],
            delegationPrompt:
              "범위가 정해진 변경을 구현하고, 관련 없는 수정은 보존하며, 변경 파일과 검증 결과를 보고하세요."
          },
          handoffTargets: ["reviewer"],
          model: "gpt-5.3-codex"
        },
        {
          id: "researcher",
          name: "리서처",
          role: "저장소 리서치 서브에이전트",
          capabilities: ["저장소 탐색", "의존성 검사", "소스 기반 답변"],
          constraints: [
            "검사한 프로젝트 파일이나 공식 1차 출처를 바탕으로 답합니다.",
            "출력은 간결하게 유지하고 관련 파일 경로를 인용합니다."
          ],
          metadata: { owner: "context", phase: "phase-1" },
          description: "구현 전에 소스 기반 사실을 수집합니다.",
          assignedContexts: ["subagents/researcher.md"],
          spawnInstructions: [
            "관련 파일을 검사하고 직접 참조와 함께 답변을 요약합니다.",
            "추측하지 말고 모르는 부분을 명시합니다."
          ],
          spawnGuidance: {
            selectWhen: [
              "메인 에이전트가 구현 전에 소스 기반 저장소 사실을 필요로 할 때 사용합니다.",
              "의존성, 관례, 소유권 경계를 병렬 조사할 때 사용합니다."
            ],
            avoidWhen: [
              "다음 작업이 긴급한 차단 구현 단계일 때는 피합니다.",
              "답변이 발견 사항 보고보다 파일 변경을 필요로 할 때는 피합니다."
            ],
            delegationPrompt:
              "관련 파일을 검사하고 경로, 미확인 사항, 권장 다음 단계를 포함해 간결한 발견 사항을 반환하세요."
          },
          handoffTargets: ["implementer", "reviewer"],
          model: "gpt-5.3-codex"
        }
      ],
      handoffConstraints: {
        requireSummary: true,
        requireChangedFiles: true,
        requireOpenQuestions: true,
        maxParallelSubagents: 3,
        allowedHandoffTargets: ["implementer", "reviewer", "researcher"],
        blockedHandoffTargets: [],
        handoffPromptTemplate:
          "요약, 변경 또는 검사한 파일, 열린 질문, 남은 리스크를 반환하세요."
      }
    },
    contextComposition: [
      {
        contextId: "ctx-codex-preset",
        order: 0,
        sourceRef: "presets/codex.md",
        required: true,
        selection: {
          selectionKind: "heading",
          heading: "Implementation workflow",
          anchor: null,
          lineStart: null,
          lineEnd: null,
          includeChildren: true
        }
      }
    ]
  }
];

export const stats = {
  contextCount: contexts.length,
  wikilinkCount: contexts.reduce((total, context) => total + context.wikilinks.length, 0),
  pendingReviews: contexts.filter((context) => context.reviewStatus === "pending").length,
  localOverrides: contexts.filter((context) => context.scope === "local").length
};
