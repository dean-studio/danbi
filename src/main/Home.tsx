import { useEffect, useMemo, useState } from "react";
import {
  BarChart3,
  CalendarDays,
  CheckCheck,
  Clock4,
  FileText,
  GitCommit,
  Plus,
  Server,
} from "lucide-react";
import {
  ipc,
  type CommitSummary,
  type DailySnapshot,
} from "@/lib/ipc";
import { Keycap } from "@/main/ShortcutsHelp";
import { markVaultAllRead } from "@/main/Sidebar";
import { cn } from "@/lib/utils";
import { useApp } from "@/state/store";

/** 오늘(홈). 아침에 열면 "오늘 무슨 일이 있었나" 가 먼저 — DailyCard 가
 *  최상단, 사용량 카드는 사용량 화면으로 이동. 활동 피드(2단계)가
 *  들어오기 전까지 "최근 작업" 은 커밋 요약을 사람이 읽을 수 있는 행으로
 *  풀어 보여주고 클릭하면 문서가 열린다. */
export function Home({ onOpenShortcuts }: { onOpenShortcuts?: () => void }) {
  const tree = useApp((s) => s.tree);
  const selectDomain = useApp((s) => s.selectDomain);
  const setView = useApp((s) => s.setView);
  const openConnectSheet = useApp((s) => s.openConnectSheet);

  const [commits, setCommits] = useState<CommitSummary[]>([]);
  const [daily, setDaily] = useState<DailySnapshot | null>(null);
  const [marking, setMarking] = useState(false);

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

  const recent = useMemo(
    () =>
      commits
        .filter((c) => !c.summary.includes("(pre)"))
        .map((c) => ({ ...c, parsed: parseCommitSummary(c.summary) }))
        .slice(0, 12),
    [commits],
  );

  // 빈 vault (프로젝트 0개) 에선 집계 카드를 감춘다 — 첫 프로젝트를
  // 만들기 전까진 환영 + 시작 안내만.
  const hasProjects = (tree?.projects?.length ?? 0) > 0;
  const today = new Date();
  const dateLabel = `${today.getFullYear()}-${String(today.getMonth() + 1).padStart(2, "0")}-${String(today.getDate()).padStart(2, "0")} (${"일월화수목금토"[today.getDay()]})`;

  return (
    <div className="flex h-full flex-col">
      <header
        data-tauri-drag-region
        className="flex h-12 shrink-0 items-center justify-between border-b border-hairline px-6"
      >
        <div className="flex items-center gap-2">
          <span className="text-[15px] font-medium tracking-[0.2px] text-ink">
            오늘
          </span>
          <span className="font-mono text-[12px] text-stone">{dateLabel}</span>
        </div>
        <div className="flex items-center gap-1.5">
          <button
            onClick={async () => {
              setMarking(true);
              try {
                await markVaultAllRead();
              } finally {
                setMarking(false);
              }
            }}
            disabled={marking}
            title="사이드바의 변경 표시를 모두 읽음으로"
            className="inline-flex h-7 items-center gap-1.5 rounded-md border border-hairline bg-surface-elevated px-2.5 text-[12px] text-body hover:border-hairline-strong hover:text-on-dark disabled:opacity-50"
          >
            <CheckCheck size={12} /> 읽음 처리
          </button>
          {onOpenShortcuts && (
            <button
              onClick={onOpenShortcuts}
              title="단축키 표"
              className="inline-flex h-7 items-center gap-1.5 rounded-md px-2 text-[12px] text-mute hover:bg-surface-elevated hover:text-on-dark"
            >
              <Keycap>⌘</Keycap>
              <Keycap>/</Keycap>
            </button>
          )}
        </div>
      </header>

      <div className="flex-1 min-h-0 overflow-auto">
        <div className="mx-auto max-w-[960px] px-7 py-9">
          {!hasProjects && (
            <>
              <h1 className="text-[32px] font-medium leading-tight text-ink">
                단비에 오신 걸 환영해요 👋
              </h1>
              <p className="mt-2 text-[15px] text-mute">
                아직 vault 가 비어 있어요. 좌측 사이드바의 + 로 첫 프로젝트를
                만들면 여기에 활동이 채워집니다.
              </p>
            </>
          )}

          {hasProjects && (
            <>
              {/* 오늘 — 최상단. */}
              {daily && <DailyCard snapshot={daily} onOpen={selectDomain} first />}

              {/* 최근 작업 — 커밋 요약을 사람이 읽을 행으로. */}
              {recent.length > 0 && (
                <section className="mt-7 overflow-hidden rounded-lg border border-hairline bg-surface">
                  <header className="flex items-center justify-between border-b border-hairline px-4 py-2.5">
                    <div className="flex items-center gap-1.5">
                      <GitCommit size={13} className="text-mute" />
                      <h2 className="text-[14px] font-medium text-ink">최근 작업</h2>
                    </div>
                    <span className="text-caption-sm text-stone">
                      {commits.length.toLocaleString()} commits
                    </span>
                  </header>
                  <ul className="divide-y divide-hairline">
                    {recent.map((c) => {
                      const { op, project, domain } = c.parsed;
                      const openable = !!project && !!domain;
                      return (
                        <li key={c.id}>
                          <button
                            onClick={() => {
                              if (openable) selectDomain(project!, domain!);
                            }}
                            disabled={!openable}
                            title={c.summary}
                            className={cn(
                              "flex w-full items-center gap-3 px-4 py-2 text-left transition-colors",
                              openable
                                ? "hover:bg-surface-elevated"
                                : "cursor-default",
                            )}
                          >
                            <span className="w-[44px] shrink-0 font-mono text-[11px] text-stone">
                              {formatHM(c.ts * 1000)}
                            </span>
                            <span className="shrink-0 rounded-xs bg-surface-elevated px-1.5 py-px text-[10px] font-medium uppercase tracking-[0.2px] text-on-dark-mute">
                              {op}
                            </span>
                            {project ? (
                              <>
                                <span className="shrink-0 text-[12px] text-mute">{project}</span>
                                <span className="min-w-0 flex-1 truncate font-mono text-[12px] text-body">
                                  {domain}
                                </span>
                              </>
                            ) : (
                              <span className="min-w-0 flex-1 truncate text-[12px] text-body">
                                {c.summary}
                              </span>
                            )}
                            <span className="shrink-0 text-caption-sm text-stone">
                              {timeAgo(c.ts * 1000)}
                            </span>
                          </button>
                        </li>
                      );
                    })}
                  </ul>
                </section>
              )}

              {/* 사용량·에이전트는 별도 화면/시트 — 입구만. */}
              <div className="mt-6 flex flex-wrap items-center gap-2">
                <button
                  onClick={() => setView("usage")}
                  className="inline-flex h-8 items-center gap-1.5 rounded-md border border-hairline bg-surface-elevated px-3 text-[12px] text-body hover:border-hairline-strong hover:text-on-dark"
                >
                  <BarChart3 size={12} /> 사용량 보기
                  <Keycap>⌘⇧U</Keycap>
                </button>
                <button
                  onClick={() => openConnectSheet(null)}
                  className="inline-flex h-8 items-center gap-1.5 rounded-md border border-hairline bg-surface-elevated px-3 text-[12px] text-body hover:border-hairline-strong hover:text-on-dark"
                >
                  <Server size={12} /> 에이전트 연결 관리
                </button>
              </div>
            </>
          )}
        </div>
      </div>
    </div>
  );
}

