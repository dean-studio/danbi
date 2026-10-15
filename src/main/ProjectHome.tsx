import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  Archive,
  Check,
  ChevronDown,
  ChevronRight,
  Loader2,
  Network,
  Plus,
  RefreshCw,
  Sparkles,
  Target,
  X,
} from "lucide-react";
import {
  ipc,
  type DayCounts,
  type Goal,
  type ProjectJournalView,
  type ProjectNode,
  type TriggerKind,
  type ProjectGroup,
} from "@/lib/ipc";
import { ActivityFeed } from "@/main/ActivityFeed";
import { ConnectStatusBand } from "@/main/ConnectSheet";
import { ReviewMini } from "@/main/ReviewMini";
import { projectIconOf } from "@/components/ProjectIconPicker";
import { ReindexProgressCard } from "@/main/Settings";
import { cn } from "@/lib/utils";
import { useApp } from "@/state/store";
import { McpInboundProjectMini } from "./McpInboundProjectMini";

/** 그룹이 없을 때 매번 새 배열을 만들지 않도록(zustand 선택자 안정성). */
const NO_GROUPS: ProjectGroup[] = [];

/** 프로젝트 홈. 헤더(이름 · [+ 새 노트] 흰 CTA 1개) → 히어로(아이콘·
 *  이름·목표 한 줄) → 에이전트 상태 띠(연결 시트 입구) → 이 프로젝트
 *  액션 → 지난 7일 → MCP mini → 받아 적은 것. 연결/스킬 버튼 8개와
 *  ConnectCard 는 연결 시트(ConnectSheet)로 흡수됐다. */
export function ProjectHome({
  project,
  onAddDomain,
  onOpenGraph,
}: {
  project: string;
  onAddDomain: (project: string) => void;
  onOpenGraph: (project: string) => void;
}) {
  const tree = useApp((s) => s.tree);
  const selectDomain = useApp((s) => s.selectDomain);
  const openConnectSheet = useApp((s) => s.openConnectSheet);
  const setView = useApp((s) => s.setView);

  const projectNode: ProjectNode | undefined = useMemo(
    () => tree?.projects.find((p) => p.name === project),
    [tree, project],
  );

  if (!projectNode) {
    return (
      <div className="flex h-full items-center justify-center text-caption-sm text-stone">
        프로젝트를 찾을 수 없어요.
      </div>
    );
  }

  return (
    <div className="flex h-full flex-col">
      <header
        data-tauri-drag-region
        className="flex h-12 shrink-0 items-center justify-between border-b border-hairline px-5"
      >
        <div className="flex min-w-0 items-center gap-2">
          <span className="truncate text-[13px] font-medium tracking-[0.2px] text-ink">
            {project}
          </span>
        </div>
        {/* 흰 CTA 는 뷰포트당 1개 — 새 노트(⌘N). */}
        <button
          onClick={() => onAddDomain(project)}
          title="새 노트 (⌘N)"
          className="inline-flex h-8 items-center gap-1.5 rounded-md bg-primary px-3 text-[12px] font-medium text-on-primary hover:bg-primary-pressed"
        >
          <Plus size={13} /> 새 노트
        </button>
      </header>

      <div className="flex-1 min-h-0 overflow-auto">
        <div className="mx-auto max-w-[820px] px-6 py-8">
          <ProjectHero project={project} />
          <div className="mt-5">
            <ConnectStatusBand
              project={project}
              onOpenSheet={() => openConnectSheet(project)}
            />
          </div>
          {/* 이 프로젝트 피드 | 지난 7일 · 검토 N (화면 B) */}
          <div className="mt-6 grid gap-5" style={{ gridTemplateColumns: "minmax(0,1fr) 260px" }}>
            <ActivityFeed
              scopeProject={project}
              limit={30}
              compact
              onOpenDoc={selectDomain}
              className="rounded-lg border border-hairline bg-surface"
            />
            <div className="flex min-w-0 flex-col gap-4">
              <div className="rounded-lg border border-hairline bg-surface px-4 py-4">
                <SevenDayBar project={project} embedded />
              </div>
              <ReviewMini
                project={project}
                max={3}
                onOpenDoc={selectDomain}
                onOpenAll={() => setView("review")}
              />
            </div>
          </div>
          <div className="mt-6">
            <RecentDocsCard project={projectNode} onOpen={(d) => selectDomain(project, d)} />
          </div>
          <div className="mt-6">
            <ProjectActions
              project={project}
              onOpenGraph={() => onOpenGraph(project)}
            />
          </div>
          <Collapsible id="journal" title="받아 적은 것" className="mt-6">
            <AutoJournalPanel
              project={project}
              onOpen={(d) => selectDomain(project, d)}
            />
          </Collapsible>
          <Collapsible id="usage" title="사용량 (이 프로젝트 MCP 저장 토큰)" className="mt-4">
            <McpInboundProjectMini
              project={project}
              onOpenDomain={(d) => selectDomain(project, d)}
            />
          </Collapsible>
        </div>
      </div>
    </div>
  );
}

/** 최근 문서 — daily 제외, mtime 순 2열 카드, H1 제목 표시. 뼈대 문서
 *  (purpose/schema/SKILL) 는 아래 칩 행. */
