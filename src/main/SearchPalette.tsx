import { useEffect, useRef, useState } from "react";
import { FileText, Loader2, Search as SearchIcon, Sparkles, X } from "lucide-react";
import { ipc, type SearchHit } from "@/lib/ipc";
import { cn } from "@/lib/utils";
import { useApp } from "@/state/store";

type Mode = "local" | "full" | "ai-running" | "ai-done" | "ai-error";

export function SearchPalette({
  open,
  onClose,
}: {
  open: boolean;
  onClose: () => void;
}) {
  const selectDomain = useApp((s) => s.selectDomain);
  const [query, setQuery] = useState("");
  const [hits, setHits] = useState<SearchHit[]>([]);
  const [mode, setMode] = useState<Mode>("local");
  const [aiSummary, setAiSummary] = useState<string>("");
  const [err, setErr] = useState<string | null>(null);
  const [activeIdx, setActiveIdx] = useState(0);
  const inputRef = useRef<HTMLInputElement | null>(null);
  const localReqRef = useRef(0);

  useEffect(() => {
    if (!open) return;
    setActiveIdx(0);
    setErr(null);
    setMode("local");
    setAiSummary("");
    setTimeout(() => inputRef.current?.focus(), 0);
  }, [open]);

  // Tier 1 (local, instant) then Tier 2 (tantivy, ms) — both free.
  useEffect(() => {
    if (!open) return;
    const q = query.trim();
    if (q.length < 1) {
      setHits([]);
      setMode("local");
      setAiSummary("");
      return;
    }
    const myId = ++localReqRef.current;

    // Tier 1 — fire immediately for that typed-through feeling.
    (async () => {
      try {
        const res = await ipc.searchLocal(q, 20);
        if (myId !== localReqRef.current) return;
        setHits(res);
        setMode("local");
        setAiSummary("");
        setActiveIdx(0);
      } catch {
        /* ignore; Tier 2 will follow anyway */
      }
    })();

    // Tier 2 — full-body tantivy, ~150ms debounce so we don't hammer it.
    const t = setTimeout(async () => {
      try {
        const res = await ipc.searchFull(q, 20);
        if (myId !== localReqRef.current) return;
        // Merge with Tier 1 results: prefer tantivy ordering but keep any
        // filename-only matches the preview search found but body didn't.
        setHits((prev) => mergeHits(res, prev));
        setMode("full");
        setActiveIdx(0);
      } catch (e) {
        if (myId !== localReqRef.current) return;
        console.error("[danbi] searchFull:", e);
        // Keep Tier 1 results visible even if Tier 2 blows up.
      }
    }, 150);
    return () => clearTimeout(t);
  }, [query, open]);

  async function reRankWithAi() {
    const q = query.trim();
    if (q.length < 2) return;
    setMode("ai-running");
    setErr(null);
    try {
      const res = await ipc.searchVault(q);
      setHits(res.hits);
      setAiSummary(res.summary);
      setMode("ai-done");
      setActiveIdx(0);
    } catch (e) {
      setMode("ai-error");
      setErr(String(e));
    }
  }

  useEffect(() => {
    if (!open) return;
    function onKey(e: KeyboardEvent) {
      if (e.key === "Escape") {
        onClose();
        return;
      }
      if (e.key === "ArrowDown") {
        e.preventDefault();
        setActiveIdx((i) => Math.min(hits.length - 1, i + 1));
      }
      if (e.key === "ArrowUp") {
        e.preventDefault();
        setActiveIdx((i) => Math.max(0, i - 1));
      }
      if (e.key === "Enter" && hits[activeIdx]) {
        e.preventDefault();
        const h = hits[activeIdx];
        selectDomain(h.project, h.domain);
        onClose();
      }
      if ((e.metaKey || e.ctrlKey) && e.key === "Enter") {
        e.preventDefault();
        reRankWithAi();
      }
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open, hits, activeIdx, onClose, query]);

  if (!open) return null;

  const aiRunning = mode === "ai-running";

  return (
    <div
      className="fixed inset-0 z-40 flex items-start justify-center bg-canvas/70 pt-24 backdrop-blur-sm"
      onMouseDown={(e) => {
        if (e.target === e.currentTarget) onClose();
      }}
    >
      <div className="flex w-full max-w-[640px] flex-col overflow-hidden rounded-xl border border-hairline bg-surface shadow-2xl shadow-black/40">
        <div className="flex items-center gap-2 border-b border-hairline px-3 py-2.5">
          <SearchIcon size={13} className="text-mute" />
          <input
            ref={inputRef}
            type="text"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder="vault 에서 찾기…"
            className="flex-1 bg-transparent text-[14px] text-ink outline-none placeholder:text-stone"
          />
          <button
            onClick={reRankWithAi}
            disabled={query.trim().length < 2 || aiRunning}
            className={cn(
              "inline-flex h-7 items-center gap-1 rounded-sm border px-2 text-[11px] font-medium transition-colors",
              mode === "ai-done"
                ? "border-accent-blue bg-accent-blue-soft text-accent-blue"
                : "border-hairline bg-surface-elevated text-body hover:border-hairline-strong hover:text-on-dark",
              (query.trim().length < 2 || aiRunning) && "opacity-50",
            )}
            title="Haiku로 재정렬 (⌘Enter)"
          >
            {aiRunning ? (
              <Loader2 size={10} className="animate-spin" />
            ) : (
              <Sparkles size={10} />
            )}
            AI 재정렬
          </button>
          <button
            onClick={onClose}
            className="grid h-6 w-6 place-items-center rounded-sm text-stone transition-colors hover:text-on-dark"
            title="닫기 (Esc)"
          >
            <X size={11} />
          </button>
        </div>

        <div className="max-h-[360px] overflow-y-auto">
          {err && (
            <div className="px-3 py-2 font-mono text-[11px] text-accent-red">
              {err}
            </div>
          )}
          {!err && hits.length === 0 && query.trim().length >= 1 && (
            <div className="px-3 py-4 text-caption-sm text-stone">
              결과가 없어요. <span className="text-mute">AI 재정렬</span> 버튼으로
              다시 시도해 보세요.
            </div>
          )}
          {!err && query.trim().length < 1 && (
            <div className="px-3 py-4 text-caption-sm text-stone">
              파일명·헤딩·본문 앞머리에서 즉시 검색합니다.
            </div>
          )}
          {hits.map((h, i) => {
            const active = i === activeIdx;
            return (
              <button
                key={`${h.project}/${h.domain}/${i}`}
                onMouseEnter={() => setActiveIdx(i)}
                onClick={() => {
                  selectDomain(h.project, h.domain);
                  onClose();
                }}
                className={cn(
                  "flex w-full items-start gap-3 border-l-2 px-3 py-2 text-left transition-colors",
                  active
                    ? "border-accent-blue bg-accent-blue-soft"
                    : "border-transparent bg-transparent hover:bg-surface-elevated",
                )}
              >
                <FileText size={12} className="mt-0.5 shrink-0 text-mute" />
                <div className="min-w-0 flex-1">
                  <div className="flex items-baseline gap-1.5 text-[13px]">
                    <span className="text-mute">{h.project}</span>
                    <span className="text-stone">/</span>
                    <span className="truncate font-mono text-ink">
                      {h.domain}
                    </span>
                    {mode === "ai-done" && (
                      <span className="shrink-0 rounded-xs bg-accent-blue-soft px-1.5 py-0.5 text-[10px] font-medium uppercase tracking-[0.4px] text-accent-blue">
                        AI
                      </span>
                    )}
                    <span className="ml-auto shrink-0 text-caption-sm text-stone">
                      {Math.round(h.relevance * 100)}%
                    </span>
                  </div>
                  {h.snippet && (
                    <div className="mt-0.5 text-caption-sm leading-[1.5] text-body">
                      {h.snippet}
                    </div>
                  )}
                </div>
              </button>
            );
          })}
        </div>

        <div className="flex items-center justify-between border-t border-hairline bg-surface px-3 py-1.5 text-caption-sm">
          <div className="text-stone">
            {mode === "local" && "즉시 검색 (프리뷰)"}
            {mode === "full" && "전체 본문 검색 (tantivy)"}
            {mode === "ai-running" && "AI가 재정렬 중…"}
            {mode === "ai-done" && (aiSummary || "AI 재정렬 완료")}
            {mode === "ai-error" && <span className="text-accent-red">AI 실패</span>}
          </div>
          <div className="flex items-center gap-2 text-stone">
            <kbd className="rounded-xs border border-hairline bg-surface-elevated px-1 font-mono text-[10px]">
              ↑↓
            </kbd>{" "}
            선택
            <kbd className="rounded-xs border border-hairline bg-surface-elevated px-1 font-mono text-[10px]">
              ⏎
            </kbd>{" "}
            열기
            <kbd className="rounded-xs border border-hairline bg-surface-elevated px-1 font-mono text-[10px]">
              ⌘⏎
            </kbd>{" "}
            AI
          </div>
        </div>
      </div>
    </div>
  );
}

/**
 * Merge Tier 2 (tantivy) results on top while preserving any Tier 1 hits
 * that didn't appear in the full-body search — usually filename-only matches.
 */
function mergeHits(primary: SearchHit[], fallback: SearchHit[]): SearchHit[] {
  const seen = new Set<string>();
  const out: SearchHit[] = [];
  for (const h of primary) {
    const key = `${h.project}/${h.domain}`;
    if (!seen.has(key)) {
      seen.add(key);
      out.push(h);
    }
  }
  for (const h of fallback) {
    const key = `${h.project}/${h.domain}`;
    if (!seen.has(key)) {
      seen.add(key);
      out.push(h);
    }
  }
  return out;
}
