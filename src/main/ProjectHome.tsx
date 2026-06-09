import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  Archive,
  CalendarDays,
  Check,
  Clock4,
  Copy,
  Edit3,
  FileText,
  Ghost,
  History,
  Link2,
  Loader2,
  MessageCircle,
  Network,
  Plus,
  RefreshCw,
  Send,
  Server,
  Sparkles,
  Target,
  X,
} from "lucide-react";
import { writeText as clipboardWriteText } from "@tauri-apps/plugin-clipboard-manager";
import {
  ipc,
  type BriefingResult,
  type DailyNoteRef,
  type DailySnapshot,
  type DayCounts,
  type GhostLink,
  type GhostStore,
  type Goal,
  type McpProjectEndpoint,
  type ProjectContextStatus,
  type ProjectJournalView,
  type ProjectNode,
  type QaAnswer,
  type TriggerKind,
  type VaultSuggestion,
} from "@/lib/ipc";
import { projectIconOf } from "@/components/ProjectIconPicker";
import { ReindexProgressCard } from "@/main/Settings";
import { cn } from "@/lib/utils";
import { useApp } from "@/state/store";
import { McpInboundProjectMini } from "./McpInboundProjectMini";

export function ProjectHome({
  project,
  onAddDomain,
  onOpenGraph,
  onCopyMcpInstall,
}: {
  project: string;
  onAddDomain: (project: string) => void;
  onOpenGraph: (project: string) => void;
  onCopyMcpInstall: (project: string) => void;
}) {
  const tree = useApp((s) => s.tree);
  const linkIndex = useApp((s) => s.linkIndex);
  const selectDomain = useApp((s) => s.selectDomain);
  // Auto-Journal 화면 외 기존 패널들은 화면에서 빠졌지만 state/핸들러 코드는
  // 향후 복원을 위해 보존. 현재 탭에서만 안 보이게 하는 게 목적이라 코드는
  // 그대로 두되 lint 가 unused 라고 막지 않도록 prefix `_` 만 붙임.

  const [_daily, setDaily] = useState<DailySnapshot | null>(null);
  const [_suggestions, setSuggestions] = useState<VaultSuggestion[]>([]);
  const [_endpoint, setEndpoint] = useState<McpProjectEndpoint | null>(null);
  const [_ghost, setGhost] = useState<GhostStore | null>(null);
  const [_ghostScanning, setGhostScanning] = useState(false);
  const [_ghostError, setGhostError] = useState<string | null>(null);

  const [_qaInput, _setQaInput] = useState("");
  const [_qaAnswer, setQaAnswer] = useState<QaAnswer | null>(null);
  const [_qaLoading, setQaLoading] = useState(false);
  const [_qaError, setQaError] = useState<string | null>(null);

  const [_briefing, setBriefing] = useState<BriefingResult | null>(null);
  const [_briefingRange, setBriefingRange] = useState<
    "today" | "yesterday" | "last_week"
  >("today");
  const [_briefingLoading, setBriefingLoading] = useState(false);
  const [_briefingError, setBriefingError] = useState<string | null>(null);

  const [_projectCtx, setProjectCtx] = useState<ProjectContextStatus | null>(
    null,
  );

  const projectNode: ProjectNode | undefined = useMemo(
    () => tree?.projects.find((p) => p.name === project),
    [tree, project],
  );

  const _refreshProjectCtx = async () => {
    try {
      const ctx = await ipc.projectContextStatus(project);
      setProjectCtx(ctx);
    } catch {
      setProjectCtx(null);
    }
  };

  // Side-effect data load disabled now that the panels using it are
  // hidden. Kept commented out for easy future restore.
  /* useEffect(() => {
    ipc.dailySnapshot().then(setDaily).catch(() => setDaily(null));
    ipc.vaultSuggestions().then(setSuggestions).catch(() => setSuggestions([]));
    ipc.mcpProjectEndpoint(project).then(setEndpoint).catch(() => setEndpoint(null));
    ipc.ghostList(project).then(setGhost).catch(() => setGhost(null));
    _refreshProjectCtx();
  }, [project, tree]); */

  // Keep references so TS doesn't strip them from the closure when
  // someone re-enables the side effect above. Cheap no-ops.
  void setDaily;
  void setSuggestions;
  void setEndpoint;
  void setGhost;
  void setGhostScanning;
  void setGhostError;
  void setQaAnswer;
  void setQaLoading;
  void setQaError;
  void setBriefing;
  void setBriefingRange;
  void setBriefingLoading;
  void setBriefingError;
  void _refreshProjectCtx;

  async function _askQa() {
    const q = _qaInput.trim();
    if (!q) return;
    setQaLoading(true);
    setQaError(null);
    try {
      const res = await ipc.projectQaAsk(project, q);
      setQaAnswer(res);
    } catch (e) {
      setQaError(String(e));
    } finally {
      setQaLoading(false);
    }
  }
  void _askQa;

  async function _scanGhost() {
    setGhostScanning(true);
    setGhostError(null);
    try {
      const store = await ipc.ghostScan(project);
      setGhost(store);
    } catch (e) {
      setGhostError(String(e));
    } finally {
      setGhostScanning(false);
    }
  }
  void _scanGhost;

  async function _acceptGhost(id: string) {
    try {
      const store = await ipc.ghostAccept(project, id);
      setGhost(store);
    } catch (e) {
      setGhostError(String(e));
    }
  }
  void _acceptGhost;

  async function _rejectGhost(id: string) {
    try {
      const store = await ipc.ghostReject(project, id);
      setGhost(store);
    } catch (e) {
      setGhostError(String(e));
    }
  }
  void _rejectGhost;

  async function _runBriefing(
    range: "today" | "yesterday" | "last_week",
  ) {
    setBriefingRange(range);
    setBriefingLoading(true);
    setBriefingError(null);
    try {
      const res = await ipc.projectBriefing(project, range);
      setBriefing(res);
    } catch (e) {
      setBriefingError(String(e));
    } finally {
      setBriefingLoading(false);
    }
  }
  void _runBriefing;

  // The old dashboard derived a bunch of memos from suggestions/daily/
  // linkIndex — kept in a parked scope for future restore. Wrapped in
  // a self-IIFE that the linter sees as "used" but does no work.
  void useMemo(() => {
    const _ = { tree, linkIndex, project };
    return _;
  }, [tree, linkIndex, project]);
  void projectNode;

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
        className="flex h-10 shrink-0 items-center justify-between border-b border-hairline px-5"
      >
        <div className="flex items-center gap-2">
          <span className="text-[13px] font-medium tracking-[0.2px] text-ink">
            {project}
          </span>
          <span className="rounded-xs bg-surface-elevated px-1.5 py-0.5 text-[10px] text-on-dark-mute">
            Auto-Journal
          </span>
        </div>
        {/* 헤더 우측 액션은 ProjectActions 카드로 이동 (히어로 아래).
            여기엔 컴팩트 버튼이 좁아서 잘 안 보였음 — 사용자 피드백
            반영. 헤더는 라벨만 남김. */}
      </header>

      <div className="flex-1 min-h-0 overflow-auto">
        <div className="mx-auto max-w-[820px] px-6 py-8">
          <ProjectHero
            project={project}
            onCopyInstall={() => onCopyMcpInstall(project)}
            onEditSkill={async () => {
              // 사이드바에 SKILL.md 가 안 보이는 vault (예전엔
              // ~/.claude 만 만들고 vault seed 가 없던 케이스) 도
              // 한 번 install 을 거치면 vault SKILL.md 가 자동
              // 생성된다. 실패해도 selectDomain 은 시도 — 빈
              // 에디터라도 사용자가 수동 작성할 수 있게.
              try {
                await ipc.installSkill(project);
              } catch (e) {
                console.error("[danbi] ensure SKILL.md failed", e);
              }
              selectDomain(project, "SKILL.md");
            }}
          />
          <div className="mt-6">
            <ProjectActions
              project={project}
              onAddDomain={() => onAddDomain(project)}
              onOpenGraph={() => onOpenGraph(project)}
            />
          </div>
          <div className="mt-6">
            <GoalsCard project={project} />
          </div>
          <div className="mt-6">
            <SevenDayBar project={project} />
          </div>
          <div className="mt-6">
            <McpInboundProjectMini
              project={project}
              onOpenDomain={(d) => selectDomain(project, d)}
            />
          </div>
          <div className="mt-6">
            <AutoJournalPanel
              project={project}
              onOpen={(d) => selectDomain(project, d)}
            />
          </div>
        </div>
      </div>
    </div>
  );
}

