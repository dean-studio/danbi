import { useEffect, useState } from "react";
import {
  Activity,
  Clock4,
  FileText,
  Leaf,
  Link2,
  RefreshCw,
  Sparkles,
} from "lucide-react";
import {
  ipc,
  type DashboardSnapshot,
  type GhostSuggestion,
  type VaultSuggestion,
} from "@/lib/ipc";
import { cn } from "@/lib/utils";

/**
 * "오늘의 단비" briefing card — renders the aggregate DashboardSnapshot from
 * `ipc.dashboardSnapshot()` as three quick-glance sections:
 *
 *   1) 제안  — ghost link suggestions + healing (orphan / empty / oversized)
 *   2) 회상  — reminiscence from 1w / 1m / 1y ago
 *   3) 활동  — rolling 7-day commit count
 *
 * Each row is clickable and routes through the callbacks so the host screen
 * (Home / ProjectHome) controls where the user actually lands.
 */
export function BriefingCard({
  onOpenDoc,
  onOpenGhostLinks,
}: {
  onOpenDoc: (project: string, domain: string) => void;
  onOpenGhostLinks?: (project: string) => void;
}) {
  const [data, setData] = useState<DashboardSnapshot | null>(null);
  const [loading, setLoading] = useState(false);
  const [lastLoadedAt, setLastLoadedAt] = useState<number | null>(null);

  const load = async () => {
    setLoading(true);
    try {
      const snap = await ipc.dashboardSnapshot();
      setData(snap);
      setLastLoadedAt(Date.now());
    } catch {
      /* ignore — card just doesn't render */
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    load();
  }, []);

  if (!data) {
    return (
      <section className="mt-6 rounded-lg border border-hairline bg-surface p-4">
        <div className="flex items-center gap-2 text-[13px] text-stone">
          <Leaf size={13} className="text-accent-green" />
          {loading ? "브리핑을 가져오는 중…" : "브리핑 준비 중."}
        </div>
      </section>
    );
  }

  const ghostCount = data.ghost_suggestions.length;
  const healingCount = data.healing.length;
  const suggestionTotal = ghostCount + healingCount;

  const reminTotal =
    data.daily.one_week_ago.length +
    data.daily.one_month_ago.length +
    data.daily.one_year_ago.length;

  const quiet =
    suggestionTotal === 0 &&
    reminTotal === 0 &&
    data.activity.commit_count === 0;

  return (
    <section className="mt-6 rounded-lg border border-hairline bg-surface p-4">
      <header className="flex items-center justify-between">
        <div className="flex items-center gap-2">
          <Leaf size={13} className="text-accent-green" />
          <h2 className="text-[14px] font-medium text-ink">
            단비가 발견한 것들
          </h2>
          <span className="font-mono text-caption-sm text-stone">
            {data.generated_at}
          </span>
        </div>
        <div className="flex items-center gap-2">
          {lastLoadedAt && (
            <span className="text-caption-sm text-stone">
              {timeAgoShort(lastLoadedAt)}
            </span>
          )}
          <button
            onClick={load}
            disabled={loading}
            title="새로고침"
            className="grid h-6 w-6 place-items-center rounded-sm text-stone transition-colors hover:bg-surface-elevated hover:text-on-dark"
          >
            <RefreshCw size={11} className={loading ? "animate-spin" : ""} />
          </button>
        </div>
      </header>

      {quiet ? (
        <div className="mt-3 text-caption-sm text-stone">
          오늘은 잠잠해요. vault가 조용합니다.
        </div>
      ) : (
        <div className="mt-3 flex flex-col gap-4">
          {suggestionTotal > 0 && (
            <Section
              icon={<Sparkles size={11} className="text-accent-yellow" />}
              label="제안"
              count={suggestionTotal}
            >
              {data.ghost_suggestions.slice(0, 3).map((g) => (
                <GhostRow
                  key={`${g.project}:${g.id}`}
                  ghost={g}
                  onClick={() => onOpenGhostLinks?.(g.project)}
                />
              ))}
              {data.healing.slice(0, 4).map((s, i) => (
                <HealingRow key={i} s={s} onOpen={onOpenDoc} />
              ))}
              {ghostCount + data.healing.length >
                Math.min(3, ghostCount) + Math.min(4, healingCount) && (
                <div className="pl-5 text-caption-sm text-stone">
                  +{suggestionTotal - Math.min(3, ghostCount) - Math.min(4, healingCount)}{" "}
                  더
                </div>
              )}
            </Section>
          )}

          {reminTotal > 0 && (
            <Section
              icon={<Clock4 size={11} className="text-accent-blue" />}
              label="회상"
              count={reminTotal}
            >
              <ReminRow
                label="1주 전"
                notes={data.daily.one_week_ago}
                onOpen={onOpenDoc}
              />
              <ReminRow
                label="1개월 전"
                notes={data.daily.one_month_ago}
                onOpen={onOpenDoc}
              />
              <ReminRow
                label="1년 전"
                notes={data.daily.one_year_ago}
                onOpen={onOpenDoc}
              />
            </Section>
          )}

          <Section
            icon={<Activity size={11} className="text-accent-green" />}
            label="활동"
            count={data.activity.commit_count}
          >
            <div className="pl-5 text-caption-sm text-mute">
              지난 {data.activity.days}일 동안{" "}
              <span className="text-on-dark">
                {data.activity.commit_count}
              </span>{" "}
              개 커밋
              {data.activity.changed_files.length > 0 &&
                ` · ${data.activity.changed_files.length}개 파일 변경`}
              .
            </div>
            {data.activity.recent_summaries.length > 0 && (
              <ul className="mt-1 flex flex-col gap-0.5 pl-5">
                {data.activity.recent_summaries.slice(0, 3).map((s, i) => (
                  <li
                    key={i}
                    className="truncate text-caption-sm text-body"
                    title={s}
                  >
                    <span className="text-stone">·</span> {s}
                  </li>
                ))}
              </ul>
            )}
          </Section>
        </div>
      )}
    </section>
  );
}