const SKELETON_DOCS = ["purpose.md", "schema.md", "SKILL.md"];
function RecentDocsCard({
  project,
  onOpen,
}: {
  project: ProjectNode;
  onOpen: (domain: string) => void;
}) {
  const docs = useMemo(() => {
    const all: { name: string; ms: number; title: string | null }[] = [];
    const push = (d: { name: string; modified_ms?: number | null; title?: string | null }) => {
      if (d.name.startsWith("daily/") || SKELETON_DOCS.includes(d.name)) return;
      all.push({ name: d.name, ms: d.modified_ms ?? 0, title: d.title ?? null });
    };
    project.domains.forEach(push);
    for (const f of project.subfolders) {
      if (f.name === "daily") continue;
      f.domains.forEach(push);
      for (const g of f.subfolders ?? []) g.domains.forEach(push);
    }
    all.sort((a, b) => b.ms - a.ms);
    return all.slice(0, 6);
  }, [project]);
  const skeleton = SKELETON_DOCS.filter((n) => project.domains.some((d) => d.name === n));

  return (
    <section className="rounded-lg border border-hairline bg-surface p-4">
      <div className="mb-2.5 flex items-center justify-between">
        <span className="text-[13px] font-medium text-ink">최근 문서</span>
        <span className="text-caption-sm text-stone">daily 제외 · 수정순</span>
      </div>
      {docs.length === 0 ? (
        <div className="text-[12px] text-mute">
          아직 문서가 없어요. 헤더의 [새 노트] 로 시작하세요.
        </div>
      ) : (
        <div className="grid grid-cols-2 gap-2">
          {docs.map((d) => (
            <button
              key={d.name}
              onClick={() => onOpen(d.name)}
              title={d.name}
              className="flex min-w-0 flex-col gap-0.5 rounded-md border border-hairline bg-surface-elevated px-3 py-2 text-left transition-colors hover:border-hairline-strong"
            >
              <span className="truncate text-[13px] text-ink">
                {d.title ?? d.name.replace(/\.md$/, "")}
              </span>
              <span className="flex items-center justify-between gap-2">
                <span className="truncate font-mono text-[11px] text-mute">{d.name}</span>
                <span className="shrink-0 text-[11px] text-stone">
                  {d.ms ? relAgo(d.ms) : ""}
                </span>
              </span>
            </button>
          ))}
        </div>
      )}
      {skeleton.length > 0 && (
        <div className="mt-2.5 flex flex-wrap items-center gap-1.5">
          {skeleton.map((n) => (
            <button
              key={n}
              onClick={() => onOpen(n)}
              className="inline-flex h-6 items-center rounded-full border border-hairline bg-surface-elevated px-2.5 font-mono text-[11px] text-body hover:border-hairline-strong hover:text-on-dark"
            >
              {n}
            </button>
          ))}
        </div>
      )}
    </section>
  );
}

function relAgo(ms: number): string {
  const diff = Math.max(0, Date.now() - ms);
  const m = Math.floor(diff / 60_000);
  if (m < 1) return "방금";
  if (m < 60) return `${m}m`;
  const h = Math.floor(m / 60);
  if (h < 24) return `${h}h`;
  return `${Math.floor(h / 24)}d`;
}

/** 접힘 섹션 — 상태는 localStorage. 기본 접힘. */
function Collapsible({
  id,
  title,
  className,
  children,
}: {
  id: string;
  title: string;
  className?: string;
  children: React.ReactNode;
}) {
  const key = `danbi:project-home-open:${id}`;
  const [open, setOpen] = useState(() => {
    try {
      return localStorage.getItem(key) === "1";
    } catch {
      return false;
    }
  });
  return (
    <section className={className}>
      <button
        onClick={() =>
          setOpen((v) => {
            try {
              localStorage.setItem(key, v ? "0" : "1");
            } catch {
              /* ignore */
            }
            return !v;
          })
        }
        className="flex w-full items-center gap-1.5 rounded-sm px-1 py-1 text-left text-[13px] font-medium text-body hover:text-on-dark"
      >
        {open ? <ChevronDown size={13} className="text-mute" /> : <ChevronRight size={13} className="text-mute" />}
        {title}
      </button>
      {open && <div className="mt-2">{children}</div>}
    </section>
  );
}

/** 히어로 — 그룹 라벨(소문자 대문자) + 아이콘 + 프로젝트명 + 목표 한 줄.
 *  Goals 의 노란 카드는 폐지하고 inline edit 한 줄로(UI 검토 #8). */
function ProjectHero({ project }: { project: string }) {
  const groups = useApp((s) => s.cfg?.project_groups) ?? NO_GROUPS;
  const iconName = useApp((s) => s.cfg?.project_icons?.[project] ?? null);
  const Icon = projectIconOf(iconName);
  const groupLabel = useMemo(() => {
    const g = groups.find((g) => g.projects.includes(project));
    return g?.label ?? null;
  }, [groups, project]);

  return (
    <div className="flex flex-col gap-2">
      {groupLabel && (
        <div className="text-[12px] font-semibold uppercase tracking-[0.6px] text-mute">
          {groupLabel}
        </div>
      )}
      <div className="flex min-w-0 items-center gap-3">
        <span className="grid h-9 w-9 shrink-0 place-items-center rounded-md bg-surface-elevated text-on-dark-mute">
          <Icon size={20} />
        </span>
        <h1 className="truncate text-[28px] font-semibold leading-[1.15] tracking-[-0.01em] text-ink">
          {project}
        </h1>
      </div>
      <GoalsLine project={project} />
    </div>
  );
}