/** Big page title — group label (small uppercase) + project icon +
 *  project name (large). Single source of truth for "어느 프로젝트를
 *  보고 있는지" so the user always knows the scope at a glance. */
function ProjectHero({
  project,
  onCopyInstall,
  onEditSkill,
}: {
  project: string;
  onCopyInstall: () => void;
  onEditSkill: () => void;
}) {
  const groups = useApp((s) => s.cfg?.project_groups ?? []);
  const iconName = useApp(
    (s) => s.cfg?.project_icons?.[project] ?? null,
  );
  const Icon = projectIconOf(iconName);
  const groupLabel = useMemo(() => {
    const g = groups.find((g) => g.projects.includes(project));
    return g?.label ?? null;
  }, [groups, project]);

  // Skill installation state. Polled once on mount; re-checked after
  // the user clicks "설치" so the label flips to "업데이트".
  const [skillInstalled, setSkillInstalled] = useState(false);
  const [skillFlash, setSkillFlash] = useState<string | null>(null);
  useEffect(() => {
    ipc.skillStatus(project).then(setSkillInstalled).catch(() => {});
  }, [project]);
  useEffect(() => {
    if (!skillFlash) return;
    const t = setTimeout(() => setSkillFlash(null), 3500);
    return () => clearTimeout(t);
  }, [skillFlash]);
  const installSkill = async () => {
    try {
      const path = await ipc.installSkill(project);
      setSkillInstalled(true);
      setSkillFlash(skillInstalled ? "업데이트됨" : "설치됨");
      console.log("[danbi] skill installed at", path);
    } catch (e) {
      console.error("[danbi] install skill failed", e);
      setSkillFlash(`실패: ${e}`);
    }
  };

  return (
    <div className="flex flex-col gap-2">
      {groupLabel && (
        <div className="text-[12px] font-semibold uppercase tracking-[0.6px] text-mute">
          {groupLabel}
        </div>
      )}
      <div className="flex items-center justify-between gap-4">
        <div className="flex min-w-0 items-center gap-3">
          <span className="grid h-9 w-9 shrink-0 place-items-center rounded-md bg-surface-elevated text-on-dark-mute">
            <Icon size={20} />
          </span>
          <h1 className="truncate text-[28px] font-semibold leading-[1.15] tracking-[-0.01em] text-ink">
            {project}
          </h1>
        </div>
        {/* Compact Claude Code connect actions — sits next to the
         *  title so the user can copy install/MCP cmds without
         *  scrolling. Tooltips carry the full intent. */}
        <div className="flex shrink-0 items-center gap-1.5">
          {/* 필수 두 개만 헤더에 노출:
              - Claude Code 연결 (MCP 등록 명령 복사)
              - Skill 설치/갱신 (Claude Code 의 자동 활성화 가이드)
              CLAUDE.md 단비 블록은 프로젝트별 강제 규칙이 필요한 사용자만
              가끔 쓰는 거라 사이드바 우클릭 → "CLAUDE.md 단비 블록 복사"
              로 옮겨서 헤더 혼선을 줄임. */}
          <button
            type="button"
            onClick={onCopyInstall}
            title="Claude Code 설치 명령 복사 (필수 1단계)"
            className="inline-flex h-8 items-center gap-1.5 rounded-md border border-hairline bg-surface-elevated px-3 text-[12px] text-body hover:border-hairline-strong hover:text-on-dark"
          >
            <Server size={12} /> Claude Code 연결
          </button>
          <button
            type="button"
            onClick={installSkill}
            title={
              skillInstalled
                ? `~/.claude/skills/danbi-${project}/SKILL.md 갱신 — 사이드바의 SKILL.md 를 편집한 뒤 누르면 ~/.claude 로 동기화됩니다`
                : `~/.claude/skills/danbi-${project}/SKILL.md 설치 (필수 2단계) — 처음 설치 시 사이드바에 편집 가능한 SKILL.md 가 생성됩니다`
            }
            className={cn(
              "inline-flex h-8 items-center gap-1.5 rounded-md border px-3 text-[12px] transition-colors",
              skillInstalled
                ? "border-accent-green/40 bg-accent-green-soft text-accent-green hover:border-accent-green"
                : "border-hairline bg-surface-elevated text-body hover:border-hairline-strong hover:text-on-dark",
            )}
          >
            <Sparkles size={12} />
            {skillInstalled ? "Skill 갱신" : "Skill 설치"}
          </button>
          {skillInstalled && (
            <button
              type="button"
              onClick={onEditSkill}
              title={`${project}/SKILL.md 편집 — 저장 후 'Skill 갱신' 누르면 ~/.claude 로 동기화됩니다`}
              className="inline-flex h-8 items-center gap-1.5 rounded-md border border-hairline bg-surface-elevated px-3 text-[12px] text-body hover:border-hairline-strong hover:text-on-dark"
            >
              <Edit3 size={12} /> Skill 수정
            </button>
          )}
          {/* CLAUDE.md 버튼 보존 (필요시 다시 노출):
          <button
            type="button"
            onClick={onCopyClaudeMd}
            title="CLAUDE.md 단비 블록 복사 (프로젝트별 강제 규칙)"
            className="inline-flex h-8 items-center gap-1.5 rounded-md border border-hairline bg-surface-elevated px-3 text-[12px] text-body hover:border-hairline-strong hover:text-on-dark"
          >
            <Copy size={12} /> CLAUDE.md
          </button>
          */}
        </div>
      </div>
      {skillFlash && (
        <div className="self-end text-[11px] text-accent-green">
          ✓ Skill {skillFlash}
        </div>
      )}
    </div>
  );
}

