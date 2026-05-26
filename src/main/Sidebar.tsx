import { useEffect, useRef, useState } from "react";
import {
  ArrowDown,
  ArrowUp,
  ArrowUpCircle,
  Bell,
  ChevronDown,
  ChevronRight,
  FileText,
  Folder,
  FolderOpen,
  FolderPlus,
  GripVertical,
  Inbox,
  Layers,
  Network,
  Plus,
  RefreshCw,
  Search as SearchIcon,
  Settings as SettingsIcon,
  Trash2,
} from "lucide-react";
import { writeText as clipboardWriteText } from "@tauri-apps/plugin-clipboard-manager";
import { ipc, type ProjectGroup, type TriggerKind } from "@/lib/ipc";
import { applyPendingUpdate, dismissUpdatePill } from "@/lib/updater";
import { ContextMenu, type MenuItem } from "@/components/ContextMenu";
import { shortModel } from "@/components/ModelPicker";
import { projectIconOf } from "@/components/ProjectIconPicker";
import { projectColorVars } from "@/components/ProjectColorPicker";
import { Wordmark } from "@/components/Wordmark";
import { cn } from "@/lib/utils";
import { useApp } from "@/state/store";

type CtxState =
  | { kind: "none" }
  | { kind: "project"; x: number; y: number; project: string }
  | { kind: "domain"; x: number; y: number; project: string; domain: string }
  | { kind: "folder"; x: number; y: number; project: string; folder: string };

/** Recursive shape of a sub-folder as carried in the vault tree. The
 *  backend currently caps depth at 2, but the type itself is recursive
 *  so the renderer doesn't have to special-case the inner level. */
type SubfolderShape = {
  name: string;
  domains: { name: string; bytes: number; title?: string | null }[];
  subfolders: SubfolderShape[];
};

