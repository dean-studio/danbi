import { useEffect, useMemo, useRef, useState } from "react";
import { ipc, type WarmupProgress, type WarmupTask } from "@/lib/ipc";
import { useProjectColor } from "@/lib/projectColor";
import { cn } from "@/lib/utils";
import { useApp } from "@/state/store";

/** 첫 로딩 "준비 중" 패널.
 *
 *  앱 프로세스가 새로 뜬 뒤 처음 한 번만 오래 걸리는 곳(Claude Code 대화 기록
 *  파싱, 검색 색인 빌드)에서 회색 막대만 보여 주면 멈춘 것처럼 보인다. 여기선
 *  Rust `warmup.rs` 가 남기는 **실제 진행률**(파일 n / 총 m, 단계 이름)을
 *  300ms 마다 읽어 단계 문구·막대·경과 시간으로 보여 준다. Rust 가 아직
 *  진행률을 남기기 전이거나 다른 구간이 느릴 땐 경과 시간 기반 문구로 대신한다.
 *
 *  - 600ms 안에 끝나면 아예 안 보인다(번쩍임 방지).
 *  - 결과가 오면 `revealClass` 를 결과 컨테이너에 붙여 부드럽게 전환.
 *  - 애니메이션은 전부 CSS(`.danbi-warm-*`) — `prefers-reduced-motion` 은 CSS 에서 끈다.
 *  - 색: 토큰 + 프로젝트 색(일러스트성). 포화 accent 를 chrome 에 쓰지 않는다. */

const SHOW_DELAY_MS = 600;
const POLL_MS = 300;
const HINT_AFTER_MS = 2500;
/** 진행률 없이 시간으로만 문구를 돌릴 때 한 문구가 머무는 시간 */
const FALLBACK_STEP_MS = 4000;

type TaskCopy = {
  /** Rust phase 이름 → 문구 */
  phases: Record<string, string>;
  /** Rust 쪽 작업은 끝났는데 결과가 아직 안 온 동안 */
  after: string;
  /** 진행률을 한 번도 못 봤을 때 경과 시간으로 돌리는 문구 */
  fallback: string[];
  hint: string;
};

const COPY: Record<WarmupTask, TaskCopy> = {
  usage: {
    phases: {
      listing: "대화 기록 파일 찾는 중…",
      parsing: "대화 기록 읽는 중…",
      merging: "시간순으로 합치는 중…",
    },
    after: "프로젝트별로 나누는 중…",
    fallback: ["준비하는 중…", "대화 기록 살펴보는 중…", "프로젝트별로 나누는 중…"],
    hint: "처음 한 번만 오래 걸려요 — 다음부터는 바로 떠요.",
  },
  search: {
    phases: {
      scanning: "문서 목록 훑는 중…",
      indexing: "검색 색인 만드는 중…",
      committing: "색인 마무리하는 중…",
    },
    after: "비슷한 기록 고르는 중…",
    fallback: ["준비하는 중…", "검색 색인 확인하는 중…", "비슷한 기록 고르는 중…"],
    hint: "색인은 처음 한 번만 만들어요 — 다음부터는 바로 떠요.",
  },
};

export type WarmupState = {
  task: WarmupTask;
  /** 600ms 지연 뒤 true — 이때만 패널을 그린다 */
  visible: boolean;
  label: string;
  /** 총량을 아는 단계면 0~1, 아니면 null */
  ratio: number | null;
  done: number;
  total: number;
  /** 총량 아는 단계를 한 번이라도 지났고 지금은 마무리 중 */
  settling: boolean;
  elapsedMs: number;
  showHint: boolean;
  /** 로딩이 끝난 뒤 결과 컨테이너에 붙이는 전환 클래스. 패널을 안 보여 줬으면 빈 문자열 */
  revealClass: string;
};

