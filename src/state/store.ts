import { create } from "zustand";
import type {
  Attachment,
  DanbiConfig,
  Intent,
  LinkIndex,
  PlanPreview,
  ProjectGroup,
  ProjectJournalView,
  RoutingResult,
  VaultTree,
} from "@/lib/ipc";

export type Selection = {
  project: string | null;
  domain: string | null;
};

export type ChatTurn = {
  id: string;
  user: string;
  status:
    | "routing"
    | "clarify"
    | "planning"
    | "ready"
    | "applying"
    | "applied"
    | "cancelled"
    | "error";
  route?: RoutingResult;
  plan?: PlanPreview;
  attachments?: Attachment[];
  commitAfter?: string | null;
  error?: string;
  createdAt: number;
};

export type AppStore = {
  cfg: DanbiConfig | null;
  setCfg: (cfg: DanbiConfig) => void;

  tree: VaultTree | null;
  setTree: (t: VaultTree) => void;

  selection: Selection;
  selectProject: (project: string | null) => void;
  selectDomain: (project: string, domain: string) => void;
  clearSelection: () => void;

  turns: ChatTurn[];
  addTurn: (t: ChatTurn) => void;
  patchTurn: (id: string, patch: Partial<ChatTurn>) => void;
  clearTurns: () => void;

  linkIndex: LinkIndex | null;
  setLinkIndex: (idx: LinkIndex) => void;

  /** User-defined project groups for the sidebar. Mirrored from
   *  `DanbiConfig.project_groups` on load so other components can read it
   *  without re-parsing cfg. */
  groups: ProjectGroup[];
  setGroups: (g: ProjectGroup[]) => void;

  /** project name → commits since last seen. Drives the "N" badge in
   *  the sidebar. Optimistically set to 0 locally when the user opens
   *  a project; server refreshes on next load. */
  projectUpdates: Record<string, number>;
  setProjectUpdates: (u: Record<string, number>) => void;
  clearProjectUpdate: (project: string) => void;

  /** "<project>/<domain>" → "new" | "modified". Per-domain change
   *  flags for the sidebar's domain row dot/badge. Cleared one-by-one
   *  as the user opens each domain. */
  domainUpdates: Record<string, "new" | "modified">;
  setDomainUpdates: (u: Record<string, "new" | "modified">) => void;
  clearDomainUpdate: (project: string, domain: string) => void;

  /** Cached Auto-Journal view per project. Lets ProjectHome render
   *  instantly on re-entry while a background fetch refreshes it
   *  (stale-while-revalidate). */
  projectJournalCache: Record<string, ProjectJournalView>;
  setProjectJournal: (project: string, v: ProjectJournalView) => void;

  /** 백그라운드로 진행 중인 LLM 작업 — daily 요약, purpose 작성, ghost
   *  스캔 등. 사이드바 푸터의 progress pill 이 이걸 보고 indeterminate
   *  바를 그리고, 완료되면 클릭 가능 결과 + macOS 알림으로 신호.
   *
   *  status:
   *   - "running"  : 진행 중. footer 에 progress 표시.
   *   - "done"     : 완료. exportId 가 있으면 클릭 시 다시 열기.
   *   - "error"    : 실패. message 표시.
   *  null = pill 자체 미표시. */
  bgJob: BgJob | null;
  setBgJob: (job: BgJob | null) => void;

  /** 알림 history — 종 아이콘 popover 에서 보여줌. 요약 완료, 에러,
   *  큰 결정 등이 이 list 에 누적된다. 사용자가 popover 를 열거나
   *  notification 을 클릭하면 read 처리. 메모리에만 보관 (앱 재시작 시
   *  비워짐) — 영구 기록은 vault 의 export history 가 별도로 함. */
  notifications: AppNotification[];
  pushNotification: (n: Omit<AppNotification, "id" | "createdAt" | "read">) => void;
  markAllNotificationsRead: () => void;
  clearNotifications: () => void;

  /** 사용자가 단비 안에서 노트를 직접 저장한 시점 (ms epoch).
   *  watcher 가 그 직후 발화하는 vault:changed 이벤트는 우리가 만든
   *  것이므로 vault refresh 한 번 건너뛰어도 안전 — 큰 vault 에서는
   *  이 skip 이 에디터 체감 속도를 크게 좌우한다. */
  lastSelfSaveAt: number;
  markSelfSave: () => void;

  /** purpose / schema 자동 작성이 끝난 결과를 잠시 보관. 사용자가
   *  알림 / toast 를 클릭해 해당 노트로 이동하면 PurposeSchemaHint 가
   *  이걸 보고 즉시 미리보기 모달을 띄운다. 적용·취소 시 null 로 비움. */
  pendingCompose: {
    project: string;
    target: "purpose" | "schema";
    markdown: string;
    provider: string;
  } | null;
  setPendingCompose: (v: AppStore["pendingCompose"]) => void;

  /** Tauri updater 가 새 버전을 발견하면 여기에 메타가 들어온다.
   *  사이드바 footer 의 UpdatePill 이 이걸 보고 "v0.4.0 사용 가능" 을
   *  띄움. 사용자가 다운로드를 시작하면 status 가 진행되며,
   *  완료 후 재시작 액션까지 같은 pill 이 안내. */
  updateInfo: UpdateInfo | null;
  setUpdateInfo: (v: UpdateInfo | null) => void;

  /** Lightweight global toast — 어떤 컴포넌트에서든 짧은 성공/실패
   *  메시지를 띄울 수 있게 하는 store-level 슬롯. Workspace 의
   *  로컬 toast 시스템과 별개로 동작 — 둘 다 화면 우상단에 동시에
   *  떠도 된다 (Z-스택 분리). */
  toast: { tone: "ok" | "err"; text: string; id: number } | null;
  showToast: (tone: "ok" | "err", text: string) => void;
};