export function Sidebar({
  onAddProject,
  onAddProjectToGroup,
  onAddDomain,
  onRenameProject,
  onDeleteProject,
  onRenameDomain,
  onDeleteDomain,
  onOpenSettings,
  onOpenSearch,
  onOpenGraph,
  onOpenReviews,
  reviewCount,
  onOpenTrash,
  trashHasUnseen,
  onCopyMcpInstall,
  onCopyClaudeMdTemplate,
  onChangeProjectIcon,
  onChangeProjectColor,
  onMarkProjectRead,
  onAddDomainInFolder,
  onAddFolder,
  onAddSubFolder,
  onRenameFolder,
  onDeleteFolder,
  onMoveDomain,
  onRefreshTree,
}: {
  onAddProject: () => void;
  /** Like `onAddProject` but pre-targets a group so the new project lands
   *  inside it. Triggered from the group header's `+` button. */
  onAddProjectToGroup: (groupId: string) => void;
  onAddDomain: (project: string) => void;
  /** Like `onAddDomain` but pre-targets a sub-folder so the new file
   *  lands inside it. Used by the folder context menu's "이 폴더에 새
   *  파일…" entry. */
  onAddDomainInFolder: (project: string, folder: string) => void;
  onRenameProject: (project: string) => void;
  onDeleteProject: (project: string) => void;
  onRenameDomain: (project: string, domain: string) => void;
  onDeleteDomain: (project: string, domain: string) => void;
  onOpenSettings: () => void;
  onOpenSearch: () => void;
  onOpenGraph: () => void;
  onOpenReviews: () => void;
  reviewCount: number;
  onOpenTrash: () => void;
  /** 마지막 본 시점 이후 새 휴지통 항목이 있는지. true 면 사이드바
   *  Trash2 아이콘 옆에 미시 dot 으로 알림. 모달 한 번 열면 사라짐. */
  trashHasUnseen: boolean;
  onCopyMcpInstall: (project: string) => void;
  onCopyClaudeMdTemplate: (project: string) => void;
  onChangeProjectIcon: (project: string) => void;
  onChangeProjectColor: (project: string) => void;
  onMarkProjectRead: (project: string) => void;
  onAddFolder: (project: string) => void;
  /** Create a folder NESTED inside `parentFolder` (e.g. parent="daily"
   *  → "daily/2026-01"). Triggered from the folder context menu's
   *  "이 안에 새 폴더…" entry. */
  onAddSubFolder: (project: string, parentFolder: string) => void;
  onRenameFolder: (project: string, folder: string) => void;
  onDeleteFolder: (project: string, folder: string) => void;
  /** Move a domain into the given sub-folder (null = move to project root). */
  onMoveDomain: (
    project: string,
    domain: string,
    toFolder: string | null,
  ) => void;
  /** Force a vault tree refetch. Bound to the sidebar refresh button so
   *  users can manually re-sync after an external change (e.g. a file
   *  copied via Finder, or an MCP write from another agent). */
  onRefreshTree: () => void;
}) {
  const tree = useApp((s) => s.tree);
  const cfg = useApp((s) => s.cfg);
  const selection = useApp((s) => s.selection);
  const selectProject = useApp((s) => s.selectProject);
  const selectDomain = useApp((s) => s.selectDomain);
  const clearSelection = useApp((s) => s.clearSelection);
  const groups = useApp((s) => s.groups);
  const setGroups = useApp((s) => s.setGroups);
  const projectUpdates = useApp((s) => s.projectUpdates);
  const clearProjectUpdate = useApp((s) => s.clearProjectUpdate);
  const domainUpdates = useApp((s) => s.domainUpdates);

  // Settings 토글로 사용자가 끌 수 있는 visibility 힌트들. undefined 또는
  // true 일 때만 표시 (legacy 사용자는 기본 ON 유지).
  const showProjectCount = cfg?.appearance?.unseen_project_count !== false;

  // Project header badge — count of CHANGED files inside the project,
  // including nested folders. We prefer the file-level signal over the
  // git-commit count (`projectUpdates`) so the project badge stays in
  // sync with the per-folder badges (which read from domainUpdates).
  // Falls back to projectUpdates only when there are no per-domain dots
  // (e.g. fresh git history but no mtime drift).
  const projectChangeCount = (project: string): number => {
    if (!showProjectCount) return 0;
    let n = 0;
    const prefix = `${project}/`;
    for (const k in domainUpdates) {
      if (k.startsWith(prefix)) n += 1;
    }
    return n > 0 ? n : projectUpdates[project] ?? 0;
  };

  const [expanded, setExpanded] = useState<Record<string, boolean>>({});
  const [ctx, setCtx] = useState<CtxState>({ kind: "none" });
  const [dragProject, setDragProject] = useState<string | null>(null);
  const [dropHint, setDropHint] = useState<string | null>(null);

  // Sidebar width — drag-resizable. Persisted in localStorage so the
  // user's preferred layout survives reloads. Defaults to 256px (the
  // old fixed `w-64`). Clamped to [200, 480] so UX doesn't fall apart.
  const SIDEBAR_WIDTH_KEY = "danbi.sidebarWidth";
  const SIDEBAR_MIN = 200;
  const SIDEBAR_MAX = 480;
  const SIDEBAR_DEFAULT = 256;
  const [sidebarWidth, setSidebarWidth] = useState<number>(() => {
    if (typeof window === "undefined") return SIDEBAR_DEFAULT;
    const saved = window.localStorage.getItem(SIDEBAR_WIDTH_KEY);
    const n = saved ? parseInt(saved, 10) : NaN;
    return Number.isFinite(n)
      ? Math.min(SIDEBAR_MAX, Math.max(SIDEBAR_MIN, n))
      : SIDEBAR_DEFAULT;
  });
  const [resizing, setResizing] = useState(false);

  // Persist on EVERY width change, not just mouseup. This guarantees
  // restoration even if:
  //  - the user drags off the window and the synthetic mouseup is lost
  //  - the OS force-quits the app mid-drag
  //  - cmd+Q happens before the drag ends
  // Debounced 100ms so we don't hammer localStorage on every mousemove.
  useEffect(() => {
    const t = window.setTimeout(() => {
      window.localStorage.setItem(SIDEBAR_WIDTH_KEY, String(sidebarWidth));
    }, 100);
    return () => window.clearTimeout(t);
  }, [sidebarWidth]);

  // Last-resort flush on tab/window close — covers the case where the
  // debounce timer hasn't fired yet.
  useEffect(() => {
    function onLeave() {
      window.localStorage.setItem(SIDEBAR_WIDTH_KEY, String(sidebarWidth));
    }
    window.addEventListener("beforeunload", onLeave);
    window.addEventListener("pagehide", onLeave);
    return () => {
      window.removeEventListener("beforeunload", onLeave);
      window.removeEventListener("pagehide", onLeave);
    };
  }, [sidebarWidth]);

  useEffect(() => {
    if (!resizing) return;
    function onMove(e: MouseEvent) {
      const next = Math.min(
        SIDEBAR_MAX,
        Math.max(SIDEBAR_MIN, e.clientX),
      );
      setSidebarWidth(next);
    }
    function onUp() {
      setResizing(false);
    }
    document.body.style.cursor = "col-resize";
    document.body.style.userSelect = "none";
    window.addEventListener("mousemove", onMove);
    window.addEventListener("mouseup", onUp);
    return () => {
      document.body.style.cursor = "";
      document.body.style.userSelect = "";
      window.removeEventListener("mousemove", onMove);
      window.removeEventListener("mouseup", onUp);
    };
  }, [resizing]);

  const resetSidebarWidth = () => {
    setSidebarWidth(SIDEBAR_DEFAULT);
  };
  // React state updates are batched/async, so the first dragover right
  // after dragstart can still see `dragProject === null`. If that call
  // doesn't preventDefault, the browser rejects the subsequent drop
  // entirely. We mirror the dragged name in a ref so the dragover
  // handlers can check it synchronously on that very first tick.
  const dragProjectRef = useRef<string | null>(null);
  // Inline editors — Tauri webview blocks window.prompt/confirm so we
  // drive create/rename via actual <input>s instead.
  const [creatingGroup, setCreatingGroup] = useState(false);
  const [newGroupLabel, setNewGroupLabel] = useState("");
  const [renamingGroupId, setRenamingGroupId] = useState<string | null>(null);
  const [renameDraft, setRenameDraft] = useState("");
  const [pendingDelete, setPendingDelete] = useState<string | null>(null);

  function toggle(project: string) {
    setExpanded((m) => {
      const next = { ...m, [project]: !m[project] };
      // 펼치는 순간 (그리고 아직 캐시 없을 때) 백그라운드로 journal view
      // 가져와서 daily/*.md 옆 chip 데이터를 채운다. 추가 IPC 한 번이지만
      // 사용자가 그 프로젝트를 보고 있는 시점이라 자연스럽게 묻힘.
      if (next[project] && !useApp.getState().projectJournalCache[project]) {
        ipc
          .projectJournalView(project)
          .then((v) => useApp.getState().setProjectJournal(project, v))
          .catch(() => {});
      }
      return next;
    });
  }

  /** Which group does a given project belong to? `null` = Ungrouped. */
  function groupOf(project: string): string | null {
    for (const g of groups) {
      if (g.projects.includes(project)) return g.id;
    }
    return null;
  }

  /** Project names that have been placed into at least one group. The
   *  remainder are rendered in the top-level Ungrouped section. */
  const groupedNames = new Set<string>(groups.flatMap((g) => g.projects));
  const ungrouped = (tree?.projects ?? [])
    .map((p) => p.name)
    .filter((n) => !groupedNames.has(n));

  async function persistGroups(next: ProjectGroup[]) {
    try {
      const saved = await ipc.groupsSet(next);
      setGroups(saved);
    } catch (e) {
      console.error(e);
    }
  }

  /** Parse a "<project>::<domain>" payload from a domain dataTransfer
   *  drop and dispatch the move via the parent's onMoveDomain. The dest
   *  folder is fully-qualified (e.g. "daily/2026-01"); pass null to drop
   *  back to the project root. Cross-project moves are silently ignored
   *  for now — backend doesn't support that and the UX would be
   *  surprising. */
  function dispatchDomainDrop(
    destProject: string,
    destFolder: string | null,
    payload: string,
  ) {
    const idx = payload.indexOf("::");
    if (idx < 0) return;
    const srcProject = payload.slice(0, idx);
    const srcDomain = payload.slice(idx + 2);
    if (srcProject !== destProject) return;
    // Same source folder = no-op.
    const slash = srcDomain.lastIndexOf("/");
    const currentFolder = slash > 0 ? srcDomain.slice(0, slash) : null;
    if ((currentFolder ?? null) === (destFolder ?? null)) return;
    onMoveDomain(destProject, srcDomain, destFolder);
  }

  /** Move a project into the given destination group (null = Ungrouped).
   *  If `beforeProject` is set, insert before that project inside the
   *  destination; otherwise append. A project only lives in one group at
   *  a time, so we pull it out of every group first. */
  function moveProject(
    project: string,
    destGroupId: string | null,
    beforeProject: string | null,
  ) {
    const cleaned: ProjectGroup[] = groups.map((g) => ({
      ...g,
      projects: g.projects.filter((p) => p !== project),
    }));
    if (destGroupId === null) {
      // Ungrouped — already removed everywhere, nothing else to do.
      persistGroups(cleaned);
      return;
    }
    const next = cleaned.map((g) => {
      if (g.id !== destGroupId) return g;
      const projects = [...g.projects];
      if (beforeProject && projects.includes(beforeProject)) {
        const idx = projects.indexOf(beforeProject);
        projects.splice(idx, 0, project);
      } else {
        projects.push(project);
      }
      return { ...g, projects };
    });
    persistGroups(next);
  }

  /** Move a group up or down in the sidebar order. dir = -1 (up) or +1
   *  (down). Edges (already at top/bottom) are silent no-ops. */
  function moveGroup(id: string, dir: -1 | 1) {
    const idx = groups.findIndex((g) => g.id === id);
    if (idx < 0) return;
    const next = idx + dir;
    if (next < 0 || next >= groups.length) return;
    const reordered = [...groups];
    const [g] = reordered.splice(idx, 1);
    reordered.splice(next, 0, g);
    persistGroups(reordered);
  }

  /** Move a project up/down inside its own group. dir = -1 (up) / +1
   *  (down). No-op if the project is ungrouped or already at the edge. */
  function moveProjectWithinGroup(project: string, dir: -1 | 1) {
    const gIdx = groups.findIndex((g) => g.projects.includes(project));
    if (gIdx < 0) return;
    const g = groups[gIdx];
    const pIdx = g.projects.indexOf(project);
    const target = pIdx + dir;
    if (target < 0 || target >= g.projects.length) return;
    const projects = [...g.projects];
    const [p] = projects.splice(pIdx, 1);
    projects.splice(target, 0, p);
    const next = groups.map((x, i) =>
      i === gIdx ? { ...x, projects } : x,
    );
    persistGroups(next);
  }

  /** Lookup helper: does this project sit inside a group, and if so,
   *  what's its index + the group size? Used by the context menu to
   *  decide whether ↑/↓ items are enabled. */
  function projectGroupPosition(
    project: string,
  ): { idx: number; size: number } | null {
    for (const g of groups) {
      const i = g.projects.indexOf(project);
      if (i >= 0) return { idx: i, size: g.projects.length };
    }
    return null;
  }

  function commitNewGroup() {
    const label = newGroupLabel.trim();
    setCreatingGroup(false);
    setNewGroupLabel("");
    if (!label) return;
    const id = `g-${Date.now().toString(36)}`;
    persistGroups([
      ...groups,
      { id, label, projects: [], collapsed: false },
    ]);
  }

  function startRenameGroup(id: string) {
    const g = groups.find((x) => x.id === id);
    if (!g) return;
    setRenamingGroupId(id);
    setRenameDraft(g.label);
  }

  function commitRename() {
    if (!renamingGroupId) return;
    const label = renameDraft.trim();
    const id = renamingGroupId;
    setRenamingGroupId(null);
    setRenameDraft("");
    if (!label) return;
    persistGroups(
      groups.map((x) => (x.id === id ? { ...x, label } : x)),
    );
  }

  function confirmDelete(id: string) {
    // Toggle — clicking delete twice in a row actually removes the group.
    if (pendingDelete === id) {
      persistGroups(groups.filter((x) => x.id !== id));
      setPendingDelete(null);
    } else {
      setPendingDelete(id);
    }
  }

  function toggleGroupCollapsed(id: string) {
    persistGroups(
      groups.map((x) =>
        x.id === id ? { ...x, collapsed: !x.collapsed } : x,
      ),
    );
  }

  // ---- Path copy helpers ----
  // The vault root is the source of truth for absolute paths. Everything
  // a user might want to copy is some prefix of `<vault>/Projects/<...>`.
  const vaultRoot = tree?.vault_path ?? null;

  /** Build the absolute filesystem path for a project / folder / domain
   *  and copy it to the clipboard. Falls back silently if vault path is
   *  unavailable (shouldn't happen at runtime, but keeps the menu safe). */
  const copyPath = async (segments: string[]) => {
    if (!vaultRoot) return;
    const path = [vaultRoot, "Projects", ...segments].join("/");
    try {
      await clipboardWriteText(path);
    } catch {
      /* clipboard API unavailable — ignore */
    }
  };

  /** Build the vault-relative path ("dean_works_agent/daily/2026-05/17.md")
   *  for a domain / folder / project. Useful for pasting into MCP tool
   *  arguments where you only need the relative form. */
  const copyRelativePath = async (segments: string[]) => {
    try {
      await clipboardWriteText(segments.join("/"));
    } catch {
      /* ignore */
    }
  };

  /** Open Finder with the given path selected. For files this highlights
   *  the file inside its parent folder; for folders it opens the folder
   *  itself. Falls back silently if the vault root isn't ready or the
   *  plugin call fails (sandbox / unsupported platform). */
  const revealInFinder = async (segments: string[]) => {
    if (!vaultRoot) return;
    const path = [vaultRoot, "Projects", ...segments].join("/");
    try {
      const { revealItemInDir } = await import("@tauri-apps/plugin-opener");
      await revealItemInDir(path);
    } catch {
      /* opener plugin unavailable — ignore */
    }
  };

  const projectMenu = (project: string): MenuItem[] => {
    // 같은 그룹 안에서 위/아래 이동. 그룹 밖이면 (ungrouped) 표시 안 함.
    const pos = projectGroupPosition(project);
    const reorderItems: MenuItem[] = [];
    if (pos && pos.size > 1) {
      reorderItems.push(
        {
          label: "↑ 그룹 안에서 위로",
          onClick: () => moveProjectWithinGroup(project, -1),
          disabled: pos.idx === 0,
        },
        {
          label: "↓ 그룹 안에서 아래로",
          onClick: () => moveProjectWithinGroup(project, 1),
          disabled: pos.idx === pos.size - 1,
        },
        { kind: "divider" },
      );
    }
    return [
    ...reorderItems,
    {
      label: "새 도메인 파일…",
      onClick: () => onAddDomain(project),
    },
    {
      label: "새 폴더…",
      onClick: () => onAddFolder(project),
    },
    { label: "이름 바꾸기", onClick: () => onRenameProject(project) },
    { label: "아이콘 변경…", onClick: () => onChangeProjectIcon(project) },
    { label: "색상 변경…", onClick: () => onChangeProjectColor(project) },
    { label: "모두 읽음으로 표시", onClick: () => onMarkProjectRead(project) },
    { kind: "divider" },
    {
      label: "경로 복사 (절대)",
      onClick: () => copyPath([project]),
    },
    {
      label: "경로 복사 (vault 기준)",
      onClick: () => copyRelativePath([project]),
    },
    {
      label: "Finder에서 보기",
      onClick: () => revealInFinder([project]),
    },
    { kind: "divider" },
    {
      label: "Claude Code 설치 명령 복사",
      onClick: () => onCopyMcpInstall(project),
    },
    {
      label: "CLAUDE.md 단비 블록 복사",
      onClick: () => onCopyClaudeMdTemplate(project),
    },
    { kind: "divider" },
    { label: "프로젝트 삭제", onClick: () => onDeleteProject(project), danger: true },
    ];
  };

  // Flat list of every sub-folder under a project (top-level + nested),
  // used as targets in the domain "move to…" menu. The names are already
  // fully-qualified (`daily`, `daily/2026-01`) so we just recurse and
  // collect.
  const subfoldersOf = (project: string): string[] => {
    const proj = tree?.projects.find((p) => p.name === project);
    if (!proj) return [];
    const out: string[] = [];
    function walk(folders: SubfolderShape[]) {
      for (const f of folders) {
        out.push(f.name);
        if (f.subfolders?.length) walk(f.subfolders);
      }
    }
    walk(proj.subfolders);
    return out;
  };

  const domainMenu = (project: string, domain: string): MenuItem[] => {
    // Currently selected folder = the part before the last "/" in the
    // domain key (or null if it lives at the project root). We dim the
    // current location in the move list so the user knows where they are.
    const slashIdx = domain.lastIndexOf("/");
    const currentFolder = slashIdx > 0 ? domain.slice(0, slashIdx) : null;
    const folders = subfoldersOf(project);
    const moveTargets: MenuItem[] = [];
    if (currentFolder !== null) {
      moveTargets.push({
        label: "↑ 프로젝트 루트로",
        onClick: () => onMoveDomain(project, domain, null),
      });
    }
    for (const f of folders) {
      if (f === currentFolder) continue;
      moveTargets.push({
        label: `→ ${f}/`,
        onClick: () => onMoveDomain(project, domain, f),
      });
    }
    if (moveTargets.length > 0) {
      moveTargets.push({ kind: "divider" });
    }
    return [
      { label: "이름 바꾸기", onClick: () => onRenameDomain(project, domain) },
      {
        label: "경로 복사 (절대)",
        onClick: () => copyPath([project, domain]),
      },
      {
        label: "경로 복사 (vault 기준)",
        onClick: () => copyRelativePath([project, domain]),
      },
      {
        label: "Finder에서 보기",
        onClick: () => revealInFinder([project, domain]),
      },
      { kind: "divider" },
      ...moveTargets,
      {
        label: "도메인 삭제",
        onClick: () => onDeleteDomain(project, domain),
        danger: true,
      },
    ];
  };

  const folderMenu = (project: string, folder: string): MenuItem[] => {
    // 이미 한 번 nested 면 더 깊은 sub-folder 는 만들지 못하게 막는다
    // (백엔드도 거부하지만 메뉴에서 미리 빼주면 사용자가 헷갈리지 않음).
    const depth = folder.split("/").length;
    const items: MenuItem[] = [
      {
        label: "이 폴더에 새 파일…",
        onClick: () => onAddDomainInFolder(project, folder),
      },
    ];
    if (depth < 2) {
      items.push({
        label: "이 안에 새 폴더…",
        onClick: () => onAddSubFolder(project, folder),
      });
    }
    items.push({
      label: "폴더 이름 바꾸기",
      onClick: () => onRenameFolder(project, folder),
    });
    items.push({ kind: "divider" });
    items.push({
      label: "경로 복사 (절대)",
      onClick: () => copyPath([project, folder]),
    });
    items.push({
      label: "경로 복사 (vault 기준)",
      onClick: () => copyRelativePath([project, folder]),
    });
    items.push({
      label: "Finder에서 보기",
      onClick: () => revealInFinder([project, folder]),
    });
    items.push({ kind: "divider" });
    items.push({
      label: "폴더 삭제",
      onClick: () => onDeleteFolder(project, folder),
      danger: true,
    });
    return items;
  };

  return (
    <aside
      className="relative flex h-full shrink-0 select-none flex-col border-r border-hairline bg-surface [&_input]:select-text"
      style={{ width: sidebarWidth }}
    >
      {/* Drag handle on the right edge — wider hit area than the visible
          line so users don't have to pixel-hunt. The visible feedback is
          a 1px column that highlights on hover/active. */}
      <div
        onMouseDown={(e) => {
          e.preventDefault();
          setResizing(true);
        }}
        onDoubleClick={resetSidebarWidth}
        title="드래그로 폭 조절 · 더블클릭 시 기본값"
        className="absolute right-0 top-0 z-10 h-full w-2 -mr-1 cursor-col-resize group"
      >
        <div
          className={cn(
            "absolute right-1 top-0 h-full w-px transition-colors",
            resizing
              ? "bg-accent-blue"
              : "bg-transparent group-hover:bg-hairline-strong",
          )}
        />
      </div>
      <div className="flex h-full w-full flex-col">
      <header
        data-tauri-drag-region
        className="flex h-12 shrink-0 items-center justify-between gap-2 border-b border-hairline pr-2 pl-4"
      >
        <button
          onClick={clearSelection}
          title="홈으로"
          className="group flex h-8 shrink-0 items-center rounded-md px-1.5 transition-colors hover:bg-surface-elevated"
        >
          <Wordmark className="h-5 w-auto shrink-0 transition-opacity group-hover:opacity-90" />
        </button>
        <div className="flex items-center gap-0.5">
          <button
            onClick={onOpenSearch}
            title="검색 (⌘K)"
            className="grid h-8 w-8 place-items-center rounded-md text-mute transition-colors hover:bg-surface-elevated hover:text-on-dark"
          >
            <SearchIcon size={16} />
          </button>
          <button
            onClick={onOpenGraph}
            title="그래프 (⌘G)"
            className="grid h-8 w-8 place-items-center rounded-md text-mute transition-colors hover:bg-surface-elevated hover:text-on-dark"
          >
            <Network size={16} />
          </button>
          {/* 리뷰 인박스는 ghost scan / compound preview 등 LLM 결과를
              사용자가 승인·거부하는 곳이라, LLM 미연결 상태에선 의미 없음.
              cfg.provider 가 있을 때만 노출. */}
          {cfg?.provider && (
            <button
              onClick={onOpenReviews}
              title={`리뷰 인박스${reviewCount > 0 ? ` (${reviewCount})` : ""}`}
              className="relative grid h-8 w-8 place-items-center rounded-md text-mute transition-colors hover:bg-surface-elevated hover:text-on-dark"
            >
              <Inbox size={16} />
              {reviewCount > 0 && (
                <span className="absolute -right-1 -top-1 grid h-4 min-w-[16px] place-items-center rounded-full bg-accent-red px-1 text-[10px] font-semibold leading-none text-on-dark">
                  {reviewCount > 9 ? "9+" : reviewCount}
                </span>
              )}
            </button>
          )}
          <button
            onClick={onOpenTrash}
            title="휴지통"
            className="relative grid h-8 w-8 place-items-center rounded-md text-mute transition-colors hover:bg-surface-elevated hover:text-on-dark"
          >
            <Trash2 size={15} />
            {trashHasUnseen && (
              <span
                className="absolute h-1.5 w-1.5 rounded-full bg-accent-yellow"
                style={{ right: 6, top: 6 }}
                aria-hidden
              />
            )}
          </button>
          <NotificationBell />
          <button
            onClick={onRefreshTree}
            title="새로고침"
            className="grid h-8 w-8 place-items-center rounded-md text-mute transition-colors hover:bg-surface-elevated hover:text-on-dark"
          >
            <RefreshCw size={15} />
          </button>
          <button
            onClick={onAddProject}
            title="프로젝트 추가"
            className="grid h-8 w-8 place-items-center rounded-md text-mute transition-colors hover:bg-surface-elevated hover:text-on-dark"
          >
            <Plus size={17} />
          </button>
        </div>
      </header>

      <div className="flex-1 overflow-y-auto py-2">
        {/* tree 가 null 이면 아직 listTree 가 안 끝난 로딩 상태 — 빈 CTA
            를 띄우면 vault 가 큰 사용자에게도 매번 "프로젝트 없음" 깜빡임
            이 보인다. 진짜 빈 vault (tree.projects.length === 0) 일 때만
            CTA. tree=null 동안은 그냥 빈 공간. */}
        {tree === null ? null : tree.projects.length === 0 ? (
          <div className="flex h-full flex-col items-center justify-center gap-3 px-6 text-center">
            <div className="grid h-14 w-14 place-items-center rounded-full border border-dashed border-hairline-strong bg-surface-elevated">
              <FolderPlus size={26} className="text-stone" />
            </div>
            <div className="text-[14px] font-medium text-on-dark">
              아직 프로젝트가 없어요
            </div>
            <div className="text-[12px] leading-[1.55] text-mute">
              vault 의 첫 프로젝트를 만들면
              <br />
              사이드바 트리가 채워집니다.
            </div>
            <button
              onClick={onAddProject}
              className="mt-1 inline-flex h-9 items-center gap-1.5 rounded-md bg-primary px-4 text-[13px] font-medium text-on-primary transition-colors hover:bg-primary-pressed"
            >
              <Plus size={14} />
              새 프로젝트 만들기
            </button>
          </div>
        ) : (
          <>
            {/* Ungrouped projects (no header) */}
            {ungrouped.length > 0 && (
              <DropZone
                active={dropHint === "__ungrouped__"}
                onDragOver={(e) => {
                  // preventDefault on every dragover while a project
                  // drag is in progress — React state may not yet be
                  // set on the very first tick after dragstart, but
                  // the ref always is.
                  if (
                    dragProjectRef.current ||
                    e.dataTransfer.types.includes("text/plain")
                  ) {
                    e.preventDefault();
                    e.dataTransfer.dropEffect = "move";
                    setDropHint("__ungrouped__");
                  }
                }}
                onDragLeave={() => setDropHint(null)}
                onDrop={(e) => {
                  e.preventDefault();
                  const name =
                    dragProjectRef.current ??
                    e.dataTransfer.getData("text/plain");
                  if (name) moveProject(name, null, null);
                  setDragProject(null);
                  dragProjectRef.current = null;
                  setDropHint(null);
                }}
              >
                {ungrouped.map((name) => {
                  const p = tree.projects.find((x) => x.name === name);
                  if (!p) return null;
                  return (
                    <ProjectNode
                      key={p.name}
                      project={p}
                      expanded={expanded[p.name] ?? false}
                      selection={selection}
                      updates={projectChangeCount(p.name)}
                      isDragging={dragProject === p.name}
                      onDragStart={() => {
                        dragProjectRef.current = p.name;
                        setDragProject(p.name);
                      }}
                      onDragEnd={() => {
                        dragProjectRef.current = null;
                        setDragProject(null);
                        setDropHint(null);
                      }}
                      onDragOverRow={(_e) => {
                        const dragged = dragProjectRef.current;
                        if (!dragged || dragged === p.name) return;
                        setDropHint(`row:${p.name}`);
                      }}
                      onDropOnRow={(e) => {
                        const dragged =
                          dragProjectRef.current ??
                          e.dataTransfer.getData("text/plain");
                        if (dragged && dragged !== p.name) {
                          moveProject(dragged, groupOf(p.name), p.name);
                        }
                        dragProjectRef.current = null;
                        setDragProject(null);
                        setDropHint(null);
                      }}
                      rowDropHint={dropHint === `row:${p.name}`}
                      onToggle={() => toggle(p.name)}
                      onSelect={() => {
                        selectProject(p.name);
                        clearProjectUpdate(p.name);
                        ipc.projectMarkSeen(p.name).catch(() => {});
                      }}
                      onSelectDomain={(dom) => {
                        selectDomain(p.name, dom);
                        clearProjectUpdate(p.name);
                        ipc.projectMarkSeen(p.name).catch(() => {});
                      }}
                      onProjectContext={(e) =>
                        setCtx({
                          kind: "project",
                          x: e.clientX,
                          y: e.clientY,
                          project: p.name,
                        })
                      }
                      onDomainContext={(e, dom) =>
                        setCtx({
                          kind: "domain",
                          x: e.clientX,
                          y: e.clientY,
                          project: p.name,
                          domain: dom,
                        })
                      }
                      onAddDomain={() => onAddDomain(p.name)}
                      onFolderContext={(e, folder) =>
                        setCtx({
                          kind: "folder",
                          x: e.clientX,
                          y: e.clientY,
                          project: p.name,
                          folder,
                        })
                      }
                      onDropDomainOnFolder={(folder, payload) =>
                        dispatchDomainDrop(p.name, folder, payload)
                      }
                      onDropDomainOnRoot={(payload) =>
                        dispatchDomainDrop(p.name, null, payload)
                      }
                    />
                  );
                })}
              </DropZone>
            )}

            {/* Named groups */}
            {groups.map((g, idx) => (
              <GroupSection
                key={g.id}
                group={g}
                tree={tree}
                expanded={expanded}
                selection={selection}
                projectChangeCount={projectChangeCount}
                dragProject={dragProject}
                dropHint={dropHint}
                setDragProject={setDragProject}
                setDropHint={setDropHint}
                moveProject={moveProject}
                groupOf={groupOf}
                renaming={renamingGroupId === g.id}
                renameDraft={renameDraft}
                onRenameDraft={setRenameDraft}
                onRenameCommit={commitRename}
                onRenameCancel={() => {
                  setRenamingGroupId(null);
                  setRenameDraft("");
                }}
                pendingDelete={pendingDelete === g.id}
                dragProjectRef={dragProjectRef}
                onDragStartForProject={(name) => {
                  dragProjectRef.current = name;
                  setDragProject(name);
                }}
                onDragEnd={() => {
                  dragProjectRef.current = null;
                  setDragProject(null);
                  setDropHint(null);
                }}
                onToggleGroup={() => toggleGroupCollapsed(g.id)}
                onRenameGroup={() => startRenameGroup(g.id)}
                onDeleteGroup={() => confirmDelete(g.id)}
                onMoveGroupUp={() => moveGroup(g.id, -1)}
                onMoveGroupDown={() => moveGroup(g.id, 1)}
                onAddProjectToGroup={() => onAddProjectToGroup(g.id)}
                canMoveUp={idx > 0}
                canMoveDown={idx < groups.length - 1}
                onToggleProject={toggle}
                onSelectProject={(name) => {
                  selectProject(name);
                  clearProjectUpdate(name);
                  ipc.projectMarkSeen(name).catch(() => {});
                }}
                onSelectDomain={(proj, dom) => {
                  selectDomain(proj, dom);
                  clearProjectUpdate(proj);
                  ipc.projectMarkSeen(proj).catch(() => {});
                }}
                onProjectContext={(e, project) =>
                  setCtx({
                    kind: "project",
                    x: e.clientX,
                    y: e.clientY,
                    project,
                  })
                }
                onDomainContext={(e, project, domain) =>
                  setCtx({
                    kind: "domain",
                    x: e.clientX,
                    y: e.clientY,
                    project,
                    domain,
                  })
                }
                onAddDomain={onAddDomain}
                onFolderContext={(e, project, folder) =>
                  setCtx({
                    kind: "folder",
                    x: e.clientX,
                    y: e.clientY,
                    project,
                    folder,
                  })
                }
                onDropDomainOnFolder={(project, folder, payload) =>
                  dispatchDomainDrop(project, folder, payload)
                }
              />
            ))}

            {/* Create group */}
            {creatingGroup ? (
              <div className="mx-1 mt-2 flex items-center gap-1.5 rounded-sm border border-hairline bg-surface-elevated px-2 py-1">
                <Layers size={11} className="shrink-0 text-stone" />
                <input
                  autoFocus
                  value={newGroupLabel}
                  onChange={(e) => setNewGroupLabel(e.target.value)}
                  onBlur={commitNewGroup}
                  onKeyDown={(e) => {
                    if (e.key === "Enter") commitNewGroup();
                    if (e.key === "Escape") {
                      setCreatingGroup(false);
                      setNewGroupLabel("");
                    }
                  }}
                  placeholder="그룹 이름"
                  className="h-5 flex-1 bg-transparent text-[12px] text-ink outline-none placeholder:text-stone"
                />
              </div>
            ) : (
              <button
                onClick={() => {
                  setCreatingGroup(true);
                  setNewGroupLabel("");
                }}
                className="mx-1 mt-2 flex w-[calc(100%-8px)] items-center gap-1.5 rounded-sm px-2 py-1 text-left text-[12px] text-stone transition-colors hover:bg-surface-elevated hover:text-body"
              >
                <Layers size={11} />새 그룹
              </button>
            )}
          </>
        )}
      </div>

      <SidebarFooter
        vaultPath={tree?.vault_path ?? null}
        onOpenSettings={onOpenSettings}
      />



      {ctx.kind === "project" && (
        <ContextMenu
          x={ctx.x}
          y={ctx.y}
          items={projectMenu(ctx.project)}
          onClose={() => setCtx({ kind: "none" })}
        />
      )}
      {ctx.kind === "domain" && (
        <ContextMenu
          x={ctx.x}
          y={ctx.y}
          items={domainMenu(ctx.project, ctx.domain)}
          onClose={() => setCtx({ kind: "none" })}
        />
      )}
      {ctx.kind === "folder" && (
        <ContextMenu
          x={ctx.x}
          y={ctx.y}
          items={folderMenu(ctx.project, ctx.folder)}
          onClose={() => setCtx({ kind: "none" })}
        />
      )}
      </div>
    </aside>
  );
}

