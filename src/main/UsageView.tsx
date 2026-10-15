import { useEffect, useMemo, useState } from "react";
import {
  Activity,
  FileText,
  GitCommit,
  HardDrive,
  Layers,
} from "lucide-react";
import {
  ipc,
  type ActivityOverview,
  type CommitSummary,
} from "@/lib/ipc";
import { cn } from "@/lib/utils";
import { useApp } from "@/state/store";
import { McpInboundCard } from "./McpInboundCard";
import { ClaudeCodeUsageCard } from "./ClaudeCodeUsageCard";
import { DanbiLlmUsageCard } from "./DanbiLlmUsageCard";
import { ErrorBoundary } from "@/components/ErrorBoundary";
import {
  PROJECT_COLOR_KEYS,
  projectColorVars,
} from "@/components/ProjectColorPicker";

/**
 * 사용량 화면 — 홈 첫 화면을 차지하던 사용량·집계 카드를 그대로 옮긴
 * 곳. 카드 컴포넌트는 변경 없음: Claude Code 사용량 · 단비 LLM 사용량 ·
 * MCP 저장 토큰 · 프로젝트 활동 분포 · 통계 4칸 · 30일 활동.
 */
export function UsageView() {
  const tree = useApp((s) => s.tree);
  const selectProject = useApp((s) => s.selectProject);
  const [commits, setCommits] = useState<CommitSummary[]>([]);

  useEffect(() => {
    ipc
      .recentCommits(200)
      .then(setCommits)
      .catch(() => setCommits([]));
  }, [tree]);

  const stats = useMemo(() => {
    const projects = tree?.projects ?? [];
    const domainCount = projects.reduce((a, p) => a + p.domains.length, 0);
    const bytes = projects.reduce(
      (a, p) => a + p.domains.reduce((b, d) => b + d.bytes, 0),
      0,
    );
    return {
      projectCount: projects.length,
      domainCount,
      bytes,
    };
  }, [tree]);

  const activity = useMemo(() => build30DayActivity(commits), [commits]);
  const hasProjects = (tree?.projects?.length ?? 0) > 0;

  return (
    <div className="flex h-full flex-col">
      <header
        data-tauri-drag-region
        className="flex h-12 shrink-0 items-center border-b border-hairline px-6"
      >
        <span className="text-[15px] font-medium tracking-[0.2px] text-ink">
          사용량
        </span>
      </header>

      <div className="flex-1 min-h-0 overflow-auto">
        <div className="mx-auto max-w-[1040px] px-6 py-8">
          {!hasProjects ? (
            <div className="text-[13px] text-mute">
              아직 프로젝트가 없어요. 첫 프로젝트를 만들면 여기에 사용량이
              채워집니다.
            </div>
          ) : (
            <>
              {/* 사용량/집계 카드들은 신규 데이터 소스(transcript·usage.jsonl·
                  MCP 로그) 를 파싱하므로, 한 카드가 예기치 못한 데이터에
                  터져도 화면 전체가 검게 되지 않도록 각각 ErrorBoundary. */}
              <ErrorBoundary scope="ClaudeCodeUsageCard" fallback={cardFallback}>
                <ClaudeCodeUsageCard />
              </ErrorBoundary>

              <ErrorBoundary scope="DanbiLlmUsageCard" fallback={cardFallback}>
                <DanbiLlmUsageCard />
              </ErrorBoundary>

              <ErrorBoundary scope="McpInboundCard" fallback={cardFallback}>
                <McpInboundCard onOpenProject={selectProject} />
              </ErrorBoundary>

              <div className="mt-6">
                <ErrorBoundary
                  scope="ProjectActivityCard"
                  fallback={cardFallback}
                >
                  <ProjectActivityCard />
                </ErrorBoundary>
              </div>

              <div className="mt-7 grid grid-cols-4 gap-3">
                <Stat
                  icon={<Layers size={16} />}
                  label="프로젝트"
                  value={String(stats.projectCount)}
                  hint="projects"
                  accent="blue"
                />
                <Stat
                  icon={<FileText size={16} />}
                  label="도메인"
                  value={String(stats.domainCount)}
                  hint="markdown"
                  accent="green"
                />
                <Stat
                  icon={<HardDrive size={16} />}
                  label="총 용량"
                  value={prettyBytes(stats.bytes)}
                  hint="all text"
                  accent="yellow"
                />
                <Stat
                  icon={<GitCommit size={16} />}
                  label="최근 작업"
                  value={
                    commits.length > 0 ? timeAgo(commits[0].ts * 1000) : "—"
                  }
                  hint="last commit"
                  accent="red"
                />
              </div>

              <div className="mt-6">
                <ActivityCard commits={commits.length} data={activity} />
              </div>
            </>
          )}
        </div>
      </div>
    </div>
  );
}

