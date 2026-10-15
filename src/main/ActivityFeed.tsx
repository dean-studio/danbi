import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Bot, ChevronDown, ChevronRight, Droplets, Loader2 } from "lucide-react";
import {
  ipc,
  onVaultChanged,
  type ActivityEntry,
  type DocDiff,
} from "@/lib/ipc";
import { ClaudeIcon, CodexIcon, CursorIcon } from "@/components/AgentIcons";
import { changeOpLabel, changeOpTone, DiffBlock } from "@/main/DocInspector";
import { cn } from "@/lib/utils";

/**
 * 활동 피드 — "누가 언제 무엇을 썼고 되돌릴 수 있나" 를 한 곳에.
 * `activity_feed`(커밋당 1행, (pre) 제외) 위에서 읽기만 하고, 되돌리기는
 * `activity_revert`(문서 단위). 홈(전체)과 프로젝트 홈(scopeProject)이
 * 같은 컴포넌트를 범위만 바꿔 쓴다.
 */

/** 같은 actor·client·문서의 연속 기록을 하나로 접는 창. */
const GROUP_WINDOW_SEC = 10 * 60;
const DEFAULT_LIMIT = 50;
/** vault:changed 뒤 상단 갱신 페이지 크기 — 전체 재조회 대신. */
const REFRESH_LIMIT = 20;

export type ActivityStats = {
  sinceYesterday: number;
  byClient: Record<string, number>;
};

export function clientLabel(client: string): string {
  switch (client) {
    case "claude-code":
    case "claude_code":
      return "Claude Code";
    case "codex":
      return "Codex";
    case "cursor":
      return "Cursor";
    case "human":
      return "사람";
    case "danbi":
      return "단비";
    default:
      return "에이전트";
  }
}

/** 피드 전용 op 라벨 — 인스펙터와 같은 키는 거기 걸 쓰고, 피드에만 오는
 *  op(save/capture/revert/migration/init…)만 여기서. */
function feedOpLabel(op: string): string {
  switch (op) {
    case "save":
      return "저장";
    case "capture":
      return "캡처";
    case "log":
      return "로그";
    case "create_file":
      return "생성";
    case "create_folder":
      return "폴더";
    case "restore":
      return "복원";
    case "revert":
      return "되돌림";
    case "migration":
      return "마이그레이션";
    case "init":
      return "초기화";
    case "rest":
      return "REST";
    default:
      return changeOpLabel(op);
  }
}

function feedOpTone(op: string): string {
  if (op === "revert" || op === "restore") return "bg-accent-yellow-soft text-accent-yellow";
  return changeOpTone(op);
}

type Group = {
  key: string;
  /** 최신 항목이 [0]. */
  entries: ActivityEntry[];
};

type Section = { label: string; groups: Group[] };

function startOfDay(ms: number): number {
  const d = new Date(ms);
  d.setHours(0, 0, 0, 0);
  return d.getTime();
}

function hhmm(tsSec: number): string {
  const d = new Date(tsSec * 1000);
  return `${String(d.getHours()).padStart(2, "0")}:${String(d.getMinutes()).padStart(2, "0")}`;
}

function sameDoc(a: ActivityEntry, b: ActivityEntry): boolean {
  return (
    a.actor === b.actor &&
    a.client === b.client &&
    (a.project ?? "") === (b.project ?? "") &&
    (a.domain ?? "") === (b.domain ?? "")
  );
}

/** 최신순 정렬된 항목을 10분 창 안의 연속 기록끼리 묶고 날짜 섹션으로. */
function buildSections(entries: ActivityEntry[]): Section[] {
  const groups: Group[] = [];
  for (const e of entries) {
    const last = groups[groups.length - 1];
    const tail = last?.entries[last.entries.length - 1];
    if (
      last &&
      tail &&
      e.project &&
      e.domain &&
      sameDoc(tail, e) &&
      tail.ts - e.ts <= GROUP_WINDOW_SEC
    ) {
      last.entries.push(e);
    } else {
      groups.push({ key: e.id, entries: [e] });
    }
  }
  const todayStart = startOfDay(Date.now());
  const yesterdayStart = todayStart - 24 * 60 * 60 * 1000;
  const sections: Section[] = [];
  for (const g of groups) {
    const ms = g.entries[0].ts * 1000;
    let label: string;
    if (ms >= todayStart) label = "오늘";
    else if (ms >= yesterdayStart) label = "어제";
    else {
      const d = new Date(ms);
      label = `${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}`;
    }
    const cur = sections[sections.length - 1];
    if (cur && cur.label === label) cur.groups.push(g);
    else sections.push({ label, groups: [g] });
  }
  return sections;
}

