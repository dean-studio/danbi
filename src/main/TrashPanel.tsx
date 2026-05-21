import { useCallback, useEffect, useState } from "react";
import { FolderArchive, RotateCcw, Trash2, X } from "lucide-react";
import { ipc, type TrashEntry } from "@/lib/ipc";
import { cn } from "@/lib/utils";

/**
 * Modal-style panel for the soft-delete trash. Lists every entry stored
 * under `.danbi/trash/`, lets the user restore (move payload back to its
 * original project/path) or permanently purge individual entries, and
 * exposes a "비우기" button for nuking everything at once.
 *
 * Restore conflicts (target already exists at the original path) bubble
 * up as a per-row error message so the user can sort it out without the
 * panel falling over.
 */
export function TrashPanel({
  open,
  onClose,
  onAfterChange,
}: {
  open: boolean;
  onClose: () => void;
  /** Called whenever the trash mutates (restore / purge / empty) so the
   *  parent can refresh the vault tree. */
  onAfterChange: () => void;
}) {
  const [entries, setEntries] = useState<TrashEntry[]>([]);
  const [busy, setBusy] = useState(false);
  const [rowErr, setRowErr] = useState<Record<string, string>>({});

  const refresh = useCallback(async () => {
    try {
      setEntries(await ipc.trashList());
    } catch (e) {
      console.error("[danbi] trashList failed", e);
      setEntries([]);
    }
  }, []);

  useEffect(() => {
    if (open) refresh();
  }, [open, refresh]);

  // Esc to close — small dialog UX nicety.
  useEffect(() => {
    if (!open) return;
    function onKey(e: KeyboardEvent) {
      if (e.key === "Escape") onClose();
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [open, onClose]);

  if (!open) return null;

  const restore = async (entry: TrashEntry) => {
    setBusy(true);
    setRowErr((m) => {
      const next = { ...m };
      delete next[entry.id];
      return next;
    });
    try {
      await ipc.trashRestore(entry.id);
      await refresh();
      onAfterChange();
    } catch (e) {
      setRowErr((m) => ({ ...m, [entry.id]: String(e) }));
    } finally {
      setBusy(false);
    }
  };

  const purge = async (entry: TrashEntry) => {
    setBusy(true);
    try {
      await ipc.trashPurge(entry.id);
      await refresh();
      onAfterChange();
    } catch (e) {
      setRowErr((m) => ({ ...m, [entry.id]: String(e) }));
    } finally {
      setBusy(false);
    }
  };

  const emptyAll = async () => {
    if (entries.length === 0) return;
    setBusy(true);
    try {
      await ipc.trashEmpty();
      await refresh();
      onAfterChange();
    } catch (e) {
      console.error("[danbi] trashEmpty failed", e);
    } finally {
      setBusy(false);
    }
  };

  return (
    <div
      className="fixed inset-0 z-40 flex items-center justify-center bg-canvas/70 backdrop-blur-sm"
      onMouseDown={(e) => {
        if (e.target === e.currentTarget) onClose();
      }}
    >
      <div className="flex h-[640px] w-[760px] flex-col overflow-hidden rounded-lg border border-hairline bg-surface">
        <header
          data-tauri-drag-region
          className="flex h-14 shrink-0 items-center justify-between border-b border-hairline px-6"
        >
          <div className="flex items-center gap-2.5">
            <Trash2 size={18} className="text-accent-yellow" />
            <span className="text-[16px] font-semibold text-ink">휴지통</span>
            {entries.length > 0 && (
              <span className="rounded-xs bg-surface-elevated px-2 py-0.5 text-[12px] uppercase tracking-[0.4px] text-on-dark-mute">
                {entries.length}
              </span>
            )}
          </div>
          <div className="flex items-center gap-2">
            {entries.length > 0 && (
              <button
                onClick={emptyAll}
                disabled={busy}
                className="inline-flex h-9 items-center gap-1.5 rounded-md border border-hairline bg-surface-elevated px-3 text-[13px] text-body hover:border-accent-red hover:text-accent-red disabled:opacity-60"
              >
                모두 비우기
              </button>
            )}
            <button
              onClick={onClose}
              className="grid h-9 w-9 place-items-center rounded-md text-mute transition-colors hover:bg-surface-elevated hover:text-on-dark"
              title="닫기 (Esc)"
            >
              <X size={18} />
            </button>
          </div>
        </header>

        <div className="flex-1 min-h-0 overflow-auto">
          {entries.length === 0 ? (
            <div className="flex h-full flex-col items-center justify-center gap-3 text-stone">
              <FolderArchive size={40} className="opacity-50" />
              <div className="text-[15px]">휴지통이 비어 있어요</div>
              <div className="text-[13px] text-mute">
                삭제한 도메인 / 폴더가 여기로 옮겨져요. 30일이 지나면 단비가
                자동으로 영구 삭제합니다.
              </div>
            </div>
          ) : (
            <>
              <div className="border-b border-hairline bg-surface px-6 py-3 text-[13px] text-mute">
                삭제한 지 30일이 지난 항목은 단비 시작 시 자동으로 영구
                삭제돼요. 그 전엔 언제든 복원 가능합니다.
              </div>
              <ul className="flex flex-col">
              {entries.map((entry) => (
                <li
                  key={entry.id}
                  className="flex items-start gap-4 border-b border-hairline px-6 py-4 last:border-b-0"
                >
                  <div className="mt-1 grid h-10 w-10 shrink-0 place-items-center rounded-md bg-surface-elevated text-stone">
                    {entry.kind === "folder" ? (
                      <FolderArchive size={20} />
                    ) : (
                      <Trash2 size={20} />
                    )}
                  </div>
                  <div className="min-w-0 flex-1">
                    <div className="flex items-center gap-2">
                      <span className="rounded-xs bg-surface-elevated px-2 py-0.5 text-[12px] font-medium uppercase tracking-[0.4px] text-on-dark-mute">
                        {entry.project}
                      </span>
                      <code className="truncate font-mono text-[14px] text-ink">
                        {entry.original_path || "(프로젝트 전체)"}
                      </code>
                    </div>
                    <div className="mt-1.5 flex items-center gap-2 text-[13px] text-stone">
                      <span>
                        {entry.kind === "folder"
                          ? "폴더"
                          : entry.kind === "project"
                            ? "프로젝트"
                            : "파일"}
                      </span>
                      <span>·</span>
                      <span>{prettyBytes(entry.size_bytes)}</span>
                      <span>·</span>
                      <span title={new Date(entry.deleted_at * 1000).toLocaleString()}>
                        {timeAgo(entry.deleted_at * 1000)}
                      </span>
                    </div>
                    {rowErr[entry.id] && (
                      <div className="mt-2 rounded-sm border border-accent-red/40 bg-accent-red-soft px-2.5 py-1.5 text-[13px] text-accent-red">
                        {rowErr[entry.id]}
                      </div>
                    )}
                  </div>
                  <div className="flex shrink-0 items-center gap-2">
                    <button
                      onClick={() => restore(entry)}
                      disabled={busy}
                      title="원래 위치로 복원"
                      className={cn(
                        "inline-flex h-10 items-center gap-1.5 rounded-md border border-hairline bg-surface-elevated px-3.5 text-[13px] text-body hover:border-hairline-strong hover:text-on-dark disabled:opacity-60",
                      )}
                    >
                      <RotateCcw size={15} /> 복원
                    </button>
                    <button
                      onClick={() => purge(entry)}
                      disabled={busy}
                      title="영구 삭제"
                      className="grid h-10 w-10 place-items-center rounded-md border border-hairline bg-surface-elevated text-mute hover:border-accent-red hover:text-accent-red disabled:opacity-60"
                    >
                      <Trash2 size={15} />
                    </button>
                  </div>
                </li>
              ))}
              </ul>
            </>
          )}
        </div>

        <footer className="shrink-0 border-t border-hairline bg-surface-elevated px-6 py-3 text-[12px] text-stone">
          저장 위치:{" "}
          <code className="font-mono text-on-dark-mute">~/Danbi_Vault/.danbi/trash/</code>
        </footer>
      </div>
    </div>
  );
}

function prettyBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  return `${(n / (1024 * 1024)).toFixed(1)} MB`;
}

function timeAgo(ms: number): string {
  const diff = Date.now() - ms;
  if (diff < 60_000) return "방금";
  if (diff < 3_600_000) return `${Math.floor(diff / 60_000)}분 전`;
  if (diff < 86_400_000) return `${Math.floor(diff / 3_600_000)}시간 전`;
  if (diff < 7 * 86_400_000) return `${Math.floor(diff / 86_400_000)}일 전`;
  const d = new Date(ms);
  return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}`;
}
