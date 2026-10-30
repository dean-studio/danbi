import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Check, Inbox, RefreshCw } from "lucide-react";
import {
  ipc,
  onReviewsChanged,
  onVaultChanged,
  type DanbiConfig,
  type DashboardSnapshot,
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
  | "ghost"
  | "orphan"
  | "empty"
  | "large"
  | "ontology"
  | "review";

const FILTERS: Array<{ key: Filter; label: string }> = [
  { key: "all", label: "전체" },
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
};
type Row = ReviewRow;

/** 표시 순서: ghost → 저신뢰 → 온톨로지 → 큰 파일 → 빈 파일 → 고아(최근 수정 우선). */
const FILTER_ORDER: Record<Exclude<Filter, "all">, number> = {
  ghost: 0,
  review: 1,
  ontology: 2,
  large: 3,
  empty: 4,
  orphan: 5,
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
  actions: {
    open: (row: ReviewRow) => { project: string; domain: string } | null;
    approve: (row: ReviewRow) => Promise<void>;
    dismiss: (row: ReviewRow) => Promise<void>;
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
    return () => {
      alive = false;
      for (const fn of offs) fn();
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

  const approve = useCallback(
    async (r: Row) => {
      try {
        if (r.ghost) {
          await ipc.ghostAccept(r.ghost.project, r.ghost.id);
          setHidden((s) => new Set(s).add(r.key));
          load();
        } else if (r.review) {
          const next = await ipc.reviewsResolve(r.review.id, "resolved");
          setReviews(next.items);
        } else if (r.filter === "orphan" && r.domain) {
          const path = `${r.project}/${r.domain}`;
          const list = cfg?.review_excluded_paths ?? [];
          if (!list.includes(path)) await persistExcluded([...list, path]);
        }
      } catch (e) {
        setErr(String(e));
      }
    },
    [cfg?.review_excluded_paths, load, persistExcluded],
  );

  const dismiss = useCallback(
    async (r: Row) => {
      try {
        if (r.ghost) {
          await ipc.ghostReject(r.ghost.project, r.ghost.id);
          setHidden((s) => new Set(s).add(r.key));
          load();
        } else if (r.review) {
          const next = await ipc.reviewsResolve(r.review.id, "dismissed");
          setReviews(next.items);
        } else {
          // 빈 파일·큰 파일·고아 "무시" 는 세션 한정 숨김 — 다음 스캔에서
          // 그대로면 다시 보인다.
          setHidden((s) => new Set(s).add(r.key));
        }
      } catch (e) {
        setErr(String(e));
      }
    },
    [load],
  );

  return {
    rows,
    loading,
    initial: !snap,
    err,
    counts,
    pendingTotal: rows.length,
    actions: { open, approve, dismiss, reload: load, persistExcluded },
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
          <div className="text-caption-md text-mute">
            GHOST {counts.ghost} · 고아 {counts.orphan} · 빈 파일 {counts.empty}{" "}
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
                에이전트가 쓰거나 ghost 스캔이 돌면 여기에 쌓여요.
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
  onGraph,
}: {
  row: Row;
  active: boolean;
  onHover: () => void;
  onOpen: () => void;
  onApprove?: () => void;
  onDismiss: () => void;
  onGraph?: () => void;
}) {
  const canOpen = !!row.domain;
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
    default:
      return kind;
  }
}
