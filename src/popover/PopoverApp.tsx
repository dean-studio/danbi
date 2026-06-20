import { useEffect, useRef, useState } from "react";
import { ArrowRight, ChevronRight, Power, Send } from "lucide-react";
import {
  ipc,
  type ActivityOverview,
  type McpStatus,
  type ProjectActivity,
} from "@/lib/ipc";
import { cn } from "@/lib/utils";
import { installTheme, type ThemeChoice } from "@/lib/theme";
import { projectIconOf } from "@/components/ProjectIconPicker";

// 메뉴바 popover — 단비 본체로 가는 작은 진입점.
// 구성: (1) 컴팩트 "단비 열기" CTA, (2) 최근 활동 프로젝트 top 4 퀵셔트,
// (3) MCP 상태 + 종료 버튼 footer. 그 외 (daily/recent/reviews/settings)
// 패널은 본체에서 더 잘 보여서 popover 에선 제거.
export function PopoverApp() {
  const [mcp, setMcp] = useState<McpStatus | null>(null);
  const [activity, setActivity] = useState<ActivityOverview | null>(null);
  // Quick log — popover 만으로 한 줄 메모를 가장 최근 활동 프로젝트의
  // 오늘 daily 노트에 append. 키보드만으로 메뉴바 → 입력 → 닫힘 루프가
  // 끝나는 게 핵심.
  const [quickInput, setQuickInput] = useState("");
  const [quickStatus, setQuickStatus] = useState<
    null | "saving" | "ok" | "err"
  >(null);
  const inputRef = useRef<HTMLTextAreaElement | null>(null);

  useEffect(() => {
    // 첫 paint 가 끝난 다음 tick 에 IPC 시작 — webview 가 즉시 화면을
    // 그리고 나서 백엔드 호출이 들어가야 macOS 비치볼 spinner 가 안 뜬다.
    // requestAnimationFrame 두 번 = "paint 끝난 직후" 보장.
    requestAnimationFrame(() => {
      requestAnimationFrame(() => {
        (async () => {
          try {
            const cfg = await ipc.loadConfig();
            const choice =
              (cfg?.appearance.theme as ThemeChoice | undefined) ?? "dark";
            installTheme(choice);
          } catch {
            /* ignore */
          }
        })();
        refresh();
      });
    });
  }, []);

  useEffect(() => {
    function onFocus() {
      refresh();
    }
    function onKey(e: KeyboardEvent) {
      if (e.key === "Escape") {
        ipc.hidePopover().catch(() => {});
      }
    }
    window.addEventListener("focus", onFocus);
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("focus", onFocus);
      window.removeEventListener("keydown", onKey);
    };
  }, []);

  function refresh() {
    // 두 호출은 서로 독립이라 병렬로. activity 는 backend 60s TTL 캐시 +
    // 5분 prefetch 가 있어서 보통 즉시 응답이지만, 캐시 miss 첫 호출은
    // 잠깐 걸릴 수 있으니 mcp 표시는 그동안 막지 않음.
    ipc.mcpStatus().then(setMcp).catch(() => setMcp(null));
    ipc
      .projectActivityOverview(30)
      .then(setActivity)
      .catch(() => setActivity(null));
  }

  function openMain() {
    ipc.openMainWindow().catch(() => {});
  }
  function openProject(p: string) {
    ipc.openProjectInMain(p).catch(() => {});
  }
  function quit() {
    ipc.quitApp().catch(() => {});
  }

  /** 활동 점수 가장 높은 프로젝트. 사용자가 "이게 뭘 의미하는지" 명확히
   *  알도록 입력란 placeholder 에 프로젝트명을 노출. */
  const quickTarget: string | null = (activity?.by_project ?? [])
    .filter((p) => p.activity_score > 0)
    .map((p) => p.project)[0] ?? null;

  async function submitQuick() {
    const text = quickInput.trim();
    if (!text || !quickTarget || quickStatus === "saving") return;
    setQuickStatus("saving");
    try {
      await ipc.danbiLogQuick(quickTarget, text);
      setQuickInput("");
      setQuickStatus("ok");
      // 짧게 visual feedback 후 popover 자동 닫힘 → 사용자가 "메모 저장됐다"
      // 확인 후 흐름 끊기지 않게.
      setTimeout(() => {
        setQuickStatus(null);
        ipc.hidePopover().catch(() => {});
      }, 600);
    } catch (e) {
      console.error("[danbi] quick log failed", e);
      setQuickStatus("err");
      setTimeout(() => setQuickStatus(null), 1800);
    }
  }

  const top: ProjectActivity[] = (activity?.by_project ?? [])
    .filter((p) => p.activity_score > 0)
    .slice(0, 4);

  return (
    <div
      className="fixed inset-0 flex flex-col overflow-hidden rounded-xl border border-hairline bg-surface/95 text-ink backdrop-blur-xl shadow-2xl shadow-black/40"
      style={{ cursor: "default" }}
    >
      <div className="flex flex-col gap-2 px-3 pt-3">
        {/* 작은 CTA — 보던 텍스트는 그대로 두되 button 자체는 컴팩트하게 */}
        <button
          type="button"
          onClick={openMain}
          className="group flex h-9 w-full items-center justify-between rounded-md bg-primary px-3 text-[13px] font-semibold text-on-primary transition-colors hover:bg-primary-pressed"
        >
          <span className="flex items-center gap-2">
            <DanbiDropletIcon size={14} />
            단비 열기
          </span>
          <ArrowRight
            size={13}
            className="transition-transform group-hover:translate-x-0.5"
          />
        </button>

        {top.length > 0 && (
          <ul className="flex flex-col gap-0.5 pt-1">
            {top.map((p) => (
              <ProjectQuickRow
                key={p.project}
                project={p}
                onOpen={() => openProject(p.project)}
              />
            ))}
          </ul>
        )}

        {quickTarget && (
          <div className="flex flex-col gap-1.5 pt-2">
            <div className="flex items-center justify-between text-[10px] uppercase tracking-[0.4px] text-stone">
              <span>빠른 메모</span>
              <span className="font-mono normal-case tracking-normal">
                → {quickTarget}/오늘
              </span>
            </div>
            <div
              className={cn(
                "flex items-end gap-1.5 rounded-md border bg-surface-elevated p-1.5 transition-colors",
                quickStatus === "err"
                  ? "border-accent-red/60"
                  : quickStatus === "ok"
                    ? "border-accent-green/60"
                    : "border-hairline focus-within:border-hairline-strong",
              )}
            >
              <textarea
                ref={inputRef}
                value={quickInput}
                onChange={(e) => setQuickInput(e.target.value)}
                onKeyDown={(e) => {
                  if (
                    (e.key === "Enter" && (e.metaKey || e.ctrlKey)) ||
                    (e.key === "Enter" && !e.shiftKey && quickInput.trim().length < 80)
                  ) {
                    // Cmd+Enter 는 항상 제출, 짧은 한 줄 입력이면 그냥
                    // Enter 로도 제출 (TODO 메모용).
                    e.preventDefault();
                    submitQuick();
                  }
                }}
                placeholder="한 줄 메모 → daily 노트에 append (⌘↵)"
                rows={2}
                className="flex-1 resize-none bg-transparent px-1 text-[12px] text-body placeholder:text-stone/70 focus:outline-none"
              />
              <button
                type="button"
                onClick={submitQuick}
                disabled={!quickInput.trim() || quickStatus === "saving"}
                className={cn(
                  "inline-flex h-7 w-7 shrink-0 items-center justify-center rounded-sm transition-colors",
                  quickInput.trim() && quickStatus !== "saving"
                    ? "bg-primary text-on-primary hover:bg-primary-pressed"
                    : "bg-surface text-stone",
                )}
                title="저장 (⌘↵)"
              >
                <Send size={11} />
              </button>
            </div>
            {quickStatus === "err" && (
              <span className="text-[10px] text-accent-red">
                저장 실패 — 본체 열어 확인해주세요
              </span>
            )}
          </div>
        )}
      </div>

      <div className="flex-1" />

      <footer className="flex items-center justify-between border-t border-hairline px-3 py-2 text-[11px] text-mute">
        <span className="flex items-center gap-1.5">
          <span
            className={cn(
              "h-1.5 w-1.5 rounded-full",
              mcp?.running ? "bg-accent-green" : "bg-stone",
            )}
          />
          {mcp?.running ? `MCP :${mcp.port}` : "MCP 중지됨"}
        </span>
        <button
          type="button"
          onClick={quit}
          title="단비 종료"
          className="flex items-center gap-1 rounded-sm px-1.5 py-1 text-stone transition-colors hover:bg-surface-elevated hover:text-accent-red"
        >
          <Power size={11} />
          <span>종료</span>
        </button>
      </footer>
    </div>
  );
}

