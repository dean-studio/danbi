import { useEffect } from "react";
import { X } from "lucide-react";
import appIconUrl from "@/assets/danbi-app-icon.png";
import { useAppVersion } from "@/lib/useAppVersion";

/**
 * Custom "About Danbi" dialog shown when the macOS app menu "About" item
 * is clicked. We render this ourselves so the experience is identical in
 * `tauri dev` (where the native NSAbout panel can't find our bundled
 * icon) and in shipped `.app` builds.
 *
 * Visual language mirrors the macOS native About window: centered app
 * icon, product name, version, short description, copyright line. No
 * extra chrome — one close affordance.
 */
export function AboutDialog({
  open,
  onClose,
}: {
  open: boolean;
  onClose: () => void;
}) {
  const version = useAppVersion();
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
      className="fixed inset-0 z-50 flex items-center justify-center bg-canvas/70 backdrop-blur-sm"
      onMouseDown={(e) => {
        if (e.target === e.currentTarget) onClose();
      }}
    >
      <div className="relative w-[380px] overflow-hidden rounded-xl border border-hairline bg-surface shadow-2xl shadow-black/50">
        <button
          onClick={onClose}
          className="absolute right-3 top-3 grid h-6 w-6 place-items-center rounded-sm text-mute transition-colors hover:bg-surface-elevated hover:text-on-dark"
          title="닫기 (Esc)"
        >
          <X size={12} />
        </button>

        <div className="flex flex-col items-center gap-3 px-6 pb-6 pt-8">
          <img
            src={appIconUrl}
            alt="Danbi"
            draggable={false}
            className="h-24 w-24 select-none rounded-2xl shadow-lg shadow-black/30"
          />
          <div className="text-center">
            <div className="text-[18px] font-medium text-ink">단비 (Danbi)</div>
            <div className="mt-0.5 text-caption-sm text-mute">
              {version ? `Version ${version}` : " "}
            </div>
          </div>

          <p className="mt-2 text-center text-[13px] leading-[1.6] text-body">
            로컬 마크다운 wiki를 AI 에이전트의 공유 뇌로 만드는 macOS 앱.
            <br />
            모호한 메모를 구조화된 프로젝트 지식으로 전환합니다.
          </p>

          <div className="mt-3 w-full border-t border-hairline" />

          <div className="flex flex-col items-center gap-0.5 text-caption-sm text-stone">
            <span>카파시 Wiki-LLM 방법론 + MCP 내장</span>
            <span>Tauri · React · tantivy · BlockNote</span>
            <a
              href="https://github.com/dean-studio/danbi"
              target="_blank"
              rel="noreferrer"
              className="mt-1 text-mute hover:text-on-dark hover:underline"
            >
              github.com/dean-studio/danbi ↗
            </a>
            <span className="mt-1">
              © 2026{" "}
              <a
                href="https://dean.kr"
                target="_blank"
                rel="noreferrer"
                className="text-mute hover:text-on-dark hover:underline"
              >
                Dean Works inc.
              </a>{" "}
              All rights reserved.
            </span>
          </div>
        </div>
      </div>
    </div>
  );
}
