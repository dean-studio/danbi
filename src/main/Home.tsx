import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { CheckCheck, ChevronDown, Clock4, Send, Server } from "lucide-react";
import {
  ipc,
  type ActivityEntry,
  type DailySnapshot,
} from "@/lib/ipc";
import { ActivityFeed, clientLabel, type ActivityStats } from "@/main/ActivityFeed";
import { useMcpSummary } from "@/main/ConnectSheet";
import { ReviewMini } from "@/main/ReviewMini";
import { Keycap } from "@/main/ShortcutsHelp";
import { markVaultAllRead } from "@/main/Sidebar";
import { cn } from "@/lib/utils";
import { useApp } from "@/state/store";

/** ⌘J — Workspace 가 쏘고 빠른 메모 입력이 듣는 포커스 신호. */
export const FOCUS_MEMO_EVENT = "danbi:focus-memo";

/** 오늘(홈, 화면 A). 아침에 열면 30초 안에 "어제 이후 누가 무엇을 썼고
 *  내가 봐야 할 게 몇 개인지" 를 알고, 떠오른 메모를 바로 남긴다.
 *  빠른 메모(흰 CTA 1개) → 요약 한 줄 → 활동 피드 | 검토함·지난 오늘·
 *  에이전트. LLM 없이 동작(피드=git, 검토=ghost store+healing, 메모=
 *  quick capture 의 LLM 우회 경로). */
export function Home({ onOpenShortcuts }: { onOpenShortcuts?: () => void }) {
  const tree = useApp((s) => s.tree);
  const selectDomain = useApp((s) => s.selectDomain);
  const setView = useApp((s) => s.setView);
  const openConnectSheet = useApp((s) => s.openConnectSheet);

  const [scope, setScope] = useState<string | null>(null);
  const [stats, setStats] = useState<ActivityStats | null>(null);
  const [reviewCount, setReviewCount] = useState(0);
  const [optimistic, setOptimistic] = useState<ActivityEntry[]>([]);
  const [marking, setMarking] = useState(false);

  const projects = useMemo(() => tree?.projects.map((p) => p.name) ?? [], [tree]);
  const hasProjects = projects.length > 0;
  const today = new Date();
  const dateLabel = `${today.getFullYear()}-${String(today.getMonth() + 1).padStart(2, "0")}-${String(today.getDate()).padStart(2, "0")} (${"일월화수목금토"[today.getDay()]})`;

  // 메모 전송 직후 피드 맨 위에 넣는 옵티미스틱 행. 워처 → 피드 갱신이
  // 수 초 안에 실제 커밋 행을 가져오면 id 가 달라 중복되니 8초 뒤 거둔다.
  const pushOptimistic = useCallback((e: ActivityEntry) => {
    setOptimistic((rows) => [e, ...rows]);
    window.setTimeout(() => {
      setOptimistic((rows) => rows.filter((r) => r.id !== e.id));
    }, 8000);
  }, []);

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
          {hasProjects && (
            <ScopeSelect projects={projects} value={scope} onChange={setScope} />
          )}
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
              className="inline-flex h-7 items-center gap-1 rounded-md px-2 text-[12px] text-mute hover:bg-surface-elevated hover:text-on-dark"
            >
              <Keycap>⌘</Keycap>
              <Keycap>/</Keycap>
            </button>
          )}
        </div>
      </header>

      <div className="flex-1 min-h-0 overflow-auto">
        <div className="mx-auto max-w-[1040px] px-6 py-6">
          {!hasProjects ? (
            <>
              <h1 className="text-[32px] font-medium leading-tight text-ink">
                단비에 오신 걸 환영해요 👋
              </h1>
              <p className="mt-2 text-[15px] text-mute">
                아직 vault 가 비어 있어요. 좌측 사이드바의 + 로 첫 프로젝트를
                만들면 여기에 활동이 채워집니다.
              </p>
            </>
          ) : (
            <>
              <QuickMemo projects={projects} onStored={pushOptimistic} />

              <SummaryLine
                stats={stats}
                reviewCount={reviewCount}
                onOpenReview={() => setView("review")}
              />

              <div className="mt-4 grid gap-5" style={{ gridTemplateColumns: "minmax(0,1fr) 320px" }}>
                <ActivityFeed
                  scopeProject={scope}
                  limit={50}
                  onOpenDoc={selectDomain}
                  onStats={setStats}
                  optimistic={optimistic}
                  className="rounded-lg border border-hairline bg-surface"
                />
                <div className="flex min-w-0 flex-col gap-4">
                  <ReviewMini
                    project={scope}
                    max={4}
                    onOpenDoc={selectDomain}
                    onOpenAll={() => setView("review")}
                    onCount={setReviewCount}
                  />
                  <PastTodayCard onOpen={selectDomain} />
                  <AgentsCard
                    stats={stats}
                    onManage={() => openConnectSheet(scope)}
                  />
                </div>
              </div>
            </>
          )}
        </div>
      </div>
    </div>
  );
}

