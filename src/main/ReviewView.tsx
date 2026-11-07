import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Check, Inbox, RefreshCw, Sparkles } from "lucide-react";
import {
  ipc,
  onReviewsChanged,
  onVaultChanged,
  type DanbiConfig,
  type DashboardSnapshot,
  type DreamAction,
  type DreamChoice,
  type GhostSuggestion,
  type ReviewItem,
  type VaultSuggestion,
} from "@/lib/ipc";
import { relationLabel } from "@/lib/ontology";
import { cn } from "@/lib/utils";
import { useApp } from "@/state/store";

/**
 * 검토함 — ghost 제안 · self-healing(고아/빈 파일/큰 파일) · 저신뢰 라우팅을
 * 한 큐로 보여주는 메인 화면. 세 소스(`dashboardSnapshot`·`reviewsList`)
 * 를 읽기만 하고, 새 저장소를 만들지 않는다(리뷰 큐에 ghost/healing 을
 * 복제하지 않음). 고아 "제외" 만 `cfg.review_excluded_paths` 에 남는다.
 */

export type Filter =
  | "all"
  | "dream"
  | "ghost"
  | "orphan"
  | "empty"
  | "large"
  | "ontology"
  | "review";

const FILTERS: Array<{ key: Filter; label: string }> = [
  { key: "all", label: "전체" },
  { key: "dream", label: "정리" },
  { key: "ghost", label: "GHOST" },
  { key: "orphan", label: "고아" },
  { key: "empty", label: "빈 파일" },
  { key: "large", label: "큰 파일" },
  { key: "ontology", label: "온톨로지" },
  { key: "review", label: "저신뢰" },
];

/** 뼈대·색인 문서는 참조가 없어도 고아가 아니다 (서버의 is_index_like 와
 *  같은 취지의 클라이언트 보강). */
const INDEX_LIKE = new Set([
  "purpose.md",
  "schema.md",
  "SKILL.md",
  "MEMORY_index.md",
  "README.md",
]);

/** 고아 목록은 최근 수정 우선, 이 개수 뒤부터는 접는다. */
const ORPHAN_VISIBLE = 20;

/** 큐의 한 행. 세 소스를 같은 모양으로 정규화해 키보드·필터를 한 코드로. */
export type ReviewRow = {
  key: string;
  filter: Exclude<Filter, "all">;
  badge: string;
  badgeTone: string;
  project: string;
  domain: string | null;
  reason: string;
  /** 정렬용 — 고아는 문서 mtime, ghost·리뷰는 생성 시각. */
  ts: number;
  /** 승인/제외 동작의 라벨. null 이면 열기·무시만. */
  approveLabel: string | null;
  /** 이유 앞에 붙는 작은 뱃지 (ghost 제안의 관계 이름 등). */
  tag?: string;
  ghost?: GhostSuggestion;
  review?: ReviewItem;
  /** 쉬는 시간 정리 항목 — 선택지가 여럿이라 `choices` 로 그린다. */
  dream?: DreamAction;
  /** 정리 항목의 선택지. 첫 번째가 `approve`(A 키)와 같다. */
  choices?: DreamChoiceButton[];
};
type Row = ReviewRow;

export type DreamChoiceButton = {
  label: string;
  choice: DreamChoice;
  /** 파일을 바꾸는 선택인지 (버튼 톤). */
  writes: boolean;
  hint?: string;
};

/** 표시 순서: 정리 → ghost → 저신뢰 → 온톨로지 → 큰 파일 → 빈 파일 → 고아(최근 수정 우선). */
const FILTER_ORDER: Record<Exclude<Filter, "all">, number> = {
  dream: 0,
  ghost: 1,
  review: 2,
  ontology: 3,
  large: 4,
  empty: 5,
  orphan: 6,
};

/**
 * 검토함의 데이터 층 — dashboardSnapshot·reviewsList 조회, vault/reviews
 * 변경 구독, 제외 목록(cfg)·세션 숨김 적용, 승인/무시 동작. ReviewView(전체
 * 화면)와 ReviewMini(홈·프로젝트 홈 요약)가 같은 큐를 본다.
 */