function computeStats(entries: ActivityEntry[]): ActivityStats {
  const since = (startOfDay(Date.now()) - 24 * 60 * 60 * 1000) / 1000;
  const byClient: Record<string, number> = {};
  let n = 0;
  for (const e of entries) {
    if (e.ts < since) continue;
    n += 1;
    byClient[e.client] = (byClient[e.client] ?? 0) + 1;
  }
  return { sinceYesterday: n, byClient };
}

function mergeTop(existing: ActivityEntry[], fresh: ActivityEntry[]): ActivityEntry[] {
  const seen = new Set(existing.map((e) => e.id));
  const added = fresh.filter((e) => !seen.has(e.id));
  if (added.length === 0) return existing;
  return [...added, ...existing].sort((a, b) => b.ts - a.ts);
}

function inEditable(): boolean {
  const el = document.activeElement as HTMLElement | null;
  if (!el) return false;
  const tag = el.tagName.toLowerCase();
  return (
    tag === "input" ||
    tag === "textarea" ||
    el.getAttribute("contenteditable") === "true" ||
    !!el.closest?.("[contenteditable=\"true\"]")
  );
}

/** entry.files 에서 이 문서의 경로 — 되돌리기 path. */
function revertPath(e: ActivityEntry): string | null {
  if (e.project && e.domain) {
    const want = `Projects/${e.project}/${e.domain}`;
    if (e.files.includes(want)) return want;
  }
  return e.files[0] ?? null;
}