/** "Claude Code 가 받아 적은 것" — Auto-Journal 메인 패널.
 *  vault 의 daily/*.md 를 trigger 별로 파싱해서 오늘 카운트 +
 *  최근 항목 + 7일 막대로 보여준다. LLM 호출 없이 동작. */
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

function _McpFooter({
  project,
  onCopyInstall,
  onCopyClaudeMd,
  embedded = false,
}: {
  project: string;
  onCopyInstall: () => void;
  onCopyClaudeMd: () => void;
  embedded?: boolean;
}) {
  const inner = (
    <div className="flex h-full items-center justify-between gap-3">
      <span className="min-w-0 flex-1 text-[12px] font-medium text-ink">
        Claude Code 에 연결
      </span>
      <div className="flex shrink-0 items-center gap-1.5">
        <button
          onClick={onCopyInstall}
          title={`${project} MCP 설치 명령 복사`}
          className="inline-flex h-7 items-center gap-1 rounded-sm border border-hairline bg-surface px-2 text-[11px] text-body hover:border-hairline-strong hover:text-on-dark"
        >
          설치 명령
        </button>
        <button
          onClick={onCopyClaudeMd}
          title="CLAUDE.md 단비 블록 복사"
          className="inline-flex h-7 items-center gap-1 rounded-sm border border-hairline bg-surface px-2 text-[11px] text-body hover:border-hairline-strong hover:text-on-dark"
        >
          CLAUDE.md
        </button>
      </div>
    </div>
  );
  if (embedded) return <div>{inner}</div>;
  return (
    <div className="rounded-md border border-hairline bg-surface-elevated px-4 py-3">
      {inner}
    </div>
  );
}

function _DailyCard({
  today,
  todayNotes,
  week,
  month,
  year,
  project,
  onOpen,
  hasOtherProjects,
}: {
  today: string;
  todayNotes: DailyNoteRef[];
  week: DailyNoteRef[];
  month: DailyNoteRef[];
  year: DailyNoteRef[];
  project: string;
  onOpen: (domain: string) => void;
  hasOtherProjects: boolean;
}) {
  const [creating, setCreating] = useState(false);

  async function createToday() {
    setCreating(true);
    try {
      const domain = await ipc.ensureTodayNote(project);
      onOpen(domain);
    } finally {
      setCreating(false);
    }
  }

  const hasPast = week.length + month.length + year.length > 0;
  void hasOtherProjects; // kept for future: could suggest linking to other projects' daily notes.

  return (
    <section className="mt-6 rounded-lg border border-hairline bg-surface p-4">
      <header className="flex items-center justify-between">
        <div className="flex items-center gap-2">
          <CalendarDays size={13} className="text-accent-blue" />
          <h2 className="text-[14px] font-medium text-ink">오늘</h2>
          <span className="font-mono text-caption-sm text-stone">{today}</span>
        </div>
        {todayNotes.length === 0 && (
          <button
            onClick={createToday}
            disabled={creating}
            className="inline-flex h-6 items-center gap-1 rounded-sm border border-hairline bg-surface-elevated px-2 text-[11px] text-body hover:border-hairline-strong hover:text-on-dark"
          >
            <Plus size={10} /> 오늘 노트
          </button>
        )}
      </header>

      {todayNotes.length > 0 ? (
        <ul className="mt-2 flex flex-col gap-1">
          {todayNotes.map((n) => (
            <li key={n.domain}>
              <button
                onClick={() => onOpen(n.domain)}
                className="flex w-full items-center gap-2 rounded-sm px-2 py-1 text-left transition-colors hover:bg-surface-elevated"
              >
                <FileText size={11} className="text-mute" />
                <span className="font-mono text-caption-md text-body">
                  {n.domain}
                </span>
                <span className="ml-auto text-caption-sm text-stone">
                  {n.bytes > 0 ? `${(n.bytes / 1024).toFixed(1)}KB` : "empty"}
                </span>
              </button>
            </li>
          ))}
        </ul>
      ) : (
        <div className="mt-2 text-caption-sm text-stone">
          오늘 데일리 노트가 없어요.
        </div>
      )}

      {hasPast && (
        <div className="mt-4 border-t border-hairline pt-3">
          <div className="mb-1.5 flex items-center gap-1.5">
            <Clock4 size={11} className="text-accent-yellow" />
            <span className="text-caption-sm uppercase tracking-[0.4px] text-mute">
              지난 오늘
            </span>
          </div>
          <div className="flex flex-col gap-1">
            <PastRow label="1주 전" notes={week} onOpen={onOpen} />
            <PastRow label="1개월 전" notes={month} onOpen={onOpen} />
            <PastRow label="1년 전" notes={year} onOpen={onOpen} />
          </div>
        </div>
      )}
    </section>
  );
}

function PastRow({
  label,
  notes,
  onOpen,
}: {
  label: string;
  notes: DailyNoteRef[];
  onOpen: (domain: string) => void;
}) {
  if (notes.length === 0) return null;
  return (
    <div className="flex flex-wrap items-center gap-1.5 text-caption-sm">
      <span className="w-14 shrink-0 text-stone">{label}</span>
      {notes.map((n) => (
        <button
          key={n.domain}
          onClick={() => onOpen(n.domain)}
          className="inline-flex items-center gap-1 rounded-full border border-hairline bg-surface-elevated px-2 py-0.5 text-[11px] text-body hover:border-hairline-strong hover:text-on-dark"
        >
          <span className="font-mono text-[10px]">{n.date}</span>
        </button>
      ))}
    </div>
  );
}