export function useWarmup(task: WarmupTask, loading: boolean): WarmupState {
  const [now, setNow] = useState(0);
  const [visible, setVisible] = useState(false);
  const [progress, setProgress] = useState<WarmupProgress | null>(null);
  const startedAt = useRef(0);
  const sawActive = useRef(false);
  const sawTotal = useRef(false);
  const showed = useRef(false);

  // 로딩 시작/끝 — 시계·지연 노출·진행률 조회를 한 effect 에서 관리한다.
  useEffect(() => {
    if (!loading) {
      setVisible(false);
      setProgress(null);
      return;
    }
    startedAt.current = Date.now();
    sawActive.current = false;
    sawTotal.current = false;
    showed.current = false;
    setNow(startedAt.current);
    setProgress(null);
    setVisible(false);

    const show = window.setTimeout(() => {
      showed.current = true;
      setVisible(true);
    }, SHOW_DELAY_MS);
    const tick = window.setInterval(() => setNow(Date.now()), 250);

    let alive = true;
    let inFlight = false;
    const poll = async () => {
      if (inFlight) return;
      inFlight = true;
      try {
        const p = await ipc.warmupProgress(task);
        if (!alive) return;
        if (p?.active) {
          sawActive.current = true;
          if (p.total > 0) sawTotal.current = true;
        }
        setProgress(p);
      } catch {
        /* 진행률은 보조 정보 — 실패해도 시간 기반 문구로 간다 */
      } finally {
        inFlight = false;
      }
    };
    poll();
    const pollId = window.setInterval(poll, POLL_MS);

    return () => {
      alive = false;
      window.clearTimeout(show);
      window.clearInterval(tick);
      window.clearInterval(pollId);
    };
  }, [task, loading]);

  const elapsedMs = loading ? Math.max(0, now - startedAt.current) : 0;
  const copy = COPY[task];

  // 진행률이 "이번 로딩" 것인지 가려낸다 — Rust 는 지난 실행의 마지막 값을
  // (active=false 로) 남겨 두므로, 이번에 active 를 본 뒤에만 믿는다.
  const live = progress && progress.active ? progress : null;
  const finishedThisRun = !live && sawActive.current;

  let label: string;
  let ratio: number | null = null;
  let done = 0;
  let total = 0;
  if (live) {
    label = copy.phases[live.phase] ?? copy.fallback[0];
    if (live.total > 0) {
      done = live.done;
      total = live.total;
      ratio = Math.min(1, done / total);
    }
  } else if (finishedThisRun) {
    label = copy.after;
  } else {
    const i = Math.min(copy.fallback.length - 1, Math.floor(elapsedMs / FALLBACK_STEP_MS));
    label = copy.fallback[i];
  }
  const settling = ratio === null && sawTotal.current;

  return {
    task,
    visible: loading && visible,
    label,
    ratio,
    done,
    total,
    settling,
    elapsedMs,
    showHint: elapsedMs >= HINT_AFTER_MS,
    revealClass: !loading && showed.current ? "danbi-warm-reveal" : "",
  };
}

/** 패널 막대·점에 쓸 프로젝트 색 — 규모 순 상위 몇 개(일러스트성 용도). */
export function useWarmupAccents(limit = 4, exclude?: string): string[] {
  const tree = useApp((s) => s.tree);
  const projectColor = useProjectColor();
  return useMemo(() => {
    const names = (tree?.projects ?? []).map((p) => p.name).filter((n) => n !== exclude);
    const picked = names.slice(0, limit);
    return picked.length ? picked.map(projectColor) : [];
  }, [tree, projectColor, limit, exclude]);
}

function formatElapsed(ms: number): string {
  const s = Math.floor(ms / 1000);
  if (s < 60) return `${s}초`;
  const m = Math.floor(s / 60);
  const r = s % 60;
  return `${m}분 ${String(r).padStart(2, "0")}초`;
}

/** 유령 행 폭 — 매번 같은 패턴이면 기계적이라 살짝 다르게 */
const GHOST_WIDTHS = ["62%", "48%", "71%", "55%", "66%", "44%"];

