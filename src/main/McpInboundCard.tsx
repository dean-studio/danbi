import { useEffect, useMemo, useState } from "react";
import {
  AlertTriangle,
  ChevronRight,
  Code2,
  Coins,
  Download,
  FileText,
  Info,
  Sparkles,
  TrendingUp,
} from "lucide-react";
import { save as saveDialog } from "@tauri-apps/plugin-dialog";
import {
  ipc,
  type McpClientBreakdown,
  type McpInboundRange,
  type McpProjectStats,
  type McpToolBreakdown,
  type McpVaultSummary,
} from "@/lib/ipc";
import { cn } from "@/lib/utils";

/**
 * "MCP 저장 토큰 (추정)" — Home 카드.
 *
 * Counts content tokens that external agents (Claude Code, Codex)
 * stored in the vault via MCP write tools. The big number is a rough
 * proxy for "how much knowledge got pushed into the vault" — not the
 * LLM bill. The disclaimer banner is permanent and cannot be dismissed
 * because the metric is fundamentally an estimate, not a billed total.
 *
 * Drill-down: clicking a project row opens its detail view, which the
 * host (Home) is responsible for routing — we just expose `onOpenProject`.
 */
export function McpInboundCard({
  onOpenProject,
}: {
  onOpenProject?: (project: string) => void;
}) {
  const [range, setRange] = useState<McpInboundRange>("7d");
  const [data, setData] = useState<McpVaultSummary | null>(null);
  const [loading, setLoading] = useState(false);

  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    ipc
      .dashboardMcpInbound(range)
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

  const sparklineMax = useMemo(() => {
    if (!data || data.daily.length === 0) return 1;
    return Math.max(1, ...data.daily.map((d) => d.tokens));
  }, [data]);

  const empty = !loading && (!data || data.total_calls === 0);

  return (
    <section className="mt-7 overflow-hidden rounded-lg border border-hairline bg-surface">
      <header className="flex items-center justify-between gap-3 border-b border-hairline px-4 py-2.5">
        <div className="flex items-center gap-1.5">
          <Sparkles size={13} className="text-accent-yellow" />
          <h2 className="text-[14px] font-medium text-ink">
            MCP 저장 토큰
          </h2>
          <span className="ml-1 rounded-xs border border-hairline bg-surface-elevated px-1.5 py-0.5 font-mono text-[10px] uppercase tracking-wider text-stone">
            추정
          </span>
        </div>
        <div className="flex items-center gap-2">
          <ExportMenu />
          <RangeToggle range={range} onChange={setRange} />
        </div>
      </header>

      {loading && !data ? (
        <McpCardSkeleton />
      ) : empty ? (
        <EmptyState />
      ) : (
        <div className="px-4 py-4">
          <div className="flex items-baseline gap-3">
            <span className="font-mono text-[36px] font-medium leading-none tracking-tight text-ink tabular-nums">
              {fmtTokens(data?.total_tokens ?? 0)}
            </span>
            <span className="text-[12px] text-stone">tokens</span>
            <span className="ml-auto text-[12px] text-stone">
              {(data?.total_calls ?? 0).toLocaleString()} calls
            </span>
          </div>

          {/* Reference cost estimate. Subtitle to the big number, never
              presented as the primary metric. */}
          {data?.cost_estimate && (
            <div className="mt-1.5 flex items-center gap-1.5 text-[11px] text-stone">
              <Coins size={11} className="text-stone" />
              <span>
                같은 분량을 <strong className="font-medium text-on-dark">{prettyModel(data.cost_estimate.model_stem)}</strong> input으로 다시 보낸다고 가정하면 약{" "}
                <strong className="font-mono font-medium text-on-dark">₩{Math.round(data.cost_estimate.krw).toLocaleString()}</strong>
                {" "}({"$"}{data.cost_estimate.usd.toFixed(4)})
              </span>
              <span className="rounded-xs border border-hairline bg-surface-elevated px-1.5 py-0.5 font-mono text-[10px] uppercase tracking-wider text-stone">
                참고용
              </span>
            </div>
          )}

          {/* Sparkline — bare bones SVG. ~120px tall. */}
          <Sparkline daily={data?.daily ?? []} max={sparklineMax} />

          {/* 도구별 breakdown — 에이전트별 패널은 홈 카드에선 노이즈가
              많아 제거. 에이전트별 분포가 필요하면 프로젝트 상세에서 확인. */}
          <div className="mt-4">
            <BreakdownPanel
              title="도구별"
              icon={<Code2 size={11} className="text-accent-green" />}
              rows={(data?.by_tool ?? []).map((t: McpToolBreakdown) => ({
                label: t.tool,
                tokens: t.tokens,
                calls: t.calls,
              }))}
              total={data?.total_tokens ?? 0}
            />
          </div>

          {/* Anomalies — only render when we have any. Calls out
              spikes vs the 7-day baseline. Yellow accent matches the
              warning level (not red — not an error, just unusual). */}
          {(data?.anomalies?.length ?? 0) > 0 && (
            <div className="mt-4 rounded-md border border-accent-yellow-soft bg-accent-yellow-soft/30 p-3">
              <div className="mb-2 flex items-center gap-1.5 text-[11px] uppercase tracking-wider text-accent-yellow">
                <AlertTriangle size={11} />
                <span>평소보다 많이 저장됨</span>
              </div>
              <ul className="space-y-1.5">
                {data!.anomalies.map((a) => (
                  <li
                    key={`${a.project}/${a.domain}/${a.date}`}
                    className="flex items-baseline gap-2 text-[12px]"
                  >
                    <button
                      type="button"
                      onClick={() => onOpenProject?.(a.project)}
                      className="truncate text-on-dark hover:underline"
                    >
                      {a.project}
                      <span className="text-stone"> / </span>
                      <span className="font-mono text-[11px]">{a.domain}</span>
                    </button>
                    <span className="ml-auto shrink-0 font-mono text-[11px] text-stone">
                      {a.date}
                    </span>
                    <span className="shrink-0 font-mono text-[11px] tabular-nums text-on-dark">
                      {fmtTokens(a.tokens)}
                    </span>
                    <span className="shrink-0 rounded-xs bg-accent-yellow/20 px-1.5 py-0.5 font-mono text-[10px] tabular-nums text-accent-yellow">
                      ×{a.multiple.toFixed(1)}
                    </span>
                  </li>
                ))}
              </ul>
            </div>
          )}

          {/* Top contributors across the vault — distinct from
              "프로젝트별" below in that this is the (project, domain)
              granularity. Cleaner picture of "where did the writing
              actually go?" */}
          {(data?.top_contributors?.length ?? 0) > 0 && (
            <div className="mt-4">
              <div className="mb-2 flex items-center gap-1.5 text-[12px] text-stone">
                <TrendingUp size={11} className="text-accent-green" />
                <span>가장 많이 저장된 곳</span>
              </div>
              <ul className="divide-y divide-hairline rounded-md border border-hairline bg-surface-elevated">
                {data!.top_contributors.map((c) => (
                  <li
                    key={`${c.project}/${c.domain}`}
                    className="flex items-center gap-3 px-3 py-2 text-[12px]"
                  >
                    <button
                      type="button"
                      onClick={() => onOpenProject?.(c.project)}
                      className="min-w-0 flex-1 truncate text-left text-on-dark hover:underline"
                    >
                      <span>{c.project}</span>
                      <span className="text-stone"> / </span>
                      <span className="font-mono text-[11px]">{c.domain}</span>
                    </button>
                    <span className="shrink-0 font-mono tabular-nums text-on-dark">
                      {fmtTokens(c.tokens)}
                    </span>
                    <span className="shrink-0 font-mono text-[11px] text-stone tabular-nums">
                      {c.calls.toLocaleString()}
                    </span>
                  </li>
                ))}
              </ul>
            </div>
          )}

          {/* Heatmap — 24×7 grid of when the saving happens. Cells
              get a yellow tint proportional to the cell's share of the
              hottest cell. Empty windows render as a flat grid. */}
          {data && data.heatmap && data.heatmap.total_tokens > 0 && (
            <HeatmapPanel heatmap={data.heatmap} />
          )}

          {/* Top projects. */}
          <div className="mt-4">
            <div className="mb-2 flex items-center gap-1.5 text-[12px] text-stone">
              <FileText size={11} className="text-stone" />
              <span>프로젝트별 Top 10</span>
            </div>
            <ul className="divide-y divide-hairline">
              {(data?.by_project ?? []).slice(0, 10).map((p) => (
                <ProjectRow
                  key={p.project}
                  project={p}
                  total={data?.total_tokens ?? 1}
                  onOpen={onOpenProject}
                />
              ))}
              {(data?.by_project?.length ?? 0) === 0 && (
                <li className="py-3 text-[12px] text-stone">
                  이 기간에는 저장된 콘텐츠가 없어요.
                </li>
              )}
            </ul>
          </div>
        </div>
      )}

      {/* Permanent disclaimer banner — never dismissable. */}
      <DisclaimerBanner text={data?.disclaimer} />
    </section>
  );
}

