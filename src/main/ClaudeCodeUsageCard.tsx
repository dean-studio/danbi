import { useEffect, useState } from "react";
import {
  Activity,
  Box,
  CalendarDays,
  Coins,
  History,
  Layers,
  RefreshCw,
  ScrollText,
  Sparkles,
  TrendingUp,
} from "lucide-react";
import {
  ipc,
  type CcDailyPoint,
  type CcEffectiveMode,
  type CcRange,
  type CcSummaryWithMode,
  type CcTotals,
} from "@/lib/ipc";
import { cn } from "@/lib/utils";

/**
 * Claude Code 사용량 카드 (v0.7.0).
 *
 * `~/.claude/projects/**\/*.jsonl` transcript 를 직접 읽어 토큰·비용·세션·
 * 모델/프로젝트별 분포·히트맵·히스토리를 보여준다. agentcat connectors
 * 와 달리 OAuth endpoint 호출 없이 자기 디스크의 자기 파일만 사용 — 권한
 * 이슈/유지보수 부담 0.
 *
 * 모드별 UI 차별화 (`effective_mode`):
 *   - `bedrock` / `api_key` → 종량형. 큰 비용 + 모델별 USD/KRW.
 *   - `subscription`        → 정액. 비용은 숨기고 토큰만.
 *   - `mixed`               → 둘 다. 비용은 종량 부분만.
 *   - `unknown`             → transcript 없음. 안내만.
 */
export function ClaudeCodeUsageCard() {
  const [tab, setTab] = useState<"current" | "history">("current");
  const [range, setRange] = useState<CcRange>("today");
  const [data, setData] = useState<CcSummaryWithMode | null>(null);
  const [loading, setLoading] = useState(false);
  const [reindexing, setReindexing] = useState(false);

  const reload = () => {
    setLoading(true);
    ipc
      .dashboardClaudeCode(range)
      .then(setData)
      .catch(() => setData(null))
      .finally(() => setLoading(false));
  };

  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    ipc
      .dashboardClaudeCode(range)
      .then((d) => {
        if (!cancelled) setData(d);
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
  }, [range]);

  const reindex = async () => {
    setReindexing(true);
    try {
      await ipc.dashboardClaudeCodeReindex();
      reload();
    } finally {
      setReindexing(false);
    }
  };

  const summary = data?.summary;
  const mode = data?.effective_mode ?? "unknown";
  const enabled = data?.enabled ?? true;
  const showCost = mode === "bedrock" || mode === "api_key" || mode === "mixed";

  const empty =
    !loading && (summary == null || summary.totals.calls === 0);

  return (
    <section className="mt-7 overflow-hidden rounded-lg border border-hairline bg-surface">
      <header className="flex flex-wrap items-center justify-between gap-3 border-b border-hairline px-4 py-2.5">
        <div className="flex items-center gap-1.5">
          <Sparkles size={13} className="text-accent-yellow" />
          <h2 className="text-[14px] font-medium text-ink">
            Claude Code 사용량
          </h2>
          <ModeBadge mode={mode} />
        </div>
        <div className="flex items-center gap-2">
          <button
            type="button"
            onClick={reindex}
            disabled={reindexing}
            className="inline-flex h-7 items-center gap-1 rounded-md border border-hairline bg-surface-elevated px-2 text-[11px] text-stone transition-colors hover:border-hairline-strong hover:text-on-dark disabled:opacity-50"
            title="transcript 캐시 무효화 후 다시 읽기"
          >
            <RefreshCw
              size={11}
              className={cn(reindexing && "animate-spin")}
            />
            <span>다시 인덱싱</span>
          </button>
          <TabToggle tab={tab} onChange={setTab} />
          {tab === "current" && <RangeToggle range={range} onChange={setRange} />}
        </div>
      </header>

      {!enabled ? (
        <DisabledState />
      ) : tab === "current" ? (
        <CurrentTab
          loading={loading}
          empty={empty}
          summary={summary}
          mode={mode}
          showCost={showCost}
        />
      ) : (
        <HistoryTab showCost={showCost} />
      )}
    </section>
  );
}

// ---------- 현재 탭 ------------------------------------------------------