export type UpdateInfo =
  | {
      status: "available";
      version: string;
      currentVersion: string;
      notes: string | null;
    }
  | {
      status: "downloading";
      version: string;
      progress: number; // 0..1
    }
  | {
      status: "ready";
      version: string;
    }
  | {
      status: "error";
      version: string | null;
      message: string;
    };

export type AppNotification = {
  id: string;
  createdAt: number;
  read: boolean;
  tone: "ok" | "err" | "info";
  title: string;
  body?: string;
  /** 클릭 시 동작. */
  action?:
    | { kind: "open-export"; exportId: string }
    | { kind: "select-domain"; project: string; domain: string }
    | {
        kind: "open-compose";
        project: string;
        target: "purpose" | "schema";
        markdown: string;
      }
    | { kind: "open-graph"; project: string };
};

/** 백그라운드 LLM 작업.
 *  - summarize : daily 노트 요약 + HTML export. done 시 exportId 로 페이지 열기.
 *  - compose   : purpose / schema 자동 작성. done 시 미리보기 모달로 (적용 전).
 *  - ghost     : ghost links 자동 제안. done 시 그래프에서 점선으로 자동 표시.
 *
 *  status 별 공통 필드는 project, startedAt/finishedAt. action 결과는
 *  kind 마다 다른 자리 (exportId, markdown, count). */
export type BgJob =
  | {
      kind: "summarize";
      status: "running";
      project: string;
      domain: string;
      startedAt: number;
    }
  | {
      kind: "summarize";
      status: "done";
      project: string;
      domain: string;
      exportId: string;
      finishedAt: number;
    }
  | {
      kind: "summarize";
      status: "error";
      project: string;
      domain: string;
      message: string;
      finishedAt: number;
    }
  | {
      kind: "compose";
      target: "purpose" | "schema";
      status: "running";
      project: string;
      startedAt: number;
    }
  | {
      kind: "compose";
      target: "purpose" | "schema";
      status: "done";
      project: string;
      markdown: string;
      provider: string;
      finishedAt: number;
    }
  | {
      kind: "compose";
      target: "purpose" | "schema";
      status: "error";
      project: string;
      message: string;
      finishedAt: number;
    }
  | {
      kind: "ghost";
      status: "running";
      project: string;
      startedAt: number;
    }
  | {
      kind: "ghost";
      status: "done";
      project: string;
      pendingCount: number;
      finishedAt: number;
    }
  | {
      kind: "ghost";
      status: "error";
      project: string;
      message: string;
      finishedAt: number;
    }
  /** vault 구조 변경 작업 (프로젝트 생성/이름 변경/삭제). LLM 작업은 아니지만
   *  큰 vault 에선 mutation + 후속 listTree/buildLinkIndex 가 수 초 걸려
   *  사용자가 다이얼로그 누른 후 마우스 progress 만 보게 된다. 다이얼로그는
   *  즉시 닫고 이 toast 로 진행 상태를 알린다. */
  | {
      kind: "vault";
      op: "create" | "rename" | "delete";
      status: "running";
      project: string;
      startedAt: number;
    }
  | {
      kind: "vault";
      op: "create" | "rename" | "delete";
      status: "done";
      project: string;
      finishedAt: number;
    }
  | {
      kind: "vault";
      op: "create" | "rename" | "delete";
      status: "error";
      project: string;
      message: string;
      finishedAt: number;
    };

