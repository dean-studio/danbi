/**
 * Theme switching — applies the active mode to <html data-theme="…">
 * so the CSS-variable overrides in index.css take effect.
 *
 * cfg.appearance.theme is one of "dark" | "light" | "system".
 * "system" follows prefers-color-scheme and live-updates on change.
 */

export type ThemeChoice = "dark" | "light" | "system";
export type ResolvedTheme = "dark" | "light";

const MEDIA_QUERY = "(prefers-color-scheme: light)";

export function resolveTheme(choice: ThemeChoice): ResolvedTheme {
  if (choice === "system") {
    if (typeof window !== "undefined" && window.matchMedia) {
      return window.matchMedia(MEDIA_QUERY).matches ? "light" : "dark";
    }
    return "dark";
  }
  return choice;
}

export function applyTheme(resolved: ResolvedTheme) {
  if (typeof document === "undefined") return;
  document.documentElement.dataset.theme = resolved;
}

/**
 * Install the active theme and, when the choice is "system", subscribe to
 * system-level changes so the app follows OS appearance live. Returns a
 * cleanup that removes the listener (if any) — call it before re-installing.
 */
export function installTheme(choice: ThemeChoice): () => void {
  applyTheme(resolveTheme(choice));

  if (choice !== "system") return () => undefined;
  if (typeof window === "undefined" || !window.matchMedia) return () => undefined;

  const mql = window.matchMedia(MEDIA_QUERY);
  const listener = (e: MediaQueryListEvent) => {
    applyTheme(e.matches ? "light" : "dark");
  };

  mql.addEventListener("change", listener);
  return () => mql.removeEventListener("change", listener);
}