/** 카드가 렌더 중 터졌을 때 그 자리에만 그리는 작은 안내. 앱 전체는
 *  살아있고, "다시 시도" 로 그 카드만 재마운트한다. (Home 의 것과 동일 —
 *  순환 import 를 피하려고 복제.) */
function cardFallback(error: Error, reset: () => void) {
  return (
    <section className="mt-7 rounded-lg border border-hairline bg-surface px-4 py-4">
      <div className="text-[13px] text-on-dark">
        이 카드를 표시하는 중 문제가 생겼어요.
      </div>
      <div className="mt-1 truncate text-[11px] text-stone" title={error.message}>
        {error.message}
      </div>
      <button
        type="button"
        onClick={reset}
        className="mt-2 inline-flex h-7 items-center rounded-md border border-hairline bg-surface-elevated px-2.5 text-[11px] text-body transition-colors hover:border-hairline-strong hover:text-on-dark"
      >
        다시 시도
      </button>
    </section>
  );
}

/** Stat card with a left-edge accent strip so the four-up row reads as
 *  distinct buckets without relying on heavy color fills. */
function Stat({
  icon,
  label,
  value,
  hint,
  accent,
}: {
  icon: React.ReactNode;
  label: string;
  value: string;
  hint: string;
  accent: "blue" | "green" | "yellow" | "red";
}) {
  const stripMap: Record<typeof accent, string> = {
    blue: "bg-accent-blue",
    green: "bg-accent-green",
    yellow: "bg-accent-yellow",
    red: "bg-accent-red",
  };
  const iconMap: Record<typeof accent, string> = {
    blue: "text-accent-blue",
    green: "text-accent-green",
    yellow: "text-accent-yellow",
    red: "text-accent-red",
  };
  return (
    <div className="relative overflow-hidden rounded-lg border border-hairline bg-surface px-4 py-4 pl-5">
      <span
        className={cn(
          "absolute inset-y-0 left-0 w-0.5",
          stripMap[accent],
        )}
      />
      <div className="flex items-center gap-2">
        <span className={iconMap[accent]}>{icon}</span>
        <span className="text-[12px] uppercase tracking-[0.4px] text-stone">
          {label}
        </span>
      </div>
      <span className="mt-1.5 block text-[26px] font-medium leading-none text-ink">
        {value}
      </span>
      <span className="mt-1.5 block text-[12px] text-mute">{hint}</span>
    </div>
  );
}

function ActivityCard({
  commits,
  data,
}: {
  commits: number;
  data: number[];
}) {
  return (
    <section className="overflow-hidden rounded-lg border border-hairline bg-surface">
      <header className="flex items-center justify-between border-b border-hairline px-5 py-3">
        <div className="flex items-center gap-2">
          <GitCommit size={15} className="text-accent-blue" />
          <h2 className="text-[16px] font-medium text-ink">30일 활동</h2>
        </div>
        <span className="rounded-xs bg-accent-blue-soft px-2 py-0.5 text-[12px] font-medium text-accent-blue">
          {commits.toLocaleString()} commits
        </span>
      </header>
      <div className="px-5 py-4">
        <Sparkline data={data} />
      </div>
    </section>
  );
}

