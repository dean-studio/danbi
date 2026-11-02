import { useCallback, useMemo } from "react";
import type { SubfolderNode } from "@/lib/ipc";
import { useApp } from "@/state/store";

// 주제군·프로젝트 색 — 20색. 앞쪽일수록 서로 멀리 떨어진 색.
export const GROUP_PALETTE = [
  "#57c1ff", "#59d499", "#ffc533", "#ff6161", "#b47cff",
  "#7fffd4", "#ff8c42", "#5eead4", "#fb7185", "#a78bfa",
  "#fbbf24", "#34d399", "#60a5fa", "#f472b6", "#f87171",
  "#4ade80", "#38bdf8", "#fcd34d", "#c084fc", "#facc15",
];

function countDocs(folders: SubfolderNode[]): number {
  let n = 0;
  for (const f of folders) n += f.domains.length + countDocs(f.subfolders ?? []);
  return n;
}

/**
 * 프로젝트 → 색. 문서 많은 순으로 팔레트를 배정해 큰 프로젝트끼리 색이
 * 겹치지 않게 한다. vault 트리 기준이라 그래프에서 프로젝트 하나만 걸러
 * 보거나 문서 인스펙터에서 봐도 같은 프로젝트는 같은 색이다.
 */
export function useProjectColor(): (project: string) => string {
  const tree = useApp((s) => s.tree);
  const rank = useMemo(() => {
    const sizes = (tree?.projects ?? []).map(
      (p) => [p.name, p.domains.length + countDocs(p.subfolders)] as const,
    );
    sizes.sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0]));
    return new Map(sizes.map(([name], i) => [name, i]));
  }, [tree]);
  return useCallback(
    (project: string) => GROUP_PALETTE[(rank.get(project) ?? 0) % GROUP_PALETTE.length],
    [rank],
  );
}