function _McpCard({
  project,
  endpoint,
  onCopyInstall,
  onCopyClaudeMd,
}: {
  project: string;
  endpoint: McpProjectEndpoint | null;
  onCopyInstall: () => void;
  onCopyClaudeMd: () => void;
}) {
  void project;
  const [copiedUrl, setCopiedUrl] = useState(false);

  async function copyUrl() {
    if (!endpoint) return;
    try {
      await clipboardWriteText(endpoint.url);
      setCopiedUrl(true);
      setTimeout(() => setCopiedUrl(false), 1500);
    } catch {
      /* handled in parent via toast */
    }
  }

  return (
    <section className="rounded-lg border border-hairline bg-surface p-4">
      <div className="mb-2 flex items-center gap-1.5">
        <Server size={13} className="text-accent-blue" />
        <h2 className="text-[14px] font-medium text-ink">MCP</h2>
      </div>
      {endpoint ? (
        <>
          <div className="mb-2 flex items-center gap-1.5 rounded-sm border border-hairline bg-surface-elevated px-2 py-1.5">
            <Link2 size={11} className="shrink-0 text-stone" />
            <span className="flex-1 truncate font-mono text-[11px] text-body">
              {endpoint.url}
            </span>
            <button
              onClick={copyUrl}
              className="grid h-5 w-5 place-items-center rounded-xs text-stone transition-colors hover:bg-surface-card hover:text-on-dark"
              title="URL 복사"
            >
              {copiedUrl ? <Check size={10} /> : <Copy size={10} />}
            </button>
          </div>
          <div className="flex flex-col gap-1.5">
            <button
              onClick={onCopyInstall}
              className="inline-flex h-7 items-center justify-center rounded-sm border border-hairline bg-surface-elevated text-[11px] text-body hover:border-hairline-strong hover:text-on-dark"
            >
              Claude Code 설치 명령 복사
            </button>
            <button
              onClick={onCopyClaudeMd}
              className="inline-flex h-7 items-center justify-center rounded-sm border border-hairline bg-surface-elevated text-[11px] text-body hover:border-hairline-strong hover:text-on-dark"
            >
              CLAUDE.md 단비 블록 복사
            </button>
          </div>
        </>
      ) : (
        <div className="text-caption-sm text-stone">
          MCP 서버가 꺼져 있어요. ⌘, 설정 → MCP 에서 켜주세요.
        </div>
      )}
    </section>
  );
}

function _SuggestionRow({
  s,
  onOpen,
}: {
  s: VaultSuggestion;
  onOpen: (domain: string) => void;
}) {
  if (s.kind === "EmptyProject") return null;
  const label =
    s.kind === "Orphan" ? "ORPHAN" : s.kind === "Empty" ? "EMPTY" : "LARGE";
  const color =
    s.kind === "Oversized"
      ? "text-accent-yellow bg-accent-yellow-soft"
      : s.kind === "Orphan"
        ? "text-accent-blue bg-accent-blue-soft"
        : "text-mute bg-surface-elevated";
  const hint =
    s.kind === "Orphan"
      ? "어디에서도 참조되지 않아요."
      : s.kind === "Empty"
        ? "내용이 없어요."
        : `${(s.bytes / 1024).toFixed(1)}KB — 섹션으로 나누세요.`;
  return (
    <li className="flex items-center gap-2 text-caption-md">
      <span
        className={cn(
          "shrink-0 rounded-xs px-1.5 py-0.5 text-[10px] uppercase tracking-[0.4px]",
          color,
        )}
      >
        {label}
      </span>
      <button
        onClick={() => onOpen(s.domain)}
        className="font-mono text-body transition-colors hover:text-on-dark"
      >
        {s.domain}
      </button>
      <span className="text-stone">— {hint}</span>
    </li>
  );
}

function _QaSection({
  value,
  onChange,
  onAsk,
  loading,
  error,
  answer,
  onOpen,
  llmReady,
}: {
  value: string;
  onChange: (v: string) => void;
  onAsk: () => void;
  loading: boolean;
  error: string | null;
  answer: QaAnswer | null;
  onOpen: (domain: string) => void;
  llmReady: boolean;
}) {
  return (
    <section className="mt-6 rounded-lg border border-hairline bg-surface p-4">
      <header className="mb-2 flex items-center gap-1.5">
        <MessageCircle size={13} className="text-accent-blue" />
        <h2 className="text-[14px] font-medium text-ink">이 프로젝트에 물어보기</h2>
        {!llmReady && <LlmLockBadge />}
      </header>
      <div className="flex items-center gap-2">
        <input
          value={value}
          onChange={(e) => onChange(e.target.value)}
          onKeyDown={(e) => {
            if (
              e.key === "Enter" &&
              !e.nativeEvent.isComposing &&
              !loading &&
              llmReady
            ) {
              e.preventDefault();
              onAsk();
            }
          }}
          disabled={!llmReady}
          placeholder={
            llmReady
              ? "예: auth 어떻게 정했지? — 이 vault만 근거로 답해요"
              : "LLM 미연결 — Settings 에서 Provider 연결 후 사용"
          }
          className="flex-1 rounded-md border border-hairline bg-surface-elevated px-3 py-1.5 text-[13px] text-on-dark outline-none focus:border-hairline-strong disabled:cursor-not-allowed disabled:opacity-60"
        />
        <button
          onClick={onAsk}
          disabled={!llmReady || loading || value.trim().length === 0}
          title={!llmReady ? "LLM 연결 필요" : undefined}
          className="inline-flex h-8 items-center gap-1.5 rounded-md bg-primary px-3 text-[12px] font-medium text-on-primary transition-colors hover:bg-primary-pressed disabled:opacity-60 disabled:cursor-not-allowed"
        >
          {loading ? (
            <Loader2 size={12} className="animate-spin" />
          ) : (
            <Send size={11} />
          )}
          {loading ? "찾는 중…" : "물어보기"}
        </button>
      </div>

      {error && (
        <div className="mt-2 rounded-sm border border-hairline bg-surface-elevated p-2 font-mono text-[11px] text-accent-red">
          {error}
        </div>
      )}

      {answer && (
        <div className="mt-3 rounded-md border border-hairline bg-surface-elevated p-3">
          <div className="whitespace-pre-wrap text-[13px] leading-[1.6] text-on-dark">
            {answer.answer}
          </div>
          {answer.citations.length > 0 && (
            <div className="mt-3 flex flex-wrap gap-1.5">
              {answer.citations.map((c, i) => (
                <button
                  key={`${c.domain}-${i}`}
                  onClick={() => onOpen(c.domain)}
                  title={c.note}
                  className="inline-flex items-center gap-1 rounded-full border border-hairline bg-surface px-2 py-0.5 text-[11px] text-body hover:border-hairline-strong hover:text-on-dark"
                >
                  <FileText size={10} className="text-stone" />
                  <span className="font-mono text-[10px]">{c.domain}</span>
                </button>
              ))}
            </div>
          )}
          {answer.citations.length === 0 && answer.sources.length > 0 && (
            <div className="mt-3 text-caption-sm text-stone">
              참고한 문서 없음 — 검색 결과: {answer.sources.slice(0, 5).join(", ")}
            </div>
          )}
        </div>
      )}
    </section>
  );
}