/** Thin drop target wrapper that exists so empty groups and the
 *  ungrouped bucket both show a visible "drop here" strip when a
 *  project is being dragged over them. */
function DropZone({
  active,
  children,
  onDragOver,
  onDragLeave,
  onDrop,
}: {
  active: boolean;
  children: React.ReactNode;
  onDragOver: (e: React.DragEvent) => void;
  onDragLeave: () => void;
  onDrop: (e: React.DragEvent) => void;
}) {
  return (
    <div
      onDragOver={onDragOver}
      onDragLeave={onDragLeave}
      onDrop={onDrop}
      className={cn(
        "rounded-sm transition-colors",
        active && "bg-accent-blue-soft/40",
      )}
    >
      {children}
    </div>
  );
}

/** "N" pill next to a project name, indicating new commits since the
 *  last time the user opened it. Hidden when count is 0. When the project
 *  has a custom accent color, paint the badge with that color too so the
 *  whole row reads as one identity. */
function UpdateBadge({
  count,
  colorFg,
}: {
  count: number;
  colorFg?: string | null;
}) {
  if (!count) return null;
  return (
    <span
      className={cn(
        "ml-1 grid h-4 min-w-[16px] place-items-center rounded-full px-1 text-[10px] font-medium leading-none text-on-primary",
        colorFg ? null : "bg-accent-blue",
      )}
      style={colorFg ? { backgroundColor: colorFg } : undefined}
    >
      {count > 9 ? "9+" : count}
    </span>
  );
}

