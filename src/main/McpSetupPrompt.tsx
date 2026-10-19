import { useEffect, useState } from "react";
import { Server, X } from "lucide-react";
import { PrimaryButton, SecondaryButton } from "@/components/WizardShell";
import { ipc } from "@/lib/ipc";
import { useApp } from "@/state/store";

/**
 * 앱을 열었을 때 MCP 가 꺼져 있거나, 켜져 있어도 아직 어느 에이전트에도
 * 연결된 적이 없으면 한 번 띄우는 안내. 단비는 Claude Code · Codex 가
 * vault 를 기억으로 쓸 때 가치가 생기므로, 첫 설치 사용자가 이 단계를
 * 놓치지 않게 한다. 실제 연결은 기존 연결 시트에 맡긴다.
 *
 * - "나중에" → 이번 실행 동안만 숨김
 * - "다시 보지 않기" → localStorage 에 영구 기록
 */
const DISMISS_KEY = "danbi:mcp-setup-prompt-dismissed";

type Phase = "off" | "connect";

export function McpSetupPrompt() {
  const tree = useApp((s) => s.tree);
  const selection = useApp((s) => s.selection);
  const openConnectSheet = useApp((s) => s.openConnectSheet);
  const [phase, setPhase] = useState<Phase | null>(null);
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState<string | null>(null);

  const hasProjects = (tree?.projects.length ?? 0) > 0;

  useEffect(() => {
    if (!hasProjects) return;
    try {
      if (localStorage.getItem(DISMISS_KEY) === "1") return;
    } catch {
      /* storage 막힘 — 그냥 판단 진행 */
    }
    let alive = true;
    (async () => {
      const [st, conns] = await Promise.all([
        ipc.mcpStatus().catch(() => null),
        ipc.mcpConnectionsList().catch(() => []),
      ]);
      if (!alive || !st) return;
      if (!st.running) setPhase("off");
      else if (conns.length === 0) setPhase("connect");
    })();
    return () => {
      alive = false;
    };
    // 실행당 한 번만 판단 — 프로젝트가 처음 보이는 시점.
  }, [hasProjects]);

  useEffect(() => {
    if (!phase) return;
    function onKey(e: KeyboardEvent) {
      if (e.key === "Escape") {
        e.preventDefault();
        setPhase(null);
      }
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [phase]);

  if (!phase) return null;

  const close = () => setPhase(null);
  const neverAgain = () => {
    try {
      localStorage.setItem(DISMISS_KEY, "1");
    } catch {
      /* no-op */
    }
    close();
  };

  async function enable() {
    setBusy(true);
    setErr(null);
    try {
      await ipc.mcpEnable();
      const conns = await ipc.mcpConnectionsList().catch(() => []);
      if (conns.length === 0) setPhase("connect");
      else close();
    } catch (e) {
      setErr(String(e).slice(0, 200));
    } finally {
      setBusy(false);
    }
  }

  function connect() {
    const project = selection.project ?? tree?.projects[0]?.name ?? null;
    close();
    openConnectSheet(project);
  }

  const off = phase === "off";

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-canvas/70 backdrop-blur-sm"
      onMouseDown={(e) => {
        if (e.target === e.currentTarget) close();
      }}
    >
      <div className="relative w-[440px] rounded-lg border border-hairline-strong bg-surface-card p-6">
        <button
          type="button"
          onClick={close}
          title="닫기 (Esc)"
          className="absolute right-3 top-3 grid h-6 w-6 place-items-center rounded-sm text-mute hover:bg-surface-elevated hover:text-on-dark"
        >
          <X size={12} />
        </button>

        <div className="mb-3 grid h-9 w-9 place-items-center rounded-md border border-hairline bg-surface-elevated text-on-dark">
          <Server size={16} />
        </div>
        <h2 className="text-[17px] font-semibold text-ink">
          {off ? "MCP 서버가 꺼져 있어요" : "Claude Code 에 단비를 연결하세요"}
        </h2>
        <p className="mt-2 text-[13px] leading-[1.65] text-body">
          {off
            ? "Claude Code · Codex · Cursor 가 vault 를 읽고 쓰려면 MCP 서버가 켜져 있어야 해요."
            : "코드 폴더를 고르면 그 폴더의 .mcp.json 에 연결 정보가 기록돼요. 다음 세션부터 에이전트가 단비 vault 를 기억으로 씁니다."}
        </p>

        <ul className="mt-3 flex flex-col gap-1 text-[12px] leading-[1.6] text-mute">
          <li>· 서버는 이 Mac 안(127.0.0.1)에서만 열려요. 다른 기기에서는 접속할 수 없어요.</li>
          <li>· 접속할 때마다 단비가 만든 토큰을 확인해요.</li>
          <li>· 언제든 연결 관리에서 끄거나 연결을 지울 수 있어요.</li>
        </ul>

        {err && <div className="mt-3 text-[12px] text-accent-red">{err}</div>}

        <div className="mt-5 flex items-center justify-between gap-2">
          <button
            type="button"
            onClick={neverAgain}
            className="text-[12px] text-mute hover:text-on-dark"
          >
            다시 보지 않기
          </button>
          <div className="flex items-center gap-2">
            <SecondaryButton onClick={close}>나중에</SecondaryButton>
            {off ? (
              <PrimaryButton onClick={enable} disabled={busy}>
                {busy ? "켜는 중…" : "MCP 켜기"}
              </PrimaryButton>
            ) : (
              <PrimaryButton onClick={connect}>연결하기</PrimaryButton>
            )}
          </div>
        </div>
      </div>
    </div>
  );
}