function _BriefingSection({
  range,
  loading,
  error,
  result,
  onRun,
  onOpen,
  llmReady,
}: {
  range: "today" | "yesterday" | "last_week";
  loading: boolean;
  error: string | null;
  result: BriefingResult | null;
  onRun: (r: "today" | "yesterday" | "last_week") => void;
  onOpen: (domain: string) => void;
  llmReady: boolean;
}) {
  const rangeLabel: Record<typeof range, string> = {
    today: "오늘",
    yesterday: "어제",
    last_week: "지난 7일",
  };

  return (
    <section className="mt-6 rounded-lg border border-hairline bg-surface p-4">
      <header className="mb-2 flex items-center justify-between">
        <div className="flex items-center gap-1.5">
          <History size={13} className="text-accent-blue" />
          <h2 className="text-[14px] font-medium text-ink">변경 브리핑</h2>
          {!llmReady && <LlmLockBadge />}
        </div>
        <div className="flex items-center gap-1">
          {(["today", "yesterday", "last_week"] as const).map((r) => (
            <button
              key={r}
              onClick={() => onRun(r)}
              disabled={loading || !llmReady}
              title={!llmReady ? "LLM 연결 필요" : undefined}
              className={cn(
                "inline-flex h-6 items-center rounded-sm px-2 text-[11px] transition-colors disabled:cursor-not-allowed disabled:opacity-50",
                range === r && result
                  ? "bg-primary text-on-primary hover:bg-primary-pressed"
                  : "border border-hairline bg-surface-elevated text-body hover:border-hairline-strong hover:text-on-dark",
              )}
            >
              {rangeLabel[r]}
            </button>
          ))}
        </div>
      </header>

      {error && (
        <div className="mt-1 rounded-sm border border-hairline bg-surface-elevated p-2 font-mono text-[11px] text-accent-red">
          {error}
        </div>
      )}

      {loading ? (
        <div className="mt-1 flex items-center gap-1.5 text-caption-sm text-stone">
          <Loader2 size={11} className="animate-spin" />
          git 로그 훑는 중…
        </div>
      ) : !result ? (
        <div className="text-caption-sm text-stone">
          돌아왔을 때 이 프로젝트에서 뭐가 바뀌었는지 Writer가 정리해요. 버튼을 눌러보세요.
        </div>
      ) : (
        <>
          <div className="whitespace-pre-wrap text-[13px] leading-[1.6] text-on-dark">
            {result.summary}
          </div>
          {result.changed_files.length > 0 && (
            <div className="mt-3 flex flex-wrap gap-1.5">
              {result.changed_files.map((f) => (
                <button
                  key={f}
                  onClick={() => onOpen(f)}
                  className="inline-flex items-center gap-1 rounded-full border border-hairline bg-surface-elevated px-2 py-0.5 text-[11px] text-body hover:border-hairline-strong hover:text-on-dark"
                >
                  <FileText size={10} className="text-stone" />
                  <span className="font-mono text-[10px]">{f}</span>
                </button>
              ))}
            </div>
          )}
        </>
      )}
    </section>
  );
}

function _GhostSection({
  store,
  scanning,
  error,
  onScan,
  onAccept,
  onReject,
  onOpen,
  llmReady,
}: {
  store: GhostStore | null;
  scanning: boolean;
  error: string | null;
  onScan: () => void;
  onAccept: (id: string) => void;
  onReject: (id: string) => void;
  onOpen: (domain: string) => void;
  llmReady: boolean;
}) {
  const pending = (store?.links ?? []).filter((l) => l.status === "pending");
  const accepted = (store?.links ?? []).filter(
    (l) => l.status === "accepted",
  ).length;
  const lastScan = store?.last_scan_at ?? null;
  const hasScanned = lastScan !== null;

  return (
    <section className="mt-6 rounded-lg border border-hairline bg-surface p-4">
      <header className="mb-2 flex items-center justify-between">
        <div className="flex items-center gap-1.5">
          <Ghost size={13} className="text-accent-blue" />
          <h2 className="text-[14px] font-medium text-ink">Ghost Links</h2>
          <span className="text-caption-sm text-stone">
            {hasScanned
              ? `${pending.length} 제안 · ${accepted} 확정`
              : "아직 스캔 안 함"}
          </span>
          {!llmReady && <LlmLockBadge />}
        </div>
        <button
          onClick={onScan}
          disabled={scanning || !llmReady}
          title={!llmReady ? "LLM 연결 필요" : undefined}
          className="inline-flex h-7 items-center gap-1.5 rounded-md bg-primary px-3 text-[12px] font-medium text-on-primary transition-colors hover:bg-primary-pressed disabled:opacity-60 disabled:cursor-not-allowed"
        >
          {scanning ? (
            <Loader2 size={11} className="animate-spin" />
          ) : (
            <Sparkles size={11} />
          )}
          {scanning ? "스캔 중…" : hasScanned ? "다시 스캔" : "새 제안 찾기"}
        </button>
      </header>

      {error && (
        <div className="mb-2 rounded-sm border border-hairline bg-surface-elevated p-2 font-mono text-[11px] text-accent-red">
          {error}
        </div>
      )}

      {!hasScanned ? (
        <div className="text-caption-sm text-stone">
          AI가 프로젝트 내 문서들을 훑어 서로 연결될 법한 쌍을 제안합니다.
          Haiku 1회 호출(≈ 수 센트).
        </div>
      ) : pending.length === 0 ? (
        <div className="text-caption-sm text-stone">
          대기 중인 제안이 없어요. 새 문서를 쓰고 다시 스캔해 보세요.
        </div>
      ) : (
        <ul className="flex flex-col gap-1.5">
          {pending.map((l) => (
            <GhostRow
              key={l.id}
              link={l}
              onAccept={() => onAccept(l.id)}
              onReject={() => onReject(l.id)}
              onOpen={onOpen}
            />
          ))}
        </ul>
      )}
    </section>
  );
}

