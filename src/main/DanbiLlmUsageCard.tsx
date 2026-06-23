import { useEffect, useState } from "react";
import { Cpu, Sparkles } from "lucide-react";
import { ipc, type CcRange, type UsageSummary } from "@/lib/ipc";
import { cn } from "@/lib/utils";

/**
 * 단비 자체 LLM 호출 사용량 (Phase 5).
 *
 * 단비 본체가 라우팅/writer/embed 등으로 Bedrock·기타 provider 에
 * 직접 쏜 토큰을 보여준다. Claude Code 카드가 "사용자가 Claude Code 로
 * 쓴 비용", 이 카드가 "단비 앱 자신이 쓴 비용". 두 카드는 정확히
 * 다른 데이터 소스 (jsonl vs usage.jsonl) 를 본다.
 */
export function DanbiLlmUsageCard() {
  const [range, setRange] = useState<CcRange>("today");
  const [data, setData] = useState<UsageSummary | null>(null);
  const [loading, setLoading] = useState(false);

  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    ipc
      .dashboardDanbiLlm(range)
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

  const empty = !loading && (data == null || data.calls === 0);
  const totalIn = data?.by_role.reduce((s, r) => s + r.input_tokens, 0) ?? 0;
  const totalOut = data?.by_role.reduce((s, r) => s + r.output_tokens, 0) ?? 0;

  return (
    <section className="mt-7 overflow-hidden rounded-lg border border-hairline bg-surface">
      <header className="flex items-center justify-between gap-3 border-b border-hairline px-4 py-2.5">
        <div className="flex items-center gap-1.5">
          <Sparkles size={13} className="text-accent-blue" />
          <h2 className="text-[14px] font-medium text-ink">
            단비 자체 LLM 사용량
          </h2>
          <span className="ml-1 rounded-xs border border-hairline bg-surface-elevated px-1.5 py-0.5 font-mono text-[10px] uppercase tracking-wider text-stone">
            정확
          </span>
        </div>
        <RangeToggle range={range} onChange={setRange} />
      </header>

      {loading && !data ? (
        <Skeleton />
      ) : empty ? (
        <Empty />
      ) : (
        <div className="px-4 py-4">
          <div className="flex flex-wrap items-baseline gap-3">
            <span className="font-mono text-[36px] font-medium leading-none tracking-tight text-ink tabular-nums">
              ₩{Math.round(data?.total_krw ?? 0).toLocaleString()}
            </span>
            <span className="text-[12px] text-stone">
              KRW · ${(((data?.total_krw ?? 0) / (data?.krw_per_usd ?? 1380))).toFixed(2)}
            </span>
            <span className="ml-auto text-[12px] text-stone">
              {(data?.calls ?? 0).toLocaleString()} calls
            </span>
          </div>

          <div className="mt-3 grid grid-cols-2 gap-2">
            <Stat label="input" value={totalIn} />
            <Stat label="output" value={totalOut} />
          </div>

          <div className="mt-4">
            <div className="mb-2 flex items-center gap-1.5 text-[12px] text-stone">
              <Cpu size={11} className="text-accent-green" />
              <span>역할별</span>
            </div>
            <ul className="divide-y divide-hairline rounded-md border border-hairline bg-surface-elevated">
              {data!.by_role.map((r) => {
                const totalT = r.input_tokens + r.output_tokens;
                return (
                  <li
                    key={r.role}
                    className="flex items-baseline gap-2 px-3 py-2 text-[12px]"
                  >
                    <span className="min-w-0 flex-1 truncate text-on-dark">
                      <span>{r.role}</span>
                      {r.top_model && (
                        <span className="ml-2 truncate font-mono text-[10px] text-stone">
                          {prettyModel(r.top_model)}
                        </span>
                      )}
                    </span>
                    <span className="shrink-0 font-mono text-[11px] tabular-nums text-on-dark">
                      {fmtTokens(totalT)}
                    </span>
                    <span className="shrink-0 font-mono text-[11px] tabular-nums text-stone">
                      ₩{Math.round(r.krw).toLocaleString()}
                    </span>
                  </li>
                );
              })}
            </ul>
          </div>

          <p className="mt-4 text-[11px] leading-snug text-stone">
            단비 본 앱이 Bedrock·기타 provider 에 쏜 호출의 토큰입니다.
            응답 헤더의 usage 필드 → 단가표 매칭 → KRW 환산. 비용은 실제
            청구액에 매우 가깝습니다.
          </p>
        </div>
      )}
    </section>
  );
}

function Stat({ label, value }: { label: string; value: number }) {
  return (
    <div className="rounded-md border border-hairline bg-surface-elevated px-3 py-2">
      <div className="text-[10px] uppercase tracking-wider text-stone">
        {label}
      </div>
      <div className="mt-0.5 font-mono text-[16px] font-medium tabular-nums text-on-dark">
        {fmtTokens(value)}
      </div>
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

function Skeleton() {
  return (
    <div className="px-4 py-5">
      <div className="h-9 w-32 animate-pulse rounded bg-surface-elevated" />
      <div className="mt-3 grid grid-cols-2 gap-2">
        {Array.from({ length: 2 }).map((_, i) => (
          <div
            key={i}
            className="h-12 animate-pulse rounded-md bg-surface-elevated"
          />
        ))}
      </div>
    </div>
  );
}

function Empty() {
  return (
    <div className="px-4 py-10 text-center">
      <p className="text-[13px] text-on-dark">아직 호출 기록이 없어요.</p>
      <p className="mt-1 text-[11px] text-stone">
        단비 라우팅/writer/embed 가 처음 호출되면 자동으로 잡힙니다.
      </p>
    </div>
  );
}

function fmtTokens(n: number): string {
  if (n >= 1_000_000_000) return `${(n / 1_000_000_000).toFixed(2)}B`;
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(2)}M`;
  if (n >= 1_000) return `${(n / 1_000).toFixed(1)}k`;
  return n.toLocaleString();
}

function prettyModel(id: string): string {
  const stems = [
    "claude-opus-4-7",
    "claude-opus-4-6",
    "claude-sonnet-4-6",
    "claude-sonnet-4-5",
    "claude-haiku-4-5",
    "claude-haiku-3-5",
    "titan-embed-text-v2",
    "titan-embed-text-v1",
    "voyage-",
    "gpt-4o",
    "gpt-4.1",
    "gemini-",
  ];
  const lower = id.toLowerCase();
  for (const s of stems) if (lower.includes(s)) return s;
  return id;
}
