import { useEffect, useState } from "react";
import { Bot, Code2, Coins, FileText, Info, Sparkles } from "lucide-react";
import {
  ipc,
  type McpInboundRange,
  type McpProjectDetail,
} from "@/lib/ipc";
import { cn } from "@/lib/utils";

/**
 * Per-project MCP inbound mini card.
 *
 * Shows the same metric family as the global `McpInboundCard` but
 * scoped to one project — sits in `ProjectHome`. Lighter-weight than
 * the home card: no anomaly callout, no heatmap, no top contributors,
 * because at the project level the user is already drilling in.
 *
 * The disclaimer banner is identical and permanent — the metric is
 * still an estimate, scoping it doesn't change that.
 */
export function McpInboundProjectMini({
  project,
  onOpenDomain,
}: {
  project: string;
  onOpenDomain?: (domain: string) => void;
}) {
  const [range, setRange] = useState<McpInboundRange>("7d");
  const [data, setData] = useState<McpProjectDetail | null>(null);
  const [loading, setLoading] = useState(false);

  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    ipc
      .dashboardMcpInboundProject(project, range)
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
  }, [project, range]);

  const empty = !loading && (!data || data.total_calls === 0);

  return (
    <section className="overflow-hidden rounded-lg border border-hairline bg-surface">
      <header className="flex items-center justify-between gap-3 border-b border-hairline px-4 py-2.5">
        <div className="flex items-center gap-1.5">
          <Sparkles size={13} className="text-accent-yellow" />
          <h2 className="text-[14px] font-medium text-ink">MCP 저장 토큰</h2>
          <span className="ml-1 rounded-xs border border-hairline bg-surface-elevated px-1.5 py-0.5 font-mono text-[10px] uppercase tracking-wider text-stone">
            추정
          </span>
        </div>
        <RangeToggle range={range} onChange={setRange} />
      </header>

      {empty ? (
        <div className="flex flex-col items-center gap-1 px-4 py-6 text-center">
          <Sparkles size={16} className="text-stone" />
          <p className="text-[12px] text-on-dark">
            이 프로젝트에 저장된 콘텐츠가 없어요.
          </p>
          <p className="max-w-[420px] text-[11px] text-stone">
            Claude Code / Codex 가 이 프로젝트에 쓰기 시작하면 여기에 누적돼요.
          </p>
        </div>
      ) : (
        <div className="px-4 py-4">
          <div className="flex items-baseline gap-3">
            <span className="font-mono text-[28px] font-medium leading-none tracking-tight text-ink tabular-nums">
              {fmtTokens(data?.total_tokens ?? 0)}
            </span>
            <span className="text-[12px] text-stone">tokens</span>
            <span className="ml-auto text-[12px] text-stone">
              {(data?.total_calls ?? 0).toLocaleString()} calls
            </span>
          </div>

          {data?.cost_estimate && (
            <div className="mt-1.5 flex items-center gap-1.5 text-[11px] text-stone">
              <Coins size={11} className="text-stone" />
              <span>
                재투입 시 약{" "}
                <strong className="font-mono font-medium text-on-dark">
                  ₩{Math.round(data.cost_estimate.krw).toLocaleString()}
                </strong>
              </span>
              <span className="rounded-xs border border-hairline bg-surface-elevated px-1.5 py-0.5 font-mono text-[10px] uppercase tracking-wider text-stone">
                참고용
              </span>
            </div>
          )}

          <div className="mt-4 grid grid-cols-2 gap-3">
            <Mini
              title="에이전트별"
              icon={<Bot size={11} className="text-accent-blue" />}
              rows={(data?.by_client ?? []).map((c) => ({
                label: clientLabel(c.client),
                tokens: c.tokens,
              }))}
            />
            <Mini
              title="도구별"
              icon={<Code2 size={11} className="text-accent-green" />}
              rows={(data?.by_tool ?? []).map((t) => ({
                label: t.tool,
                tokens: t.tokens,
              }))}
            />
          </div>

          {(data?.by_domain?.length ?? 0) > 0 && (
            <div className="mt-4">
              <div className="mb-2 flex items-center gap-1.5 text-[12px] text-stone">
                <FileText size={11} className="text-stone" />
                <span>도메인별 Top {Math.min(5, data!.by_domain.length)}</span>
              </div>
              <ul className="divide-y divide-hairline rounded-md border border-hairline bg-surface-elevated">
                {data!.by_domain.slice(0, 5).map((d) => {
                  const pct =
                    (data?.total_tokens ?? 0) > 0
                      ? (d.tokens / (data?.total_tokens ?? 1)) * 100
                      : 0;
                  return (
                    <li
                      key={d.domain}
                      className="flex items-center gap-3 px-3 py-2 text-[12px]"
                    >
                      <button
                        type="button"
                        onClick={() => onOpenDomain?.(d.domain)}
                        className={cn(
                          "min-w-0 flex-1 truncate text-left font-mono text-[11px] text-on-dark",
                          onOpenDomain && "hover:underline",
                        )}
                      >
                        {d.domain}
                      </button>
                      <div className="hidden h-1 w-20 shrink-0 overflow-hidden rounded-full bg-surface md:block">
                        <div
                          className="h-full bg-accent-yellow/70"
                          style={{ width: `${Math.min(100, pct)}%` }}
                        />
                      </div>
                      <span className="shrink-0 font-mono tabular-nums text-on-dark">
                        {fmtTokens(d.tokens)}
                      </span>
                      <span className="shrink-0 font-mono text-[11px] text-stone tabular-nums">
                        {d.calls.toLocaleString()}
                      </span>
                    </li>
                  );
                })}
              </ul>
            </div>
          )}
        </div>
      )}

      <div className="border-t border-hairline bg-surface-elevated/40 px-4 py-2.5">
        <div className="flex items-start gap-2 text-[11px] leading-relaxed text-stone">
          <Info size={11} className="mt-0.5 shrink-0 text-stone" />
          <span>
            {data?.disclaimer ??
              "이 숫자는 단비에 저장된 콘텐츠의 추정 토큰입니다. Claude Code / Codex의 실제 LLM 청구액과는 다릅니다."}
          </span>
        </div>
      </div>
    </section>
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

function Mini({
  title,
  icon,
  rows,
}: {
  title: string;
  icon: React.ReactNode;
  rows: { label: string; tokens: number }[];
}) {
  return (
    <div className="rounded-md border border-hairline bg-surface-elevated p-3">
      <div className="mb-2 flex items-center gap-1.5 text-[11px] uppercase tracking-wider text-stone">
        {icon}
        <span>{title}</span>
      </div>
      <ul className="space-y-1">
        {rows.length === 0 ? (
          <li className="text-[12px] text-stone">—</li>
        ) : (
          rows.slice(0, 4).map((r) => (
            <li key={r.label} className="flex items-baseline justify-between gap-2 text-[12px]">
              <span className="truncate text-on-dark">{r.label}</span>
              <span className="shrink-0 font-mono tabular-nums text-on-dark">
                {fmtTokens(r.tokens)}
              </span>
            </li>
          ))
        )}
      </ul>
    </div>
  );
}

function fmtTokens(n: number): string {
  if (n < 1_000) return n.toLocaleString();
  if (n < 1_000_000) return `${(n / 1_000).toFixed(1).replace(/\.0$/, "")}k`;
  if (n < 1_000_000_000) return `${(n / 1_000_000).toFixed(2).replace(/\.00$/, "")}M`;
  return `${(n / 1_000_000_000).toFixed(2).replace(/\.00$/, "")}B`;
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