function GhostRow({
  link,
  onAccept,
  onReject,
  onOpen,
}: {
  link: GhostLink;
  onAccept: () => void;
  onReject: () => void;
  onOpen: (domain: string) => void;
}) {
  return (
    <li className="flex items-center gap-2 rounded-sm border border-hairline bg-surface-elevated px-2.5 py-2">
      <div className="flex min-w-0 flex-1 flex-col gap-1">
        <div className="flex items-center gap-1.5 text-caption-md">
          <button
            onClick={() => onOpen(link.source_domain)}
            className="truncate font-mono text-body hover:text-on-dark"
            title={link.source_domain}
          >
            {link.source_domain}
          </button>
          <Link2 size={10} className="shrink-0 text-stone" />
          <button
            onClick={() => onOpen(link.target_domain)}
            className="truncate font-mono text-body hover:text-on-dark"
            title={link.target_domain}
          >
            {link.target_domain}
          </button>
        </div>
        {link.reason && (
          <div className="truncate text-caption-sm text-stone" title={link.reason}>
            {link.reason}
          </div>
        )}
      </div>
      <div className="flex shrink-0 items-center gap-1">
        <button
          onClick={onReject}
          title="기각"
          className="grid h-6 w-6 place-items-center rounded-xs text-stone transition-colors hover:bg-surface-card hover:text-on-dark"
        >
          <X size={11} />
        </button>
        <button
          onClick={onAccept}
          title="확정 — 소스 문서 하단에 [[target]] 추가"
          className="grid h-6 w-6 place-items-center rounded-xs text-accent-blue transition-colors hover:bg-accent-blue-soft"
        >
          <Check size={11} />
        </button>
      </div>
    </li>
  );
}

function _ProjectContextSection({
  project,
  ctx,
  onRefresh,
  onOpen,
}: {
  project: string;
  ctx: ProjectContextStatus | null;
  onRefresh: () => void;
  onOpen: (domain: string) => void;
}) {
  const [creating, setCreating] = useState(false);
  const [err, setErr] = useState<string | null>(null);

  const hasAny = ctx?.has_purpose || ctx?.has_schema;
  const hasBoth = ctx?.has_purpose && ctx?.has_schema;

  async function seed() {
    setCreating(true);
    setErr(null);
    try {
      await ipc.projectContextEnsure(project);
      await onRefresh();
    } catch (e) {
      setErr(String(e));
    } finally {
      setCreating(false);
    }
  }

  return (
    <section className="mt-6 rounded-lg border border-hairline bg-surface p-4">
      <header className="mb-2 flex items-center justify-between">
        <div className="flex items-center gap-2">
          <Sparkles size={13} className="text-accent-blue" />
          <h2 className="text-[14px] font-medium text-ink">
            프로젝트 맥락
          </h2>
          <span
            className={cn(
              "rounded-xs px-1.5 py-0.5 text-[10px] uppercase tracking-[0.4px]",
              hasBoth
                ? "bg-accent-green-soft text-accent-green"
                : hasAny
                  ? "bg-accent-yellow-soft text-accent-yellow"
                  : "bg-surface-elevated text-mute",
            )}
          >
            {hasBoth ? "Wiki-LLM 준비됨" : hasAny ? "일부 설정됨" : "미설정"}
          </span>
        </div>
        {!hasBoth && (
          <button
            disabled={creating}
            onClick={seed}
            className="inline-flex h-7 items-center gap-1 rounded-sm border border-hairline bg-surface-elevated px-2 text-[12px] text-body hover:text-on-dark"
          >
            {creating ? "생성 중…" : "자동 생성"}
          </button>
        )}
      </header>

      <p className="mb-3 text-caption-sm leading-[1.6] text-mute">
        <code className="font-mono text-on-dark-mute">purpose.md</code> 와{" "}
        <code className="font-mono text-on-dark-mute">schema.md</code> 는 Writer
        가 모든 편집에서 참고하는 프로젝트 규칙이에요. 설정할수록 LLM 응답이 이
        프로젝트의 맥락에 더 잘 맞춰집니다.
      </p>

      <div className="grid grid-cols-2 gap-2">
        <ContextFileRow
          kind="purpose.md"
          exists={!!ctx?.has_purpose}
          clipped={!!ctx?.purpose_clipped}
          onOpen={() => onOpen("purpose.md")}
        />
        <ContextFileRow
          kind="schema.md"
          exists={!!ctx?.has_schema}
          clipped={!!ctx?.schema_clipped}
          onOpen={() => onOpen("schema.md")}
        />
      </div>

      {err && (
        <div className="mt-3 rounded-md border border-hairline bg-surface-elevated p-2 font-mono text-[11px] text-accent-red">
          {err}
        </div>
      )}
    </section>
  );
}

function ContextFileRow({
  kind,
  exists,
  clipped,
  onOpen,
}: {
  kind: string;
  exists: boolean;
  clipped: boolean;
  onOpen: () => void;
}) {
  return (
    <button
      onClick={onOpen}
      disabled={!exists}
      className={cn(
        "flex items-center justify-between rounded-md border p-3 text-left transition-colors",
        exists
          ? "border-hairline bg-surface-elevated hover:border-hairline-strong"
          : "border-dashed border-hairline bg-surface opacity-60",
      )}
    >
      <div className="flex flex-col">
        <span className="font-mono text-[12px] text-ink">{kind}</span>
        <span className="mt-0.5 text-caption-sm text-mute">
          {exists
            ? clipped
              ? "크기 초과 — 요약 권장"
              : "Writer 에 주입 중"
            : "없음"}
        </span>
      </div>
      <span
        className={cn(
          "text-[11px] uppercase tracking-[0.4px]",
          exists
            ? clipped
              ? "text-accent-yellow"
              : "text-accent-green"
            : "text-stone",
        )}
      >
        {exists ? (clipped ? "CLIP" : "OK") : "—"}
      </span>
    </button>
  );
}

/** Small amber "LLM 필요" chip. Goes in section headers where the
 *  primary action is gated on a configured provider. */
function LlmLockBadge() {
  return (
    <span
      className="ml-auto inline-flex items-center gap-1 rounded-xs bg-accent-yellow-soft px-1.5 py-0.5 text-[10px] font-semibold uppercase tracking-[0.5px] text-accent-yellow"
      title="LLM 필요 · Settings → Provider 연결"
    >
      LLM 필요
    </span>
  );
}