/** Max chat turns kept in memory. Older turns are dropped to bound RAM —
 *  each turn can carry full extracted attachment text + plan/diff payloads. */
const MAX_TURNS = 100;

export const useApp = create<AppStore>((set) => ({
  cfg: null,
  setCfg: (cfg) => set({ cfg }),
  tree: null,
  setTree: (t) => set({ tree: t }),
  selection: { project: null, domain: null },
  selectProject: (project) => set({ selection: { project, domain: null } }),
  selectDomain: (project, domain) => set({ selection: { project, domain } }),
  clearSelection: () => set({ selection: { project: null, domain: null } }),

  turns: [],
  // v0.8.0: cap the chat log. Each turn can pin heavy payloads —
  // extracted attachment text, routing results, plan/diff previews — so an
  // unbounded array was the main frontend RAM climb over a long session.
  // Keep the most recent MAX_TURNS (matches the `notifications` cap pattern).
  addTurn: (t) =>
    set((s) => ({ turns: [...s.turns, t].slice(-MAX_TURNS) })),
  patchTurn: (id, patch) =>
    set((s) => ({
      turns: s.turns.map((t) => (t.id === id ? { ...t, ...patch } : t)),
    })),
  clearTurns: () => set({ turns: [] }),

  linkIndex: null,
  setLinkIndex: (idx) => set({ linkIndex: idx }),

  groups: [],
  setGroups: (g) => set({ groups: g }),

  projectUpdates: {},
  setProjectUpdates: (u) => set({ projectUpdates: u }),
  clearProjectUpdate: (project) =>
    set((s) => {
      if (!s.projectUpdates[project]) return s;
      const next = { ...s.projectUpdates };
      delete next[project];
      return { projectUpdates: next };
    }),

  domainUpdates: {},
  setDomainUpdates: (u) => set({ domainUpdates: u }),
  clearDomainUpdate: (project, domain) =>
    set((s) => {
      const key = `${project}/${domain}`;
      if (!s.domainUpdates[key]) return s;
      const next = { ...s.domainUpdates };
      delete next[key];
      return { domainUpdates: next };
    }),

  projectJournalCache: {},
  setProjectJournal: (project, v) =>
    set((s) => ({
      projectJournalCache: { ...s.projectJournalCache, [project]: v },
    })),

  bgJob: null,
  setBgJob: (job) => set({ bgJob: job }),

  lastSelfSaveAt: 0,
  markSelfSave: () => set({ lastSelfSaveAt: Date.now() }),

  pendingCompose: null,
  setPendingCompose: (v) => set({ pendingCompose: v }),

  updateInfo: null,
  setUpdateInfo: (v) => set({ updateInfo: v }),

  toast: null,
  showToast: (tone, text) => {
    const id = Date.now() + Math.floor(Math.random() * 1000);
    set({ toast: { tone, text, id } });
    // Auto-dismiss after 2.4s — same as the Workspace local toast.
    // We check the id before clearing so a newer toast doesn't get
    // wiped by an older one's timer.
    setTimeout(() => {
      set((s) => (s.toast?.id === id ? { toast: null } : s));
    }, 2400);
  },

  notifications: [],
  pushNotification: (n) =>
    set((s) => ({
      notifications: [
        {
          id: `n-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 6)}`,
          createdAt: Date.now(),
          read: false,
          ...n,
        },
        ...s.notifications,
      ].slice(0, 50), // 최대 50개 유지
    })),
  markAllNotificationsRead: () =>
    set((s) => ({
      notifications: s.notifications.map((n) => ({ ...n, read: true })),
    })),
  clearNotifications: () => set({ notifications: [] }),
}));

export type IntentLabel = { label: string; tone: "neutral" | "write" | "ask" };

export function intentMeta(intent: Intent): IntentLabel {
  switch (intent) {
    case "append":
      return { label: "추가", tone: "write" };
    case "rewrite":
      return { label: "재작성", tone: "write" };
    case "summarize":
      return { label: "요약", tone: "write" };
    case "ask":
      return { label: "질문", tone: "ask" };
    case "compound":
      return { label: "합성", tone: "write" };
    default:
      return { label: "불확실", tone: "neutral" };
  }
}