/** `danbi: mcp log · 단비/daily/2026-10-15.md · …` 같은 커밋 요약을
 *  op / project / domain 으로. 형식이 다르면 project=null (원문 표시). */
function parseCommitSummary(summary: string): {
  op: string;
  project: string | null;
  domain: string | null;
} {
  const body = summary.replace(/^danbi:\s*/, "");
  const parts = body.split(" · ").map((p) => p.trim());
  let project: string | null = null;
  let domain: string | null = null;
  for (const part of parts) {
    const m = part.match(/^([^/\s][^/]*)\/(.+\.md)$/);
    if (m) {
      project = m[1];
      domain = m[2];
      break;
    }
  }
  const rawOp = parts[0] ?? body;
  const op = OP_LABELS[rawOp] ?? OP_LABELS[rawOp.split(" ")[0]] ?? rawOp.slice(0, 14);
  return { op, project, domain };
}

const OP_LABELS: Record<string, string> = {
  "mcp log": "로그",
  "mcp append": "추가",
  "mcp create_file": "생성",
  "mcp create_folder": "폴더",
  "mcp replace_section": "섹션 교체",
  "mcp upsert_item": "항목 갱신",
  "rest overwrite": "REST 저장",
  "rest append": "REST 추가",
  "editor": "저장",
  "save": "저장",
  "quick_capture": "캡처",
  "restore before": "되돌림",
  "restore at": "복원",
  "undo": "되돌림",
  "apply": "AI 편집",
  "compound": "Compound",
  "create": "생성",
  "rename": "이름 변경",
  "delete": "삭제",
  "move": "이동",
  "session-end": "세션 기록",
};

export function cardFallback(error: Error, reset: () => void) {
  return (
    <section className="mt-7 rounded-lg border border-hairline bg-surface px-4 py-4">
      <div className="text-[13px] text-on-dark">
        이 카드를 표시하는 중 문제가 생겼어요.
      </div>
      <div className="mt-1 truncate text-[11px] text-stone" title={error.message}>
        {error.message}
      </div>
      <button
        type="button"
        onClick={reset}
        className="mt-2 inline-flex h-7 items-center rounded-md border border-hairline bg-surface-elevated px-2.5 text-[11px] text-body transition-colors hover:border-hairline-strong hover:text-on-dark"
      >
        다시 시도
      </button>
    </section>
  );
}

function DailyCard({
  snapshot,
  onOpen,
  first = false,
}: {
  snapshot: DailySnapshot;
  onOpen: (project: string, domain: string) => void;
  /** 최상단 카드면 위 여백 없음. */
  first?: boolean;
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
    <section className={cn("rounded-lg border border-hairline bg-surface p-5", !first && "mt-7")}>
      <header className="flex items-center justify-between">
        <div className="flex items-center gap-2.5">
          <CalendarDays size={16} className="text-mute" />
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
            <Clock4 size={11} className="text-stone" />
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

export function prettyBytes(n: number): string {
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