export function useReviewRows(project?: string | null): {
  rows: ReviewRow[];
  loading: boolean;
  /** 첫 조회 전인지 — 스켈레톤 판단용. */
  initial: boolean;
  err: string | null;
  counts: Record<Exclude<Filter, "all">, number>;
  pendingTotal: number;
  /** 손이 필요한 것만(정리·관계 제안·저신뢰·온톨로지) — 사이드바 숫자. 고아·빈/큰 파일 같은 참고용은 뺀다. */
  actionable: number;
  actions: {
    open: (row: ReviewRow) => { project: string; domain: string } | null;
    approve: (row: ReviewRow) => Promise<void>;
    dismiss: (row: ReviewRow) => Promise<void>;
    /** 정리 항목의 선택지 하나를 실행한다 (파일 수정은 git 커밋됨). */
    choose: (row: ReviewRow, choice: DreamChoice) => Promise<void>;
    reload: () => void;
    /** 고아 제외 목록 전체 교체 (제외 해제에 쓴다). */
    persistExcluded: (next: string[]) => Promise<void>;
  };
} {
  const cfg = useApp((s) => s.cfg);
  const tree = useApp((s) => s.tree);
  const [snap, setSnap] = useState<DashboardSnapshot | null>(null);
  const [reviews, setReviews] = useState<ReviewItem[]>([]);
  const [loading, setLoading] = useState(false);
  const [err, setErr] = useState<string | null>(null);
  // 빈 파일·큰 파일의 "무시" 와 아직 서버에 반영 전인 항목 — 세션 한정.
  const [hidden, setHidden] = useState<Set<string>>(() => new Set());

  const load = useCallback(async () => {
    setLoading(true);
    setErr(null);
    try {
      const [s, r] = await Promise.all([
        ipc.dashboardSnapshot(),
        ipc.reviewsList().catch(() => ({ items: [] as ReviewItem[] })),
      ]);
      setSnap(s);
      setReviews(r.items);
    } catch (e) {
      setErr(String(e));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    load();
    let alive = true;
    const offs: Array<() => void> = [];
    onVaultChanged(() => load()).then((fn) => {
      if (alive) offs.push(fn);
      else fn();
    });
    onReviewsChanged(() => load()).then((fn) => {
      if (alive) offs.push(fn);
      else fn();
    });
    const onLocal = () => load();
    window.addEventListener(LOCAL_REVIEWS_EVENT, onLocal);
    return () => {
      alive = false;
      for (const fn of offs) fn();
      window.removeEventListener(LOCAL_REVIEWS_EVENT, onLocal);
    };
  }, [load]);

  const excluded = useMemo(
    () => new Set(cfg?.review_excluded_paths ?? []),
    [cfg?.review_excluded_paths],
  );

  // "프로젝트/도메인" → mtime. 고아 정렬·"오래된 고아" 접기에 쓴다.
  const mtimeOf = useMemo(() => {
    const m = new Map<string, number>();
    for (const p of tree?.projects ?? []) {
      const visit = (domains: { name: string; modified_ms: number | null }[]) => {
        for (const d of domains) {
          if (d.modified_ms != null) m.set(`${p.name}/${d.name}`, d.modified_ms);
        }
      };
      visit(p.domains);
      for (const sf of p.subfolders) {
        visit(sf.domains);
        for (const sf2 of sf.subfolders) visit(sf2.domains);
      }
    }
    return m;
  }, [tree]);

  const rows = useMemo<Row[]>(() => {
    const out: Row[] = [];
    if (snap) {
      for (const g of snap.ghost_suggestions) {
        if (project && g.project !== project) continue;
        out.push({
          key: `ghost:${g.project}:${g.id}`,
          filter: "ghost",
          badge: "GHOST",
          badgeTone: "text-accent-blue",
          project: g.project,
          domain: g.source_domain,
          reason: g.relation
            ? `${g.source_domain} → ${g.target_domain} · ${g.reason}`
            : `${g.source_domain} ↔ ${g.target_domain} · ${g.reason}`,
          ts: g.created_at * 1000,
          approveLabel: "승인",
          tag: g.relation ? relationLabel(g.relation) : undefined,
          ghost: g,
        });
      }
      for (const h of snap.healing) {
        if (project && h.project !== project) continue;
        const row = healingRow(h, mtimeOf);
        if (!row) continue;
        if (row.filter === "orphan" && excluded.has(`${row.project}/${row.domain}`))
          continue;
        out.push(row);
      }
    }
    for (const it of reviews) {
      if (it.status !== "pending") continue;
      if (project && it.project !== project) continue;
      const dreamRow = it.kind.startsWith("dream_") ? dreamRowOf(it) : null;
      if (dreamRow) {
        out.push(dreamRow);
        continue;
      }
      out.push({
        key: `review:${it.id}`,
        filter: "review",
        badge: kindLabel(it.kind),
        badgeTone: "text-on-dark-mute",
        project: it.project ?? "—",
        domain: it.domain,
        reason: it.reason,
        ts: it.created_at * 1000,
        approveLabel: "해결",
        review: it,
      });
    }
    return out
      .filter((r) => !hidden.has(r.key))
      .sort((a, b) => FILTER_ORDER[a.filter] - FILTER_ORDER[b.filter] || b.ts - a.ts);
  }, [snap, reviews, project, excluded, hidden, mtimeOf]);

  const counts = useMemo(() => {
    const c: Record<Exclude<Filter, "all">, number> = {
      dream: 0,
      ghost: 0,
      orphan: 0,
      empty: 0,
      large: 0,
      ontology: 0,
      review: 0,
    };
    for (const r of rows) c[r.filter] += 1;
    return c;
  }, [rows]);

  const persistExcluded = useCallback(
    async (next: string[]) => {
      if (!cfg?.vault_path) return;
      const nextCfg: DanbiConfig = { ...cfg, review_excluded_paths: next };
      try {
        await ipc.saveConfig(cfg.vault_path, nextCfg);
        useApp.getState().setCfg(nextCfg);
      } catch (e) {
        setErr(String(e));
      }
    },
    [cfg],
  );

  const open = useCallback((r: Row) => {
    if (r.ghost) return { project: r.ghost.project, domain: r.ghost.source_domain };
    if (r.domain) return { project: r.project, domain: r.domain };
    return null;
  }, []);

  const choose = useCallback(
    async (r: Row, choice: DreamChoice) => {
      if (!r.review) return;
      setErr(null);
      try {
        await ipc.dreamApply(r.review.id, choice);
        setHidden((s) => new Set(s).add(r.key));
        notifyReviewsChanged();
      } catch (e) {
        setErr(String(e));
      }
    },
    [load],
  );

  const approve = useCallback(
    async (r: Row) => {
      try {
        if (r.dream && r.choices?.[0]) {
          await choose(r, r.choices[0].choice);
        } else if (r.ghost) {
          await ipc.ghostAccept(r.ghost.project, r.ghost.id);
          setHidden((s) => new Set(s).add(r.key));
          notifyReviewsChanged();
        } else if (r.review) {
          const next = await ipc.reviewsResolve(r.review.id, "resolved");
          setReviews(next.items);
          notifyReviewsChanged();
        } else if (r.filter === "orphan" && r.domain) {
          const path = `${r.project}/${r.domain}`;
          const list = cfg?.review_excluded_paths ?? [];
          if (!list.includes(path)) await persistExcluded([...list, path]);
        }
      } catch (e) {
        setErr(String(e));
      }
    },
    [cfg?.review_excluded_paths, choose, load, persistExcluded],
  );

  const dismiss = useCallback(
    async (r: Row) => {
      try {
        if (r.dream) {
          // "그대로 두기" — 아무것도 안 바꾸고 다시 묻지 않는다.
          await choose(r, { choice: "keep" });
        } else if (r.ghost) {
          await ipc.ghostReject(r.ghost.project, r.ghost.id);
          setHidden((s) => new Set(s).add(r.key));
          notifyReviewsChanged();
        } else if (r.review) {
          const next = await ipc.reviewsResolve(r.review.id, "dismissed");
          setReviews(next.items);
          notifyReviewsChanged();
        } else {
          // 빈 파일·큰 파일·고아 "무시" 는 세션 한정 숨김 — 다음 스캔에서
          // 그대로면 다시 보인다.
          setHidden((s) => new Set(s).add(r.key));
        }
      } catch (e) {
        setErr(String(e));
      }
    },
    [choose, load],
  );

  return {
    rows,
    loading,
    initial: !snap,
    err,
    counts,
    pendingTotal: rows.length,
    actionable: rows.filter((r) => r.filter === "dream" || r.filter === "ghost" || r.filter === "review" || r.filter === "ontology").length,
    actions: { open, approve, dismiss, choose, reload: load, persistExcluded },
  };
}

export function ReviewView({
  project,
  onOpenDoc,
  onOpenGraph,
}: {
  project?: string | null;
  onOpenDoc: (project: string, domain: string) => void;
  onOpenGraph?: (project: string) => void;
}) {
  const cfg = useApp((s) => s.cfg);
  const { rows, loading, initial, err, counts, actions } = useReviewRows(project);
  const { reload: load, persistExcluded } = actions;
  const [filter, setFilter] = useState<Filter>("all");
  const [active, setActive] = useState(0);
  const [showOldOrphans, setShowOldOrphans] = useState(false);
  // "추천대로 적용" — 확인 한 번 → 순서대로 실행 → 결과 한 줄.
  const recommended = useMemo(
    () => rows.map((r) => ({ r, rec: recommendationOf(r) })).filter((x): x is { r: Row; rec: Recommendation } => x.rec !== null),
    [rows],
  );
  const [bulk, setBulk] = useState<
    | { phase: "idle" }
    | { phase: "confirm" }
    | { phase: "running"; done: number; total: number }
    | { phase: "done"; ok: number; failed: number }
    | { phase: "finding" }
  >({ phase: "idle" });
  const applyRecommended = useCallback(async () => {
    const recs = recommended.map((x) => x.rec);
    const dreams = recs.flatMap((r) => (r.dream ? [r.dream] : []));
    const ghosts = recs.flatMap((r) => (r.ghost ? [r.ghost] : []));
    const total = recs.length;
    let ok = 0;
    let failed = 0;
    setBulk({ phase: "running", done: 0, total });
    // 정리 항목은 백엔드에서 한꺼번에 — (pre) 스냅샷 하나·커밋 하나라 한 번에 되돌릴 수 있다.
    if (dreams.length > 0) {
      try {
        const res = await ipc.dreamApplyMany(dreams);
        ok += res.ok;
        failed += res.failed.length;
      } catch {
        failed += dreams.length;
      }
      setBulk({ phase: "running", done: dreams.length, total });
    }
    for (const [i, g] of ghosts.entries()) {
      try {
        await ipc.ghostAccept(g.project, g.id);
        ok += 1;
      } catch {
        // 이미 처리됐거나 문서가 바뀐 제안 — 남겨 두고 계속.
        failed += 1;
      }
      setBulk({ phase: "running", done: dreams.length + i + 1, total });
    }
    setBulk({ phase: "done", ok, failed });
    notifyReviewsChanged();
  }, [recommended]);
  // 검토함은 20건까지만 올라온다 — 다 처리하면 다음 정리 거리를 바로 찾는다.
  const findMore = useCallback(async () => {
    setBulk({ phase: "finding" });
    try {
      await ipc.dreamRunNow();
    } catch {
      /* 아래 새로고침으로 충분 */
    }
    setBulk({ phase: "idle" });
    notifyReviewsChanged();
  }, []);
  const [showExcluded, setShowExcluded] = useState(false);

  // 필터 적용 + 고아는 ORPHAN_VISIBLE 개 뒤를 접는다 (rows 는 이미 정렬됨).
  const { visible, foldedOrphans } = useMemo(() => {
    const list = rows.filter((r) => filter === "all" || r.filter === filter);
    if (showOldOrphans) return { visible: list, foldedOrphans: 0 };
    let seenOrphans = 0;
    const visible: Row[] = [];
    let folded = 0;
    for (const r of list) {
      if (r.filter === "orphan") {
        seenOrphans += 1;
        if (seenOrphans > ORPHAN_VISIBLE) {
          folded += 1;
          continue;
        }
      }
      visible.push(r);
    }
    return { visible, foldedOrphans: folded };
  }, [rows, filter, showOldOrphans]);

  useEffect(() => {
    setActive((i) => Math.min(i, Math.max(0, visible.length - 1)));
  }, [visible.length]);

  const open = useCallback(
    (r: Row) => {
      const t = actions.open(r);
      if (t) onOpenDoc(t.project, t.domain);
    },
    [actions, onOpenDoc],
  );
  const { approve, dismiss } = actions;

  // J/K 이동 · ↩ 열기 · A 승인/제외 · X 무시 — 입력에 포커스 없을 때만.
  const visibleRef = useRef(visible);
  visibleRef.current = visible;
  const activeRef = useRef(active);
  activeRef.current = active;
  useEffect(() => {
    function onKey(e: KeyboardEvent) {
      if (e.metaKey || e.ctrlKey || e.altKey) return;
      const t = e.target as HTMLElement | null;
      if (
        t &&
        (t.tagName === "INPUT" ||
          t.tagName === "TEXTAREA" ||
          t.isContentEditable)
      )
        return;
      const list = visibleRef.current;
      const key = e.key.toLowerCase();
      if (key === "j") {
        e.preventDefault();
        setActive((i) => Math.min(list.length - 1, i + 1));
      } else if (key === "k") {
        e.preventDefault();
        setActive((i) => Math.max(0, i - 1));
      } else if (e.key === "Enter") {
        const r = list[activeRef.current];
        if (r) {
          e.preventDefault();
          open(r);
        }
      } else if (key === "a") {
        const r = list[activeRef.current];
        if (r?.approveLabel) {
          e.preventDefault();
          approve(r);
        }
      } else if (key === "x") {
        const r = list[activeRef.current];
        if (r) {
          e.preventDefault();
          dismiss(r);
        }
      }
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [open, approve, dismiss]);

  const excludedList = useMemo(() => {
    const all = cfg?.review_excluded_paths ?? [];
    return project ? all.filter((p) => p.startsWith(`${project}/`)) : all;
  }, [cfg?.review_excluded_paths, project]);

  return (
    <div className="flex h-full flex-col">
      <header
        data-tauri-drag-region
        className="flex h-12 shrink-0 items-center justify-between border-b border-hairline px-6"
      >
        <div className="flex items-center gap-2">
          <span className="text-[15px] font-medium tracking-[0.2px] text-ink">
            검토함
          </span>
          {project && (
            <span className="inline-flex h-6 items-center rounded-full border border-hairline bg-surface-elevated px-2.5 text-[11px] text-on-dark">
              @{project}
            </span>
          )}
        </div>
        <button
          onClick={load}
          disabled={loading}
          title="새로고침"
          className="grid h-7 w-7 place-items-center rounded-md text-mute transition-colors hover:bg-surface-elevated hover:text-on-dark"
        >
          <RefreshCw size={12} className={loading ? "animate-spin" : ""} />
        </button>
      </header>

      <div className="flex-1 min-h-0 overflow-auto">
        <div className="mx-auto max-w-[1040px] px-6 py-6">
          {(recommended.length > 0 || bulk.phase !== "idle") && (
            <RecommendBanner
              items={recommended.map((x) => x.rec)}
              leftover={rows.length - recommended.length}
              state={bulk}
              onAsk={() => setBulk({ phase: "confirm" })}
              onCancel={() => setBulk({ phase: "idle" })}
              onApply={applyRecommended}
              onFindMore={findMore}
              onDismissResult={() => setBulk({ phase: "idle" })}
            />
          )}
          <div className="text-caption-md text-mute">
            정리 {counts.dream} · GHOST {counts.ghost} · 고아 {counts.orphan} · 빈 파일 {counts.empty}{" "}
            · 큰 파일 {counts.large} · 온톨로지 {counts.ontology} · 저신뢰{" "}
            {counts.review}
          </div>

          <div className="mt-3 flex flex-wrap items-center gap-1.5">
            {FILTERS.map((f) => {
              const on = filter === f.key;
              const n = f.key === "all" ? rows.length : counts[f.key];
              return (
                <button
                  key={f.key}
                  onClick={() => setFilter(f.key)}
                  className={cn(
                    "inline-flex h-7 items-center gap-1.5 rounded-full border px-3 text-[12px] transition-colors",
                    on
                      ? "border-hairline-strong bg-surface-card text-on-dark"
                      : "border-hairline bg-surface text-mute hover:text-on-dark",
                  )}
                >
                  {f.label}
                  <span className="font-mono text-[11px] text-stone">{n}</span>
                </button>
              );
            })}
          </div>

          {err && (
            <div className="mt-3 rounded-md border border-hairline bg-surface-elevated px-3 py-2 text-caption-md text-accent-red">
              {err}
            </div>
          )}

          {initial && loading ? (
            <ul className="mt-4 flex flex-col gap-1">
              {[0, 1, 2].map((i) => (
                <li
                  key={i}
                  className="h-14 animate-pulse rounded-md border border-hairline bg-surface-elevated"
                />
              ))}
            </ul>
          ) : visible.length === 0 ? (
            <div className="mt-10 flex flex-col items-center gap-2 text-center">
              <Inbox size={22} className="text-stone" />
              <div className="text-[13px] text-mute">검토할 것이 없어요.</div>
              <div className="text-caption-sm text-stone">
                에이전트가 쓰거나 ghost 스캔·쉬는 시간 정리가 돌면 여기에 쌓여요.
              </div>
            </div>
          ) : (
            <ul className="mt-4 overflow-hidden rounded-lg border border-hairline bg-surface">
              {visible.map((r, i) => (
                <ReviewRow
                  key={r.key}
                  row={r}
                  active={i === active}
                  onHover={() => setActive(i)}
                  onOpen={() => open(r)}
                  onApprove={r.approveLabel ? () => approve(r) : undefined}
                  onDismiss={() => dismiss(r)}
                  onChoose={r.dream ? (c) => actions.choose(r, c) : undefined}
                  onGraph={
                    r.ghost && onOpenGraph
                      ? () => onOpenGraph(r.ghost!.project)
                      : undefined
                  }
                />
              ))}
            </ul>
          )}

          {foldedOrphans > 0 && (
            <button
              onClick={() => setShowOldOrphans(true)}
              className="mt-2 text-caption-md text-mute hover:text-on-dark"
            >
              오래된 고아 {foldedOrphans}건 펼치기
            </button>
          )}
          {showOldOrphans && (
            <button
              onClick={() => setShowOldOrphans(false)}
              className="mt-2 text-caption-md text-mute hover:text-on-dark"
            >
              오래된 고아 접기
            </button>
          )}

          {excludedList.length > 0 && (
            <div className="mt-6">
              <button
                onClick={() => setShowExcluded((v) => !v)}
                className="text-caption-md text-mute hover:text-on-dark"
              >
                {showExcluded ? "제외 목록 접기" : `제외한 ${excludedList.length}건 보기`}
              </button>
              {showExcluded && (
                <ul className="mt-2 overflow-hidden rounded-lg border border-hairline bg-surface">
                  {excludedList.map((p) => (
                    <li
                      key={p}
                      className="flex items-center justify-between gap-3 border-b border-hairline px-4 py-2 last:border-b-0"
                    >
                      <span className="truncate font-mono text-[12px] text-body">
                        {p}
                      </span>
                      <button
                        onClick={() =>
                          persistExcluded(
                            (cfg?.review_excluded_paths ?? []).filter(
                              (x) => x !== p,
                            ),
                          )
                        }
                        className="inline-flex h-7 shrink-0 items-center rounded-md border border-hairline bg-surface-elevated px-2 text-[12px] text-body hover:border-hairline-strong hover:text-on-dark"
                      >
                        다시 보기
                      </button>
                    </li>
                  ))}
                </ul>
              )}
            </div>
          )}

          <div className="mt-6 flex items-center gap-3 text-caption-sm text-stone">
            <Key>J</Key>
            <Key>K</Key> 이동 <Key>↩</Key> 열기 <Key>A</Key> 승인·제외{" "}
            <Key>X</Key> 무시
          </div>
        </div>
      </div>
    </div>
  );
}

function ReviewRow({
  row,
  active,
  onHover,
  onOpen,
  onApprove,
  onDismiss,
  onChoose,
  onGraph,
}: {
  row: Row;
  active: boolean;
  onHover: () => void;
  onOpen: () => void;
  onApprove?: () => void;
  onDismiss: () => void;
  onChoose?: (choice: DreamChoice) => void;
  onGraph?: () => void;
}) {
  const canOpen = !!row.domain;
  if (row.dream && row.choices && onChoose) {
    return (
      <DreamRow
        row={row}
        active={active}
        onHover={onHover}
        onOpen={canOpen ? onOpen : undefined}
        onChoose={onChoose}
      />
    );
  }
  return (
    <li
      onMouseEnter={onHover}
      className={cn(
        "flex items-center gap-3 border-b border-hairline px-4 py-2.5 last:border-b-0 transition-colors",
        active ? "bg-surface-elevated" : "hover:bg-surface-elevated/60",
      )}
    >
      <span
        className={cn(
          "shrink-0 rounded-xs bg-surface-elevated px-1.5 py-0.5 text-[10px] uppercase tracking-[0.4px]",
          row.badgeTone,
        )}
      >
        {row.badge}
      </span>
      <div className="min-w-0 flex-1">
        <div className="flex items-center gap-1.5 text-caption-md">
          <span className="text-mute">{row.project}</span>
          {row.domain && (
            <>
              <span className="text-stone">/</span>
              <span className="truncate font-mono text-[12px] text-body">
                {row.domain}
              </span>
            </>
          )}
        </div>
        <div className="flex min-w-0 items-center gap-1.5">
          {row.tag && <RelationTag>{row.tag}</RelationTag>}
          <div className="truncate text-[13px] text-ink" title={row.reason}>
            {row.reason}
          </div>
        </div>
        {row.ghost?.proposed_by && row.ghost.proposed_by !== "scan" && (
          <div className="text-caption-sm text-stone">
            에이전트 제안 ({row.ghost.proposed_by})
          </div>
        )}
        {row.ghost?.relation && row.ghost.relation !== "relates_to" && (
          <div className="text-caption-sm text-stone">
            승인하면 {row.ghost.source_domain} 에{" "}
            <span className="font-mono">
              {row.tag}:: [[{row.ghost.target_domain}]]
            </span>{" "}
            로 적어요.
          </div>
        )}
      </div>
      <div className="flex shrink-0 items-center gap-1.5">
        {canOpen && (
          <ActionButton onClick={onOpen}>열기</ActionButton>
        )}
        {onGraph && <ActionButton onClick={onGraph}>그래프</ActionButton>}
        {onApprove && (
          <ActionButton onClick={onApprove} title={row.approveLabel ?? undefined}>
            <Check size={11} /> {row.approveLabel}
          </ActionButton>
        )}
        <ActionButton onClick={onDismiss} muted>
          무시
        </ActionButton>
      </div>
    </li>
  );
}

/**
 * 쉬는 시간 정리 항목 한 행. 이유 아래에 선택지를 전부 펼친다 — 링크 후보
 * 여러 개, "아직 맞아요/대체됨", "옮기기" 처럼 한 가지 승인으로 끝나지
 * 않는 항목이라서. 마지막 "그대로 두기" 는 아무것도 안 바꾸고 다시 묻지
 * 않는다.
 */
function DreamRow({
  row,
  active,
  onHover,
  onOpen,
  onChoose,
}: {
  row: Row;
  active: boolean;
  onHover: () => void;
  onOpen?: () => void;
  onChoose: (choice: DreamChoice) => void;
}) {
  const d = row.dream!;
  const [expanded, setExpanded] = useState(false);
  const batchItems = d.kind === "fix_link_batch" ? d.items : null;
  return (
    <li
      onMouseEnter={onHover}
      className={cn(
        "flex flex-col gap-2 border-b border-hairline px-4 py-3 last:border-b-0 transition-colors",
        active ? "bg-surface-elevated" : "hover:bg-surface-elevated/60",
      )}
    >
      <div className="flex items-center gap-3">
        <span
          className={cn(
            "shrink-0 rounded-xs bg-surface-elevated px-1.5 py-0.5 text-[10px] uppercase tracking-[0.4px]",
            row.badgeTone,
          )}
        >
          {row.badge}
        </span>
        <div className="min-w-0 flex-1">
          <div className="flex items-center gap-1.5 text-caption-md">
            <span className="text-mute">{row.project}</span>
            {row.domain && (
              <>
                <span className="text-stone">/</span>
                <span className="truncate font-mono text-[12px] text-body">
                  {row.domain}
                </span>
              </>
            )}
            {d.kind === "fix_link" && d.sources.length > 1 && (
              <span className="text-stone">외 {d.sources.length - 1}곳</span>
            )}
          </div>
          <div className="text-[13px] text-ink" title={row.reason}>
            {dreamHeadline(d)}
          </div>
          <div className="text-caption-sm text-stone">{dreamDetail(d)}</div>
        </div>
        {onOpen && <ActionButton onClick={onOpen}>열기</ActionButton>}
      </div>
      {batchItems && (
        <div className="pl-[46px]">
          <button
            onClick={(e) => {
              e.stopPropagation();
              setExpanded((v) => !v);
            }}
            className="text-caption-md text-mute hover:text-on-dark"
          >
            {expanded ? "목록 접기" : `${batchItems.length}종 펼쳐보기`}
          </button>
          {expanded && (
            <ul className="mt-1.5 overflow-hidden rounded-md border border-hairline bg-surface">
              {batchItems.map((it) => (
                <li
                  key={it.key}
                  className="flex items-center gap-2 border-b border-hairline px-3 py-1.5 text-[12px] last:border-b-0"
                  title={`${it.why} · ${it.sources.join(", ")}`}
                >
                  <span className="truncate font-mono text-body">[[{it.broken}]]</span>
                  <span className="shrink-0 text-stone">→</span>
                  <span className="truncate font-mono text-on-dark">{it.target}</span>
                  <span className="ml-auto shrink-0 text-caption-sm text-stone">
                    문서 {it.sources.length}곳
                  </span>
                </li>
              ))}
            </ul>
          )}
        </div>
      )}
      <div className="flex flex-wrap items-center gap-1.5 pl-[46px]">
        {row.choices!.map((c, i) => (
          <button
            key={i}
            onClick={(e) => {
              e.stopPropagation();
              onChoose(c.choice);
            }}
            title={c.hint}
            className={cn(
              "inline-flex h-7 items-center gap-1 rounded-md border px-2 text-[12px] transition-colors",
              c.writes
                ? "border-hairline bg-surface-elevated text-body hover:border-hairline-strong hover:text-on-dark"
                : "border-transparent text-stone hover:text-on-dark",
            )}
          >
            {c.writes && i === 0 && <Check size={11} />}
            {c.label}
          </button>
        ))}
      </div>
    </li>
  );
}

function dreamHeadline(d: DreamAction): string {
  switch (d.kind) {
    case "fix_link":
      return `[[${d.broken}]] 가 가리키는 문서가 없어요.`;
    case "fix_link_batch": {
      const docs = new Set(d.items.flatMap((i) => i.sources)).size;
      return `폴더가 빠지거나 표기만 다른 링크 ${d.items.length}종(문서 ${docs}곳)을 한 번에 고칠까요?`;
    }
    case "stale":
      return `stale_after: ${d.stale_after} 이 지났어요 — 아직 맞는 내용인가요?`;
    case "cites_superseded":
      return `대체된 문서 ${d.target} 를 아직 인용하고 있어요.`;
    case "promote":
      return `'${d.title}' 을 ${d.target} 로 옮길까요?`;
  }
}

function dreamDetail(d: DreamAction): string {
  switch (d.kind) {
    case "fix_link":
      return d.options.length
        ? "비슷한 문서를 찾았어요. 고르면 링크를 바꾸고 git 에 기록해요."
        : "비슷한 문서를 못 찾았어요. 링크를 풀거나 그대로 둘 수 있어요.";
    case "fix_link_batch":
      return `예: [[${d.items[0]?.broken}]] → ${d.items[0]?.target}. 후보가 하나뿐인 확실한 것만 모았어요. 승인하면 전부 고치고 커밋 1개로 남겨요.`;
    case "stale":
      return "\"아직 맞아요\" 를 고르면 확인 표시를 남기고 90일 뒤로 미뤄요.";
    case "cites_superseded":
      return d.replacement
        ? `새 문서: ${d.replacement}`
        : "어떤 문서로 대체됐는지는 못 찾았어요.";
    case "promote":
      return `daily 의 "${d.heading.replace(/^#+\s*/, "")}" 섹션 — daily 엔 링크 한 줄만 남아요.`;
  }
}

/** `dream_*` 검토 항목 → 행 + 선택지. payload 가 깨져 있으면 null. */
/** "추천대로 적용" 이 실행할 선택 — 판단 없이 맞다고 볼 수 있는 것만.
 *  오래된 문서 확인(내용이 맞는지는 사람만 앎)·애매한 링크(후보가 둘 이상
 *  비슷함)·후보 없는 링크·고아/빈 파일 같은 정보성 항목은 null → 남긴다. */
export type Recommendation = {
  kind: "link" | "promote" | "relation";
  /** 정리 항목 — 백엔드에서 한꺼번에(커밋 하나) 실행. */
  dream?: { id: string; choice: DreamChoice };
  /** 관계 제안 — 하나씩 승인. */
  ghost?: { project: string; id: string };
};

/** 검토함이 바뀌었다고 같은 창의 다른 화면(사이드바 숫자·홈 카드)에 알린다. */
const LOCAL_REVIEWS_EVENT = "danbi:reviews-local";
export function notifyReviewsChanged() {
  window.dispatchEvent(new Event(LOCAL_REVIEWS_EVENT));
}

function recommendationOf(r: Row): Recommendation | null {
  const d = r.dream;
  const id = r.review?.id;
  if (d && id) {
    switch (d.kind) {
      case "fix_link": {
        const [best, next] = d.options;
        if (d.ambiguous || !best) return null;
        // 둘째 후보가 거의 같은 점수면 고를 근거가 없다.
        if (next && best.confidence - next.confidence < 0.1) return null;
        return { kind: "link", dream: { id, choice: { choice: "relink", target: best.target } } };
      }
      case "fix_link_batch":
        return { kind: "link", dream: { id, choice: { choice: "apply_all" } } };
      case "cites_superseded":
        return d.replacement ? { kind: "link", dream: { id, choice: { choice: "relink", target: d.replacement } } } : null;
      case "promote":
        return { kind: "promote", dream: { id, choice: { choice: "promote" } } };
      default:
        return null;
    }
  }
  if (r.ghost) {
    return { kind: "relation", ghost: { project: r.ghost.project, id: r.ghost.id } };
  }
  return null;
}

function dreamRowOf(it: ReviewItem): Row | null {
  const d = it.payload;
  if (!d || !("kind" in d)) return null;
  const base = {
    key: `dream:${it.id}`,
    filter: "dream" as const,
    badge: "정리",
    badgeTone: "text-accent-green",
    project: it.project ?? "—",
    domain: it.domain,
    reason: it.reason,
    ts: it.created_at * 1000,
    review: it,
    dream: d as DreamAction,
  };
  const keep: DreamChoiceButton = {
    label: "그대로 두기",
    choice: { choice: "keep" },
    writes: false,
    hint: "아무것도 바꾸지 않고 다시 묻지 않아요",
  };
  let choices: DreamChoiceButton[];
  switch (d.kind) {
    case "fix_link":
      choices = [
        ...d.options.map<DreamChoiceButton>((o) => ({
          label: `→ ${o.target}`,
          choice: { choice: "relink", target: o.target },
          writes: true,
          hint: `${o.why} (${Math.round(o.confidence * 100)}%)`,
        })),
        { label: "링크 풀기", choice: { choice: "remove" }, writes: true, hint: "대괄호를 빼고 글자만 남겨요" },
        keep,
      ];
      break;
    case "fix_link_batch":
      choices = [
        { label: `모두 고치기 (${d.items.length}종)`, choice: { choice: "apply_all" }, writes: true, hint: "목록의 링크를 전부 바꾸고 커밋 1개" },
        { ...keep, hint: "묶음 전체를 보류하고 다시 묻지 않아요" },
      ];
      break;
    case "stale":
      choices = [
        { label: "아직 맞아요", choice: { choice: "verify" }, writes: true, hint: "verified 표시 + stale_after 90일 연장" },
        { label: "대체됨", choice: { choice: "supersede" }, writes: true, hint: "status: superseded 로 표시" },
        keep,
      ];
      break;
    case "cites_superseded":
      choices = [
        ...(d.replacement
          ? [{ label: `→ ${d.replacement}`, choice: { choice: "relink", target: d.replacement } as DreamChoice, writes: true, hint: "새 문서를 가리키게 바꿔요" }]
          : []),
        { label: "링크 풀기", choice: { choice: "remove" }, writes: true, hint: "대괄호를 빼고 글자만 남겨요" },
        keep,
      ];
      break;
    case "promote":
      choices = [
        { label: "옮기기", choice: { choice: "promote" }, writes: true, hint: `${d.target} 를 만들고 daily 엔 링크만` },
        keep,
      ];
      break;
    default:
      return null;
  }
  // A 키·미니 카드의 "승인" 은 첫 선택지 — 단, 링크를 푸는 것만 남은 항목은
  // 실수로 누르기 쉬워 전체 화면에서 직접 고르게 둔다.
  const first = choices[0];
  const approveLabel = first.writes && first.choice.choice !== "remove" ? first.label : null;
  return { ...base, approveLabel, choices };
}

function RecommendBanner({
  items,
  leftover,
  state,
  onAsk,
  onCancel,
  onApply,
  onFindMore,
  onDismissResult,
}: {
  items: Recommendation[];
  leftover: number;
  state:
    | { phase: "idle" }
    | { phase: "confirm" }
    | { phase: "running"; done: number; total: number }
    | { phase: "done"; ok: number; failed: number }
    | { phase: "finding" };
  onAsk: () => void;
  onCancel: () => void;
  onApply: () => void;
  onFindMore: () => void;
  onDismissResult: () => void;
}) {
  const n = (k: Recommendation["kind"]) => items.filter((i) => i.kind === k).length;
  const parts = [
    n("link") && `링크 고치기 ${n("link")}`,
    n("promote") && `daily → 주제 문서 옮기기 ${n("promote")}`,
    n("relation") && `관계 연결 ${n("relation")}`,
  ].filter(Boolean);
  return (
    <section className="mb-5 rounded-lg border border-hairline bg-surface-elevated px-5 py-4">
      {state.phase === "finding" ? (
        <div className="text-[14px] text-body">다음 정리 거리를 찾는 중…</div>
      ) : state.phase === "done" ? (
        <div className="flex items-center justify-between gap-4">
          <div className="text-[14px] text-ink">
            추천대로 {state.ok}건을 적용했어요.
            {state.failed > 0 && <span className="ml-1 text-mute">{state.failed}건은 이미 바뀌어 건너뛰었어요.</span>}
            <div className="mt-0.5 text-caption-md text-mute">
              정리 항목은 커밋 하나로 묶여 한 번에 되돌릴 수 있어요. 검토함에는 20건씩 올라오니, 더 정리하려면 다음 정리 거리를 찾으세요.
            </div>
          </div>
          <div className="flex shrink-0 items-center gap-3">
            <button onClick={onDismissResult} className="text-[12px] text-mute hover:text-on-dark">
              닫기
            </button>
            <button
              onClick={onFindMore}
              className="h-10 rounded-md border border-hairline bg-surface-card px-4 text-[13px] text-ink transition-colors hover:bg-surface"
            >
              다음 정리 거리 찾기
            </button>
          </div>
        </div>
      ) : (
        <div className="flex items-center justify-between gap-6">
          <div className="min-w-0">
            <div className="flex items-center gap-2 text-[15px] font-medium text-ink">
              <Sparkles size={15} className="text-mute" />
              추천대로 {items.length}건을 한 번에 정리할 수 있어요
            </div>
            <div className="mt-1 text-caption-md text-mute">
              {parts.join(" · ")}
              {leftover > 0 && ` — 판단이 필요한 ${leftover}건(오래된 문서·애매한 링크 등)은 그대로 남겨요.`}
            </div>
          </div>
          {state.phase === "running" ? (
            <div className="shrink-0 text-[13px] tabular-nums text-body">
              적용 중 {state.done}/{state.total}
            </div>
          ) : state.phase === "confirm" ? (
            <div className="flex shrink-0 items-center gap-2">
              <button onClick={onCancel} className="h-10 px-3 text-[13px] text-mute hover:text-on-dark">
                취소
              </button>
              <button
                onClick={onApply}
                className="h-10 rounded-md bg-primary px-5 text-[14px] font-medium text-on-primary transition-colors hover:bg-primary-pressed"
              >
                {items.length}건 적용하기
              </button>
            </div>
          ) : (
            <button
              onClick={onAsk}
              disabled={items.length === 0}
              className="h-10 shrink-0 rounded-md bg-primary px-5 text-[14px] font-medium text-on-primary transition-colors hover:bg-primary-pressed disabled:opacity-40"
            >
              추천대로 적용
            </button>
          )}
        </div>
      )}
    </section>
  );
}

function ActionButton({
  onClick,
  children,
  muted,
  title,
}: {
  onClick: () => void;
  children: React.ReactNode;
  muted?: boolean;
  title?: string;
}) {
  return (
    <button
      onClick={(e) => {
        e.stopPropagation();
        onClick();
      }}
      title={title}
      className={cn(
        "inline-flex h-7 items-center gap-1 rounded-md border border-hairline bg-surface-elevated px-2 text-[12px] transition-colors hover:border-hairline-strong hover:text-on-dark",
        muted ? "text-stone" : "text-body",
      )}
    >
      {children}
    </button>
  );
}

/** 관계 이름 뱃지 — 중립 톤. ReviewMini 도 쓴다. */
export function RelationTag({ children }: { children: React.ReactNode }) {
  return (
    <span className="inline-flex h-[18px] shrink-0 items-center rounded-xs border border-hairline-strong bg-surface-elevated px-1.5 text-[10px] text-on-dark">
      {children}
    </span>
  );
}

function Key({ children }: { children: React.ReactNode }) {
  return (
    <kbd className="inline-flex h-5 min-w-[20px] items-center justify-center rounded-xs border border-hairline bg-surface-elevated px-1 font-mono text-[10px] text-mute">
      {children}
    </kbd>
  );
}

function healingRow(
  h: VaultSuggestion,
  mtimeOf: Map<string, number>,
): Row | null {
  if (h.kind === "EmptyProject") {
    return {
      key: `heal:emptyproject:${h.project}`,
      filter: "empty",
      badge: "EMPTY",
      badgeTone: "text-on-dark-mute",
      project: h.project,
      domain: null,
      reason: "프로젝트가 비어있어요.",
      ts: 0,
      approveLabel: null,
    };
  }
  const path = `${h.project}/${h.domain}`;
  const ts = mtimeOf.get(path) ?? 0;
  switch (h.kind) {
    case "Orphan": {
      const base = h.domain.split("/").pop() ?? h.domain;
      if (INDEX_LIKE.has(base)) return null;
      return {
        key: `heal:orphan:${path}`,
        filter: "orphan",
        badge: "ORPHAN",
        badgeTone: "text-accent-blue",
        project: h.project,
        domain: h.domain,
        reason: "들어오는 링크가 없어요. 관련 문서에서 [[링크]] 를 걸거나 제외하세요.",
        ts,
        approveLabel: "제외",
      };
    }
    case "Empty":
      return {
        key: `heal:empty:${path}`,
        filter: "empty",
        badge: "EMPTY",
        badgeTone: "text-on-dark-mute",
        project: h.project,
        domain: h.domain,
        reason: "비어있는 문서예요.",
        ts,
        approveLabel: null,
      };
    case "Oversized":
      return {
        key: `heal:large:${path}`,
        filter: "large",
        badge: "LARGE",
        badgeTone: "text-accent-yellow",
        project: h.project,
        domain: h.domain,
        reason: `${(h.bytes / 1024).toFixed(1)}KB — 섹션으로 분할하는 걸 고려하세요.`,
        ts,
        approveLabel: null,
      };
    case "UnknownType":
    case "UnknownStatus":
    case "UnknownRelation":
    case "DanglingTypedLink":
      return ontologyRow(h, path, ts);
    default: {
      // 백엔드가 새 종류를 더해도 화면이 깨지지 않게 — 타입상 never.
      const unknown = h as { kind?: string; project?: string; domain?: string };
      return {
        key: `heal:other:${unknown.kind ?? "?"}:${path}`,
        filter: "ontology",
        badge: (unknown.kind ?? "기타").slice(0, 10).toUpperCase(),
        badgeTone: "text-on-dark-mute",
        project: unknown.project ?? "—",
        domain: unknown.domain ?? null,
        reason: "정리 제안이 있어요.",
        ts,
        approveLabel: null,
      };
    }
  }
}

type OntologySuggestion = Extract<
  VaultSuggestion,
  { kind: "UnknownType" | "UnknownStatus" | "UnknownRelation" | "DanglingTypedLink" }
>;

/** 온톨로지 정리 제안 4종 — 한국어 안내 문구. 고칠 곳은 문서 자체라 열기·무시만. */
function ontologyRow(h: OntologySuggestion, path: string, ts: number): Row {
  const base = {
    filter: "ontology" as const,
    badgeTone: "text-on-dark-mute",
    project: h.project,
    domain: h.domain,
    ts,
    approveLabel: null,
  };
  switch (h.kind) {
    case "UnknownType":
      return {
        ...base,
        key: `heal:onto-type:${path}:${h.value}`,
        badge: "TYPE",
        reason: `모르는 문서 종류 '${h.value}' — schema.md 의 ## 온톨로지 에 추가하거나 type 값을 바꾸세요.`,
      };
    case "UnknownStatus":
      return {
        ...base,
        key: `heal:onto-status:${path}:${h.value}`,
        badge: "STATUS",
        reason: `모르는 상태 '${h.value}' — active(유효) · superseded(대체됨) · done(완료) 중 하나로 바꾸거나 schema.md 에 추가하세요.`,
      };
    case "UnknownRelation":
      return {
        ...base,
        key: `heal:onto-rel:${path}:${h.key}`,
        badge: "RELATION",
        reason: `모르는 관계 '${h.key}::' — schema.md 의 ## 온톨로지 에 추가하거나 영향·원인·대체 같은 이름으로 바꾸세요.`,
      };
    case "DanglingTypedLink": {
      const target = h.target.startsWith(`${h.project}/`)
        ? h.target.slice(h.project.length + 1)
        : h.target;
      return {
        ...base,
        key: `heal:onto-dangling:${path}:${h.relation}:${h.target}`,
        badge: "BROKEN",
        reason: `끊긴 관계 링크: ${relationLabel(h.relation)} → ${target} (문서 없음)`,
      };
    }
  }
}

function kindLabel(kind: string): string {
  switch (kind) {
    case "low_confidence_plan":
      return "저신뢰";
    case "unclear_destination":
      return "목적지";
    case "suggest_split":
      return "분할";
    case "duplicate_content":
      return "중복";
    case "broken_link":
      return "링크";
    case "ghost_candidate":
      return "제안";
    case "dream_fix_link":
    case "dream_fix_link_batch":
    case "dream_recheck":
    case "dream_promote":
      return "정리";
    default:
      return kind;
  }
}
