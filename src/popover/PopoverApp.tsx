import { useCallback, useEffect, useMemo, useState } from "react";
import {
  AlertTriangle,
  ArrowRight,
  Clock,
  FileText,
  Settings as SettingsIcon,
} from "lucide-react";
import {
  ipc,
  type CommitSummary,
  type DailySnapshot,
  type McpStatus,
  type ReviewStore,
} from "@/lib/ipc";
import { cn } from "@/lib/utils";
import { installTheme, type ThemeChoice } from "@/lib/theme";

type Tab = "home" | "recent" | "reviews" | "settings";

type LoadState = {
  daily: DailySnapshot | null;
  recents: CommitSummary[];
  reviews: ReviewStore | null;
  mcp: McpStatus | null;
  autostart: boolean | null;
};

const initialState: LoadState = {
  daily: null,
  recents: [],
  reviews: null,
  mcp: null,
  autostart: null,
};

export function PopoverApp() {
  const [tab, setTab] = useState<Tab>("home");
  const [state, setState] = useState<LoadState>(initialState);

  const refresh = useCallback(async () => {
    const [daily, recents, reviews, mcp, autostart] = await Promise.all([
      ipc.dailySnapshot().catch(() => null),
      ipc.recentCommits(6).catch<CommitSummary[]>(() => []),
      ipc.reviewsList().catch(() => null),
      ipc.mcpStatus().catch(() => null),
      ipc.autostartStatus().catch<boolean | null>(() => null),
    ]);
    setState({ daily, recents, reviews, mcp, autostart });
  }, []);

  useEffect(() => {
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
  }, [refresh]);

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
  }, [refresh]);

  const pendingReviews = useMemo(() => {
    const items = state.reviews?.items ?? [];
    return items.filter((r) => r.status === "pending").length;
  }, [state.reviews]);

  return (
    <div className="fixed inset-0 flex overflow-hidden rounded-xl border border-hairline bg-surface/95 text-ink backdrop-blur-xl shadow-2xl shadow-black/40">
      <Sidebar
        tab={tab}
        setTab={setTab}
        reviewCount={pendingReviews}
      />
      <div className="flex min-w-0 flex-1 flex-col">
        <OpenDanbiCta />
        <div className="flex-1 overflow-y-auto px-4 pb-3">
          {tab === "home" && <HomePanel state={state} />}
          {tab === "recent" && <RecentPanel recents={state.recents} />}
          {tab === "reviews" && <ReviewsPanel reviews={state.reviews} />}
          {tab === "settings" && (
            <SettingsPanel
              mcp={state.mcp}
              autostart={state.autostart}
              onAutostartChange={async (next) => {
                await ipc.autostartSet(next).catch(() => {});
                refresh();
              }}
            />
          )}
        </div>
        <StatusBar mcp={state.mcp} />
      </div>
    </div>
  );
}

function Sidebar({
  tab,
  setTab,
  reviewCount,
}: {
  tab: Tab;
  setTab: (t: Tab) => void;
  reviewCount: number;
}) {
  return (
    <div className="flex w-14 flex-col items-center gap-1 border-r border-hairline bg-surface py-3">
      <TabButton
        active={tab === "home"}
        label="홈"
        onClick={() => setTab("home")}
        icon={<DanbiDropletIcon size={16} />}
      />
      <TabButton
        active={tab === "recent"}
        label="최근"
        onClick={() => setTab("recent")}
        icon={<Clock size={16} />}
      />
      <TabButton
        active={tab === "reviews"}
        label="리뷰"
        onClick={() => setTab("reviews")}
        icon={<AlertTriangle size={16} />}
        badge={reviewCount > 0 ? reviewCount : undefined}
      />
      <div className="flex-1" />
      <TabButton
        active={tab === "settings"}
        label="설정"
        onClick={() => setTab("settings")}
        icon={<SettingsIcon size={16} />}
      />
    </div>
  );
}

function TabButton({
  active,
  label,
  icon,
  onClick,
  badge,
}: {
  active: boolean;
  label: string;
  icon: React.ReactNode;
  onClick: () => void;
  badge?: number;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      title={label}
      className={cn(
        "relative grid h-10 w-10 place-items-center rounded-md text-mute transition-colors",
        active
          ? "bg-surface-card text-on-dark"
          : "hover:bg-surface-elevated hover:text-ink",
      )}
    >
      {icon}
      {badge !== undefined && (
        <span className="absolute -right-0.5 -top-0.5 grid h-4 min-w-[16px] place-items-center rounded-full bg-accent-red px-1 text-[10px] font-semibold text-on-dark">
          {badge}
        </span>
      )}
    </button>
  );
}