function Sparkline({ data }: { data: number[] }) {
  const max = Math.max(1, ...data);
  return (
    <div className="flex h-14 items-end gap-[3px]">
      {data.map((v, i) => {
        const pct = (v / max) * 100;
        return (
          <div
            key={i}
            className={cn(
              "flex-1 rounded-xs transition-colors",
              v === 0
                ? "bg-hairline"
                : v / max > 0.66
                  ? "bg-accent-blue"
                  : "bg-accent-blue-soft",
            )}
            style={{ height: `${Math.max(pct, v === 0 ? 6 : 12)}%` }}
            title={`${v} commits · ${i + 1}일 전`}
          />
        );
      })}
    </div>
  );
}

function build30DayActivity(commits: CommitSummary[]): number[] {
  const bins = new Array(30).fill(0);
  const now = Date.now();
  for (const c of commits) {
    const age = now - c.ts * 1000;
    const day = Math.floor(age / (24 * 60 * 60 * 1000));
    if (day >= 0 && day < 30) {
      bins[29 - day] += 1;
    }
  }
  return bins;
}

function prettyBytes(n: number): string {
  if (n < 1024) return `${n}B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)}KB`;
  return `${(n / (1024 * 1024)).toFixed(1)}MB`;
}

function timeAgo(ms: number): string {
  const diff = Date.now() - ms;
  if (diff < 60 * 1000) return "방금";
  if (diff < 60 * 60 * 1000) return `${Math.floor(diff / (60 * 1000))}분 전`;
  if (diff < 24 * 60 * 60 * 1000)
    return `${Math.floor(diff / (60 * 60 * 1000))}시간 전`;
  if (diff < 7 * 24 * 60 * 60 * 1000)
    return `${Math.floor(diff / (24 * 60 * 60 * 1000))}일 전`;
  const d = new Date(ms);
  return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}`;
}

// ---------- 프로젝트 활동 분포 (도넛) -----------------------------------
// 활동 점수 = commit + MCP 호출 가중합. "활동량" 이라 절대 시간(분) 은
// 아니지만 트렌드 비교는 정확하다.

const RANGE_OPTIONS: Array<{ days: number; label: string }> = [
  { days: 7, label: "7일" },
  { days: 30, label: "30일" },
  { days: 90, label: "90일" },
];

function ProjectActivityCard() {
  const [days, setDays] = useState(30);
  const [data, setData] = useState<ActivityOverview | null>(null);
  const [loading, setLoading] = useState(false);
  const projectColors = useApp((s) => s.cfg?.project_colors ?? {});
  const selectProject = useApp((s) => s.selectProject);

  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    ipc
      .projectActivityOverview(days)
      .then((res) => {
        if (!cancelled) setData(res);
      })
      .catch(() => {
        if (!cancelled) setData(null);
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [days]);

  const active = useMemo(
    () => (data?.by_project ?? []).filter((p) => p.activity_score > 0),
    [data],
  );

  const totalScore = useMemo(
    () => active.reduce((acc, p) => acc + p.activity_score, 0),
    [active],
  );

  // 프로젝트별 cfg 색을 우선 쓰되, 같은 도넛 안에서 색이 충돌하면
  // 팔레트 남은 키 중에서 채워서 segment 구분이 명확하게.
  const colorByProject = useMemo(
    () => assignDistinctColors(active.map((p) => p.project), projectColors),
    [active, projectColors],
  );

  return (
    <section className="overflow-hidden rounded-lg border border-hairline bg-surface">
      <header className="flex items-center justify-between border-b border-hairline px-5 py-3">
        <div className="flex items-center gap-2">
          <Activity size={14} className="text-stone" />
          <span className="text-[12px] font-semibold uppercase tracking-[0.6px] text-mute">
            프로젝트 활동 분포
          </span>
        </div>
        <div className="flex items-center gap-1">
          {RANGE_OPTIONS.map((opt) => (
            <button
              key={opt.days}
              onClick={() => setDays(opt.days)}
              className={cn(
                "rounded-sm px-2 py-1 text-[11px] transition-colors",
                days === opt.days
                  ? "bg-surface-elevated text-ink"
                  : "text-stone hover:text-mute",
              )}
            >
              {opt.label}
            </button>
          ))}
        </div>
      </header>

      {loading && !data && <ActivityCardSkeleton />}

      {data && active.length === 0 && (
        <div className="px-5 py-10 text-center text-[13px] text-stone">
          이 기간에 활동이 없어요.
        </div>
      )}

      {data && active.length > 0 && (
        <div className="grid gap-6 p-5 md:grid-cols-[180px_1fr]">
          {/* Donut */}
          <div className="flex flex-col items-center gap-3">
            <ActivityDonut
              segments={active.map((p) => ({
                key: p.project,
                value: p.activity_score,
                color: colorByProject[p.project],
              }))}
            />
            <div className="flex flex-col items-center text-center">
              <span className="text-[20px] font-medium text-ink">
                {totalScore.toLocaleString()}
              </span>
              <span className="text-[11px] text-stone">
                {days}일 활동량
              </span>
            </div>
          </div>

          {/* Ranked bars — clickable rows that jump straight into the
              project's dashboard. */}
          <ul className="flex flex-col gap-1">
            {active.map((p) => {
              const pct =
                totalScore > 0 ? (p.activity_score / totalScore) * 100 : 0;
              const color = colorByProject[p.project];
              return (
                <li key={p.project}>
                  <button
                    type="button"
                    onClick={() => selectProject(p.project)}
                    className="group flex w-full items-center gap-3 rounded-md px-2 py-1.5 text-left transition-colors hover:bg-surface-elevated"
                  >
                    <span
                      className="h-2.5 w-2.5 shrink-0 rounded-full"
                      style={{ background: color }}
                    />
                    <span className="w-28 truncate text-[13px] text-ink">
                      {p.project}
                    </span>
                    <div className="relative h-1.5 flex-1 overflow-hidden rounded-full bg-surface-elevated">
                      <div
                        className="absolute inset-y-0 left-0 rounded-full"
                        style={{
                          width: `${Math.max(pct, 1.5)}%`,
                          background: color,
                        }}
                      />
                    </div>
                    <span className="w-20 shrink-0 whitespace-nowrap text-right font-mono text-[11px] tabular-nums text-mute">
                      {p.activity_score.toLocaleString()}
                    </span>
                    <span className="w-12 shrink-0 whitespace-nowrap text-right text-[10px] text-stone">
                      {pct.toFixed(0)}%
                    </span>
                  </button>
                </li>
              );
            })}
          </ul>
        </div>
      )}

      {data && (
        <footer className="flex flex-wrap items-center gap-x-5 gap-y-1 border-t border-hairline bg-surface-elevated/30 px-5 py-2.5 text-[11px] text-stone">
          <span>
            commit{" "}
            <span className="font-mono text-mute">
              {data.total_commits.toLocaleString()}
            </span>
          </span>
          <span>
            MCP 호출{" "}
            <span className="font-mono text-mute">
              {data.total_mcp_calls.toLocaleString()}
            </span>
          </span>
          <span>
            MCP 토큰{" "}
            <span className="font-mono text-mute">
              {compactNumber(data.total_mcp_tokens)}
            </span>
          </span>
        </footer>
      )}
    </section>
  );
}

function ActivityCardSkeleton() {
  return (
    <div className="grid gap-6 p-5 md:grid-cols-[180px_1fr]">
      <div className="flex flex-col items-center gap-3">
        <div className="h-[140px] w-[140px] animate-pulse rounded-full border-[18px] border-surface-elevated" />
        <div className="flex flex-col items-center gap-1.5">
          <div className="h-5 w-16 animate-pulse rounded-sm bg-surface-elevated" />
          <div className="h-3 w-14 animate-pulse rounded-sm bg-surface-elevated" />
        </div>
      </div>
      <ul className="flex flex-col gap-2">
        {[0, 1, 2, 3, 4].map((i) => (
          <li key={i} className="flex items-center gap-3 py-1">
            <span className="h-2.5 w-2.5 shrink-0 animate-pulse rounded-full bg-surface-elevated" />
            <span
              className="h-3 animate-pulse rounded-sm bg-surface-elevated"
              style={{ width: `${50 + ((i * 7) % 30)}%` }}
            />
            <span className="ml-auto h-3 w-10 animate-pulse rounded-sm bg-surface-elevated" />
          </li>
        ))}
      </ul>
    </div>
  );
}

function compactNumber(n: number): string {
  if (n < 1000) return n.toString();
  if (n < 1_000_000) return `${(n / 1000).toFixed(1)}k`;
  return `${(n / 1_000_000).toFixed(1)}M`;
}

/** Build a `project → hex` map for the donut so adjacent slices can't
 *  share a color. We seed with each project's configured color when it's
 *  unique; collisions and unset projects get filled deterministically
 *  from the remaining palette keys (in declaration order). When the
 *  palette runs out (>8 active projects) we wrap, which is fine — the
 *  ranked-list label below the donut disambiguates. */
function assignDistinctColors(
  projects: string[],
  cfg: Record<string, string | undefined> = {},
): Record<string, string> {
  const palette = PROJECT_COLOR_KEYS;
  const result: Record<string, string> = {};
  const used = new Set<string>();
  const pending: string[] = [];

  // Pass 1: take cfg color when free.
  for (const p of projects) {
    const key = cfg[p];
    if (key && (palette as readonly string[]).includes(key) && !used.has(key)) {
      used.add(key);
      result[p] = projectColorVars(key).fg;
    } else {
      pending.push(p);
    }
  }

  // Pass 2: unique fallback from palette in declaration order.
  let cursor = 0;
  for (const p of pending) {
    let pick: string | null = null;
    for (let tries = 0; tries < palette.length; tries++) {
      const candidate = palette[cursor % palette.length];
      cursor++;
      if (!used.has(candidate)) {
        pick = candidate;
        used.add(candidate);
        break;
      }
    }
    if (!pick) {
      // Wrap — palette exhausted. Hue-shift via cursor so we still vary.
      pick = palette[cursor % palette.length];
      cursor++;
    }
    result[p] = projectColorVars(pick).fg;
  }
  return result;
}

/** SVG donut. Each segment is a stroke-dasharray slice on a centered
 *  circle. 1.5° gap between slices keeps colors visually distinct on
 *  the dark surface without losing tiny segments. */
function ActivityDonut({
  segments,
}: {
  segments: Array<{ key: string; value: number; color: string }>;
}) {
  const size = 140;
  const stroke = 18;
  const radius = (size - stroke) / 2;
  const cx = size / 2;
  const cy = size / 2;
  const circumference = 2 * Math.PI * radius;
  const total = segments.reduce((acc, s) => acc + s.value, 0);

  let offset = 0;
  const arcs = segments
    .filter((s) => s.value > 0)
    .map((s) => {
      const fraction = total > 0 ? s.value / total : 0;
      const length = fraction * circumference;
      const dasharray = `${length} ${circumference - length}`;
      const dashoffset = -offset;
      offset += length;
      return { ...s, dasharray, dashoffset };
    });

  return (
    <svg
      width={size}
      height={size}
      viewBox={`0 0 ${size} ${size}`}
      className="block"
    >
      <circle
        cx={cx}
        cy={cy}
        r={radius}
        fill="none"
        stroke="var(--color-surface-elevated)"
        strokeWidth={stroke}
      />
      {arcs.map((a, i) => (
        <circle
          key={a.key + i}
          cx={cx}
          cy={cy}
          r={radius}
          fill="none"
          stroke={a.color}
          strokeWidth={stroke}
          strokeDasharray={a.dasharray}
          strokeDashoffset={a.dashoffset}
          strokeLinecap="butt"
          transform={`rotate(-90 ${cx} ${cy})`}
        />
      ))}
    </svg>
  );
}
