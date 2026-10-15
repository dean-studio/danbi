import { useCallback, useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import {
  Check,
  ChevronDown,
  ChevronRight,
  Copy,
  Loader2,
  RefreshCw,
  Server,
  Sparkles,
  X,
} from "lucide-react";
import { writeText as clipboardWriteText } from "@tauri-apps/plugin-clipboard-manager";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import {
  ipc,
  type AgentConnectResult,
  type AgentConnectionStatus,
  type AgentTarget,
  type McpProjectEndpoint,
  type McpRotateResult,
  type McpStatus,
} from "@/lib/ipc";
import { buildClaudeMdBlock } from "@/lib/claudeMdTemplate";
import { ClaudeIcon, CodexIcon, CursorIcon } from "@/components/AgentIcons";
import { cn } from "@/lib/utils";
import { useApp } from "@/state/store";

/**
 * 연결 시트 — 프로젝트별 "에이전트 연결" 의 유일한 자리 (UI 개편 §4.8).
 * 설정 MCP 섹션·프로젝트 홈 Hero/ConnectCard·홈 SkillSetupCard·훅 패널에
 * 흩어져 있던 것을 한 시트로. 데이터는 #9 의 agentConnect /
 * agentConnectionStatus / mcpRotateToken 을 그대로 소비한다.
 */

const SECONDARY_BTN =
  "inline-flex h-8 items-center gap-1.5 rounded-md border border-hairline bg-surface-elevated px-3 text-[12px] text-body hover:border-hairline-strong hover:text-on-dark disabled:opacity-50";
const ICON_BTN =
  "grid h-6 w-6 place-items-center rounded-sm text-stone hover:text-on-dark disabled:opacity-50";

type Flash = { kind: "ok" | "err"; msg: string } | null;

const AGENTS: { key: AgentTarget; label: string; icon: ReactNode; skill: boolean }[] = [
  { key: "claude", label: "Claude Code", icon: <ClaudeIcon size={13} />, skill: true },
  { key: "codex", label: "Codex", icon: <CodexIcon size={13} />, skill: true },
  { key: "cursor", label: "Cursor", icon: <CursorIcon size={13} />, skill: false },
];

function agentLabel(key: AgentTarget): string {
  return AGENTS.find((a) => a.key === key)?.label ?? key;
}

function clientLabel(client: string | null): string {
  if (client === "claude_code") return "Claude Code";
  if (client === "codex") return "Codex";
  if (client === "cursor") return "Cursor";
  return client ?? "";
}

function hhmm(ms: number): string {
  const d = new Date(ms);
  const sameDay = new Date().toDateString() === d.toDateString();
  const hm = `${String(d.getHours()).padStart(2, "0")}:${String(d.getMinutes()).padStart(2, "0")}`;
  if (sameDay) return hm;
  return `${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")} ${hm}`;
}

/** "마지막 접속 14:32 (Claude Code)" — 메모리 presence(핸드셰이크·읽기
 *  포함) 우선, 없으면 usage 로그의 마지막 쓰기("마지막 기록"). */
function lastRecordText(st: AgentConnectionStatus | null): string | null {
  if (!st) return null;
  const ts = st.last_call_at ?? st.last_write_at;
  if (!ts) return null;
  const client = st.last_call_at ? st.last_call_client : st.last_write_client;
  const who = clientLabel(client);
  const what = st.last_call_at ? "마지막 접속" : "마지막 기록";
  return `${what} ${hhmm(ts)}${who ? ` (${who})` : ""}`;
}

type AgentState = "connected" | "waiting" | "off" | "none";

/** 호출자 토큰(`claude-code`, `claude_code`, `codex`, `cursor`) → 에이전트. */
function agentOfClient(client: string | null | undefined): AgentTarget | null {
  const c = (client ?? "").replace(/_/g, "-");
  if (c.startsWith("claude")) return "claude";
  if (c.startsWith("codex")) return "codex";
  if (c.startsWith("cursor")) return "cursor";
  return null;
}

/** 띠·시트가 함께 쓰는 연결 판정. 파일을 썼다고 "연결됨"이 아니다 —
 *  서버가 켜져 있고 그 에이전트가 실제로 호출해 와야 연결됨. Skill 은
 *  전역 파일이라 판정에 섞지 않는다. */
function agentState(st: AgentConnectionStatus | null, agent: AgentTarget): AgentState {
  if (!st) return "none";
  const hasFile = st.connections.some((c) => c.agent === agent && c.exists);
  if (!hasFile) return "none";
  if (!st.server_running) return "off";
  const called =
    (st.last_call_at && agentOfClient(st.last_call_client) === agent) ||
    (st.last_write_at && agentOfClient(st.last_write_client) === agent);
  return called ? "connected" : "waiting";
}

const STATE_LABEL: Record<AgentState, string> = {
  connected: "연결됨",
  waiting: "연결 파일 기록 · 호출 대기",
  off: "MCP 서버 꺼짐",
  none: "미연결",
};

/** 연결 결과 한 줄 + 예전 `claude mcp add` 등록이 겹치면 안내 (ProjectHome
 *  ConnectCard 와 같은 문구). 사용자 설정 파일은 건드리지 않는다. */
function describeConnect(label: string, res: AgentConnectResult, skill: boolean): string {
  const local = res.legacy.filter((l) => l.scope === "local");
  return (
    (res.action === "updated" ? `${label} 연결을 갱신했어요` : `${res.path} 에 연결했어요`) +
    (skill ? " · SKILL 설치 완료" : "") +
    ` — ${label} 를 재시작하면 도구가 뜹니다.` +
    (local.length > 0
      ? ` 이 폴더에 예전 방식으로 등록한 ${local
          .map((l) => l.name)
          .join(", ")} 서버는 \`claude mcp remove ${local[0].name}\` 로 지우면 도구가 겹치지 않아요.`
      : "")
  );
}

/** 프로젝트 하나의 연결·스킬 상태. 시트와 상태 띠가 공유. */
function useProjectConnection(project: string | null, deps: unknown[] = []) {
  const [status, setStatus] = useState<AgentConnectionStatus | null>(null);
  const [skills, setSkills] = useState<Record<AgentTarget, boolean>>({
    claude: false,
    codex: false,
    cursor: false,
  });
  const latest = useRef(project);
  latest.current = project;
  const reload = useCallback(async () => {
    if (!project) {
      setStatus(null);
      return;
    }
    const [st, claude, codex] = await Promise.all([
      ipc.agentConnectionStatus(project).catch(() => null),
      ipc.skillStatus(project, "claude").catch(() => false),
      ipc.skillStatus(project, "codex").catch(() => false),
    ]);
    // 프로젝트를 빠르게 바꾸면 이전 응답이 늦게 와서 덮어쓸 수 있다.
    if (latest.current !== project) return;
    setStatus(st);
    setSkills({ claude, codex, cursor: false });
  }, [project]);
  useEffect(() => {
    reload();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [reload, ...deps]);
  const connectedAgents = (status?.connections ?? [])
    .filter((c) => c.exists)
    .map((c) => c.agent);
  return { status, skills, connectedAgents, reload };
}

/** 사이드바 푸터 점용 — 서버 상태 + 단비가 기록한 연결 파일 수. */
export function useMcpSummary(): {
  running: boolean;
  port: number;
  connectionCount: number;
} {
  const [summary, setSummary] = useState({ running: false, port: 47921, connectionCount: 0 });
  useEffect(() => {
    let alive = true;
    const load = async () => {
      const [st, list] = await Promise.all([
        ipc.mcpStatus().catch(() => null),
        ipc.mcpConnectionsList().catch(() => []),
      ]);
      if (!alive) return;
      setSummary({
        running: !!st?.running,
        port: st?.port ?? 47921,
        connectionCount: list.length,
      });
    };
    load();
    window.addEventListener("focus", load);
    return () => {
      alive = false;
      window.removeEventListener("focus", load);
    };
  }, []);
  return summary;
}

/** 프로젝트 홈 상태 띠 (§4.5). 한 줄: 에이전트별 ●/○ · Skill, 우측에
 *  마지막 기록 + [연결 관리]. 어느 것도 연결 안 됐으면 안내 문구를 덧붙인다
 *  (예전 ConnectCard 의 dismiss 키로 그 문구만 숨김 — 띠 자체는 항상). */
export function ConnectStatusBand({
  project,
  onOpenSheet,
}: {
  project: string;
  onOpenSheet: () => void;
}) {
  const tree = useApp((s) => s.tree);
  const { status, skills, connectedAgents } = useProjectConnection(project, [tree]);
  const dismissKey = `danbi:connect-card-dismissed:${project}`;
  const [hintDismissed, setHintDismissed] = useState(false);
  useEffect(() => {
    setHintDismissed(localStorage.getItem(dismissKey) === "1");
  }, [dismissKey]);

  const none = status !== null && connectedAgents.length === 0;
  const last = lastRecordText(status);

  return (
    <div className="rounded-lg border border-hairline bg-surface-elevated px-4 py-2.5">
      <div className="flex flex-wrap items-center justify-between gap-x-4 gap-y-1.5">
        <div className="flex min-w-0 flex-wrap items-center gap-x-4 gap-y-1 text-[12px]">
          {AGENTS.map((a) => {
            const state = agentState(status, a.key);
            const on = state === "connected";
            return (
              <span key={a.key} className="inline-flex items-center gap-1.5">
                <span
                  className={cn(
                    "h-1.5 w-1.5 rounded-full",
                    on ? "bg-accent-green" : "border border-stone bg-transparent",
                  )}
                />
                <span className={on ? "text-on-dark" : "text-mute"}>
                  {a.label} {STATE_LABEL[state]}
                </span>
                {a.skill && (
                  <span className="text-mute">· Skill {skills[a.key] ? "✓" : "○"}</span>
                )}
              </span>
            );
          })}
        </div>
        <div className="flex shrink-0 items-center gap-3">
          {last && <span className="text-caption-sm text-mute">{last}</span>}
          {status && !status.server_running && (
            <span className="text-caption-sm text-mute">MCP 서버 꺼짐</span>
          )}
          <button type="button" onClick={onOpenSheet} className={SECONDARY_BTN}>
            <Server size={12} /> 연결 관리
          </button>
        </div>
      </div>
      {none && !hintDismissed && (
        <div className="mt-2 flex items-center justify-between gap-3 border-t border-hairline pt-2 text-[12px] text-body">
          <span>
            아직 어느 에이전트도 연결되지 않았어요 · 코드 폴더를 고르면{" "}
            <code>.mcp.json</code> 에 바로 기록돼요.
          </span>
          <button
            type="button"
            onClick={() => {
              localStorage.setItem(dismissKey, "1");
              setHintDismissed(true);
            }}
            className="shrink-0 text-[11px] text-mute hover:text-on-dark"
          >
            숨기기
          </button>
        </div>
      )}
    </div>
  );
}

export function ConnectSheet({
  open,
  project,
  onClose,
  onOpenDoc,
}: {
  open: boolean;
  project: string | null;
  onClose: () => void;
  onOpenDoc?: (project: string, domain: string) => void;
}) {
  useEffect(() => {
    if (!open) return;
    function onKey(e: KeyboardEvent) {
      if (e.key === "Escape") {
        e.preventDefault();
        onClose();
      }
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [open, onClose]);

  if (!open) return null;
  return (
    <div
      className="fixed inset-0 z-50 flex items-start justify-center bg-canvas/70 pt-[10vh] backdrop-blur-sm"
      onMouseDown={(e) => {
        if (e.target === e.currentTarget) onClose();
      }}
    >
      <div className="flex max-h-[80vh] w-[560px] flex-col overflow-hidden rounded-lg border border-hairline-strong bg-surface-card">
        <SheetBody project={project} onClose={onClose} onOpenDoc={onOpenDoc} />
      </div>
    </div>
  );
}

function SheetBody({
  project,
  onClose,
  onOpenDoc,
}: {
  project: string | null;
  onClose: () => void;
  onOpenDoc?: (project: string, domain: string) => void;
}) {
  const [mcp, setMcp] = useState<McpStatus | null>(null);
  const [endpoint, setEndpoint] = useState<McpProjectEndpoint | null>(null);
  const [mcpBusy, setMcpBusy] = useState(false);
  const [err, setErr] = useState<string | null>(null);
  const [copied, setCopied] = useState<string | null>(null);
  const [flash, setFlash] = useState<Flash>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [hookOpen, setHookOpen] = useState(false);
  const [advancedOpen, setAdvancedOpen] = useState(false);
  const { status, skills, reload } = useProjectConnection(project);

  const loadMcp = useCallback(async () => {
    try {
      setMcp(await ipc.mcpStatus());
    } catch (e) {
      setErr(String(e));
    }
    if (project) {
      ipc.mcpProjectEndpoint(project).then(setEndpoint).catch(() => setEndpoint(null));
    }
  }, [project]);
  useEffect(() => {
    loadMcp();
  }, [loadMcp]);
  useEffect(() => {
    if (!flash) return;
    const t = setTimeout(() => setFlash(null), 6000);
    return () => clearTimeout(t);
  }, [flash]);

  async function copy(label: string, value: string) {
    try {
      await clipboardWriteText(value);
      setCopied(label);
      setTimeout(() => setCopied(null), 1200);
    } catch {
      /* ignore */
    }
  }

  async function toggleServer() {
    setMcpBusy(true);
    setErr(null);
    try {
      setMcp(mcp?.enabled ? await ipc.mcpDisable() : await ipc.mcpEnable());
    } catch (e) {
      setErr(String(e));
    } finally {
      setMcpBusy(false);
    }
  }

  async function connect(agent: AgentTarget) {
    if (!project) return;
    setBusy(`connect:${agent}`);
    setFlash(null);
    try {
      const st = mcp ?? (await ipc.mcpStatus());
      if (!st.enabled || !st.running) {
        setFlash({ kind: "err", msg: "MCP 서버가 꺼져 있어요. 위에서 먼저 켜주세요." });
        return;
      }
      const label = agentLabel(agent);
      const targetDir = await openDialog({
        directory: true,
        multiple: false,
        title: `${label} 가 실행될 코드 폴더 선택 · ${project}`,
      });
      if (typeof targetDir !== "string") return;
      if (agent === "codex") {
        const res = await ipc.codexConnect(project, targetDir);
        await ipc.installSkill(project, "codex").catch(() => {});
        setFlash({
          kind: "ok",
          msg:
            (res.action === "updated" ? "Codex 연결을 갱신했어요" : `${res.path} 에 연결했어요`) +
            (res.global_removed ? " · 기존 전역 연결 제거" : "") +
            " · SKILL 설치 완료 — Codex 에서 폴더를 신뢰하고 재시작하면 도구가 뜹니다.",
        });
      } else {
        const res = await ipc.agentConnect(project, agent, targetDir);
        if (agent === "claude") await ipc.installSkill(project, "claude").catch(() => {});
        setFlash({ kind: "ok", msg: describeConnect(label, res, agent === "claude") });
      }
      await reload();
    } catch (e) {
      setFlash({ kind: "err", msg: `연결 실패: ${e}` });
    } finally {
      setBusy(null);
    }
  }

  async function refreshSkill(agent: AgentTarget) {
    if (!project) return;
    setBusy(`skill:${agent}`);
    try {
      const was = skills[agent];
      await ipc.installSkill(project, agent);
      setFlash({ kind: "ok", msg: `${agentLabel(agent)} Skill ${was ? "갱신됨" : "설치됨"}` });
      await reload();
    } catch (e) {
      setFlash({ kind: "err", msg: `Skill 실패: ${e}` });
    } finally {
      setBusy(null);
    }
  }

  async function copyClaudeMd() {
    if (!project) return;
    try {
      await clipboardWriteText(buildClaudeMdBlock(project));
      setFlash({
        kind: "ok",
        msg: `"${project}" 용 CLAUDE.md 단비 블록을 복사했어요. 기존 CLAUDE.md 에 붙여넣거나, Claude Code 에 "CLAUDE.md 단비 블록 업데이트해줘" 라고 말하세요.`,
      });
    } catch (e) {
      setFlash({ kind: "err", msg: `복사 실패: ${e}` });
    }
  }

  const title = project ? `${project} — 에이전트 연결` : "에이전트 연결";
  const last = lastRecordText(status);

  return (
    <>
      <header className="flex h-11 shrink-0 items-center justify-between border-b border-hairline px-4">
        <div className="truncate text-[14px] font-medium text-ink">{title}</div>
        <button
          type="button"
          onClick={onClose}
          className="grid h-7 w-7 place-items-center rounded-sm text-mute hover:bg-surface-elevated hover:text-on-dark"
          title="닫기 (esc)"
        >
          <X size={14} />
        </button>
      </header>

      <div className="min-h-0 flex-1 overflow-y-auto px-4 py-3">
        {/* MCP 서버 줄 */}
        <div className="flex flex-wrap items-center gap-x-3 gap-y-1.5 text-[12px]">
          <span className="text-mute">MCP 서버</span>
          <span className="inline-flex items-center gap-1.5">
            <span
              className={cn(
                "h-1.5 w-1.5 rounded-full",
                mcp?.running ? "bg-accent-green" : "bg-stone",
              )}
            />
            <span className="text-on-dark">
              {mcp ? (mcp.running ? `실행 중 :${mcp.port}` : "꺼짐") : "…"}
            </span>
          </span>
          <button
            type="button"
            onClick={toggleServer}
            disabled={mcpBusy || !mcp}
            className="text-[11px] text-mute hover:text-on-dark disabled:opacity-50"
          >
            {mcp?.enabled ? "끄기" : "켜기"}
          </button>
          {mcp?.enabled && (
            <>
              <span className="text-stone">·</span>
              <span className="text-mute">토큰</span>
              <code className="font-mono text-[11px] text-on-dark-mute">••••{mcp.token.slice(-4)}</code>
              <button
                type="button"
                onClick={() => copy("token", mcp.token)}
                className={ICON_BTN}
                title="토큰 복사"
              >
                {copied === "token" ? <Check size={11} /> : <Copy size={11} />}
              </button>
              <RotateControl onRotated={(res) => setMcp(res.status)} />
            </>
          )}
        </div>
        {err && <div className="mt-1 text-[11px] text-accent-red">{err}</div>}

        {!project ? (
          <div className="mt-4 text-[12px] leading-[1.6] text-mute">
            프로젝트를 고르면 Claude Code · Codex · Cursor 연결을 여기서 관리할 수 있어요.
            사이드바에서 프로젝트를 선택하세요.
          </div>
        ) : (
          AGENTS.map((a) => {
            const files = (status?.connections ?? []).filter(
              (c) => c.agent === a.key && c.exists,
            );
            const state = agentState(status, a.key);
            const on = state === "connected";
            return (
              <section key={a.key} className="mt-4 border-t border-hairline pt-3">
                <div className="flex items-center gap-1.5 text-[11px] font-medium uppercase tracking-[0.4px] text-mute">
                  {a.icon} {a.label}
                </div>
                <div className="mt-1.5 flex flex-wrap items-center gap-x-3 gap-y-1 text-[12px]">
                  <span className="inline-flex items-center gap-1.5">
                    <span
                      className={cn(
                        "h-1.5 w-1.5 rounded-full",
                        on ? "bg-accent-green" : "border border-stone",
                      )}
                    />
                    <span className={on ? "text-on-dark" : "text-mute"}>
                      {STATE_LABEL[state]}
                    </span>
                  </span>
                  {files.map((f) => (
                    <code
                      key={f.path}
                      className="truncate font-mono text-[11px] text-on-dark-mute"
                      title={f.path}
                    >
                      ({shortHome(f.path)})
                    </code>
                  ))}
                  {on && last && <span className="text-mute">{last}</span>}
                </div>
                {a.skill && (
                  <div className="mt-1 text-[12px] text-mute">
                    Skill {skills[a.key] ? "✓" : "○"}
                    {a.key === "claude" && " · 훅은 아래 [훅 설정…] 에서"}
                  </div>
                )}
                <div className="mt-2 flex flex-wrap items-center gap-1.5">
                  <button
                    type="button"
                    disabled={busy !== null}
                    onClick={() => connect(a.key)}
                    className={SECONDARY_BTN}
                  >
                    {busy === `connect:${a.key}` ? (
                      <Loader2 size={12} className="animate-spin" />
                    ) : (
                      <Server size={12} />
                    )}
                    폴더에 연결…
                  </button>
                  {a.skill && (
                    <button
                      type="button"
                      disabled={busy !== null}
                      onClick={() => refreshSkill(a.key)}
                      className={SECONDARY_BTN}
                    >
                      <Sparkles size={12} /> {skills[a.key] ? "Skill 갱신" : "Skill 설치"}
                      {skills[a.key] && <Check size={12} className="text-accent-green" />}
                    </button>
                  )}
                  {a.key === "claude" && (
                    <>
                      <button
                        type="button"
                        onClick={() => setHookOpen((v) => !v)}
                        className={cn(
                          SECONDARY_BTN,
                          hookOpen && "border-hairline-strong text-on-dark",
                        )}
                      >
                        훅 설정…
                      </button>
                      <button type="button" onClick={copyClaudeMd} className={SECONDARY_BTN}>
                        <Copy size={12} /> CLAUDE.md 블록 복사
                      </button>
                      {onOpenDoc && (
                        <button
                          type="button"
                          onClick={async () => {
                            try {
                              await ipc.installSkill(project);
                            } catch {
                              /* 빈 에디터라도 열어준다 */
                            }
                            onOpenDoc(project, "SKILL.md");
                            onClose();
                          }}
                          className="text-[11px] text-mute hover:text-on-dark"
                          title="Codex · Claude Code 공용 SKILL.md 원본 편집"
                        >
                          Skill 수정
                        </button>
                      )}
                    </>
                  )}
                </div>
                {a.key === "claude" && hookOpen && <HookPanel project={project} />}
              </section>
            );
          })
        )}

        {flash && (
          <div
            className={cn(
              "mt-3 break-all text-[12px] leading-[1.6]",
              flash.kind === "err" ? "text-accent-red" : "text-accent-green",
            )}
          >
            {flash.kind === "err" ? "✕" : "✓"} {flash.msg}
          </div>
        )}

        {/* 고급 — 명령 복사·JSON. 설정 MCP 섹션에서 옮겨 옴. */}
        <section className="mt-4 border-t border-hairline pt-3">
          <button
            type="button"
            onClick={() => setAdvancedOpen((v) => !v)}
            className="flex items-center gap-1 text-[11px] font-medium uppercase tracking-[0.4px] text-mute hover:text-on-dark"
          >
            {advancedOpen ? <ChevronDown size={12} /> : <ChevronRight size={12} />} 고급
          </button>
          {advancedOpen && mcp && (
            <AdvancedCommands mcp={mcp} endpoint={endpoint} copied={copied} onCopy={copy} />
          )}
        </section>

        <AllSkillsButton />
      </div>
    </>
  );
}

function shortHome(p: string): string {
  const m = p.match(/^\/Users\/[^/]+\/(.*)$/);
  return m ? `~/${m[1]}` : p;
}

/** 토큰 회전 — 인라인 확인 (webview confirm 은 못 믿는다) + 결과. */
function RotateControl({ onRotated }: { onRotated: (res: McpRotateResult) => void }) {
  const [confirm, setConfirm] = useState<number | null>(null);
  const [result, setResult] = useState<McpRotateResult | null>(null);
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState<string | null>(null);

  async function ask() {
    setErr(null);
    setResult(null);
    try {
      setConfirm((await ipc.mcpConnectionsList()).length);
    } catch {
      setConfirm(0);
    }
  }
  async function run() {
    setBusy(true);
    setConfirm(null);
    try {
      const res = await ipc.mcpRotateToken();
      setResult(res);
      onRotated(res);
    } catch (e) {
      setErr(String(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <>
      <button
        type="button"
        onClick={ask}
        disabled={busy || confirm !== null}
        className={ICON_BTN}
        title="새 토큰 생성"
      >
        <RefreshCw size={11} className={busy ? "animate-spin" : ""} />
      </button>
      {(confirm !== null || result || err) && (
        <div className="basis-full rounded-md border border-hairline bg-surface-elevated p-3 text-caption-sm leading-[1.6]">
          {confirm !== null && (
            <>
              <div className="text-[13px] font-medium text-on-dark">토큰을 새로 만들까요?</div>
              <div className="mt-1 text-mute">
                지금 토큰으로 붙어 있는 에이전트는 끊겨요. 단비가 기록한 연결 파일 {confirm}개
                (.mcp.json · .cursor/mcp.json · .codex/config.toml)는 자동으로 갱신되고,
                터미널에서 직접 <code>claude mcp add</code> 한 등록은 새 토큰으로 다시
                등록해야 해요.
              </div>
              <div className="mt-2 flex items-center gap-2">
                <button type="button" onClick={run} className={SECONDARY_BTN}>
                  새 토큰 만들기
                </button>
                <button
                  type="button"
                  onClick={() => setConfirm(null)}
                  className="text-[12px] text-mute hover:text-on-dark"
                >
                  취소
                </button>
              </div>
            </>
          )}
          {result && (
            <>
              <div className="text-on-dark">
                새 토큰으로 바꿨어요 · 연결 파일 {result.updated.length}개 갱신
                {result.removed.length > 0 && ` · 사라진 파일 ${result.removed.length}개 정리`}
              </div>
              {result.updated.map((p) => (
                <div key={p} className="truncate font-mono text-[11px] text-mute">
                  {p}
                </div>
              ))}
              {result.failed.map((p) => (
                <div key={p} className="font-mono text-[11px] text-accent-red">
                  갱신 실패: {p}
                </div>
              ))}
              <div className="mt-1 text-mute">에이전트를 재시작하면 새 토큰으로 붙어요.</div>
            </>
          )}
          {err && <div className="text-accent-red">{err}</div>}
        </div>
      )}
    </>
  );
}

/** Claude Code 세션 훅 — ProjectHome 의 ClaudeHookPanel 을 그대로. */
function HookPanel({ project }: { project: string }) {
  const [busy, setBusy] = useState(false);
  const [sessionEnd, setSessionEnd] = useState(false);
  const [flash, setFlash] = useState<Flash>(null);

  async function install() {
    const targetDir = await openDialog({
      directory: true,
      multiple: false,
      title: `Claude Code 를 실행할 코드 폴더 선택 · ${project}`,
    });
    if (typeof targetDir !== "string") return;
    setBusy(true);
    setFlash(null);
    try {
      const res = await ipc.claudeHooksInstall(project, targetDir, sessionEnd);
      setFlash({
        kind: "ok",
        msg: `${res.action === "updated" ? "훅을 갱신했어요" : "훅을 설치했어요"}${res.session_end ? " (종료 기록 포함)" : ""} · ${res.path} — 다음 세션부터 적용돼요.`,
      });
    } catch (e) {
      setFlash({ kind: "err", msg: `설치 실패: ${e}` });
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="mt-2 rounded-md border border-hairline bg-surface-elevated p-3">
      <div className="text-[13px] font-medium text-on-dark">Claude Code 세션 훅</div>
      <p className="mt-1 text-[12px] leading-[1.6] text-body">
        Claude Code 세션을 열 때마다 <span className="text-on-dark">{project}</span> 의 최근
        문서·daily 노트·할 일을 짧게 넣어줘요. 고른 폴더의{" "}
        <code>.claude/settings.local.json</code> 에만 기록하고 Git 에서는 빼둡니다. 토큰은
        파일에 남기지 않아요.
      </p>
      <div className="mt-2 flex items-center justify-between gap-3">
        <label
          className="flex items-center gap-2 text-[12px] text-body"
          title="세션이 끝나면 대화를 AI 로 요약해 오늘 daily 노트에 남겨요. 세션 중에 이미 단비에 기록했거나 아주 짧은 세션은 건너뛰어요. AI 연동이 꺼져 있으면 아무것도 하지 않아요."
        >
          <input
            type="checkbox"
            checked={sessionEnd}
            onChange={(e) => setSessionEnd(e.target.checked)}
          />
          세션이 끝나면 자동으로 기록
        </label>
        <button
          type="button"
          disabled={busy}
          onClick={install}
          className={cn(SECONDARY_BTN, "bg-surface-card")}
        >
          {busy ? <Loader2 size={12} className="animate-spin" /> : <Server size={12} />}
          폴더 선택 후 설치
        </button>
      </div>
      {flash && (
        <div
          className={cn(
            "mt-2 break-all text-[12px]",
            flash.kind === "err" ? "text-accent-red" : "text-accent-green",
          )}
        >
          {flash.kind === "err" ? "✕" : "✓"} {flash.msg}
        </div>
      )}
    </div>
  );
}

/** 읽기(user)·쓰기(local) 명령과 JSON — 설정 MCP 에 있던 것. 쓰기 명령은
 *  이 프로젝트 엔드포인트로, 프로젝트가 없으면 읽기만. */
function AdvancedCommands({
  mcp,
  endpoint,
  copied,
  onCopy,
}: {
  mcp: McpStatus;
  endpoint: McpProjectEndpoint | null;
  copied: string | null;
  onCopy: (label: string, value: string) => void;
}) {
  const readCommand = `claude mcp add --transport http -s user danbi_read "${mcp.url}/read" --header "Authorization: Bearer ${mcp.token}"`;
  const writeCommand = endpoint
    ? `claude mcp add --transport http -s local danbi "${endpoint.url}/write" --header "Authorization: Bearer ${mcp.token}"`
    : "";
  const snippet = JSON.stringify(
    {
      mcpServers: {
        danbi_read: {
          type: "http",
          url: `${mcp.url}/read`,
          headers: { Authorization: `Bearer ${mcp.token}` },
        },
        ...(endpoint
          ? {
              danbi: {
                type: "http",
                url: `${endpoint.url}/write`,
                headers: { Authorization: `Bearer ${mcp.token}` },
              },
            }
          : {}),
      },
    },
    null,
    2,
  );
  const rows: { key: string; label: string; value: string }[] = [
    { key: "read", label: "읽기 서버 (user, 한 번만)", value: readCommand },
    ...(writeCommand
      ? [{ key: "write", label: "쓰기 서버 (local, 코드 폴더에서)", value: writeCommand }]
      : []),
    { key: "json", label: "JSON (mcp.json 에 직접 붙여넣기)", value: snippet },
  ];
  return (
    <div className="mt-2 flex flex-col gap-2">
      <div className="text-caption-sm leading-[1.6] text-mute">
        [폴더에 연결…] 이 이 두 서버를 코드 폴더에 바로 기록해요. 직접 등록하려면 아래 명령을
        쓰세요.
      </div>
      {rows.map((r) => (
        <div key={r.key}>
          <div className="mb-1 flex items-center justify-between">
            <span className="text-caption-sm uppercase tracking-[0.4px] text-mute">{r.label}</span>
            <button
              type="button"
              onClick={() => onCopy(r.key, r.value)}
              className="inline-flex h-6 items-center gap-1 rounded-sm border border-hairline px-2 text-[11px] text-body hover:border-hairline-strong hover:text-on-dark"
            >
              {copied === r.key ? <Check size={10} /> : <Copy size={10} />} 복사
            </button>
          </div>
          <pre className="whitespace-pre-wrap break-all rounded-md border border-hairline bg-surface p-2.5 font-mono text-[11px] leading-[1.6] text-body">
            {r.value}
          </pre>
        </div>
      ))}
    </div>
  );
}

/** 홈 SkillSetupCard 의 "전체 설치" 를 버튼 하나로. */
function AllSkillsButton() {
  // 선택자는 store 의 값을 그대로 돌려줘야 한다 — 매번 새 배열을 만들면
  // zustand v5 가 변경으로 보고 무한 렌더(React #185)에 빠진다.
  const tree = useApp((s) => s.tree);
  const projects = useMemo(() => tree?.projects.map((p) => p.name) ?? [], [tree]);
  const [progress, setProgress] = useState<{ done: number; total: number } | null>(null);
  const [flash, setFlash] = useState<Flash>(null);

  async function run() {
    setFlash(null);
    setProgress({ done: 0, total: projects.length });
    let failed = 0;
    for (let i = 0; i < projects.length; i++) {
      const p = projects[i];
      await ipc.installSkill(p, "claude").catch(() => {
        failed += 1;
      });
      await ipc.installSkill(p, "codex").catch(() => {
        failed += 1;
      });
      setProgress({ done: i + 1, total: projects.length });
    }
    setProgress(null);
    setFlash(
      failed > 0
        ? { kind: "err", msg: `${projects.length}개 중 ${failed}건 실패` }
        : { kind: "ok", msg: `${projects.length}개 프로젝트 Skill 갱신 완료 (Claude Code · Codex)` },
    );
  }

  if (projects.length === 0) return null;
  return (
    <div className="mt-4 flex flex-wrap items-center gap-3 border-t border-hairline pt-3">
      <button type="button" onClick={run} disabled={progress !== null} className={SECONDARY_BTN}>
        {progress ? <Loader2 size={12} className="animate-spin" /> : <Sparkles size={12} />}
        모든 프로젝트 Skill 갱신
        {progress && (
          <span className="text-mute">
            {progress.done}/{progress.total}
          </span>
        )}
      </button>
      {flash && (
        <span
          className={cn(
            "text-[12px]",
            flash.kind === "err" ? "text-accent-red" : "text-accent-green",
          )}
        >
          {flash.kind === "err" ? "✕" : "✓"} {flash.msg}
        </span>
      )}
    </div>
  );
}