function OpenDanbiCta() {
  return (
    <div className="px-4 pt-4">
      <button
        type="button"
        onClick={() => {
          ipc.openMainWindow().catch(() => {});
        }}
        className="group flex h-12 w-full items-center justify-between rounded-md bg-primary px-4 text-[14px] font-semibold text-on-primary transition-colors hover:bg-primary-pressed"
      >
        <span>단비 열기</span>
        <ArrowRight
          size={16}
          className="transition-transform group-hover:translate-x-0.5"
        />
      </button>
    </div>
  );
}

function HomePanel({ state }: { state: LoadState }) {
  const daily = state.daily;
  const dateLabel = daily?.today ?? todayISO();
  const todayNotes = daily?.today_notes ?? [];
  return (
    <div className="flex flex-col gap-4 pt-4">
      <Section title={`오늘 · ${dateLabel}`}>
        {todayNotes.length > 0 ? (
          <ul className="flex flex-col gap-1">
            {todayNotes.slice(0, 3).map((n) => (
              <li
                key={`${n.project}/${n.domain}`}
                className="flex items-center justify-between gap-2 rounded-md px-2 py-1.5 text-[13px] hover:bg-surface-elevated"
              >
                <span className="flex min-w-0 items-center gap-2">
                  <FileText size={14} className="shrink-0 text-mute" />
                  <span className="truncate">{n.project}</span>
                </span>
                <span className="shrink-0 text-[11px] text-mute">
                  {n.domain}
                </span>
              </li>
            ))}
          </ul>
        ) : (
          <EmptyLine text="아직 오늘 노트에 기록이 없어요." />
        )}
      </Section>

      <Section title="최근 수정">
        {state.recents.length === 0 ? (
          <EmptyLine text="변경 이력이 없어요." />
        ) : (
          <ul className="flex flex-col">
            {state.recents.slice(0, 5).map((c) => (
              <li
                key={c.id}
                className="flex items-center justify-between gap-2 rounded-md px-2 py-1.5 text-[13px] hover:bg-surface-elevated"
              >
                <span className="flex min-w-0 items-center gap-2">
                  <FileText size={14} className="shrink-0 text-mute" />
                  <span className="truncate">{c.summary}</span>
                </span>
                <span className="shrink-0 text-[11px] text-mute">
                  {relativeTime(c.ts)}
                </span>
              </li>
            ))}
          </ul>
        )}
      </Section>
    </div>
  );
}

function RecentPanel({ recents }: { recents: CommitSummary[] }) {
  return (
    <div className="pt-4">
      <Section title="최근 수정">
        {recents.length === 0 ? (
          <EmptyLine text="변경 이력이 없어요." />
        ) : (
          <ul className="flex flex-col">
            {recents.map((c) => (
              <li
                key={c.id}
                className="flex items-center justify-between gap-2 rounded-md px-2 py-2 text-[13px] hover:bg-surface-elevated"
              >
                <span className="flex min-w-0 items-center gap-2">
                  <FileText size={14} className="shrink-0 text-mute" />
                  <span className="truncate">{c.summary}</span>
                </span>
                <span className="shrink-0 text-[11px] text-mute">
                  {relativeTime(c.ts)}
                </span>
              </li>
            ))}
          </ul>
        )}
      </Section>
    </div>
  );
}

function ReviewsPanel({ reviews }: { reviews: ReviewStore | null }) {
  const pending = reviews?.items.filter((r) => r.status === "pending") ?? [];
  return (
    <div className="pt-4">
      <Section title="대기 중인 리뷰">
        {pending.length === 0 ? (
          <EmptyLine text="처리할 리뷰가 없어요." />
        ) : (
          <ul className="flex flex-col gap-1">
            {pending.slice(0, 8).map((r) => (
              <li
                key={r.id}
                className="rounded-md border border-hairline bg-surface-elevated px-3 py-2"
              >
                <div className="flex items-center justify-between gap-2 text-[13px]">
                  <span className="truncate">
                    {r.project ?? "(프로젝트 없음)"}
                  </span>
                  <span className="text-[11px] text-mute">{r.kind}</span>
                </div>
                <p className="mt-1 truncate text-[12px] text-body">
                  {r.reason}
                </p>
              </li>
            ))}
          </ul>
        )}
      </Section>
    </div>
  );
}