/** 목표 한 줄 — "목표: A · B [편집]". 편집을 열면 목록 + 추가/보관.
 *  Claude 세션에 자동 노출되는 goals 의 가벼운 입구. accent 없음. */
function GoalsLine({ project }: { project: string }) {
  const [goals, setGoals] = useState<Goal[]>([]);
  const [open, setOpen] = useState(false);
  const [draft, setDraft] = useState("");
  const [busyId, setBusyId] = useState<string | null>(null);

  const load = useCallback(async () => {
    try {
      setGoals(await ipc.goalsList(project));
    } catch (e) {
      console.error("[danbi] goals list failed", e);
      setGoals([]);
    }
  }, [project]);
  useEffect(() => {
    load();
  }, [load]);

  async function handleAdd() {
    const title = draft.trim();
    if (!title) return;
    try {
      await ipc.goalsAdd(project, title);
      setDraft("");
      await load();
    } catch (e) {
      console.error("[danbi] goals add failed", e);
    }
  }
  async function handleArchive(id: string) {
    setBusyId(id);
    try {
      await ipc.goalsArchive(project, id);
      await load();
    } finally {
      setBusyId(null);
    }
  }

  return (
    <div className="flex flex-col gap-1.5">
      <div className="flex min-w-0 items-center gap-2 text-[13px]">
        <Target size={13} className="shrink-0 text-mute" />
        <span className="shrink-0 text-mute">목표:</span>
        <span className="min-w-0 flex-1 truncate text-body">
          {goals.length > 0
            ? goals.map((g) => g.title).join(" · ")
            : "아직 없어요 — 지금 뭘 하려는지 한 줄이면 Claude 세션에 자동으로 노출돼요"}
        </span>
        <button
          onClick={() => setOpen((v) => !v)}
          className="shrink-0 text-[11px] text-mute hover:text-on-dark"
        >
          {open ? "닫기" : "편집"}
        </button>
      </div>
      {open && (
        <div className="rounded-md border border-hairline bg-surface-elevated p-3">
          {goals.length > 0 && (
            <ul className="flex flex-col gap-1">
              {goals.map((g) => (
                <li
                  key={g.id}
                  className="group flex items-start justify-between gap-3 text-[13px]"
                >
                  <div className="flex min-w-0 flex-col">
                    <span className="truncate text-ink">{g.title}</span>
                    {g.note && (
                      <span className="truncate text-[11px] text-stone">{g.note}</span>
                    )}
                  </div>
                  <button
                    type="button"
                    disabled={busyId === g.id}
                    onClick={() => handleArchive(g.id)}
                    className="shrink-0 rounded-sm p-1 text-stone opacity-0 transition group-hover:opacity-100 hover:text-on-dark disabled:opacity-30"
                    title="보관"
                  >
                    {busyId === g.id ? (
                      <Loader2 className="h-3.5 w-3.5 animate-spin" />
                    ) : (
                      <Archive className="h-3.5 w-3.5" />
                    )}
                  </button>
                </li>
              ))}
            </ul>
          )}
          <div className={cn("flex items-center gap-2", goals.length > 0 && "mt-2")}>
            <input
              value={draft}
              onChange={(e) => setDraft(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter" && !e.nativeEvent.isComposing) {
                  e.preventDefault();
                  handleAdd();
                } else if (e.key === "Escape") {
                  setOpen(false);
                }
              }}
              placeholder="예: v0.8.8 범위 확정"
              className="h-8 flex-1 rounded-md border border-hairline bg-surface px-2.5 text-[13px] text-ink placeholder:text-stone focus:border-hairline-strong focus:outline-none"
            />
            <button
              type="button"
              onClick={handleAdd}
              disabled={!draft.trim()}
              className="inline-flex h-8 items-center rounded-md border border-hairline bg-surface px-3 text-[12px] text-body hover:border-hairline-strong hover:text-on-dark disabled:opacity-50"
            >
              추가
            </button>
          </div>
        </div>
      )}
    </div>
  );
}

