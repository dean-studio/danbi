import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Check, ChevronRight, FileText, Plus, Search, X } from "lucide-react";
import {
  ipc,
  type CaptureContext,
  type SearchHit,
} from "@/lib/ipc";
import { cn } from "@/lib/utils";
import { installTheme, type ThemeChoice } from "@/lib/theme";

type Status =
  | { kind: "idle" }
  | { kind: "ok"; target: string; summary: string }
  | { kind: "error"; message: string };

type Mode = "input" | "search" | "pick-project" | "pick-domain";

const HEIGHT_COLLAPSED = 176;
const HEIGHT_SEARCH = 480;
const HEIGHT_PREVIEW = 620;

/**
 * Quick Capture popup — Gemini-inspired pill with two main modes:
 *  • input  : text → append to project/domain (LLM-uninvolved, see backend
 *             capture_append_no_llm).
 *  • search : query the BM25 (+ optional RRF) hybrid index. Click a hit to
 *             expand its preview snippet inside the popup; click again to
 *             collapse. Tab toggles between the two main modes.
 *
 * Picker modes (pick-project / pick-domain) are sub-states of input — they
 * never appear from search. Esc returns to whichever main mode the user
 * was in.
 */
export function CaptureApp() {
  const [ctx, setCtx] = useState<CaptureContext | null>(null);
  const [input, setInput] = useState("");
  const [project, setProject] = useState<string | null>(null);
  const [domain, setDomain] = useState<string | null>(null);
  const [status, setStatus] = useState<Status>({ kind: "idle" });
  const [mode, setMode] = useState<Mode>("input");
  // Remember which main mode (input vs search) is active so picker → main
  // returns to the right place. Sub-modes never overwrite this.
  const [mainMode, setMainMode] = useState<"input" | "search">("input");
  const inputRef = useRef<HTMLInputElement | null>(null);

  // ---- search state ----
  const [searchHits, setSearchHits] = useState<SearchHit[]>([]);
  const [searchLoading, setSearchLoading] = useState(false);
  const [expandedKey, setExpandedKey] = useState<string | null>(null);
  const searchAbort = useRef<{ q: string } | null>(null);

  const refreshCtx = useCallback(async () => {
    try {
      const c = await ipc.captureContext();
      setCtx(c);
      setProject((cur) => cur ?? c.last_project);
      setDomain((cur) => cur ?? c.last_domain);
    } catch (e) {
      console.error("[danbi] captureContext failed:", e);
    }
  }, []);

  useEffect(() => {
    refreshCtx();
    inputRef.current?.focus();
    (async () => {
      try {
        const cfg = await ipc.loadConfig();
        const choice = (cfg?.appearance.theme as ThemeChoice | undefined) ?? "dark";
        installTheme(choice);
      } catch {
        /* ignore */
      }
      try {
        const { isPermissionGranted, requestPermission } = await import(
          "@tauri-apps/plugin-notification"
        );
        if (!(await isPermissionGranted())) {
          await requestPermission();
        }
      } catch {
        /* ignore */
      }
    })();
  }, [refreshCtx]);

  useEffect(() => {
    function onFocus() {
      refreshCtx();
      if (mode === "input" || mode === "search") inputRef.current?.focus();
    }
    window.addEventListener("focus", onFocus);
    return () => window.removeEventListener("focus", onFocus);
  }, [refreshCtx, mode]);

  // popup height — drives the resize_capture IPC. Bigger when search has
  // hits (or a hit is expanded for preview), collapsed otherwise.
  useEffect(() => {
    let target = HEIGHT_COLLAPSED;
    if (mode === "search") {
      if (expandedKey) target = HEIGHT_PREVIEW;
      else if (searchHits.length > 0 || searchLoading) target = HEIGHT_SEARCH;
    }
    ipc.resizeCapture(target).catch(() => {});
  }, [mode, searchHits.length, searchLoading, expandedKey]);

  // Esc handling — search mode collapses results before closing; pickers
  // bubble back to the main mode they came from.
  useEffect(() => {
    function onKey(e: KeyboardEvent) {
      if (e.key === "Escape") {
        if (mode === "pick-project" || mode === "pick-domain") {
          setMode(mainMode);
          setTimeout(() => inputRef.current?.focus(), 0);
          return;
        }
        if (mode === "search" && expandedKey) {
          setExpandedKey(null);
          return;
        }
        if (mode === "search" && (input.trim().length > 0 || searchHits.length > 0)) {
          setInput("");
          setSearchHits([]);
          return;
        }
        ipc.hideCapture().catch(() => {});
      }
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [mode, mainMode, expandedKey, input, searchHits.length]);

  // Tab toggles between input ↔ search. Pickers ignore — they have their
  // own ChipRow keyboard semantics.
  useEffect(() => {
    function onKey(e: KeyboardEvent) {
      if (e.key !== "Tab") return;
      if (mode === "pick-project" || mode === "pick-domain") return;
      // Ignore Tab when the user is mid-suggestion-acceptance (handled in
      // the input's onKeyDown — that path also calls preventDefault).
      if (mode === "input" && suggest && !project) return;
      e.preventDefault();
      const next: "input" | "search" = mode === "search" ? "input" : "search";
      setMainMode(next);
      setMode(next);
      setExpandedKey(null);
      setStatus({ kind: "idle" });
      setTimeout(() => inputRef.current?.focus(), 0);
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [mode, project, input]);

  // ---- live search query ----
  // Run on every input change while mode === "search". Tiny 120ms debounce
  // keeps us from spamming the (now async, possibly hybrid) IPC on each
  // keystroke. Out-of-order responses are dropped via the q-stamp ref.
  useEffect(() => {
    if (mode !== "search") {
      setSearchHits([]);
      setSearchLoading(false);
      return;
    }
    const q = input.trim();
    if (!q) {
      setSearchHits([]);
      setSearchLoading(false);
      return;
    }
    setSearchLoading(true);
    searchAbort.current = { q };
    const handle = window.setTimeout(async () => {
      try {
        const hits = await ipc.searchFull(q, 12);
        if (searchAbort.current?.q !== q) return; // stale
        setSearchHits(hits);
      } catch (e) {
        console.error("[danbi] search failed", e);
        if (searchAbort.current?.q === q) setSearchHits([]);
      } finally {
        if (searchAbort.current?.q === q) setSearchLoading(false);
      }
    }, 120);
    return () => window.clearTimeout(handle);
  }, [input, mode]);

  const suggest = useMemo(() => {
    if (mode !== "input") return null;
    if (!ctx || project) return null;
    const trimmed = input.trim();
    if (trimmed.length < 2) return null;
    const first = trimmed.split(/\s+/)[0]?.replace(/[^\p{L}\p{N}_-]/gu, "");
    const last = trimmed
      .split(/\s+/)
      .slice(-1)[0]
      ?.replace(/[^\p{L}\p{N}_-]/gu, "");
    const candidates = [first, last].filter(Boolean) as string[];
    for (const token of candidates) {
      const hit = ctx.projects.find(
        (p) => p.toLowerCase() === token.toLowerCase(),
      );
      if (hit) return { project: hit, matchedToken: token };
      const prefix = ctx.projects.find((p) =>
        p.toLowerCase().startsWith(token.toLowerCase()),
      );
      if (prefix && token.length >= 2) {
        return { project: prefix, matchedToken: token };
      }
    }
    return null;
  }, [input, ctx, project, mode]);

  function acceptSuggestion() {
    if (!suggest) return;
    setProject(suggest.project);
    const t = suggest.matchedToken;
    const re = new RegExp(`(^|\\s)${escapeRegex(t)}(\\s|$)`, "i");
    setInput((prev) => prev.replace(re, " ").replace(/\s+/g, " ").trim());
  }

  async function submit() {
    const trimmed = input.trim();
    if (!trimmed) return;

    if (!project) {
      setMode("pick-project");
      setStatus({
        kind: "error",
        message: "프로젝트를 먼저 선택해주세요",
      });
      return;
    }

    const stickyProject = project;
    const stickyDomain = domain ?? undefined;
    setInput("");
    setStatus({ kind: "idle" });
    ipc.hideCapture().catch(() => {});

    ipc
      .quickCapture(trimmed, stickyProject, stickyDomain)
      .then(async (res) => {
        if (res.status === "stored") {
          setProject(res.project);
          setDomain(res.domain);
        } else {
          if (res.project) setProject(res.project);
          await ipc.toggleCapture().catch(() => {});
          setStatus({
            kind: "error",
            message:
              res.clarification_type === "project"
                ? "프로젝트를 먼저 선택해주세요"
                : "도메인 파일이 없어요",
          });
        }
      })
      .catch(async (e) => {
        await ipc.toggleCapture().catch(() => {});
        setStatus({ kind: "error", message: String(e) });
        try {
          const { sendNotification, isPermissionGranted, requestPermission } =
            await import("@tauri-apps/plugin-notification");
          if (!(await isPermissionGranted())) {
            await requestPermission();
          }
          sendNotification({
            title: "단비 · 저장 실패",
            body: String(e).slice(0, 240),
          });
        } catch {
          /* ignore */
        }
      });
  }

  const domainOptions = useMemo(
    () => (project && ctx ? ctx.domains[project] ?? [] : []),
    [project, ctx],
  );

  function backToMain() {
    setMode(mainMode);
    setTimeout(() => inputRef.current?.focus(), 0);
  }

  return (
    <div
      className="flex h-full w-full flex-col items-stretch px-6 py-4"
      data-tauri-drag-region
    >
      {/* Context label — clickable project · domain above the pill.
          Hidden in search mode; the pill itself shows a search icon then. */}
      {mainMode === "input" && (
        <div
          className="mx-auto mb-3 flex max-w-fit items-center justify-center gap-1.5 rounded-full border border-hairline bg-surface/80 px-3 py-1 text-[13px] text-mute backdrop-blur-md"
        >
          <LabelButton
            kind="proj"
            value={project}
            placeholder="프로젝트 고르기"
            active={mode === "pick-project"}
            onClick={() =>
              setMode((m) => (m === "pick-project" ? mainMode : "pick-project"))
            }
            onClear={
              project
                ? () => {
                    setProject(null);
                    setDomain(null);
                  }
                : undefined
            }
          />
          {project && (
            <>
              <span className="text-stone">·</span>
              <LabelButton
                kind="file"
                value={domain}
                placeholder={domainOptions.length === 0 ? "파일 없음" : "자동"}
                active={mode === "pick-domain"}
                mono
                disabled={domainOptions.length === 0 && !domain}
                onClick={() =>
                  setMode((m) => (m === "pick-domain" ? mainMode : "pick-domain"))
                }
                onClear={domain ? () => setDomain(null) : undefined}
              />
            </>
          )}
        </div>
      )}

      {/* Pill — body swaps between input / search / chip picker */}
      <div
        className="flex h-16 items-center gap-3 rounded-full border border-hairline bg-surface-elevated px-5 shadow-xl shadow-black/30"
        data-tauri-drag-region={false}
      >
        <button
          type="button"
          onClick={() => {
            if (mode === "pick-project" || mode === "pick-domain") {
              backToMain();
            } else {
              ipc.hideCapture().catch(() => {});
            }
          }}
          className="grid h-10 w-10 shrink-0 place-items-center rounded-full text-mute transition-colors hover:bg-surface-card hover:text-on-dark"
          title={
            mode === "pick-project" || mode === "pick-domain"
              ? "돌아가기 (Esc)"
              : "닫기 (Esc)"
          }
        >
          {mainMode === "search" ? (
            <Search size={18} />
          ) : (
            <Plus size={20} className="rotate-45" />
          )}
        </button>

        {(mode === "input" || mode === "search") && (
          <form
            onSubmit={(e) => {
              e.preventDefault();
              if (mode === "input" && suggest && !project) {
                acceptSuggestion();
                return;
              }
              if (mode === "search") {
                // Enter on first hit = open it. Falls back to no-op.
                const first = searchHits[0];
                if (first) openHit(first);
                return;
              }
              submit();
            }}
            className="flex min-w-0 flex-1 items-center gap-3"
          >
            <input
              ref={inputRef}
              type="text"
              value={input}
              onChange={(e) => setInput(e.target.value)}
              onKeyDown={(e) => {
                if (
                  mode === "input" &&
                  (e.key === "Tab" || e.key === "ArrowRight") &&
                  suggest &&
                  !project
                ) {
                  e.preventDefault();
                  acceptSuggestion();
                }
              }}
              placeholder={
                mode === "search"
                  ? "vault 안에서 검색하기"
                  : project
                    ? `${project}${domain ? ` / ${domain}` : ""} 에 기록할 내용…`
                    : "단비에게 물어보기"
              }
              className="min-w-0 flex-1 bg-transparent text-[17px] text-ink outline-none placeholder:text-mute"
            />
          </form>
        )}

        {mode === "pick-project" && (
          <ChipRow
            items={ctx?.projects ?? []}
            activeValue={project}
            emptyLabel="등록된 프로젝트가 없어요"
            onPick={(p) => {
              setProject(p);
              setDomain(null);
              backToMain();
            }}
          />
        )}

        {mode === "pick-domain" && (
          <ChipRow
            items={domainOptions}
            activeValue={domain}
            emptyLabel="도메인 파일이 없어요"
            mono
            onPick={(d) => {
              setDomain(d);
              backToMain();
            }}
          />
        )}
      </div>

      {/* Search results panel — only in search mode, expands the popup. */}
      {mode === "search" && (input.trim().length > 0 || searchHits.length > 0) && (
        <div className="mt-3 flex-1 overflow-hidden rounded-2xl border border-hairline bg-surface/95 backdrop-blur-md">
          <div className="h-full overflow-y-auto">
            {searchLoading && searchHits.length === 0 ? (
              <div className="px-4 py-3 text-[13px] text-stone">검색 중…</div>
            ) : searchHits.length === 0 ? (
              <div className="px-4 py-3 text-[13px] text-stone">
                결과 없음 — 다른 키워드를 시도해보세요
              </div>
            ) : (
              <ul className="flex flex-col">
                {searchHits.map((h) => {
                  const key = `${h.project}/${h.domain}`;
                  const expanded = expandedKey === key;
                  return (
                    <li key={key}>
                      <button
                        type="button"
                        onClick={() =>
                          setExpandedKey((cur) => (cur === key ? null : key))
                        }
                        onDoubleClick={() => openHit(h)}
                        className={cn(
                          "flex w-full items-start gap-3 px-4 py-2.5 text-left transition-colors",
                          expanded
                            ? "bg-accent-blue-soft/40 text-on-dark"
                            : "hover:bg-surface-elevated/60",
                        )}
                      >
                        <FileText size={13} className="mt-1 shrink-0 text-mute" />
                        <div className="min-w-0 flex-1">
                          <div className="flex items-center gap-1.5 text-[13px]">
                            <span className="text-mute">{h.project}</span>
                            <span className="text-stone">/</span>
                            <span className="truncate font-mono text-body">
                              {h.domain}
                            </span>
                          </div>
                          <div
                            className={cn(
                              "mt-1 text-[12px] leading-[1.55] text-mute",
                              expanded ? "" : "line-clamp-2",
                            )}
                          >
                            {h.snippet || "(미리보기 없음)"}
                          </div>
                          {expanded && (
                            <div className="mt-2 flex items-center gap-2 text-[11px] text-stone">
                              <button
                                type="button"
                                onClick={(e) => {
                                  e.stopPropagation();
                                  openHit(h);
                                }}
                                className="inline-flex items-center gap-1 rounded-sm border border-accent-blue bg-accent-blue px-2 py-1 text-on-primary hover:bg-primary-pressed"
                              >
                                단비에서 열기
                                <ChevronRight size={11} />
                              </button>
                              <span className="text-stone">
                                relevance{" "}
                                {(h.relevance * 100).toFixed(0)}%
                              </span>
                            </div>
                          )}
                        </div>
                      </button>
                    </li>
                  );
                })}
              </ul>
            )}
          </div>
        </div>
      )}

      {/* Helper row — mode-specific hint at the bottom. text-mute 가 아닌
          text-body 로 잡아서 가독성 확보. capture popup 은 메인 윈도우와
          달리 transparent 배경 위에 떠있어서 흐린 색이면 시각적으로
          묻힌다. */}
      <div className="mt-3 flex min-h-[20px] shrink-0 items-center justify-center gap-2 text-[13px]">
        {mode === "pick-project" ? (
          <span className="text-body">
            프로젝트를 선택하세요 · <Kbd>Esc</Kbd> 취소
          </span>
        ) : mode === "pick-domain" ? (
          <span className="text-body">
            파일을 선택하세요 · <Kbd>Esc</Kbd> 취소
          </span>
        ) : mode === "search" ? (
          <span className="text-body">
            <Kbd>Tab</Kbd> 입력 모드 · <Kbd>Enter</Kbd> 첫 결과 열기 ·{" "}
            <Kbd>Esc</Kbd> 닫기
          </span>
        ) : suggest && !project ? (
          <button
            onClick={acceptSuggestion}
            className="inline-flex items-center gap-1 text-body hover:text-on-dark"
          >
            <Kbd>⇥</Kbd>
            <span>
              <span className="text-accent-blue">{suggest.project}</span>{" "}
              프로젝트로 고정
            </span>
          </button>
        ) : status.kind === "error" ? (
          <span className="text-accent-red">{status.message}</span>
        ) : status.kind === "ok" ? (
          <span className="text-accent-green">
            ✓ {status.target} — {status.summary}
          </span>
        ) : (
          <span className="text-body">
            <Kbd>Enter</Kbd> 전송 · <Kbd>Tab</Kbd> 검색 모드 ·{" "}
            <Kbd>Esc</Kbd> 닫기
          </span>
        )}
      </div>
    </div>
  );

  /** Open the given hit by switching the main app to that selection.
   *  Closes the popup so the user lands directly on the target doc. */
  function openHit(hit: SearchHit) {
    ipc.captureOpenHit(hit.project, hit.domain).catch((e) => {
      console.error("[danbi] open hit failed", e);
    });
    ipc.hideCapture().catch(() => {});
  }
}

function Kbd({ children }: { children: React.ReactNode }) {
  return (
    <kbd className="mx-0.5 rounded-sm border border-hairline-strong bg-surface-card px-1.5 py-0.5 font-mono text-[11px] font-medium text-on-dark">
      {children}
    </kbd>
  );
}

function LabelButton({
  kind,
  value,
  placeholder,
  active,
  mono,
  disabled,
  onClick,
  onClear,
}: {
  kind: "proj" | "file";
  value: string | null;
  placeholder: string;
  active: boolean;
  mono?: boolean;
  disabled?: boolean;
  onClick: () => void;
  onClear?: () => void;
}) {
  return (
    <div className="relative flex items-center">
      <button
        onClick={onClick}
        disabled={disabled}
        className={cn(
          "inline-flex items-center gap-1 rounded-full px-2 py-0.5 transition-colors",
          disabled
            ? "cursor-default text-stone"
            : active
              ? "bg-surface-elevated text-on-dark"
              : value
                ? "text-on-dark hover:bg-surface-elevated"
                : "text-mute hover:text-on-dark",
        )}
      >
        <span className="text-[11px] uppercase tracking-[0.6px] opacity-60">
          {kind}
        </span>
        <span
          className={cn(
            "text-[13px]",
            mono && "font-mono",
            value ? "font-medium" : "italic",
          )}
        >
          {value ?? placeholder}
        </span>
      </button>
      {onClear && (
        <button
          onClick={(e) => {
            e.stopPropagation();
            onClear();
          }}
          className="ml-0.5 inline-flex h-4 w-4 place-items-center rounded-full text-stone hover:text-on-dark"
          title="초기화"
        >
          <X size={9} />
        </button>
      )}
    </div>
  );
}

function ChipRow({
  items,
  activeValue,
  emptyLabel,
  mono,
  onPick,
}: {
  items: string[];
  activeValue: string | null;
  emptyLabel: string;
  mono?: boolean;
  onPick: (value: string) => void;
}) {
  if (items.length === 0) {
    return (
      <div className="flex min-w-0 flex-1 items-center">
        <span className="text-[13px] italic text-stone">{emptyLabel}</span>
      </div>
    );
  }
  return (
    <div
      className="flex min-w-0 flex-1 items-center gap-2 overflow-x-auto whitespace-nowrap [scrollbar-width:none] [&::-webkit-scrollbar]:hidden"
    >
      {items.map((it) => {
        const isActive = it === activeValue;
        return (
          <button
            key={it}
            onClick={() => onPick(it)}
            className={cn(
              "inline-flex shrink-0 items-center gap-1 rounded-full border px-3 py-1.5 text-[13px] transition-colors",
              mono && "font-mono text-[12px]",
              isActive
                ? "border-on-dark bg-on-dark/10 text-on-dark"
                : "border-hairline bg-surface text-body hover:border-hairline-strong hover:text-on-dark",
            )}
          >
            {isActive && <Check size={11} />}
            {it}
          </button>
        );
      })}
    </div>
  );
}

function escapeRegex(s: string): string {
  return s.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}
