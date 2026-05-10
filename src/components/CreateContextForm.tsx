import { FilePlus2, FolderInput, X } from "lucide-react";
import { useId, useMemo, useState } from "react";
import type { FormEvent } from "react";

import type { VaultScope } from "../data/mockData";

export type CreateContextFormValues = {
  fileName: string;
  folderPath: string;
  vaultScope: VaultScope;
  content: string;
};

type ContextTemplate = {
  id: string;
  label: string;
  content: string;
};

type CreateContextFormProps = {
  statusMessage?: string;
  isSubmitting: boolean;
  onCancel: () => void;
  onSubmit: (values: CreateContextFormValues) => Promise<void>;
};

const templates: ContextTemplate[] = [
  {
    id: "blank",
    label: "빈 노트",
    content: ""
  },
  {
    id: "agent",
    label: "에이전트 컨텍스트",
    content: "# 에이전트 컨텍스트\n\n## 목적\n\n## 운영 메모\n\n## 관련 항목\n\n- [[공유 컨텍스트]]\n"
  },
  {
    id: "skill",
    label: "스킬 노트",
    content: "# 스킬\n\n## 사용할 때\n\n## 워크플로\n\n## 제약\n"
  }
];

export function CreateContextForm({
  statusMessage,
  isSubmitting,
  onCancel,
  onSubmit
}: CreateContextFormProps) {
  const formId = useId();
  const [fileName, setFileName] = useState("agent.md");
  const [folderPath, setFolderPath] = useState("agents");
  const [vaultScope, setVaultScope] = useState<VaultScope>("local");
  const [templateId, setTemplateId] = useState(templates[1].id);
  const [content, setContent] = useState(templates[1].content);

  const selectedTemplate = useMemo(
    () => templates.find((template) => template.id === templateId) ?? templates[0],
    [templateId]
  );

  function handleTemplateChange(nextTemplateId: string) {
    const nextTemplate =
      templates.find((template) => template.id === nextTemplateId) ?? templates[0];
    setTemplateId(nextTemplate.id);
    setContent(nextTemplate.content);
  }

  async function handleSubmit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    await onSubmit({
      fileName: fileName.trim(),
      folderPath: folderPath.trim(),
      vaultScope,
      content
    });
  }

  return (
    <section className="panel create-context-panel" aria-labelledby={`${formId}-heading`}>
      <div className="panel-heading">
        <div>
          <p className="eyebrow">생성</p>
          <h2 id={`${formId}-heading`}>새 컨텍스트 파일</h2>
        </div>
        <button
          type="button"
          className="icon-button"
          aria-label="생성 폼 닫기"
          disabled={isSubmitting}
          onClick={onCancel}
        >
          <X aria-hidden="true" />
        </button>
      </div>

      <form className="create-context-form" onSubmit={handleSubmit}>
        <label>
          <span>파일 이름</span>
          <input
            required
            disabled={isSubmitting}
            placeholder="agent.md"
            value={fileName}
            onChange={(event) => setFileName(event.target.value)}
          />
        </label>

        <div className="form-row">
          <label>
            <span>보관함 위치</span>
            <select
              disabled={isSubmitting}
              value={vaultScope}
              onChange={(event) => setVaultScope(event.target.value as VaultScope)}
            >
              <option value="local">프로젝트 로컬 .ctx</option>
              <option value="global">전역 ~/.ctx</option>
            </select>
          </label>

          <label>
            <span>폴더</span>
            <input
              disabled={isSubmitting}
              placeholder="agents"
              value={folderPath}
              onChange={(event) => setFolderPath(event.target.value)}
            />
          </label>
        </div>

        <label>
          <span>템플릿</span>
          <select
            disabled={isSubmitting}
            value={selectedTemplate.id}
            onChange={(event) => handleTemplateChange(event.target.value)}
          >
            {templates.map((template) => (
              <option key={template.id} value={template.id}>
                {template.label}
              </option>
            ))}
          </select>
        </label>

        <label>
          <span>초기 내용</span>
          <textarea
            disabled={isSubmitting}
            rows={9}
            value={content}
            onChange={(event) => setContent(event.target.value)}
            placeholder="# Context title"
          />
        </label>

        {statusMessage ? (
          <p className="form-status" role="alert">
            {statusMessage}
          </p>
        ) : null}

        <div className="form-actions">
          <button
            type="button"
            className="secondary-button"
            disabled={isSubmitting}
            onClick={onCancel}
          >
            취소
          </button>
          <button type="submit" disabled={isSubmitting}>
            <FilePlus2 aria-hidden="true" />
            {isSubmitting ? "생성 중" : "파일 생성"}
          </button>
        </div>
      </form>

      <div className="create-context-location">
        <FolderInput aria-hidden="true" />
        <span>
          {vaultScope === "local" ? ".ctx" : "~/.ctx"}/contexts
          {folderPath.trim() ? `/${folderPath.trim()}` : ""}
        </span>
      </div>
    </section>
  );
}
