import {
  startTransition,
  useCallback,
  useEffect,
  useRef,
  useState,
} from "react";
import { Check, X } from "lucide-react";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { Dialog } from "@/components/Dialog";
import { ProjectIconPicker } from "@/components/ProjectIconPicker";
import { ProjectColorPicker } from "@/components/ProjectColorPicker";
import { LoadingScreen } from "@/App";
import { AboutDialog } from "@/main/AboutDialog";
import {
  PrimaryButton,
  SecondaryButton,
} from "@/components/WizardShell";
import { CommandBar } from "@/main/CommandBar";
import { CommandPalette, type PaletteCommand, type PaletteMode } from "@/main/CommandPalette";
import { ConnectSheet } from "@/main/ConnectSheet";
import { DocView } from "@/main/DocView";
import { GraphView } from "@/main/GraphView";
import { FOCUS_MEMO_EVENT, Home } from "@/main/Home";
import { ProjectHome } from "@/main/ProjectHome";
import { ReviewView } from "@/main/ReviewView";
import { Settings } from "@/main/Settings";
import { ShortcutsHelp } from "@/main/ShortcutsHelp";
import { TrashPanel } from "@/main/TrashPanel";
import { UsageView } from "@/main/UsageView";
import { Sidebar, markVaultAllRead } from "@/main/Sidebar";
import {
  ipc,
  onReviewsChanged,
  onVaultChanged,
  type VaultChangedPayload,
  type VaultTemplate,
} from "@/lib/ipc";
import { SELF_SAVE_WINDOW_MS, useApp } from "@/state/store";
import { cn } from "@/lib/utils";

type DialogState =
  | { kind: "none" }
  | { kind: "add-project"; groupId?: string }
  | { kind: "add-domain"; project: string; folder?: string | null }
  | { kind: "rename-project"; project: string }
  | { kind: "rename-domain"; project: string; domain: string }
  | { kind: "confirm-delete-project"; project: string }
  | { kind: "confirm-delete-domain"; project: string; domain: string }
  | { kind: "add-folder"; project: string; parent: string | null }
  | { kind: "rename-folder"; project: string; folder: string }
  | { kind: "confirm-delete-folder"; project: string; folder: string };