/** 헤더 [전체 ▾] — 피드·검토함 범위. */
function ScopeSelect({
  projects,
  value,
  onChange,
}: {
  projects: string[];
  value: string | null;
  onChange: (v: string | null) => void;
}) {
  return (
    <label className="relative inline-flex h-7 items-center rounded-md border border-hairline bg-surface-elevated pl-2.5 pr-6 text-[12px] text-body hover:border-hairline-strong">
      <select
        value={value ?? ""}
        onChange={(e) => onChange(e.target.value || null)}
        className="absolute inset-0 cursor-pointer opacity-0"
        aria-label="범위"
      >
        <option value="">전체</option>
        {projects.map((p) => (
          <option key={p} value={p}>
            {p}
          </option>
        ))}
      </select>
      <span className="pointer-events-none truncate">{value ?? "전체"}</span>
      <ChevronDown size={11} className="pointer-events-none absolute right-2 text-mute" />
    </label>
  );
}

/** 빠른 메모 — 백엔드 quick_capture(LLM 없음)의
 *  메인 윈도우 입구. 프로젝트 칩 기본 = 마지막 사용 프로젝트, 도메인 칩
 *  기본 = daily(오늘 노트, 백엔드가 자동 라우팅). ↵ 전송, ⇧↵ 줄바꿈,
 *  ⌘J 포커스. 흰 CTA 는 이 [기록] 하나. */
