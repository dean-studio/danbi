import { useEffect, useMemo, useState } from "react";
import {
  CalendarDays,
  Clock4,
  FileText,
  FolderOpen,
  GitCommit,
  HardDrive,
  Layers,
  Plus,
  Sparkles,
} from "lucide-react";
import {
  ipc,
  type CommitSummary,
  type DailySnapshot,
} from "@/lib/ipc";
import { cn } from "@/lib/utils";
import { useApp } from "@/state/store";
import { McpInboundCard } from "./McpInboundCard";

export function Home({ onAddProject }: { onAddProject: () => void }) {
  const cfg = useApp((s) => s.cfg);
  const tree = useApp((s) => s.tree);
  const selectDomain = useApp((s) => s.selectDomain);
  const selectProject = useApp((s) => s.selectProject);

  const [commits, setCommits] = useState<CommitSummary[]>([]);
  const [daily, setDaily] = useState<DailySnapshot | null>(null);

  useEffect(() => {
    ipc
      .recentCommits(200)
      .then(setCommits)
      .catch(() => setCommits([]));
    ipc
      .dailySnapshot()
      .then(setDaily)
      .catch(() => setDaily(null));
  }, [tree]);


  const stats = useMemo(() => {
    const projects = tree?.projects ?? [];
    const domainCount = projects.reduce((a, p) => a + p.domains.length, 0);
    const bytes = projects.reduce(
      (a, p) => a + p.domains.reduce((b, d) => b + d.bytes, 0),
      0,
    );
    return {
      projectCount: projects.length,
      domainCount,
      bytes,
    };
  }, [tree]);

  const activity = useMemo(() => build30DayActivity(commits), [commits]);

  return (
    <div className="flex h-full flex-col">
      <header
        data-tauri-drag-region
        className="flex h-12 shrink-0 items-center justify-between border-b border-hairline px-6"
      >
        <span className="text-[15px] font-medium tracking-[0.2px] text-ink">
          홈
        </span>
        <span className="text-[13px] text-stone">
          {cfg?.vault_path?.split("/").slice(-1)[0] ?? "Vault"}
        </span>
      </header>

      <div className="flex-1 min-h-0 overflow-auto">
        <div className="mx-auto max-w-[960px] px-7 py-9">
          <h1 className="text-[32px] font-medium leading-tight text-ink">
            좋은 하루예요 👋
          </h1>
          <p className="mt-2 text-[15px] text-mute">
            좌측에서 도메인을 고르거나 바로 명령해도 돼요.
          </p>

          {/* MCP inbound — Claude Code / Codex 가 단비에 저장한 콘텐츠 추정량.
              0.4.0 부터 홈 최상단으로 노출 (활동 신호가 가장 먼저 보이도록). */}
          <McpInboundCard onOpenProject={selectProject} />

          {/* Claude Code 통합 셋업 — Skill 한 번 설치 / CLAUDE.md 프로젝트별 */}
          <SkillSetupCard />

          {/* Daily */}
          {daily && <DailyCard snapshot={daily} onOpen={selectDomain} />}

          {/* Stats row — accent strip 4종 */}
          <div className="mt-7 grid grid-cols-4 gap-3">
            <Stat
              icon={<Layers size={16} />}
              label="프로젝트"
              value={String(stats.projectCount)}
              hint="projects"
              accent="blue"
            />
            <Stat
              icon={<FileText size={16} />}
              label="도메인"
              value={String(stats.domainCount)}
              hint="markdown"
              accent="green"
            />
            <Stat
              icon={<HardDrive size={16} />}
              label="총 용량"
              value={prettyBytes(stats.bytes)}
              hint="all text"
              accent="yellow"
            />
            <Stat
              icon={<GitCommit size={16} />}
              label="최근 작업"
              value={commits.length > 0 ? timeAgo(commits[0].ts * 1000) : "—"}
              hint="last commit"
              accent="red"
            />
          </div>

          {/* 30일 활동 — 사용량 카드는 0.1 정체성 (LLM 키 0개) 와 안 어울려서 제거 */}
          <div className="mt-6">
            <ActivityCard commits={commits.length} data={activity} />
          </div>

          {/* Project grid */}
          <section className="mt-9">
            <div className="mb-4 flex items-center justify-between">
              <h2 className="text-[18px] font-medium text-ink">프로젝트</h2>
              <button
                onClick={onAddProject}
                className="inline-flex h-8 items-center gap-1.5 rounded-sm border border-hairline bg-surface-elevated px-3 text-[13px] text-body transition-colors hover:border-hairline-strong hover:text-on-dark"
              >
                <Plus size={13} /> 새 프로젝트
              </button>
            </div>
            {tree && tree.projects.length === 0 ? (
              <div className="flex flex-col items-center gap-3 rounded-lg border border-dashed border-hairline bg-surface py-12 text-center">
                <FolderOpen size={22} className="text-stone" />
                <div className="text-[13px] text-mute">
                  아직 프로젝트가 없어요.
                  <br />
                  <span className="text-stone">
                    좌측 상단 +버튼 혹은 여기에서 시작해 보세요.
                  </span>
                </div>
                <button
                  onClick={onAddProject}
                  className="inline-flex h-8 items-center rounded-md bg-primary px-3 text-[13px] font-medium text-on-primary hover:bg-primary-pressed"
                >
                  프로젝트 만들기
                </button>
              </div>
            ) : (
              <div className="grid grid-cols-2 gap-3">
                {tree?.projects.map((p) => {
                  const lastModified = p.domains.reduce(
                    (a, d) => Math.max(a, d.modified_ms ?? 0),
                    0,
                  );
                  const bytes = p.domains.reduce((a, d) => a + d.bytes, 0);
                  const empty = p.domains.length === 0;
                  return (
                    <button
                      key={p.name}
                      onClick={() => selectProject(p.name)}
                      className={cn(
                        "group flex flex-col items-stretch rounded-lg border p-4 text-left transition-colors",
                        empty
                          ? "border-dashed border-hairline bg-surface/60 hover:border-hairline-strong"
                          : "border-hairline bg-surface-elevated hover:border-hairline-strong",
                      )}
                    >
                      <div className="flex items-center justify-between gap-2">
                        <span
                          className={cn(
                            "truncate text-[16px] font-medium",
                            empty ? "text-mute" : "text-ink",
                          )}
                        >
                          {p.name}
                        </span>
                        <span
                          className={cn(
                            "shrink-0 rounded-xs px-2 py-0.5 font-mono text-[11px]",
                            empty
                              ? "bg-surface text-stone"
                              : "bg-accent-blue-soft text-accent-blue",
                          )}
                        >
                          {p.domains.length}
                        </span>
                      </div>
                      {!empty && (
                        <div className="mt-2.5 flex flex-wrap gap-1">
                          {p.domains.slice(0, 4).map((d) => (
                            <span
                              key={d.name}
                              className="inline-flex items-center gap-1 rounded-xs bg-surface px-2 py-0.5 font-mono text-[12px] text-on-dark-mute"
                            >
                              <FileText size={10} className="text-stone" />
                              {d.name}
                            </span>
                          ))}
                          {p.domains.length > 4 && (
                            <span className="text-[12px] text-stone">
                              +{p.domains.length - 4}
                            </span>
                          )}
                        </div>
                      )}
                      <div className="mt-3.5 flex items-center justify-between text-[12px] text-stone">
                        <span>
                          {lastModified > 0
                            ? `최근 ${timeAgo(lastModified)}`
                            : "비어있음"}
                        </span>
                        <span className="font-mono">{prettyBytes(bytes)}</span>
                      </div>
                    </button>
                  );
                })}
              </div>
            )}
          </section>

          {/* Recent activity — table */}
          {commits.length > 0 && (
            <section className="mt-7 overflow-hidden rounded-lg border border-hairline bg-surface">
              <header className="flex items-center justify-between border-b border-hairline px-4 py-2.5">
                <div className="flex items-center gap-1.5">
                  <GitCommit size={13} className="text-accent-blue" />
                  <h2 className="text-[14px] font-medium text-ink">
                    최근 작업
                  </h2>
                </div>
                <span className="text-caption-sm text-stone">
                  {commits.length.toLocaleString()} commits
                </span>
              </header>
              <table className="w-full text-caption-md">
                <tbody className="divide-y divide-hairline">
                  {commits.slice(0, 8).map((c) => (
                    <tr key={c.id} className="hover:bg-surface-elevated">
                      <td className="px-4 py-2 font-mono text-[11px] text-stone">
                        {c.id.slice(0, 7)}
                      </td>
                      <td className="truncate px-2 py-2 text-body">
                        {c.summary}
                      </td>
                      <td className="px-4 py-2 text-right text-caption-sm text-stone">
                        {timeAgo(c.ts * 1000)}
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </section>
          )}

        </div>
      </div>
    </div>
  );
}

function DailyCard({
  snapshot,
  onOpen,
}: {
  snapshot: DailySnapshot;
  onOpen: (project: string, domain: string) => void;
}) {
  const tree = useApp((s) => s.tree);
  const [creating, setCreating] = useState(false);

  async function createForProject(project: string) {
    setCreating(true);
    try {
      const domain = await ipc.ensureTodayNote(project);
      onOpen(project, domain);
    } finally {
      setCreating(false);
    }
  }

  const hasPast =
    snapshot.one_week_ago.length +
      snapshot.one_month_ago.length +
      snapshot.one_year_ago.length >
    0;

  const projects = tree?.projects ?? [];

  return (
    <section className="mt-7 rounded-lg border border-hairline bg-surface p-5">
      <header className="flex items-center justify-between">
        <div className="flex items-center gap-2.5">
          <CalendarDays size={16} className="text-accent-blue" />
          <h2 className="text-[18px] font-medium text-ink">오늘</h2>
          <span className="font-mono text-[13px] text-stone">
            {snapshot.today}
          </span>
        </div>
        <div className="flex items-center gap-1.5">
          {snapshot.today_notes.length === 0 &&
            projects.length > 0 &&
            projects.slice(0, 3).map((p) => (
              <button
                key={p.name}
                onClick={() => createForProject(p.name)}
                disabled={creating}
                className="inline-flex h-6 items-center gap-1 rounded-sm border border-hairline bg-surface-elevated px-2 text-[11px] text-body hover:border-hairline-strong hover:text-on-dark"
              >
                <Plus size={10} />
                {p.name}
              </button>
            ))}
        </div>
      </header>

      {snapshot.today_notes.length > 0 ? (
        <ul className="mt-3 flex flex-col gap-1">
          {[...snapshot.today_notes]
            .sort((a, b) => (b.modified_ms ?? 0) - (a.modified_ms ?? 0))
            .map((n) => (
            <li key={`${n.project}/${n.domain}`}>
              <button
                onClick={() => onOpen(n.project, n.domain)}
                className="flex w-full items-center gap-2.5 rounded-sm px-2 py-1.5 text-left transition-colors hover:bg-surface-elevated"
              >
                <FileText size={13} className="text-mute" />
                <span className="text-[14px] text-mute">{n.project}</span>
                <span className="text-stone">/</span>
                <span className="font-mono text-[14px] text-body">
                  {n.domain}
                </span>
                <span className="ml-auto flex items-center gap-3 text-[12px] text-stone">
                  {n.modified_ms && (
                    <span className="font-mono" title={new Date(n.modified_ms).toLocaleString()}>
                      {formatHM(n.modified_ms)}
                    </span>
                  )}
                  <span>
                    {n.bytes > 0 ? `${(n.bytes / 1024).toFixed(1)}KB` : "empty"}
                  </span>
                </span>
              </button>
            </li>
          ))}
        </ul>
      ) : (
        <div className="mt-3 text-[13px] text-stone">
          오늘 작성한 데일리 노트가 없어요. 프로젝트별로 시작할 수 있어요.
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
            <ReminiscenceRow
              label="1주 전"
              notes={snapshot.one_week_ago}
              onOpen={onOpen}
            />
            <ReminiscenceRow
              label="1개월 전"
              notes={snapshot.one_month_ago}
              onOpen={onOpen}
            />
            <ReminiscenceRow
              label="1년 전"
              notes={snapshot.one_year_ago}
              onOpen={onOpen}
            />
          </div>
        </div>
      )}
    </section>
  );
}

function ReminiscenceRow({
  label,
  notes,
  onOpen,
}: {
  label: string;
  notes: DailySnapshot["one_week_ago"];
  onOpen: (project: string, domain: string) => void;
}) {
  if (notes.length === 0) return null;
  return (
    <div className="flex flex-wrap items-center gap-1.5 text-caption-sm">
      <span className="w-14 shrink-0 text-stone">{label}</span>
      {notes.map((n) => (
        <button
          key={`${n.project}/${n.domain}`}
          onClick={() => onOpen(n.project, n.domain)}
          className="inline-flex items-center gap-1 rounded-full border border-hairline bg-surface-elevated px-2 py-0.5 text-[11px] text-body hover:border-hairline-strong hover:text-on-dark"
        >
          <span className="text-mute">{n.project}</span>
          <span className="text-stone">·</span>
          <span className="font-mono text-[10px]">{n.date}</span>
        </button>
      ))}
    </div>
  );
}

/** Stat card with a left-edge accent strip so the four-up row reads as
 *  distinct buckets without relying on heavy color fills. */
function Stat({
  icon,
  label,
  value,
  hint,
  accent,
}: {
  icon: React.ReactNode;
  label: string;
  value: string;
  hint: string;
  accent: "blue" | "green" | "yellow" | "red";
}) {
  const stripMap: Record<typeof accent, string> = {
    blue: "bg-accent-blue",
    green: "bg-accent-green",
    yellow: "bg-accent-yellow",
    red: "bg-accent-red",
  };
  const iconMap: Record<typeof accent, string> = {
    blue: "text-accent-blue",
    green: "text-accent-green",
    yellow: "text-accent-yellow",
    red: "text-accent-red",
  };
  return (
    <div className="relative overflow-hidden rounded-lg border border-hairline bg-surface px-4 py-4 pl-5">
      <span
        className={cn(
          "absolute inset-y-0 left-0 w-0.5",
          stripMap[accent],
        )}
      />
      <div className="flex items-center gap-2">
        <span className={iconMap[accent]}>{icon}</span>
        <span className="text-[12px] uppercase tracking-[0.4px] text-stone">
          {label}
        </span>
      </div>
      <span className="mt-1.5 block text-[26px] font-medium leading-none text-ink">
        {value}
      </span>
      <span className="mt-1.5 block text-[12px] text-mute">{hint}</span>
    </div>
  );
}

function ActivityCard({
  commits,
  data,
}: {
  commits: number;
  data: number[];
}) {
  return (
    <section className="overflow-hidden rounded-lg border border-hairline bg-surface">
      <header className="flex items-center justify-between border-b border-hairline px-5 py-3">
        <div className="flex items-center gap-2">
          <GitCommit size={15} className="text-accent-blue" />
          <h2 className="text-[16px] font-medium text-ink">30일 활동</h2>
        </div>
        <span className="rounded-xs bg-accent-blue-soft px-2 py-0.5 text-[12px] font-medium text-accent-blue">
          {commits.toLocaleString()} commits
        </span>
      </header>
      <div className="px-5 py-4">
        <Sparkline data={data} />
      </div>
    </section>
  );
}

function Sparkline({ data }: { data: number[] }) {
  const max = Math.max(1, ...data);
  return (
    <div className="flex h-14 items-end gap-[3px]">
      {data.map((v, i) => {
        const pct = (v / max) * 100;
        return (
          <div
            key={i}
            className={cn(
              "flex-1 rounded-xs transition-colors",
              v === 0
                ? "bg-hairline"
                : v / max > 0.66
                  ? "bg-accent-blue"
                  : "bg-accent-blue-soft",
            )}
            style={{ height: `${Math.max(pct, v === 0 ? 6 : 12)}%` }}
            title={`${v} commits · ${i + 1}일 전`}
          />
        );
      })}
    </div>
  );
}

function build30DayActivity(commits: CommitSummary[]): number[] {
  const bins = new Array(30).fill(0);
  const now = Date.now();
  for (const c of commits) {
    const age = now - c.ts * 1000;
    const day = Math.floor(age / (24 * 60 * 60 * 1000));
    if (day >= 0 && day < 30) {
      bins[29 - day] += 1;
    }
  }
  return bins;
}

function prettyBytes(n: number): string {
  if (n < 1024) return `${n}B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)}KB`;
  return `${(n / (1024 * 1024)).toFixed(1)}MB`;
}

function formatHM(ms: number): string {
  const d = new Date(ms);
  const h = String(d.getHours()).padStart(2, "0");
  const m = String(d.getMinutes()).padStart(2, "0");
  return `${h}:${m}`;
}

function timeAgo(ms: number): string {
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

/** Top-of-Home card explaining the two-step Claude Code 통합:
 *   1. MCP 서버 등록 (사이드바 → 프로젝트 우클릭 → Claude Code 설치 명령)
 *   2. Skill 설치 (각 프로젝트 홈 헤더의 "Skill 설치" 버튼)
 *  Skill 은 프로젝트별로 분리돼서 각 SKILL 안에 그 프로젝트의 scoped
 *  endpoint URL 이 박힘. 여기서는 일괄 설치/갱신 단축 버튼 + 현재
 *  설치 현황만 보여준다.
 */
function SkillSetupCard() {
  const tree = useApp((s) => s.tree);
  const projects = useMemo(
    () => tree?.projects.map((p) => p.name) ?? [],
    [tree],
  );
  const [installed, setInstalled] = useState<Record<string, boolean>>({});
  const [busy, setBusy] = useState(false);
  const [flash, setFlash] = useState<string | null>(null);

  // Refresh per-project install status whenever the project list changes.
  useEffect(() => {
    let cancelled = false;
    (async () => {
      const next: Record<string, boolean> = {};
      for (const p of projects) {
        try {
          next[p] = await ipc.skillStatus(p);
        } catch {
          next[p] = false;
        }
      }
      if (!cancelled) setInstalled(next);
    })();
    return () => {
      cancelled = true;
    };
  }, [projects]);

  useEffect(() => {
    if (!flash) return;
    const t = setTimeout(() => setFlash(null), 4000);
    return () => clearTimeout(t);
  }, [flash]);

  const installAll = async () => {
    setBusy(true);
    try {
      for (const p of projects) {
        await ipc.installSkill(p);
      }
      const next: Record<string, boolean> = {};
      for (const p of projects) next[p] = true;
      setInstalled(next);
      setFlash(`${projects.length} 개 프로젝트 Skill 설치/갱신 완료`);
    } catch (e) {
      setFlash(`실패: ${e}`);
    } finally {
      setBusy(false);
    }
  };

  const installedCount = Object.values(installed).filter(Boolean).length;

  return (
    <section className="mt-6 rounded-lg border border-hairline bg-surface p-4">
      <header className="flex items-start justify-between gap-3">
        <div className="flex items-start gap-2.5 min-w-0">
          <Sparkles size={16} className="mt-[2px] shrink-0 text-accent-blue" />
          <div className="min-w-0">
            <div className="text-[14px] font-semibold text-ink">
              Claude Code 통합
            </div>
            <div className="mt-0.5 text-[12px] leading-[1.6] text-mute">
              <strong className="text-on-dark">Skill</strong> 은 프로젝트별로 만들어져요 — 각
              SKILL 에 그 프로젝트의 scoped MCP endpoint 가 박혀서, Claude
              가 "Bonny 어제 뭐 했지" 같은 발화에 정확한 vault 로 라우팅.
              모든 프로젝트 한 번에 설치하려면 오른쪽 버튼.
              <strong className="text-on-dark"> CLAUDE.md</strong> 는 프로젝트별 강제 규칙이
              필요할 때만 보조 (사이드바 우클릭 메뉴).
            </div>
          </div>
        </div>
        <button
          type="button"
          onClick={installAll}
          disabled={busy || projects.length === 0}
          className={cn(
            "inline-flex h-8 shrink-0 items-center gap-1.5 rounded-md border px-3 text-[12px] transition-colors disabled:opacity-60",
            installedCount === projects.length && projects.length > 0
              ? "border-accent-green/40 bg-accent-green-soft text-accent-green hover:border-accent-green"
              : "border-hairline bg-surface-elevated text-body hover:border-hairline-strong hover:text-on-dark",
          )}
        >
          <Sparkles size={12} />
          {projects.length === 0
            ? "프로젝트 없음"
            : busy
              ? "설치 중…"
              : installedCount === projects.length
                ? "모두 갱신"
                : `Skill ${projects.length} 개 설치`}
        </button>
      </header>
      {flash && (
        <div className="mt-2 text-[11px] text-accent-green">✓ {flash}</div>
      )}
      {projects.length > 0 && (
        <div className="mt-3 grid grid-cols-3 gap-2 text-[11px]">
          {projects.map((p) => (
            <div
              key={p}
              className={cn(
                "flex items-center gap-1.5 rounded-md border px-2 py-1.5",
                installed[p]
                  ? "border-accent-green/40 bg-accent-green-soft/30"
                  : "border-hairline bg-surface-elevated",
              )}
            >
              <span
                className={cn(
                  "h-1.5 w-1.5 shrink-0 rounded-full",
                  installed[p] ? "bg-accent-green" : "bg-stone",
                )}
              />
              <span className="truncate text-body">{p}</span>
            </div>
          ))}
        </div>
      )}
    </section>
  );
}
