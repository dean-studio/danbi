import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { ArrowLeft, Check, ChevronDown, ChevronUp, Copy, FileText, Loader2, MoreHorizontal, PanelRight, Save, Sparkles, X } from "lucide-react";
import { writeText as clipboardWriteText } from "@tauri-apps/plugin-clipboard-manager";
import { convertFileSrc } from "@tauri-apps/api/core";
import { BlockNoteEditor, type PartialBlock } from "@blocknote/core";
import { useCreateBlockNote } from "@blocknote/react";
import { BlockNoteView } from "@blocknote/mantine";
import "@blocknote/mantine/style.css";
import {
  exportBlocksToMarkdown,
  parseMarkdownRestoringImages,
} from "@/main/imageRoundTrip";
import { DOC_CONFLICT, ipc } from "@/lib/ipc";
import { DocInspector } from "@/main/DocInspector";
import { cn } from "@/lib/utils";
import { useApp } from "@/state/store";
import { resolveTheme, type ThemeChoice } from "@/lib/theme";
import { isWikiHref, parseWikiHref } from "@/main/wikiLinks";

/** 마지막 입력 뒤 이만큼 조용하면 자동 저장. */
const AUTOSAVE_DELAY_MS = 1500;

function useResolvedTheme(): "dark" | "light" {
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
  return resolved;
}

const INSPECTOR_OPEN_KEY = "danbi:inspector-open";

