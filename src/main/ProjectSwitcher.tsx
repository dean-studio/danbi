import { useEffect, useMemo, useRef, useState } from "react";
import { ArrowRight } from "lucide-react";
import { useApp } from "@/state/store";
import { cn } from "@/lib/utils";
import { projectIconOf } from "@/components/ProjectIconPicker";

/** ⌘P 로 띄우는 프로젝트 빠른 전환 팔레트.
 *  - 사이드바 트리에서 프로젝트만 추출, 최근 활동 순으로 정렬.
 *  - 입력은 fuzzy match (subsequence) — 한국어/영어 동시 지원.
 *  - 키보드만으로 ↑↓ + Enter 로 전환, Esc 로 닫힘.
 *  - 단순 modal — 프로젝트 수가 보통 5~30 개라 가상화 불필요. */
export function ProjectSwitcher({
  open,
  onClose,
}: {
  open: boolean;
  onClose: () => void;
}) {
  const tree = useApp((s) => s.tree);
  const selectProject = useApp((s) => s.selectProject);
  const currentProject = useApp((s) => s.selection.project);

  const [query, setQuery] = useState("");
  const [active, setActive] = useState(0);
  const inputRef = useRef<HTMLInputElement | null>(null);

  useEffect(() => {
    if (!open) return;
    setQuery("");
    setActive(0);
    // input focus + select-all 다음 tick — modal mount 후 textarea/input
    // 들이 다시 blur 되는 것 방지.
    const id = requestAnimationFrame(() => {
      inputRef.current?.focus();
      inputRef.current?.select();
    });
    return () => cancelAnimationFrame(id);
  }, [open]);

  const allProjects = useMemo(() => {
    if (!tree) return [];
    return tree.projects.map((p) => p.name);
  }, [tree]);

  // 정렬: 알파벳순. 현재 프로젝트는 최하단 (사용자가 토글 흐름에서
  // 자기 자신을 다시 누를 일 적음).
  const ranked = useMemo(() => {
    const list = [...allProjects];
    list.sort((a, b) => {
      if (a === currentProject && b !== currentProject) return 1;
      if (b === currentProject && a !== currentProject) return -1;
      return a.localeCompare(b);
    });
    return list;
  }, [allProjects, currentProject]);

  const filtered = useMemo(() => {
    const q = query.trim().toLowerCase();
    if (!q) return ranked;
    return ranked.filter((name) => fuzzyMatch(name.toLowerCase(), q));
  }, [ranked, query]);

  useEffect(() => {
    setActive((i) => Math.min(i, Math.max(0, filtered.length - 1)));
  }, [filtered.length]);

  if (!open) return null;

  function commit(name: string) {
    selectProject(name);
    onClose();
  }

  function onKey(e: React.KeyboardEvent) {
    if (e.key === "Escape") {
      e.preventDefault();
      onClose();
      return;
    }
    if (e.key === "ArrowDown") {
      e.preventDefault();
      setActive((i) => Math.min(filtered.length - 1, i + 1));
      return;
    }
    if (e.key === "ArrowUp") {
      e.preventDefault();
      setActive((i) => Math.max(0, i - 1));
      return;
    }
    if (e.key === "Enter") {
      e.preventDefault();
      const name = filtered[active];
      if (name) commit(name);
    }
  }

  return (
    <div
      className="fixed inset-0 z-50 flex items-start justify-center bg-canvas/70 pt-[15vh] backdrop-blur-sm"
      onMouseDown={(e) => {
        if (e.target === e.currentTarget) onClose();
      }}
    >
      <div className="flex w-[480px] flex-col overflow-hidden rounded-lg border border-hairline bg-surface shadow-2xl shadow-black/40">
        <div className="border-b border-hairline px-4 py-3">
          <input
            ref={inputRef}
            type="text"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            onKeyDown={onKey}
            placeholder="프로젝트 이름 검색…"
            className="w-full bg-transparent text-[14px] text-ink placeholder:text-stone focus:outline-none"
          />
        </div>
        {filtered.length === 0 ? (
          <div className="px-4 py-6 text-center text-[12px] text-stone">
            매치되는 프로젝트가 없어요.
          </div>
        ) : (
          <ul className="max-h-[50vh] overflow-y-auto py-1">
            {filtered.map((name, idx) => (
              <ProjectSwitcherRow
                key={name}
                name={name}
                isActive={idx === active}
                isCurrent={name === currentProject}
                onHover={() => setActive(idx)}
                onSelect={() => commit(name)}
              />
            ))}
          </ul>
        )}
        <div className="flex items-center justify-between border-t border-hairline px-4 py-2 text-[10px] uppercase tracking-[0.4px] text-stone">
          <span>↑↓ 이동 · ⏎ 전환</span>
          <span>esc 닫기</span>
        </div>
      </div>
    </div>
  );
}

function ProjectSwitcherRow({
  name,
  isActive,
  isCurrent,
  onHover,
  onSelect,
}: {
  name: string;
  isActive: boolean;
  isCurrent: boolean;
  onHover: () => void;
  onSelect: () => void;
}) {
  const iconName = useApp((s) => s.cfg?.project_icons?.[name] ?? null);
  const Icon = projectIconOf(iconName);
  return (
    <li>
      <button
        type="button"
        onMouseEnter={onHover}
        onClick={onSelect}
        className={cn(
          "flex w-full items-center justify-between gap-3 px-4 py-2 text-left text-[13px] transition-colors",
          isActive
            ? "bg-surface-elevated text-on-dark"
            : "text-body hover:bg-surface-elevated/60",
        )}
      >
        <span className="flex min-w-0 items-center gap-2">
          <Icon size={14} className="shrink-0 text-stone" />
          <span className="truncate font-medium">{name}</span>
          {isCurrent && (
            <span className="rounded-xs bg-accent-blue-soft px-1 py-px text-[9px] font-medium uppercase leading-none tracking-[0.2px] text-accent-blue">
              현재
            </span>
          )}
        </span>
        {isActive && (
          <ArrowRight size={12} className="shrink-0 text-stone" />
        )}
      </button>
    </li>
  );
}

/** 부분문자열 fuzzy: query 의 각 char 가 target 에 순서대로 등장하면 매치.
 *  (cmd+P 류 팔레트의 표준 패턴) */
function fuzzyMatch(target: string, query: string): boolean {
  if (!query) return true;
  let i = 0;
  for (const ch of target) {
    if (ch === query[i]) {
      i += 1;
      if (i >= query.length) return true;
    }
  }
  return false;
}