/** One project + expanded domain tree. Split out of the main render so
 *  both grouped and ungrouped sections share the exact same row. */
function ProjectNode({
  project,
  expanded,
  selection,
  updates,
  isDragging,
  rowDropHint,
  onDragStart,
  onDragEnd,
  onDragOverRow,
  onDropOnRow,
  onToggle,
  onSelect,
  onSelectDomain,
  onProjectContext,
  onDomainContext,
  onAddDomain,
  onFolderContext,
  onDropDomainOnFolder,
  onDropDomainOnRoot,
}: {
  project: {
    name: string;
    domains: { name: string; bytes: number; title?: string | null }[];
    subfolders: SubfolderShape[];
  };
  expanded: boolean;
  selection: { project: string | null; domain: string | null };
  updates: number;
  isDragging: boolean;
  rowDropHint: boolean;
  onDragStart: () => void;
  onDragEnd: () => void;
  onDragOverRow: (e: React.DragEvent) => void;
  onDropOnRow: (e: React.DragEvent) => void;
  onToggle: () => void;
  onSelect: () => void;
  onSelectDomain: (domain: string) => void;
  onProjectContext: (e: React.MouseEvent) => void;
  onDomainContext: (e: React.MouseEvent, domain: string) => void;
  onAddDomain: () => void;
  onFolderContext: (e: React.MouseEvent, folder: string) => void;
  /** Drop handler for a `.md` file dragged onto a sub-folder row inside
   *  this project. The payload is the verbatim "<project>::<domain>"
   *  string from the dataTransfer. */
  onDropDomainOnFolder: (folder: string, payload: string) => void;
  /** Drop handler for the project header itself — moves a dropped file
   *  back to the project root. Always belongs to THIS project, but the
   *  payload may identify a different source project so the parent does
   *  the actual cross-project routing. */
  onDropDomainOnRoot: (payload: string) => void;
}) {
  const projectActive =
    selection.project === project.name && !selection.domain;
  const colorKey = useApp(
    (s) => s.cfg?.project_colors?.[project.name] ?? null,
  );
  const colorVars = projectColorVars(colorKey);
  const hasCustomColor = !!colorKey;
  return (
    <div
      className={cn(
        "rounded-md px-1 transition-colors",
        // When the user has expanded this project, give the entire
        // subtree a card surface so the active scope is unmistakable.
        // Helps a lot in long sidebars where multiple projects live.
        expanded && "bg-surface-elevated/60 py-1",
      )}
      style={
        expanded && hasCustomColor
          ? { boxShadow: `inset 2px 0 0 0 ${colorVars.fg}` }
          : undefined
      }
    >
      {/* Drag lives on just the header row so expanded domains don't get
       *  dragged accidentally. dragover/drop on the header row insert
       *  the dragged project *before* this one inside the same group. */}
      <div
        className={cn(
          "group flex items-center gap-0.5 rounded-sm border-t-2 border-transparent",
          rowDropHint && "border-accent-blue",
        )}
        draggable
        onDragStart={(e) => {
          e.dataTransfer.effectAllowed = "move";
          e.dataTransfer.setData("text/plain", project.name);
          onDragStart();
        }}
        onDragEnd={onDragEnd}
        onDragOver={(e) => {
          // Allow drop on this row even when React hasn't processed the
          // dragstart yet (initial-tick race). preventDefault must run
          // unconditionally when a drag is in progress, otherwise the
          // subsequent drop event is suppressed by the browser.
          if (e.dataTransfer.types.includes("text/plain")) {
            e.preventDefault();
            e.dataTransfer.dropEffect = "move";
          }
          onDragOverRow(e);
        }}
        onDrop={(e) => {
          e.preventDefault();
          onDropOnRow(e);
        }}
      >
        <span
          className={cn(
            "grid h-6 w-3 cursor-grab place-items-center rounded-sm text-stone opacity-0 transition-opacity group-hover:opacity-100",
            isDragging && "opacity-100",
          )}
          aria-label="드래그로 이동"
        >
          <GripVertical size={10} />
        </span>
        <button
          onClick={() => {
            onSelect();
            onToggle();
          }}
          onContextMenu={(e) => {
            e.preventDefault();
            onProjectContext(e);
          }}
          className={cn(
            "flex flex-1 items-center gap-1.5 rounded-sm px-2 py-1 text-left text-[14px] transition-colors",
            projectActive
              ? hasCustomColor
                ? null
                : "bg-accent-blue-soft text-accent-blue"
              : expanded
                ? "font-medium text-on-dark"
                : "text-body hover:bg-surface-elevated hover:text-on-dark",
            isDragging && "opacity-50",
          )}
          style={
            projectActive && hasCustomColor
              ? { backgroundColor: colorVars.soft, color: colorVars.fg }
              : undefined
          }
        >
          {expanded ? (
            <ChevronDown
              size={13}
              className={cn(
                "shrink-0",
                projectActive && !hasCustomColor
                  ? "text-accent-blue"
                  : !projectActive
                    ? "text-on-dark"
                    : null,
              )}
              style={
                projectActive && hasCustomColor
                  ? { color: colorVars.fg }
                  : undefined
              }
            />
          ) : (
            <ChevronRight size={13} className="shrink-0 text-mute" />
          )}
          <ProjectIcon project={project.name} />
          <span className="flex-1 truncate">{project.name}</span>
          <UpdateBadge
            count={updates}
            colorFg={hasCustomColor ? colorVars.fg : null}
          />
        </button>
      </div>

      {expanded && (
        <div
          className="ml-3 mt-1 border-l border-hairline-strong pl-7"
          onDragOver={(e) => {
            // 프로젝트 root 영역은 sub-folder 가 아니라서, 폴더 row 가
            // 이미 핸들링한 드롭이 아닐 때만 여기로 떨어지게 한다.
            if (e.dataTransfer.types.includes("application/danbi-domain")) {
              e.preventDefault();
              e.dataTransfer.dropEffect = "move";
            }
          }}
          onDrop={(e) => {
            const payload = e.dataTransfer.getData("application/danbi-domain");
            if (payload) {
              e.preventDefault();
              onDropDomainOnRoot(payload);
            }
          }}
        >
          {project.domains.length === 0 && project.subfolders.length === 0 && (
            <div className="px-2 py-1 text-caption-sm text-stone">
              비어있음
            </div>
          )}
          {project.domains.map((d) => {
            const active =
              selection.project === project.name &&
              selection.domain === d.name;
            return (
              <DomainRow
                key={d.name}
                projectName={project.name}
                domain={d.name}
                title={d.title}
                active={active}
                onSelect={() => onSelectDomain(d.name)}
                onContextMenu={(e) => {
                  e.preventDefault();
                  onDomainContext(e, d.name);
                }}
              />
            );
          })}
          {project.subfolders.map((sf) => (
            <SubfolderRow
              key={sf.name}
              project={project.name}
              subfolder={sf}
              activeDomain={
                selection.project === project.name ? selection.domain : null
              }
              onSelect={(dom) => onSelectDomain(dom)}
              onDomainContext={(e, dom) => {
                e.preventDefault();
                onDomainContext(e, dom);
              }}
              onFolderContext={(e, folder) => {
                e.preventDefault();
                onFolderContext(e, folder);
              }}
              onDropDomain={(folder, payload) =>
                onDropDomainOnFolder(folder, payload)
              }
            />
          ))}
          <button
            onClick={onAddDomain}
            className="mt-0.5 flex w-full items-center gap-1.5 rounded-sm px-2 py-1 text-left text-[13px] text-stone transition-colors hover:bg-surface-elevated hover:text-body"
          >
            <Plus size={13} />새 도메인
          </button>
        </div>
      )}
    </div>
  );
}