function QuickMemo({
  projects,
  onStored,
}: {
  projects: string[];
  onStored: (e: ActivityEntry) => void;
}) {
  const tree = useApp((s) => s.tree);
  const lastProject = useApp((s) => s.cfg?.capture?.last_project ?? null);
  const showToast = useApp((s) => s.showToast);
  const [project, setProject] = useState<string>(
    () => (lastProject && projects.includes(lastProject) ? lastProject : projects[0]) ?? "",
  );
  const [domain, setDomain] = useState<string>("daily");
  const [text, setText] = useState("");
  const [busy, setBusy] = useState(false);
  const inputRef = useRef<HTMLTextAreaElement>(null);

  // 프로젝트 목록이 바뀌었는데(삭제 등) 현재 값이 사라졌으면 첫 항목으로.
  useEffect(() => {
    if (!projects.includes(project)) setProject(projects[0] ?? "");
  }, [projects, project]);

  useEffect(() => {
    function onFocus() {
      inputRef.current?.focus();
    }
    window.addEventListener(FOCUS_MEMO_EVENT, onFocus);
    return () => window.removeEventListener(FOCUS_MEMO_EVENT, onFocus);
  }, []);

  // 도메인 후보 — daily + 그 프로젝트의 최상위 .md.
  const domains = useMemo(() => {
    const node = tree?.projects.find((p) => p.name === project);
    return ["daily", ...(node?.domains.map((d) => d.name) ?? [])];
  }, [tree, project]);
  useEffect(() => {
    if (!domains.includes(domain)) setDomain("daily");
  }, [domains, domain]);

  async function send() {
    const message = text.trim();
    if (!message || !project || busy) return;
    setBusy(true);
    try {
      const res = await ipc.quickCapture(
        message,
        project,
        domain === "daily" ? undefined : domain,
      );
      if (res.status !== "stored") {
        showToast("err", "프로젝트를 먼저 골라주세요.");
        return;
      }
      setText("");
      onStored({
        id: `local-${Date.now()}`,
        ts: Math.floor(Date.now() / 1000),
        client: "human",
        actor: "human",
        project: res.project,
        domain: res.domain,
        op: "quick_capture",
        preview: message.split("\n")[0].slice(0, 60),
        files: [`Projects/${res.project}/${res.domain}`],
      });
      inputRef.current?.focus();
    } catch (e) {
      showToast("err", `기록 실패: ${String(e).slice(0, 120)}`);
    } finally {
      setBusy(false);
    }
  }

  const chip =
    "h-7 max-w-[160px] cursor-pointer appearance-none truncate rounded-full border border-hairline bg-surface-elevated pl-2.5 pr-6 text-[12px] text-body hover:border-hairline-strong focus:outline-none";

  return (
    <div className="flex items-center gap-2 rounded-md border border-hairline bg-surface-elevated px-2.5 focus-within:border-hairline-strong">
      <span className="relative shrink-0">
        <select
          value={project}
          onChange={(e) => setProject(e.target.value)}
          className={chip}
          aria-label="프로젝트"
        >
          {projects.map((p) => (
            <option key={p} value={p}>
              {p}
            </option>
          ))}
        </select>
        <ChevronDown size={11} className="pointer-events-none absolute right-2 top-2 text-mute" />
      </span>
      <span className="relative shrink-0">
        <select
          value={domain}
          onChange={(e) => setDomain(e.target.value)}
          className={chip}
          aria-label="도메인"
        >
          {domains.map((d) => (
            <option key={d} value={d}>
              {d === "daily" ? "daily (오늘)" : d}
            </option>
          ))}
        </select>
        <ChevronDown size={11} className="pointer-events-none absolute right-2 top-2 text-mute" />
      </span>
      <textarea
        ref={inputRef}
        value={text}
        onChange={(e) => setText(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "Enter" && !e.shiftKey && !e.nativeEvent.isComposing) {
            e.preventDefault();
            send();
          }
        }}
        rows={1}
        placeholder="지금 떠오른 것… (↵ 기록 · ⇧↵ 줄바꿈 · ⌘J 포커스)"
        className="min-h-[44px] flex-1 resize-none bg-transparent py-3 text-[14px] text-ink outline-none placeholder:text-stone"
        style={{ maxHeight: 140 }}
      />
      <button
        onClick={send}
        disabled={busy || !text.trim() || !project}
        title="기록 (↵)"
        className="inline-flex h-8 shrink-0 items-center gap-1.5 rounded-md bg-primary px-3 text-[12px] font-medium text-on-primary transition-colors hover:bg-primary-pressed disabled:opacity-40"
      >
        <Send size={12} /> 기록
      </button>
    </div>
  );
}

/** "어제 이후 14건 · Claude Code 9 · Codex 3 · 사람 2 · 검토 대기 6". */
function SummaryLine({
  stats,
  reviewCount,
  onOpenReview,
}: {
  stats: ActivityStats | null;
  reviewCount: number;
  onOpenReview: () => void;
}) {
  const parts: string[] = [];
  if (stats) {
    parts.push(`어제 이후 ${stats.sinceYesterday}건`);
    const by = Object.entries(stats.byClient).sort((a, b) => b[1] - a[1]);
    for (const [client, n] of by) parts.push(`${clientLabel(client)} ${n}`);
  }
  return (
    <div className="mt-2.5 flex flex-wrap items-center gap-x-1.5 px-1 text-[12px] text-mute">
      {parts.map((p, i) => (
        <span key={p} className="flex items-center gap-1.5">
          {i > 0 && <span className="text-stone">·</span>}
          <span>{p}</span>
        </span>
      ))}
      {reviewCount > 0 && (
        <>
          {parts.length > 0 && <span className="text-stone">·</span>}
          <button onClick={onOpenReview} className="text-body hover:text-on-dark">
            검토 대기 {reviewCount}
          </button>
        </>
      )}
      {parts.length === 0 && reviewCount === 0 && <span>&nbsp;</span>}
    </div>
  );
}

