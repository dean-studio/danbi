import { useCallback, useEffect, useMemo, useState } from "react";
import { ChevronDown, ChevronRight, ListChecks, RefreshCw, X } from "lucide-react";
import { ipc, type Checklist, type TodoInbox, type TodoItem } from "@/lib/ipc";
import { useProjectColor } from "@/lib/projectColor";
import { cn } from "@/lib/utils";
import { useApp } from "@/state/store";
import { TodoRow, daysAgo, sourceLabel, todoKey, useTodoToggle } from "@/main/pulseUi";

/** 할 일함 — vault 전체(또는 한 프로젝트)의 열린 할 일을 한 화면에.
 *  모으는 것: `- [ ]` 체크박스 · `### TODO`/`## 다음 할 일` 아래 목록 ·
 *  `다음 할 일:`/`TODO:` 줄. 열린 체크박스가 많은 계획 문서는 아래
 *  '체크리스트' 로 묶는다. 체크하면 원래 문서의 그 줄이 `[x]` 가 된다. */

const PERIODS: { key: number | null; label: string }[] = [
  { key: 7, label: "7일" },
  { key: 30, label: "30일" },
  { key: null, label: "전체" },
];
const PERIOD_KEY = "danbi:todo-period";
const GROUP_VISIBLE = 8;

function readPeriod(): number | null {
  try {
    const v = localStorage.getItem(PERIOD_KEY);
    if (v === "all") return null;
    const n = Number(v);
    return n === 7 || n === 30 ? n : 7;
  } catch {
    return 7;
  }
}

