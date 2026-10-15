import { useEffect } from "react";
import { X } from "lucide-react";

/** ⌘/ 단축키 표. 앱 안에 목록이 없던 문제(UI 검토 #12)의 해결 — 전부
 *  keycap 으로 표시. 새 단축키를 추가하면 여기도 같이. */
const GROUPS: { title: string; rows: { keys: string[]; label: string }[] }[] = [
  {
    title: "이동",
    rows: [
      { keys: ["⌘", "K"], label: "통합 팔레트 — 문서·프로젝트·명령 (`>` 명령 · `@` 프로젝트 · `#` daily)" },
      { keys: ["⌘", "P"], label: "팔레트를 @ 프로젝트 모드로" },
      { keys: ["⌘", "J"], label: "오늘(홈)으로" },
      { keys: ["⌘", "⇧", "R"], label: "검토함" },
      { keys: ["⌘", "⇧", "U"], label: "사용량" },
      { keys: ["⌘", "G"], label: "그래프" },
      { keys: ["⌘", "["], label: "뒤로 (문서 → 프로젝트 홈 → 홈)" },
      { keys: ["⌘", "1…9"], label: "사이드바 순서대로 프로젝트 1~9" },
      { keys: ["⌘", "\\"], label: "사이드바 접기 / 펼치기" },
    ],
  },
  {
    title: "문서",
    rows: [
      { keys: ["⌘", "N"], label: "현재 프로젝트에 새 노트" },
      { keys: ["⌘", "⇧", "D"], label: "현재(또는 최근) 프로젝트의 오늘 daily 열기" },
      { keys: ["⌘", "S"], label: "저장 (자동 저장이 켜져 있어도 즉시)" },
      { keys: ["⌘", "F"], label: "문서 안에서 찾기" },
      { keys: ["⌘", "I"], label: "인스펙터 (변경·링크·정보) 토글" },
    ],
  },
  {
    title: "목록 (검토함 · 피드, 입력 포커스 없을 때)",
    rows: [
      { keys: ["J", "K"], label: "아래 / 위로 이동" },
      { keys: ["↩"], label: "열기" },
      { keys: ["A"], label: "승인 / 제외" },
      { keys: ["X"], label: "무시" },
    ],
  },
  {
    title: "기타",
    rows: [
      { keys: ["⌘", ","], label: "설정" },
      { keys: ["⌘", "/"], label: "이 표" },
      { keys: ["⌃", "Space"], label: "Quick Capture (전역)" },
    ],
  },
];

export function Keycap({ children }: { children: string }) {
  return (
    <kbd className="inline-flex h-5 min-w-[20px] items-center justify-center rounded-xs border border-hairline bg-surface-elevated px-1.5 font-mono text-[11px] leading-none text-on-dark-mute">
      {children}
    </kbd>
  );
}

export function ShortcutsHelp({
  open,
  onClose,
}: {
  open: boolean;
  onClose: () => void;
}) {
  useEffect(() => {
    if (!open) return;
    function onKey(e: KeyboardEvent) {
      if (e.key === "Escape") onClose();
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [open, onClose]);
  if (!open) return null;
  return (
    <div
      className="fixed inset-0 z-50 flex items-start justify-center bg-canvas/70 pt-20 backdrop-blur-sm"
      onMouseDown={(e) => {
        if (e.target === e.currentTarget) onClose();
      }}
    >
      <div className="flex max-h-[80vh] w-full max-w-[640px] flex-col overflow-hidden rounded-lg border border-hairline-strong bg-surface-card">
        <div className="flex items-center justify-between border-b border-hairline px-4 py-2.5">
          <span className="text-[13px] font-medium text-ink">단축키</span>
          <button
            onClick={onClose}
            className="grid h-6 w-6 place-items-center rounded-sm text-mute hover:bg-surface-elevated hover:text-on-dark"
            title="닫기 (esc)"
          >
            <X size={13} />
          </button>
        </div>
        <div className="grid grid-cols-1 gap-4 overflow-y-auto p-4 sm:grid-cols-2">
          {GROUPS.map((g) => (
            <section key={g.title}>
              <div className="mb-1.5 text-[11px] font-medium uppercase tracking-[0.5px] text-stone">
                {g.title}
              </div>
              <ul className="flex flex-col gap-1">
                {g.rows.map((r) => (
                  <li
                    key={r.label}
                    className="flex items-start justify-between gap-3 text-[12px] text-body"
                  >
                    <span className="min-w-0 flex-1 leading-[1.5]">{r.label}</span>
                    <span className="flex shrink-0 items-center gap-0.5">
                      {r.keys.map((k, i) => (
                        <Keycap key={i}>{k}</Keycap>
                      ))}
                    </span>
                  </li>
                ))}
              </ul>
            </section>
          ))}
        </div>
      </div>
    </div>
  );
}