function Section({
  icon,
  label,
  count,
  children,
}: {
  icon: React.ReactNode;
  label: string;
  count: number;
  children: React.ReactNode;
}) {
  return (
    <div>
      <div className="mb-1.5 flex items-center gap-1.5">
        {icon}
        <span className="text-caption-sm uppercase tracking-[0.4px] text-mute">
          {label}
        </span>
        <span className="text-caption-sm text-stone">{count}</span>
      </div>
      <div className="flex flex-col gap-0.5">{children}</div>
    </div>
  );
}

function GhostRow({
  ghost,
  onClick,
}: {
  ghost: GhostSuggestion;
  onClick: () => void;
}) {
  const src = ghost.source_domain.split("/").slice(-1)[0];
  const tgt = ghost.target_domain.split("/").slice(-1)[0];
  return (
    <button
      onClick={onClick}
      className="flex w-full items-start gap-2 rounded-sm px-2 py-1.5 text-left transition-colors hover:bg-surface-elevated"
    >
      <span className="mt-0.5 shrink-0 rounded-xs bg-accent-blue-soft px-1.5 py-0.5 text-[10px] font-medium uppercase tracking-[0.4px] text-accent-blue">
        GHOST
      </span>
      <div className="min-w-0 flex-1">
        <div className="flex min-w-0 items-center gap-1.5 text-[13px]">
          <span className="shrink-0 text-mute">{ghost.project}</span>
          <span className="text-stone">·</span>
          <span className="flex min-w-0 items-center gap-1 truncate font-mono text-[12px]">
            <span className="truncate text-body">{src}</span>
            <Link2 size={10} className="shrink-0 text-stone" />
            <span className="truncate text-body">{tgt}</span>
          </span>
        </div>
        <div
          className="mt-0.5 truncate text-[12px] leading-[1.45] text-mute"
          title={ghost.reason}
        >
          {ghost.reason}
        </div>
      </div>
    </button>
  );
}