function ProjectQuickRow({
  project,
  onOpen,
}: {
  project: ProjectActivity;
  onOpen: () => void;
}) {
  const Icon = projectIconOf(null);
  return (
    <button
      type="button"
      onClick={onOpen}
      className="group flex items-center justify-between gap-2 rounded-md px-2 py-1.5 text-left transition-colors hover:bg-surface-elevated"
    >
      <span className="flex min-w-0 items-center gap-2">
        <Icon size={13} className="shrink-0 text-stone" />
        <span className="truncate text-[12px] text-ink">{project.project}</span>
      </span>
      <span className="flex shrink-0 items-center gap-2 text-[10px] text-stone">
        <span className="font-mono tabular-nums">
          {project.activity_score}
        </span>
        <ChevronRight
          size={11}
          className="text-stone/60 transition-transform group-hover:translate-x-0.5"
        />
      </span>
    </button>
  );
}

/** Single-color droplet glyph — same path as the menu-bar tray icon. */
function DanbiDropletIcon({ size = 16 }: { size?: number }) {
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 22 22"
      fill="currentColor"
      aria-hidden="true"
    >
      <path d="M11 2 C 11 2, 4 11, 4 14.6 C 4 18.13, 7.13 21, 11 21 C 14.87 21, 18 18.13, 18 14.6 C 18 11, 11 2, 11 2 Z" />
    </svg>
  );
}
