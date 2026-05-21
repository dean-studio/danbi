import { useEffect, useState } from "react";
import lightSrc from "@/assets/danbi-wordmark-light.png";
import darkSrc from "@/assets/danbi-wordmark-dark.png";
import { resolveTheme, type ThemeChoice } from "@/lib/theme";
import { useApp } from "@/state/store";

/**
 * "danbi" wordmark that picks the right asset for the active theme:
 *   - dark → white glyphs on dark (danbi-wordmark-dark.png)
 *   - light → black glyphs on white (danbi-wordmark-light.png)
 *
 * `system` mode listens to prefers-color-scheme so the logo flips live
 * with the OS appearance. Falls back to dark if no cfg is loaded yet.
 */
export function Wordmark({ className }: { className?: string }) {
  const choice = useApp(
    (s) => (s.cfg?.appearance.theme as ThemeChoice | undefined) ?? "dark",
  );
  const [resolved, setResolved] = useState(() => resolveTheme(choice));

  useEffect(() => {
    setResolved(resolveTheme(choice));
    if (choice !== "system") return;
    const mql = window.matchMedia("(prefers-color-scheme: light)");
    const listener = (e: MediaQueryListEvent) =>
      setResolved(e.matches ? "light" : "dark");
    mql.addEventListener?.("change", listener);
    return () => mql.removeEventListener?.("change", listener);
  }, [choice]);

  const src = resolved === "light" ? lightSrc : darkSrc;

  return (
    <img
      src={src}
      alt="danbi"
      draggable={false}
      className={"select-none " + (className ?? "")}
    />
  );
}