/** A named group — header row (draggable drop target) + its projects. */
function GroupSection({
  group,
  tree,
  expanded,
  selection,
  projectChangeCount,
  dragProject,
  dropHint,
  setDragProject,
  setDropHint,
  moveProject,
  groupOf,
  renaming,
  renameDraft,
  onRenameDraft,
  onRenameCommit,
  onRenameCancel,
  pendingDelete,
  dragProjectRef,
  onDragStartForProject,
  onDragEnd,
  onToggleGroup,
  onRenameGroup,
  onDeleteGroup,
  onMoveGroupUp,
  onMoveGroupDown,
  onAddProjectToGroup,
  canMoveUp,
  canMoveDown,
  onToggleProject,
  onSelectProject,
  onSelectDomain,
  onProjectContext,
  onDomainContext,
  onAddDomain,
  onFolderContext,
  onDropDomainOnFolder,
}: {
  group: ProjectGroup;
  tree: {
    projects: {
      name: string;
      domains: { name: string; bytes: number; title?: string | null }[];
      subfolders: SubfolderShape[];
    }[];
  };
  expanded: Record<string, boolean>;
  selection: { project: string | null; domain: string | null };
  /** mtime-based change count (files with a lit dot). Parent owns the
   *  calculation so each folder row's badge stays in sync. */
  projectChangeCount: (name: string) => number;
  dragProject: string | null;
  dropHint: string | null;
  setDragProject: (v: string | null) => void;
  setDropHint: (v: string | null) => void;
  moveProject: (
    project: string,
    destGroupId: string | null,
    beforeProject: string | null,
  ) => void;
  groupOf: (project: string) => string | null;
  renaming: boolean;
  renameDraft: string;
  onRenameDraft: (v: string) => void;
  onRenameCommit: () => void;
  onRenameCancel: () => void;
  pendingDelete: boolean;
  dragProjectRef: React.MutableRefObject<string | null>;
  onDragStartForProject: (name: string) => void;
  onDragEnd: () => void;
  onToggleGroup: () => void;
  onRenameGroup: () => void;
  onDeleteGroup: () => void;
  onMoveGroupUp: () => void;
  onMoveGroupDown: () => void;
  onAddProjectToGroup: () => void;
  canMoveUp: boolean;
  canMoveDown: boolean;
  onToggleProject: (name: string) => void;
  onSelectProject: (name: string) => void;
  onSelectDomain: (project: string, domain: string) => void;
  onProjectContext: (e: React.MouseEvent, project: string) => void;
  onDomainContext: (
    e: React.MouseEvent,
    project: string,
    domain: string,
  ) => void;
  onAddDomain: (project: string) => void;
  onFolderContext: (e: React.MouseEvent, project: string, folder: string) => void;
  /** Domain DnD drop dispatcher. Pass `folder = null` to drop into a
   *  project root. */
  onDropDomainOnFolder: (
    project: string,
    folder: string | null,
    payload: string,
  ) => void;
}) {
  const groupKey = `group:${group.id}`;
  const isOver = dropHint === groupKey;
  return (
    <section
      className="mt-3 px-1"
      onDragOver={(e) => {
        if (
          dragProjectRef.current ||
          e.dataTransfer.types.includes("text/plain")
        ) {
          e.preventDefault();
          e.dataTransfer.dropEffect = "move";
          setDropHint(groupKey);
        }
      }}
      onDragLeave={() => {
        if (dropHint === groupKey) setDropHint(null);
      }}
      onDrop={(e) => {
        e.preventDefault();
        const dragged =
          dragProjectRef.current ?? e.dataTransfer.getData("text/plain");
        if (dragged) moveProject(dragged, group.id, null);
        dragProjectRef.current = null;
        setDragProject(null);
        setDropHint(null);
      }}
    >
      <div
        className={cn(
          "group mb-0.5 flex items-center gap-1 rounded-sm px-2 py-1",
          isOver &&
            "bg-accent-blue-soft/40 outline outline-1 outline-accent-blue/50",
        )}
      >
        {renaming ? (
          <>
            <Layers size={11} className="shrink-0 text-stone" />
            <input
              autoFocus
              value={renameDraft}
              onChange={(e) => onRenameDraft(e.target.value)}
              onBlur={onRenameCommit}
              onKeyDown={(e) => {
                if (e.key === "Enter") onRenameCommit();
                if (e.key === "Escape") onRenameCancel();
              }}
              className="h-5 flex-1 bg-transparent text-[12px] text-ink outline-none"
            />
          </>
        ) : (
          <>
            <button
              onClick={onToggleGroup}
              className="flex flex-1 items-center gap-1 text-left"
              title={group.collapsed ? "펼치기" : "접기"}
            >
              {group.collapsed ? (
                <ChevronRight size={11} className="shrink-0 text-stone" />
              ) : (
                <ChevronDown size={11} className="shrink-0 text-stone" />
              )}
              <span className="text-[11px] font-medium uppercase tracking-[0.6px] text-mute">
                {group.label}
              </span>
              <span className="text-[11px] font-medium text-mute">
                {group.projects.length}
              </span>
            </button>
            <button
              onClick={onMoveGroupUp}
              disabled={!canMoveUp}
              title="그룹 위로 이동"
              className="grid h-7 w-7 place-items-center rounded-sm text-stone opacity-0 transition-opacity hover:bg-surface-elevated hover:text-on-dark group-hover:opacity-100 disabled:cursor-not-allowed disabled:opacity-20 group-hover:disabled:opacity-20"
            >
              <ArrowUp size={14} />
            </button>
            <button
              onClick={onMoveGroupDown}
              disabled={!canMoveDown}
              title="그룹 아래로 이동"
              className="grid h-7 w-7 place-items-center rounded-sm text-stone opacity-0 transition-opacity hover:bg-surface-elevated hover:text-on-dark group-hover:opacity-100 disabled:cursor-not-allowed disabled:opacity-20 group-hover:disabled:opacity-20"
            >
              <ArrowDown size={14} />
            </button>
            <button
              onClick={onAddProjectToGroup}
              title="이 그룹에 프로젝트 추가"
              className="grid h-7 w-7 place-items-center rounded-sm text-stone opacity-0 transition-opacity hover:bg-surface-elevated hover:text-on-dark group-hover:opacity-100"
            >
              <Plus size={14} />
            </button>
            <button
              onClick={onRenameGroup}
              title="이름 변경"
              className="grid h-7 w-7 place-items-center rounded-sm text-stone opacity-0 transition-opacity hover:bg-surface-elevated hover:text-on-dark group-hover:opacity-100"
            >
              <SettingsIcon size={14} />
            </button>
            <button
              onClick={onDeleteGroup}
              title={pendingDelete ? "한 번 더 클릭해서 삭제" : "그룹 삭제"}
              className={cn(
                "grid h-7 w-7 place-items-center rounded-sm transition-colors",
                pendingDelete
                  ? "bg-accent-red-soft text-accent-red opacity-100"
                  : "text-stone opacity-0 hover:bg-surface-elevated hover:text-accent-red group-hover:opacity-100",
              )}
            >
              <Plus size={14} className="rotate-45" />
            </button>
          </>
        )}
      </div>

      {!group.collapsed && (
        <>
          {group.projects.length === 0 && (
            <div
              className={cn(
                "mx-1 rounded-sm border border-dashed px-2 py-2.5 text-center text-[11px]",
                dragProject
                  ? "border-accent-blue/60 bg-accent-blue-soft/40 text-accent-blue"
                  : "border-hairline-strong text-mute",
              )}
            >
              {dragProject ? "여기에 놓기" : "비어있음 — 프로젝트를 끌어다 놓으세요"}
            </div>
          )}
          {group.projects.map((name) => {
            const p = tree.projects.find((x) => x.name === name);
            if (!p) return null;
            return (
              <ProjectNode
                key={p.name}
                project={p}
                expanded={expanded[p.name] ?? false}
                selection={selection}
                updates={projectChangeCount(p.name)}
                isDragging={dragProject === p.name}
                onDragStart={() => onDragStartForProject(p.name)}
                onDragEnd={onDragEnd}
                onDragOverRow={(_e) => {
                  const dragged = dragProjectRef.current;
                  if (!dragged || dragged === p.name) return;
                  setDropHint(`row:${p.name}`);
                }}
                onDropOnRow={(e) => {
                  const dragged =
                    dragProjectRef.current ??
                    e.dataTransfer.getData("text/plain");
                  if (dragged && dragged !== p.name) {
                    moveProject(dragged, groupOf(p.name), p.name);
                  }
                  dragProjectRef.current = null;
                  setDragProject(null);
                  setDropHint(null);
                }}
                rowDropHint={dropHint === `row:${p.name}`}
                onToggle={() => onToggleProject(p.name)}
                onSelect={() => onSelectProject(p.name)}
                onSelectDomain={(dom) => onSelectDomain(p.name, dom)}
                onProjectContext={(e) => onProjectContext(e, p.name)}
                onDomainContext={(e, dom) => onDomainContext(e, p.name, dom)}
                onAddDomain={() => onAddDomain(p.name)}
                onFolderContext={(e, folder) =>
                  onFolderContext(e, p.name, folder)
                }
                onDropDomainOnFolder={(folder, payload) =>
                  onDropDomainOnFolder(p.name, folder, payload)
                }
                onDropDomainOnRoot={(payload) =>
                  onDropDomainOnFolder(p.name, null, payload)
                }
              />
            );
          })}
        </>
      )}
    </section>
  );
}