/** 지난 오늘 — 1주/1달/1년 전 daily. 기존 ReminiscenceRow 그대로. */
function PastTodayCard({
  onOpen,
}: {
  onOpen: (project: string, domain: string) => void;
}) {
  const tree = useApp((s) => s.tree);
  const [snap, setSnap] = useState<DailySnapshot | null>(null);
  useEffect(() => {
    ipc.dailySnapshot().then(setSnap).catch(() => setSnap(null));
  }, [tree]);
  if (!snap) return null;
  const total =
    snap.one_week_ago.length + snap.one_month_ago.length + snap.one_year_ago.length;
  if (total === 0) return null;
  return (
    <section className="rounded-lg border border-hairline bg-surface px-4 py-3">
      <div className="mb-2 flex items-center gap-1.5">
        <Clock4 size={11} className="text-stone" />
        <span className="text-caption-sm uppercase tracking-[0.4px] text-mute">
          지난 오늘
        </span>
      </div>
      <div className="flex flex-col gap-1.5">
        <ReminiscenceRow label="1주 전" notes={snap.one_week_ago} onOpen={onOpen} />
        <ReminiscenceRow label="1개월 전" notes={snap.one_month_ago} onOpen={onOpen} />
        <ReminiscenceRow label="1년 전" notes={snap.one_year_ago} onOpen={onOpen} />
      </div>
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

/** 에이전트 — MCP 상태 + 어제 이후 에이전트별 기록 수 + [연결 관리]. */
function AgentsCard({
  stats,
  onManage,
}: {
  stats: ActivityStats | null;
  onManage: () => void;
}) {
  const mcp = useMcpSummary();
  const agents = ["claude-code", "codex", "cursor"] as const;
  return (
    <section className="rounded-lg border border-hairline bg-surface px-4 py-3">
      <div className="mb-2 flex items-center justify-between">
        <span className="text-caption-sm uppercase tracking-[0.4px] text-mute">에이전트</span>
        <span className="flex items-center gap-1.5 text-[11px] text-mute">
          <span
            className={cn(
              "h-1.5 w-1.5 rounded-full",
              mcp.running ? "bg-accent-green" : "bg-stone",
            )}
          />
          {mcp.running ? `MCP :${mcp.port}` : "MCP 꺼짐"}
        </span>
      </div>
      <ul className="flex flex-col gap-1 text-[12px]">
        {agents.map((a) => {
          const n = stats?.byClient[a] ?? 0;
          return (
            <li key={a} className="flex items-center justify-between">
              <span className={n > 0 ? "text-body" : "text-mute"}>{clientLabel(a)}</span>
              <span className="font-mono text-[11px] text-stone">
                {n > 0 ? `${n}건` : "—"}
              </span>
            </li>
          );
        })}
        <li className="flex items-center justify-between">
          <span className="text-mute">연결 파일</span>
          <span className="font-mono text-[11px] text-stone">{mcp.connectionCount}개</span>
        </li>
      </ul>
      <button
        onClick={onManage}
        className="mt-2.5 inline-flex h-7 w-full items-center justify-center gap-1.5 rounded-md border border-hairline bg-surface-elevated text-[12px] text-body hover:border-hairline-strong hover:text-on-dark"
      >
        <Server size={12} /> 연결 관리
      </button>
    </section>
  );
}