// ---------- subcomponents -------------------------------------------

function HeatmapPanel({ heatmap }: { heatmap: import("@/lib/ipc").McpHeatmap }) {
  const days = ["일", "월", "화", "수", "목", "금", "토"];
  // Mark "now" so the user can spot themselves on the grid. Defensive:
  // if the timezone math returns something odd we just don't highlight.
  const now = new Date();
  const nowDow = now.getDay();
  const nowHour = now.getHours();
  return (
    <div className="mt-4 rounded-md border border-hairline bg-surface-elevated p-3">
      <div className="mb-2 flex items-center gap-1.5 text-[11px] uppercase tracking-wider text-stone">
        <span>시간대 (현지 시각)</span>
      </div>
      <div className="overflow-hidden">
        <div className="flex">
          <div className="flex w-6 flex-col items-end pr-1">
            {days.map((d) => (
              <span
                key={d}
                className="flex h-3 items-center font-mono text-[9px] text-stone"
              >
                {d}
              </span>
            ))}
          </div>
          <div
            className="grid flex-1 gap-[1px]"
            style={{ gridTemplateColumns: "repeat(24, minmax(0, 1fr))" }}
          >
            {heatmap.cells.flatMap((row, dow) =>
              row.map((v, hour) => {
                const ratio = heatmap.max_cell > 0 ? v / heatmap.max_cell : 0;
                const isNow = dow === nowDow && hour === nowHour;
                // alpha 0.05 ~ 0.85 so even tiny cells get a slight tint
                // when the bucket has *any* value, fully empty stays bg.
                const tint =
                  v === 0
                    ? "transparent"
                    : `rgba(245, 197, 24, ${Math.max(0.06, Math.min(0.85, ratio * 0.85))})`;
                return (
                  <div
                    key={`${dow}-${hour}`}
                    className={cn(
                      "h-3 rounded-[1px] border transition-colors",
                      isNow
                        ? "border-on-dark"
                        : "border-hairline/30",
                    )}
                    style={{ background: tint }}
                    title={`${days[dow]} ${hour
                      .toString()
                      .padStart(2, "0")}:00 — ${fmtTokens(v)} tokens`}
                  />
                );
              }),
            )}
          </div>
        </div>
        <div className="mt-1 flex">
          <div className="w-6" />
          <div
            className="grid flex-1 gap-[1px]"
            style={{ gridTemplateColumns: "repeat(24, minmax(0, 1fr))" }}
          >
            {Array.from({ length: 24 }, (_, h) => (
              <span
                key={h}
                className={cn(
                  "text-center font-mono text-[8px] text-stone",
                  h % 6 === 0 ? "" : "opacity-0",
                )}
              >
                {h.toString().padStart(2, "0")}
              </span>
            ))}
          </div>
        </div>
      </div>
    </div>
  );
}

