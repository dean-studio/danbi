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

/** Resolve a color key to the dark-mode hex used for the swatch preview.
 *  Light-mode is handled at runtime via the CSS variable system, but for
 *  the picker preview we just paint the dark hex — the difference is
 *  cosmetic and avoids reading computed styles per swatch. */
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

/** Map a stored color key (or null) to the CSS variables we use across
 *  the sidebar. Always returns valid vars — unknown keys collapse to the
 *  default blue accent so a stale config never breaks the UI. */
export function projectColorVars(key: string | null | undefined): {
  fg: string;
  soft: string;
} {
  const k = (PROJECT_COLOR_KEYS as readonly string[]).includes(key ?? "")
    ? (key as ProjectColorKey)
    : "blue";
  return {
    fg: `var(--color-accent-${k})`,
    soft: `var(--color-accent-${k}-soft)`,
  };
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
              type="button"
              onClick={() => onSelect(key)}
              title={KEY_LABELS[key]}
              className={cn(
                "flex h-16 flex-col items-center justify-center gap-1 rounded-md border transition-colors",
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