export function DocView({
  refreshKey,
  onOpenGraph,
  onRenameDomain,
  onDeleteDomain,
}: {
  refreshKey: number;
  /** ··· 메뉴 "그래프에서 보기". 그래프 뷰가 현재 프로젝트로 spotlight
   *  된 상태로 열림. */
  onOpenGraph?: () => void;
  /** ··· 메뉴 이름 바꾸기 / 삭제 — Workspace 의 다이얼로그를 연다. */
  onRenameDomain?: (project: string, domain: string) => void;
  onDeleteDomain?: (project: string, domain: string) => void;
}) {
  const cfg = useApp((s) => s.cfg);
  const selection = useApp((s) => s.selection);
  const selectProject = useApp((s) => s.selectProject);
  const selectDomain = useApp((s) => s.selectDomain);
  const setBgJob = useApp((s) => s.setBgJob);
  const pushNotification = useApp((s) => s.pushNotification);
  const tree = useApp((s) => s.tree);
  const editorTheme = useResolvedTheme();

  const vaultPath = cfg?.vault_path ?? null;
  const project = selection.project;

  // BlockNote captures `uploadFile` once at editor creation, so we read the
  // live vault/project through refs instead of closing over stale values.
  const vaultPathRef = useRef<string | null>(vaultPath);
  const projectRef = useRef<string | null>(project);
  useEffect(() => {
    vaultPathRef.current = vaultPath;
  }, [vaultPath]);
  useEffect(() => {
    projectRef.current = project;
  }, [project]);

  const uploadFile = useMemo(
    () =>
      async (file: File): Promise<string> => {
        try {
          const v = vaultPathRef.current;
          const p = projectRef.current;
          if (!v || !p) {
            throw new Error("프로젝트를 먼저 선택하세요");
          }
          const buf = new Uint8Array(await file.arrayBuffer());
          const rawName = file.name && file.name.trim().length > 0
            ? file.name
            : `pasted-${Date.now()}.png`;
          const rel = await ipc.saveAsset(v, p, rawName, buf);
          const abs = await ipc.resolveAsset(v, p, rel);
          return convertFileSrc(abs);
        } catch (e) {
          console.error("[danbi] uploadFile failed:", e);
          setErr(`이미지 업로드 실패: ${e}`);
          throw e;
        }
      },
    [],
  );

  const editor = useCreateBlockNote({
    initialContent: [{ type: "paragraph", content: "" } as PartialBlock],
    uploadFile,
  });

  const [loading, setLoading] = useState(false);
  const [err, setErr] = useState<string | null>(null);
  const [dirty, setDirty] = useState(false);
  const [saving, setSaving] = useState(false);
  const [bytes, setBytes] = useState(0);
  /** 이 세션에서 마지막으로 저장한 시각 — 헤더 "저장됨 14:32". */
  const [lastSavedAt, setLastSavedAt] = useState<number | null>(null);
  /** 인스펙터 복원 뒤 강제 재로드용. */
  const [reloadTick, setReloadTick] = useState(0);
  /** 편집 중에 다른 곳(에이전트 등)이 이 문서를 바꿔 저장을 보류한 상태.
   *  풀릴 때까지 자동 저장을 멈추고 배너로 선택을 받는다. */
  const [conflict, setConflict] = useState(false);
  const conflictRef = useRef(false);
  const setConflictState = useCallback((v: boolean) => {
    conflictRef.current = v;
    setConflict(v);
  }, []);

  // Last markdown we synced to disk. `dirty` 는 onChange 플래그 — 키 입력마다
  // 전체 마크다운을 직렬화해 비교하지 않는다 (큰 문서에서 타이핑이 끊기던
  // 원인). 실제 "바뀐 게 없음" 판정은 save() 에서 한 번만 한다.
  const lastSyncedMd = useRef<string>("");
  // Suppress the onChange handler when we rehydrate from disk.
  const suppressChange = useRef(false);
  // 타이머·load() 처럼 stale closure 를 피해야 하는 곳에서 읽는 미러.
  const dirtyRef = useRef(false);
  const markDirty = useCallback((v: boolean) => {
    dirtyRef.current = v;
    setDirty(v);
  }, []);

  // Track which doc the *current* editor content represents. Used so a
  // watcher-triggered refreshKey bump doesn't rebuild the editor when
  // we'd just be replacing the document with itself — the rebuild
  // visibly flickers because BlockNote rerenders the entire tree.
  //
  // 저장은 항상 *이 문서* 로 간다 (selection 이 아니라). 사이드바에서 다른
  // 문서를 고른 직후엔 selection 은 이미 B 인데 에디터 본문은 아직 A 라서,
  // selection 기준으로 쓰면 A 본문이 B 경로에 저장되는 사고가 났다.
  const loadedKey = useRef<string | null>(null);
  const loadedDoc = useRef<{
    vault: string;
    project: string;
    domain: string;
  } | null>(null);
  // 진행 중인 저장. load() 는 전환 전에 이걸 기다린 뒤 flush 한다.
  const saveInFlight = useRef<Promise<void> | null>(null);
  const autosaveTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const cancelAutosave = useCallback(() => {
    if (autosaveTimer.current) {
      clearTimeout(autosaveTimer.current);
      autosaveTimer.current = null;
    }
  }, []);

  /** 에디터 본문을 `loadedDoc` 경로에 저장. 바뀐 게 없으면 쓰기 없이
   *  dirty 만 내린다. 동시 호출은 진행 중인 저장을 그대로 돌려준다. */
  const save = useCallback((overwrite = false): Promise<void> => {
    if (saveInFlight.current) return saveInFlight.current;
    const target = loadedDoc.current;
    if (!target || !dirtyRef.current) return Promise.resolve();
    cancelAutosave();
    if (conflictRef.current && !overwrite) {
      return Promise.reject(new Error(`${DOC_CONFLICT}: 저장 보류 중`));
    }
    const run = (async () => {
      setSaving(true);
      try {
        const live = exportBlocksToMarkdown(editor);
        const md = rewriteMdAbsoluteToRelative(live);
        if (md === lastSyncedMd.current) {
          // 타이핑 후 되돌린 경우 — 디스크와 같으니 커밋을 만들지 않는다.
          if (loadedDoc.current === target) markDirty(false);
          return;
        }
        // 우리가 만든 저장이라는 marker — Workspace 의 watcher refresh 가
        // 이 직후 발화하는 vault:changed 를 한 번 무시하게 해서 사이드바
        // listTree + linkIndex 재빌드를 건너뛴다.
        useApp
          .getState()
          .markSelfSave(`Projects/${target.project}/${target.domain}`);
        try {
          // 마지막으로 읽거나 쓴 본문을 base 로 넘겨, 그 사이 에이전트가 쓴
          // 내용을 덮어쓰지 않게 한다. 사용자가 덮어쓰기를 고르면 base 없이.
          await ipc.writeDoc(
            target.vault,
            target.project,
            target.domain,
            md,
            overwrite ? undefined : lastSyncedMd.current,
          );
        } catch (e) {
          if (String(e).startsWith(DOC_CONFLICT) && loadedDoc.current === target) {
            setConflictState(true);
          }
          throw e;
        }
        // 저장 중에 문서가 바뀌지 않았을 때만 상태를 내린다 (load() 가
        // flush 를 기다리므로 보통은 항상 같다).
        if (loadedDoc.current === target) {
          lastSyncedMd.current = md;
          setBytes(md.length);
          markDirty(false);
          setLastSavedAt(Date.now());
          setConflictState(false);
        }
      } finally {
        setSaving(false);
        saveInFlight.current = null;
      }
    })();
    saveInFlight.current = run;
    return run;
  }, [editor, cancelAutosave, markDirty, setConflictState]);

  // 실패를 toast 로 알리는 변형 — 본문은 에디터에 그대로 남아 있으므로
  // 전체 패널을 에러로 바꾸지 않는다 (사용자가 다시 ⌘S 하면 됨).
  const saveOrNotify = useCallback(() => {
    const target = loadedDoc.current;
    return save().catch((e) => {
      // 충돌은 문서 위 배너가 알린다 — toast 를 겹쳐 띄우지 않는다.
      if (String(e).includes(DOC_CONFLICT)) return;
      pushNotification({
        tone: "err",
        title: "저장 실패",
        body: `${target?.project}/${target?.domain} — ${String(e).slice(0, 120)}`,
      });
    });
  }, [save, pushNotification]);

  useEffect(() => {
    let alive = true;
    async function load() {
      const nextKey =
        cfg?.vault_path && selection.project && selection.domain
          ? `${cfg.vault_path}|${selection.project}|${selection.domain}`
          : null;
      const isSwitchingDoc = loadedKey.current !== nextKey;

      // 다른 문서로 가기 전에 미저장분을 "떠나는" 문서에 flush. 실패하면
      // 전환을 멈추고 에러를 보여줘 본문이 사라지지 않게 한다.
      if (isSwitchingDoc && loadedDoc.current) {
        cancelAutosave();
        try {
          if (saveInFlight.current) await saveInFlight.current;
          if (dirtyRef.current) await save();
        } catch (e) {
          const left = loadedDoc.current;
          pushNotification({
            tone: "err",
            title: "저장 실패 — 문서를 전환하지 않았어요",
            body: `${left?.project}/${left?.domain} — ${String(e).slice(0, 120)}`,
          });
          if (left) selectDomain(left.project, left.domain);
          return;
        }
        if (!alive) return;
      }

      if (!cfg?.vault_path || !selection.project || !selection.domain) {
        suppressChange.current = true;
        await replaceEditorFromMd(editor, "", null, null);
        suppressChange.current = false;
        lastSyncedMd.current = "";
        loadedKey.current = null;
        loadedDoc.current = null;
        setBytes(0);
        markDirty(false);
        setErr(null);
        return;
      }
      // If the user has unsaved edits, don't stomp them when the watcher
      // re-fires (e.g. we just saved). Only reload when clean.
      if (!isSwitchingDoc && dirtyRef.current) return;

      const selKey = nextKey!;

      // For watcher-fired reloads on the SAME doc, peek at the file
      // first. Skip the editor rebuild entirely if bytes are identical
      // — that's the case 99% of the time (git auto-commit, .danbi
      // metadata writes, asset folder churn, etc.).
      try {
        const md = await ipc.readDoc(
          cfg.vault_path,
          selection.project,
          selection.domain,
        );
        if (!alive) return;
        if (!isSwitchingDoc && md === lastSyncedMd.current) {
          // No change — keep editor untouched. No flicker.
          return;
        }
        if (isSwitchingDoc) setLoading(true);
        setErr(null);
        suppressChange.current = true;
        await replaceEditorFromMd(
          editor,
          md,
          cfg.vault_path,
          selection.project,
        );
        suppressChange.current = false;
        lastSyncedMd.current = md;
        loadedKey.current = selKey;
        setConflictState(false);
        if (isSwitchingDoc) setLastSavedAt(null);
        loadedDoc.current = {
          vault: cfg.vault_path,
          project: selection.project,
          domain: selection.domain,
        };
        setBytes(md.length);
        markDirty(false);
      } catch (e) {
        if (alive) setErr(String(e));
      } finally {
        if (alive) setLoading(false);
      }
    }
    load();
    return () => {
      alive = false;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [cfg?.vault_path, selection.project, selection.domain, refreshKey, reloadTick]);

  // 언마운트(프로젝트 홈으로 이동 등) 시에도 미저장분을 flush.
  useEffect(() => {
    return () => {
      cancelAutosave();
      if (dirtyRef.current) saveOrNotify();
    };
  }, [saveOrNotify, cancelAutosave]);

  /** 충돌 배너: 내 편집을 버리고 디스크(에이전트가 쓴) 버전으로 다시 연다. */
  async function loadDiskVersion() {
    const target = loadedDoc.current;
    if (!target) return;
    cancelAutosave();
    try {
      const md = await ipc.readDoc(target.vault, target.project, target.domain);
      if (loadedDoc.current !== target) return;
      suppressChange.current = true;
      await replaceEditorFromMd(editor, md, target.vault, target.project);
      suppressChange.current = false;
      lastSyncedMd.current = md;
      setBytes(md.length);
      markDirty(false);
      setConflictState(false);
    } catch (e) {
      suppressChange.current = false;
      setErr(String(e));
    }
  }

  async function revert() {
    if (!dirty) return;
    cancelAutosave();
    suppressChange.current = true;
    await replaceEditorFromMd(
      editor,
      lastSyncedMd.current,
      cfg?.vault_path ?? null,
      selection.project ?? null,
    );
    suppressChange.current = false;
    setBytes(lastSyncedMd.current.length);
    markDirty(false);
  }

  // ⌘S / Ctrl+S to save. ⌘F / Ctrl+F to open the in-doc find bar.
  useEffect(() => {
    function onKey(e: KeyboardEvent) {
      if ((e.metaKey || e.ctrlKey) && e.key === "s") {
        e.preventDefault();
        saveOrNotify();
      } else if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "f") {
        e.preventDefault();
        setFindOpen(true);
      }
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [saveOrNotify]);

  // wiki-link 클릭 — 본문에서 [[link]] 가 markdown anchor 로 변환돼서
  // <a href="danbi:..."> 로 렌더된다. 클릭을 가로채서 selection 만 바꾸고
  // 외부 브라우저로 나가지 않게 한다.
  useEffect(() => {
    function onClick(e: MouseEvent) {
      const target = e.target as HTMLElement | null;
      if (!target) return;
      const anchor = target.closest("a") as HTMLAnchorElement | null;
      if (!anchor) return;
      const href = anchor.getAttribute("href");
      if (!isWikiHref(href)) return;
      e.preventDefault();
      e.stopPropagation();
      const parsed = parseWikiHref(href!);
      if (!parsed) return;
      // project 가 명시돼있고 vault 트리에 존재하면 그 프로젝트로,
      // 아니면 현재 프로젝트 안에서 찾는다. head 가 프로젝트가 아니면
      // (예: "notes/foo.md" — head="notes" 가 sub-folder) 현재 프로젝트
      // 의 하위 경로로 해석한다.
      const projects = tree?.projects?.map((p) => p.name) ?? [];
      let proj = parsed.project;
      let dom = parsed.domain;
      if (proj && !projects.includes(proj)) {
        // head 가 프로젝트가 아니라 sub-folder 였음 → 합쳐서 현재
        // 프로젝트의 도메인으로.
        dom = `${proj}/${dom}`;
        proj = null;
      }
      const finalProject = proj ?? selection.project;
      if (!finalProject) return;
      // .md 가 빠진 형태도 받아준다.
      const finalDomain = dom.endsWith(".md") ? dom : `${dom}.md`;
      selectDomain(finalProject, finalDomain);
    }
    const root = document.querySelector(".danbi-editor");
    if (!root) return;
    root.addEventListener("click", onClick as EventListener, true);
    return () =>
      root.removeEventListener("click", onClick as EventListener, true);
  }, [selection.project, tree, selectDomain]);

  // ---- In-doc find (⌘F) ----
  // We highlight matches by walking text nodes inside `.danbi-editor`,
  // wrapping each match in a `<mark data-danbi-match>`, and scrolling
  // the active one into view. BlockNote's editor renders into a
  // contenteditable so wrapping nodes is delicate — we ALWAYS unwrap
  // before re-applying so the user's saved markdown stays untouched.
  const [findOpen, setFindOpen] = useState(false);
  const [findQuery, setFindQuery] = useState("");
  // Daily 노트 요약 / HTML 추출 — AI 연동 켰을 때만 활성. 미연동이면
  // 버튼 자체는 보이지만 disabled 라서 사용자가 "이런 게 있구나" 인지
  // 시키고 클릭 시 Settings 로 안내.
  const [summaryResult, setSummaryResult] = useState<{
    summary_md: string;
    html: string;
    provider: string;
    export_id: string;
  } | null>(null);
  const [summaryError, setSummaryError] = useState<string | null>(null);
  // 이 노트의 export history. daily 노트 화면 진입 시 / 새 요약 직후
  // 자동으로 새로고침. 한 노트당 평소 0~수십개 정도라 한 번에 다 받음.
  const [exportHistory, setExportHistory] = useState<
    import("@/lib/ipc").ExportRecord[]
  >([]);
  const isDailyNote = (() => {
    const d = selection.domain ?? "";
    // daily/YYYY-MM-DD.md 형식
    return /^daily\/\d{4}-\d{2}-\d{2}\.md$/.test(d);
  })();
  const aiConnected = !!cfg?.embed_provider;

  // History 자동 동기화 — 노트 바뀌거나 새 요약/추출 만들면 list 다시
  // 받음. daily 든 일반 노트든 둘 다 export 가능하므로 isDailyNote
  // 조건 없음.
  useEffect(() => {
    if (!selection.project || !selection.domain) {
      setExportHistory([]);
      return;
    }
    let alive = true;
    ipc
      .listExports(selection.project, selection.domain)
      .then((rs) => {
        if (alive) setExportHistory(rs);
      })
      .catch(() => {
        if (alive) setExportHistory([]);
      });
    return () => {
      alive = false;
    };
  }, [
    selection.project,
    selection.domain,
    summaryResult,
  ]);

  const [findIndex, setFindIndex] = useState(0);
  const [findCount, setFindCount] = useState(0);

  const clearFindHighlights = useCallback(() => {
    document
      .querySelectorAll<HTMLElement>(
        ".danbi-editor mark[data-danbi-match]",
      )
      .forEach((el) => {
        const parent = el.parentNode;
        if (!parent) return;
        // Replace the <mark> with its text content.
        parent.replaceChild(document.createTextNode(el.textContent ?? ""), el);
        parent.normalize();
      });
  }, []);

  const applyFindHighlights = useCallback(
    (q: string): number => {
      clearFindHighlights();
      const root = document.querySelector(".danbi-editor");
      if (!root || !q) return 0;
      const lower = q.toLowerCase();
      // Collect text nodes first, then mutate — mutating during the
      // tree walk invalidates the iterator.
      const walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT);
      const nodes: Text[] = [];
      let n: Node | null = walker.nextNode();
      while (n) {
        if (n.nodeValue && n.nodeValue.toLowerCase().includes(lower)) {
          nodes.push(n as Text);
        }
        n = walker.nextNode();
      }
      let total = 0;
      for (const node of nodes) {
        const text = node.nodeValue ?? "";
        const lowerText = text.toLowerCase();
        const frag = document.createDocumentFragment();
        let cursor = 0;
        while (cursor < text.length) {
          const idx = lowerText.indexOf(lower, cursor);
          if (idx < 0) {
            frag.appendChild(
              document.createTextNode(text.slice(cursor)),
            );
            break;
          }
          if (idx > cursor) {
            frag.appendChild(
              document.createTextNode(text.slice(cursor, idx)),
            );
          }
          const m = document.createElement("mark");
          m.setAttribute("data-danbi-match", String(total));
          m.style.backgroundColor = "var(--color-accent-yellow-soft)";
          m.style.color = "inherit";
          m.style.borderRadius = "2px";
          m.style.padding = "0 1px";
          m.textContent = text.slice(idx, idx + q.length);
          frag.appendChild(m);
          cursor = idx + q.length;
          total += 1;
        }
        node.parentNode?.replaceChild(frag, node);
      }
      return total;
    },
    [clearFindHighlights],
  );

  // Re-highlight whenever the query changes.
  useEffect(() => {
    if (!findOpen) return;
    const q = findQuery;
    if (!q) {
      clearFindHighlights();
      setFindCount(0);
      setFindIndex(0);
      return;
    }
    // Slight debounce so each keystroke doesn't replay the whole walk.
    const id = setTimeout(() => {
      const total = applyFindHighlights(q);
      setFindCount(total);
      setFindIndex(0);
    }, 80);
    return () => clearTimeout(id);
  }, [findQuery, findOpen, applyFindHighlights, clearFindHighlights]);

  // Whenever the active index changes, scroll + focus the active <mark>:
  //  - tint background full yellow + thick outline so it pops
  //  - center it in the viewport
  //  - move the editor caret onto it so the user can keep typing in
  //    place after closing the find bar (Esc)
  useEffect(() => {
    if (!findOpen) return;
    const marks = document.querySelectorAll<HTMLElement>(
      ".danbi-editor mark[data-danbi-match]",
    );
    marks.forEach((m, i) => {
      const active = i === findIndex;
      m.style.backgroundColor = active
        ? "var(--color-accent-yellow)"
        : "var(--color-accent-yellow-soft)";
      m.style.color = active ? "var(--color-canvas)" : "inherit";
      m.style.outline = active ? "2px solid var(--color-accent-yellow)" : "";
      m.style.outlineOffset = active ? "1px" : "";
    });
    const active = marks[findIndex];
    if (active) {
      active.scrollIntoView({ behavior: "smooth", block: "center" });
      // Place the caret inside the active match. We do this BUT keep
      // focus on the find input so typing continues to filter — the
      // caret is just a positional anchor for when the user hits Esc.
      try {
        const range = document.createRange();
        range.selectNodeContents(active);
        const sel = window.getSelection();
        if (sel) {
          sel.removeAllRanges();
          sel.addRange(range);
        }
      } catch {
        /* ignore caret placement errors — Safari/contenteditable quirks */
      }
    }
  }, [findIndex, findCount, findOpen]);

  // Cleanup highlights when the find bar closes or the document changes.
  useEffect(() => {
    if (!findOpen) {
      clearFindHighlights();
      setFindQuery("");
      setFindCount(0);
      setFindIndex(0);
    }
  }, [findOpen, clearFindHighlights]);
  useEffect(() => {
    return () => clearFindHighlights();
  }, [selection.domain, selection.project, clearFindHighlights]);

  function nextMatch(delta: number) {
    if (findCount === 0) return;
    setFindIndex((i) => (i + delta + findCount) % findCount);
  }

  // 자동 저장 — 입력이 멈추고 AUTOSAVE_DELAY_MS 뒤 + 에디터 blur 시. 커밋은
  // Rust `writeDoc` 파이프라인(vcs) 안에서 만들어지므로 되돌리기는 그대로.
  // Settings > 편집 > 자동 저장 으로 끌 수 있다 (기본 켜짐). 꺼도 문서 전환
  // 시 flush 는 항상 한다.
  const autosaveEnabled = cfg?.editor.autosave ?? true;
  const autosaveRef = useRef(autosaveEnabled);
  useEffect(() => {
    autosaveRef.current = autosaveEnabled;
    if (!autosaveEnabled) cancelAutosave();
  }, [autosaveEnabled, cancelAutosave]);
  const scheduleAutosave = useCallback(() => {
    if (!autosaveRef.current || conflictRef.current) return;
    cancelAutosave();
    autosaveTimer.current = setTimeout(() => {
      autosaveTimer.current = null;
      saveOrNotify();
    }, AUTOSAVE_DELAY_MS);
  }, [saveOrNotify, cancelAutosave]);
  useEffect(() => {
    if (!autosaveEnabled) return;
    // We target the editor's outermost container. `focusout` bubbles
    // (unlike `blur`), so a document-level listener catches any transition
    // away from the editor even when focus moves to a sibling text input.
    function onFocusOut(e: FocusEvent) {
      // Only act if focus left the BlockNote container entirely. If the
      // new focus target is still inside `.danbi-editor`, it's just
      // moving between internal blocks — not a true blur.
      const nextTarget = e.relatedTarget as HTMLElement | null;
      if (nextTarget && nextTarget.closest(".danbi-editor")) return;
      if (!dirtyRef.current) return;
      saveOrNotify();
    }
    const el = document.querySelector(".danbi-editor");
    if (!el) return;
    el.addEventListener("focusout", onFocusOut as EventListener);
    return () => el.removeEventListener("focusout", onFocusOut as EventListener);
  }, [autosaveEnabled, saveOrNotify, loading, selection.project, selection.domain]);

  // 헤더 ··· 메뉴 · 인스펙터(⌘I, 마지막 상태 기억) · 브레드크럼.
  const [menuOpen, setMenuOpen] = useState(false);
  const [inspectorOpen, setInspectorOpen] = useState<boolean>(() => {
    try {
      return localStorage.getItem(INSPECTOR_OPEN_KEY) === "1";
    } catch {
      return false;
    }
  });
  const toggleInspector = useCallback(() => {
    setInspectorOpen((v) => {
      try {
        localStorage.setItem(INSPECTOR_OPEN_KEY, v ? "0" : "1");
      } catch {
        /* ignore */
      }
      return !v;
    });
  }, []);
  useEffect(() => {
    function onKey(e: KeyboardEvent) {
      if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "i" && !e.shiftKey) {
        e.preventDefault();
        toggleInspector();
      }
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [toggleInspector]);
  const crumbs = (() => {
    const segs = (selection.domain ?? "").split("/");
    return { folders: segs.slice(0, -1), file: segs[segs.length - 1] ?? "" };
  })();

  async function revealInFinder() {
    if (!cfg?.vault_path || !selection.project || !selection.domain) return;
    try {
      const { revealItemInDir } = await import("@tauri-apps/plugin-opener");
      await revealItemInDir(
        [cfg.vault_path, "Projects", selection.project, selection.domain].join("/"),
      );
    } catch {
      /* opener unavailable */
    }
  }

  async function runSummarize() {
    if (!aiConnected) {
      setSummaryError(
        "AI 연동이 꺼져있어요. Settings → 임베딩 에서 Gemini 또는 Bedrock 을 연결하면 활성화됩니다.",
      );
      return;
    }
    if (!selection.project || !selection.domain) return;
    const proj = selection.project;
    const dom = selection.domain;
    setSummaryError(null);
    // 백그라운드 패턴 — IPC 를 fire-and-forget 로 시작하고
    // store 의 bgJob 에 진행 상태만 띄운다. 사용자는 그 사이
    // 다른 노트 편집·검색·설정 다 가능. 완료 시 macOS 알림
    // 으로 신호 + Sidebar progress pill 도 클릭 가능 결과로
    // 전환된다.
    setBgJob({
      kind: "summarize",
      status: "running",
      project: proj,
      domain: dom,
      startedAt: Date.now(),
    });
    ipc
      .summarizeDaily(proj, dom)
      .then((r) => {
        setBgJob({
          kind: "summarize",
          status: "done",
          project: proj,
          domain: dom,
          exportId: r.export_id,
          finishedAt: Date.now(),
        });
        // 사용자가 그 노트에 머물러있으면 모달도 즉시 띄움.
        // 다른 노트로 이동했으면 알림만 받고 history 에서
        // 클릭으로 열도록.
        const cur = useApp.getState().selection;
        if (cur.project === proj && cur.domain === dom) {
          setSummaryResult(r);
        }
        // 단비 내부 알림만 — macOS 시스템 알림 안 띄움.
        // 사이드바 헤더의 종 아이콘에 +1 배지 + popover
        // list 에 누적. 클릭 시 결과 페이지로 라우팅.
        pushNotification({
          tone: "ok",
          title: "요약 완료",
          body: `${proj}/${dom}`,
          action: { kind: "open-export", exportId: r.export_id },
        });
      })
      .catch((e) => {
        setBgJob({
          kind: "summarize",
          status: "error",
          project: proj,
          domain: dom,
          message: String(e).slice(0, 240),
          finishedAt: Date.now(),
        });
        pushNotification({
          tone: "err",
          title: "요약 실패",
          body: `${proj}/${dom} — ${String(e).slice(0, 120)}`,
        });
      });
  }

  async function runExportHtml() {
    if (!selection.project || !selection.domain) return;
    const proj = selection.project;
    const dom = selection.domain;
    try {
      const r = await ipc.exportDocHtml(proj, dom);
      await ipc.openHtmlPreview(
        r.html,
        dom.replace(/\.md$/, ""),
      );
      pushNotification({
        tone: "ok",
        title: "HTML 추출 완료",
        body: `${proj}/${dom}`,
        action: { kind: "open-export", exportId: r.export_id },
      });
    } catch (e) {
      pushNotification({
        tone: "err",
        title: "HTML 추출 실패",
        body: `${proj}/${dom} — ${String(e).slice(0, 120)}`,
      });
    }
  }

  if (!selection.project || !selection.domain) {
    return (
      <div className="flex h-full flex-col">
        <div
          data-tauri-drag-region
          className="h-10 w-full shrink-0 border-b border-hairline"
        />
        <div className="flex flex-1 flex-col items-center justify-center gap-3 text-center">
          <FileText size={24} className="text-stone" />
          <div className="text-[13px] text-mute">
            좌측에서 프로젝트와 도메인 파일을 선택하세요
          </div>
        </div>
      </div>
    );
  }

  return (
    <div className="flex h-full flex-col">
      <header
        data-tauri-drag-region
        className="flex h-10 shrink-0 items-center justify-between border-b border-hairline px-4"
      >
        <div className="flex min-w-0 items-center gap-1.5">
          <button
            onClick={() => {
              if (selection.project) selectProject(selection.project);
            }}
            title="프로젝트 홈으로 (⌘[)"
            className="inline-flex h-6 w-6 shrink-0 place-items-center rounded-sm text-mute transition-colors hover:bg-surface-elevated hover:text-on-dark"
          >
            <ArrowLeft size={13} />
          </button>
          {/* 브레드크럼 — 프로젝트 › 폴더 › 파일. 프로젝트는 홈으로. */}
          <button
            onClick={() => {
              if (selection.project) selectProject(selection.project);
            }}
            className="shrink-0 text-[12px] text-mute transition-colors hover:text-on-dark"
          >
            {selection.project}
          </button>
          {crumbs.folders.map((f) => (
            <span key={f} className="flex shrink-0 items-center gap-1.5">
              <span className="text-stone">›</span>
              <span className="text-[12px] text-mute">{f}</span>
            </span>
          ))}
          <span className="text-stone">›</span>
          <span className="truncate font-mono text-[13px] text-ink">
            {crumbs.file}
          </span>
          <span
            className={cn(
              "ml-2 shrink-0 text-[11px]",
              dirty ? "text-on-dark-mute" : "text-stone",
            )}
          >
            {saving
              ? "저장 중…"
              : dirty
                ? "수정됨 — ⌘S"
                : lastSavedAt
                  ? `저장됨 ${formatHM(lastSavedAt)}`
                  : ""}
          </span>
        </div>
        <div className="flex shrink-0 items-center gap-1.5">
          <span className="text-caption-sm text-stone">
            {bytes.toLocaleString()} chars
          </span>
          {dirty && (
            <button
              onClick={saveOrNotify}
              disabled={saving}
              className="inline-flex h-7 items-center gap-1 rounded-sm bg-primary px-2.5 text-[12px] font-medium text-on-primary transition-colors hover:bg-primary-pressed"
              title="저장 (⌘S)"
            >
              <Save size={11} /> {saving ? "저장 중…" : "저장"}
            </button>
          )}
          {/* ··· 오버플로 — 헤더 컨트롤 9개를 2개로. */}
          <div className="relative">
            <button
              onClick={() => setMenuOpen((v) => !v)}
              title="더 보기"
              className={cn(
                "inline-flex h-7 w-7 items-center justify-center rounded-sm border border-hairline bg-surface-elevated text-body transition-colors hover:border-hairline-strong hover:text-on-dark",
                menuOpen && "border-hairline-strong text-on-dark",
              )}
            >
              <MoreHorizontal size={13} />
            </button>
            {menuOpen && (
              <>
                <div className="fixed inset-0 z-30" onClick={() => setMenuOpen(false)} />
                <div className="absolute right-0 top-9 z-40 w-[240px] overflow-hidden rounded-md border border-hairline-strong bg-surface-card py-1">
                  {isDailyNote && (
                    <MenuItem
                      label={aiConnected ? "요약 / HTML" : "요약 / HTML (AI 연동 필요)"}
                      onClick={() => {
                        setMenuOpen(false);
                        runSummarize();
                      }}
                    />
                  )}
                  {!isDailyNote && (
                    <MenuItem
                      label="HTML 추출"
                      onClick={() => {
                        setMenuOpen(false);
                        runExportHtml();
                      }}
                    />
                  )}
                  {exportHistory.slice(0, 5).map((rec) => (
                    <MenuItem
                      key={rec.id}
                      label={`이전 요약 · ${exportDateLabel(rec.created_at)}`}
                      hint={`${(rec.md_bytes / 1024).toFixed(1)}KB`}
                      onClick={async () => {
                        setMenuOpen(false);
                        try {
                          await ipc.openExport(rec.id);
                        } catch (e) {
                          console.error("[danbi] open export", e);
                        }
                      }}
                    />
                  ))}
                  <MenuDivider />
                  {onOpenGraph && (
                    <MenuItem
                      label="그래프에서 보기"
                      shortcut="⌘G"
                      onClick={() => {
                        setMenuOpen(false);
                        onOpenGraph();
                      }}
                    />
                  )}
                  <MenuItem
                    label="인스펙터 (변경 · 링크 · 정보)"
                    shortcut="⌘I"
                    onClick={() => {
                      setMenuOpen(false);
                      toggleInspector();
                    }}
                  />
                  {dirty && (
                    <MenuItem
                      label="수정 내용 되돌리기"
                      onClick={() => {
                        setMenuOpen(false);
                        revert();
                      }}
                    />
                  )}
                  <MenuDivider />
                  <MenuItem
                    label="Finder에서 보기"
                    onClick={() => {
                      setMenuOpen(false);
                      revealInFinder();
                    }}
                  />
                  <MenuItem
                    label="경로 복사 (vault 기준)"
                    onClick={() => {
                      setMenuOpen(false);
                      clipboardWriteText(`${selection.project}/${selection.domain}`).catch(() => {});
                    }}
                  />
                  {onRenameDomain && (
                    <MenuItem
                      label="이름 바꾸기"
                      onClick={() => {
                        setMenuOpen(false);
                        if (selection.project && selection.domain)
                          onRenameDomain(selection.project, selection.domain);
                      }}
                    />
                  )}
                  {onDeleteDomain && (
                    <MenuItem
                      label="삭제…"
                      danger
                      onClick={() => {
                        setMenuOpen(false);
                        if (selection.project && selection.domain)
                          onDeleteDomain(selection.project, selection.domain);
                      }}
                    />
                  )}
                </div>
              </>
            )}
          </div>
          <button
            onClick={toggleInspector}
            title="인스펙터 (⌘I)"
            className={cn(
              "inline-flex h-7 w-7 items-center justify-center rounded-sm border border-hairline bg-surface-elevated text-body transition-colors hover:border-hairline-strong hover:text-on-dark",
              inspectorOpen && "border-hairline-strong text-on-dark",
            )}
          >
            <PanelRight size={13} />
          </button>
        </div>
      </header>

      {/* Summary 결과 모달 — 요약 markdown 미리보기 + HTML 다운로드/복사. */}
      {(summaryResult || summaryError) && (
        <div
          className="fixed inset-0 z-40 flex items-center justify-center bg-canvas/70 backdrop-blur-sm"
          onMouseDown={(e) => {
            if (e.target === e.currentTarget) {
              setSummaryResult(null);
              setSummaryError(null);
            }
          }}
        >
          <div className="flex max-h-[80vh] w-[640px] flex-col overflow-hidden rounded-lg border border-hairline bg-surface">
            <header className="flex h-11 shrink-0 items-center justify-between border-b border-hairline px-4">
              <span className="inline-flex items-center gap-2 text-[14px] font-medium text-ink">
                <Sparkles size={14} className="text-accent-blue" />
                {selection.domain} · 요약
              </span>
              <button
                onClick={() => {
                  setSummaryResult(null);
                  setSummaryError(null);
                }}
                className="grid h-7 w-7 place-items-center rounded-sm text-mute transition-colors hover:bg-surface-elevated hover:text-on-dark"
              >
                <X size={13} />
              </button>
            </header>
            <div className="flex-1 overflow-y-auto px-5 py-4">
              {summaryError ? (
                <div className="rounded-md border border-accent-red/40 bg-accent-red-soft/30 p-3 text-[12.5px] leading-[1.6] text-accent-red">
                  {summaryError}
                </div>
              ) : summaryResult ? (
                <pre className="whitespace-pre-wrap font-sans text-[13px] leading-[1.65] text-body">
                  {summaryResult.summary_md}
                </pre>
              ) : null}
            </div>
            {summaryResult && (
              <footer className="flex shrink-0 items-center justify-between border-t border-hairline bg-surface-elevated px-4 py-2.5 text-[12px]">
                <span className="text-stone">
                  via {summaryResult.provider}
                </span>
                <div className="flex items-center gap-2">
                  <button
                    type="button"
                    onClick={async () => {
                      const { writeText } = await import(
                        "@tauri-apps/plugin-clipboard-manager"
                      );
                      await writeText(summaryResult.summary_md).catch(() => {});
                    }}
                    className="inline-flex h-7 items-center gap-1 rounded-sm border border-hairline bg-surface px-2 text-body hover:border-hairline-strong hover:text-on-dark"
                    title="요약 markdown 클립보드 복사"
                  >
                    <Copy size={11} /> 요약 복사
                  </button>
                  <button
                    type="button"
                    onClick={async () => {
                      try {
                        const { save } = await import(
                          "@tauri-apps/plugin-dialog"
                        );
                        const stem = (selection.domain ?? "summary").replace(
                          /\.md$/,
                          "",
                        );
                        const path = await save({
                          defaultPath: `${stem}.html`,
                          filters: [{ name: "HTML", extensions: ["html"] }],
                        });
                        if (path) {
                          await ipc.saveHtmlToPath(
                            path as string,
                            summaryResult.html,
                          );
                        }
                      } catch (e) {
                        console.error("[danbi] save html failed", e);
                      }
                    }}
                    className="inline-flex h-7 items-center gap-1 rounded-sm border border-hairline bg-surface px-2 text-body hover:border-hairline-strong hover:text-on-dark"
                    title="HTML 파일로 저장"
                  >
                    <Save size={11} /> 파일로 저장
                  </button>
                  <button
                    type="button"
                    onClick={async () => {
                      try {
                        await ipc.openHtmlPreview(
                          summaryResult.html,
                          (selection.domain ?? "summary").replace(/\.md$/, ""),
                        );
                      } catch (e) {
                        console.error("[danbi] open preview failed", e);
                      }
                    }}
                    className="inline-flex h-7 items-center gap-1 rounded-sm border border-accent-blue bg-accent-blue px-3 font-medium text-on-primary hover:bg-primary-pressed"
                    title="새 창에서 HTML 페이지로 열기"
                  >
                    <Sparkles size={11} /> HTML 페이지 열기
                  </button>
                </div>
              </footer>
            )}
          </div>
        </div>
      )}

      {conflict && (
        <div className="mx-6 mt-3 flex flex-wrap items-center gap-2 rounded-md border border-hairline-strong bg-surface-elevated px-3 py-2 text-[12px] text-on-dark">
          <span className="min-w-0 flex-1">
            편집하는 동안 다른 곳에서 이 문서가 바뀌었어요. 저장을 멈췄어요.
          </span>
          <button
            type="button"
            onClick={loadDiskVersion}
            className="rounded-md border border-hairline px-2.5 py-1 text-[12px] text-on-dark hover:bg-surface-card"
          >
            디스크 버전 불러오기
          </button>
          <button
            type="button"
            onClick={() => {
              save(true).catch((e) =>
                pushNotification({
                  tone: "err",
                  title: "저장 실패",
                  body: String(e).slice(0, 120),
                }),
              );
            }}
            className="rounded-md border border-hairline px-2.5 py-1 text-[12px] text-mute hover:bg-surface-card hover:text-on-dark"
          >
            내 편집으로 덮어쓰기
          </button>
        </div>
      )}
      {findOpen && (
        <div className="flex shrink-0 items-center gap-1.5 border-b border-hairline bg-surface-elevated px-5 py-2">
          <input
            autoFocus
            value={findQuery}
            onChange={(e) => setFindQuery(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Escape") {
                e.preventDefault();
                setFindOpen(false);
              } else if (e.key === "Enter") {
                e.preventDefault();
                nextMatch(e.shiftKey ? -1 : 1);
              }
            }}
            placeholder="이 문서에서 찾기 (⌘F)"
            className="h-7 w-72 rounded-sm border border-hairline bg-surface px-2 text-[12px] text-ink outline-none placeholder:text-stone focus:border-hairline-strong"
          />
          <span className="font-mono text-[11px] tabular-nums text-mute">
            {findCount === 0 ? "0/0" : `${findIndex + 1}/${findCount}`}
          </span>
          <button
            onClick={() => nextMatch(-1)}
            disabled={findCount === 0}
            title="이전 일치 (Shift+Enter)"
            className="grid h-6 w-6 place-items-center rounded-sm text-mute transition-colors hover:bg-surface hover:text-on-dark disabled:opacity-40"
          >
            <ChevronUp size={12} />
          </button>
          <button
            onClick={() => nextMatch(1)}
            disabled={findCount === 0}
            title="다음 일치 (Enter)"
            className="grid h-6 w-6 place-items-center rounded-sm text-mute transition-colors hover:bg-surface hover:text-on-dark disabled:opacity-40"
          >
            <ChevronDown size={12} />
          </button>
          <div className="ml-auto flex items-center gap-1.5">
            <span className="text-[10px] uppercase tracking-[0.4px] text-stone">
              Esc 닫기
            </span>
            <button
              onClick={() => setFindOpen(false)}
              className="grid h-6 w-6 place-items-center rounded-sm text-mute transition-colors hover:bg-surface hover:text-on-dark"
              title="닫기"
            >
              <X size={11} />
            </button>
          </div>
        </div>
      )}

      {/* 빈 골격 (purpose.md / schema.md) 안내 — 새 프로젝트 만들면
          단비가 빈 템플릿을 자동 생성하는데 사용자가 손으로 채우기 전엔
          역할이 모호하다. Claude Code 같은 외부 AI 에 위임하라는 가이드. */}
      <PurposeSchemaHint
        project={selection.project ?? null}
        domain={selection.domain ?? null}
        bytes={bytes}
        loading={loading}
        aiConnected={aiConnected}
        onApply={async (md) => {
          if (
            !cfg?.vault_path ||
            !selection.project ||
            !selection.domain
          ) {
            return;
          }
          // AI 작성 결과를 vault 에 즉시 반영. 본문이 빈 골격이라
          // 덮어써도 사용자가 잃을 게 거의 없음. 이후 사용자가 에디터
          // 에서 직접 다듬도록 그대로 둠. dirty 플래그는 watcher 가
          // 로드 한 번 더 돌면서 자연 sync.
          await ipc.writeDoc(
            cfg.vault_path,
            selection.project,
            selection.domain,
            md,
          );
        }}
      />

      <div className="flex min-h-0 flex-1">
      <div className="min-w-0 flex-1 overflow-auto">
        {loading ? (
          <div className="px-6 py-4 text-[13px] text-mute">읽는 중…</div>
        ) : err ? (
          <div className="m-6 rounded-md border border-hairline bg-surface-elevated p-3 font-mono text-[12px] text-accent-red">
            {err}
          </div>
        ) : (
          <div
            className="danbi-editor"
            onCopy={stripMarkdownHardBreaksOnCopy}
            onCut={stripMarkdownHardBreaksOnCopy}
          >
            <BlockNoteView
              editor={editor}
              editable={true}
              theme={editorTheme}
              onChange={() => {
                if (suppressChange.current) return;
                // 직렬화 없이 플래그만. 글자 수는 저장 시점에 갱신된다.
                if (!dirtyRef.current) markDirty(true);
                scheduleAutosave();
              }}
            />
          </div>
        )}
      </div>
      {inspectorOpen && selection.project && selection.domain && (
        <DocInspector
          project={selection.project}
          domain={selection.domain}
          refreshKey={refreshKey + (lastSavedAt ?? 0)}
          onOpenDoc={(p, d) => selectDomain(p, d)}
          onRestored={() => {
            // 복원 커밋 뒤 에디터를 디스크 기준으로 다시 읽는다.
            markDirty(false);
            loadedKey.current = null;
            setReloadTick((t) => t + 1);
          }}
          onClose={toggleInspector}
        />
      )}
      </div>
    </div>
  );
}

function MenuItem({
  label,
  hint,
  shortcut,
  danger,
  onClick,
}: {
  label: string;
  hint?: string;
  shortcut?: string;
  danger?: boolean;
  onClick: () => void;
}) {
  return (
    <button
      onClick={onClick}
      className={cn(
        "flex w-full items-center justify-between gap-3 px-3 py-1.5 text-left text-[12.5px] transition-colors hover:bg-surface-elevated",
        danger ? "text-accent-red" : "text-body hover:text-on-dark",
      )}
    >
      <span className="truncate">{label}</span>
      {(hint || shortcut) && (
        <span className="shrink-0 font-mono text-[10px] text-stone">{hint ?? shortcut}</span>
      )}
    </button>
  );
}

function MenuDivider() {
  return <div className="my-1 h-px bg-hairline" />;
}

function formatHM(ms: number): string {
  const d = new Date(ms);
  return `${String(d.getHours()).padStart(2, "0")}:${String(d.getMinutes()).padStart(2, "0")}`;
}

function exportDateLabel(createdAtSec: number): string {
  const dt = new Date(createdAtSec * 1000);
  return `${dt.getFullYear()}-${String(dt.getMonth() + 1).padStart(2, "0")}-${String(dt.getDate()).padStart(2, "0")} ${formatHM(dt.getTime())}`;
}

/**
 * BlockNote 가 클립보드의 text/plain 슬롯에 markdown 을 쓰는데, 줄바꿈을
 * CommonMark hard break (`\\\n`) 로 직렬화한다. 다른 메모/문서 앱에 붙여
 * 넣으면 매 줄 끝에 `\` 가 보여서 거슬린다. React 의 onCopy 가 PM plugin
 * 의 setData 직후에 발화하므로, 여기서 text/plain 만 한 번 더 덮어써서
 * trailing `\` 를 떼낸다.
 */
function stripMarkdownHardBreaksOnCopy(
  e: React.ClipboardEvent<HTMLDivElement>,
) {
  const dt = e.clipboardData;
  if (!dt) return;
  const md = dt.getData("text/plain");
  if (!md || !md.includes("\\\n")) return;
  // Only strip a *trailing* backslash on a line — escaped backslashes inside
  // text (e.g. literal "\\n") look like `\\\n` only at line ends after
  // BlockNote's hard-break serializer.
  const cleaned = md.replace(/\\\n/g, "\n");
  if (cleaned !== md) dt.setData("text/plain", cleaned);
}

/**
 * Resolves relative `_assets/…` image refs in markdown (both `![](…)` and
 * `<img src="…">` forms) into absolute asset:// URLs the WebView can render.
 * Rewrites happen on a snapshot only; the stored markdown stays relative.
 */
async function rewriteMdRelativeToAbsolute(
  md: string,
  vaultPath: string,
  project: string,
): Promise<string> {
  const tasks: Promise<[string, string]>[] = [];

  // ![alt](rel)
  const mdRe = /!\[([^\]]*)\]\((\.?\/?_assets\/[^)]+)\)/g;
  for (const m of md.matchAll(mdRe)) {
    const rel = m[2].replace(/^\.\//, "");
    tasks.push(
      (async () => {
        const abs = await ipc.resolveAsset(vaultPath, project, rel);
        return [m[0], `![${m[1]}](${convertFileSrc(abs)})`] as [string, string];
      })(),
    );
  }

  // <img src="rel" width="123" alt="…" />  — preserve width.
  const imgRe = /<img\b([^>]*?)\bsrc=["'](\.?\/?_assets\/[^"']+)["']([^>]*)\/?>/g;
  for (const m of md.matchAll(imgRe)) {
    const before = m[1];
    const rel = m[2].replace(/^\.\//, "");
    const after = m[3];
    tasks.push(
      (async () => {
        const abs = await ipc.resolveAsset(vaultPath, project, rel);
        return [
          m[0],
          `<img${before}src="${convertFileSrc(abs)}"${after}>`,
        ] as [string, string];
      })(),
    );
  }

  const pairs = await Promise.all(tasks);
  let out = md;
  for (const [from, to] of pairs) out = out.split(from).join(to);
  return out;
}

/**
 * Collapses absolute asset/tauri URLs back to the `_assets/…` relative form
 * so the saved markdown stays portable (vault can be moved / synced elsewhere).
 * Handles both `![](…)` and `<img src="…" …>` variants.
 */
function rewriteMdAbsoluteToRelative(md: string): string {
  const toRel = (url: string): string => {
    try {
      const cleaned = String(url).split("#")[0].split("?")[0];
      const idx = cleaned.indexOf("_assets/");
      if (idx < 0) return url;
      return decodeURIComponent(cleaned.slice(idx));
    } catch {
      return url;
    }
  };

  let out = md.replace(
    /!\[([^\]]*)\]\(([^)]*?\/_assets\/[^)]+)\)/g,
    (_all, alt, url) => `![${alt}](${toRel(url)})`,
  );
  out = out.replace(
    /<img\b([^>]*?)\bsrc=["']([^"']*?\/_assets\/[^"']+)["']([^>]*)\/?>/g,
    (_all, before, url, after) => `<img${before}src="${toRel(url)}"${after}>`,
  );
  return out;
}


