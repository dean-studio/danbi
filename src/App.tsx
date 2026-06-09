import { useEffect, useRef, useState } from "react";
import { Onboarding } from "@/wizard/Onboarding";
import { Workspace } from "@/main/Workspace";
import { Wordmark } from "@/components/Wordmark";
import { ipc } from "@/lib/ipc";
import { useApp } from "@/state/store";
import { installTheme, type ThemeChoice } from "@/lib/theme";
import { runUpdateCheck } from "@/lib/updater";

type AppState = "loading" | "onboarding" | "ready";

/** Read once at module init so the very first render of <App/> already
 *  knows the user wants onboarding — avoids a brief flash of Workspace
 *  while the async useEffect resolves cfg. */
function readForceOnboardingFlag(): boolean {
  if (typeof window === "undefined") return false;
  const hash = window.location.hash === "#onboarding";
  const ls = window.localStorage.getItem("danbi.forceOnboarding") === "1";
  return hash || ls;
}

export default function App() {
  // Initialize state synchronously based on the flag — the useEffect
  // below only takes effect AFTER the first paint, so without this
  // guard the user briefly sees Workspace flicker before getting
  // dropped into Onboarding.
  const [state, setState] = useState<AppState>(() =>
    readForceOnboardingFlag() ? "onboarding" : "loading",
  );
  const setCfg = useApp((s) => s.setCfg);
  const themeChoice = useApp((s) =>
    (s.cfg?.appearance.theme as ThemeChoice | undefined) ?? "dark",
  );
  // StrictMode runs effects twice in dev. We only want to consume
  // (clear) the force-onboarding flag once — the second run sees an
  // empty flag and would otherwise demote us back to "ready".
  const didConsumeOnce = useRef(false);

  useEffect(() => {
    if (didConsumeOnce.current) return;
    didConsumeOnce.current = true;
    (async () => {
      const hasHash =
        typeof window !== "undefined" &&
        window.location.hash === "#onboarding";
      const hasFlag =
        typeof window !== "undefined" &&
        window.localStorage.getItem("danbi.forceOnboarding") === "1";
      const forceOnboarding = hasHash || hasFlag;
      console.log(
        "[danbi] App load — forceOnboarding=",
        forceOnboarding,
        "(hash:",
        hasHash,
        ", localStorage:",
        hasFlag,
        ")",
      );
      if (hasFlag) {
        window.localStorage.removeItem("danbi.forceOnboarding");
      }
      if (hasHash) {
        history.replaceState(
          null,
          "",
          window.location.pathname + window.location.search,
        );
      }
      try {
        const cfg = await ipc.loadConfig();
        if (cfg) setCfg(cfg);
        if (forceOnboarding) {
          // State already set to "onboarding" synchronously at mount
          // by readForceOnboardingFlag(). Don't overwrite.
          setState("onboarding");
          return;
        }
        if (cfg && cfg.vault_path) {
          setState("ready");
        } else {
          setState("onboarding");
        }
      } catch {
        setState("onboarding");
      }
    })();
  }, [setCfg]);

  useEffect(() => installTheme(themeChoice), [themeChoice]);

  // Vault 가 열린 뒤 3초 정도 idle 한 다음 GitHub Releases 에 업데이트
  // 체크. 첫 paint·tree load 등이 끝난 시점에서 한 번만 — 24h 스로틀은
  // runUpdateCheck 내부에서 처리하므로 dev 재시작에서도 부담 없음.
  useEffect(() => {
    if (state !== "ready") return;
    const t = window.setTimeout(() => {
      void runUpdateCheck(false);
    }, 3_000);
    return () => window.clearTimeout(t);
  }, [state]);

  return (
    <div className="h-full w-full bg-canvas">
      {state === "loading" && <LoadingScreen />}
      {state === "onboarding" && (
        <OnboardingShell>
          <Onboarding
            onDone={(cfg) => {
              setCfg(cfg);
              setState("ready");
            }}
          />
        </OnboardingShell>
      )}
      {state === "ready" && <Workspace />}
    </div>
  );
}

/**
 * Splash shown during the first ipc.loadConfig() round-trip. A single
 * droplet falls into a still pool, ripples spread, and a faint wordmark
 * fades in. Kept under ~120 lines of inline CSS so it can render
 * before any other JS chunk has parsed.
 *
 * Exported so Workspace can keep showing the same splash while its
 * heavy first IPCs (listTree, recentCommits, daily) resolve — without
 * this the user sees Workspace flash empty cards for ~300ms and macOS
 * paints the busy beach-ball cursor over the dead time.
 */
export function LoadingScreen() {
  // Cycling tagline phrases. The fade-in/out happens via key change
  // re-mounting the span, which restarts the CSS animation cleanly.
  const phrases = [
    "메모를 지식으로",
    "흩어진 생각을 한 곳으로",
    "단비가 깨어나는 중",
    "vault 를 살펴보는 중",
    "AI 와 같이 쓰는 노트",
  ];
  const [phraseIndex, setPhraseIndex] = useState(0);
  useEffect(() => {
    const id = window.setInterval(() => {
      setPhraseIndex((i) => (i + 1) % phrases.length);
    }, 2400);
    return () => window.clearInterval(id);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  return (
    <main
      data-tauri-drag-region
      className="relative flex h-full w-full items-center justify-center overflow-hidden bg-canvas"
      style={{ cursor: "default" }}
    >
      <style>{`
        @keyframes danbi-splash-in {
          from { opacity: 0; transform: translateY(10px); }
          to   { opacity: 1; transform: translateY(0); }
        }
        @keyframes danbi-typo-cycle {
          0%   { opacity: 0; transform: translateY(6px); }
          15%  { opacity: 1; transform: translateY(0); }
          80%  { opacity: 1; transform: translateY(0); }
          100% { opacity: 0; transform: translateY(-6px); }
        }
        .danbi-splash-wordmark {
          animation: danbi-splash-in 0.7s ease-out both;
        }
        .danbi-splash-tagline-wrap {
          animation: danbi-splash-in 0.7s ease-out 0.2s both;
          font-size: 15px;
          letter-spacing: 0.4px;
          color: var(--color-mute);
          height: 22px;
          line-height: 22px;
        }
        .danbi-splash-typo {
          display: inline-block;
          animation: danbi-typo-cycle 2.4s ease-in-out both;
        }
      `}</style>
      <div className="flex flex-col items-center gap-5">
        <div className="danbi-splash-wordmark">
          <Wordmark className="h-16 w-auto" />
        </div>
        <div className="danbi-splash-tagline-wrap">
          <span key={phraseIndex} className="danbi-splash-typo">
            {phrases[phraseIndex]}
          </span>
        </div>
      </div>
    </main>
  );
}

/**
 * Onboarding still needs the system titlebar area for dragging since its
 * content is a centered card, not a chrome-hugging layout.
 */
function OnboardingShell({ children }: { children: React.ReactNode }) {
  return (
    <div className="flex h-full w-full flex-col">
      <div data-tauri-drag-region className="h-7 w-full shrink-0" />
      <div className="flex-1 min-h-0">{children}</div>
    </div>
  );
}