function CurrentTab({
  loading,
  empty,
  summary,
  mode,
  showCost,
}: {
  loading: boolean;
  empty: boolean;
  summary?: CcSummaryWithMode["summary"];
  mode: CcEffectiveMode;
  showCost: boolean;
}) {
  if (loading && !summary) return <CardSkeleton />;
  if (empty) return <EmptyState mode={mode} />;
  if (!summary) return null;

  const totals = summary.totals;

  return (
    <div className="px-4 py-4">
      {/* 큰 숫자 행 */}
      <div className="flex flex-wrap items-baseline gap-3">
        <span className="font-mono text-[36px] font-medium leading-none tracking-tight text-ink tabular-nums">
          {fmtTokens(totals.total_tokens)}
        </span>
        <span className="text-[12px] text-stone">tokens</span>
        {showCost && (
          <span className="ml-1 inline-flex items-baseline gap-1.5 rounded-md border border-hairline bg-surface-elevated px-2 py-1 font-mono text-[12px] text-on-dark tabular-nums">
            ₩{Math.round(totals.krw).toLocaleString()}
            <span className="text-[10px] text-stone">
              (${totals.usd.toFixed(2)})
            </span>
          </span>
        )}
        <span className="ml-auto text-[12px] text-stone">
          {totals.calls.toLocaleString()} calls · {totals.sessions} sessions
        </span>
      </div>

      {/* 토큰 4축 분리 — input / output / cache_creation / cache_read */}
      <div className="mt-3 grid grid-cols-2 gap-2 sm:grid-cols-4">
        <Stat label="input" value={totals.input_tokens} />
        <Stat label="output" value={totals.output_tokens} />
        <Stat
          label="cache write"
          value={totals.cache_creation_tokens}
          hint="input × 1.25"
        />
        <Stat
          label="cache read"
          value={totals.cache_read_tokens}
          hint="input × 0.10"
        />
      </div>

      {mode === "subscription" && (
        <p className="mt-3 rounded-md border border-hairline bg-surface-elevated px-3 py-2 text-[11px] text-stone">
          Pro/Max 구독 모드 — 토큰만 의미가 있고 비용은 정액입니다. 잔여
          한도/리셋 시간은 transcript 만으로는 알 수 없어요.
        </p>
      )}

      {/* 일별 sparkline */}
      <Sparkline daily={summary.daily} className="mt-4" />

      {/* 모델별 + 프로젝트별 — 2열 */}
      <div className="mt-4 grid gap-3 lg:grid-cols-2">
        <BreakdownPanel
          title="모델별"
          icon={<Box size={11} className="text-accent-blue" />}
          rows={summary.by_model.map((m) => ({
            label: prettyModel(m.model),
            extra: m.backend === "bedrock" ? "Bedrock" : "Anthropic API",
            tokens: m.totals.total_tokens,
            usd: m.totals.usd,
            krw: m.totals.krw,
          }))}
          showCost={showCost}
        />
        <BreakdownPanel
          title="프로젝트별 (cwd)"
          icon={<Layers size={11} className="text-accent-green" />}
          rows={summary.by_project.map((p) => ({
            label: p.label,
            extra: p.cwd,
            tokens: p.totals.total_tokens,
            usd: p.totals.usd,
            krw: p.totals.krw,
          }))}
          showCost={showCost}
        />
      </div>

      {/* 시간대 히트맵 */}
      <Heatmap hourly={summary.hourly} className="mt-4" />

      {/* disclaimer */}
      <p className="mt-4 text-[11px] leading-snug text-stone">
        {summary.disclaimer}
      </p>
    </div>
  );
}

// ---------- 히스토리 탭 (Phase 4.5) -------------------------------------