export function ActivityFeed({
  scopeProject,
  limit = DEFAULT_LIMIT,
  onOpenDoc,
  onStats,
  optimistic,
  compact = false,
  className,
}: {
  scopeProject?: string | null;
  limit?: number;
  onOpenDoc: (project: string, domain: string) => void;
  onStats?: (s: ActivityStats) => void;
  optimistic?: ActivityEntry[];
  compact?: boolean;
  className?: string;
}) {
  const scope = scopeProject ?? null;
  const [entries, setEntries] = useState<ActivityEntry[]>([]);
  const [loading, setLoading] = useState(true);
  const [loadingMore, setLoadingMore] = useState(false);
  const [exhausted, setExhausted] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [activeKey, setActiveKey] = useState<string | null>(null);
  const [expanded, setExpanded] = useState<Set<string>>(new Set());
  const [diffOpen, setDiffOpen] = useState<Set<string>>(new Set());
  const diffCache = useRef<Map<string, DocDiff>>(new Map());
  const [diffs, setDiffs] = useState<Record<string, DocDiff | "loading" | "error">>({});
  // 되돌리기 상태 — id 별 단계: confirm → (force) → busy / error.
  const [revert, setRevert] = useState<{
    id: string;
    stage: "confirm" | "force";
    warning?: string;
    later?: number;
    busy?: boolean;
    error?: string;
  } | null>(null);
  const onStatsRef = useRef(onStats);
  onStatsRef.current = onStats;

  const loadFirst = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const rows = await ipc.activityFeed({ scopeProject: scope ?? undefined, limit });
      setEntries(rows);
      setExhausted(rows.length < limit);
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  }, [scope, limit]);

  useEffect(() => {
    setEntries([]);
    setExpanded(new Set());
    setDiffOpen(new Set());
    setRevert(null);
    loadFirst();
  }, [loadFirst]);

  // vault:changed → 상단 20건만 받아 새 id 를 끼워 넣는다.
  useEffect(() => {
    let alive = true;
    let timer: ReturnType<typeof setTimeout> | null = null;
    let unlisten: (() => void) | null = null;
    onVaultChanged(() => {
      if (timer) clearTimeout(timer);
      timer = setTimeout(async () => {
        timer = null;
        try {
          const fresh = await ipc.activityFeed({
            scopeProject: scope ?? undefined,
            limit: REFRESH_LIMIT,
          });
          if (alive) setEntries((cur) => mergeTop(cur, fresh));
        } catch {
          /* ignore — next event retries */
        }
      }, 500);
    }).then((fn) => {
      if (alive) unlisten = fn;
      else fn();
    });
    return () => {
      alive = false;
      if (timer) clearTimeout(timer);
      if (unlisten) unlisten();
    };
  }, [scope]);

  // 호스트의 옵티미스틱 행 + 서버 행 (id 중복 제거, 최신순).
  const all = useMemo(() => {
    if (!optimistic || optimistic.length === 0) return entries;
    const ids = new Set(entries.map((e) => e.id));
    const extra = optimistic.filter((e) => !ids.has(e.id));
    return extra.length === 0 ? entries : [...extra, ...entries].sort((a, b) => b.ts - a.ts);
  }, [entries, optimistic]);

  useEffect(() => {
    onStatsRef.current?.(computeStats(all));
  }, [all]);

  const sections = useMemo(() => buildSections(all), [all]);
  const groupOrder = useMemo(
    () => sections.flatMap((s) => s.groups.map((g) => g.key)),
    [sections],
  );
  const groupByKey = useMemo(() => {
    const m = new Map<string, Group>();
    for (const s of sections) for (const g of s.groups) m.set(g.key, g);
    return m;
  }, [sections]);

  async function loadMore() {
    const last = all[all.length - 1];
    if (!last || loadingMore) return;
    setLoadingMore(true);
    try {
      const rows = await ipc.activityFeed({
        scopeProject: scope ?? undefined,
        before: last.ts,
        limit,
      });
      setEntries((cur) => {
        const seen = new Set(cur.map((e) => e.id));
        return [...cur, ...rows.filter((r) => !seen.has(r.id))];
      });
      if (rows.length < limit) setExhausted(true);
    } catch (e) {
      setError(String(e));
    } finally {
      setLoadingMore(false);
    }
  }

  async function toggleDiff(e: ActivityEntry) {
    setDiffOpen((s) => {
      const n = new Set(s);
      if (n.has(e.id)) n.delete(e.id);
      else n.add(e.id);
      return n;
    });
    if (!e.project || !e.domain) return;
    if (diffCache.current.has(e.id) || diffs[e.id] === "loading") return;
    setDiffs((d) => ({ ...d, [e.id]: "loading" }));
    try {
      const res = await ipc.docDiff(e.project, e.domain, e.id);
      diffCache.current.set(e.id, res);
      setDiffs((d) => ({ ...d, [e.id]: res }));
    } catch {
      setDiffs((d) => ({ ...d, [e.id]: "error" }));
    }
  }

  async function runRevert(e: ActivityEntry, force: boolean) {
    const path = revertPath(e);
    if (!path) {
      setRevert({ id: e.id, stage: "confirm", error: "되돌릴 파일이 없어요." });
      return;
    }
    setRevert((r) => (r ? { ...r, busy: true, error: undefined } : r));
    try {
      const res = await ipc.activityRevert(e.id, path, force);
      if (!res.applied) {
        setRevert({
          id: e.id,
          stage: "force",
          warning: res.warning,
          later: res.later_commits,
        });
        return;
      }
      setRevert(null);
      // 되돌림 커밋이 새 행으로 들어온다 — 첫 페이지만 다시.
      const rows = await ipc.activityFeed({ scopeProject: scope ?? undefined, limit });
      setEntries(rows);
      setExhausted(rows.length < limit);
    } catch (err) {
      setRevert((r) =>
        r ? { ...r, busy: false, error: String(err).slice(0, 200) } : r,
      );
    }
  }

  // J/K/↩/Z — 입력 포커스 없을 때만.
  useEffect(() => {
    function onKey(ev: KeyboardEvent) {
      if (ev.metaKey || ev.ctrlKey || ev.altKey) return;
      if (ev.key === "Escape") {
        if (revert) {
          ev.preventDefault();
          setRevert(null);
        }
        return;
      }
      if (inEditable()) return;
      const k = ev.key.toLowerCase();
      if (k !== "j" && k !== "k" && k !== "enter" && k !== "z") return;
      if (groupOrder.length === 0) return;
      const idx = activeKey ? groupOrder.indexOf(activeKey) : -1;
      if (k === "j" || k === "k") {
        ev.preventDefault();
        const next =
          k === "j"
            ? Math.min(groupOrder.length - 1, idx + 1)
            : Math.max(0, idx < 0 ? 0 : idx - 1);
        setActiveKey(groupOrder[next]);
        document
          .querySelector(`[data-feed-key="${CSS.escape(groupOrder[next])}"]`)
          ?.scrollIntoView({ block: "nearest" });
        return;
      }
      const g = activeKey ? groupByKey.get(activeKey) : undefined;
      const head = g?.entries[0];
      if (!head || !head.project || !head.domain) return;
      if (k === "enter") {
        ev.preventDefault();
        onOpenDoc(head.project, head.domain);
      } else if (k === "z") {
        ev.preventDefault();
        setRevert({ id: head.id, stage: "confirm" });
      }
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [groupOrder, groupByKey, activeKey, onOpenDoc, revert]);

  if (loading && all.length === 0) {
    return (
      <div className={cn("flex flex-col gap-2 p-3", className)}>
        {[0, 1, 2].map((i) => (
          <div key={i} className="h-9 animate-pulse rounded-md bg-surface-elevated" />
        ))}
      </div>
    );
  }
  if (error && all.length === 0) {
    return (
      <div className={cn("flex items-center gap-2 p-3 text-[12px] text-mute", className)}>
        <span className="truncate">피드를 읽지 못했어요: {error}</span>
        <button
          onClick={loadFirst}
          className="shrink-0 rounded-sm border border-hairline bg-surface-elevated px-2 py-0.5 text-[11px] text-body hover:border-hairline-strong hover:text-on-dark"
        >
          다시 시도
        </button>
      </div>
    );
  }
  if (all.length === 0) {
    return (
      <div className={cn("p-4 text-[13px] text-mute", className)}>
        어제 이후 기록이 없어요. ⌃Space 로 메모하거나 에이전트를 연결해 보세요
      </div>
    );
  }

  return (
    <div className={cn("flex flex-col", className)}>
      {sections.map((s) => (
        <div key={s.label}>
          <div className="px-3 pb-1 pt-2.5 text-[10px] font-medium uppercase tracking-[0.5px] text-stone">
            {s.label}
          </div>
          {s.groups.map((g) => {
            const head = g.entries[0];
            const isOpen = expanded.has(g.key);
            const rows = isOpen ? g.entries : [head];
            return (
              <div key={g.key} data-feed-key={g.key}>
                {rows.map((e, i) => (
                  <FeedRow
                    key={e.id}
                    entry={e}
                    bundle={i === 0 && g.entries.length > 1 ? g.entries.length : 0}
                    bundleOpen={isOpen}
                    nested={i > 0}
                    scoped={!!scope}
                    compact={compact}
                    active={activeKey === g.key && i === 0}
                    diffOpen={diffOpen.has(e.id)}
                    diff={diffs[e.id] ?? diffCache.current.get(e.id)}
                    revert={revert?.id === e.id ? revert : null}
                    onActivate={() => setActiveKey(g.key)}
                    onToggleBundle={() =>
                      setExpanded((s2) => {
                        const n = new Set(s2);
                        if (n.has(g.key)) n.delete(g.key);
                        else n.add(g.key);
                        return n;
                      })
                    }
                    onOpen={() => {
                      if (e.project && e.domain) onOpenDoc(e.project, e.domain);
                    }}
                    onToggleDiff={() => toggleDiff(e)}
                    onAskRevert={() => setRevert({ id: e.id, stage: "confirm" })}
                    onConfirmRevert={(force) => runRevert(e, force)}
                    onCancelRevert={() => setRevert(null)}
                  />
                ))}
              </div>
            );
          })}
        </div>
      ))}
      {!exhausted && (
        <button
          onClick={loadMore}
          disabled={loadingMore}
          className="mx-3 my-2 inline-flex h-8 items-center justify-center gap-1.5 rounded-md border border-hairline bg-surface-elevated text-[12px] text-body hover:border-hairline-strong hover:text-on-dark disabled:opacity-50"
        >
          {loadingMore && <Loader2 size={12} className="animate-spin" />}
          더 보기 · 이전 {limit}건
        </button>
      )}
      {error && all.length > 0 && (
        <div className="px-3 pb-2 text-[11px] text-accent-red">{error}</div>
      )}
    </div>
  );
}

function ActorIcon({ entry }: { entry: ActivityEntry }) {
  if (entry.actor === "human") {
    return <span className="inline-block h-2 w-2 rounded-full bg-on-dark" aria-label="사람" />;
  }
  if (entry.actor === "danbi") {
    return <Droplets size={12} className="text-mute" aria-label="단비" />;
  }
  switch (entry.client) {
    case "claude-code":
    case "claude_code":
      return <ClaudeIcon size={12} />;
    case "codex":
      return <CodexIcon size={12} />;
    case "cursor":
      return <CursorIcon size={12} />;
    default:
      return <Bot size={12} className="text-mute" aria-label="에이전트" />;
  }
}

function FeedRow({
  entry: e,
  bundle,
  bundleOpen,
  nested,
  scoped,
  compact,
  active,
  diffOpen,
  diff,
  revert,
  onActivate,
  onToggleBundle,
  onOpen,
  onToggleDiff,
  onAskRevert,
  onConfirmRevert,
  onCancelRevert,
}: {
  entry: ActivityEntry;
  /** 0 = 단독, N = 이 행이 N건 묶음의 머리. */
  bundle: number;
  bundleOpen: boolean;
  nested: boolean;
  scoped: boolean;
  compact: boolean;
  active: boolean;
  diffOpen: boolean;
  diff: DocDiff | "loading" | "error" | undefined;
  revert: {
    stage: "confirm" | "force";
    warning?: string;
    later?: number;
    busy?: boolean;
    error?: string;
  } | null;
  onActivate: () => void;
  onToggleBundle: () => void;
  onOpen: () => void;
  onToggleDiff: () => void;
  onAskRevert: () => void;
  onConfirmRevert: (force: boolean) => void;
  onCancelRevert: () => void;
}) {
  const openable = !!e.project && !!e.domain;
  const canRevert = openable && e.files.length > 0 && e.op !== "init" && e.op !== "migration";
  const btn =
    "inline-flex h-6 items-center rounded-sm border border-hairline bg-surface px-1.5 text-[11px] text-body hover:border-hairline-strong hover:text-on-dark";
  return (
    <div
      className={cn(
        "group border-b border-hairline last:border-b-0",
        nested && "bg-surface/60",
      )}
    >
      <div
        onMouseEnter={onActivate}
        onClick={onActivate}
        onDoubleClick={onOpen}
        className={cn(
          "flex min-h-[56px] flex-col justify-center gap-0.5 px-3 transition-colors",
          compact ? "py-1" : "py-1.5",
          nested && "pl-8",
          active ? "bg-surface-elevated" : "hover:bg-surface-elevated",
        )}
      >
        <div className="flex min-w-0 items-center gap-2">
          <span className="w-[38px] shrink-0 font-mono text-[11px] text-stone">
            {hhmm(e.ts)}
          </span>
          <span className="grid w-3.5 shrink-0 place-items-center">
            <ActorIcon entry={e} />
          </span>
          <span className="shrink-0 text-[12px] text-mute">{clientLabel(e.client)}</span>
          {!scoped && e.project && (
            <span className="shrink-0 rounded-xs bg-surface-elevated px-1.5 text-[10px] uppercase leading-[16px] text-on-dark-mute">
              {e.project}
            </span>
          )}
          <span
            className={cn(
              "shrink-0 rounded-xs px-1.5 py-px text-[10px] font-medium uppercase leading-none tracking-[0.2px]",
              feedOpTone(e.op),
            )}
          >
            {feedOpLabel(e.op)}
          </span>
          {e.target && (
            <span className="min-w-0 truncate text-[12px] text-body">{e.target}</span>
          )}
          {bundle > 1 && (
            <button
              onClick={(ev) => {
                ev.stopPropagation();
                onToggleBundle();
              }}
              className="ml-auto inline-flex shrink-0 items-center gap-0.5 text-[11px] text-mute hover:text-on-dark"
              title={bundleOpen ? "묶음 접기" : "묶음 펼치기"}
            >
              ({bundle}건 묶음){" "}
              {bundleOpen ? <ChevronDown size={11} /> : <ChevronRight size={11} />}
            </button>
          )}
        </div>
        <div className="flex min-w-0 items-center gap-2 pl-[52px]">
          {openable ? (
            <span className="shrink-0 font-mono text-[11px] text-body">{e.domain}</span>
          ) : (
            <span className="shrink-0 text-[11px] text-stone">문서 없음</span>
          )}
          {e.preview && (
            <span className="min-w-0 truncate text-[12px] text-mute">{e.preview}</span>
          )}
          {openable && (
            <span
              className={cn(
                "ml-auto flex shrink-0 items-center gap-1 transition-opacity",
                active ? "opacity-100" : "opacity-0 group-hover:opacity-100",
              )}
            >
              <button
                onClick={(ev) => {
                  ev.stopPropagation();
                  onOpen();
                }}
                className={btn}
              >
                열기
              </button>
              <button
                onClick={(ev) => {
                  ev.stopPropagation();
                  onToggleDiff();
                }}
                className={cn(btn, diffOpen && "border-hairline-strong text-on-dark")}
              >
                diff
              </button>
              {canRevert && (
                <button
                  onClick={(ev) => {
                    ev.stopPropagation();
                    onAskRevert();
                  }}
                  className={btn}
                  title="이 커밋이 이 문서에 한 변경만 되돌리기 (Z)"
                >
                  이 변경 되돌리기
                </button>
              )}
            </span>
          )}
        </div>
      </div>

      {diffOpen && (
        <div className="px-3 pb-2 pl-[64px]">
          {diff === "loading" || diff === undefined ? (
            <div className="h-9 animate-pulse rounded-md bg-surface-elevated" />
          ) : diff === "error" ? (
            <div className="text-[11px] text-accent-red">diff 를 읽지 못했어요.</div>
          ) : (
            <DiffBlock diff={diff} />
          )}
        </div>
      )}

      {revert && (
        <div className="mx-3 mb-2 ml-[64px] rounded-md border border-hairline bg-surface-elevated p-2.5 text-[12px]">
          {revert.stage === "confirm" ? (
            <div className="text-body">
              <span className="font-mono text-[11px] text-stone">{e.id.slice(0, 7)}</span>
              {" 이 "}
              <span className="font-mono">{e.domain}</span>
              {" 에 한 변경만 되돌릴까요? 새 커밋으로 남아요."}
            </div>
          ) : (
            <div className="text-body">
              {revert.warning ?? "이후 변경이 있어요."}{" "}
              <span className="text-mute">
                이후 변경 {revert.later ?? 0}건이 있어요 — 그래도 되돌리기
              </span>
            </div>
          )}
          {revert.error && (
            <div className="mt-1 text-[11px] text-accent-red">{revert.error}</div>
          )}
          <div className="mt-2 flex items-center gap-1.5">
            <button
              onClick={(ev) => {
                ev.stopPropagation();
                onConfirmRevert(revert.stage === "force");
              }}
              disabled={revert.busy}
              className="inline-flex h-7 items-center gap-1 rounded-sm border border-hairline-strong bg-surface px-2.5 text-[12px] text-on-dark hover:bg-surface-card disabled:opacity-50"
            >
              {revert.busy && <Loader2 size={11} className="animate-spin" />}
              {revert.stage === "force" ? "그래도 되돌리기" : "되돌리기"}
            </button>
            <button
              onClick={(ev) => {
                ev.stopPropagation();
                onCancelRevert();
              }}
              className="inline-flex h-7 items-center rounded-sm px-2 text-[12px] text-mute hover:text-on-dark"
            >
              취소 (esc)
            </button>
          </div>
        </div>
      )}
    </div>
  );
}
