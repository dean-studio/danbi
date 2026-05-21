import { useEffect, useRef } from "react";

export function Dialog({
  open,
  onClose,
  title,
  children,
  footer,
  width = 420,
}: {
  open: boolean;
  onClose: () => void;
  title: string;
  children: React.ReactNode;
  footer?: React.ReactNode;
  width?: number;
}) {
  const ref = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    if (!open) return;
    function onKey(e: KeyboardEvent) {
      if (e.key === "Escape") onClose();
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [open, onClose]);

  if (!open) return null;
  return (
    <div
      className="fixed inset-0 z-40 flex items-center justify-center bg-canvas/70 backdrop-blur-sm"
      onMouseDown={(e) => {
        if (e.target === e.currentTarget) onClose();
      }}
    >
      <div
        ref={ref}
        style={{ width }}
        className="rounded-lg border border-hairline bg-surface"
      >
        <header className="border-b border-hairline px-6 py-4">
          <div className="flex items-center gap-2.5">
            <div className="h-1.5 w-1.5 rounded-full bg-accent-blue" aria-hidden />
            <span className="text-[15px] font-medium tracking-[0.2px] text-on-dark">
              {title}
            </span>
          </div>
        </header>
        <div className="p-6">{children}</div>
        {footer && (
          <footer className="flex items-center justify-end gap-2 border-t border-hairline px-6 py-4">
            {footer}
          </footer>
        )}
      </div>
    </div>
  );
}
