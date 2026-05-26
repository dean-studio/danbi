import { useEffect, useRef } from "react";
import { cn } from "@/lib/utils";

/** Per-project accent color keys. Stored verbatim in
 *  `cfg.project_colors[project]`. Keep this list small (<10) — too many
 *  colors makes the sidebar feel chaotic, and a finite set lets us
 *  guarantee every key has a real CSS variable behind it.
 *
 *  The four extras (purple/cyan/pink/orange) live in `index.css` next to
 *  the original blue/yellow/green/red so dark and light themes both
 *  resolve. Adding a key here without the matching CSS var will silently
 *  fall back to default blue. */
export const PROJECT_COLOR_KEYS = [
  "blue",
  "yellow",
  "green",
  "red",
  "purple",
  "cyan",
  "pink",
  "orange",
] as const;

export type ProjectColorKey = (typeof PROJECT_COLOR_KEYS)[number];

const KEY_LABELS: Record<ProjectColorKey, string> = {
  blue: "기본 (Blue)",
  yellow: "Yellow",
  green: "Green",
  red: "Red",
  purple: "Purple",
  cyan: "Cyan",
  pink: "Pink",
  orange: "Orange",
};

/** Resolve a color key to the dark-mode hex used for the swatch preview
 *  AND for inline style usage across the sidebar. We deliberately bypass
 *  Tailwind v4's `@theme` var indirection here — v4 tree-shakes theme
 *  vars that aren't referenced by a utility class, and keys like
 *  `purple`/`cyan`/`pink`/`orange` aren't used as `bg-accent-*` anywhere,
 *  so their CSS custom properties get pruned. Inline-style consumers
 *  receiving `var(--color-accent-purple)` then resolve to nothing.
 *  Hex strings sidestep that entirely. */
const SWATCH_HEX: Record<ProjectColorKey, string> = {
  blue: "#57c1ff",
  yellow: "#ffc533",
  green: "#59d499",
  red: "#ff6161",
  purple: "#b08bff",
  cyan: "#5ed3e0",
  pink: "#ff8fc8",
  orange: "#ff9554",
};

/** Soft (alpha-blended) tone used for active row backgrounds. Same hex
 *  family as SWATCH_HEX but at ~15% alpha so it reads as a tint over the
 *  dark surface without overwhelming the foreground icon/text. */
const SWATCH_SOFT: Record<ProjectColorKey, string> = {
  blue: "rgba(87, 193, 255, 0.15)",
  yellow: "rgba(255, 197, 51, 0.15)",
  green: "rgba(89, 212, 153, 0.15)",
  red: "rgba(255, 97, 97, 0.15)",
  purple: "rgba(176, 139, 255, 0.15)",
  cyan: "rgba(94, 211, 224, 0.15)",
  pink: "rgba(255, 143, 200, 0.15)",
  orange: "rgba(255, 149, 84, 0.15)",
};

/** Map a stored color key (or null) to concrete color strings we use
 *  across the sidebar. Always returns valid colors — unknown keys
 *  collapse to the default blue accent so a stale config never breaks
 *  the UI. */
export function projectColorVars(key: string | null | undefined): {
  fg: string;
  soft: string;
} {
  const k = (PROJECT_COLOR_KEYS as readonly string[]).includes(key ?? "")
    ? (key as ProjectColorKey)
    : "blue";
  return { fg: SWATCH_HEX[k], soft: SWATCH_SOFT[k] };
}

export function ProjectColorPicker({
  value,
  onSelect,
  onClear,
  onClose,
}: {
  value: string | null;
  onSelect: (key: ProjectColorKey) => void;
  onClear: () => void;
  onClose: () => void;
}) {
  // Picker 가 열릴 때 현재 선택된 색 버튼으로 키보드 포커스를 옮긴다.
  // value 가 null 이면 디폴트 (blue) 로 떨어짐 — picker 가 처음 열린 직후
  // Tab 한 번이면 바로 다른 색으로 이동할 수 있도록 anchor 가 명확해야.
  const activeRef = useRef<HTMLButtonElement | null>(null);
  useEffect(() => {
    activeRef.current?.focus();
  }, []);

  return (
    <div
      className="flex flex-col gap-3"
      onKeyDown={(e) => {
        if (e.key === "Escape") onClose();
      }}
    >
      <div className="grid grid-cols-4 gap-2">
        {PROJECT_COLOR_KEYS.map((key) => {
          const active = value === key || (!value && key === "blue");
          return (
            <button
              key={key}
              ref={active ? activeRef : undefined}
              type="button"
              onClick={() => onSelect(key)}
              title={KEY_LABELS[key]}
              className={cn(
                "flex h-16 flex-col items-center justify-center gap-1 rounded-md border transition-colors focus:outline-none",
                active
                  ? "border-transparent bg-surface-elevated ring-2 ring-offset-2 ring-offset-surface"
                  : "border-hairline bg-surface-elevated hover:border-hairline-strong",
              )}
              style={
                active
                  ? ({ "--tw-ring-color": SWATCH_HEX[key] } as React.CSSProperties)
                  : undefined
              }
            >
              <span
                className="h-5 w-5 rounded-full"
                style={{ backgroundColor: SWATCH_HEX[key] }}
              />
              <span className="text-[10px] text-mute">{KEY_LABELS[key]}</span>
            </button>
          );
        })}
      </div>
      <div className="flex items-center justify-between text-caption-sm text-mute">
        <span>활성 행·헤더·카드 테두리에 적용됩니다.</span>
        <button
          type="button"
          onClick={onClear}
          className="rounded-sm px-2 py-1 text-mute hover:text-ink"
        >
          기본으로 되돌리기
        </button>
      </div>
    </div>
  );
}