function SubfolderRow({
  project,
  subfolder,
  activeDomain,
  onSelect,
  onDomainContext,
  onFolderContext,
  onDropDomain,
}: {
  project: string;
  subfolder: SubfolderShape;
  activeDomain: string | null;
  onSelect: (domain: string) => void;
  onDomainContext: (e: React.MouseEvent, domain: string) => void;
  onFolderContext: (e: React.MouseEvent, folder: string) => void;
  /** Called when a domain dragged from another row drops onto this folder.
   *  `payload` is the verbatim "<project>::<domain>" string from
   *  dataTransfer; the parent decides if/how to dispatch a move. */
  onDropDomain: (folder: string, payload: string) => void;
}) {
  // 'notes' 폴더는 보통 잡다한 메모가 길게 누적돼있어서 프로젝트 펼칠
  // 때마다 사이드바 절반을 차지함. 처음엔 닫힌 상태로 시작 — 사용자가
  // 한 번 펼치면 그 세션 동안 유지. 프로젝트 접었다 다시 열면 또 닫혀
  // 시작 (의도된 동작 — 시각적 노이즈 줄이기).
  const segmentsForOpen = subfolder.name.split("/");
  const lastSeg = segmentsForOpen[segmentsForOpen.length - 1];
  const startClosed = lastSeg === "notes";
  const [open, setOpen] = useState(!startClosed);
  const [dragOver, setDragOver] = useState(false);
  // Show the latest few daily notes first — user cares about "today" most.
  const sorted =
    subfolder.name === "daily"
      ? [...subfolder.domains].reverse()
      : subfolder.domains;
  // Display label is just the LAST segment of the path so the indent
  // does the work of communicating hierarchy. The full path stays in the
  // data layer for folder ops.
  const segments = subfolder.name.split("/");
  const label = segments[segments.length - 1];
  // File count rolled up from this folder + its nested children.
  const childCount =
    subfolder.domains.length +
    subfolder.subfolders.reduce((acc, s) => acc + s.domains.length, 0);
  // Number of *changed* files inside this folder (new or modified since
  // last seen). Powers the small accent-blue badge on the folder row.
  const domainUpdates = useApp((s) => s.domainUpdates);
  const showSidebarDots = useApp(
    (s) => s.cfg?.appearance?.unseen_sidebar_dots !== false,
  );
  const changeCount = (() => {
    if (!showSidebarDots) return 0;
    let n = 0;
    function walk(folder: SubfolderShape) {
      for (const d of folder.domains) {
        if (domainUpdates[`${project}/${d.name}`]) n += 1;
      }
      for (const f of folder.subfolders ?? []) walk(f);
    }
    walk(subfolder);
    return n;
  })();

  // 현재 선택된 도메인이 이 폴더 (또는 nested) 안에 들어있는지. wiki tree
  // 의 도메인은 풀 path 로 들어와 있으므로 (예: "daily/2026-05-17.md")
  // subfolder.name 이 그 prefix 면 이 폴더 안에 활성 도메인이 있다.
  const containsActive = (() => {
    if (!activeDomain) return false;
    return activeDomain.startsWith(`${subfolder.name}/`);
  })();

  // 프로젝트 고유 색이 설정되어 있으면 폴더 아이콘·chevron·active 톤
  // 모두 그 색으로 통일. 미설정 프로젝트는 기존 blue 그대로.
  const colorKey = useApp(
    (s) => s.cfg?.project_colors?.[project] ?? null,
  );
  const colorVars = projectColorVars(colorKey);
  const hasCustomColor = !!colorKey;

  return (
    <div
      className="mt-0.5"
      onDragOver={(e) => {
        if (e.dataTransfer.types.includes("application/danbi-domain")) {
          e.preventDefault();
          e.dataTransfer.dropEffect = "move";
          setDragOver(true);
        }
      }}
      onDragLeave={() => setDragOver(false)}
      onDrop={(e) => {
        const payload = e.dataTransfer.getData("application/danbi-domain");
        if (payload) {
          e.preventDefault();
          e.stopPropagation();
          onDropDomain(subfolder.name, payload);
        }
        setDragOver(false);
      }}
    >
      <button
        onClick={() => setOpen((v) => !v)}
        onContextMenu={(e) => onFolderContext(e, subfolder.name)}
        className={cn(
          "flex w-full items-center gap-1.5 rounded-sm px-2 py-1 text-left text-[14px] transition-colors",
          // 현재 선택된 도메인이 이 폴더 (또는 nested) 안에 있으면 폴더
          // 자체도 활성 톤으로 — 사용자가 어느 컨텍스트에 있는지 트리에서
          // 잃지 않도록.
          containsActive && !hasCustomColor
            ? "bg-accent-blue-soft/40 font-medium text-on-dark"
            : containsActive
              ? "font-medium text-on-dark"
              : "text-body hover:bg-surface-elevated hover:text-on-dark",
          dragOver && !hasCustomColor &&
            "bg-accent-blue-soft/40 outline outline-1 outline-accent-blue/50",
        )}
        style={(() => {
          if (containsActive && hasCustomColor) {
            return { backgroundColor: colorVars.soft };
          }
          if (dragOver && hasCustomColor) {
            return {
              backgroundColor: colorVars.soft,
              outline: `1px solid ${colorVars.fg}`,
            };
          }
          return undefined;
        })()}
      >
        {open ? (
          <ChevronDown
            size={13}
            className={cn(
              "shrink-0",
              containsActive && !hasCustomColor ? "text-accent-blue" : null,
              !containsActive ? "text-mute" : null,
            )}
            style={
              containsActive && hasCustomColor
                ? { color: colorVars.fg }
                : undefined
            }
          />
        ) : (
          <ChevronRight
            size={13}
            className={cn(
              "shrink-0",
              containsActive && !hasCustomColor ? "text-accent-blue" : null,
              !containsActive ? "text-mute" : null,
            )}
            style={
              containsActive && hasCustomColor
                ? { color: colorVars.fg }
                : undefined
            }
          />
        )}
        {open ? (
          <FolderOpen
            size={13}
            className={cn(
              "shrink-0",
              hasCustomColor
                ? null
                : containsActive
                  ? "text-accent-blue"
                  : "text-accent-blue/80",
            )}
            style={
              hasCustomColor
                ? { color: colorVars.fg, opacity: containsActive ? 1 : 0.8 }
                : undefined
            }
          />
        ) : (
          <Folder
            size={13}
            className={cn(
              "shrink-0",
              hasCustomColor
                ? null
                : containsActive
                  ? "text-accent-blue"
                  : "text-accent-blue/80",
            )}
            style={
              hasCustomColor
                ? { color: colorVars.fg, opacity: containsActive ? 1 : 0.8 }
                : undefined
            }
          />
        )}
        <span className="flex-1 truncate">{label}</span>
        {changeCount > 0 && (
          <span
            className={cn(
              "grid h-4 min-w-[16px] place-items-center rounded-full px-1 text-[10px] font-medium leading-none text-on-primary",
              hasCustomColor ? null : "bg-accent-blue",
            )}
            style={hasCustomColor ? { backgroundColor: colorVars.fg } : undefined}
            title={`이 폴더에 변경된 파일 ${changeCount}개`}
          >
            {changeCount > 9 ? "9+" : changeCount}
          </span>
        )}
        <span className="text-[11px] text-stone">{childCount}</span>
      </button>
      {open && (
        <div className="pl-4">
          {/* Nested folders first so the user sees the hierarchy before
              the leaf files. */}
          {subfolder.subfolders.map((nested) => (
            <SubfolderRow
              key={nested.name}
              project={project}
              subfolder={nested}
              activeDomain={activeDomain}
              onSelect={onSelect}
              onDomainContext={onDomainContext}
              onFolderContext={onFolderContext}
              onDropDomain={onDropDomain}
            />
          ))}
          {sorted.length === 0 && subfolder.subfolders.length === 0 && (
            <div className="px-2 py-1 text-caption-sm text-stone">
              비어있음
            </div>
          )}
          {sorted.map((d) => {
            const active = activeDomain === d.name;
            // Show only the filename — the folder context is already
            // implied by indentation.
            const display = d.name.includes("/")
              ? d.name.split("/").pop() ?? d.name
              : d.name;
            return (
              <DomainRow
                key={d.name}
                projectName={project}
                domain={d.name}
                displayName={display}
                title={d.title}
                active={active}
                onSelect={() => onSelect(d.name)}
                onContextMenu={(e) => onDomainContext(e, d.name)}
              />
            );
          })}
        </div>
      )}
    </div>
  );
}

/** Single domain row with a "modified/new" indicator. The dot/badge is
 *  fed from the global `domainUpdates` map, which the Workspace
 *  refreshes on watcher events. Clicking the row clears that single
 *  domain's badge (server-side `domain_mark_seen`) and selects it. */
function DomainRow({
  projectName,
  domain,
  displayName,
  title,
  active,
  onSelect,
  onContextMenu,
}: {
  projectName: string;
  domain: string;
  /** Optional override — used by SubfolderRow to strip the folder prefix. */
  displayName?: string;
  /** First H1/H2 from the file body, dimmed next to the filename so users
   *  can scan project files by topic without opening each one. Backend
   *  omits this for daily-style append-only folders. */
  title?: string | null;
  active: boolean;
  onSelect: () => void;
  onContextMenu: (e: React.MouseEvent) => void;
}) {
  const rawChange = useApp(
    (s) => s.domainUpdates[`${projectName}/${domain}`] ?? null,
  );
  const showSidebarDots = useApp(
    (s) => s.cfg?.appearance?.unseen_sidebar_dots !== false,
  );
  const change = showSidebarDots ? rawChange : null;
  const clearDomainUpdate = useApp((s) => s.clearDomainUpdate);
  const colorKey = useApp(
    (s) => s.cfg?.project_colors?.[projectName] ?? null,
  );
  const colorVars = projectColorVars(colorKey);
  const hasCustomColor = !!colorKey;
  // daily/*.md 인 경우 그 파일에 등장한 trigger kind 들을 미니 chip 으로
  // 노출. ProjectJournalView 캐시에서 곧장 꺼내쓰므로 추가 IPC 없음.
  // selector 는 *반드시* 안정 참조를 반환해야 zustand 가 무한 렌더 안 한다
  // — `?? []` 는 매 호출마다 새 배열이라 위험. 캐시 안의 배열을 그대로
  // 반환하고 (`undefined` 일 수 있음), 렌더 시점에 분기.
  const fileKinds = useApp((s) => {
    if (!domain.startsWith("daily/") || !domain.endsWith(".md")) return undefined;
    return s.projectJournalCache[projectName]?.daily_file_kinds[domain];
  });
  return (
    <button
      draggable
      onDragStart={(e) => {
        // 자체 MIME 사용해서 프로젝트 DnD 와 충돌 방지.
        // payload = "<project>::<domain>".
        e.dataTransfer.effectAllowed = "move";
        e.dataTransfer.setData(
          "application/danbi-domain",
          `${projectName}::${domain}`,
        );
      }}
      onClick={() => {
        // 표시(dot) 가 꺼져 있어도 backend lastSeen 은 항상 갱신해서,
        // 사용자가 토글을 다시 켰을 때 옛날 변경이 한꺼번에 쏟아지지 않게.
        if (rawChange) {
          clearDomainUpdate(projectName, domain);
          ipc.domainMarkSeen(projectName, domain).catch(() => {});
        }
        onSelect();
      }}
      onContextMenu={onContextMenu}
      className={cn(
        "group relative flex w-full items-center gap-1.5 rounded-sm px-2 py-1 text-left text-[13px] transition-colors",
        active
          ? hasCustomColor
            ? "font-medium"
            : "bg-accent-blue-soft font-medium text-accent-blue"
          : "text-body hover:bg-surface-elevated hover:text-on-dark",
      )}
      style={
        active && hasCustomColor
          ? { backgroundColor: colorVars.soft, color: colorVars.fg }
          : undefined
      }
      title={title ? `${domain} — ${title}` : domain}
    >
      {/* 좌측 가장자리 accent strip — 사이드바 settings nav 의
          active 표시와 같은 패턴. 도메인이 깊이 indent 돼있어도 어느
          파일이 열려있는지 한 눈에 인지 가능. */}
      {active && (
        <span
          className={cn(
            "absolute inset-y-1 left-0 w-[2px] rounded-r-full",
            hasCustomColor ? null : "bg-accent-blue",
          )}
          style={hasCustomColor ? { backgroundColor: colorVars.fg } : undefined}
        />
      )}
      <FileText
        size={12}
        className={cn(
          "shrink-0 self-start",
          title ? "mt-[3px]" : "",
          active && !hasCustomColor ? "text-accent-blue" : null,
          !active ? "text-mute" : null,
        )}
        style={
          active && hasCustomColor ? { color: colorVars.fg } : undefined
        }
      />
      <span className="flex min-w-0 flex-1 flex-col">
        <span className="truncate font-mono">{displayName ?? domain}</span>
        {title && (
          <span
            className={cn(
              "truncate text-[11px] leading-tight transition-colors",
              active
                ? hasCustomColor
                  ? null
                  : "text-accent-blue/70"
                : "text-stone group-hover:text-on-dark-mute",
            )}
            style={
              active && hasCustomColor
                ? { color: colorVars.fg, opacity: 0.75 }
                : undefined
            }
          >
            {title}
          </span>
        )}
      </span>
      {fileKinds && fileKinds.length > 0 && (
        <DomainKindChips kinds={fileKinds} />
      )}
      {change && (
        <DomainChangeDot
          kind={change}
          colorFg={hasCustomColor ? colorVars.fg : null}
        />
      )}
    </button>
  );
}