function HealingRow({
  s,
  onOpen,
}: {
  s: VaultSuggestion;
  onOpen: (project: string, domain: string) => void;
}) {
  if (s.kind === "EmptyProject") {
    return (
      <div className="flex items-start gap-2 px-2 py-1.5">
        <span className="mt-0.5 shrink-0 rounded-xs bg-surface-elevated px-1.5 py-0.5 text-[10px] uppercase tracking-[0.4px] text-mute">
          EMPTY
        </span>
        <div className="min-w-0 flex-1">
          <div className="truncate text-[13px] text-on-dark">{s.project}</div>
          <div className="truncate text-[12px] text-mute">
            프로젝트가 비어있어요.
          </div>
        </div>
      </div>
    );
  }
  const label =
    s.kind === "Orphan" ? "ORPHAN" : s.kind === "Empty" ? "EMPTY" : "LARGE";
  const tint =
    s.kind === "Oversized"
      ? "bg-accent-yellow-soft text-accent-yellow"
      : s.kind === "Orphan"
        ? "bg-accent-blue-soft text-accent-blue"
        : "bg-surface-elevated text-mute";
  const hint =
    s.kind === "Orphan"
      ? "참조 없음"
      : s.kind === "Empty"
        ? "비어있음"
        : `${(s.bytes / 1024).toFixed(1)}KB — 섹션으로 분할하는 걸 고려하세요.`;
  return (
    <button
      onClick={() => onOpen(s.project, s.domain)}
      className="flex w-full items-start gap-2 rounded-sm px-2 py-1.5 text-left transition-colors hover:bg-surface-elevated"
    >
      <span
        className={cn(
          "mt-0.5 shrink-0 rounded-xs px-1.5 py-0.5 text-[10px] uppercase tracking-[0.4px]",
          tint,
        )}
      >
        {label}
      </span>
      <div className="min-w-0 flex-1">
        <div className="flex min-w-0 items-center gap-1 truncate text-[13px]">
          <FileText size={10} className="shrink-0 text-stone" />
          <span className="text-mute">{s.project}</span>
          <span className="text-stone">/</span>
          <span className="truncate font-mono text-[12px] text-body">
            {s.domain}
          </span>
        </div>
        <div className="mt-0.5 truncate text-[12px] leading-[1.45] text-mute">
          {hint}
        </div>
      </div>
    </button>
  );
}

function ReminRow({
  label,
  notes,
  onOpen,
}: {
  label: string;
  notes: DashboardSnapshot["daily"]["one_week_ago"];
  onOpen: (project: string, domain: string) => void;
}) {
  if (notes.length === 0) return null;
  return (
    <div className="flex flex-wrap items-center gap-1.5 px-2 py-0.5 text-caption-sm">
      <span className="w-12 shrink-0 text-stone">{label}</span>
      {notes.map((n) => (
        <button
          key={`${n.project}/${n.domain}`}
          onClick={() => onOpen(n.project, n.domain)}
          className="inline-flex items-center gap-1 rounded-full border border-hairline bg-surface-elevated px-2 py-0.5 text-[11px] text-body hover:border-hairline-strong hover:text-on-dark"
        >
          <span className="text-mute">{n.project}</span>
          <span className="text-stone">·</span>
          <span className="font-mono text-[10px]">{n.date}</span>
        </button>
      ))}
    </div>
  );
}

function timeAgoShort(ms: number): string {
  const diff = Date.now() - ms;
  if (diff < 60_000) return "방금";
  if (diff < 3_600_000) return `${Math.floor(diff / 60_000)}분 전`;
  return `${Math.floor(diff / 3_600_000)}시간 전`;
}
