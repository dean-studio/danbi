import { useEffect, useState } from "react";
import { Check, Inbox, RefreshCw, X } from "lucide-react";
import { ipc, type ReviewItem, type ReviewStore } from "@/lib/ipc";
import { cn } from "@/lib/utils";
import { useApp } from "@/state/store";

/**
 * Overlay showing the Review queue — items the agent flagged as needing
 * human judgment (low-confidence routings, healing warnings, borderline
 * ghost suggestions, etc.). Mirrors the overlay pattern used by GraphView
 * so activation / escape behavior feels consistent.
 */
export function ReviewPanel({
  open,
  onClose,
}: {
  open: boolean;
  onClose: () => void;
}) {
  const selectDomain = useApp((s) => s.selectDomain);
  const [store, setStore] = useState<ReviewStore | null>(null);
  const [loading, setLoading] = useState(false);
  const [err, setErr] = useState<string | null>(null);

  async function refresh() {
    setLoading(true);
    setErr(null);
    try {
      setStore(await ipc.reviewsList());
    } catch (e) {
      setErr(String(e));
    } finally {
      setLoading(false);
    }
  }

  useEffect(() => {
    if (open) refresh();
  }, [open]);

  useEffect(() => {
    if (!open) return;
    function onKey(e: KeyboardEvent) {
      if (e.key === "Escape") onClose();
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [open, onClose]);

  async function mark(id: string, status: "resolved" | "dismissed") {
    try {
      const next = await ipc.reviewsResolve(id, status);
      setStore(next);
    } catch (e) {
      setErr(String(e));
    }
  }

  if (!open) return null;

  const pending = (store?.items ?? []).filter((it) => it.status === "pending");
  const past = (store?.items ?? [])
    .filter((it) => it.status !== "pending")
    .sort((a, b) => (b.resolved_at ?? 0) - (a.resolved_at ?? 0))
    .slice(0, 10);

  return (
    <div
      className="fixed inset-0 z-40 flex items-center justify-center bg-canvas/70 backdrop-blur-sm"
      onMouseDown={(e) => {
        if (e.target === e.currentTarget) onClose();
      }}
    >
      <div className="flex h-[620px] w-[720px] flex-col overflow-hidden rounded-lg border border-hairline bg-surface">
        <header
          data-tauri-drag-region
          className="flex h-10 shrink-0 items-center justify-between border-b border-hairline px-4"
        >
          <div className="flex items-center gap-2 text-[13px] font-medium text-ink">
            <Inbox size={13} className="text-accent-blue" />
            리뷰 인박스
            <span className="text-caption-sm text-stone">
              {pending.length} 대기
            </span>
          </div>
          <div className="flex items-center gap-1.5">
            <button
              onClick={refresh}
              disabled={loading}
              className="grid h-6 w-6 place-items-center rounded-sm text-mute hover:bg-surface-elevated hover:text-on-dark"
              title="새로고침"
            >
              <RefreshCw size={11} className={loading ? "animate-spin" : ""} />
            </button>
            <button
              onClick={onClose}
              className="grid h-6 w-6 place-items-center rounded-sm text-mute hover:bg-surface-elevated hover:text-on-dark"
              title="닫기 (Esc)"
            >
              <X size={12} />
            </button>
          </div>
        </header>

        <div className="flex-1 min-h-0 overflow-auto px-5 py-4">
          {err && (
            <div className="mb-3 rounded-md border border-hairline bg-surface-elevated p-2 font-mono text-[11px] text-accent-red">
              {err}
            </div>
          )}

          {pending.length === 0 && past.length === 0 && !loading && (
            <div className="flex h-full flex-col items-center justify-center gap-2 text-center">
              <Inbox size={22} className="text-stone" />
              <div className="text-[13px] text-mute">
                리뷰할 항목이 없어요.
              </div>
              <div className="text-caption-sm text-stone">
                단비가 확신하지 못한 작업은 여기에 쌓여요.
              </div>
            </div>
          )}

          {pending.length > 0 && (
            <section className="mb-5">
              <h3 className="mb-2 text-[11px] uppercase tracking-[0.6px] text-mute">
                대기 중
              </h3>
              <ul className="flex flex-col gap-2">
                {pending.map((it) => (
                  <ReviewRow
                    key={it.id}
                    item={it}
                    onOpen={() =>
                      it.project &&
                      it.domain &&
                      selectDomain(it.project, it.domain)
                    }
                    onResolve={() => mark(it.id, "resolved")}
                    onDismiss={() => mark(it.id, "dismissed")}
                  />
                ))}
              </ul>
            </section>
          )}

          {past.length > 0 && (
            <section>
              <h3 className="mb-2 text-[11px] uppercase tracking-[0.6px] text-mute">
                최근 처리됨
              </h3>
              <ul className="flex flex-col gap-1">
                {past.map((it) => (
                  <li
                    key={it.id}
                    className="flex items-center gap-2 rounded-sm border border-hairline bg-surface-elevated px-2 py-1.5 text-caption-md text-stone"
                  >
                    <span
                      className={cn(
                        "rounded-xs px-1.5 py-0.5 text-[10px] uppercase tracking-[0.4px]",
                        it.status === "resolved"
                          ? "bg-accent-green-soft text-accent-green"
                          : "bg-surface-card text-mute",
                      )}
                    >
                      {it.status === "resolved" ? "해결" : "무시"}
                    </span>
                    <span className="truncate">{it.reason}</span>
                    <span className="ml-auto text-stone">{kindLabel(it.kind)}</span>
                  </li>
                ))}
              </ul>
            </section>
          )}
        </div>
      </div>
    </div>
  );
}

function ReviewRow({
  item,
  onOpen,
  onResolve,
  onDismiss,
}: {
  item: ReviewItem;
  onOpen: () => void;
  onResolve: () => void;
  onDismiss: () => void;
}) {
  return (
    <li className="flex items-start gap-3 rounded-md border border-hairline bg-surface-elevated p-3">
      <div className="flex-1 min-w-0">
        <div className="mb-1 flex items-center gap-2">
          <span className="rounded-xs bg-accent-blue-soft px-1.5 py-0.5 text-[10px] font-medium uppercase tracking-[0.4px] text-accent-blue">
            {kindLabel(item.kind)}
          </span>
          {item.project && (
            <span className="text-caption-sm text-mute">{item.project}</span>
          )}
          {item.domain && (
            <>
              <span className="text-stone">/</span>
              <span className="font-mono text-[11px] text-body">
                {item.domain}
              </span>
            </>
          )}
        </div>
        <div className="text-[13px] leading-[1.5] text-ink">{item.reason}</div>
      </div>
      <div className="flex items-center gap-1.5">
        {item.project && item.domain && (
          <button
            onClick={onOpen}
            className="inline-flex h-7 items-center rounded-sm border border-hairline bg-surface px-2 text-[12px] text-body hover:text-on-dark"
          >
            열기
          </button>
        )}
        <button
          onClick={onResolve}
          className="inline-flex h-7 items-center gap-1 rounded-sm bg-primary px-2 text-[12px] font-medium text-on-primary hover:bg-primary-pressed"
          title="해결됨"
        >
          <Check size={11} /> 해결
        </button>
        <button
          onClick={onDismiss}
          className="inline-flex h-7 items-center rounded-sm border border-hairline bg-surface px-2 text-[12px] text-stone hover:text-on-dark"
          title="무시"
        >
          무시
        </button>
      </div>
    </li>
  );
}

function kindLabel(kind: string): string {
  switch (kind) {
    case "low_confidence_plan":
      return "신뢰도";
    case "unclear_destination":
      return "목적지";
    case "suggest_split":
      return "분할";
    case "duplicate_content":
      return "중복";
    case "broken_link":
      return "링크";
    case "ghost_candidate":
      return "제안";
    default:
      return kind;
  }
}