function SettingsPanel({
  mcp,
  autostart,
  onAutostartChange,
}: {
  mcp: McpStatus | null;
  autostart: boolean | null;
  onAutostartChange: (next: boolean) => void;
}) {
  return (
    <div className="flex flex-col gap-4 pt-4">
      <Section title="시작">
        <Toggle
          label="로그인 시 자동 시작"
          sub="재부팅 후에도 단비가 메뉴바에 자동으로 상주합니다."
          checked={autostart ?? false}
          onChange={onAutostartChange}
        />
      </Section>

      <Section title="MCP 서버">
        <div className="rounded-md border border-hairline bg-surface-elevated px-3 py-2 text-[12px] text-body">
          {mcp ? (
            mcp.running ? (
              <span>
                실행 중 · 포트{" "}
                <span className="text-ink">{mcp.port}</span>
              </span>
            ) : (
              <span className="text-mute">중지됨</span>
            )
          ) : (
            <span className="text-mute">상태 확인 중…</span>
          )}
        </div>
      </Section>
    </div>
  );
}

function StatusBar({ mcp }: { mcp: McpStatus | null }) {
  const running = mcp?.running;
  return (
    <div className="flex items-center justify-between border-t border-hairline bg-surface px-4 py-2 text-[11px] text-mute">
      <span className="flex items-center gap-1.5">
        <span
          className={cn(
            "h-1.5 w-1.5 rounded-full",
            running ? "bg-accent-green" : "bg-stone",
          )}
        />
        {running ? `MCP :${mcp?.port ?? "—"}` : "MCP 중지됨"}
      </span>
      <span>vault 동기화됨</span>
    </div>
  );
}

function Section({
  title,
  children,
}: {
  title: string;
  children: React.ReactNode;
}) {
  return (
    <div>
      <div className="mb-1.5 px-2 text-[11px] uppercase tracking-[0.6px] text-mute">
        {title}
      </div>
      {children}
    </div>
  );
}

function EmptyLine({ text }: { text: string }) {
  return <div className="px-2 text-[12px] text-mute">{text}</div>;
}

function Toggle({
  label,
  sub,
  checked,
  onChange,
}: {
  label: string;
  sub?: string;
  checked: boolean;
  onChange: (next: boolean) => void;
}) {
  return (
    <button
      type="button"
      onClick={() => onChange(!checked)}
      className="flex w-full items-center justify-between gap-3 rounded-md border border-hairline bg-surface-elevated px-3 py-2 text-left transition-colors hover:bg-surface-card"
    >
      <span className="min-w-0">
        <span className="block text-[13px] text-ink">{label}</span>
        {sub && (
          <span className="block text-[11px] text-mute">{sub}</span>
        )}
      </span>
      <span
        className={cn(
          "relative h-5 w-9 shrink-0 rounded-full transition-colors",
          checked ? "bg-primary" : "bg-stone",
        )}
      >
        <span
          className={cn(
            "absolute top-0.5 h-4 w-4 rounded-full bg-surface-card shadow-sm transition-transform",
            checked ? "translate-x-[18px]" : "translate-x-0.5",
          )}
        />
      </span>
    </button>
  );
}

function todayISO() {
  const d = new Date();
  return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}`;
}

function relativeTime(tsMs: number) {
  const diff = Date.now() - tsMs;
  const abs = Math.abs(diff);
  const min = 60_000;
  const hr = 60 * min;
  const day = 24 * hr;
  if (abs < min) return "방금";
  if (abs < hr) return `${Math.floor(abs / min)}분 전`;
  if (abs < day) return `${Math.floor(abs / hr)}시간 전`;
  const d = Math.floor(abs / day);
  if (d < 7) return `${d}일 전`;
  const date = new Date(tsMs);
  return `${date.getMonth() + 1}/${date.getDate()}`;
}

/** Single-color droplet glyph — same path as the menu-bar tray icon
 *  (`src-tauri/icons/tray.svg`). Inline so it inherits text color via
 *  `currentColor`, lets the active/inactive tab tones drive it. */
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
