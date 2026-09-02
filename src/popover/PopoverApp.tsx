import { useEffect, useRef, useState } from "react";
import { ArrowRight, ChevronRight, Coins, Power } from "lucide-react";
import {
  ipc,
  type ActivityOverview,
  type CcSummary,
  type McpStatus,
  type ProjectActivity,
} from "@/lib/ipc";
import { cn } from "@/lib/utils";
import { installTheme, type ThemeChoice } from "@/lib/theme";
import { projectIconOf } from "@/components/ProjectIconPicker";

// 메뉴바 popover — 단비 본체로 가는 작은 진입점.
// 구성: (1) 컴팩트 "단비 열기" CTA, (2) Claude Code 사용량 mini,
// (3) 최근 활동 프로젝트 top 4 퀵셔트, (4) MCP 상태 + 종료 footer.
// 빠른 메모 입력은 v0.7.0 에서 제거 — popover 는 글랜스 + 본체 진입점에
// 집중. 본격 입력은 본체나 Quick Capture 단축키로.
export function PopoverApp() {
  const [mcp, setMcp] = useState<McpStatus | null>(null);
  const [activity, setActivity] = useState<ActivityOverview | null>(null);
  const [usage, setUsage] = useState<CcSummary | null>(null);
  const [usageEnabled, setUsageEnabled] = useState(true);
  // 마지막 refresh 시각 — focus 연타 시 재조회를 throttle 한다.
  const lastRefreshRef = useRef(0);

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
      // 최근 8s 안에 이미 새로고침했으면 skip — 팝오버가 뜬 채 창 포커스가
      // 오갈 때마다 백엔드를 두드리지 않도록.
      if (Date.now() - lastRefreshRef.current < 8000) return;
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
    lastRefreshRef.current = Date.now();
    // 두 호출은 서로 독립이라 병렬로. activity 는 backend 60s TTL 캐시 +
    // 5분 prefetch 가 있어서 보통 즉시 응답이지만, 캐시 miss 첫 호출은
    // 잠깐 걸릴 수 있으니 mcp 표시는 그동안 막지 않음.
    ipc.mcpStatus().then(setMcp).catch(() => setMcp(null));
    ipc
      .projectActivityOverview(30)
      .then(setActivity)
      .catch(() => setActivity(null));
    // Claude Code 사용량 — Settings 의 tray_usage 가 OFF 면 패널 자체 숨김.
    ipc
      .loadConfig()
      .then((cfg) => {
        const on = cfg?.usage?.tray_usage ?? true;
        setUsageEnabled(on);
        if (!on) {
          setUsage(null);
          return;
        }
        ipc.dashboardClaudeCode("today").then(setUsage).catch(() => setUsage(null));
      })
      .catch(() => setUsageEnabled(true));
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

        {usageEnabled && usage && usage.totals.total_tokens > 0 && (
          <UsageMini summary={usage} />
        )}

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

function UsageMini({ summary }: { summary: CcSummary }) {
  const t = summary.totals;
  return (
    <div className="rounded-md border border-hairline bg-surface-elevated px-2.5 py-2">
      <div className="flex items-center justify-between text-[10px] uppercase tracking-[0.4px] text-stone">
        <span className="flex items-center gap-1">
          <Coins size={10} />
          오늘 Claude Code
        </span>
      </div>
      <div className="mt-1 flex items-baseline justify-between gap-2">
        <span className="font-mono text-[18px] font-medium tabular-nums text-on-dark">
          {fmtToks(t.total_tokens)}
        </span>
        <span className="text-[10px] text-stone">tokens</span>
      </div>
      <div className="mt-1 flex items-center gap-1.5 text-[10px] text-stone">
        <Bar label="in" value={t.input_tokens} max={t.total_tokens} />
        <Bar label="out" value={t.output_tokens} max={t.total_tokens} tone="green" />
        <Bar
          label="cache"
          value={t.cache_creation_tokens + t.cache_read_tokens}
          max={t.total_tokens}
          tone="yellow"
        />
      </div>
    </div>
  );
}

function Bar({
  label,
  value,
  max,
  tone = "blue",
}: {
  label: string;
  value: number;
  max: number;
  tone?: "blue" | "green" | "yellow";
}) {
  const ratio = max > 0 ? value / max : 0;
  const color =
    tone === "green"
      ? "bg-accent-green"
      : tone === "yellow"
        ? "bg-accent-yellow"
        : "bg-accent-blue";
  return (
    <div className="flex flex-1 items-center gap-1">
      <span className="w-7 text-[9px]">{label}</span>
      <div className="relative h-1 flex-1 overflow-hidden rounded-xs bg-surface">
        <div
          className={`h-full ${color}`}
          style={{ width: `${ratio * 100}%` }}
        />
      </div>
    </div>
  );
}

function fmtToks(n: number): string {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(2)}M`;
  if (n >= 1_000) return `${(n / 1_000).toFixed(1)}k`;
  return n.toLocaleString();
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