export function TodoView() {
  const tree = useApp((s) => s.tree);
  const selection = useApp((s) => s.selection);
  const selectDomain = useApp((s) => s.selectDomain);
  const selectProject = useApp((s) => s.selectProject);
  const todoRev = useApp((s) => s.todoRev);
  const archived = useApp((s) => s.cfg?.project_archived);
  const { toggle, busy } = useTodoToggle();

  const [scope, setScope] = useState<string | null>(selection.project);
  const [period, setPeriod] = useState<number | null>(readPeriod);
  const [data, setData] = useState<TodoInbox | null>(null);
  const [loading, setLoading] = useState(false);
  const [err, setErr] = useState<string | null>(null);
  const [gone, setGone] = useState<Set<string>>(new Set());

  useEffect(() => {
    try {
      localStorage.setItem(PERIOD_KEY, period === null ? "all" : String(period));
    } catch {
      /* 저장 못 해도 화면은 동작 */
    }
  }, [period]);

  const load = useCallback(async () => {
    setLoading(true);
    try {
      setData(await ipc.todoInbox(period, scope));
      setGone(new Set());
      setErr(null);
    } catch (e) {
      setErr(String(e));
    } finally {
      setLoading(false);
    }
  }, [period, scope]);

  // tree 가 바뀌면(에이전트가 새 TODO 를 씀) 조용히 다시 읽는다.
  useEffect(() => {
    load();
  }, [load, todoRev, tree, archived]);

  const items = useMemo(
    () => (data?.items ?? []).filter((t) => !gone.has(todoKey(t))),
    [data, gone],
  );
  const groups = useMemo(() => {
    const m = new Map<string, TodoItem[]>();
    for (const t of items) {
      const arr = m.get(t.project) ?? [];
      arr.push(t);
      m.set(t.project, arr);
    }
    return [...m.entries()];
  }, [items]);

  async function done(t: TodoItem) {
    setGone((g) => new Set(g).add(todoKey(t)));
    const ok = await toggle(t);
    if (!ok) {
      setGone((g) => {
        const n = new Set(g);
        n.delete(todoKey(t));
        return n;
      });
    }
  }

  const projects = useMemo(() => tree?.projects.map((p) => p.name) ?? [], [tree]);

  return (
    <div className="flex h-full flex-col">
      <header
        data-tauri-drag-region
        className="flex h-12 shrink-0 items-center justify-between border-b border-hairline px-6"
      >
        <div className="flex items-center gap-2">
          <span className="text-[15px] font-medium tracking-[0.2px] text-ink">할 일</span>
          {scope && (
            <span className="inline-flex h-6 items-center gap-1 rounded-full border border-hairline bg-surface-elevated pl-2.5 pr-1 text-[11px] text-on-dark">
              @{scope}
              <button
                onClick={() => setScope(null)}
                title="전체 보기"
                className="grid h-4 w-4 place-items-center rounded-full text-mute hover:text-on-dark"
              >
                <X size={10} />
              </button>
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
        <div className="mx-auto max-w-[880px] px-6 py-6">
          <div className="flex flex-wrap items-center gap-1.5">
            {PERIODS.map((p) => {
              const on = period === p.key;
              return (
                <button
                  key={p.label}
                  onClick={() => setPeriod(p.key)}
                  className={cn(
                    "inline-flex h-7 items-center rounded-full border px-3 text-[12px] transition-colors",
                    on
                      ? "border-hairline-strong bg-surface-card text-on-dark"
                      : "border-hairline bg-surface text-mute hover:text-on-dark",
                  )}
                >
                  {p.label}
                </button>
              );
            })}
            <label className="relative ml-1 inline-flex h-7 items-center rounded-full border border-hairline bg-surface pl-3 pr-7 text-[12px] text-mute hover:text-on-dark">
              <select
                value={scope ?? ""}
                onChange={(e) => setScope(e.target.value || null)}
                className="absolute inset-0 cursor-pointer opacity-0"
                aria-label="프로젝트"
              >
                <option value="">모든 프로젝트</option>
                {projects.map((p) => (
                  <option key={p} value={p}>
                    {p}
                  </option>
                ))}
              </select>
              <span className="pointer-events-none truncate">{scope ?? "모든 프로젝트"}</span>
              <ChevronDown size={11} className="pointer-events-none absolute right-2.5" />
            </label>
          </div>

          {data && (
            <div className="mt-3 text-caption-md text-mute">
              열린 할 일 {data.total_items - gone.size}
              {data.checklists.length > 0 && ` · 체크리스트 ${data.checklists.length}`}
              {data.archived_skipped > 0 && ` · 보관한 프로젝트 ${data.archived_skipped}개 제외`}
              {period !== null && " · 그 기간에 쓰거나 고친 문서만"}
            </div>
          )}
          {err && (
            <div className="mt-3 rounded-md border border-hairline bg-surface-elevated px-3 py-2 text-caption-md text-accent-red">
              {err}
            </div>
          )}

          {!data ? (
            <ul className="mt-4 flex flex-col gap-1">
              {[0, 1, 2].map((i) => (
                <li key={i} className="h-14 animate-pulse rounded-md border border-hairline bg-surface-elevated" />
              ))}
            </ul>
          ) : items.length === 0 && data.checklists.length === 0 ? (
            <div className="mt-12 flex flex-col items-center gap-2 text-center">
              <ListChecks size={22} className="text-stone" />
              <div className="text-[13px] text-mute">
                {period !== null ? `최근 ${period}일 안에는 열린 할 일이 없어요.` : "열린 할 일이 없어요."}
              </div>
              <div className="text-caption-sm text-stone">
                문서에 <span className="font-mono">- [ ] 할 일</span> 을 적거나, 에이전트가{" "}
                <span className="font-mono">다음 할 일:</span> 을 남기면 여기 모여요.
              </div>
            </div>
          ) : (
            <div className="mt-4 flex flex-col gap-4">
              {groups.map(([project, list]) => (
                <ProjectGroup
                  key={project}
                  project={project}
                  items={list}
                  busy={busy}
                  scoped={!!scope}
                  onDone={done}
                  onOpenDoc={(t) => selectDomain(t.project, t.domain)}
                  onOpenProject={() => selectProject(project)}
                />
              ))}
              {data.truncated && (
                <div className="text-center text-caption-sm text-stone">
                  최근 {data.items.length}개까지만 보여요 — 기간이나 프로젝트를 좁혀 보세요.
                </div>
              )}
              {data.checklists.length > 0 && (
                <Checklists
                  rows={data.checklists}
                  showProject={!scope}
                  onOpen={(c) => selectDomain(c.project, c.domain)}
                />
              )}
            </div>
          )}
        </div>
      </div>
    </div>
  );
}

function ProjectGroup({
  project,
  items,
  busy,
  scoped,
  onDone,
  onOpenDoc,
  onOpenProject,
}: {
  project: string;
  items: TodoItem[];
  busy: string | null;
  scoped: boolean;
  onDone: (t: TodoItem) => void;
  onOpenDoc: (t: TodoItem) => void;
  onOpenProject: () => void;
}) {
  const color = useProjectColor()(project);
  const [all, setAll] = useState(false);
  const shown = all ? items : items.slice(0, GROUP_VISIBLE);
  return (
    <section className="overflow-hidden rounded-lg border border-hairline bg-surface">
      {!scoped && (
        <button
          onClick={onOpenProject}
          className="flex w-full items-center gap-2 border-b border-hairline px-4 py-2.5 text-left transition-colors hover:bg-surface-elevated"
          title="프로젝트 홈 열기"
        >
          <span className="h-2 w-2 shrink-0 rounded-full" style={{ background: color }} />
          <span className="text-[13px] font-medium text-ink">{project}</span>
          <span className="font-mono text-[11px] text-stone">{items.length}</span>
          <span className="ml-auto text-[11px] text-stone">{daysAgo(items[0]?.at_ms)}</span>
          <ChevronRight size={12} className="text-mute" />
        </button>
      )}
      <ul className="divide-y divide-hairline">
        {shown.map((t) => (
          <TodoRow
            key={todoKey(t)}
            item={t}
            busy={busy === todoKey(t)}
            onDone={() => onDone(t)}
            onOpen={() => onOpenDoc(t)}
          />
        ))}
      </ul>
      {items.length > GROUP_VISIBLE && (
        <button
          onClick={() => setAll((v) => !v)}
          className="w-full border-t border-hairline px-4 py-2 text-left text-[11px] text-mute hover:text-on-dark"
        >
          {all ? "접기" : `+${items.length - GROUP_VISIBLE}개 더 보기`}
        </button>
      )}
    </section>
  );
}

function Checklists({
  rows,
  showProject,
  onOpen,
}: {
  rows: Checklist[];
  showProject: boolean;
  onOpen: (c: Checklist) => void;
}) {
  const projectColor = useProjectColor();
  return (
    <section className="overflow-hidden rounded-lg border border-hairline bg-surface">
      <div className="border-b border-hairline px-4 py-2.5">
        <div className="text-[13px] font-medium text-ink">진행 중인 체크리스트</div>
        <div className="text-caption-sm text-stone">
          열린 체크박스가 많은 계획 문서는 항목을 펼치지 않고 남은 수만 보여요.
        </div>
      </div>
      <ul className="divide-y divide-hairline">
        {rows.map((c) => {
          const total = c.open + c.done;
          const pct = Math.round((c.done / Math.max(1, total)) * 100);
          return (
            <li key={`${c.project}/${c.domain}`}>
              <button
                onClick={() => onOpen(c)}
                className="flex w-full items-center gap-3 px-4 py-2 text-left transition-colors hover:bg-surface-elevated"
                title={`${c.project}/${c.domain}`}
              >
                {showProject && (
                  <span
                    className="h-1.5 w-1.5 shrink-0 rounded-full"
                    style={{ background: projectColor(c.project) }}
                  />
                )}
                <span className="min-w-0 flex-1">
                  <span className="block truncate text-[13px] text-body">
                    {c.title || sourceLabel(c.domain)}
                  </span>
                  <span className="block truncate text-[11px] text-stone">
                    {showProject && `${c.project} · `}
                    <span className="font-mono">{c.domain}</span> · {daysAgo(c.at_ms)}
                  </span>
                </span>
                <span className="flex w-[120px] shrink-0 items-center gap-2">
                  <span className="block h-[3px] flex-1 overflow-hidden rounded-full bg-surface-elevated">
                    <span className="block h-full rounded-full bg-on-dark-mute" style={{ width: `${pct}%` }} />
                  </span>
                  <span className="w-[52px] text-right font-mono text-[11px] text-stone">
                    {c.done}/{total}
                  </span>
                </span>
              </button>
            </li>
          );
        })}
      </ul>
    </section>
  );
}