function HistoryTab({ showCost }: { showCost: boolean }) {
  const [daily, setDaily] = useState<CcDailyPoint[] | null>(null);
  const [monthly, setMonthly] = useState<CcDailyPoint[] | null>(null);
  const [yearAgo, setYearAgo] = useState<CcTotals | null>(null);
  const [topDays, setTopDays] = useState<CcDailyPoint[] | null>(null);

  useEffect(() => {
    let cancelled = false;
    Promise.all([
      ipc.dashboardClaudeCodeDaily(365),
      ipc.dashboardClaudeCodeMonthly(),
      ipc.dashboardClaudeCode("all"),
    ]).then(([d, m, all]) => {
      if (cancelled) return;
      setDaily(d);
      setMonthly(m);
      setYearAgo(all.summary.year_ago_today);
      setTopDays(all.summary.top_days);
    });
    return () => {
      cancelled = true;
    };
  }, []);

  if (!daily || !monthly) {
    return <CardSkeleton />;
  }

  return (
    <div className="space-y-5 px-4 py-4">
      {/* 90일 sparkline */}
      <div>
        <SectionHead
          icon={<Activity size={11} className="text-accent-blue" />}
          title="최근 90일"
          right={
            <span className="font-mono text-[11px] text-stone tabular-nums">
              {totalSpan(daily.slice(-90))}
            </span>
          }
        />
        <Sparkline daily={daily.slice(-90)} className="mt-2" />
      </div>

      {/* 1년 잔디 */}
      <div>
        <SectionHead
          icon={<CalendarDays size={11} className="text-accent-green" />}
          title="1년 잔디 (cell = 그날 토큰)"
        />
        <YearGrass daily={daily} />
      </div>

      {/* 월별 추이 + 1년 전 비교 */}
      <div className="grid gap-3 lg:grid-cols-2">
        <MonthlyTrend monthly={monthly} showCost={showCost} />
        <YearAgoCompare yearAgo={yearAgo} />
      </div>

      {/* 가장 비쌌던 날 */}
      {topDays && topDays.length > 0 && (
        <div>
          <SectionHead
            icon={<TrendingUp size={11} className="text-accent-yellow" />}
            title={showCost ? "가장 비쌌던 날" : "가장 많이 쓴 날"}
          />
          <ul className="mt-2 divide-y divide-hairline rounded-md border border-hairline bg-surface-elevated">
            {topDays.map((d) => (
              <li
                key={d.date}
                className="flex items-baseline gap-3 px-3 py-2 text-[12px]"
              >
                <span className="font-mono text-[11px] text-stone tabular-nums">
                  {d.date}
                </span>
                <span className="ml-auto font-mono tabular-nums text-on-dark">
                  {fmtTokens(d.totals.total_tokens)}
                </span>
                {showCost && (
                  <span className="font-mono text-[11px] text-stone tabular-nums">
                    ₩{Math.round(d.totals.krw).toLocaleString()}
                  </span>
                )}
              </li>
            ))}
          </ul>
        </div>
      )}
    </div>
  );
}

function totalSpan(daily: CcDailyPoint[]): string {
  const t = daily.reduce((s, d) => s + d.totals.total_tokens, 0);
  return `합계 ${fmtTokens(t)}`;
}

function MonthlyTrend({
  monthly,
  showCost,
}: {
  monthly: CcDailyPoint[];
  showCost: boolean;
}) {
  const last12 = monthly.slice(-12);
  const max = Math.max(1, ...last12.map((m) => m.totals.total_tokens));
  return (
    <div>
      <SectionHead
        icon={<History size={11} className="text-accent-blue" />}
        title="월별 추이 (최근 12개월)"
      />
      <ul className="mt-2 space-y-1">
        {last12.map((m) => {
          const ratio = m.totals.total_tokens / max;
          return (
            <li key={m.date} className="flex items-center gap-2">
              <span className="w-16 shrink-0 font-mono text-[11px] text-stone">
                {m.date}
              </span>
              <div className="relative h-2 flex-1 overflow-hidden rounded-xs bg-surface-elevated">
                <div
                  className="h-full bg-on-dark/60"
                  style={{ width: `${ratio * 100}%` }}
                />
              </div>
              <span className="w-20 shrink-0 text-right font-mono text-[11px] tabular-nums text-on-dark">
                {fmtTokens(m.totals.total_tokens)}
              </span>
              {showCost && (
                <span className="w-20 shrink-0 text-right font-mono text-[11px] tabular-nums text-stone">
                  ₩{Math.round(m.totals.krw).toLocaleString()}
                </span>
              )}
            </li>
          );
        })}
      </ul>
    </div>
  );
}

function YearAgoCompare({ yearAgo }: { yearAgo: CcTotals | null }) {
  return (
    <div>
      <SectionHead
        icon={<History size={11} className="text-accent-yellow" />}
        title="1년 전 오늘"
      />
      {yearAgo ? (
        <div className="mt-2 rounded-md border border-hairline bg-surface-elevated px-3 py-3">
          <div className="font-mono text-[20px] font-medium tabular-nums text-on-dark">
            {fmtTokens(yearAgo.total_tokens)}
          </div>
          <div className="mt-0.5 text-[11px] text-stone">
            {yearAgo.calls} calls · {yearAgo.sessions} sessions
          </div>
          {yearAgo.usd > 0 && (
            <div className="mt-1 font-mono text-[12px] text-stone tabular-nums">
              ₩{Math.round(yearAgo.krw).toLocaleString()} / ${yearAgo.usd.toFixed(2)}
            </div>
          )}
        </div>
      ) : (
        <p className="mt-2 rounded-md border border-hairline bg-surface-elevated px-3 py-3 text-[11px] text-stone">
          작년 오늘 기록이 없어요. 단비 transcript 가 1년치 모이면 여기에
          비교가 나타납니다.
        </p>
      )}
    </div>
  );
}

