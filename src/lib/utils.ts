import { clsx, type ClassValue } from "clsx";
import { twMerge } from "tailwind-merge";
import type { MouseEvent } from "react";

export function cn(...inputs: ClassValue[]) {
  return twMerge(clsx(inputs));
}

/** `<a target="_blank">` 는 Tauri webview 안에서 아무 일도 안 한다 —
 *  외부 링크의 onClick 에 걸어 기본 브라우저로 연다. */
export function openExternal(e: MouseEvent<HTMLAnchorElement>) {
  e.preventDefault();
  const href = e.currentTarget.href;
  import("@tauri-apps/plugin-opener")
    .then(({ openUrl }) => openUrl(href))
    .catch((err) => console.error("[danbi] openUrl failed", href, err));
}
