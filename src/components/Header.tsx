import { Play, Plus, RefreshCcw } from "lucide-react";

type HeaderProps = {
  onNewContext: () => void;
  onRescanContexts: () => void;
  isRescanning: boolean;
};

export function Header({ isRescanning, onNewContext, onRescanContexts }: HeaderProps) {
  return (
    <header className="app-header">
      <div>
        <p className="eyebrow">ctx desktop</p>
        <h1>컨텍스트 보관함</h1>
      </div>
      <div className="header-actions">
        <button
          type="button"
          className="icon-button"
          aria-label="보관함 다시 스캔"
          disabled={isRescanning}
          onClick={onRescanContexts}
        >
          <RefreshCcw aria-hidden="true" />
        </button>
        <button type="button" className="secondary-button" onClick={onNewContext}>
          <Plus aria-hidden="true" />
          새 컨텍스트
        </button>
        <button type="button">
          <Play aria-hidden="true" />
          프리셋 실행
        </button>
      </div>
    </header>
  );
}