// ---------- 1년 잔디 -----------------------------------------------------

function YearGrass({ daily }: { daily: CcDailyPoint[] }) {
  // 365 칸 — 7행 × ~53열 GitHub 잔디.
  const last365 = daily.slice(-365);
  const max = Math.max(1, ...last365.map((d) => d.totals.total_tokens));

  // 첫 칸을 일요일에 맞춤.
  const first = last365[0];
  const firstDow = first
    ? new Date(first.date + "T00:00:00").getDay()
    : 0;
  const cells: (CcDailyPoint | null)[] = [];
  for (let i = 0; i < firstDow; i += 1) cells.push(null);
  cells.push(...last365);
  while (cells.length % 7 !== 0) cells.push(null);

  const cols: (CcDailyPoint | null)[][] = [];
  for (let c = 0; c < cells.length / 7; c += 1) {
    cols.push(cells.slice(c * 7, c * 7 + 7));
  }

  return (
    <div className="mt-2 overflow-x-auto">
      <div className="inline-flex gap-[2px]">
        {cols.map((col, i) => (
          <div key={i} className="flex flex-col gap-[2px]">
            {col.map((cell, j) => {
              if (!cell) {
                return (
                  <div
                    key={j}
                    className="h-[10px] w-[10px] rounded-[1px] border border-hairline/30 bg-transparent"
                  />
                );
              }
              const ratio = cell.totals.total_tokens / max;
              const tint =
                cell.totals.total_tokens === 0
                  ? "transparent"
                  : `rgba(245, 197, 24, ${Math.max(0.07, Math.min(0.85, ratio * 0.85))})`;
              return (
                <div
                  key={j}
                  className="h-[10px] w-[10px] rounded-[1px] border border-hairline/30"
                  style={{ background: tint }}
                  title={`${cell.date} — ${fmtTokens(cell.totals.total_tokens)} tokens · ₩${Math.round(cell.totals.krw).toLocaleString()}`}
                />
              );
            })}
          </div>
        ))}
      </div>
    </div>
  );
}

// ---------- 공통 컴포넌트 -----------------------------------------------

function SectionHead({
  icon,
  title,
  right,
}: {
  icon: React.ReactNode;
  title: string;
  right?: React.ReactNode;
}) {
  return (
    <div className="flex items-center gap-2 text-[12px] text-stone">
      {icon}
      <span>{title}</span>
      {right && <span className="ml-auto">{right}</span>}
    </div>
  );
}

function Stat({
  label,
  value,
  hint,
}: {
  label: string;
  value: number;
  hint?: string;
}) {
  return (
    <div className="rounded-md border border-hairline bg-surface-elevated px-3 py-2">
      <div className="flex items-center gap-1 text-[10px] uppercase tracking-wider text-stone">
        <span>{label}</span>
        {hint && <span className="text-stone/70">· {hint}</span>}
      </div>
      <div className="mt-0.5 font-mono text-[16px] font-medium tabular-nums text-on-dark">
        {fmtTokens(value)}
      </div>
    </div>
  );
}

type BreakdownRow = {
  label: string;
  extra?: string;
  tokens: number;
  usd: number;
  krw: number;
};

function BreakdownPanel({
  title,
  icon,
  rows,
  showCost,
}: {
  title: string;
  icon: React.ReactNode;
  rows: BreakdownRow[];
  showCost: boolean;
}) {
  if (rows.length === 0) return null;
  const totalT = rows.reduce((s, r) => s + r.tokens, 0);
  return (
    <div>
      <div className="mb-2 flex items-center gap-1.5 text-[12px] text-stone">
        {icon}
        <span>{title}</span>
      </div>
      <ul className="divide-y divide-hairline rounded-md border border-hairline bg-surface-elevated">
        {rows.map((r) => {
          const pct = totalT === 0 ? 0 : (r.tokens / totalT) * 100;
          return (
            <li
              key={`${r.label}/${r.extra ?? ""}`}
              className="flex items-baseline gap-2 px-3 py-2 text-[12px]"
            >
              <span className="min-w-0 flex-1 truncate text-on-dark">
                {r.label}
                {r.extra && (
                  <span className="ml-1.5 truncate font-mono text-[10px] text-stone">
                    {r.extra}
                  </span>
                )}
              </span>
              <span className="shrink-0 font-mono text-[10px] text-stone tabular-nums">
                {pct.toFixed(0)}%
              </span>
              <span className="shrink-0 font-mono text-[11px] tabular-nums text-on-dark">
                {fmtTokens(r.tokens)}
              </span>
              {showCost && (
                <span className="shrink-0 font-mono text-[11px] tabular-nums text-stone">
                  ₩{Math.round(r.krw).toLocaleString()}
                </span>
              )}
            </li>
          );
        })}
      </ul>
    </div>
  );
}