/** daily/*.md 파일 옆에 노출되는 trigger kind 미니 chip 행. 글자 없이
 *  점 형태로 컴팩트하게 — 사이드바에 4~6 px 만 차지하면서 "이 파일에는
 *  결정·재발 방지가 있다" 를 한눈에. 자세한 라벨은 hover 툴팁. */
function DomainKindChips({ kinds }: { kinds: TriggerKind[] }) {
  return (
    <span className="flex shrink-0 items-center gap-1">
      {kinds.map((k) => (
        <DomainKindChip key={k} kind={k} />
      ))}
    </span>
  );
}

function DomainKindChip({ kind }: { kind: TriggerKind }) {
  const { label, cls } = (
    {
      decision: { label: "결정", cls: "bg-accent-blue-soft text-accent-blue" },
      cause: { label: "원인", cls: "bg-accent-red-soft text-accent-red" },
      todo: { label: "TODO", cls: "bg-accent-yellow-soft text-accent-yellow" },
      knowhow: {
        label: "노하우",
        cls: "bg-accent-green-soft text-accent-green",
      },
      pitfall: { label: "재발 방지", cls: "bg-surface-elevated text-on-dark" },
      other: { label: "메모", cls: "bg-surface-elevated text-stone" },
    } as const
  )[kind];
  return (
    <span
      className={cn(
        "rounded-xs px-1 py-px text-[9px] font-medium uppercase leading-none tracking-[0.2px]",
        cls,
      )}
      title={label}
    >
      {label}
    </span>
  );
}

/** Lucide icon resolved from `cfg.project_icons[name]`. Falls back to a
 *  plain folder when the user hasn't picked one yet. */
function ProjectIcon({ project }: { project: string }) {
  const iconName = useApp(
    (s) => s.cfg?.project_icons?.[project] ?? null,
  );
  const Icon = projectIconOf(iconName);
  // Inherit the parent button's text color so the icon picks up
  // active/expanded/hover tones automatically.
  return <Icon size={13} className="shrink-0 opacity-80" />;
}

/** Tiny "new"/"modified" indicator. We use a colored dot rather than
 *  text labels to keep rows compact. Tooltip carries the meaning. The
 *  "modified" dot picks up the project's custom color when set so it
 *  matches the rest of that project's identity; "new" always stays green
 *  because the semantic (newly created file) is universal. */
function DomainChangeDot({
  kind,
  colorFg,
}: {
  kind: "new" | "modified";
  colorFg?: string | null;
}) {
  if (kind === "new") {
    return (
      <span
        className="h-1.5 w-1.5 shrink-0 rounded-full bg-accent-green"
        title="신규"
      />
    );
  }
  return (
    <span
      className={cn(
        "h-1.5 w-1.5 shrink-0 rounded-full",
        colorFg ? null : "bg-accent-blue",
      )}
      style={colorFg ? { backgroundColor: colorFg } : undefined}
      title="수정됨"
    />
  );
}

function shortPath(p: string): string {
  const home = "/Users/";
  if (p.startsWith(home)) {
    const rest = p.slice(home.length);
    const slash = rest.indexOf("/");
    if (slash >= 0) return "~/" + rest.slice(slash + 1);
  }
  return p;
}

function SidebarFooter({
  vaultPath,
  onOpenSettings,
}: {
  vaultPath: string | null;
  onOpenSettings: () => void;
}) {
  const cfg = useApp((s) => s.cfg);
  const embedModel = cfg?.embed_model ?? null;
  // 자동화 LLM 모델 — 사용자가 명시 선택했으면 그 값, 아니면 provider
  // 별 디폴트. provider 자체가 없으면 라인 자체 안 노출.
  const providerKind = cfg?.embed_provider?.kind ?? null;
  const automationModel = (() => {
    const explicit = cfg?.automation_model?.trim();
    if (explicit) return explicit;
    if (providerKind === "google") return "gemini-2.5-flash-lite";
    if (providerKind === "bedrock")
      return "us.anthropic.claude-haiku-4-5-20251001-v1:0";
    return null;
  })();

  if (!vaultPath) return null;

  return (
    <footer className="shrink-0 border-t border-hairline px-2 py-2.5">
      <UpdatePill />
      <BgJobPill />
      <div className="flex items-center justify-between gap-2">
        <div
          className="px-1 flex-1 truncate font-mono text-[12px] text-mute"
          title={vaultPath}
        >
          {shortPath(vaultPath)}
        </div>
        <button
          onClick={onOpenSettings}
          title="설정 (⌘,)"
          className="grid h-8 w-8 shrink-0 place-items-center rounded-sm text-mute transition-colors hover:bg-surface-elevated hover:text-on-dark"
        >
          <SettingsIcon size={18} />
        </button>
      </div>
      {/* AI 연동 (cfg.embed_provider) 가 켜져있을 때만 노출.
          - embed: 의미 검색 인덱싱 모델
          - llm  : 요약 / purpose 자동 작성 / ghost 제안에 쓰는 모델 */}
      {(embedModel || automationModel) && (
        <div className="mt-1.5 flex flex-col">
          {embedModel && (
            <ModelLine
              label="embed"
              id={embedModel}
              onClick={onOpenSettings}
            />
          )}
          {automationModel && (
            <ModelLine
              label="llm"
              id={automationModel}
              onClick={onOpenSettings}
            />
          )}
        </div>
      )}
    </footer>
  );
}

/** Tauri updater 신호를 footer pill 로 표시.
 *  - available  : "v{X} 사용 가능" 클릭 → 다운로드 시작
 *  - downloading: 진행 바 (progress 0..1)
 *  - ready      : 다운로드 완료 — relaunch 는 applyPendingUpdate 가 이미
 *                 호출했으므로 사용자가 인지할 짧은 신호만
 *  - error      : 사용자가 닫을 수 있는 에러 toast (자동 8초 후 사라짐) */
function UpdatePill() {
  const info = useApp((s) => s.updateInfo);
  useEffect(() => {
    if (!info || info.status !== "error") return;
    const t = window.setTimeout(() => {
      const cur = useApp.getState().updateInfo;
      if (cur && cur.status === "error") dismissUpdatePill();
    }, 8_000);
    return () => window.clearTimeout(t);
  }, [info]);

  if (!info) return null;

  if (info.status === "available") {
    return (
      <div className="mb-2 rounded-md border border-accent-blue/40 bg-accent-blue-soft/40 px-2.5 py-2">
        <div className="flex items-center gap-2">
          <span className="grid h-4 w-4 shrink-0 place-items-center rounded-full bg-accent-blue text-on-primary">
            <ArrowUpCircle size={11} />
          </span>
          <span className="flex-1 truncate text-[11px] text-accent-blue">
            v{info.version} 사용 가능
          </span>
          <button
            type="button"
            onClick={() => {
              void applyPendingUpdate();
            }}
            className="text-[10px] text-stone hover:text-on-dark"
          >
            설치 →
          </button>
        </div>
        <div className="mt-1 flex items-center gap-2 pl-6 text-[10px] text-stone">
          <button
            type="button"
            onClick={async () => {
              try {
                const { openUrl } = await import("@tauri-apps/plugin-opener");
                await openUrl(
                  `https://github.com/dean-studio/danbi/releases/tag/v${info.version}`,
                );
              } catch (e) {
                console.error("[danbi] open release notes", e);
              }
            }}
            className="hover:text-on-dark"
          >
            업데이트 로그 ↗
          </button>
          <span aria-hidden>·</span>
          <button
            type="button"
            onClick={() => dismissUpdatePill()}
            className="hover:text-on-dark"
          >
            나중에
          </button>
        </div>
      </div>
    );
  }

  if (info.status === "downloading") {
    const pct = Math.max(0, Math.min(100, Math.round(info.progress * 100)));
    return (
      <div className="mb-2 rounded-md border border-hairline bg-surface-elevated px-2.5 py-2">
        <div className="flex items-center gap-2 text-[11px]">
          <span className="grid h-4 w-4 place-items-center rounded-full bg-accent-blue-soft text-[9px] font-bold text-accent-blue">
            ↓
          </span>
          <span className="flex-1 truncate text-body">
            v{info.version} 다운로드 중
          </span>
          <span className="font-mono text-stone">{pct}%</span>
        </div>
        <div className="mt-1.5 h-1 overflow-hidden rounded-full bg-surface">
          <div
            className="h-full bg-accent-blue transition-[width] duration-200 ease-out"
            style={{ width: `${pct}%` }}
          />
        </div>
      </div>
    );
  }

  if (info.status === "ready") {
    return (
      <div className="mb-2 flex w-full items-center gap-2 rounded-md border border-accent-green/40 bg-accent-green-soft/40 px-2.5 py-2">
        <span className="grid h-4 w-4 shrink-0 place-items-center rounded-full bg-accent-green text-[9px] font-bold text-on-primary">
          ✓
        </span>
        <span className="flex-1 truncate text-[11px] text-accent-green">
          v{info.version} 설치 완료 — 재시작 중
        </span>
      </div>
    );
  }

  // error
  return (
    <div className="mb-2 rounded-md border border-accent-red/40 bg-accent-red-soft/40 px-2.5 py-2">
      <div className="flex items-center gap-2 text-[11px]">
        <span className="grid h-4 w-4 place-items-center rounded-full bg-accent-red text-[9px] font-bold text-on-primary">
          !
        </span>
        <span className="flex-1 truncate text-accent-red">
          업데이트 실패
        </span>
        <button
          onClick={() => dismissUpdatePill()}
          className="text-[10px] text-stone hover:text-on-dark"
        >
          닫기
        </button>
      </div>
      <div className="mt-1 truncate font-mono text-[10px] text-mute">
        {info.message}
      </div>
    </div>
  );
}

/** 백그라운드 LLM 작업 (요약 / purpose 작성 / ghost 스캔) 진행 표시.
 *  store.bgJob 을 읽어 footer 상단에 작은 pill 로 띄운다.
 *  - running: indeterminate 진행 바 + 작업 라벨
 *  - done   : 클릭 가능한 결과 ("✓ 새 요약 — 페이지 열기")
 *  - error  : 빨간색 에러 메시지 (자동 사라짐 8초)
 *  None 이면 자체 안 보이게. */