async function replaceEditorFromMd(
  editor: BlockNoteEditor,
  md: string,
  vaultPath: string | null,
  project: string | null,
) {
  const resolved =
    vaultPath && project
      ? await rewriteMdRelativeToAbsolute(md, vaultPath, project)
      : md;
  const blocks =
    resolved.trim().length === 0
      ? [{ type: "paragraph", content: "" } as PartialBlock]
      : parseMarkdownRestoringImages(editor, resolved);
  editor.replaceBlocks(editor.document, blocks);
}

/** 새 프로젝트 만들면 `purpose.md` 와 `schema.md` 가 헤더 한 줄만으로
 *  시드된다. 사용자가 외부 AI 에이전트에게 시킬 수 있도록 즉시 복사
 *  가능한 구체적인 한 줄 프롬프트를 노출한다. 프로젝트명을 포함해서
 *  AI 가 어느 vault 의 어느 도메인을 작성해야 하는지 헷갈리지 않게.
 *
 *  표시 조건:
 *    - 도메인이 purpose.md 또는 schema.md
 *    - 본문 길이가 작은 임계 미만 (= 사용자가 아직 안 채움)
 *    - 로딩 중 아님
 *  600 chars 임계는 사용자가 한 두 줄만 적어둔 상태면 여전히 도움말이
 *  의미 있어서 너무 빡빡하지 않은 게 좋다. */