function _Stat({
  label,
  value,
  hint,
}: {
  label: string;
  value: string;
  hint: string;
}) {
  return (
    <div className="flex flex-col rounded-lg border border-hairline bg-surface px-3 py-3">
      <span className="text-[11px] uppercase tracking-[0.4px] text-stone">
        {label}
      </span>
      <span className="mt-1 text-[20px] font-medium leading-none text-ink">
        {value}
      </span>
      <span className="mt-1 text-caption-sm text-mute">{hint}</span>
    </div>
  );
}

function _prettyBytes(n: number): string {
  if (n < 1024) return `${n}B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)}KB`;
  return `${(n / (1024 * 1024)).toFixed(1)}MB`;
}

function _timeAgo(ms: number): string {
  const diff = Date.now() - ms;
  if (diff < 60 * 1000) return "방금";
  if (diff < 60 * 60 * 1000) return `${Math.floor(diff / (60 * 1000))}분 전`;
  if (diff < 24 * 60 * 60 * 1000)
    return `${Math.floor(diff / (60 * 60 * 1000))}시간 전`;
  if (diff < 7 * 24 * 60 * 60 * 1000)
    return `${Math.floor(diff / (24 * 60 * 60 * 1000))}일 전`;
  const d = new Date(ms);
  return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}`;
}

// Parked components/helpers from the previous dashboard. Re-exported so
// noUnusedLocals doesn't complain — keeps them in the bundle but the
// new ProjectHome doesn't render them. Restore by importing wherever.
export {
  _DailyCard,
  _McpCard,
  _McpFooter,
  _SuggestionRow,
  _QaSection,
  _BriefingSection,
  _GhostSection,
  _ProjectContextSection,
  _Stat,
  _prettyBytes,
  _timeAgo,
};

/** Inline action card on the project home — three big affordances:
 *  1) 검색 (this project only): search box that queries vault then
 *     filters to this project; clicking a hit jumps to the file.
 *  2) 그래프 (this project): opens GraphView scoped to this project.
 *  3) 재인덱싱 (this project): runs the per-project vector reindex
 *     with progress feedback (powered by ProjectReindexButton).
 *  + 도메인 추가 quick action.
 *
 *  Placed between ProjectHero and SevenDayBar so the user sees them
 *  immediately without scrolling. The buttons in the small header used
 *  to live here, but they were too compact to notice — the user
 *  explicitly asked for these three to "잘 보이게".
 */
function ProjectActions({
  project,
  onAddDomain,
  onOpenGraph,
}: {
  project: string;
  onAddDomain: () => void;
  onOpenGraph: () => void;
}) {
  const [query, setQuery] = useState("");
  const [results, setResults] = useState<
    Array<{
      project: string;
      domain: string;
      snippet: string;
      relevance: number;
    }>
  >([]);
  const [searching, setSearching] = useState(false);
  const selectDomain = useApp((s) => s.selectDomain);
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

  // Debounced project-scoped search. We hit the vault-wide tantivy
  // index then filter to this project — same UX as the global ⌘K
  // palette but pinned to the current project.
  useEffect(() => {
    const q = query.trim();
    if (!q) {
      setResults([]);
      return;
    }
    setSearching(true);
    const id = setTimeout(async () => {
      try {
        const hits = await ipc.searchFull(q, 30);
        const filtered = hits
          .filter((h) => h.project === project)
          .slice(0, 8)
          .map((h) => ({
            project: h.project,
            domain: h.domain,
            snippet: h.snippet ?? "",
            relevance: h.relevance ?? 0,
          }));
        setResults(filtered);
      } catch (e) {
        console.error("[danbi] project search failed", e);
        setResults([]);
      } finally {
        setSearching(false);
      }
    }, 200);
    return () => clearTimeout(id);
  }, [query, project]);

  return (
    <section className="rounded-lg border border-hairline bg-surface-elevated p-5">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <div className="text-[14px] font-semibold uppercase tracking-[0.5px] text-mute">
          이 프로젝트 안에서
        </div>
        <div className="flex flex-wrap items-center gap-2">
          {/* 그래프 — 보조 액션. 기본 hairline 톤. */}
          <button
            onClick={onOpenGraph}
            title={`${project} 그래프 보기`}
            className="inline-flex h-10 items-center gap-2 rounded-md border border-hairline bg-surface px-4 text-[14px] font-medium text-body hover:border-hairline-strong hover:text-on-dark"
          >
            <Network size={16} /> 그래프
          </button>
          {/* Ghost links 자동 제안 — AI 연동 켰을 때만 활성. 결과는
              그래프에서 점선으로 자동 표시되고 ReviewPanel 에서 accept/
              reject 가능. */}
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
          {/* 재인덱싱 — accent-blue 강조 (자주 쓸 핵심 액션). */}
          <ProjectReindexButton project={project} compact={false} />
          {/* 도메인 추가 — primary CTA, 흰 필. 한 뷰포트에 하나만 노출. */}
          <button
            onClick={onAddDomain}
            className="inline-flex h-10 items-center gap-2 rounded-md bg-primary px-4 text-[14px] font-medium text-on-primary hover:bg-primary-pressed"
          >
            <Plus size={16} /> 도메인 추가
          </button>
        </div>
      </div>
      {ghostFlash && (
        <div className="mt-3 rounded-md border border-hairline bg-surface px-3 py-2 text-[13px] text-mute">
          {ghostFlash}
        </div>
      )}

      {/* Project-scoped search */}
      <div className="mt-4">
        <div className="flex items-center gap-2 rounded-md border border-hairline bg-surface px-3">
          <Sparkles size={15} className="shrink-0 text-mute" />
          <input
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder={`"${project}" 안에서 검색…`}
            className="h-11 flex-1 bg-transparent text-[14px] text-ink placeholder:text-stone outline-none"
          />
          {searching && (
            <Loader2 size={14} className="shrink-0 animate-spin text-mute" />
          )}
          {query && !searching && (
            <button
              onClick={() => setQuery("")}
              className="grid h-6 w-6 place-items-center rounded-sm text-mute hover:bg-surface-elevated hover:text-on-dark"
              title="지우기"
            >
              <X size={13} />
            </button>
          )}
        </div>
        {query.trim() && (
          <div className="mt-2 max-h-[260px] overflow-auto rounded-md border border-hairline bg-surface">
            {results.length === 0 && !searching && (
              <div className="px-3 py-3 text-[12px] text-stone">
                매칭되는 노트가 없어요.
              </div>
            )}
            {results.map((hit) => (
              <button
                key={`${hit.project}/${hit.domain}`}
                onClick={() => selectDomain(hit.project, hit.domain)}
                className="flex w-full flex-col items-start gap-1 border-b border-hairline px-3 py-2 text-left last:border-b-0 hover:bg-surface-elevated"
              >
                <div className="flex w-full items-center justify-between gap-2">
                  <code className="truncate font-mono text-[12px] text-ink">
                    {hit.domain}
                  </code>
                  {hit.relevance > 0 && (
                    <span
                      className="shrink-0 rounded-xs bg-surface-elevated px-1.5 py-0.5 font-mono text-[10px] tabular-nums text-stone"
                      title="매칭 점수 (RRF 결합)"
                    >
                      {hit.relevance.toFixed(2)}
                    </span>
                  )}
                </div>
                {hit.snippet && (
                  <span className="line-clamp-2 whitespace-pre-wrap text-[11px] leading-[1.55] text-body">
                    <Highlighted text={hit.snippet} query={query} />
                  </span>
                )}
              </button>
            ))}
          </div>
        )}
      </div>
    </section>
  );
}

/** Highlight the user's query terms inside a search snippet. Splits the
 *  query on whitespace, escapes regex metacharacters, and wraps each
 *  match in a `<mark>` styled with accent-yellow. Case-insensitive but
 *  Korean-friendly (no \\b word boundaries — those break on CJK). */
function Highlighted({ text, query }: { text: string; query: string }) {
  const terms = query
    .trim()
    .split(/\s+/)
    .filter((t) => t.length > 0)
    .map((t) => t.replace(/[.*+?^${}()|[\]\\]/g, "\\$&"));
  if (terms.length === 0) return <>{text}</>;
  const re = new RegExp(`(${terms.join("|")})`, "gi");
  const parts = text.split(re);
  return (
    <>
      {parts.map((part, i) => {
        if (i % 2 === 1) {
          return (
            <mark
              key={i}
              className="rounded-xs bg-accent-yellow/30 px-0.5 text-accent-yellow"
            >
              {part}
            </mark>
          );
        }
        return <span key={i}>{part}</span>;
      })}
    </>
  );
}

/** Per-project reindex button. Used in two places:
 *  - inline in ProjectActions (compact = false, full label "재인덱싱")
 *  - elsewhere we may want a tiny chip-sized variant later (compact mode).
 *  The global reindex lives in Settings — this exists so users can refresh
 *  semantic search after dropping new docs into one project without
 *  paying to re-embed everything. */
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
            : // ProjectActions 카드의 핵심 액션 — accent-blue tint 로
              // 그래프 (기본) 와 도메인 추가 (primary CTA) 사이에서
              // 시각적 hierarchy 잡음.
              "h-10 border border-accent-blue/40 bg-accent-blue-soft px-4 text-[14px] font-medium text-accent-blue hover:border-accent-blue hover:bg-accent-blue/15",
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
function GoalsCard({ project }: { project: string }) {
  const [goals, setGoals] = useState<Goal[]>([]);
  const [loading, setLoading] = useState(false);
  const [adding, setAdding] = useState(false);
  const [draft, setDraft] = useState("");
  const [busyId, setBusyId] = useState<string | null>(null);

  const load = useCallback(async () => {
    setLoading(true);
    try {
      const g = await ipc.goalsList(project);
      setGoals(g);
    } catch (e) {
      console.error("[danbi] goals list failed", e);
      setGoals([]);
    } finally {
      setLoading(false);
    }
  }, [project]);

  useEffect(() => {
    load();
  }, [load]);

  async function handleAdd() {
    const title = draft.trim();
    if (!title) {
      setAdding(false);
      return;
    }
    try {
      await ipc.goalsAdd(project, title);
      setDraft("");
      setAdding(false);
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
    } catch (e) {
      console.error("[danbi] goals archive failed", e);
    } finally {
      setBusyId(null);
    }
  }

  return (
    <section className="rounded-lg border border-hairline bg-surface p-4">
      <div className="flex items-center justify-between">
        <div className="flex items-center gap-2">
          <Target className="h-3.5 w-3.5 text-stone" />
          <span className="text-[12px] font-semibold uppercase tracking-[0.6px] text-mute">
            Goals
          </span>
          {goals.length > 0 && (
            <span className="text-[11px] tabular-nums text-stone">
              {goals.length}
            </span>
          )}
        </div>
        <button
          type="button"
          onClick={() => {
            setAdding(true);
            setDraft("");
          }}
          className="flex items-center gap-1 rounded-md border border-hairline bg-surface-elevated px-2 py-1 text-[11px] text-mute hover:text-ink"
        >
          <Plus className="h-3 w-3" />
          추가
        </button>
      </div>

      {goals.length === 0 && !adding && !loading && (
        <p className="mt-3 text-[12px] text-stone">
          아직 goal 이 없어요. 이 프로젝트에서 지금 뭘 하려는지 한 줄로
          남겨두면 Claude 세션에 자동으로 노출돼요.
        </p>
      )}

      {goals.length > 0 && (
        <ul className="mt-3 flex flex-col gap-1.5">
          {goals.map((g) => (
            <li
              key={g.id}
              className="group flex items-start justify-between gap-3 rounded-md border border-hairline bg-surface-elevated px-3 py-2"
            >
              <div className="flex min-w-0 flex-col">
                <span className="truncate text-[13px] text-ink">
                  {g.title}
                </span>
                {g.note && (
                  <span className="mt-0.5 truncate text-[11px] text-stone">
                    {g.note}
                  </span>
                )}
              </div>
              <button
                type="button"
                disabled={busyId === g.id}
                onClick={() => handleArchive(g.id)}
                className="shrink-0 rounded-sm p-1 text-stone opacity-0 transition group-hover:opacity-100 hover:text-ink disabled:opacity-30"
                title="archive"
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

      {adding && (
        <div className="mt-3 flex items-center gap-2">
          <input
            autoFocus
            value={draft}
            onChange={(e) => setDraft(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter") {
                e.preventDefault();
                handleAdd();
              } else if (e.key === "Escape") {
                setAdding(false);
                setDraft("");
              }
            }}
            placeholder="예: v0.5 릴리즈 노트 정리"
            className="flex-1 rounded-md border border-hairline bg-surface-elevated px-2.5 py-1.5 text-[13px] text-ink placeholder:text-stone focus:border-ink focus:outline-none"
          />
          <button
            type="button"
            onClick={handleAdd}
            className="rounded-md bg-primary px-3 py-1.5 text-[12px] font-medium text-on-primary hover:opacity-90"
          >
            저장
          </button>
          <button
            type="button"
            onClick={() => {
              setAdding(false);
              setDraft("");
            }}
            className="rounded-md p-1.5 text-stone hover:text-ink"
          >
            <X className="h-3.5 w-3.5" />
          </button>
        </div>
      )}
    </section>
  );
}