function Sparkline({
  daily,
  className,
}: {
  daily: CcDailyPoint[];
  className?: string;
}) {
  if (daily.length === 0) return null;
  const max = Math.max(1, ...daily.map((d) => d.totals.total_tokens));
  return (
    <div className={cn("rounded-md border border-hairline bg-surface-elevated p-3", className)}>
      <div className="flex h-16 items-end gap-[2px]">
        {daily.map((d) => {
          const ratio = d.totals.total_tokens / max;
          return (
            <div
              key={d.date}
              className="flex-1 rounded-[1px] bg-on-dark/60"
              style={{ height: `${Math.max(2, ratio * 100)}%` }}
              title={`${d.date} — ${fmtTokens(d.totals.total_tokens)} tokens`}
            />
          );
        })}
      </div>
      <div className="mt-1 flex justify-between font-mono text-[10px] text-stone">
        <span>{daily[0]?.date}</span>
        <span>{daily[daily.length - 1]?.date}</span>
      </div>
    </div>
  );
}

function Heatmap({
  hourly,
  className,
}: {
  hourly: { dow: number; hour: number; tokens: number }[];
  className?: string;
}) {
  // 7×24 매트릭스로 펼침.
  const cells: number[][] = Array.from({ length: 7 }, () =>
    Array(24).fill(0),
  );
  let max = 0;
  for (const h of hourly) {
    cells[h.dow][h.hour] = h.tokens;
    if (h.tokens > max) max = h.tokens;
  }
  if (max === 0) return null;

  const days = ["일", "월", "화", "수", "목", "금", "토"];
  const now = new Date();
  const nowDow = now.getDay();
  const nowHour = now.getHours();

  return (
    <div
      className={cn(
        "rounded-md border border-hairline bg-surface-elevated p-3",
        className,
      )}
    >
      <div className="mb-2 flex items-center gap-1.5 text-[12px] text-stone">
        <ScrollText size={11} className="text-accent-yellow" />
        <span>요일 × 시간</span>
      </div>
      {cells.map((row, dow) => (
        <div key={dow} className="flex items-center gap-1">
          <span className="w-5 text-[10px] text-stone">{days[dow]}</span>
          <div
            className="grid flex-1 gap-[1px]"
            style={{ gridTemplateColumns: "repeat(24, minmax(0, 1fr))" }}
          >
            {row.map((v, hour) => {
              const ratio = max > 0 ? v / max : 0;
              const isNow = dow === nowDow && hour === nowHour;
              const tint =
                v === 0
                  ? "transparent"
                  : `rgba(245, 197, 24, ${Math.max(0.06, Math.min(0.85, ratio * 0.85))})`;
              return (
                <div
                  key={hour}
                  className={cn(
                    "h-3 rounded-[1px] border",
                    isNow ? "border-on-dark" : "border-hairline/30",
                  )}
                  style={{ background: tint }}
                  title={`${days[dow]} ${hour.toString().padStart(2, "0")}:00 — ${fmtTokens(v)} tokens`}
                />
              );
            })}
          </div>
        </div>
      ))}
    </div>
  );
}

// ---------- 컨트롤 ------------------------------------------------------

function TabToggle({
  tab,
  onChange,
}: {
  tab: "current" | "history";
  onChange: (t: "current" | "history") => void;
}) {
  return (
    <div className="flex items-center gap-0.5 rounded-md border border-hairline bg-surface-elevated p-0.5">
      {(["current", "history"] as const).map((v) => (
        <button
          key={v}
          onClick={() => onChange(v)}
          className={cn(
            "rounded-xs px-2 py-1 text-[11px] tracking-tight transition-colors",
            tab === v
              ? "bg-surface-card text-on-dark"
              : "text-stone hover:text-on-dark",
          )}
        >
          {v === "current" ? "현재" : "히스토리"}
        </button>
      ))}
    </div>
  );
}

