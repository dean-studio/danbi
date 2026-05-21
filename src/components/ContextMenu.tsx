import { useEffect } from "react";

export type MenuItem =
  | {
      kind?: "item";
      label: string;
      onClick: () => void;
      danger?: boolean;
      /** 비활성 상태 — edge 에서 ↑/↓ 이동처럼 가능여부에 따라 흐리게. */
      disabled?: boolean;
    }
  | { kind: "divider" };

export function ContextMenu({
  x,
  y,
  items,
  onClose,
}: {
  x: number;
  y: number;
  items: MenuItem[];
  onClose: () => void;
}) {
  useEffect(() => {
    function onAny() {
      onClose();
    }
    function onKey(e: KeyboardEvent) {
      if (e.key === "Escape") onClose();
    }
    window.addEventListener("mousedown", onAny);
    window.addEventListener("keydown", onKey);
    window.addEventListener("blur", onAny);
    return () => {
      window.removeEventListener("mousedown", onAny);
      window.removeEventListener("keydown", onKey);
      window.removeEventListener("blur", onAny);
    };
  }, [onClose]);

  return (
    <div
      style={{ left: x, top: y }}
      className="fixed z-50 min-w-44 rounded-md border border-hairline bg-surface py-1 shadow-lg shadow-black/40"
      onMouseDown={(e) => e.stopPropagation()}
    >
      {items.map((it, i) => {
        if (it.kind === "divider") {
          return (
            <div
              key={i}
              className="my-1 border-t border-hairline"
              aria-hidden
            />
          );
        }
        return (
          <button
            key={i}
            disabled={it.disabled}
            onMouseDown={(e) => {
              if (it.disabled) return;
              e.stopPropagation();
              it.onClick();
              onClose();
            }}
            className={
              "block w-full px-3 py-1.5 text-left text-[13px] transition-colors disabled:cursor-not-allowed disabled:opacity-40 " +
              (it.danger
                ? "text-accent-red hover:bg-accent-red-soft"
                : "text-body hover:bg-surface-elevated hover:text-on-dark")
            }
          >
            {it.label}
          </button>
        );
      })}
    </div>
  );
}
