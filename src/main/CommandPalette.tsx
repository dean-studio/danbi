import { useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import {
  ChevronRight,
  FileText,
  Loader2,
  Search as SearchIcon,
  Sparkles,
  X,
} from "lucide-react";
import { ipc, type CommitSummary, type SearchHit } from "@/lib/ipc";
import { cn } from "@/lib/utils";
import { useApp } from "@/state/store";
import { projectIconOf } from "@/components/ProjectIconPicker";

/** ⌘K 통합 팔레트 — SearchPalette(문서) + ProjectSwitcher(프로젝트) +
 *  명령을 한 창에 합친다 (UI 개편 §4.7).
 *
 *  접두 문법:
 *   - 없음  → 문서 5 · 프로젝트 3 · 명령 3 혼합
 *   - `>`   → 명령만
 *   - `@`   → 프로젝트만 (⌘P 가 이 모드로 연다). Tab/↩ 로 범위 칩 고정
 *   - `#`   → daily 문서만
 *  범위 칩(`@프로젝트 ×`)이 잡히면 문서 결과는 그 프로젝트로 좁힌다.
 *  빈 입력은 최근 문서 5 · 최근 프로젝트 3 · 자주 쓰는 명령 3. */

export type PaletteCommand = {
  id: string;
  label: string;
  hint?: string;
  shortcut?: string;
  run: () => void;
  keywords?: string[];
};

export type PaletteMode = "mixed" | "project";

type Prefix = "mixed" | "command" | "project" | "daily";

type SearchState = "idle" | "local" | "full" | "ai-running" | "ai-done" | "ai-error";

/** 한 줄 결과. 종류별로 섹션에 묶고, 키보드 이동은 평탄한 인덱스로. */
type Row =
  | { kind: "doc"; key: string; project: string; domain: string; snippet?: string; mtime?: number | null; matched?: "keyword" | "semantic" }
  | { kind: "project"; key: string; name: string; current: boolean }
  | { kind: "command"; key: string; cmd: PaletteCommand };

const DOC_LIMIT_MIXED = 5;
const PROJECT_LIMIT_MIXED = 3;
const COMMAND_LIMIT_MIXED = 3;
const DOC_LIMIT_FOCUSED = 20;

export function CommandPalette({
  open,
  mode,
  onClose,
  commands,
  initialScope = null,
}: {
  open: boolean;
  mode: PaletteMode;
  onClose: () => void;
  commands: PaletteCommand[];
  /** 프로젝트 홈에서 열면 그 프로젝트를 범위 칩으로 미리 고정. */
  initialScope?: string | null;
}) {
  const tree = useApp((s) => s.tree);
  const selectDomain = useApp((s) => s.selectDomain);
  const selectProject = useApp((s) => s.selectProject);
  const currentProject = useApp((s) => s.selection.project);

  const [query, setQuery] = useState("");
  const [scope, setScope] = useState<string | null>(null);
  const [hits, setHits] = useState<SearchHit[]>([]);
  const [search, setSearch] = useState<SearchState>("idle");
  const [aiSummary, setAiSummary] = useState("");
  const [err, setErr] = useState<string | null>(null);
  const [activeIdx, setActiveIdx] = useState(0);
  const [commits, setCommits] = useState<CommitSummary[]>([]);
  const inputRef = useRef<HTMLInputElement | null>(null);
  const reqRef = useRef(0);

  // 열릴 때 초기화. ⌘P 는 `@` 를 미리 넣어 프로젝트 모드로.
  useEffect(() => {
    if (!open) return;
    setQuery(mode === "project" ? "@" : "");
    setScope(initialScope ?? null);
    setHits([]);
    setSearch("idle");
    setAiSummary("");
    setErr(null);
    setActiveIdx(0);
    const id = requestAnimationFrame(() => inputRef.current?.focus());
    // 최근 순 정렬·빈 입력 최근 목록의 재료. 200개면 32개 프로젝트의 최근
    // 활동을 덮기에 충분하고, 호출은 열 때 한 번.
    ipc
      .recentCommits(200)
      .then(setCommits)
      .catch(() => setCommits([]));
    return () => cancelAnimationFrame(id);
  }, [open, mode, initialScope]);

  const { prefix, term } = useMemo(() => parsePrefix(query), [query]);

  // 프로젝트 최근 순 + 최근 문서 — 커밋 요약의 `project/domain` 토큰에서.
  const recency = useMemo(() => commitRecency(commits), [commits]);
  const allProjects = useMemo(
    () => tree?.projects.map((p) => p.name) ?? [],
    [tree],
  );
  const rankedProjects = useMemo(() => {
    const list = [...allProjects];
    list.sort((a, b) => {
      const ra = recency.projects.get(a) ?? 0;
      const rb = recency.projects.get(b) ?? 0;
      if (ra !== rb) return rb - ra;
      return a.localeCompare(b);
    });
    return list;
  }, [allProjects, recency]);

  // 문서 검색 — Tier 1(즉시) → Tier 2(tantivy, 150ms 디바운스). 명령·
  // 프로젝트 전용 모드에선 돌리지 않는다.
  const wantsDocs = prefix === "mixed" || prefix === "daily";
  useEffect(() => {
    if (!open) return;
    const q = term.trim();
    if (!wantsDocs || q.length < 1) {
      setHits([]);
      setSearch("idle");
      setAiSummary("");
      return;
    }
    const myId = ++reqRef.current;
    const limit = scope || prefix === "daily" ? 60 : 30;
    (async () => {
      try {
        const res = await ipc.searchLocal(q, limit);
        if (myId !== reqRef.current) return;
        setHits(res);
        setSearch("local");
        setAiSummary("");
        setActiveIdx(0);
      } catch {
        /* Tier 2 follows anyway */
      }
    })();
    const t = setTimeout(async () => {
      try {
        const res = await ipc.searchFull(q, limit);
        if (myId !== reqRef.current) return;
        setHits((prev) => mergeHits(res, prev));
        setSearch("full");
        setActiveIdx(0);
      } catch (e) {
        if (myId !== reqRef.current) return;
        console.error("[danbi] searchFull:", e);
      }
    }, 150);
    return () => clearTimeout(t);
  }, [term, open, wantsDocs, scope, prefix]);

  async function reRankWithAi() {
    const q = term.trim();
    if (!wantsDocs || q.length < 2) return;
    setSearch("ai-running");
    setErr(null);
    try {
      const res = await ipc.searchVault(q);
      setHits(res.hits);
      setAiSummary(res.summary);
      setSearch("ai-done");
      setActiveIdx(0);
    } catch (e) {
      setSearch("ai-error");
      setErr(String(e));
    }
  }

  // ---- 섹션 조립 ----
  const sections = useMemo<{ title: string; rows: Row[] }[]>(() => {
    const q = term.trim().toLowerCase();
    const out: { title: string; rows: Row[] }[] = [];

    const projectRows = (limit: number): Row[] => {
      const list = q
        ? rankedProjects.filter((n) => fuzzyMatch(n.toLowerCase(), q))
        : rankedProjects;
      return list.slice(0, limit).map((name) => ({
        kind: "project",
        key: `p:${name}`,
        name,
        current: name === currentProject,
      }));
    };
    const commandRows = (limit: number): Row[] => {
      const list = q
        ? commands.filter((c) =>
            [c.label, c.hint ?? "", ...(c.keywords ?? [])].some((s) =>
              fuzzyMatch(s.toLowerCase(), q),
            ),
          )
        : commands;
      return list.slice(0, limit).map((cmd) => ({
        kind: "command",
        key: `c:${cmd.id}`,
        cmd,
      }));
    };
    const docRows = (limit: number): Row[] => {
      let list = hits;
      if (scope) list = list.filter((h) => h.project === scope);
      if (prefix === "daily") list = list.filter((h) => isDaily(h.domain));
      return list.slice(0, limit).map((h) => ({
        kind: "doc",
        key: `d:${h.project}/${h.domain}`,
        project: h.project,
        domain: h.domain,
        snippet: h.snippet,
        mtime: recency.docs.get(`${h.project}/${h.domain}`) ?? null,
        matched: search === "ai-done" ? "semantic" : "keyword",
      }));
    };
    const recentDocRows = (limit: number): Row[] => {
      const rows: Row[] = [];
      for (const [key, ts] of recency.docList) {
        const [project, ...rest] = key.split("/");
        const domain = rest.join("/");
        if (!project || !domain) continue;
        if (scope && project !== scope) continue;
        if (prefix === "daily" && !isDaily(domain)) continue;
        if (!projectExists(tree, project, domain)) continue;
        rows.push({ kind: "doc", key: `d:${key}`, project, domain, mtime: ts });
        if (rows.length >= limit) break;
      }
      return rows;
    };

    if (prefix === "command") {
      out.push({ title: "명령", rows: commandRows(50) });
      return out;
    }
    if (prefix === "project") {
      out.push({ title: "프로젝트", rows: projectRows(50) });
      return out;
    }
    if (prefix === "daily") {
      out.push({
        title: q ? "Daily" : "최근 Daily",
        rows: q ? docRows(DOC_LIMIT_FOCUSED) : recentDocRows(10),
      });
      return out;
    }
    // mixed
    if (!q) {
      out.push({ title: "최근 문서", rows: recentDocRows(5) });
      out.push({ title: "최근 프로젝트", rows: projectRows(3) });
      out.push({ title: "명령", rows: commandRows(3) });
    } else {
      out.push({ title: "문서", rows: docRows(scope ? DOC_LIMIT_FOCUSED : DOC_LIMIT_MIXED) });
      if (!scope) out.push({ title: "프로젝트", rows: projectRows(PROJECT_LIMIT_MIXED) });
      out.push({ title: "명령", rows: commandRows(COMMAND_LIMIT_MIXED) });
    }
    return out.filter((s) => s.rows.length > 0);
  }, [term, prefix, scope, hits, rankedProjects, commands, currentProject, recency, tree, search]);

  const flat = useMemo(() => sections.flatMap((s) => s.rows), [sections]);
  useEffect(() => {
    setActiveIdx((i) => Math.min(i, Math.max(0, flat.length - 1)));
  }, [flat.length]);

  function pinScope(name: string) {
    setScope(name);
    setQuery("");
    setActiveIdx(0);
    inputRef.current?.focus();
  }

  function activate(row: Row) {
    if (row.kind === "doc") {
      selectDomain(row.project, row.domain);
      onClose();
    } else if (row.kind === "project") {
      // `@` 모드에서 프로젝트를 고르면 ⌘P 호환으로 그 프로젝트 홈으로.
      // 혼합 모드에서 입력이 남아 있으면 범위 칩으로만 쓰는 게 자연스럽다.
      if (prefix === "project" || !term.trim()) {
        selectProject(row.name);
        onClose();
      } else {
        pinScope(row.name);
      }
    } else {
      onClose();
      row.cmd.run();
    }
  }

  useEffect(() => {
    if (!open) return;
    function onKey(e: KeyboardEvent) {
      if (e.key === "Escape") {
        e.preventDefault();
        onClose();
        return;
      }
      if (e.key === "ArrowDown") {
        e.preventDefault();
        setActiveIdx((i) => Math.min(flat.length - 1, i + 1));
        return;
      }
      if (e.key === "ArrowUp") {
        e.preventDefault();
        setActiveIdx((i) => Math.max(0, i - 1));
        return;
      }
      if (e.key === "Tab") {
        // 첫 프로젝트 후보를 범위 칩으로 (캡처 팝업의 Tab 제안과 같은 느낌).
        e.preventDefault();
        const candidate =
          (flat[activeIdx]?.kind === "project" && (flat[activeIdx] as Extract<Row, { kind: "project" }>).name) ||
          (flat.find((r) => r.kind === "project") as Extract<Row, { kind: "project" }> | undefined)?.name ||
          (prefix === "project" || prefix === "mixed"
            ? rankedProjects.find((n) => fuzzyMatch(n.toLowerCase(), term.trim().toLowerCase()))
            : undefined);
        if (candidate) pinScope(candidate);
        return;
      }
      if (e.key === "Backspace" && query === "" && scope) {
        e.preventDefault();
        setScope(null);
        return;
      }
      if (e.key === "Enter") {
        // 한글 조합 중 Enter 는 IME 몫 (CommandBar 와 같은 처리). WebKit 은
        // 조합 확정 Enter 를 keyCode 229 로 한 번 더 보낸다.
        if (e.isComposing || e.keyCode === 229) return;
        e.preventDefault();
        // ⌘↵ = AI 재정렬 — 일반 Enter 분기보다 먼저.
        if (e.metaKey || e.ctrlKey) {
          reRankWithAi();
          return;
        }
        const row = flat[activeIdx];
        if (row) activate(row);
      }
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open, flat, activeIdx, onClose, query, scope, prefix, term, rankedProjects]);

  if (!open) return null;

  const aiRunning = search === "ai-running";
  const docCount = flat.filter((r) => r.kind === "doc").length;
  let flatIdx = 0;

  return (
    <div
      className="fixed inset-0 z-40 flex items-start justify-center bg-canvas/70 pt-24 backdrop-blur-sm"
      onMouseDown={(e) => {
        if (e.target === e.currentTarget) onClose();
      }}
    >
      <div className="flex w-full max-w-[640px] flex-col overflow-hidden rounded-lg border border-hairline-strong bg-surface-card">
        {/* 입력 줄: 아이콘 · 범위 칩 · 입력 · AI · 닫기 */}
        <div className="flex items-center gap-2 border-b border-hairline px-3 py-2.5">
          <SearchIcon size={13} className="shrink-0 text-mute" />
          {scope && (
            <span className="inline-flex h-6 shrink-0 items-center gap-1 rounded-full bg-surface-elevated pl-2 pr-1 text-[12px] text-on-dark">
              @{scope}
              <button
                type="button"
                onClick={() => {
                  setScope(null);
                  inputRef.current?.focus();
                }}
                className="grid h-4 w-4 place-items-center rounded-full text-stone hover:text-on-dark"
                title="범위 해제"
              >
                <X size={10} />
              </button>
            </span>
          )}
          <input
            ref={inputRef}
            type="text"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder={
              scope
                ? `${scope} 안에서 찾기…`
                : "검색 또는 명령…  (> 명령 · @ 프로젝트 · # daily)"
            }
            className="min-w-0 flex-1 bg-transparent text-[14px] text-ink outline-none placeholder:text-stone"
          />
          {wantsDocs && (
            <button
              onClick={reRankWithAi}
              disabled={term.trim().length < 2 || aiRunning}
              className={cn(
                "inline-flex h-7 shrink-0 items-center gap-1 rounded-md border px-2 text-[11px] font-medium transition-colors",
                search === "ai-done"
                  ? "border-hairline-strong bg-surface-elevated text-on-dark"
                  : "border-hairline bg-surface-elevated text-body hover:border-hairline-strong hover:text-on-dark",
                (term.trim().length < 2 || aiRunning) && "opacity-50",
              )}
              title="AI 로 재정렬 (⌘↩)"
            >
              {aiRunning ? (
                <Loader2 size={10} className="animate-spin" />
              ) : (
                <Sparkles size={10} />
              )}
              AI
            </button>
          )}
          <button
            onClick={onClose}
            className="grid h-6 w-6 shrink-0 place-items-center rounded-md text-stone transition-colors hover:text-on-dark"
            title="닫기 (Esc)"
          >
            <X size={11} />
          </button>
        </div>

        <div className="max-h-[400px] overflow-y-auto py-1">
          {err && (
            <div className="px-3 py-2 font-mono text-[11px] text-accent-red">
              {err}
            </div>
          )}
          {!err && flat.length === 0 && (
            <div className="px-3 py-5 text-caption-sm text-stone">
              {term.trim()
                ? "결과가 없어요. 다른 말로 찾거나 ⌘↩ 로 AI 재정렬을 해 보세요."
                : "아직 기록이 없어요. 프로젝트를 고르거나 명령을 입력해 보세요."}
            </div>
          )}
          {sections.map((sec) => (
            <div key={sec.title} className="pb-1">
              <div className="px-3 pb-1 pt-2 text-caption-sm uppercase tracking-[0.4px] text-stone">
                {sec.title}
              </div>
              {sec.rows.map((row) => {
                const idx = flatIdx++;
                const active = idx === activeIdx;
                return (
                  <PaletteRow
                    key={row.key}
                    row={row}
                    active={active}
                    onHover={() => setActiveIdx(idx)}
                    onSelect={() => activate(row)}
                  />
                );
              })}
            </div>
          ))}
        </div>

        <div className="flex items-center justify-between border-t border-hairline bg-surface px-3 py-1.5 text-caption-sm">
          <div className="truncate text-stone">
            {search === "idle" && !term.trim() && "최근 항목"}
            {search === "idle" && term.trim() && !wantsDocs && (prefix === "command" ? "명령" : "프로젝트")}
            {search === "local" && `즉시 검색 · ${docCount}건`}
            {search === "full" && `전체 본문 검색 · ${docCount}건`}
            {search === "ai-running" && "AI 가 재정렬 중…"}
            {search === "ai-done" && (aiSummary || `AI 재정렬 · ${docCount}건`)}
            {search === "ai-error" && <span className="text-accent-red">AI 실패</span>}
          </div>
          <div className="flex shrink-0 items-center gap-2 text-stone">
            <Key>↑↓</Key> 이동
            <Key>↩</Key> 열기
            <Key>⇥</Key> 범위
            {wantsDocs && (
              <>
                <Key>⌘↩</Key> AI
              </>
            )}
            <Key>esc</Key>
          </div>
        </div>
      </div>
    </div>
  );
}

function Key({ children }: { children: ReactNode }) {
  return (
    <kbd className="rounded-xs border border-hairline bg-surface-elevated px-1 font-mono text-[10px]">
      {children}
    </kbd>
  );
}

function PaletteRow({
  row,
  active,
  onHover,
  onSelect,
}: {
  row: Row;
  active: boolean;
  onHover: () => void;
  onSelect: () => void;
}) {
  const iconName = useApp((s) =>
    row.kind === "project" ? s.cfg?.project_icons?.[row.name] ?? null : null,
  );
  const rowClass = cn(
    "flex w-full items-center gap-3 px-3 py-1.5 text-left transition-colors",
    active ? "bg-surface-elevated text-on-dark" : "text-body hover:bg-surface-elevated/60",
  );
  if (row.kind === "doc") {
    return (
      <button type="button" onMouseEnter={onHover} onClick={onSelect} className={rowClass}>
        <FileText size={12} className="shrink-0 text-mute" />
        <div className="min-w-0 flex-1">
          <div className="flex items-baseline gap-1.5 text-[13px]">
            <span className="shrink-0 text-mute">{row.project}</span>
            <span className="text-stone">/</span>
            <span className="truncate font-mono text-ink">{row.domain}</span>
            {row.matched && row.snippet && (
              <span className="shrink-0 rounded-xs bg-surface-card px-1 py-px text-[9px] uppercase tracking-[0.2px] text-mute">
                {row.matched === "semantic" ? "의미" : "키워드"}
              </span>
            )}
            {row.mtime ? (
              <span className="ml-auto shrink-0 text-caption-sm text-stone">
                {timeAgo(row.mtime)}
              </span>
            ) : null}
          </div>
          {row.snippet && (
            <div className="mt-0.5 truncate text-caption-sm leading-[1.5] text-body">
              {row.snippet}
            </div>
          )}
        </div>
      </button>
    );
  }
  if (row.kind === "project") {
    const Icon = projectIconOf(iconName);
    return (
      <button type="button" onMouseEnter={onHover} onClick={onSelect} className={rowClass}>
        <Icon size={13} className="shrink-0 text-stone" />
        <span className="min-w-0 flex-1 truncate text-[13px] font-medium">{row.name}</span>
        {row.current && (
          <span className="shrink-0 rounded-xs bg-surface-card px-1 py-px text-[9px] uppercase tracking-[0.2px] text-mute">
            현재
          </span>
        )}
        {active && <ChevronRight size={12} className="shrink-0 text-stone" />}
      </button>
    );
  }
  return (
    <button type="button" onMouseEnter={onHover} onClick={onSelect} className={rowClass}>
      <span className="w-3 shrink-0 text-center font-mono text-[12px] text-stone">&gt;</span>
      <span className="min-w-0 flex-1 truncate text-[13px]">
        {row.cmd.label}
        {row.cmd.hint && <span className="ml-2 text-caption-sm text-mute">{row.cmd.hint}</span>}
      </span>
      {row.cmd.shortcut && <Key>{row.cmd.shortcut}</Key>}
    </button>
  );
}

// ---- helpers ----

function parsePrefix(raw: string): { prefix: Prefix; term: string } {
  const s = raw.trimStart();
  if (s.startsWith(">")) return { prefix: "command", term: s.slice(1) };
  if (s.startsWith("@")) return { prefix: "project", term: s.slice(1) };
  if (s.startsWith("#")) return { prefix: "daily", term: s.slice(1) };
  return { prefix: "mixed", term: raw };
}

function isDaily(domain: string): boolean {
  return /^daily\/\d{4}-\d{2}-\d{2}\.md$/.test(domain) || domain.startsWith("daily/");
}

function projectExists(
  tree: ReturnType<typeof useApp.getState>["tree"],
  project: string,
  domain: string,
): boolean {
  if (!tree) return true;
  const p = tree.projects.find((x) => x.name === project);
  if (!p) return false;
  if (p.domains.some((d) => d.name === domain)) return true;
  return p.subfolders.some(
    (f) =>
      f.domains.some((d) => d.name === domain) ||
      f.subfolders.some((g) => g.domains.some((d) => d.name === domain)),
  );
}

/** 커밋 요약 `danbi: <op> · <project>/<domain> · …` 에서 프로젝트·문서의
 *  마지막 활동 시각을 뽑는다. `(pre)` 스냅샷은 건너뛴다. */
function commitRecency(commits: CommitSummary[]): {
  projects: Map<string, number>;
  docs: Map<string, number>;
  /** 최근 순 [key, ts]. */
  docList: [string, number][];
} {
  const projects = new Map<string, number>();
  const docs = new Map<string, number>();
  for (const c of commits) {
    if (c.summary.includes("(pre)")) continue;
    const parts = c.summary.split("·").map((s) => s.trim());
    const loc = parts.find((p) => /^[^/]+\/.+\.md$/.test(p));
    if (!loc) continue;
    const ts = c.ts * 1000;
    const project = loc.split("/")[0];
    if (!projects.has(project)) projects.set(project, ts);
    if (!docs.has(loc)) docs.set(loc, ts);
  }
  const docList = [...docs.entries()].sort((a, b) => b[1] - a[1]);
  return { projects, docs, docList };
}

function timeAgo(ms: number): string {
  const diff = Math.max(0, Date.now() - ms);
  const m = Math.floor(diff / 60_000);
  if (m < 1) return "방금";
  if (m < 60) return `${m}m`;
  const h = Math.floor(m / 60);
  if (h < 24) return `${h}h`;
  return `${Math.floor(h / 24)}d`;
}

/** 부분문자열 fuzzy: query 의 각 char 가 target 에 순서대로 등장하면 매치. */
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

/** Tier 2(tantivy) 를 앞에, Tier 1 에만 있던 파일명 매치를 뒤에. */
function mergeHits(primary: SearchHit[], fallback: SearchHit[]): SearchHit[] {
  const seen = new Set<string>();
  const out: SearchHit[] = [];
  for (const h of [...primary, ...fallback]) {
    const key = `${h.project}/${h.domain}`;
    if (!seen.has(key)) {
      seen.add(key);
      out.push(h);
    }
  }
  return out;
}