export function WarmupPanel({
  state,
  rows = 3,
  accents,
  className,
  rowHeight = "h-5",
}: {
  state: WarmupState;
  /** 결과가 차지할 줄 수와 비슷하게 — 로딩 전후 높이가 덜 출렁이게 */
  rows?: number;
  /** 막대·점 색. 없으면 중립(hairline) */
  accents?: string[];
  className?: string;
  rowHeight?: string;
}) {
  const colors = accents && accents.length ? accents : [];
  const fill = useMemo(() => {
    if (colors.length === 0) return "var(--color-hairline-strong)";
    if (colors.length === 1) return colors[0];
    return `linear-gradient(90deg, ${colors.join(", ")})`;
  }, [colors]);
  const pct = state.ratio === null ? null : Math.round(state.ratio * 100);

  return (
    <div
      className={cn("danbi-warm flex flex-col gap-2", className)}
      role="status"
      aria-live="polite"
      aria-busy="true"
      aria-label={`${state.label} ${state.total > 0 ? `${state.done} / ${state.total}` : ""}`}
    >
      {/* 진행 막대 — 결과 화면의 분포 막대와 같은 자리·같은 모양 */}
      <div className="relative h-2 overflow-hidden rounded-full bg-surface-elevated">
        {pct !== null ? (
          <div
            className="relative h-full overflow-hidden rounded-full transition-[width] duration-300 ease-out"
            style={{ width: `${Math.max(2, pct)}%`, background: fill }}
          >
            <span
              className="danbi-warm-gloss absolute inset-y-0 left-0 w-1/2"
              style={{
                background:
                  "linear-gradient(90deg, transparent, rgba(255,255,255,0.35), transparent)",
              }}
            />
          </div>
        ) : state.settling ? (
          <div className="danbi-warm-settle h-full w-full rounded-full" style={{ background: fill }} />
        ) : (
          <span
            className="danbi-warm-sweep absolute inset-y-0 left-0 w-1/4 rounded-full"
            style={{ background: fill }}
          />
        )}
      </div>

      {/* 단계 문구 · 개수 · 경과 시간 */}
      <div className="flex items-baseline justify-between gap-2 text-[12px]">
        <span key={state.label} className="danbi-warm-in min-w-0 truncate text-body">
          {state.label}
        </span>
        <span className="shrink-0 font-mono text-[11px] tabular-nums text-stone">
          {state.total > 0 && (
            <>
              {state.done.toLocaleString()}
              <span className="text-mute"> / </span>
              {state.total.toLocaleString()}
              <span className="text-mute"> · </span>
            </>
          )}
          {formatElapsed(state.elapsedMs)}
        </span>
      </div>

      {/* 유령 행 — 결과 줄 자리에서 숨 쉬는 점 + 흐르는 글줄 */}
      <ul className="flex flex-col">
        {Array.from({ length: rows }).map((_, i) => (
          <li key={i} className={cn("flex items-center gap-2", rowHeight)}>
            <span
              className="danbi-warm-breathe h-1.5 w-1.5 shrink-0 rounded-full"
              style={{
                background: colors.length ? colors[i % colors.length] : "var(--color-hairline-strong)",
                animationDelay: `${i * 180}ms`,
              }}
            />
            <span
              className="danbi-warm-shimmer h-2 rounded-full"
              style={{ width: GHOST_WIDTHS[i % GHOST_WIDTHS.length], animationDelay: `${i * 120}ms` }}
            />
            <span className="ml-auto h-2 w-7 rounded-full bg-surface-elevated" />
          </li>
        ))}
      </ul>

      {/* 안내 — 자리는 늘 잡아 두고 2.5초 뒤 서서히 */}
      <div
        className={cn(
          "text-[11px] leading-[1.5] text-stone transition-opacity duration-500",
          state.showHint ? "opacity-100" : "opacity-0",
        )}
        aria-hidden={!state.showHint}
      >
        {COPY[state.task].hint}
      </div>
    </div>
  );
}
