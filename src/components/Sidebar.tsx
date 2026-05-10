import { BookOpenText, FolderSync, Home, Network, Settings, WandSparkles } from "lucide-react";

const navItems = [
  { label: "보관함", icon: Home, active: true },
  { label: "가져오기", icon: FolderSync },
  { label: "프리셋", icon: BookOpenText },
  { label: "검토", icon: WandSparkles },
  { label: "그래프", icon: Network },
  { label: "설정", icon: Settings }
];

export function Sidebar() {
  return (
    <aside className="sidebar" aria-label="기본 탐색">
      <div className="brand">
        <span>ctx</span>
      </div>
      <nav>
        {navItems.map((item) => {
          const Icon = item.icon;

          return (
            <button
              type="button"
              key={item.label}
              className={item.active ? "active" : undefined}
              aria-current={item.active ? "page" : undefined}
            >
              <Icon aria-hidden="true" />
              {item.label}
            </button>
          );
        })}
      </nav>
    </aside>
  );
}
