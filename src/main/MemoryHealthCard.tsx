import { useEffect, useState, type ReactNode } from "react";
import { Gauge } from "lucide-react";
import { ipc, type ProjectGrowth } from "@/lib/ipc";
import { cn } from "@/lib/utils";
import { useApp } from "@/state/store";

/** 기록 비대화 감시 — "많이 쓴다고 좋은 게 아니다". 에이전트가 MCP 로
 *  쌓는 기록이 불어나는지 한눈에 보고 손쓸 수 있게. LLM 없이 파일·링크
 *  색인·git 만으로 계산(`growth.rs`). 프로젝트 홈의 작은 카드와 전체 홈의
 *  "불어나는 프로젝트" 한 줄 두 가지. */

export function fmtChars(n: number): string {
  const a = Math.abs(n);
  let s: string;
  if (a >= 10_000) {
    const man = a / 10_000;
    s = man >= 10 ? `${Math.round(man)}만` : `${man.toFixed(1)}만`;
  } else if (a >= 1_000) {
    s = `${(a / 1_000).toFixed(1)}천`;
  } else {
    s = String(a);
  }
  return n < 0 ? `-${s}` : s;
}

const pct = (r: number) => `${Math.round(r * 100)}%`;

function levelLabel(level: ProjectGrowth["level"]): string {
  if (level === "bloating") return "불어나는 중";
  if (level === "watch") return "지켜볼 것";
  return "괜찮아요";
}

/** 신호 점 — chrome 에 포화색을 쓰지 않는 원칙에 따라 점 하나만 색을 가진다. */
function LevelDot({ level }: { level: ProjectGrowth["level"] }) {
  return (
    <span
      className={cn(
        "inline-block h-1.5 w-1.5 shrink-0 rounded-full",
        level === "bloating"
          ? "bg-accent-red"
          : level === "watch"
            ? "bg-accent-yellow"
            : "bg-accent-green",
      )}
    />
  );
}

/** 프로젝트 홈 카드. 문서 수·글자 수·daily 비중·고아 비율·한 달 증가(누가
 *  썼는지)와 신호 최대 2개(쉬운 말 + 무엇을 하면 되는지). */
export function MemoryHealthCard({
  project,
  onOpenDoc,
}: {
  project: string;
  onOpenDoc?: (domain: string) => void;
}) {
  const tree = useApp((s) => s.tree);
  const [g, setG] = useState<ProjectGrowth | null | undefined>(undefined);

  useEffect(() => {
    let alive = true;
    ipc
      .projectGrowth(project)
      .then((r) => alive && setG(r))
      .catch(() => alive && setG(null));
    return () => {
      alive = false;
    };
    // tree 가 바뀌면(파일 추가·수정) 다시 — 백엔드 캐시가 fingerprint 로
    // 같은 결과를 바로 돌려주니 비용은 없다.
  }, [project, tree]);

  if (g === undefined) {
    return (
      <section className="rounded-lg border border-hairline bg-surface px-4 py-3">
        <CardTitle />
        <div className="text-[12px] text-mute">…</div>
      </section>
    );
  }
  if (g === null) return null;

  const writes = g.month.writes.agent + g.month.writes.human + g.month.writes.danbi;
  const whoWrote =
    g.month.source !== "git" || writes === 0
      ? null
      : g.month.writes.agent === writes
        ? "모두 에이전트"
        : g.month.writes.human === writes
          ? "모두 사람"
          : `에이전트 ${pct(g.month.writes.agent / writes)}`;
  const big = g.largest[0];

  return (
    <section className="rounded-lg border border-hairline bg-surface px-4 py-3">
      <CardTitle right={
        <span className="flex items-center gap-1.5 text-[11px] text-mute">
          <LevelDot level={g.level} />
          {levelLabel(g.level)}
        </span>
      } />
      {/* 프로젝트 홈 오른쪽 열이 좁아 두 칸이면 보조 글자가 잘린다 — 한 줄에 하나. */}
      <dl className="flex flex-col gap-1 text-[12px]">
        <Row k="문서" v={`${g.docs}개`} sub={g.daily_docs > 0 ? `daily ${g.daily_docs}` : undefined} />
        <Row k="글자" v={`${fmtChars(g.chars)}자`} sub={g.bytes > 0 ? `daily ${pct(g.daily_share)}` : undefined} />
        <Row
          k="고아"
          v={g.orphan_candidates > 0 ? `${g.orphans}/${g.orphan_candidates}` : "—"}
          sub={g.orphan_candidates > 0 ? pct(g.orphan_ratio) : undefined}
          title="어디서도 링크되지 않은 문서 (daily·뼈대 문서 제외)"
        />
        <Row
          k="한 달"
          v={`${g.month.chars_added >= 0 ? "+" : ""}${fmtChars(g.month.chars_added)}자`}
          sub={whoWrote ?? undefined}
          title={`최근 30일 순증 · 문서 +${g.month.docs_added}${g.month.docs_removed ? ` −${g.month.docs_removed}` : ""} · 7일 ${g.week.chars_added >= 0 ? "+" : ""}${fmtChars(g.week.chars_added)}자`}
        />
      </dl>
      {big && big.bytes > 0 && (
        <div className="mt-2 flex items-center justify-between gap-2 text-[11px] text-mute">
          <span className="shrink-0">가장 큰 문서</span>
          <button
            onClick={() => onOpenDoc?.(big.domain)}
            title={big.domain}
            className="min-w-0 truncate text-body hover:text-on-dark"
          >
            {big.domain}
            <span className="ml-1 font-mono text-[10px] text-stone">{fmtChars(big.chars)}자</span>
          </button>
        </div>
      )}
      {g.signals.length > 0 && (
        <ul className="mt-2.5 flex flex-col gap-1.5 border-t border-hairline pt-2.5">
          {g.signals.slice(0, 2).map((s) => (
            <li key={s.code} className="text-[12px] leading-[1.45]">
              <div className="text-body">{s.message}</div>
              <div className="text-mute">→ {s.action}</div>
            </li>
          ))}
          {g.signals.length > 2 && (
            <li className="text-[11px] text-stone">외 {g.signals.length - 2}건</li>
          )}
        </ul>
      )}
    </section>
  );
}