function ExportMenu() {
  const [open, setOpen] = useState(false);

  const exportAs = async (format: "json" | "csv") => {
    setOpen(false);
    const ext = format === "json" ? "json" : "csv";
    const today = new Date().toISOString().slice(0, 10);
    const filename = `danbi-usage-${today}.${ext}`;
    const path = await saveDialog({
      defaultPath: filename,
      filters: [
        {
          name: format.toUpperCase(),
          extensions: [ext],
        },
      ],
    });
    if (!path || typeof path !== "string") return;
    try {
      if (format === "json") {
        await ipc.usageExportJson(path);
      } else {
        await ipc.usageExportCsv(path);
      }
    } catch (e) {
      console.error("[mcp-inbound] export failed", e);
    }
  };

  return (
    <div className="relative">
      <button
        type="button"
        onClick={() => setOpen((o) => !o)}
        title="원시 사용량 데이터 내보내기"
        className="inline-flex h-7 items-center gap-1 rounded-md border border-hairline bg-surface-elevated px-2 text-[11px] text-stone transition-colors hover:border-hairline-strong hover:text-on-dark"
      >
        <Download size={11} />
        <span>내보내기</span>
      </button>
      {open && (
        <>
          {/* Outside-click catcher */}
          <div
            className="fixed inset-0 z-40"
            onClick={() => setOpen(false)}
          />
          <div className="absolute right-0 z-50 mt-1 w-32 overflow-hidden rounded-md border border-hairline bg-surface-card shadow-lg">
            <button
              type="button"
              onClick={() => exportAs("json")}
              className="block w-full px-3 py-2 text-left text-[12px] text-on-dark hover:bg-surface-elevated"
            >
              JSON
            </button>
            <button
              type="button"
              onClick={() => exportAs("csv")}
              className="block w-full px-3 py-2 text-left text-[12px] text-on-dark hover:bg-surface-elevated"
            >
              CSV
            </button>
          </div>
        </>
      )}
    </div>
  );
}