function PurposeSchemaHint({
  project,
  domain,
  bytes,
  loading,
  aiConnected,
  onApply,
}: {
  project: string | null;
  domain: string | null;
  bytes: number;
  loading: boolean;
  aiConnected: boolean;
  onApply: (markdown: string) => Promise<void>;
}) {
  const [copied, setCopied] = useState(false);
  const setBgJob = useApp((s) => s.setBgJob);
  const pushNotification = useApp((s) => s.pushNotification);
  const pendingCompose = useApp((s) => s.pendingCompose);
  const setPendingCompose = useApp((s) => s.setPendingCompose);
  const [composeResult, setComposeResult] = useState<{
    markdown: string;
    provider: string;
  } | null>(null);
  const [composeError, setComposeError] = useState<string | null>(null);
  const [applying, setApplying] = useState(false);
  const bgJob = useApp((s) => s.bgJob);
  const composing =
    bgJob?.kind === "compose" &&
    bgJob.status === "running" &&
    bgJob.project === project;

  // 알림에서 클릭해 들어오거나, 백그라운드 compose 가 끝난 직후 같은
  // 노트에 머물러있으면 store.pendingCompose 가 채워져있다. 이 노트가
  // 그 결과의 대상이면 자동으로 미리보기 모달을 띄움.
  useEffect(() => {
    if (!pendingCompose) return;
    if (!project || !domain) return;
    const fname = domain.split("/").pop() ?? domain;
    const expected = `${pendingCompose.target}.md`;
    if (pendingCompose.project === project && fname === expected) {
      setComposeResult({
        markdown: pendingCompose.markdown,
        provider: pendingCompose.provider,
      });
      // 한 번 띄운 후엔 store 에서 비움 — 같은 알림을 두 번 띄우지 않게.
      setPendingCompose(null);
    }
  }, [pendingCompose, project, domain, setPendingCompose]);

  if (loading) return null;
  if (!domain || !project) return null;
  const fname = domain.split("/").pop() ?? domain;
  if (fname !== "purpose.md" && fname !== "schema.md") return null;
  if (bytes > 600) return null;
  const isPurpose = fname === "purpose.md";
  const target = isPurpose ? "purpose.md" : "schema.md";
  const prompt = isPurpose
    ? `단비 vault 의 "${project}" 프로젝트 \`purpose.md\` 정리해줘. vault 의 다른 도메인 파일들 (특히 daily 노트, notes/) 을 먼저 읽어서 맥락 파악하고, 이 프로젝트가 무엇을 위한 곳인지 / 무엇을 다루고 무엇을 다루지 않는지 / 지금 우선순위 를 한 문서로 정리해서 \`danbi_append\` 로 써줘.`
    : `단비 vault 의 "${project}" 프로젝트 \`schema.md\` 정리해줘. vault 의 다른 도메인 파일들 (특히 daily 노트, notes/) 의 실제 사용 패턴을 보고 파일 네이밍 / 문서 구조 / 링크 정책 / 스타일 규칙 을 한 문서로 정리해서 \`danbi_append\` 로 써줘.`;
  async function copy() {
    try {
      await clipboardWriteText(prompt);
      setCopied(true);
      window.setTimeout(() => setCopied(false), 1800);
    } catch {
      /* clipboard API unavailable — ignore */
    }
  }
  function compose() {
    if (!aiConnected) {
      setComposeError(
        "AI 연동이 꺼져있어요. Settings → 임베딩 에서 Gemini 또는 Bedrock 을 연결하면 활성화됩니다.",
      );
      return;
    }
    if (!project) return;
    const proj = project;
    const targetKind: "purpose" | "schema" = isPurpose ? "purpose" : "schema";
    setComposeError(null);
    // 요약과 같은 백그라운드 패턴 — store.bgJob 으로 진행 표시. 다른
    // 노트 편집·검색·설정 자유롭게.
    setBgJob({
      kind: "compose",
      target: targetKind,
      status: "running",
      project: proj,
      startedAt: Date.now(),
    });
    ipc
      .composePurposeSchema(proj, targetKind)
      .then((r) => {
        setBgJob({
          kind: "compose",
          target: targetKind,
          status: "done",
          project: proj,
          markdown: r.markdown,
          provider: r.provider,
          finishedAt: Date.now(),
        });
        // 결과를 store 에 박아둠 — 사용자가 같은 노트에 있든 다른 노트에
        // 있든 알림 클릭으로 selectDomain 만 발화하면 PurposeSchemaHint 가
        // pendingCompose 를 읽어 자동으로 미리보기 모달을 띄운다.
        useApp.getState().setPendingCompose({
          project: proj,
          target: targetKind,
          markdown: r.markdown,
          provider: r.provider,
        });
        pushNotification({
          tone: "ok",
          title: `${targetKind}.md 작성 완료`,
          body: `${proj} — 클릭해서 미리보기`,
          action: {
            kind: "open-compose",
            project: proj,
            target: targetKind,
            markdown: r.markdown,
          },
        });
      })
      .catch((e) => {
        setBgJob({
          kind: "compose",
          target: targetKind,
          status: "error",
          project: proj,
          message: String(e).slice(0, 240),
          finishedAt: Date.now(),
        });
        pushNotification({
          tone: "err",
          title: `${targetKind}.md 작성 실패`,
          body: `${proj} — ${String(e).slice(0, 120)}`,
        });
      });
  }
  async function apply() {
    if (!composeResult) return;
    setApplying(true);
    try {
      await onApply(composeResult.markdown);
      setComposeResult(null);
    } catch (e) {
      setComposeError(String(e));
    } finally {
      setApplying(false);
    }
  }
  return (
    <>
      <div className="border-b border-hairline bg-accent-blue-soft/40 px-6 py-3.5">
        <div className="mx-auto flex max-w-[820px] flex-col gap-2.5">
          <div className="flex items-start gap-3">
            <span className="mt-0.5 grid h-6 w-6 shrink-0 place-items-center rounded-full bg-accent-blue text-[11px] font-semibold text-on-primary">
              ?
            </span>
            <div className="flex-1 text-[13px] leading-[1.6] text-body">
              <div className="font-medium text-on-dark">
                {target} 가 비어있어요
              </div>
              <p className="mt-0.5 text-mute">
                {aiConnected
                  ? "단비가 직접 작성하거나, 외부 AI 에이전트에 위임할 수 있어요."
                  : "아래 프롬프트를 복사해서 Claude Code · Codex 같은 외부 AI 에이전트에 붙여넣으세요. AI 연동을 켜면 단비가 직접 작성도 가능합니다."}
              </p>
            </div>
          </div>
          <div className="flex items-stretch gap-2">
            <code className="flex-1 select-text rounded-md border border-hairline bg-surface-elevated px-3 py-2 font-mono text-[12px] leading-[1.55] text-on-dark-mute">
              {prompt}
            </code>
            <div className="flex shrink-0 flex-col gap-2">
              <button
                type="button"
                onClick={copy}
                title="프롬프트 복사 — 외부 AI 에 붙여넣기"
                className={cn(
                  "inline-flex h-9 items-center gap-1.5 rounded-md border px-3 text-[12px] font-medium transition-colors",
                  copied
                    ? "border-accent-green bg-accent-green-soft text-accent-green"
                    : "border-hairline bg-surface-elevated text-body hover:border-hairline-strong hover:text-on-dark",
                )}
              >
                {copied ? <Check size={13} /> : <Copy size={13} />}
                {copied ? "복사됨" : "프롬프트 복사"}
              </button>
              <button
                type="button"
                onClick={compose}
                disabled={composing || !aiConnected}
                title={
                  aiConnected
                    ? `단비가 vault 발췌 보고 ${target} 작성`
                    : "AI 연동을 켜면 활성화됩니다"
                }
                className={cn(
                  "inline-flex h-9 items-center gap-1.5 rounded-md border px-3 text-[12px] font-medium transition-colors",
                  aiConnected
                    ? "border-accent-blue bg-accent-blue text-on-primary hover:bg-primary-pressed"
                    : "border-hairline bg-surface-elevated/40 text-stone",
                )}
              >
                {composing ? (
                  <Loader2 size={13} className="animate-spin" />
                ) : (
                  <Sparkles size={13} />
                )}
                {composing ? "작성 중…" : "단비가 작성"}
              </button>
            </div>
          </div>
        </div>
      </div>

      {/* Compose 결과 미리보기 모달 — 사용자가 검토 후 적용. */}
      {(composeResult || composeError) && (
        <div
          className="fixed inset-0 z-40 flex items-center justify-center bg-canvas/70 backdrop-blur-sm"
          onMouseDown={(e) => {
            if (e.target === e.currentTarget) {
              setComposeResult(null);
              setComposeError(null);
            }
          }}
        >
          <div className="flex max-h-[80vh] w-[680px] flex-col overflow-hidden rounded-lg border border-hairline bg-surface">
            <header className="flex h-11 shrink-0 items-center justify-between border-b border-hairline px-4">
              <span className="inline-flex items-center gap-2 text-[14px] font-medium text-ink">
                <Sparkles size={14} className="text-accent-blue" />
                {project} · {target} 미리보기
              </span>
              <button
                onClick={() => {
                  setComposeResult(null);
                  setComposeError(null);
                }}
                className="grid h-7 w-7 place-items-center rounded-sm text-mute transition-colors hover:bg-surface-elevated hover:text-on-dark"
              >
                <X size={13} />
              </button>
            </header>
            <div className="flex-1 overflow-y-auto px-5 py-4">
              {composeError ? (
                <div className="rounded-md border border-accent-red/40 bg-accent-red-soft/30 p-3 text-[12.5px] leading-[1.6] text-accent-red">
                  {composeError}
                </div>
              ) : composeResult ? (
                <pre className="whitespace-pre-wrap font-sans text-[13px] leading-[1.65] text-body">
                  {composeResult.markdown}
                </pre>
              ) : null}
            </div>
            {composeResult && (
              <footer className="flex shrink-0 items-center justify-between gap-3 border-t border-hairline bg-surface-elevated px-4 py-2.5 text-[12px]">
                <span className="text-stone">
                  via {composeResult.provider} · 적용하면 현재 빈 골격을
                  이 내용으로 덮어써요
                </span>
                <div className="flex items-center gap-2">
                  <button
                    type="button"
                    onClick={() => {
                      setComposeResult(null);
                      setComposeError(null);
                    }}
                    className="inline-flex h-7 items-center gap-1 rounded-sm border border-hairline bg-surface px-2 text-body hover:border-hairline-strong hover:text-on-dark"
                  >
                    취소
                  </button>
                  <button
                    type="button"
                    onClick={apply}
                    disabled={applying}
                    className="inline-flex h-7 items-center gap-1 rounded-sm border border-accent-blue bg-accent-blue px-3 font-medium text-on-primary hover:bg-primary-pressed"
                  >
                    {applying ? (
                      <Loader2 size={11} className="animate-spin" />
                    ) : (
                      <Check size={11} />
                    )}
                    {applying ? "적용 중…" : "이대로 적용"}
                  </button>
                </div>
              </footer>
            )}
          </div>
        </div>
      )}
    </>
  );
}