function BgJobPill() {
  const job = useApp((s) => s.bgJob);
  const setJob = useApp((s) => s.setBgJob);
  const [, force] = useState(0);

  // running 일 때 1초 단위로 경과 시간 갱신.
  useEffect(() => {
    if (job?.status !== "running") return;
    const t = window.setInterval(() => force((n) => n + 1), 1000);
    return () => window.clearInterval(t);
  }, [job?.status]);

  // done / error 는 적절히 자동 정리. done 은 사용자가 인지하도록
  // 60초 정도 두고, error 는 8초 후 자동 사라짐.
  useEffect(() => {
    if (!job) return;
    if (job.status === "done") {
      const t = window.setTimeout(() => {
        const cur = useApp.getState().bgJob;
        if (cur && cur.status === "done") setJob(null);
      }, 60_000);
      return () => window.clearTimeout(t);
    }
    if (job.status === "error") {
      const t = window.setTimeout(() => {
        const cur = useApp.getState().bgJob;
        if (cur && cur.status === "error") setJob(null);
      }, 8_000);
      return () => window.clearTimeout(t);
    }
  }, [job, setJob]);

  if (!job) return null;

  // kind 별 라벨 한 줄. summarize 는 domain, compose 는 target, ghost 는
  // 요약된 카운트. project 는 모든 kind 가 가지고 있으니 공통 subline.
  const label = (() => {
    if (job.kind === "summarize") {
      if (job.status === "running") return "요약 중";
      if (job.status === "done") return "요약 완료";
      return "요약 실패";
    }
    if (job.kind === "compose") {
      if (job.status === "running") return `${job.target}.md 작성 중`;
      if (job.status === "done") return `${job.target}.md 작성 완료`;
      return `${job.target}.md 실패`;
    }
    if (job.kind === "vault") {
      const opLabel =
        job.op === "create"
          ? "생성"
          : job.op === "rename"
            ? "이름 변경"
            : "삭제";
      if (job.status === "running") return `프로젝트 ${opLabel} 중`;
      if (job.status === "done") return `프로젝트 ${opLabel} 완료`;
      return `프로젝트 ${opLabel} 실패`;
    }
    // ghost
    if (job.status === "running") return "관련 노트 분석 중";
    if (job.status === "done")
      return job.pendingCount > 0
        ? `${job.pendingCount}개 제안`
        : "추가 제안 없음";
    return "ghost 실패";
  })();
  const subPath =
    job.kind === "summarize"
      ? job.status !== "error"
        ? job.domain
        : (job as { domain: string }).domain
      : job.project;

  if (job.status === "running") {
    const elapsed = Math.max(1, Math.floor((Date.now() - job.startedAt) / 1000));
    return (
      <div className="mb-2 rounded-md border border-hairline bg-surface-elevated px-2.5 py-2">
        <div className="flex items-center gap-2 text-[11px]">
          <span className="grid h-4 w-4 place-items-center rounded-full bg-accent-blue-soft text-[9px] font-bold text-accent-blue">
            ✦
          </span>
          <span className="flex-1 truncate text-body">
            {label} · {subPath}
          </span>
          <span className="font-mono text-stone">{elapsed}s</span>
        </div>
        {/* indeterminate progress bar — 끝점 모르는 LLM 호출이라 이동
            애니메이션만. */}
        <div className="mt-1.5 h-1 overflow-hidden rounded-full bg-surface">
          <div className="danbi-bg-progress-bar h-full w-1/3 bg-accent-blue" />
        </div>
      </div>
    );
  }

  if (job.status === "done") {
    return (
      <button
        type="button"
        onClick={async () => {
          if (job.kind === "summarize") {
            try {
              await ipc.openExport(job.exportId);
            } catch (e) {
              console.error("[danbi] open export", e);
            }
          }
          // compose / ghost 결과는 각자 화면 (PurposeSchemaHint /
          // GraphView) 이 자동 표시 — pill 클릭은 닫기.
          setJob(null);
        }}
        className="mb-2 flex w-full items-center gap-2 rounded-md border border-accent-green/40 bg-accent-green-soft/40 px-2.5 py-2 text-left transition-colors hover:bg-accent-green-soft"
      >
        <span className="grid h-4 w-4 place-items-center rounded-full bg-accent-green text-[9px] font-bold text-on-primary">
          ✓
        </span>
        <span className="flex-1 truncate text-[11px] text-accent-green">
          {label} · {subPath}
        </span>
        {job.kind === "summarize" && (
          <span className="text-[10px] text-stone">페이지 열기 →</span>
        )}
      </button>
    );
  }

  // error
  return (
    <div className="mb-2 rounded-md border border-accent-red/40 bg-accent-red-soft/40 px-2.5 py-2">
      <div className="flex items-center gap-2 text-[11px]">
        <span className="grid h-4 w-4 place-items-center rounded-full bg-accent-red text-[9px] font-bold text-on-primary">
          !
        </span>
        <span className="flex-1 truncate text-accent-red">
          {label} · {subPath}
        </span>
        <button
          onClick={() => setJob(null)}
          className="text-[10px] text-stone hover:text-on-dark"
        >
          닫기
        </button>
      </div>
      <div className="mt-1 truncate font-mono text-[10px] text-mute">
        {job.message}
      </div>
    </div>
  );
}

function ModelLine({
  label,
  id,
  onClick,
}: {
  label: string;
  id: string | null;
  onClick: () => void;
}) {
  return (
    <button
      onClick={onClick}
      title={id ?? "모델 선택"}
      className="group flex w-full items-center gap-1.5 rounded-sm px-1 py-1 text-left transition-colors hover:bg-surface-elevated"
    >
      <span className="w-10 shrink-0 text-[12px] leading-none text-stone">
        {label}
      </span>
      <span className="flex-1 truncate font-mono text-[12px] leading-none text-mute group-hover:text-on-dark">
        {shortModel(id)}
      </span>
    </button>
  );
}

/** 사이드바 헤더 종 아이콘 + popover.
 *  - 안 읽은 알림 N 만큼 빨간 배지
 *  - 클릭 시 list popover 펼침 (요약 완료 / 에러 / 정보)
 *  - 항목 클릭 → 해당 export HTML 페이지 열기 또는 도메인 이동
 *  - "모두 읽음" / "모두 비우기" footer 액션 */
function NotificationBell() {
  const list = useApp((s) => s.notifications);
  const markAll = useApp((s) => s.markAllNotificationsRead);
  const clearAll = useApp((s) => s.clearNotifications);
  const selectDomain = useApp((s) => s.selectDomain);
  const [open, setOpen] = useState(false);
  const unread = list.filter((n) => !n.read).length;
  const btnRef = useRef<HTMLButtonElement | null>(null);
  // Viewport 좌표로 띄워야 사이드바가 좁을 때도 잘리지 않는다.
  // 사이드바는 relative + 옆 MainPanel 이 320px popover 를 시각적으로
  // 가리게 되니까 fixed 로 viewport 에 직접 anchor.
  const [pos, setPos] = useState<{ top: number; left: number } | null>(null);
  const POPOVER_W = 320;
  useEffect(() => {
    if (!open) {
      setPos(null);
      return;
    }
    const place = () => {
      const el = btnRef.current;
      if (!el) return;
      const rect = el.getBoundingClientRect();
      const margin = 8;
      // 버튼 아래로 2px 띄움. 오른쪽 정렬을 기본으로 하되,
      // viewport 왼쪽 가장자리를 침범하면 버튼 좌측 기준으로 조정.
      let left = rect.right - POPOVER_W;
      if (left < margin) left = margin;
      // 오른쪽도 검사 — 작은 디스플레이에서 오버플로 방지.
      const maxLeft = window.innerWidth - POPOVER_W - margin;
      if (left > maxLeft) left = Math.max(margin, maxLeft);
      setPos({ top: rect.bottom + 2, left });
    };
    place();
    window.addEventListener("resize", place);
    window.addEventListener("scroll", place, true);
    return () => {
      window.removeEventListener("resize", place);
      window.removeEventListener("scroll", place, true);
    };
  }, [open]);

  return (
    <div className="relative">
      <button
        ref={btnRef}
        onClick={() => {
          setOpen((v) => !v);
          if (!open && unread > 0) {
            // 페치 직후 다 읽음 처리하면 너무 빨라서 못 본 채 사라짐.
            // 약간 지연 후 read 처리.
            window.setTimeout(() => markAll(), 600);
          }
        }}
        title={unread > 0 ? `알림 ${unread}건` : "알림"}
        className="relative grid h-8 w-8 place-items-center rounded-md text-mute transition-colors hover:bg-surface-elevated hover:text-on-dark"
      >
        <Bell size={16} />
        {unread > 0 && (
          <span className="absolute right-0 top-[1px] grid h-4 min-w-[16px] place-items-center rounded-full bg-accent-red px-1 text-[10px] font-semibold leading-none text-on-dark">
            {unread > 9 ? "9+" : unread}
          </span>
        )}
      </button>
      {open && pos && (
        <>
          <div className="fixed inset-0 z-30" onClick={() => setOpen(false)} />
          <div
            style={{ top: pos.top, left: pos.left, width: POPOVER_W }}
            className="fixed z-40 overflow-hidden rounded-md border border-hairline bg-surface shadow-xl shadow-black/40"
          >
            <div className="flex items-center justify-between border-b border-hairline px-3 py-2 text-[11px] uppercase tracking-[0.5px] text-stone">
              <span>알림</span>
              {list.length > 0 && (
                <button
                  onClick={() => {
                    clearAll();
                    setOpen(false);
                  }}
                  className="text-stone hover:text-on-dark"
                >
                  모두 비우기
                </button>
              )}
            </div>
            <div className="max-h-[60vh] overflow-y-auto">
              {list.length === 0 ? (
                <div className="px-3 py-6 text-center text-[12px] text-stone">
                  알림이 없어요
                </div>
              ) : (
                <ul>
                  {list.map((n) => {
                    const dt = new Date(n.createdAt);
                    const stamp = `${String(dt.getHours()).padStart(2, "0")}:${String(dt.getMinutes()).padStart(2, "0")}`;
                    return (
                      <li key={n.id}>
                        <button
                          type="button"
                          onClick={async () => {
                            setOpen(false);
                            if (n.action?.kind === "open-export") {
                              try {
                                await ipc.openExport(n.action.exportId);
                              } catch (e) {
                                console.error("[danbi] open export", e);
                              }
                            } else if (
                              n.action?.kind === "select-domain"
                            ) {
                              selectDomain(
                                n.action.project,
                                n.action.domain,
                              );
                            } else if (
                              n.action?.kind === "open-compose"
                            ) {
                              // 알림 자체에 매달린 markdown 을 store 에
                              // 박고 해당 노트로 이동. PurposeSchemaHint
                              // 가 mount/sync 시 pendingCompose 를 보고
                              // 자동으로 미리보기 모달을 띄운다 — 사용자
                              // 가 또 LLM 호출할 필요 없음.
                              useApp.getState().setPendingCompose({
                                project: n.action.project,
                                target: n.action.target,
                                markdown: n.action.markdown,
                                provider: "cached",
                              });
                              selectDomain(
                                n.action.project,
                                `${n.action.target}.md`,
                              );
                            } else if (
                              n.action?.kind === "open-graph"
                            ) {
                              // 그래프 뷰는 헤더의 ⌘G 또는 사이드바
                              // Network 아이콘으로 — 알림에서 열고
                              // 싶을 때를 위해 store 의 open-graph
                              // 이벤트를 발화. 메인 윈도우가 listen.
                              try {
                                const { emit } = await import(
                                  "@tauri-apps/api/event"
                                );
                                await emit("danbi:open-graph", {
                                  project: n.action.project,
                                });
                              } catch {
                                /* ignore */
                              }
                            }
                          }}
                          className="flex w-full flex-col gap-0.5 border-b border-hairline px-3 py-2 text-left transition-colors hover:bg-surface-elevated"
                        >
                          <div className="flex items-center gap-1.5">
                            <span
                              className={cn(
                                "inline-block h-1.5 w-1.5 rounded-full",
                                n.tone === "ok"
                                  ? "bg-accent-green"
                                  : n.tone === "err"
                                    ? "bg-accent-red"
                                    : "bg-accent-blue",
                              )}
                            />
                            <span className="flex-1 truncate text-[12px] font-medium text-on-dark">
                              {n.title}
                            </span>
                            <span className="font-mono text-[10px] text-stone">
                              {stamp}
                            </span>
                          </div>
                          {n.body && (
                            <div className="ml-3 truncate text-[11px] text-mute">
                              {n.body}
                            </div>
                          )}
                        </button>
                      </li>
                    );
                  })}
                </ul>
              )}
            </div>
          </div>
        </>
      )}
    </div>
  );
}