function AutoJournalPanel({
  project,
  onOpen,
}: {
  project: string;
  onOpen: (domain: string) => void;
}) {
  // Stale-while-revalidate: render cached view instantly, fire one
  // background fetch to refresh. Re-entering the same project shows
  // results with zero perceptible delay.
  const cached = useApp((s) => s.projectJournalCache[project] ?? null);
  const setProjectJournal = useApp((s) => s.setProjectJournal);
  const [view, setView] = useState<ProjectJournalView | null>(cached);
  const [loading, setLoading] = useState(!cached);
  const [err, setErr] = useState<string | null>(null);
  // chip 클릭 시 해당 kind 만 피드에 노출. null = 전체.
  const [filterKind, setFilterKind] = useState<TriggerKind | null>(null);

  // Sync local state when the cache updates (e.g. another panel
  // refreshed it).
  useEffect(() => {
    if (cached) setView(cached);
  }, [cached]);

  const refresh = useCallback(async () => {
    try {
      // Only show the loading spinner when we have nothing to render.
      if (!cached) setLoading(true);
      const v = await ipc.projectJournalView(project);
      setView(v);
      setProjectJournal(project, v);
      setErr(null);
    } catch (e) {
      setErr(String(e));
    } finally {
      setLoading(false);
    }
    // Intentionally exclude `cached` so the latest cache value is read
    // fresh on each invocation without retriggering the effect.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [project, setProjectJournal]);

  useEffect(() => {
    refresh();
  }, [refresh]);

  if (loading && !view) {
    return (
      <div className="rounded-lg border border-hairline bg-surface p-8 text-center text-caption-sm text-mute">
        읽는 중…
      </div>
    );
  }
  if (err) {
    return (
      <div className="rounded-lg border border-hairline bg-surface-elevated p-4 font-mono text-[12px] text-accent-red">
        {err}
      </div>
    );
  }
  if (!view) return null;

  // Hero 카운트는 아래 피드(지난 7일)와 같은 horizon 으로 묶는다.
  // today_counts 만 쓰면 "오늘 0건" 인데 피드엔 어제·그제 항목이 떠
  // 보이는 미스매치가 생긴다 — 사용자가 "왜 카운트가 안 되나?" 하는
  // 그 케이스.
  const total = view.last_7_days.reduce<typeof view.today_counts>(
    (acc, d) => ({
      date: acc.date,
      decision: acc.decision + d.decision,
      cause: acc.cause + d.cause,
      todo: acc.todo + d.todo,
      knowhow: acc.knowhow + d.knowhow,
      pitfall: acc.pitfall + d.pitfall,
      other: acc.other + d.other,
    }),
    {
      date: view.today,
      decision: 0,
      cause: 0,
      todo: 0,
      knowhow: 0,
      pitfall: 0,
      other: 0,
    },
  );
  const weekTotal =
    total.decision +
    total.cause +
    total.todo +
    total.knowhow +
    total.pitfall +
    total.other;

  return (
    <div className="flex flex-col gap-6">
      {/* Hero — 지난 7일 합계와 한 문장 식별 */}
      <section className="rounded-lg border border-hairline bg-surface p-6">
        <div className="text-caption-sm uppercase tracking-[0.6px] text-mute">
          지난 7일 · {view.today} 기준
        </div>
        <div className="mt-1 flex items-baseline gap-3">
          <h1 className="text-[36px] font-semibold leading-[1] text-ink">
            {weekTotal}
          </h1>
          <span className="text-[14px] text-body">
            건의 작업이 단비에 자동 기록됐어요
          </span>
        </div>
        <div className="mt-5 grid grid-cols-6 gap-2">
          {(
            [
              ["decision", "결정", total.decision, "blue"],
              ["cause", "원인", total.cause, "red"],
              ["todo", "TODO", total.todo, "yellow"],
              ["knowhow", "노하우", total.knowhow, "green"],
              ["pitfall", "재발 방지", total.pitfall, "violet"],
              ["other", "메모", total.other, "neutral"],
            ] as const
          ).map(([kind, label, n, tone]) => (
            <CountChip
              key={kind}
              label={label}
              n={n}
              tone={tone}
              active={filterKind === kind}
              // 0개면 클릭해도 보여줄 게 없어서 비활성화. 토글 동작은
              // chip 자체에서: 같은 kind 면 null 로 해제, 다르면 그 kind 로 전환.
              onClick={
                n > 0
                  ? () => setFilterKind((k) => (k === kind ? null : kind))
                  : undefined
              }
            />
          ))}
        </div>
      </section>

      {/* Recent feed */}
      <section>
        {(() => {
          const filtered = filterKind
            ? view.recent_entries.filter((e) => e.kind === filterKind)
            : view.recent_entries;
          // 필터 없을 땐 8개로 축약, 필터 있을 땐 전부 노출 (해당 kind 만이라
          // 갯수 제한 의미 없음 — 사용자가 chip 으로 명시적 의도를 밝힌 상태).
          const shown = filterKind ? filtered : filtered.slice(0, 8);
          return (
            <>
              <header className="mb-2 flex items-center justify-between">
                <div className="flex items-center gap-2">
                  <h2 className="text-[14px] font-medium text-ink">
                    최근 받아 적은 것
                  </h2>
                  {filterKind && (
                    <button
                      type="button"
                      onClick={() => setFilterKind(null)}
                      className="inline-flex items-center gap-1 rounded-sm border border-hairline bg-surface-elevated px-1.5 py-0.5 text-[11px] text-body hover:border-hairline-strong hover:text-on-dark"
                      title="필터 해제"
                    >
                      <KindBadge kind={filterKind} /> ×
                    </button>
                  )}
                </div>
                <span className="text-caption-sm text-mute">
                  {filterKind
                    ? `${filtered.length}개 · 지난 7일`
                    : "지난 7일 · 최신 8개"}
                </span>
              </header>
              {shown.length === 0 ? (
                filterKind ? (
                  <div className="rounded-md border border-hairline bg-surface p-6 text-center text-[13px] text-mute">
                    이 카테고리에 해당하는 기록이 없어요
                  </div>
                ) : (
                  <EmptyJournal project={project} />
                )
              ) : (
                <ul className="flex flex-col gap-2">
                  {shown.map((e, i) => (
              <li
                key={`${e.date}-${i}-${e.title}`}
                className="rounded-md border border-hairline bg-surface p-3 transition-colors hover:border-hairline-strong"
              >
                <button
                  type="button"
                  onClick={() => onOpen(`daily/${e.date}.md`)}
                  className="flex w-full flex-col items-start gap-1 text-left"
                >
                  <div className="flex w-full items-center gap-2">
                    <KindBadge kind={e.kind} />
                    <span className="flex-1 truncate text-[14px] font-medium text-ink">
                      {e.title}
                    </span>
                    <span className="shrink-0 text-caption-sm text-mute">
                      {e.date}
                    </span>
                  </div>
                  {e.preview && (
                    <p className="line-clamp-2 text-[13px] leading-[1.6] text-body">
                      {e.preview}
                    </p>
                  )}
                </button>
              </li>
                  ))}
                </ul>
              )}
            </>
          );
        })()}
      </section>

    </div>
  );
}

/** 7-day activity sparkline. With `embedded` true the outer card
 *  chrome is dropped — used inside the shared utility card. */
function SevenDayBar({
  project,
  embedded = false,
}: {
  project: string;
  embedded?: boolean;
}) {
  // Reuse the AutoJournalPanel's fetch via the shared cache — no
  // second IPC trip.
  const view = useApp((s) => s.projectJournalCache[project] ?? null);

  const days = view ? [...view.last_7_days].reverse() : [];
  const sumOf = (d: DayCounts) =>
    d.decision + d.cause + d.todo + d.knowhow + d.pitfall + d.other;
  const max = Math.max(1, ...days.map(sumOf));
  const totalWeek = days.reduce((a, d) => a + sumOf(d), 0);
  const dowOf = (iso: string) => {
    const dt = new Date(`${iso}T00:00:00`);
    return ["일", "월", "화", "수", "목", "금", "토"][dt.getDay()];
  };
  // GitHub-style heatmap: opacity scaled to count / max.
  const tone = (sum: number) => {
    if (sum === 0) return "bg-surface-elevated";
    const r = sum / max;
    if (r < 0.34) return "bg-accent-blue/25";
    if (r < 0.67) return "bg-accent-blue/55";
    return "bg-accent-blue";
  };

  const inner = (
    <div>
      <div className="mb-4 flex items-baseline justify-between">
        <span className="text-[14px] font-medium text-ink">지난 7일</span>
        <span className="text-[13px] text-mute">
          총 <span className="text-on-dark tabular-nums">{totalWeek}</span> 건
        </span>
      </div>
      {!view ? (
        <div className="text-[13px] text-mute">…</div>
      ) : (
        <div className="flex justify-between gap-2">
          {days.map((d) => {
            const sum = sumOf(d);
            const isToday = d.date === view.today;
            return (
              <div
                key={d.date}
                className="flex flex-1 flex-col items-center gap-1.5"
                title={`${d.date} — ${sum}건`}
              >
                <span
                  className={cn(
                    "text-[12px] leading-none",
                    isToday
                      ? "font-semibold text-accent-blue"
                      : "text-mute",
                  )}
                >
                  {dowOf(d.date)}
                </span>
                <div
                  className={cn(
                    "grid h-12 w-full place-items-center rounded-md transition-colors",
                    tone(sum),
                    isToday && "ring-1 ring-accent-blue",
                  )}
                >
                  {sum > 0 && (
                    <span
                      className={cn(
                        "text-[14px] font-semibold tabular-nums",
                        sum / max < 0.34 ? "text-accent-blue" : "text-on-dark",
                      )}
                    >
                      {sum}
                    </span>
                  )}
                </div>
                <span className="text-[11px] leading-none text-stone tabular-nums">
                  {d.date.slice(5)}
                </span>
              </div>
            );
          })}
        </div>
      )}
    </div>
  );

  if (embedded) return <div>{inner}</div>;
  return (
    <section className="rounded-md border border-hairline bg-surface px-5 py-4">
      {inner}
    </section>
  );
}

function CountChip({
  label,
  n,
  tone,
  active = false,
  onClick,
}: {
  label: string;
  n: number;
  tone: "blue" | "red" | "yellow" | "green" | "violet" | "neutral";
  /** chip 이 현재 필터로 선택돼 있는지 — 살짝 강조된 boder + bg 로 시각화. */
  active?: boolean;
  /** 누르면 필터 토글. 없으면 정적 chip (기존 동작 유지). */
  onClick?: () => void;
}) {
  const baseCls =
    tone === "blue"
      ? "border-accent-blue/30 text-accent-blue"
      : tone === "red"
        ? "border-accent-red/30 text-accent-red"
        : tone === "yellow"
          ? "border-accent-yellow/30 text-accent-yellow"
          : tone === "green"
            ? "border-accent-green/30 text-accent-green"
            : tone === "violet"
              ? "border-on-dark/20 text-on-dark"
              : "border-hairline-strong text-stone";
  const activeCls =
    tone === "blue"
      ? "border-accent-blue bg-accent-blue-soft"
      : tone === "red"
        ? "border-accent-red bg-accent-red-soft"
        : tone === "yellow"
          ? "border-accent-yellow bg-accent-yellow-soft"
          : tone === "green"
            ? "border-accent-green bg-accent-green-soft"
            : tone === "violet"
              ? "border-on-dark bg-surface-elevated"
              : "border-on-dark-mute bg-surface-elevated";
  const cls = active ? activeCls : baseCls;
  const interactive = !!onClick;
  // n === 0 일 때 onClick 이 undefined 로 들어옴. 시각적으로도 흐리게
  // 처리해서 "눌러도 의미 없다" 는 시그널을 명확히.
  const dimmed = !interactive;
  return (
    <button
      type="button"
      onClick={onClick}
      disabled={!interactive}
      className={cn(
        "flex flex-col items-start gap-0.5 rounded-md border bg-surface px-3 py-2 text-left transition-colors",
        cls,
        interactive &&
          !active &&
          "hover:border-hairline-strong hover:bg-surface-elevated",
        dimmed && "cursor-default opacity-40",
      )}
    >
      <span className="text-[11px] font-medium tracking-[0.2px] opacity-80">
        {label}
      </span>
      <span className="text-[20px] font-semibold leading-tight tabular-nums">
        {n}
      </span>
    </button>
  );
}

function KindBadge({ kind }: { kind: TriggerKind }) {
  const map: Record<TriggerKind, { label: string; cls: string }> = {
    decision: { label: "결정", cls: "bg-accent-blue-soft text-accent-blue" },
    cause: { label: "원인", cls: "bg-accent-red-soft text-accent-red" },
    todo: { label: "TODO", cls: "bg-accent-yellow-soft text-accent-yellow" },
    knowhow: {
      label: "노하우",
      cls: "bg-accent-green-soft text-accent-green",
    },
    pitfall: {
      label: "재발 방지",
      cls: "bg-accent-blue-soft text-on-dark",
    },
    other: {
      label: "메모",
      cls: "bg-surface-elevated text-on-dark-mute",
    },
  };
  const m = map[kind];
  return (
    <span
      className={cn(
        "shrink-0 rounded-xs px-1.5 py-0.5 text-[10px] font-semibold uppercase tracking-[0.5px]",
        m.cls,
      )}
    >
      {m.label}
    </span>
  );
}

function EmptyJournal({ project }: { project: string }) {
  return (
    <div className="flex flex-col items-center gap-3 rounded-lg border border-hairline border-dashed bg-surface px-6 py-10 text-center">
      <Sparkles size={20} className="text-accent-blue" />
      <div className="flex flex-col gap-1">
        <h3 className="text-[15px] font-medium text-ink">
          아직 받아 적은 게 없어요
        </h3>
        <p className="max-w-[420px] text-[13px] leading-[1.6] text-body">
          이 프로젝트를 Claude Code MCP 와 연결하면, 거기서 내린 결정과 디버깅
          흔적이 자동으로 <span className="text-on-dark">{project}/daily/</span>{" "}
          에 누적돼요. 첫 항목이 들어오면 여기서 한눈에 볼 수 있어요.
        </p>
      </div>
    </div>
  );
}


/** 이 프로젝트 안에서 — 그래프 · 관련 노트 제안 · 재인덱싱. 전부 hairline
 *  톤(재인덱싱 accent 강조 폐지). 프로젝트 내 검색 박스는 ⌘K 팔레트가 이
 *  화면에서 열리면 @프로젝트 칩을 자동 선택하므로 제거. */
function ProjectActions({
  project,
  onOpenGraph,
}: {
  project: string;
  onOpenGraph: () => void;
}) {
  const aiConnected = useApp((s) => !!s.cfg?.embed_provider);
  const setBgJob = useApp((s) => s.setBgJob);
  const pushNotification = useApp((s) => s.pushNotification);
  const bgJob = useApp((s) => s.bgJob);
  const ghostScanning =
    bgJob?.kind === "ghost" &&
    bgJob.status === "running" &&
    bgJob.project === project;
  const [ghostFlash, setGhostFlash] = useState<string | null>(null);

  function runGhostScan() {
    if (!aiConnected) {
      setGhostFlash("AI 연동을 먼저 켜세요");
      window.setTimeout(() => setGhostFlash(null), 2400);
      return;
    }
    const proj = project;
    setGhostFlash(null);
    setBgJob({
      kind: "ghost",
      status: "running",
      project: proj,
      startedAt: Date.now(),
    });
    ipc
      .ghostScan(proj)
      .then((store) => {
        const pending = store.links.filter(
          (l) => l.status === "pending",
        ).length;
        setBgJob({
          kind: "ghost",
          status: "done",
          project: proj,
          pendingCount: pending,
          finishedAt: Date.now(),
        });
        pushNotification({
          tone: "ok",
          title: pending > 0
            ? `관련 노트 ${pending}개 제안`
            : "추가 제안 없음",
          body:
            pending > 0
              ? `${proj} — 그래프 뷰에서 점선으로 보임`
              : `${proj} — vault 가 깔끔해요`,
          action:
            pending > 0
              ? { kind: "open-graph", project: proj }
              : undefined,
        });
      })
      .catch((e) => {
        setBgJob({
          kind: "ghost",
          status: "error",
          project: proj,
          message: String(e).slice(0, 240),
          finishedAt: Date.now(),
        });
        pushNotification({
          tone: "err",
          title: "ghost 스캔 실패",
          body: `${proj} — ${String(e).slice(0, 120)}`,
        });
      });
  }


  return (
    <section className="rounded-lg border border-hairline bg-surface-elevated p-5">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <div className="text-[14px] font-semibold uppercase tracking-[0.5px] text-mute">
          이 프로젝트 안에서
        </div>
        <div className="flex flex-wrap items-center gap-2">
          <button
            onClick={onOpenGraph}
            title={`${project} 그래프 보기`}
            className="inline-flex h-10 items-center gap-2 rounded-md border border-hairline bg-surface px-4 text-[14px] font-medium text-body hover:border-hairline-strong hover:text-on-dark"
          >
            <Network size={16} /> 그래프
          </button>
          {/* Ghost links 자동 제안 — AI 연동 켰을 때만 활성. 결과는 그래프
              점선 + 검토함에서 승인/무시. */}
          <button
            onClick={runGhostScan}
            disabled={ghostScanning}
            title={
              aiConnected
                ? "vault 안에서 연결되면 좋을 문서 쌍을 LLM 이 제안 (그래프에 점선으로)"
                : "AI 연동을 켜면 활성화됩니다"
            }
            className={cn(
              "inline-flex h-10 items-center gap-2 rounded-md border px-4 text-[14px] font-medium transition-colors",
              aiConnected
                ? "border-hairline bg-surface text-body hover:border-hairline-strong hover:text-on-dark"
                : "border-hairline bg-surface/40 text-stone",
            )}
          >
            {ghostScanning ? (
              <Loader2 size={16} className="animate-spin" />
            ) : (
              <Sparkles size={16} />
            )}
            {ghostScanning ? "스캔 중…" : "관련 노트 제안"}
          </button>
          <ProjectReindexButton project={project} compact={false} />
        </div>
      </div>
      {ghostFlash && (
        <div className="mt-3 rounded-md border border-hairline bg-surface px-3 py-2 text-[13px] text-mute">
          {ghostFlash}
        </div>
      )}
    </section>
  );
}

function ProjectReindexButton({
  project,
  compact = true,
}: {
  project: string;
  compact?: boolean;
}) {
  const [open, setOpen] = useState(false);

  return (
    <>
      <button
        onClick={() => setOpen(true)}
        title={`${project} 만 재인덱싱 (변경된 파일만 embedding)`}
        className={cn(
          "inline-flex items-center gap-2 rounded-md transition-colors",
          compact
            ? "h-6 gap-1 rounded-sm border border-hairline bg-surface-elevated px-2 text-[11px] text-body hover:border-hairline-strong hover:text-on-dark"
            : // 유지보수 작업이라 그래프와 같은 hairline 톤 (accent 강조
              // 폐지 — UI 검토 #8).
              "h-10 border border-hairline bg-surface px-4 text-[14px] font-medium text-body hover:border-hairline-strong hover:text-on-dark",
        )}
      >
        <RefreshCw size={compact ? 10 : 16} />
        재인덱싱
      </button>
      <ReindexModal
        open={open}
        project={project}
        onClose={() => setOpen(false)}
      />
    </>
  );
}

/** Modal wrapper around the reindex flow. Mirrors the Settings → 임베딩
 *  panel: live progress card while running (powered by the same
 *  vector:reindex_progress event stream), success / error summary at the
 *  end. The user can dismiss with Esc or the close button — closing
 *  during a run doesn't cancel; the reindex keeps going in the
 *  background and the next open shows the latest state. */
function ReindexModal({
  open,
  project,
  onClose,
}: {
  open: boolean;
  project: string;
  onClose: () => void;
}) {
  const cfgEmbedKind = useApp(
    (s) => s.cfg?.embed_provider?.kind ?? null,
  );
  const [running, setRunning] = useState(false);
  const [progress, setProgress] = useState<
    import("@/lib/ipc").ReindexProgress | null
  >(null);
  const [done, setDone] = useState<{
    total: number;
    embedded: number;
    skipped: number;
    removed: number;
  } | null>(null);
  const [err, setErr] = useState<string | null>(null);
  // Track that we've dispatched a reindex for this open session so we
  // don't fire it again on every re-render.
  const startedRef = useRef(false);

  // Subscribe to progress events whenever the modal is mounted.
  useEffect(() => {
    if (!open) return;
    let unsubProgress: (() => void) | null = null;
    let unsubDone: (() => void) | null = null;
    (async () => {
      const { onReindexProgress, onReindexDone } = await import("@/lib/ipc");
      unsubProgress = await onReindexProgress((p) => setProgress(p));
      unsubDone = await onReindexDone(() => setProgress(null));
    })();
    return () => {
      unsubProgress?.();
      unsubDone?.();
    };
  }, [open]);

  // Auto-start the reindex on open.
  useEffect(() => {
    if (!open) {
      startedRef.current = false;
      setRunning(false);
      setProgress(null);
      setDone(null);
      setErr(null);
      return;
    }
    if (startedRef.current) return;
    startedRef.current = true;
    (async () => {
      setRunning(true);
      try {
        const r = await ipc.vectorReindexProject(project);
        setDone({
          total: r.total,
          embedded: r.embedded,
          skipped: r.skipped,
          removed: r.removed,
        });
      } catch (e) {
        setErr(String(e));
      } finally {
        setRunning(false);
        setProgress(null);
      }
    })();
  }, [open, project]);

  // Esc to close.
  useEffect(() => {
    if (!open) return;
    function onKey(e: KeyboardEvent) {
      if (e.key === "Escape") onClose();
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [open, onClose]);

  if (!open) return null;
  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-canvas/70 backdrop-blur-sm"
      onMouseDown={(e) => {
        if (e.target === e.currentTarget) onClose();
      }}
    >
      <div className="flex max-h-[80vh] w-[560px] flex-col overflow-hidden rounded-lg border border-hairline bg-surface">
        <header
          data-tauri-drag-region
          className="flex h-10 shrink-0 items-center justify-between border-b border-hairline px-5"
        >
          <div className="flex items-center gap-2">
            <RefreshCw size={13} className={running ? "animate-spin text-accent-blue" : "text-accent-blue"} />
            <span className="text-[13px] font-medium text-ink">
              재인덱싱 · {project}
            </span>
          </div>
          <button
            onClick={onClose}
            className="grid h-6 w-6 place-items-center rounded-sm text-mute transition-colors hover:bg-surface-elevated hover:text-on-dark"
            title="닫기 (Esc)"
          >
            <X size={12} />
          </button>
        </header>

        <div className="flex-1 min-h-0 overflow-auto p-5">
          {/* 진행 중 — Settings 의 ReindexProgressCard 그대로 재사용 */}
          {(running || progress) && (
            <ReindexProgressCard
              progress={progress}
              providerKind={cfgEmbedKind}
            />
          )}

          {/* 완료 결과 */}
          {done && (
            <div className="rounded-lg border border-accent-green/40 bg-accent-green-soft p-4 text-[13px] leading-[1.6] text-ink">
              <div className="flex items-center gap-2 text-accent-green">
                <Check size={14} />
                <span className="font-semibold">재인덱싱 완료</span>
              </div>
              <div className="mt-2 grid grid-cols-2 gap-2 text-[12px] text-body">
                <Stat2 label="전체" value={done.total} />
                <Stat2 label="새로 embedding" value={done.embedded} />
                <Stat2 label="캐시 적중" value={done.skipped} />
                <Stat2 label="삭제" value={done.removed} />
              </div>
              {done.embedded === 0 && done.total > 0 && (
                <div className="mt-2 text-[11px] text-stone">
                  변경된 파일 없음 — 모두 캐시 적중. dimension/모델이
                  바뀌지 않았다면 정상이에요.
                </div>
              )}
            </div>
          )}

          {/* 오류 — 자주 나오는 Gemini 429 일 때만 친절 안내 */}
          {err && (
            <div className="rounded-lg border border-accent-red/40 bg-accent-red-soft p-4">
              <div className="mb-1 inline-flex items-center gap-1.5 text-[11px] font-semibold uppercase tracking-[0.5px] text-accent-red">
                <span className="h-1.5 w-1.5 rounded-full bg-accent-red" />
                재인덱싱 실패
              </div>
              {err.includes("gemini") && err.includes("429") ? (
                <div className="space-y-2 text-[12px] leading-[1.7] text-on-dark-mute">
                  <div className="font-medium text-ink">
                    Gemini 무료 티어 quota 한도에 걸렸어요
                  </div>
                  <div>
                    <code className="text-on-dark">gemini-embedding-001</code>{" "}
                    무료 티어는 분당 5회 / 일 100회 호출 입니다. 단비가 자동으로
                    12초씩 대기하며 재시도하지만, 일일 한도가 다 차면 내일까지
                    기다려야 해요.
                  </div>
                  <details className="mt-2">
                    <summary className="cursor-pointer text-[11px] text-mute hover:text-on-dark-mute">
                      원본 에러 보기
                    </summary>
                    <pre className="mt-1 whitespace-pre-wrap font-mono text-[11px] leading-[1.6] text-mute">
                      {err}
                    </pre>
                  </details>
                </div>
              ) : (
                <div className="whitespace-pre-wrap font-mono text-[12px] leading-[1.6] text-accent-red">
                  {err}
                </div>
              )}
            </div>
          )}

          {/* 시작 직전 placeholder (running 도 progress 도 없을 때) */}
          {!running && !done && !err && (
            <div className="text-[12px] text-stone">
              곧 시작합니다…
            </div>
          )}
        </div>

        <footer className="flex shrink-0 items-center justify-between border-t border-hairline px-5 py-3">
          <span className="text-[11px] text-stone">
            {running
              ? "백그라운드로 계속 돌릴 수 있어요. 닫아도 멈추지 않아요."
              : "ⓘ Gemini 무료 한도 안에서 자동 페이싱"}
          </span>
          <button
            onClick={onClose}
            className="inline-flex h-7 items-center rounded-md border border-hairline bg-surface-elevated px-3 text-[12px] text-body hover:border-hairline-strong hover:text-on-dark"
          >
            {done || err ? "닫기" : "백그라운드로"}
          </button>
        </footer>
      </div>
    </div>
  );
}

/** 작은 통계 row — ReindexModal 결과 요약용. */
function Stat2({ label, value }: { label: string; value: number }) {
  return (
    <div className="flex items-baseline justify-between rounded-sm border border-hairline bg-surface px-2 py-1">
      <span className="text-[11px] text-stone">{label}</span>
      <span className="font-mono text-[13px] tabular-nums text-ink">
        {value}
      </span>
    </div>
  );
}

// (옛 inline progress 코드는 ReindexModal 로 통합됨.)

/** Per-project goals — short statements of "what I'm trying to do here
 *  right now". Surfaced both in this card and in MCP tool responses (as
 *  `_active_goals`) so external Claude sessions stay oriented even when
 *  the user doesn't explicitly remind them. Drift detection is not
 *  attempted — the user decides what counts as on-track. */