function RangeToggle({
  range,
  onChange,
}: {
  range: McpInboundRange;
  onChange: (r: McpInboundRange) => void;
}) {
  const opts: Array<{ value: McpInboundRange; label: string }> = [
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

function Sparkline({
  daily,
  max,
}: {
  daily: { date: string; tokens: number }[];
  max: number;
}) {
  if (daily.length === 0) return null;
  const w = 100;
  const h = 22;
  const step = daily.length > 1 ? w / (daily.length - 1) : 0;
  const points = daily.map((d, i) => {
    const x = i * step;
    const y = h - (d.tokens / max) * h;
    return `${x.toFixed(2)},${y.toFixed(2)}`;
  });
  return (
    <div className="mt-3">
      <svg
        viewBox={`0 0 ${w} ${h}`}
        preserveAspectRatio="none"
        className="h-12 w-full"
      >
        <polyline
          fill="none"
          stroke="currentColor"
          strokeWidth="0.6"
          strokeLinejoin="round"
          strokeLinecap="round"
          className="text-accent-yellow"
          points={points.join(" ")}
        />
        {/* Last-point dot. */}
        {points.length > 0 && (
          <circle
            cx={(daily.length - 1) * step}
            cy={h - (daily[daily.length - 1].tokens / max) * h}
            r="0.9"
            className="fill-accent-yellow"
          />
        )}
      </svg>
      <div className="mt-1 flex justify-between font-mono text-[10px] text-stone">
        <span>{daily[0]?.date}</span>
        <span>{daily[daily.length - 1]?.date}</span>
      </div>
    </div>
  );
}

function BreakdownPanel({
  title,
  icon,
  rows,
  total,
}: {
  title: string;
  icon: React.ReactNode;
  rows: { label: string; tokens: number; calls: number }[];
  total: number;
}) {
  return (
    <div className="rounded-md border border-hairline bg-surface-elevated p-3">
      <div className="mb-2 flex items-center gap-1.5 text-[11px] uppercase tracking-wider text-stone">
        {icon}
        <span>{title}</span>
      </div>
      <ul className="space-y-1.5">
        {rows.length === 0 ? (
          <li className="text-[12px] text-stone">—</li>
        ) : (
          rows.slice(0, 4).map((r) => {
            const pct = total > 0 ? (r.tokens / total) * 100 : 0;
            return (
              <li key={r.label}>
                <div className="flex items-baseline justify-between gap-2 text-[12px]">
                  <span className="truncate text-on-dark">{r.label}</span>
                  <span className="shrink-0 font-mono tabular-nums text-on-dark">
                    {fmtTokens(r.tokens)}
                  </span>
                </div>
                <div className="mt-0.5 h-1 overflow-hidden rounded-full bg-surface">
                  <div
                    className="h-full bg-accent-blue/60"
                    style={{ width: `${Math.min(100, pct)}%` }}
                  />
                </div>
              </li>
            );
          })
        )}
      </ul>
    </div>
  );
}

function ProjectRow({
  project,
  total,
  onOpen,
}: {
  project: McpProjectStats;
  total: number;
  onOpen?: (project: string) => void;
}) {
  const pct = total > 0 ? (project.tokens / total) * 100 : 0;
  const clickable = !!onOpen;
  return (
    <li>
      <button
        type="button"
        disabled={!clickable}
        onClick={() => onOpen?.(project.project)}
        className={cn(
          "group flex w-full items-center gap-3 py-2 text-left transition-colors",
          clickable && "hover:bg-surface-elevated",
          !clickable && "cursor-default",
        )}
      >
        <div className="flex min-w-0 flex-1 items-center gap-3">
          <span className="truncate text-[13px] text-on-dark">
            {project.project}
          </span>
          {project.by_client.slice(0, 2).map((c: McpClientBreakdown) => (
            <span
              key={c.client}
              className="hidden shrink-0 rounded-xs bg-surface px-1.5 py-0.5 font-mono text-[10px] uppercase tracking-wider text-stone sm:inline"
            >
              {clientLabel(c.client)}
            </span>
          ))}
        </div>
        <div className="hidden h-1 w-24 shrink-0 overflow-hidden rounded-full bg-surface md:block">
          <div
            className="h-full bg-accent-yellow/70"
            style={{ width: `${Math.min(100, pct)}%` }}
          />
        </div>
        <span className="shrink-0 font-mono text-[12px] tabular-nums text-on-dark">
          {fmtTokens(project.tokens)}
        </span>
        <span className="shrink-0 font-mono text-[11px] text-stone tabular-nums">
          {project.calls.toLocaleString()}
        </span>
        {clickable && (
          <ChevronRight
            size={13}
            className="shrink-0 text-stone transition-colors group-hover:text-on-dark"
          />
        )}
      </button>
    </li>
  );
}

function McpCardSkeleton() {
  return (
    <div className="px-4 py-4">
      <div className="flex items-baseline gap-3">
        <div className="h-9 w-32 animate-pulse rounded-sm bg-surface-elevated" />
        <div className="h-3 w-12 animate-pulse rounded-sm bg-surface-elevated" />
        <div className="ml-auto h-3 w-16 animate-pulse rounded-sm bg-surface-elevated" />
      </div>
      <div className="mt-4 h-[120px] animate-pulse rounded-md bg-surface-elevated" />
      <div className="mt-4 grid grid-cols-2 gap-3">
        <div className="h-24 animate-pulse rounded-md bg-surface-elevated" />
        <div className="h-24 animate-pulse rounded-md bg-surface-elevated" />
      </div>
    </div>
  );
}

function EmptyState() {
  return (
    <div className="flex flex-col items-center gap-2 px-4 py-10 text-center">
      <Sparkles size={18} className="text-stone" />
      <p className="text-[13px] text-on-dark">
        아직 MCP로 저장된 콘텐츠가 없어요.
      </p>
      <p className="max-w-[420px] text-[12px] text-stone">
        Claude Code 또는 Codex가 <code className="rounded-xs bg-surface-elevated px-1 py-0.5 font-mono text-[11px]">danbi_log</code>·
        <code className="rounded-xs bg-surface-elevated px-1 py-0.5 font-mono text-[11px]">danbi_append</code>·
        <code className="rounded-xs bg-surface-elevated px-1 py-0.5 font-mono text-[11px]">danbi_create_file</code>{" "}
        같은 도구로 vault에 쓰기 시작하면 여기에 누적돼요.
      </p>
    </div>
  );
}

function DisclaimerBanner({ text }: { text?: string }) {
  return (
    <div className="border-t border-hairline bg-surface-elevated/40 px-4 py-2.5">
      <div className="flex items-start gap-2 text-[11px] leading-relaxed text-stone">
        <Info size={11} className="mt-0.5 shrink-0 text-stone" />
        <span>
          {text ??
            "이 숫자는 단비에 저장된 콘텐츠의 추정 토큰입니다. Claude Code / Codex의 실제 LLM 청구액과는 다릅니다."}
        </span>
      </div>
    </div>
  );
}

// ---------- formatting helpers --------------------------------------

function fmtTokens(n: number): string {
  if (n < 1_000) return n.toLocaleString();
  if (n < 1_000_000) return `${(n / 1_000).toFixed(1).replace(/\.0$/, "")}k`;
  if (n < 1_000_000_000) return `${(n / 1_000_000).toFixed(2).replace(/\.00$/, "")}M`;
  return `${(n / 1_000_000_000).toFixed(2).replace(/\.00$/, "")}B`;
}

function prettyModel(stem: string): string {
  switch (stem) {
    case "claude-sonnet-4-6":
      return "Sonnet 4.6";
    case "claude-opus-4-7":
      return "Opus 4.7";
    case "claude-haiku-4-5":
      return "Haiku 4.5";
    default:
      return stem;
  }
}

function clientLabel(c: string): string {
  switch (c) {
    case "claude_code":
      return "Claude Code";
    case "codex":
      return "Codex";
    case "cursor":
      return "Cursor";
    case "continue":
      return "Continue";
    case "unknown":
      return "기타";
    default:
      return c;
  }
}