function CardTitle({ right }: { right?: ReactNode }) {
  return (
    <div className="mb-2 flex items-center justify-between">
      <span className="flex items-center gap-1.5 text-caption-sm uppercase tracking-[0.4px] text-mute">
        <Gauge size={11} className="text-stone" /> 기록 건강
      </span>
      {right}
    </div>
  );
}

function Row({
  k,
  v,
  sub,
  title,
}: {
  k: string;
  v: string;
  sub?: string;
  title?: string;
}) {
  return (
    <div className="flex min-w-0 items-baseline justify-between gap-2" title={title}>
      <dt className="shrink-0 text-mute">{k}</dt>
      <dd className="min-w-0 truncate text-right tabular-nums text-body">
        {v}
        {sub && <span className="ml-1 text-[11px] text-stone">{sub}</span>}
      </dd>
    </div>
  );
}

/** 전체 홈 — 불어나는 프로젝트만 짧게. 없으면 아무것도 그리지 않는다. */
export function GrowingProjectsCard({
  onOpen,
  max = 5,
}: {
  onOpen: (project: string) => void;
  max?: number;
}) {
  const tree = useApp((s) => s.tree);
  const [rows, setRows] = useState<ProjectGrowth[]>([]);
  useEffect(() => {
    let alive = true;
    ipc
      .growthOverview()
      .then((all) => alive && setRows(all.filter((r) => r.level === "bloating")))
      .catch(() => alive && setRows([]));
    return () => {
      alive = false;
    };
  }, [tree]);
  if (rows.length === 0) return null;
  return (
    <section className="rounded-lg border border-hairline bg-surface px-4 py-3">
      <div className="mb-2 flex items-center justify-between">
        <span className="flex items-center gap-1.5 text-caption-sm uppercase tracking-[0.4px] text-mute">
          <Gauge size={11} className="text-stone" /> 불어나는 프로젝트
        </span>
        <span className="text-[11px] text-stone">{rows.length}개</span>
      </div>
      <ul className="flex flex-col gap-1.5">
        {rows.slice(0, max).map((r) => {
          const first = r.signals[0];
          return (
            <li key={r.project}>
              <button
                onClick={() => onOpen(r.project)}
                className="flex w-full min-w-0 flex-col items-start text-left"
                title={first ? `${first.message} → ${first.action}` : undefined}
              >
                <span className="flex w-full items-center justify-between gap-2 text-[12px]">
                  <span className="truncate text-body">{r.project}</span>
                  <span className="shrink-0 font-mono text-[10px] text-stone">
                    {fmtChars(r.chars)}자
                  </span>
                </span>
                {first && (
                  <span className="w-full truncate text-[11px] text-mute">{first.message}</span>
                )}
              </button>
            </li>
          );
        })}
        {rows.length > max && (
          <li className="text-[11px] text-stone">외 {rows.length - max}개</li>
        )}
      </ul>
    </section>
  );
}