export function Workspace() {
  const cfg = useApp((s) => s.cfg);
  const setTree = useApp((s) => s.setTree);
  const patchTreeProject = useApp((s) => s.patchTreeProject);
  const selection = useApp((s) => s.selection);
  const clearSelection = useApp((s) => s.clearSelection);
  const selectDomain = useApp((s) => s.selectDomain);
  const setLinkIndex = useApp((s) => s.setLinkIndex);
  const setGroups = useApp((s) => s.setGroups);
  const setProjectUpdates = useApp((s) => s.setProjectUpdates);
  const setDomainUpdates = useApp((s) => s.setDomainUpdates);

  const [dialog, setDialog] = useState<DialogState>({ kind: "none" });
  const [input, setInput] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [refreshKey, setRefreshKey] = useState(0);
  const [iconPickerProject, setIconPickerProject] = useState<string | null>(
    null,
  );
  const [colorPickerProject, setColorPickerProject] = useState<string | null>(
    null,
  );
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [settingsInitial, setSettingsInitial] = useState<
    "appearance" | "shortcuts" | "vault" | "editor" | "mcp" | "vector" | "backup" | "about" | undefined
  >(undefined);
  // ⌘K 통합 팔레트 — SearchPalette + ProjectSwitcher 를 흡수. ⌘P 는
  // 같은 팔레트를 "@" 프로젝트 모드로 연다.
  const [paletteOpen, setPaletteOpen] = useState(false);
  const [paletteMode, setPaletteMode] = useState<PaletteMode>("mixed");
  const [graphOpen, setGraphOpen] = useState(false);
  const [graphProject, setGraphProject] = useState<string | null>(null);
  const [aboutOpen, setAboutOpen] = useState(false);
  const [shortcutsOpen, setShortcutsOpen] = useState(false);
  const [sidebarCollapsed, setSidebarCollapsed] = useState(false);
  const [trashOpen, setTrashOpen] = useState(false);
  const view = useApp((s) => s.view);
  const setView = useApp((s) => s.setView);
  const connectSheet = useApp((s) => s.connectSheet);
  const openConnectSheet = useApp((s) => s.openConnectSheet);
  const closeConnectSheet = useApp((s) => s.closeConnectSheet);
  const [trashCount, setTrashCount] = useState(0);
  // 마지막으로 사용자가 휴지통 패널을 본 시점의 항목 수. 그보다 많아지면
  // 사이드바에 미시 dot 으로 알린다 — 한 번 열어보면 다시 사라진다.
  // localStorage 에 저장해 reload 후에도 상태 유지.
  const TRASH_SEEN_KEY = "danbi.trashSeenCount";
  const [trashSeenCount, setTrashSeenCount] = useState<number>(() => {
    const v = Number(localStorage.getItem(TRASH_SEEN_KEY));
    return Number.isFinite(v) ? v : 0;
  });
  // 휴지통은 팔레트 `>휴지통` 명령으로만 열린다. 새 항목 안내 dot 은
  // 명령 힌트에 표시.
  const trashHasUnseen = trashCount > trashSeenCount;
  const [reviewCount, setReviewCount] = useState(0);

  // Refresh the trash count whenever it might've changed: panel close,
  // vault refresh, etc. The Trash2 badge in the sidebar reads this.
  const refreshTrashCount = useCallback(async () => {
    try {
      const items = await ipc.trashList();
      setTrashCount(items.length);
      // 트래시가 줄어들면 (purge/restore/empty) seen 도 같이 끌어내려야
      // 다음에 새로 삭제했을 때 dot 이 다시 뜬다. seen ≤ count 만 보장하면
      // 충분.
      setTrashSeenCount((prev) => {
        if (prev > items.length) {
          localStorage.setItem(TRASH_SEEN_KEY, String(items.length));
          return items.length;
        }
        return prev;
      });
    } catch {
      /* ignore */
    }
  }, []);

  // 리뷰 배지 — 백엔드가 항목을 넣을 때 `reviews:changed` 를 쏜다
  // (`.danbi/` 는 워처 대상이 아니라 vault:changed 로는 못 받음). 15초
  // 폴링 대신 시작 시 한 번 + 이벤트.
  useEffect(() => {
    let active = true;
    let unlisten: UnlistenFn | null = null;
    async function pull() {
      try {
        const s = await ipc.reviewsList();
        if (active) {
          setReviewCount(
            s.items.filter((it) => it.status === "pending").length,
          );
        }
      } catch {
        /* ignore */
      }
    }
    pull();
    onReviewsChanged(pull).then((fn) => {
      if (active) unlisten = fn;
      else fn();
    });
    return () => {
      active = false;
      if (unlisten) unlisten();
    };
  }, []);

  // Native "About Danbi" menu fires this event (see src-tauri/src/lib.rs).
  // We surface our own dialog so dev builds and bundled builds look alike,
  // and users see the real app icon + description we authored instead of
  // the macOS default folder fallback.
  useEffect(() => {
    let unlistenAbout: UnlistenFn | null = null;
    let unlistenSettings: UnlistenFn | null = null;
    let unlistenOpenDoc: UnlistenFn | null = null;
    let unlistenSelectProject: UnlistenFn | null = null;
    (async () => {
      unlistenAbout = await listen("about:show", () => setAboutOpen(true));
      unlistenSettings = await listen("settings:show", () =>
        setSettingsOpen(true),
      );
      // Quick Capture popup 의 검색 결과 클릭이 emit 하는 이벤트.
      // payload = { project, domain }. selection 만 바꾸고 메인 앱은
      // 알아서 그 도메인 화면으로 라우팅한다.
      unlistenOpenDoc = await listen<{ project: string; domain: string }>(
        "danbi:open-doc",
        (e) => {
          const p = e.payload?.project;
          const d = e.payload?.domain;
          if (typeof p === "string" && typeof d === "string") {
            useApp.getState().selectDomain(p, d);
          }
        },
      );
      // Popover 의 프로젝트 퀵셔트 클릭이 emit. payload = { project }.
      unlistenSelectProject = await listen<{ project: string }>(
        "danbi:select-project",
        (e) => {
          const p = e.payload?.project;
          if (typeof p === "string") {
            useApp.getState().selectProject(p);
          }
        },
      );
    })();
    return () => {
      if (unlistenAbout) unlistenAbout();
      if (unlistenSettings) unlistenSettings();
      if (unlistenOpenDoc) unlistenOpenDoc();
      if (unlistenSelectProject) unlistenSelectProject();
    };
  }, []);
  const [toast, setToast] = useState<{
    tone: "ok" | "err";
    text: string;
  } | null>(null);
  const [templates, setTemplates] = useState<VaultTemplate[]>([]);
  const [selectedTemplateId, setSelectedTemplateId] = useState<string>("");

  useEffect(() => {
    ipc
      .listTemplates()
      .then((list) => setTemplates(list))
      .catch(() => setTemplates([]));
  }, []);

  function flash(tone: "ok" | "err", text: string) {
    setToast({ tone, text });
    setTimeout(() => setToast(null), 2400);
  }

  /** 현재(또는 마지막으로 본) 프로젝트 — ⌘N · ⌘⇧D 의 대상. */
  function currentProjectOrRecent(): string | null {
    const st = useApp.getState();
    if (st.selection.project) return st.selection.project;
    return st.cfg?.capture?.last_project ?? st.tree?.projects[0]?.name ?? null;
  }

  function newNote() {
    const project = currentProjectOrRecent();
    if (!project) {
      setPaletteMode("project");
      setPaletteOpen(true);
      return;
    }
    openDialog({ kind: "add-domain", project, folder: null }, "");
  }

  async function openTodayNote() {
    const project = currentProjectOrRecent();
    if (!project) {
      setPaletteMode("project");
      setPaletteOpen(true);
      return;
    }
    try {
      const domain = await ipc.ensureTodayNote(project);
      useApp.getState().selectDomain(project, domain);
    } catch (e) {
      flash("err", `오늘 노트를 열지 못했어요: ${e}`);
    }
  }

  /** 사이드바 순서(미분류 → 그룹) 로 n번째 프로젝트 — ⌘1~9. */
  function projectByIndex(n: number): string | null {
    const st = useApp.getState();
    const all = st.tree?.projects.map((p) => p.name) ?? [];
    const grouped = new Set(st.groups.flatMap((g) => g.projects));
    const ordered = [
      ...all.filter((p) => !grouped.has(p)),
      ...st.groups.flatMap((g) => g.projects.filter((p) => all.includes(p))),
    ];
    return ordered[n - 1] ?? null;
  }

  useEffect(() => {
    function inEditable(e: KeyboardEvent): boolean {
      const target = e.target as HTMLElement | null;
      const tag = target?.tagName.toLowerCase();
      return (
        tag === "input" ||
        tag === "textarea" ||
        target?.getAttribute("contenteditable") === "true" ||
        !!target?.closest?.("[contenteditable=\"true\"]")
      );
    }
    function inEditorContent(e: KeyboardEvent): boolean {
      const target = e.target as HTMLElement | null;
      return !!target?.closest?.("[contenteditable=\"true\"]");
    }
    function onKey(e: KeyboardEvent) {
      const mod = e.metaKey || e.ctrlKey;
      if (!mod) return;
      // 에디터가 이미 처리한 키(⌘⇧U 밑줄 등)는 화면 전환으로 넘기지 않는다.
      if (e.defaultPrevented) return;
      // ⌘⇧ 조합은 에디터 서식 단축키와 겹치므로 본문 편집 중엔 건너뛴다.
      if (e.shiftKey && inEditorContent(e)) return;
      const key = e.key.toLowerCase();
      if (key === "," && !e.shiftKey) {
        e.preventDefault();
        setSettingsOpen((v) => !v);
      } else if (key === "k" && !e.shiftKey) {
        e.preventDefault();
        setPaletteMode("mixed");
        setPaletteOpen((v) => !v);
      } else if (key === "p" && !e.shiftKey) {
        // 프로젝트 빠른 전환 — 팔레트 @ 모드. 입력 안에서도 브라우저
        // 기본(인쇄)을 막아 흐름이 안 끊기게.
        e.preventDefault();
        setPaletteMode("project");
        setPaletteOpen((v) => !v);
      } else if (key === "g" && !e.shiftKey) {
        e.preventDefault();
        setGraphProject(null);
        setGraphOpen((v) => !v);
      } else if (key === "/" ) {
        e.preventDefault();
        setShortcutsOpen((v) => !v);
      } else if (key === "\\") {
        e.preventDefault();
        setSidebarCollapsed((v) => !v);
      } else if (key === "r" && e.shiftKey) {
        e.preventDefault();
        setView(useApp.getState().view === "review" ? "home" : "review");
      } else if (key === "u" && e.shiftKey) {
        e.preventDefault();
        setView(useApp.getState().view === "usage" ? "home" : "usage");
      } else if (key === "d" && e.shiftKey) {
        e.preventDefault();
        openTodayNote();
      } else if (key === "n" && !e.shiftKey) {
        e.preventDefault();
        newNote();
      } else if (key === "j" && !e.shiftKey) {
        // 홈(오늘)으로 가서 빠른 메모 입력에 포커스. 이미 홈이면 포커스만.
        e.preventDefault();
        useApp.getState().clearSelection();
        requestAnimationFrame(() =>
          window.dispatchEvent(new CustomEvent(FOCUS_MEMO_EVENT)),
        );
      } else if (/^[1-9]$/.test(e.key) && !e.shiftKey && !e.altKey) {
        if (inEditable(e)) return;
        const p = projectByIndex(Number(e.key));
        if (p) {
          e.preventDefault();
          useApp.getState().selectProject(p);
        }
      } else if (e.key === "[" || e.key === "ArrowLeft") {
        // ⌘[ / ⌘← : back. Most external mice with a "back" button send ⌘[ on macOS.
        if (inEditable(e)) return;
        const { project, domain } = useApp.getState().selection;
        if (domain && project) {
          e.preventDefault();
          useApp.getState().selectProject(project);
        } else if (project) {
          e.preventDefault();
          useApp.getState().clearSelection();
        }
      }
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // Mouse back button: domain view → project home → global home.
  useEffect(() => {
    function onMouseDown(e: MouseEvent) {
      if (e.button !== 3) return;
      const { project, domain } = useApp.getState().selection;
      if (domain && project) {
        e.preventDefault();
        useApp.getState().selectProject(project);
      } else if (project) {
        e.preventDefault();
        useApp.getState().clearSelection();
      }
    }
    window.addEventListener("mousedown", onMouseDown);
    window.addEventListener("auxclick", onMouseDown);
    return () => {
      window.removeEventListener("mousedown", onMouseDown);
      window.removeEventListener("auxclick", onMouseDown);
    };
  }, []);

  const vault = cfg?.vault_path ?? null;

  /** vault 상태 재조회. `scope` 가 있으면(워처 이벤트) 그 프로젝트만
   *  다시 긁고 config/휴지통처럼 워처 이벤트로는 바뀌지 않는 것은 건너뛴다.
   *  없으면(초기 로드·사용자 액션) 전체. */
  const refresh = useCallback(
    async (scope?: { projects: string[] }) => {
      if (!vault) return;
      try {
        if (scope) {
          // 바뀐 프로젝트만 교체. 트리가 아직 없으면 전체로 폴백.
          const hasTree = !!useApp.getState().tree;
          if (hasTree) {
            const nodes = await Promise.all(
              scope.projects.map((p) =>
                ipc.listProjectNode(vault, p).then((n) => [p, n] as const),
              ),
            );
            startTransition(() => {
              for (const [p, n] of nodes) patchTreeProject(p, n);
              setRefreshKey((k) => k + 1);
            });
          } else {
            const t = await ipc.listTree(vault);
            startTransition(() => {
              setTree(t);
              setRefreshKey((k) => k + 1);
            });
          }
        } else {
          const t = await ipc.listTree(vault);
          // 큰 vault 에선 setTree → Sidebar/Workspace 전체 재렌더가 16ms 를
          // 가볍게 넘어 input/click event 가 막힌다. startTransition 안에
          // 넣으면 React 가 이 업데이트를 interruptible 로 표시해 사용자
          // 입력에 우선권을 준다 (concurrent rendering).
          startTransition(() => {
            setTree(t);
            setRefreshKey((k) => k + 1);
          });
        }
        ipc
          .buildLinkIndex()
          .then((idx) => startTransition(() => setLinkIndex(idx)))
          .catch(() => {});
        // Update badges — cheap enough to refresh on every tree change
        // (git revwalk with a recent cutoff).
        ipc
          .projectUpdates()
          .then((u) => startTransition(() => setProjectUpdates(u)))
          .catch(() => {});
        ipc
          .domainUpdates()
          .then((u) => startTransition(() => setDomainUpdates(u)))
          .catch(() => {});
        if (scope) return;
        // 그룹(config.json)·휴지통은 워처가 보고하는 경로에 없다 — 사용자
        // 액션 뒤의 전체 refresh 에서만.
        ipc
          .loadConfig()
          .then((cfg) =>
            startTransition(() => setGroups(cfg?.project_groups ?? [])),
          )
          .catch(() => {});
        refreshTrashCount();
      } catch (e) {
        console.error(e);
      }
    },
    [
      vault,
      setTree,
      patchTreeProject,
      setLinkIndex,
      setGroups,
      setProjectUpdates,
      setDomainUpdates,
      refreshTrashCount,
    ],
  );

  // Initial load + start watcher
  useEffect(() => {
    if (!vault) return;
    let unlisten: UnlistenFn | null = null;
    // Debounce vault:changed bursts on the JS side too — even with the
    // backend already deduping by 250ms, if a save triggers multiple
    // file events (e.g. write + atime update on Mac) the cascade of
    // tree + linkIndex + groups + updates IPC was visible as sidebar
    // lag. 200ms is short enough to feel "live" but eats most bursts.
    let pending: ReturnType<typeof setTimeout> | null = null;
    // 디바운스 창 안에 들어온 이벤트의 프로젝트를 합친다. 하나라도
    // truncated(프로젝트 폴더 생성/삭제, 대량 변경)면 전체 refresh.
    let batchProjects = new Set<string>();
    let batchFull = false;
    const scheduleRefresh = (p: VaultChangedPayload) => {
      // 방금 에디터가 저장한 파일 이벤트만 걸러낸다. 같은 이벤트에 섞여
      // 들어온 에이전트 쓰기(다른 파일·다른 프로젝트)는 그대로 처리해야
      // 사이드바 갱신과 열린 문서의 외부 변경 감지가 빠지지 않는다.
      if (!p.truncated && p.paths.length > 0) {
        const now = Date.now();
        const saved = useApp.getState().selfSavedPaths;
        const others = p.paths.filter(
          (path) => !(saved[path] && now - saved[path] < SELF_SAVE_WINDOW_MS),
        );
        if (others.length === 0) return;
        const projects = new Set<string>();
        for (const path of others) {
          const m = /^Projects\/([^/]+)\//.exec(path);
          if (m) projects.add(m[1]);
        }
        if (projects.size === 0) batchFull = true;
        for (const name of projects) batchProjects.add(name);
      } else {
        if (p.truncated || p.projects.length === 0) batchFull = true;
        for (const name of p.projects) batchProjects.add(name);
      }
      if (pending) clearTimeout(pending);
      pending = setTimeout(() => {
        pending = null;
        const projects = [...batchProjects];
        const full = batchFull;
        batchProjects = new Set();
        batchFull = false;
        // 프로젝트 생성·이름 변경·삭제 직후엔 우리가 이미 refresh() 를
        // 불렀으니 watcher refresh 를 한 번 건너뛴다. (문서 저장은 위에서
        // 경로 단위로 걸러지므로 여기 걸리지 않는다.)
        const since = Date.now() - useApp.getState().lastSelfSaveAt;
        if (since < SELF_SAVE_WINDOW_MS) {
          return;
        }
        refresh(full ? undefined : { projects });
      }, 200);
    };
    // 첫 paint 가 끝난 다음 tick 에 무거운 IPC 시작. webview 가 splash
    // 화면을 먼저 그리고 나서 백엔드 호출을 시작해야 macOS 가 webview
    // 를 unresponsive 로 보지 않음 (= 비치볼 spinner 안 뜸).
    requestAnimationFrame(() => {
      requestAnimationFrame(() => {
        (async () => {
          await ipc.initVault(vault);
          await refresh();
          await ipc.startWatching(vault);
          unlisten = await onVaultChanged(scheduleRefresh);
        })();
      });
    });
    return () => {
      if (pending) clearTimeout(pending);
      if (unlisten) unlisten();
      ipc.stopWatching().catch(() => {});
    };
  }, [vault, refresh]);

  function openDialog(d: DialogState, preset = "") {
    setInput(preset);
    setError(null);
    setDialog(d);
    if (d.kind === "add-project") {
      // Default to the same template the vault was initialized with.
      const first =
        templates.find((t) => t.id === "developer")?.id ||
        templates[0]?.id ||
        "";
      setSelectedTemplateId(first);
    }
  }
  function closeDialog() {
    setDialog({ kind: "none" });
    setInput("");
    setError(null);
  }

  /** 백그라운드 vault mutation — 다이얼로그를 즉시 닫고 상단 progress toast
   *  로 진행 상태를 보여준다. handleSubmit 안에서 await 으로 묶으면 큰 vault
   *  에선 IPC + 후속 listTree/buildLinkIndex 가 3-4초 걸려 마우스 spinner 가
   *  떠 있어 답답하다.
   *
   *  react 의 click handler 는 동기 블록으로 끝나야 paint 가 들어오는데,
   *  그 안에서 곧장 IIFE 를 깨우면 await 다음 micro-task 가 paint 보다 먼저
   *  실행돼 결국 화면이 잠깐 굳어 보였다. requestAnimationFrame + setTimeout
   *  으로 한 frame 늦춰 dialog 가 사라지고 toast 가 그려지는 paint 가 먼저
   *  일어나도록 강제한다. */
  function runVaultMutation(
    op: "create" | "rename" | "delete",
    project: string,
    work: () => Promise<void>,
  ) {
    const setBgJob = useApp.getState().setBgJob;
    setBgJob({
      kind: "vault",
      op,
      status: "running",
      project,
      startedAt: Date.now(),
    });
    requestAnimationFrame(() => {
      setTimeout(async () => {
        const t0 = performance.now();
        try {
          await work();
          const t1 = performance.now();
          console.log(`[danbi] vault.${op} ipc: ${(t1 - t0).toFixed(0)}ms`);
          useApp.getState().markSelfSave();
          await refresh();
          const t2 = performance.now();
          console.log(`[danbi] vault.${op} refresh: ${(t2 - t1).toFixed(0)}ms`);
          setBgJob({
            kind: "vault",
            op,
            status: "done",
            project,
            finishedAt: Date.now(),
          });
        } catch (e) {
          setBgJob({
            kind: "vault",
            op,
            status: "error",
            project,
            message: String(e),
            finishedAt: Date.now(),
          });
        }
      }, 0);
    });
  }

  async function handleSubmit() {
    if (!vault) return;
    setError(null);
    try {
      switch (dialog.kind) {
        case "add-project": {
          const name = input.trim();
          if (!name) throw new Error("이름을 입력하세요");
          const tpl = templates.find((t) => t.id === selectedTemplateId);
          const domains = tpl?.default_domains ?? [];
          const folders =
            tpl?.default_folders ?? cfg?.default_folders ?? ["daily"];
          const targetGroupId = dialog.groupId;
          closeDialog();
          runVaultMutation("create", name, async () => {
            await ipc.createProject(vault, name, domains, folders);
            // 그룹 컨텍스트로 호출됐으면 새 프로젝트를 그 그룹에 즉시 편입.
            // (그룹 멤버십은 cfg.project_groups 에 저장 — 프로젝트 자체와는
            //  별도 트랜잭션이라 vault mutation 끝난 뒤에 한 번 덧붙인다.)
            if (targetGroupId) {
              const cur = useApp.getState().groups;
              const next = cur.map((g) =>
                g.id === targetGroupId
                  ? { ...g, projects: [...g.projects, name] }
                  : g,
              );
              const saved = await ipc.groupsSet(next);
              useApp.getState().setGroups(saved);
            }
          });
          return;
        }
        case "add-domain": {
          const name = input.trim();
          if (!name) throw new Error("이름을 입력하세요");
          if (name.includes("/"))
            throw new Error("파일 이름에 슬래시(/)는 사용할 수 없어요");
          // 폴더 컨텍스트에서 호출됐으면 그 폴더 안에 만들도록 prefix 붙임.
          const fullName = dialog.folder ? `${dialog.folder}/${name}` : name;
          const file = await ipc.createDomain(vault, dialog.project, fullName);
          selectDomain(dialog.project, file);
          break;
        }
        case "rename-project": {
          const name = input.trim();
          if (!name) throw new Error("이름을 입력하세요");
          const oldName = dialog.project;
          // 선택 상태는 optimistic 으로 즉시 갱신 — 사용자는 ProjectHome 가
          // 새 이름으로 곧장 보이길 기대한다. IPC 가 실패하면 watcher refresh
          // 로 자동 정정.
          if (selection.project === oldName) {
            if (selection.domain) selectDomain(name, selection.domain);
            else useApp.getState().selectProject(name);
          }
          closeDialog();
          runVaultMutation("rename", name, () =>
            ipc.renameProject(vault, oldName, name),
          );
          return;
        }
        case "rename-domain": {
          const name = input.trim();
          if (!name) throw new Error("이름을 입력하세요");
          const file = await ipc.renameDomain(
            vault,
            dialog.project,
            dialog.domain,
            name,
          );
          if (
            selection.project === dialog.project &&
            selection.domain === dialog.domain
          ) {
            selectDomain(dialog.project, file);
          }
          break;
        }
        case "confirm-delete-project": {
          const proj = dialog.project;
          if (selection.project === proj) clearSelection();
          closeDialog();
          runVaultMutation("delete", proj, () =>
            ipc.deleteProject(vault, proj),
          );
          return;
        }
        case "confirm-delete-domain": {
          await ipc.deleteDomain(vault, dialog.project, dialog.domain);
          if (
            selection.project === dialog.project &&
            selection.domain === dialog.domain
          ) {
            clearSelection();
          }
          break;
        }
        case "add-folder": {
          const name = input.trim();
          if (!name) throw new Error("폴더 이름을 입력하세요");
          if (name.includes("/"))
            throw new Error("폴더 이름에 슬래시(/)는 사용할 수 없어요");
          // 부모 폴더가 있으면 거기에 nested 로 생성. backend 가
          // 깊이 2 까지만 허용하니 부모가 이미 nested 면 거부됨.
          const fullPath = dialog.parent
            ? `${dialog.parent}/${name}`
            : name;
          await ipc.createFolder(vault, dialog.project, fullPath);
          break;
        }
        case "rename-folder": {
          const name = input.trim();
          if (!name) throw new Error("새 이름을 입력하세요");
          if (name.includes("/"))
            throw new Error("새 이름에 슬래시(/)는 사용할 수 없어요");
          // 사용자는 마지막 segment 만 편집하니, parent 를 다시 붙여
          // full path 를 만든 뒤 백엔드로 넘긴다.
          const segments = dialog.folder.split("/");
          segments[segments.length - 1] = name;
          const newFull = segments.join("/");
          await ipc.renameFolder(
            vault,
            dialog.project,
            dialog.folder,
            newFull,
          );
          // 현재 선택한 도메인이 이 폴더 안이면 선택 키도 따라 갱신.
          if (
            selection.project === dialog.project &&
            selection.domain?.startsWith(`${dialog.folder}/`)
          ) {
            const file = selection.domain.slice(dialog.folder.length + 1);
            selectDomain(dialog.project, `${newFull}/${file}`);
          }
          break;
        }
        case "confirm-delete-folder": {
          await ipc.deleteFolder(vault, dialog.project, dialog.folder);
          if (
            selection.project === dialog.project &&
            selection.domain?.startsWith(`${dialog.folder}/`)
          ) {
            clearSelection();
          }
          break;
        }
      }
      closeDialog();
      // mutate 직후 watcher 가 ~200ms 안에 vault:changed 를 또 쏘는데, 우리가
      // 방금 정확한 상태로 refresh() 를 부를 거라 그 watcher refresh 를
      // 건너뛰고 싶다. lastSelfSaveAt 을 찍어 두면 scheduleRefresh 의
      // 1.5s skip-window 에 걸려 중복 listTree+buildLinkIndex+projectUpdates
      // +domainUpdates 가 안 돈다 → 마우스 progress 가 1 cycle 만에 끝남.
      useApp.getState().markSelfSave();
      await refresh();
    } catch (e) {
      setError(String(e));
    }
  }

  const onEnter = (e: React.KeyboardEvent<HTMLDivElement>) => {
    if (e.key === "Enter") handleSubmit();
  };

  // ⌘K 팔레트 `>` 명령 — 사이드바 헤더에서 뺀 아이콘들(그래프·휴지통·
  // 새로고침·모두 읽음)과 화면 이동이 여기로 모인다.
  const paletteCommands: PaletteCommand[] = [
    { id: "today", label: "오늘 노트 열기", shortcut: "⌘⇧D", run: openTodayNote, keywords: ["daily", "데일리"] },
    { id: "new-note", label: "새 노트", shortcut: "⌘N", run: newNote },
    { id: "new-project", label: "새 프로젝트", run: () => openDialog({ kind: "add-project" }) },
    { id: "home", label: "오늘(홈)으로", shortcut: "⌘J", run: () => useApp.getState().clearSelection() },
    { id: "review", label: "검토함", shortcut: "⌘⇧R", run: () => setView("review") },
    { id: "usage", label: "사용량", shortcut: "⌘⇧U", run: () => setView("usage") },
    { id: "graph", label: "그래프", shortcut: "⌘G", run: () => { setGraphProject(useApp.getState().selection.project); setGraphOpen(true); } },
    { id: "connect", label: "연결 관리 (Claude Code · Codex · Cursor)", run: () => openConnectSheet(useApp.getState().selection.project), keywords: ["mcp", "skill", "cursor", "codex", "claude"] },
    { id: "trash", label: trashHasUnseen ? "휴지통 · 새 항목" : "휴지통", hint: trashCount > 0 ? `${trashCount}개` : undefined, run: () => { setTrashOpen(true); setTrashSeenCount(trashCount); localStorage.setItem(TRASH_SEEN_KEY, String(trashCount)); } },
    { id: "mark-read", label: "모두 읽음으로 표시", run: () => { markVaultAllRead().then(() => refresh()); } },
    { id: "refresh", label: "새로고침 (vault 다시 읽기)", run: () => { refresh(); } },
    { id: "settings", label: "설정", shortcut: "⌘,", run: () => setSettingsOpen(true) },
    { id: "settings-ai", label: "설정 · AI 연동", run: () => { setSettingsInitial("vector"); setSettingsOpen(true); } },
    { id: "shortcuts", label: "단축키", shortcut: "⌘/", run: () => setShortcutsOpen(true) },
    { id: "about", label: "단비 정보", run: () => setAboutOpen(true) },
  ];

  // 첫 진입 splash — tree 가 도착하기 전엔 App.tsx 의 단비 LoadingScreen
  // 그대로 보여줌. App splash 와 같은 화면이라 시각적 끊김 없이 로딩 →
  // 실 화면 swap 이 자연스럽다. macOS 비치볼이 webview 위에서 빈 사이드바
  // 위로 뜨는 케이스도 같이 막힌다.
  const treeReady = useApp((s) => s.tree !== null);
  if (!treeReady) {
    return <LoadingScreen />;
  }

  return (
    <>
      {/* BgJobToast 우상단 toast 는 너무 어수선해서 숨김. 진행 상태는
          사이드바 footer 의 BgJobPill 만으로 충분. */}
      <div className="flex h-full w-full flex-col">
        {/* NoLlmBanner 숨김 — 단비는 LLM 없이 동작하는 게 기본. 사용자가
            나중에 다시 켜고 싶으면 이 블록 다시 노출하면 됨.
        <NoLlmBanner
          providerKind={cfg?.provider?.kind}
          preset={cfg?.preset ?? null}
          onConnect={() => {
            setSettingsInitial("llm");
            setSettingsOpen(true);
          }}
        />
        */}
        <div className="flex min-h-0 flex-1">
        {/* ⌘\ 접기 — 사이드바 자체 상태(폭·펼침)는 유지한 채 숨긴다. */}
        <div className={cn("flex min-h-0 shrink-0", sidebarCollapsed && "hidden")}>
        <Sidebar
          onAddProject={() => openDialog({ kind: "add-project" })}
          onAddProjectToGroup={(groupId) =>
            openDialog({ kind: "add-project", groupId })
          }
          onAddDomain={(project) =>
            openDialog({ kind: "add-domain", project, folder: null }, "")
          }
          onAddDomainInFolder={(project, folder) =>
            openDialog({ kind: "add-domain", project, folder }, "")
          }
          onRenameProject={(project) =>
            openDialog({ kind: "rename-project", project }, project)
          }
          onDeleteProject={(project) =>
            openDialog({ kind: "confirm-delete-project", project })
          }
          onRenameDomain={(project, domain) =>
            openDialog(
              { kind: "rename-domain", project, domain },
              domain.replace(/\.md$/i, ""),
            )
          }
          onDeleteDomain={(project, domain) =>
            openDialog({ kind: "confirm-delete-domain", project, domain })
          }
          onOpenSettings={() => setSettingsOpen(true)}
          onOpenPalette={() => {
            setPaletteMode("mixed");
            setPaletteOpen(true);
          }}
          onOpenGraph={() => {
            setGraphProject(null);
            setGraphOpen(true);
          }}
          reviewCount={reviewCount}
          onOpenConnect={openConnectSheet}
          onNewNote={newNote}
          onChangeProjectIcon={(project) => setIconPickerProject(project)}
          onChangeProjectColor={(project) => setColorPickerProject(project)}
          onMarkProjectRead={async (project) => {
            try {
              await ipc.projectMarkAllRead(project);
              // Optimistic local clear so the dots disappear before
              // the watcher round-trip refreshes us.
              const s = useApp.getState();
              const nextDomains = { ...s.domainUpdates };
              for (const k of Object.keys(nextDomains)) {
                if (k.startsWith(`${project}/`)) delete nextDomains[k];
              }
              s.setDomainUpdates(nextDomains);
              s.clearProjectUpdate(project);
            } catch (e) {
              console.error(e);
            }
          }}
          onAddFolder={(project) =>
            openDialog({ kind: "add-folder", project, parent: null }, "")
          }
          onAddSubFolder={(project, parent) =>
            openDialog({ kind: "add-folder", project, parent }, "")
          }
          onRenameFolder={(project, folder) => {
            // 이름 바꾸기 다이얼로그엔 폴더의 마지막 segment 만 채워두면
            // 사용자가 그 부분만 고치게 된다 (parent 는 보존).
            const last = folder.split("/").pop() ?? folder;
            openDialog({ kind: "rename-folder", project, folder }, last);
          }}
          onDeleteFolder={(project, folder) =>
            openDialog({ kind: "confirm-delete-folder", project, folder })
          }
          onMoveDomain={async (project, domain, toFolder) => {
            if (!vault) return;
            try {
              const newName = await ipc.moveDomain(
                vault,
                project,
                domain,
                toFolder,
              );
              // 현재 선택을 따라 이동.
              if (
                selection.project === project &&
                selection.domain === domain
              ) {
                selectDomain(project, newName);
              }
              await refresh();
            } catch (e) {
              console.error("[danbi] move_domain failed", e);
            }
          }}
        />
        </div>
        <div className="flex flex-1 min-w-0 flex-col">
          <div className="flex-1 min-h-0 flex flex-col">
            {view === "review" ? (
              <ReviewView
                project={selection.project}
                onOpenDoc={(p, d) => useApp.getState().selectDomain(p, d)}
                onOpenGraph={(p) => {
                  setGraphProject(p);
                  setGraphOpen(true);
                }}
              />
            ) : view === "usage" ? (
              <UsageView />
            ) : selection.project && selection.domain ? (
              <div className="flex-1 min-h-0">
                <DocView
                  refreshKey={refreshKey}
                  onOpenGraph={() => {
                    // 현재 도메인을 spotlight 한 채로 그래프 뷰 열기.
                    // GraphView 가 initialProject 를 받아 자동 zoom 한다.
                    setGraphProject(useApp.getState().selection.project);
                    setGraphOpen(true);
                  }}
                  onRenameDomain={(project, domain) =>
                    openDialog(
                      { kind: "rename-domain", project, domain },
                      domain.replace(/\.md$/i, ""),
                    )
                  }
                  onDeleteDomain={(project, domain) =>
                    openDialog({ kind: "confirm-delete-domain", project, domain })
                  }
                />
              </div>
            ) : selection.project ? (
              <ProjectHome
                project={selection.project}
                onAddDomain={(project) =>
                  openDialog({ kind: "add-domain", project, folder: null }, "")
                }
                onOpenGraph={(project) => {
                  setGraphProject(project);
                  setGraphOpen(true);
                }}
              />
            ) : (
              <Home onOpenShortcuts={() => setShortcutsOpen(true)} />
            )}
          </div>
          {/* CommandBar 는 LLM provider 가 연결된 경우에만 노출. 단비
              기본 사용은 에디터 + 검색 + MCP 라 미연결 상태에선 빈 입력
              란을 볼 이유가 없음. */}
          {cfg?.provider && <CommandBar />}
        </div>
        </div>
      </div>

      <Dialog
        open={dialog.kind === "add-project"}
        onClose={closeDialog}
        title="새 프로젝트"
        width={560}
        footer={<DialogFooter onCancel={closeDialog} onSubmit={handleSubmit} />}
      >
        <DialogForm
          label="프로젝트 이름"
          input={input}
          setInput={setInput}
          placeholder="예: 상식이"
          onEnter={onEnter}
          error={error}
          hint="템플릿을 고르면 기본 도메인 파일과 폴더가 함께 생성됩니다."
        />
        {templates.length > 0 && (
          <div className="mt-4">
            <div className="mb-1.5 text-caption-sm uppercase tracking-[0.4px] text-mute">
              템플릿
            </div>
            <div className="grid grid-cols-2 gap-2">
              {templates.map((t) => {
                const active = t.id === selectedTemplateId;
                return (
                  <button
                    type="button"
                    key={t.id}
                    onClick={() => setSelectedTemplateId(t.id)}
                    className={
                      "flex flex-col items-stretch rounded-md border p-2.5 text-left transition-colors " +
                      (active
                        ? "border-accent-blue bg-accent-blue-soft"
                        : "border-hairline bg-surface hover:border-hairline-strong")
                    }
                  >
                    <div className="flex items-center justify-between">
                      <span
                        className={
                          "text-[13px] font-medium " +
                          (active ? "text-on-dark" : "text-ink")
                        }
                      >
                        {t.name}
                      </span>
                      {active && (
                        <span className="rounded-xs bg-accent-blue-soft px-1 py-0.5 text-[10px] font-medium uppercase tracking-[0.4px] text-accent-blue">
                          선택
                        </span>
                      )}
                    </div>
                    <div className="mt-1 line-clamp-2 text-caption-sm leading-[1.4] text-body">
                      {t.description}
                    </div>
                    {(t.default_domains.length > 0 ||
                      t.default_folders.length > 0) && (
                      <div className="mt-1.5 flex flex-wrap gap-1">
                        {t.default_domains.slice(0, 3).map((d) => (
                          <span
                            key={d}
                            className="inline-flex items-center rounded-xs bg-surface-elevated px-1.5 py-0.5 font-mono text-[10px] text-on-dark-mute"
                          >
                            {d}
                          </span>
                        ))}
                        {t.default_folders.map((f) => (
                          <span
                            key={`f-${f}`}
                            className="inline-flex items-center rounded-xs bg-surface-elevated px-1.5 py-0.5 font-mono text-[10px] text-accent-blue"
                          >
                            {f}/
                          </span>
                        ))}
                      </div>
                    )}
                  </button>
                );
              })}
            </div>
          </div>
        )}
      </Dialog>

      <Dialog
        open={dialog.kind === "add-domain"}
        onClose={closeDialog}
        title={
          dialog.kind === "add-domain"
            ? `새 도메인 · ${dialog.project}${
                dialog.folder ? `/${dialog.folder}` : ""
              }`
            : ""
        }
        footer={<DialogFooter onCancel={closeDialog} onSubmit={handleSubmit} />}
      >
        <DialogForm
          label="도메인 파일명"
          input={input}
          setInput={setInput}
          placeholder="예: api"
          onEnter={onEnter}
          error={error}
          hint={
            dialog.kind === "add-domain" && dialog.folder
              ? `${dialog.folder}/ 안에 만들어집니다. .md 는 자동으로 붙어요.`
              : ".md 확장자는 자동으로 붙습니다."
          }
          monospace
        />
      </Dialog>

      <Dialog
        open={dialog.kind === "rename-project"}
        onClose={closeDialog}
        title={
          dialog.kind === "rename-project"
            ? `프로젝트 이름 바꾸기 · ${dialog.project}`
            : ""
        }
        footer={<DialogFooter onCancel={closeDialog} onSubmit={handleSubmit} />}
      >
        <DialogForm
          label="새 이름"
          input={input}
          setInput={setInput}
          onEnter={onEnter}
          error={error}
        />
      </Dialog>

      <Dialog
        open={dialog.kind === "rename-domain"}
        onClose={closeDialog}
        title={
          dialog.kind === "rename-domain"
            ? `도메인 이름 바꾸기 · ${dialog.project}/${dialog.domain}`
            : ""
        }
        footer={<DialogFooter onCancel={closeDialog} onSubmit={handleSubmit} />}
      >
        <DialogForm
          label="새 파일명"
          input={input}
          setInput={setInput}
          onEnter={onEnter}
          error={error}
          hint=".md 확장자는 자동으로 붙습니다."
          monospace
        />
      </Dialog>

      <Dialog
        open={dialog.kind === "confirm-delete-project"}
        onClose={closeDialog}
        title="프로젝트 삭제"
        footer={
          <DialogFooter
            onCancel={closeDialog}
            onSubmit={handleSubmit}
            submitLabel="삭제"
            danger
          />
        }
      >
        <div className="text-[14px] leading-[1.6] text-body">
          <span className="font-mono text-ink">
            {dialog.kind === "confirm-delete-project" ? dialog.project : ""}
          </span>{" "}
          프로젝트와 그 안의 모든 도메인 파일을 삭제합니다. vault 가 git
          저장소라 모든 변경은 자동 커밋되어 있어요 — 실수했다면 vault
          폴더에서 <code className="rounded-xs bg-surface-elevated px-1 py-0.5 font-mono text-[12px] text-on-dark-mute">git log</code> /{" "}
          <code className="rounded-xs bg-surface-elevated px-1 py-0.5 font-mono text-[12px] text-on-dark-mute">git checkout</code>{" "}
          으로 되살릴 수 있습니다.
        </div>
        {error && (
          <div className="mt-3 rounded-md border border-hairline bg-surface-elevated p-2 font-mono text-[12px] text-accent-red">
            {error}
          </div>
        )}
      </Dialog>

      <Dialog
        open={dialog.kind === "confirm-delete-domain"}
        onClose={closeDialog}
        title="도메인 삭제"
        footer={
          <DialogFooter
            onCancel={closeDialog}
            onSubmit={handleSubmit}
            submitLabel="삭제"
            danger
          />
        }
      >
        <div className="text-[14px] leading-[1.6] text-body">
          <span className="font-mono text-ink">
            {dialog.kind === "confirm-delete-domain"
              ? `${dialog.project}/${dialog.domain}`
              : ""}
          </span>{" "}
          파일을 삭제합니다.
        </div>
        {error && (
          <div className="mt-3 rounded-md border border-hairline bg-surface-elevated p-2 font-mono text-[12px] text-accent-red">
            {error}
          </div>
        )}
      </Dialog>

      <Dialog
        open={dialog.kind === "add-folder"}
        onClose={closeDialog}
        title={
          dialog.kind === "add-folder"
            ? `새 폴더 · ${dialog.project}${
                dialog.parent ? `/${dialog.parent}` : ""
              }`
            : ""
        }
        footer={<DialogFooter onCancel={closeDialog} onSubmit={handleSubmit} />}
      >
        <DialogForm
          label="폴더 이름"
          input={input}
          setInput={setInput}
          placeholder="예: stats, 2026-01-05"
          onEnter={onEnter}
          error={error}
          hint={
            dialog.kind === "add-folder" && dialog.parent
              ? `${dialog.parent}/ 안에 만들어집니다. 슬래시(/)는 사용할 수 없어요.`
              : "프로젝트 직속 폴더로 만들어집니다. 폴더 안에 또 한 단계 폴더를 만들 수 있어요. 슬래시(/)는 사용할 수 없습니다."
          }
          monospace
        />
      </Dialog>

      <Dialog
        open={dialog.kind === "rename-folder"}
        onClose={closeDialog}
        title={
          dialog.kind === "rename-folder"
            ? `폴더 이름 바꾸기 · ${dialog.project}/${dialog.folder}`
            : ""
        }
        footer={<DialogFooter onCancel={closeDialog} onSubmit={handleSubmit} />}
      >
        <DialogForm
          label="새 폴더 이름"
          input={input}
          setInput={setInput}
          onEnter={onEnter}
          error={error}
          monospace
        />
      </Dialog>

      <Dialog
        open={dialog.kind === "confirm-delete-folder"}
        onClose={closeDialog}
        title="폴더 삭제"
        footer={
          <DialogFooter
            onCancel={closeDialog}
            onSubmit={handleSubmit}
            submitLabel="삭제"
            danger
          />
        }
      >
        <div className="text-[14px] leading-[1.6] text-body">
          <span className="font-mono text-ink">
            {dialog.kind === "confirm-delete-folder"
              ? `${dialog.project}/${dialog.folder}/`
              : ""}
          </span>{" "}
          폴더와 그 안의 모든 파일을 삭제합니다. 되돌릴 수 없으니 주의하세요.
        </div>
        {error && (
          <div className="mt-3 rounded-md border border-hairline bg-surface-elevated p-2 font-mono text-[12px] text-accent-red">
            {error}
          </div>
        )}
      </Dialog>

      <Settings
        open={settingsOpen}
        onClose={() => {
          setSettingsOpen(false);
          setSettingsInitial(undefined);
        }}
        initialSection={settingsInitial}
      />

      <CommandPalette
        open={paletteOpen}
        mode={paletteMode}
        onClose={() => setPaletteOpen(false)}
        // 프로젝트 홈에서만 범위를 미리 고정 — 문서를 보다 연 ⌘K 는 vault 전체.
        initialScope={selection.domain ? null : selection.project}
        commands={paletteCommands}
      />

      <ConnectSheet
        open={connectSheet.open}
        project={connectSheet.project}
        onClose={closeConnectSheet}
        onOpenDoc={(p, d) => useApp.getState().selectDomain(p, d)}
      />

      <ShortcutsHelp open={shortcutsOpen} onClose={() => setShortcutsOpen(false)} />

      <TrashPanel
        open={trashOpen}
        onClose={() => {
          setTrashOpen(false);
          refreshTrashCount();
        }}
        onAfterChange={() => {
          // 복원 / 영구 삭제 후 사이드바 + 카운트 즉시 갱신.
          refresh();
        }}
      />

      <GraphView
        open={graphOpen}
        initialProject={graphProject}
        onClose={() => setGraphOpen(false)}
      />

      <AboutDialog open={aboutOpen} onClose={() => setAboutOpen(false)} />

      <Dialog
        open={iconPickerProject !== null}
        onClose={() => setIconPickerProject(null)}
        title={`아이콘 선택 — ${iconPickerProject ?? ""}`}
        width={520}
      >
        {iconPickerProject && (
          <ProjectIconPicker
            value={cfg?.project_icons?.[iconPickerProject] ?? null}
            onSelect={async (iconName) => {
              if (!cfg?.vault_path) return;
              const next = {
                ...cfg,
                project_icons: {
                  ...(cfg.project_icons ?? {}),
                  [iconPickerProject]: iconName,
                },
              };
              await ipc.saveConfig(cfg.vault_path, next);
              useApp.getState().setCfg(next);
              setIconPickerProject(null);
            }}
            onClear={async () => {
              if (!cfg?.vault_path) return;
              const nextIcons = { ...(cfg.project_icons ?? {}) };
              delete nextIcons[iconPickerProject];
              const next = { ...cfg, project_icons: nextIcons };
              await ipc.saveConfig(cfg.vault_path, next);
              useApp.getState().setCfg(next);
              setIconPickerProject(null);
            }}
            onClose={() => setIconPickerProject(null)}
          />
        )}
      </Dialog>

      <Dialog
        open={colorPickerProject !== null}
        onClose={() => setColorPickerProject(null)}
        title={`색상 선택 — ${colorPickerProject ?? ""}`}
        width={420}
      >
        {colorPickerProject && (
          <ProjectColorPicker
            value={cfg?.project_colors?.[colorPickerProject] ?? null}
            onSelect={async (key) => {
              // 색은 closure 안의 cfg snapshot 이 stale 일 수 있어서 (다른
               // 컴포넌트가 setCfg 한 직후 picker 가 열려 있는 경우) 저장
               // 직전에 store 의 최신 cfg 를 다시 읽고, 그 위에 우리 patch
               // 만 얹는다. 큰 cfg 의 다른 필드를 덮어쓰는 사고 방지.
              const live = useApp.getState().cfg;
              if (!live?.vault_path) return;
              const next = {
                ...live,
                project_colors: {
                  ...(live.project_colors ?? {}),
                  [colorPickerProject]: key,
                },
              };
              await ipc.saveConfig(live.vault_path, next);
              useApp.getState().setCfg(next);
              setColorPickerProject(null);
            }}
            onClear={async () => {
              const live = useApp.getState().cfg;
              if (!live?.vault_path) return;
              const nextColors = { ...(live.project_colors ?? {}) };
              delete nextColors[colorPickerProject];
              const next = { ...live, project_colors: nextColors };
              await ipc.saveConfig(live.vault_path, next);
              useApp.getState().setCfg(next);
              setColorPickerProject(null);
            }}
            onClose={() => setColorPickerProject(null)}
          />
        )}
      </Dialog>

      {toast && (
        <div className="pointer-events-none fixed bottom-20 left-1/2 z-50 -translate-x-1/2">
          <div
            className={
              // 배경은 vault surface 위로 완전 불투명하게 — 뒤 글자가
              // 비춰서 안 읽히는 문제 해결. 폰트도 키워서 가독성 확보.
              "pointer-events-auto inline-flex max-w-[560px] items-start gap-3 rounded-lg border bg-surface-elevated px-5 py-4 text-[14px] leading-[1.55] " +
              (toast.tone === "ok"
                ? "border-accent-green text-ink"
                : "border-accent-red text-ink")
            }
          >
            <span
              className={
                "mt-1 inline-block h-2 w-2 shrink-0 rounded-full " +
                (toast.tone === "ok" ? "bg-accent-green" : "bg-accent-red")
              }
            />
            <span className="flex-1">{toast.text}</span>
          </div>
        </div>
      )}

      {/* Global store-level toast — Sidebar / 다른 컴포넌트들이 어디서든
          호출할 수 있는 글로벌 슬롯. 우상단에 띄워서 위 toast 와 충돌
          안 하게. */}
      <GlobalToast />
    </>
  );
}

function GlobalToast() {
  const t = useApp((s) => s.toast);
  if (!t) return null;
  return (
    <div className="pointer-events-none fixed right-6 top-14 z-[60]">
      <div
        className={
          "pointer-events-auto inline-flex max-w-[440px] items-start gap-3 rounded-lg border bg-surface-elevated px-4 py-3 text-[13px] leading-[1.55] " +
          (t.tone === "ok"
            ? "border-accent-green text-ink"
            : "border-accent-red text-ink")
        }
      >
        <span
          className={
            "mt-1 inline-block h-2 w-2 shrink-0 rounded-full " +
            (t.tone === "ok" ? "bg-accent-green" : "bg-accent-red")
          }
        />
        <span className="flex-1">{t.text}</span>
      </div>
    </div>
  );
}

/**
 * Top strip shown when no LLM provider is configured. Amber hairline
 * card — not error red because the local-only features still work, it's
 * just a nudge to connect when the user wants AI capabilities.
 */
// 미연결 안내 배너. 현재는 위쪽 JSX 에서 주석으로 가려두었지만 향후 LLM
// 옵션 켤 때 다시 노출하기 위해 정의는 보존. export 로 빼서 unused TS 경고
// 회피.
export function NoLlmBanner({
  providerKind,
  preset,
  onConnect,
}: {
  providerKind?: string;
  preset: string | null;
  onConnect: () => void;
}) {
  // 사용자 요청: LLM 미연결 배너 자체를 안 보이게.
  // 단비는 LLM 없이 동작하는 게 기본 사용 패턴. 옛 vault 의 preset 이
  // null 이어도 nag 하지 않도록 무조건 null. 나중에 다시 노출하려면
  // 아래 한 줄을 지우면 기존 분기 (preset === "claude_code", providerKind
  // 체크) 가 그대로 동작.
  return null;
  // eslint-disable-next-line @typescript-eslint/no-unreachable
  if (preset === "claude_code") return null;
  if (providerKind) return null;
  return (
    <div className="flex shrink-0 items-center justify-between gap-4 border-b border-accent-yellow/40 bg-accent-yellow-soft px-5 py-2.5 text-[12px]">
      <div className="flex flex-col gap-1">
        <div className="flex items-center gap-2 font-medium text-accent-yellow">
          <span className="inline-flex h-1.5 w-1.5 rounded-full bg-accent-yellow" />
          LLM 미연결
        </div>
        <div className="flex flex-wrap items-center gap-x-4 gap-y-1 pl-3.5 text-[11px]">
          <span className="inline-flex items-center gap-1.5 text-accent-green">
            <Check size={11} className="shrink-0" strokeWidth={2.5} />
            <span className="text-on-dark-mute">
              작동: <span className="text-on-dark">에디터 · 검색 · 그래프 · MCP</span>
            </span>
          </span>
          <span className="inline-flex items-center gap-1.5 text-accent-red">
            <X size={11} className="shrink-0" strokeWidth={2.5} />
            <span className="text-on-dark-mute">
              비활성: Quick Capture · Compound · Ghost 스캔
            </span>
          </span>
        </div>
      </div>
      <div className="flex items-center gap-3">
        <button
          type="button"
          onClick={async () => {
            // Hard reset: nuke provider in config so the existing
            // App.tsx logic naturally routes to Onboarding on reload.
            // Belt + suspenders we still set the hash + flag so even
            // if config save somehow no-ops the reload still works.
            try {
              const live = await ipc.loadConfig();
              if (live?.vault_path) {
                await ipc.saveConfig(live.vault_path, {
                  ...live,
                  provider: null,
                  models: { routing: null, writer: null },
                });
              }
            } catch (e) {
              console.error("[danbi] reset provider failed", e);
            }
            window.localStorage.setItem("danbi.forceOnboarding", "1");
            window.location.hash = "#onboarding";
            window.location.reload();
          }}
          className="text-[11px] text-accent-yellow underline-offset-2 hover:underline"
        >
          온보딩 다시
        </button>
        <button
          onClick={onConnect}
          className="inline-flex h-6 items-center rounded-sm bg-accent-yellow px-2 text-[11px] font-semibold text-canvas hover:bg-accent-yellow/90"
        >
          Provider 연결
        </button>
      </div>
    </div>
  );
}

function DialogForm({
  label,
  input,
  setInput,
  placeholder,
  onEnter,
  error,
  hint,
  monospace,
}: {
  label: string;
  input: string;
  setInput: (v: string) => void;
  placeholder?: string;
  onEnter: (e: React.KeyboardEvent<HTMLDivElement>) => void;
  error: string | null;
  hint?: string;
  monospace?: boolean;
}) {
  const ref = useRef<HTMLInputElement | null>(null);
  useEffect(() => {
    ref.current?.focus();
  }, []);
  return (
    <div onKeyDown={onEnter}>
      {/* Wizard 의 TextField 를 그대로 쓰면 라벨/인풋이 작아 다이얼로그
          버튼(h-11) 과 균형이 안 맞는다. 다이얼로그 전용으로 한 단계 키운
          버전을 인라인. */}
      <label className="flex flex-col gap-2">
        <span className="text-[14px] font-medium tracking-[0.2px] text-on-dark-mute">
          {label}
        </span>
        <input
          ref={ref}
          type="text"
          value={input}
          onChange={(e) => setInput(e.target.value)}
          placeholder={placeholder}
          className={cn(
            "h-12 w-full rounded-md border border-hairline bg-surface-elevated px-4 text-[16px] text-ink outline-none transition-colors placeholder:text-stone focus:border-hairline-strong focus:bg-surface-card",
            monospace && "font-mono",
          )}
        />
      </label>
      {hint && (
        <div className="mt-2 text-[13px] text-mute">{hint}</div>
      )}
      {error && (
        <div className="mt-3 rounded-md border border-hairline bg-surface-elevated p-2 font-mono text-[12px] text-accent-red">
          {error}
        </div>
      )}
    </div>
  );
}

function DialogFooter({
  onCancel,
  onSubmit,
  submitLabel = "확인",
  danger,
}: {
  onCancel: () => void;
  onSubmit: () => void;
  submitLabel?: string;
  danger?: boolean;
}) {
  return (
    <>
      <SecondaryButton onClick={onCancel}>취소</SecondaryButton>
      {danger ? (
        <button
          onClick={onSubmit}
          className="inline-flex h-9 items-center rounded-md border border-hairline bg-surface-elevated px-4 text-[14px] font-medium leading-none text-accent-red transition-colors hover:border-hairline-strong"
        >
          {submitLabel}
        </button>
      ) : (
        <PrimaryButton onClick={onSubmit}>{submitLabel}</PrimaryButton>
      )}
    </>
  );
}

// 우상단 floating BgJobToast 는 어수선해서 제거. 진행 상태는 사이드바 footer
// BgJobPill 하나로만 노출 (왼쪽 상태바). bgJobLabels 도 toast 전용이라 함께 제거.