function RangeToggle({
  range,
  onChange,
}: {
  range: CcRange;
  onChange: (r: CcRange) => void;
}) {
  const opts: Array<{ value: CcRange; label: string }> = [
    { value: "today", label: "오늘" },
    { value: "7d", label: "7일" },
    { value: "30d", label: "30일" },
    { value: "90d", label: "90일" },
    { value: "all", label: "전체" },
  ];
  return (
    <div className="flex items-center gap-0.5 rounded-md border border-hairline bg-surface-elevated p-0.5">
      {opts.map((o) => (
        <button
          key={o.value}
          onClick={() => onChange(o.value)}
          className={cn(
            "rounded-xs px-2 py-1 font-mono text-[11px] tracking-tight transition-colors",
            range === o.value
              ? "bg-surface-card text-on-dark"
              : "text-stone hover:text-on-dark",
          )}
        >
          {o.label}
        </button>
      ))}
    </div>
  );
}

function ModeBadge({ mode }: { mode: CcEffectiveMode }) {
  const map: Record<CcEffectiveMode, { text: string; tone: string }> = {
    bedrock: { text: "Bedrock", tone: "text-accent-blue" },
    api_key: { text: "API key", tone: "text-accent-green" },
    subscription: { text: "Subscription", tone: "text-accent-yellow" },
    mixed: { text: "Mixed", tone: "text-on-dark" },
    unknown: { text: "—", tone: "text-stone" },
  };
  const m = map[mode];
  return (
    <span
      className={cn(
        "ml-1 rounded-xs border border-hairline bg-surface-elevated px-1.5 py-0.5 font-mono text-[10px] uppercase tracking-wider",
        m.tone,
      )}
    >
      {m.text}
    </span>
  );
}

// ---------- 빈/오류 상태 ------------------------------------------------

function EmptyState({ mode }: { mode: CcEffectiveMode }) {
  return (
    <div className="px-4 py-10 text-center">
      <div className="mx-auto mb-3 flex h-10 w-10 items-center justify-center rounded-full border border-hairline bg-surface-elevated">
        <Coins size={16} className="text-stone" />
      </div>
      <p className="text-[13px] text-on-dark">아직 사용량이 없어요.</p>
      <p className="mt-1 text-[11px] text-stone">
        {mode === "unknown"
          ? "~/.claude/projects 에 transcript 가 없습니다. Claude Code 를 한 번이라도 실행하면 자동으로 채워집니다."
          : "선택한 기간에 호출이 없어요. 다른 기간을 선택해보세요."}
      </p>
    </div>
  );
}

function DisabledState() {
  return (
    <div className="px-4 py-10 text-center">
      <p className="text-[13px] text-on-dark">추적이 꺼져 있어요.</p>
      <p className="mt-1 text-[11px] text-stone">
        Settings → LLM 사용량 에서 "Claude Code transcript 추적" 을 켜면
        보입니다.
      </p>
    </div>
  );
}

function CardSkeleton() {
  return (
    <div className="px-4 py-5">
      <div className="h-9 w-40 animate-pulse rounded bg-surface-elevated" />
      <div className="mt-3 grid grid-cols-4 gap-2">
        {Array.from({ length: 4 }).map((_, i) => (
          <div
            key={i}
            className="h-12 animate-pulse rounded-md bg-surface-elevated"
          />
        ))}
      </div>
      <div className="mt-4 h-16 animate-pulse rounded-md bg-surface-elevated" />
    </div>
  );
}

// ---------- 유틸 --------------------------------------------------------

function fmtTokens(n: number): string {
  if (n >= 1_000_000_000) return `${(n / 1_000_000_000).toFixed(2)}B`;
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(2)}M`;
  if (n >= 1_000) return `${(n / 1_000).toFixed(1)}k`;
  return n.toLocaleString();
}

function prettyModel(id: string): string {
  // Bedrock prefix 정리. 예: "us.anthropic.claude-opus-4-7-20250201-v1:0" → "claude-opus-4-7"
  const stems = [
    "claude-opus-4-7",
    "claude-opus-4-6",
    "claude-sonnet-4-6",
    "claude-sonnet-4-5",
    "claude-haiku-4-5",
    "claude-haiku-3-5",
  ];
  const lower = id.toLowerCase();
  for (const s of stems) if (lower.includes(s)) return s;
  return id;
}
