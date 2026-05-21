import { useEffect, useState } from "react";
import {
  Copy,
  ExternalLink,
  FolderOpen,
  HardDrive,
  Info,
  Keyboard,
  Loader2,
  Moon,
  Palette,
  Pencil,
  Plug,
  RefreshCw,
  Settings as SettingsIcon,
  Sparkles,
  X,
} from "lucide-react";
import { openPath } from "@tauri-apps/plugin-opener";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { writeText as clipboardWriteText } from "@tauri-apps/plugin-clipboard-manager";
import {
  ipc,
  type DanbiConfig,
  type McpStatus,
} from "@/lib/ipc";
import { cn } from "@/lib/utils";
import { useApp } from "@/state/store";
import appIconUrl from "@/assets/danbi-app-icon.png";

type Section =
  | "appearance"
  | "shortcuts"
  | "vault"
  | "editor"
  | "mcp"
  | "backup"
  | "vector"
  | "about";

const SECTIONS_ALL: { id: Section; label: string; icon: typeof Moon }[] = [
  { id: "appearance", label: "외관", icon: Palette },
  { id: "vector", label: "AI 연동", icon: Sparkles },
  { id: "shortcuts", label: "단축키", icon: Keyboard },
  { id: "vault", label: "Vault", icon: FolderOpen },
  { id: "editor", label: "편집", icon: Pencil },
  { id: "mcp", label: "MCP", icon: Plug },
  { id: "backup", label: "백업", icon: HardDrive },
  { id: "about", label: "정보", icon: Info },
];

export function Settings({
  open,
  onClose,
  initialSection,
}: {
  open: boolean;
  onClose: () => void;
  /** Section to focus on every (re)open. The banner's "Provider 연결"
   *  button passes "llm" so users land on the right pane. */
  initialSection?: Section;
}) {
  const cfg = useApp((s) => s.cfg);
  // 0.1: 단비에 LLM 키를 넣지 않으므로 LLM 섹션은 Section 타입 단계에서
  // 이미 제거됨. SECTIONS_ALL 이 곧 노출되는 섹션 목록.
  const SECTIONS = SECTIONS_ALL;
  const [section, setSection] = useState<Section>(
    initialSection ?? "appearance",
  );

  // Re-focus the requested section every time the dialog is opened —
  // not just on first mount, so the banner can deep-link reliably.
  useEffect(() => {
    if (open && initialSection) setSection(initialSection);
  }, [open, initialSection]);

  useEffect(() => {
    if (!open) return;
    function onKey(e: KeyboardEvent) {
      if (e.key === "Escape") onClose();
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [open, onClose]);

  if (!open || !cfg) return null;

  return (
    <div
      className="fixed inset-0 z-40 flex items-center justify-center bg-canvas/70 backdrop-blur-sm"
      onMouseDown={(e) => {
        if (e.target === e.currentTarget) onClose();
      }}
    >
      <div className="flex h-[720px] w-[960px] overflow-hidden rounded-lg border border-hairline bg-surface">
        {/* Sidebar */}
        <aside className="flex w-56 shrink-0 flex-col border-r border-hairline bg-surface">
          <header
            data-tauri-drag-region
            className="flex h-12 items-center gap-2 border-b border-hairline px-5 text-[15px] font-medium text-ink"
          >
            <SettingsIcon size={16} className="text-accent-blue" />
            설정
          </header>
          <nav className="flex-1 py-2">
            {SECTIONS.map((s) => {
              const Icon = s.icon;
              const active = section === s.id;
              return (
                <button
                  key={s.id}
                  onClick={() => setSection(s.id)}
                  className={cn(
                    "relative flex w-full items-center gap-2.5 px-5 py-2 text-left text-[14px] transition-colors",
                    active
                      ? "bg-accent-blue-soft font-medium text-accent-blue"
                      : "text-body hover:bg-surface-elevated hover:text-on-dark",
                  )}
                >
                  {/* 좌측 가장자리 accent strip — 활성 메뉴를 두 번째 시각
                      신호 (배경 + strip) 로 강조해서 한 줄에서 어디 있는지
                      바로 읽힌다. */}
                  {active && (
                    <span className="absolute inset-y-1 left-0 w-[3px] rounded-r-full bg-accent-blue" />
                  )}
                  <Icon
                    size={15}
                    className={active ? "text-accent-blue" : "text-mute"}
                  />
                  {s.label}
                </button>
              );
            })}
          </nav>
          <footer className="border-t border-hairline px-5 py-2.5 text-[12px] text-stone">
            v0.2
          </footer>
        </aside>

        {/* Body */}
        <div className="flex flex-1 flex-col">
          <header
            data-tauri-drag-region
            className="flex h-12 items-center justify-between border-b border-hairline px-6"
          >
            <span className="text-[15px] font-medium text-ink">
              {SECTIONS.find((s) => s.id === section)?.label}
            </span>
            <button
              onClick={onClose}
              className="grid h-9 w-9 place-items-center rounded-md text-mute transition-colors hover:bg-surface-elevated hover:text-on-dark"
              title="닫기 (Esc)"
            >
              <X size={18} />
            </button>
          </header>
          <div className="flex-1 min-h-0 overflow-auto px-7 py-6">
            {section === "appearance" && <AppearancePanel cfg={cfg} />}
            {section === "shortcuts" && <ShortcutsPanel cfg={cfg} />}
            {section === "vault" && <VaultPanel cfg={cfg} />}
            {section === "editor" && <EditorPanel cfg={cfg} />}
            {section === "mcp" && <McpPanel />}
            {section === "backup" && <BackupPanel cfg={cfg} />}
            {section === "vector" && <VectorPanel />}
            {section === "about" && <AboutPanel cfg={cfg} />}
          </div>
        </div>
      </div>
    </div>
  );
}

// ---------------- Sections ----------------

function SectionTitle({
  title,
  hint,
}: {
  title: string;
  hint?: string;
}) {
  return (
    <div className="mb-4">
      <h2 className="text-[17px] font-medium text-ink">{title}</h2>
      {hint && <p className="mt-1 text-[12.5px] leading-[1.55] text-mute">{hint}</p>}
    </div>
  );
}


function Row({
  label,
  hint,
  children,
  stack = false,
}: {
  label: string;
  hint?: string;
  children: React.ReactNode;
  /** Stack children below the label on the full row width, instead of sitting
   *  to the right. Use when children need horizontal room (long paths, input
   *  + multiple buttons). */
  stack?: boolean;
}) {
  if (stack) {
    return (
      <div className="flex flex-col gap-2.5 border-b border-hairline py-4 last:border-b-0">
        <div className="min-w-0">
          <div className="text-[14px] text-ink">{label}</div>
          {hint && (
            <div className="mt-1 text-[12px] leading-[1.55] text-mute">
              {hint}
            </div>
          )}
        </div>
        <div>{children}</div>
      </div>
    );
  }
  return (
    <div className="flex items-start justify-between gap-6 border-b border-hairline py-4 last:border-b-0">
      <div className="min-w-0 flex-1">
        <div className="text-[14px] text-ink">{label}</div>
        {hint && (
          <div className="mt-1 text-[12px] leading-[1.55] text-mute">
            {hint}
          </div>
        )}
      </div>
      <div className="shrink-0">{children}</div>
    </div>
  );
}

function Toggle({
  value,
  onChange,
}: {
  value: boolean;
  onChange: (v: boolean) => void;
}) {
  return (
    <button
      onClick={() => onChange(!value)}
      className={cn(
        "relative inline-flex h-5 w-9 items-center rounded-full border border-hairline transition-colors",
        value ? "bg-accent-blue/70" : "bg-surface-elevated",
      )}
    >
      <span
        className={cn(
          "h-3 w-3 rounded-full bg-on-dark transition-transform",
          value ? "translate-x-[18px]" : "translate-x-[2px]",
        )}
      />
    </button>
  );
}

function Segmented({
  value,
  options,
  onChange,
}: {
  value: string;
  options: { value: string; label: string }[];
  onChange: (v: string) => void;
}) {
  return (
    <div className="inline-flex rounded-md border border-hairline bg-surface-elevated p-0.5">
      {options.map((o) => {
        const active = o.value === value;
        return (
          <button
            key={o.value}
            onClick={() => onChange(o.value)}
            className={cn(
              "rounded-sm px-2 py-0.5 text-[12px] transition-colors",
              active
                ? "bg-surface-card text-on-dark"
                : "text-body hover:text-on-dark",
            )}
          >
            {o.label}
          </button>
        );
      })}
    </div>
  );
}

function patchCfg(cfg: DanbiConfig, patch: Partial<DanbiConfig>): DanbiConfig {
  return { ...cfg, ...patch };
}

async function saveCfg(cfg: DanbiConfig) {
  if (!cfg.vault_path) return;
  await ipc.saveConfig(cfg.vault_path, cfg);
  useApp.getState().setCfg(cfg);
}

function AppearancePanel({ cfg }: { cfg: DanbiConfig }) {
  // 변경 표시 마스터 ON/OFF — 셋 중 하나라도 켜져 있으면 ON 으로 본다.
  // master 토글을 끄면 셋 다 한꺼번에 OFF, 켜면 셋 다 ON. 세부 토글은
  // master 가 ON 일 때만 펼쳐서 노출.
  const trayOn = cfg.appearance.tray_badge ?? true;
  const sidebarDotsOn = cfg.appearance.unseen_sidebar_dots ?? true;
  const projectCountOn = cfg.appearance.unseen_project_count ?? true;
  const visibilityOn = trayOn || sidebarDotsOn || projectCountOn;

  return (
    <>
      <SectionTitle
        title="외관"
        hint="단비는 다크 모드 기준으로 디자인됐습니다."
      />

      {/* 변경 표시 master 카드 — 한 번 누르면 메뉴바 뱃지 + 사이드바 dot
          + 프로젝트 카운트 셋 다 동시 토글. OFF 면 하위 토글 숨김. */}
      <div className="flex items-center justify-between gap-4 rounded-lg border border-hairline bg-surface-card p-4">
        <div className="flex flex-col gap-1">
          <div className="text-[14px] font-medium text-on-dark">변경 표시</div>
          <div className="text-caption-sm leading-[1.5] text-mute">
            {visibilityOn
              ? "메뉴바 뱃지·사이드바 dot·프로젝트 카운트로 vault 변경을 알립니다."
              : "변경 표시가 모두 꺼져 있어요. 켜면 세부 항목을 따로 조절할 수 있습니다."}
          </div>
        </div>
        <button
          type="button"
          aria-pressed={visibilityOn}
          onClick={() => {
            const next = !visibilityOn;
            saveCfg(
              patchCfg(cfg, {
                appearance: {
                  ...cfg.appearance,
                  tray_badge: next,
                  unseen_sidebar_dots: next,
                  unseen_project_count: next,
                },
              }),
            );
            ipc.trayBadgeSetEnabled(next).catch(() => {});
          }}
          className={cn(
            "relative inline-flex h-9 w-[88px] shrink-0 items-center rounded-full border px-1 transition-colors",
            visibilityOn
              ? "border-accent-blue bg-accent-blue"
              : "border-hairline bg-surface-elevated",
          )}
        >
          <span
            className={cn(
              "inline-block h-7 w-7 rounded-full bg-white shadow-sm transition-transform",
              visibilityOn ? "translate-x-[52px]" : "translate-x-0",
            )}
          />
          <span
            className={cn(
              "absolute text-[12px] font-medium",
              visibilityOn ? "left-3 text-on-primary" : "right-3 text-stone",
            )}
          >
            {visibilityOn ? "ON" : "OFF"}
          </span>
        </button>
      </div>

      {visibilityOn && (
        <>
          <Row
            label="메뉴바 뱃지"
            hint="메인창이 꺼져 있을 때 vault 내 .md 파일 변경이 생기면 메뉴바 아이콘 옆에 카운트를 표시합니다. 메인창이나 팝오버를 열면 자동으로 0 으로 리셋됩니다."
          >
            <Toggle
              value={trayOn}
              onChange={(v) => {
                saveCfg(
                  patchCfg(cfg, {
                    appearance: { ...cfg.appearance, tray_badge: v },
                  }),
                );
                ipc.trayBadgeSetEnabled(v).catch(() => {});
              }}
            />
          </Row>
          <Row
            label="사이드바 변경 dot"
            hint="새로 생기거나 수정된 도메인 파일 옆에 작은 점을 띄웁니다. 폴더 행에는 변경 파일 수가 작은 뱃지로 표시됩니다. 끄면 둘 다 사라집니다."
          >
            <Toggle
              value={sidebarDotsOn}
              onChange={(v) =>
                saveCfg(
                  patchCfg(cfg, {
                    appearance: { ...cfg.appearance, unseen_sidebar_dots: v },
                  }),
                )
              }
            />
          </Row>
          <Row
            label="프로젝트 변경 카운트"
            hint="프로젝트 row 우측에 변경 파일 개수를 N 형태로 보여줍니다. 끄면 프로젝트 row 가 깔끔해집니다."
          >
            <Toggle
              value={projectCountOn}
              onChange={(v) =>
                saveCfg(
                  patchCfg(cfg, {
                    appearance: { ...cfg.appearance, unseen_project_count: v },
                  }),
                )
              }
            />
          </Row>
        </>
      )}
    </>
  );
}

function VaultPanel({ cfg }: { cfg: DanbiConfig }) {
  return (
    <>
      <SectionTitle title="Vault" hint="모든 프로젝트 파일이 저장되는 로컬 폴더입니다." />
      <Row label="경로">
        <code className="rounded-sm border border-hairline bg-surface-elevated px-2 py-1 font-mono text-[11px] text-on-dark-mute">
          {cfg.vault_path ?? "—"}
        </code>
      </Row>
      <Row label="폴더 열기" hint="Finder 로 Vault 폴더를 엽니다.">
        <button
          disabled={!cfg.vault_path}
          onClick={() => cfg.vault_path && openPath(cfg.vault_path)}
          className="inline-flex h-7 items-center gap-1 rounded-sm border border-hairline bg-surface-elevated px-2 text-[12px] text-body hover:text-on-dark"
        >
          <ExternalLink size={11} /> 열기
        </button>
      </Row>
      <Row label="log.md 열기" hint="작업 타임라인을 기록하는 인간 가독용 로그입니다.">
        <button
          disabled={!cfg.vault_path}
          onClick={() =>
            cfg.vault_path && openPath(`${cfg.vault_path}/log.md`)
          }
          className="inline-flex h-7 items-center gap-1 rounded-sm border border-hairline bg-surface-elevated px-2 text-[12px] text-body hover:text-on-dark"
        >
          <ExternalLink size={11} /> 열기
        </button>
      </Row>
    </>
  );
}


function EditorPanel({ cfg }: { cfg: DanbiConfig }) {
  return (
    <>
      <SectionTitle title="편집" hint="중앙 문서 뷰의 동작 방식을 조정합니다." />
      <Row
        label="자동 저장"
        hint="포커스가 에디터에서 벗어나면 자동으로 저장합니다. (예정, 아직 비활성)"
      >
        <Toggle
          value={cfg.editor.autosave}
          onChange={(v) =>
            saveCfg(patchCfg(cfg, { editor: { ...cfg.editor, autosave: v } }))
          }
        />
      </Row>
      <Row
        label="긴 줄 자동 줄바꿈"
        hint="긴 라인을 가로 스크롤 없이 줄바꿈해서 보여줍니다."
      >
        <Toggle
          value={cfg.editor.word_wrap}
          onChange={(v) =>
            saveCfg(
              patchCfg(cfg, { editor: { ...cfg.editor, word_wrap: v } }),
            )
          }
        />
      </Row>
    </>
  );
}

function ShortcutsPanel({ cfg }: { cfg: DanbiConfig }) {
  const current = cfg.shortcuts.quick_capture;
  const isOn = current.trim() !== "";
  const [draft, setDraft] = useState<string>(current);
  const [recording, setRecording] = useState(false);
  const [err, setErr] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);

  useEffect(() => {
    setDraft(current);
  }, [current]);

  useEffect(() => {
    if (!recording) return;
    function onKey(e: KeyboardEvent) {
      e.preventDefault();
      e.stopPropagation();
      const mods: string[] = [];
      if (e.metaKey) mods.push("Command");
      if (e.ctrlKey) mods.push("Control");
      if (e.altKey) mods.push("Alt");
      if (e.shiftKey) mods.push("Shift");

      const keyName = keyFromEvent(e);
      if (!keyName) return;
      const accelerator = [...mods, keyName].join("+");
      setDraft(accelerator);
      setRecording(false);
      setErr(null);
    }
    window.addEventListener("keydown", onKey, { capture: true });
    return () =>
      window.removeEventListener("keydown", onKey, {
        capture: true,
      } as EventListenerOptions);
  }, [recording]);

  const changed = draft !== current;

  // 적용 + 저장. 빈 문자열은 비활성으로 해석돼 keybinding 만 해제.
  async function apply(next: string) {
    setSaving(true);
    setErr(null);
    try {
      if (next.trim()) await ipc.validateShortcut(next);
      await ipc.applyCaptureShortcut(next);
      await saveCfg(
        patchCfg(cfg, {
          shortcuts: { ...cfg.shortcuts, quick_capture: next },
        }),
      );
    } catch (e) {
      setErr(String(e));
    } finally {
      setSaving(false);
    }
  }

  return (
    <>
      <SectionTitle
        title="단축키"
        hint="Quick Capture 글로벌 단축키. 처음 설치 시 시스템 단축키 충돌 방지를 위해 OFF 입니다."
      />

      {/* 상단 큰 ON/OFF 카드 — 한 번 누르면 바로 적용된다.
          OFF 일 때는 이 카드 하나만 보이고, ON 으로 켜야 키 녹화 UI 가
          나타난다. UI 가 두 단으로 갈라지는 게 가장 명확. */}
      <div className="mb-5 flex items-center justify-between gap-4 rounded-lg border border-hairline bg-surface-card p-4">
        <div className="flex flex-col gap-1">
          <div className="flex items-center gap-2 text-[14px] font-medium text-on-dark">
            <Keyboard size={14} className="text-accent-blue" />
            Quick Capture 단축키
          </div>
          <div className="text-caption-sm leading-[1.5] text-mute">
            {isOn
              ? "어느 앱에서든 아래 조합을 누르면 단비 팝업이 뜹니다."
              : "비활성 상태입니다. 켜면 글로벌 키 조합을 등록할 수 있어요."}
          </div>
        </div>
        <button
          type="button"
          disabled={saving}
          onClick={() => {
            // ON → OFF: 즉시 비활성. 저장된 조합은 잊는다.
            // OFF → ON: 일단 Control+Space 로 켠 뒤, 사용자가 녹화로 변경.
            apply(isOn ? "" : "Control+Space");
          }}
          aria-pressed={isOn}
          className={cn(
            "relative inline-flex h-9 w-[88px] shrink-0 items-center rounded-full border px-1 transition-colors",
            isOn
              ? "border-accent-blue bg-accent-blue"
              : "border-hairline bg-surface-elevated",
            saving && "opacity-60",
          )}
        >
          <span
            className={cn(
              "inline-block h-7 w-7 rounded-full bg-white shadow-sm transition-transform",
              isOn ? "translate-x-[52px]" : "translate-x-0",
            )}
          />
          <span
            className={cn(
              "absolute text-[12px] font-medium",
              isOn ? "left-3 text-on-primary" : "right-3 text-stone",
            )}
          >
            {isOn ? "ON" : "OFF"}
          </span>
        </button>
      </div>

      {isOn && (
        <>
          <Row
            label="키 조합"
            hint="녹화 버튼을 누른 뒤 원하는 키 조합을 누르세요. 저장 버튼으로 적용됩니다."
          >
            <div className="flex items-center gap-2">
              <div
                className={cn(
                  "inline-flex h-8 min-w-[140px] items-center justify-center rounded-md border px-3 font-mono text-[13px]",
                  recording
                    ? "border-accent-blue bg-accent-blue-soft text-accent-blue"
                    : "border-hairline bg-surface-elevated text-ink",
                )}
              >
                {recording
                  ? "키 조합을 눌러주세요…"
                  : prettyAccelerator(draft || current)}
              </div>
              <button
                onClick={() => setRecording(true)}
                disabled={recording}
                className="inline-flex h-8 items-center gap-1 rounded-md border border-hairline bg-surface-elevated px-3 text-[12px] text-body hover:border-hairline-strong hover:text-on-dark"
              >
                <Keyboard size={12} /> 녹화
              </button>
              <button
                disabled={!changed || recording || saving}
                onClick={() => apply(draft)}
                className={cn(
                  "inline-flex h-8 items-center rounded-md px-3 text-[12px] font-medium transition-colors",
                  changed && !recording && !saving
                    ? "bg-primary text-on-primary hover:bg-primary-pressed"
                    : "bg-surface-elevated text-ash",
                )}
              >
                {saving ? "저장 중…" : "저장"}
              </button>
              {changed && !recording && (
                <button
                  onClick={() => {
                    setDraft(current);
                    setErr(null);
                  }}
                  className="inline-flex h-8 items-center rounded-md px-2 text-[12px] text-stone hover:text-on-dark"
                >
                  취소
                </button>
              )}
            </div>
          </Row>

          <Row
            label="빠른 초기화"
            hint="자주 쓰는 조합으로 입력란을 채워줍니다. 저장 버튼을 눌러야 적용됩니다."
          >
            <button
              onClick={() => setDraft("Control+Space")}
              className="inline-flex h-8 items-center rounded-md border border-hairline bg-surface-elevated px-3 text-[12px] text-body hover:border-hairline-strong hover:text-on-dark"
            >
              Control+Space 사용
            </button>
          </Row>

          {err && (
            <div className="mt-3 rounded-md border border-hairline bg-surface-elevated p-2 font-mono text-[12px] text-accent-red">
              {err}
            </div>
          )}

          <div className="mt-4 rounded-md border border-hairline bg-surface-elevated p-3 text-caption-sm leading-[1.6] text-mute">
            <div className="mb-1 text-body">규칙</div>
            <ul className="list-disc space-y-0.5 pl-4">
              <li>
                수식어(Modifier) 한 개 이상 + 일반 키 한 개 — 예:{" "}
                <code className="text-on-dark">Control+Space</code>
              </li>
              <li>
                다른 앱이 선점한 조합은 등록되지 않아요. 에러가 나면 다른
                조합으로 시도하세요.
              </li>
              <li>
                macOS는 <code>Command</code>, 다른 OS는 <code>Control</code> 가
                주 수식어로 쓰여요.
              </li>
            </ul>
          </div>
        </>
      )}
    </>
  );
}

function prettyAccelerator(s: string): string {
  return s
    .split("+")
    .map((p) => {
      const t = p.trim();
      const lower = t.toLowerCase();
      if (lower === "command" || lower === "cmd" || lower === "super" || lower === "meta")
        return "⌘";
      if (lower === "control" || lower === "ctrl") return "⌃";
      if (lower === "shift") return "⇧";
      if (lower === "alt" || lower === "option" || lower === "opt") return "⌥";
      if (lower === "space" || lower === "spacebar") return "Space";
      return t.length === 1 ? t.toUpperCase() : t;
    })
    .join(" ");
}

function keyFromEvent(e: KeyboardEvent): string | null {
  // Ignore pure modifier presses.
  if (
    e.key === "Meta" ||
    e.key === "Control" ||
    e.key === "Alt" ||
    e.key === "Shift"
  ) {
    return null;
  }
  // Use `code` for stable physical keys where possible.
  const code = e.code;
  if (!code) return mapKey(e.key);
  // Normalize "KeyA" → "A", "Digit1" → "1", others pass through.
  if (code.startsWith("Key")) return code.slice(3);
  if (code.startsWith("Digit")) return code.slice(5);
  switch (code) {
    case "Space":
      return "Space";
    case "Enter":
      return "Enter";
    case "Escape":
      return "Escape";
    case "Tab":
      return "Tab";
    case "Backspace":
      return "Backspace";
    case "Delete":
      return "Delete";
    case "ArrowUp":
      return "Up";
    case "ArrowDown":
      return "Down";
    case "ArrowLeft":
      return "Left";
    case "ArrowRight":
      return "Right";
    case "Comma":
      return ",";
    case "Period":
      return ".";
    case "Slash":
      return "/";
    case "Semicolon":
      return ";";
    case "Quote":
      return "'";
    case "BracketLeft":
      return "[";
    case "BracketRight":
      return "]";
    case "Backslash":
      return "\\";
    case "Backquote":
      return "`";
    case "Minus":
      return "-";
    case "Equal":
      return "=";
    default:
      if (code.startsWith("F") && /^F\d+$/.test(code)) return code;
      return mapKey(e.key);
  }
}

function mapKey(k: string): string | null {
  if (!k || k.length === 0) return null;
  if (k === " ") return "Space";
  return k.length === 1 ? k.toUpperCase() : k;
}

function McpPanel() {
  const [status, setStatus] = useState<McpStatus | null>(null);
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState<string | null>(null);
  const [copied, setCopied] = useState<string | null>(null);

  useEffect(() => {
    ipc.mcpStatus().then(setStatus).catch((e) => setErr(String(e)));
  }, []);

  async function toggle() {
    setBusy(true);
    setErr(null);
    try {
      const next = status?.enabled
        ? await ipc.mcpDisable()
        : await ipc.mcpEnable();
      setStatus(next);
    } catch (e) {
      setErr(String(e));
    } finally {
      setBusy(false);
    }
  }

  async function rotate() {
    setBusy(true);
    setErr(null);
    try {
      setStatus(await ipc.mcpRotateToken());
    } catch (e) {
      setErr(String(e));
    } finally {
      setBusy(false);
    }
  }

  async function copy(label: string, value: string) {
    try {
      await clipboardWriteText(value);
      setCopied(label);
      setTimeout(() => setCopied(null), 1200);
    } catch {
      /* ignore */
    }
  }

  const snippet = status
    ? JSON.stringify(
        {
          mcpServers: {
            danbi: {
              url: status.url,
              headers: { Authorization: `Bearer ${status.token}` },
            },
          },
        },
        null,
        2,
      )
    : "";

  const cliCommand = status
    ? `claude mcp add --transport http danbi "${status.url}" --header "Authorization: Bearer ${status.token}"`
    : "";

  const claudeMdTemplate = `<!--
  단비(Danbi) vault 연동 블록. 이 마커 사이는 단비가 관리한다.
  기존 CLAUDE.md 에 병합하는 규칙:
  1. 파일에 <!-- danbi:start --> ... <!-- danbi:end --> 마커가 이미 있으면
     그 사이만 이 블록 전체로 교체한다. 마커 밖의 사용자 콘텐츠는 건드리지 않는다.
  2. 마커가 없으면 파일 맨 끝에 이 블록 전체를 append 한다.
  3. 사용자 고유 섹션 (프로젝트 특화 빌드 명령, 팀 컨벤션 등) 은 마커 밖에 두면 유지된다.
-->
<!-- danbi:start v=2 project=<이 프로젝트 이름> -->
# <이 프로젝트 이름> — 단비 vault 연동 가이드

이 저장소는 단비(Danbi) MCP 서버의 전용 엔드포인트에 연결돼 있다.
단비 vault는 이 프로젝트의 **외부 장기 기억**이다 — 결정·디버깅·TODO·학습이
세션을 넘어 누적된다.

## 핵심 원칙 (이것만 기억하면 됨)

1. **답변 전에 단비부터 물어본다** — 일반론 대신 vault의 맥락으로 답한다.
2. **중요한 건 즉시 기록한다** — 기술 결정·버그 원인·TODO는 ${"`danbi_log`"}로 바로.
3. **쓸 때는 연결한다** — 관련 문서를 ${"`[[wiki-link]]`"}로 걸어 그래프에 누적시킨다.

## 도구 목록

**읽기** (vault 전체 조회 가능):
- \`danbi_briefing\` — 최근 활동·ghost 제안·고아 파일·daily 노트를 한 JSON으로 반환
- \`danbi_recent\` — 최근 수정된 문서 목록
- \`danbi_search\` — tantivy 전문 검색 (한국어 n-gram 지원)
- \`danbi_read\` — 특정 프로젝트/도메인 파일의 전체 내용
- \`danbi_daily\` — 오늘 + 1주/1달/1년 전 daily 노트
- \`danbi_list_projects\` — vault 전체 프로젝트·도메인 트리

**쓰기** (전용 엔드포인트라 이 프로젝트로 자동 clamp — 다른 프로젝트로 잘못 쓸 위험 0):
- \`danbi_log\` — 오늘 daily 노트에 append. project 파라미터 **생략**.
- \`danbi_append\` — 임의 도메인 파일에 append (없으면 자동 생성). project 파라미터 **생략**.

## Read — 언제 무엇을 읽는가

### 세션 시작 (새 대화 첫 턴)

반드시 이 순서로:

1. \`danbi_briefing\` 호출 → 응답에서 다음 확인:
   - \`daily.today_notes[0]\` 있으면 오늘 이미 기록된 내용이 있다는 뜻
   - \`activity.recent_summaries[0..3]\` 로 직전 작업 맥락 파악
   - \`ghost_suggestions\` 에 pending 있으면 사용자에게 알릴지 판단
2. \`daily.today_notes\` 또는 어제 노트 있으면 \`danbi_read\` 로 읽고 **한 줄 요약**
3. 읽은 요약 + 진행 중이던 TODO가 있으면 **그것부터 이어서 제안**

### 질문 응답 중

| 사용자가 이런 뉘앙스로 물으면 | 이 도구를 먼저 |
|---|---|
| "예전에 이거 어떻게 정했지?" | \`danbi_search\` → \`danbi_read\` |
| "지난주/어제 뭐 했지?" | \`danbi_briefing\` activity |
| "1년 전 오늘" / "비슷한 작업 한 적 있나" | \`danbi_daily\` |
| "X 관련 기존 노트" | \`danbi_search\` 쿼리 |
| "다른 프로젝트에선 어떻게?" | \`danbi_list_projects\` → 해당 프로젝트 \`danbi_read\` |

**금지**: 일반론으로 답하기. 검색 안 돌리고 추측하기. "아마도" "일반적으로"로 시작하기.

## Write — 언제 무엇을 기록하는가

${"`danbi_log`"}를 호출하는 **트리거**:

| 트리거 | 기록 예시 |
|---|---|
| 기술 결정 확정 | "JWT refresh 7일로 정함. 이유: 모바일 세션 유지 vs 보안 트레이드오프" |
| 버그 원인 발견 | "CORS 에러는 /api/auth preflight 미설정. next.config.js headers 에 추가" |
| 세션 종료 전 TODO | "내일: RLS 정책 테이블 4개 더 마이그레이션, 스키마 dump 비교" |
| 노하우 (되풀이하면 좋은 것) | "Supabase RLS는 service_role 키 쓰면 자동 우회 — 마이그레이션 스크립트에서 유용" |
| 재발 방지 (또 겪지 말아야 할 것) | "Tauri v2 webview에서 window.prompt 작동 안 함 → 인라인 input 대신" |
| 아키텍처 전환 | "REST → tRPC 마이그레이션 시작. 이유: 타입 안전성 + codegen 불필요" |

**기록 형식**:
- 짧은 markdown 섹션. \`###\` 헤더 + 2-4줄 본문.
- 코드는 핵심만 5줄 이내.
- 잡담·회고·감상 금지 — "결정 / 원인 / TODO / 노하우 / 재발 방지" 중 하나로 분류.

특정 도메인 파일(예: \`notes/auth.md\`)을 갱신해야 하면 ${"`danbi_append`"}.

### 기록 타이밍

- **즉시**: 위 트리거에 해당하는 순간이 오면 다른 일 끝나기 전에 바로 호출
- **세션 종료 시**: 대화가 자연스럽게 끝날 때 누락된 트리거가 있으면 몰아서
- **이중 기록 방지**: 같은 세션에서 동일 내용 중복 호출 금지 (체크 후 추가)

## Link — 언제 [[wiki-link]]를 넣는가

단비는 문서 간 연결이 쌓일수록 vault가 똑똑해진다. 그래프 뷰에서 확정 링크(실선)와 ghost 제안(점선)을 구분 렌더한다.

### 위키링크 문법

- 같은 프로젝트 내: \`[[파일명.md]]\`
- 다른 프로젝트: \`[[프로젝트명/파일명.md]]\`

### 언제 링크를 삽입하는가

${"`danbi_log`"}·${"`danbi_append`"} 로 기록할 때 다음 중 하나라도 해당되면 본문에 \`[[...]]\` 자연스럽게 포함:

1. 언급하는 개념이 기존 문서에 이미 다뤄졌음 (\`danbi_search\` 결과로 확인)
2. 같은 프로젝트 내 관련 결정/토픽이 있음
3. 답변 근거로 \`danbi_read\` 한 문서를 인용함

예시:
\`\`\`markdown
### JWT refresh 7일로 확정

세션 만료 UX 개선 위해 기존 1일 → 7일. 보안 이슈는 [[notes/auth-security.md]]
에 정리된 리스크 매트릭스 기준으로 수용 가능 범위로 판단. 관련 토큰 만료
처리는 [[notes/token-refresh-flow.md]] 참조.
\`\`\`

### Ghost 제안 처리

\`danbi_briefing\` 응답의 \`ghost_suggestions\` 는 단비가 제안한 "아직 확정 안 된 링크"다. 작업 중 해당 source 문서를 편집하게 되면, 제안된 target이 정말 관련 있는지 판단하고 ${"`[[target]]`"}을 본문에 삽입. 확정되면 그래프의 점선이 실선으로 바뀐다.

## 안티 패턴

- **❌ \`danbi_log\` 누락**: "이건 단비에 기록하기엔 너무 작은데..." — 작은 결정일수록 나중에 찾기 어렵다. 기록한다.
- **❌ 일반 지식으로 답**: "일반적으로 JWT는..." — 이 vault가 어떤 결정을 내렸는지가 중요하다. 먼저 검색.
- **❌ 링크 없이 기록**: 관련 문서가 있는데 \`[[링크]]\` 안 걺 — 그래프가 자라지 않음.
- **❌ project 파라미터 수동 지정**: 전용 엔드포인트는 프로젝트로 clamp됨. 생략이 맞다.
<!-- danbi:end -->
`;

  return (
    <>
      <SectionTitle
        title="MCP 서버"
        hint="Claude Code / Cursor 같은 외부 AI 에이전트가 단비 vault를 읽고 쓸 수 있도록 로컬 MCP 엔드포인트를 노출합니다."
      />

      <Row
        label="상태"
        hint={
          status?.running
            ? "127.0.0.1 에서 실행 중 — 외부 네트워크에서는 접근 불가."
            : "꺼져 있습니다. 활성화하면 포트 하나를 열고 토큰 기반 인증을 사용합니다."
        }
      >
        <div className="flex items-center gap-2">
          <span
            className={cn(
              "inline-flex items-center gap-1 rounded-xs px-1.5 py-0.5 text-[11px] uppercase tracking-[0.4px]",
              status?.running
                ? "bg-accent-green-soft text-accent-green"
                : "bg-surface-elevated text-mute",
            )}
          >
            <span
              className={cn(
                "h-1.5 w-1.5 rounded-full",
                status?.running ? "bg-accent-green" : "bg-stone",
              )}
            />
            {status?.running ? "실행 중" : "꺼짐"}
          </span>
          <button
            onClick={toggle}
            disabled={busy || !status}
            className={cn(
              "inline-flex h-7 items-center rounded-sm px-2 text-[12px] font-medium transition-colors",
              status?.enabled
                ? "bg-surface-elevated text-body hover:text-on-dark"
                : "bg-primary text-on-primary hover:bg-primary-pressed",
              busy && "opacity-50",
            )}
          >
            {status?.enabled ? "끄기" : "켜기"}
          </button>
        </div>
      </Row>

      {status?.enabled && (
        <>
          <Row label="URL">
            <div className="flex items-center gap-2">
              <code className="font-mono text-[11px] text-on-dark-mute">
                {status.url}
              </code>
              <button
                onClick={() => copy("url", status.url)}
                className="grid h-6 w-6 place-items-center rounded-sm text-stone hover:text-on-dark"
                title="복사"
              >
                <Copy size={11} />
              </button>
            </div>
          </Row>
          <Row
            label="토큰"
            hint="외부 AI에서 Authorization 헤더로 사용하세요. 유출 시 새로 회전하세요."
          >
            <div className="flex items-center gap-2">
              <code className="max-w-[240px] truncate font-mono text-[11px] text-on-dark-mute">
                {status.token}
              </code>
              <button
                onClick={() => copy("token", status.token)}
                className="grid h-6 w-6 place-items-center rounded-sm text-stone hover:text-on-dark"
                title="복사"
              >
                <Copy size={11} />
              </button>
              <button
                onClick={rotate}
                disabled={busy}
                className="grid h-6 w-6 place-items-center rounded-sm text-stone hover:text-on-dark"
                title="새 토큰 생성"
              >
                <RefreshCw size={11} />
              </button>
            </div>
          </Row>

          <div className="mt-4">
            <div className="mb-1 flex items-center justify-between">
              <span className="text-caption-sm uppercase tracking-[0.4px] text-mute">
                Claude Code · 한 줄 설치
              </span>
              <button
                onClick={() => copy("cli", cliCommand)}
                className="inline-flex h-6 items-center gap-1 rounded-sm border border-accent-blue bg-accent-blue-soft px-2 text-[11px] font-medium text-accent-blue"
              >
                <Copy size={10} /> 명령 복사
              </button>
            </div>
            <pre className="rounded-md border border-hairline bg-surface-elevated p-3 font-mono text-[11px] leading-[1.6] text-body whitespace-pre-wrap break-all">
              {cliCommand}
            </pre>
            <div className="mt-2 text-caption-sm leading-[1.6] text-mute">
              복사해서 터미널에 붙여넣고 실행하세요. 그 뒤 Claude Code
              세션에서 <code>/mcp</code> 로 <code>danbi</code> 서버가 뜨면
              연결 완료.
            </div>
          </div>

          <div className="mt-4">
            <div className="mb-1 flex items-center justify-between">
              <span className="text-caption-sm uppercase tracking-[0.4px] text-mute">
                JSON 스니펫 (Cursor · Zed · 수동 편집용)
              </span>
              <button
                onClick={() => copy("snippet", snippet)}
                className="inline-flex h-6 items-center gap-1 rounded-sm border border-hairline bg-surface-elevated px-2 text-[11px] text-body hover:text-on-dark"
              >
                <Copy size={10} /> 복사
              </button>
            </div>
            <pre className="rounded-md border border-hairline bg-surface-elevated p-3 font-mono text-[11px] leading-[1.6] text-body whitespace-pre-wrap">
              {snippet}
            </pre>
            <div className="mt-2 text-caption-sm leading-[1.6] text-mute">
              Cursor나 직접 편집하려면 이 JSON을 해당 에이전트의 MCP 설정에
              붙여넣으세요.
            </div>
          </div>

          <div className="mt-4">
            <div className="mb-1 flex items-center justify-between">
              <span className="text-caption-sm uppercase tracking-[0.4px] text-mute">
                CLAUDE.md 단비 블록 (마커로 관리 · 재복사 시 교체)
              </span>
              <button
                onClick={() => copy("claudemd", claudeMdTemplate)}
                className="inline-flex h-6 items-center gap-1 rounded-sm border border-accent-blue bg-accent-blue-soft px-2 text-[11px] font-medium text-accent-blue"
              >
                <Copy size={10} /> 블록 복사
              </button>
            </div>
            <pre className="max-h-[200px] overflow-auto rounded-md border border-hairline bg-surface-elevated p-3 font-mono text-[11px] leading-[1.6] text-body whitespace-pre-wrap">
              {claudeMdTemplate}
            </pre>
            <div className="mt-2 text-caption-sm leading-[1.6] text-mute">
              Claude Code 프로젝트 루트의 <code>CLAUDE.md</code> 에 이 블록을
              통째로 붙여넣으세요. 블록 양 끝의{" "}
              <code>&lt;!-- danbi:start --&gt;</code> /{" "}
              <code>&lt;!-- danbi:end --&gt;</code> 마커는 건드리지 마세요 —
              다음에 재복사할 때 Claude Code 가 마커 사이만 자동으로 교체해
              사용자 섹션을 보존합니다.{" "}
              <span className="text-stone">
                &lt;이 프로젝트 이름&gt; 두 자리만 실제 이름으로 바꿔서 쓰세요.
              </span>
            </div>
          </div>
        </>
      )}

      {err && (
        <div className="mt-3 rounded-md border border-hairline bg-surface-elevated p-2 font-mono text-[11px] text-accent-red">
          {err}
        </div>
      )}
      {copied && (
        <div className="mt-2 text-caption-sm text-accent-green">
          {copied} 복사됨
        </div>
      )}
    </>
  );
}

function BackupPanel({ cfg }: { cfg: DanbiConfig }) {
  const [path, setPath] = useState<string>(cfg.backup.path ?? "");
  const [running, setRunning] = useState(false);
  const [result, setResult] = useState<string | null>(
    cfg.backup.last_message ?? null,
  );
  const [err, setErr] = useState<string | null>(null);
  const [validating, setValidating] = useState(false);
  const dirty = path.trim() !== (cfg.backup.path ?? "");

  async function saveEnabled(v: boolean) {
    await saveCfg(
      patchCfg(cfg, {
        backup: { ...cfg.backup, enabled: v },
      }),
    );
  }

  async function savePath() {
    const trimmed = path.trim();
    setErr(null);
    setValidating(true);
    try {
      if (trimmed) await ipc.backupValidatePath(trimmed);
      await saveCfg(
        patchCfg(cfg, {
          backup: { ...cfg.backup, path: trimmed || null },
        }),
      );
    } catch (e) {
      setErr(String(e));
    } finally {
      setValidating(false);
    }
  }

  async function runNow() {
    setRunning(true);
    setErr(null);
    try {
      const r = await ipc.backupNow();
      setResult(
        `${r.copied} 복사 · ${r.skipped} 스킵 · ${r.removed} 삭제 · ${r.duration_ms}ms`,
      );
    } catch (e) {
      setErr(String(e));
    } finally {
      setRunning(false);
    }
  }

  const lastAt = cfg.backup.last_run_at
    ? timeAgo(cfg.backup.last_run_at * 1000)
    : null;

  return (
    <>
      <SectionTitle
        title="백업"
        hint="vault 를 Dropbox · iCloud · OneDrive 같은 외부 폴더로 자동 미러링합니다. 단방향 (vault → 백업 폴더) 이며, 백업 폴더에서 수정한 내용은 vault 로 돌아오지 않아요."
      />
      <Row
        label="자동 백업"
        hint="파일이 수정될 때마다 debounce 후 자동으로 백업 폴더에 동기화합니다. 앱이 켜져 있는 동안만 동작해요."
      >
        <Toggle
          value={cfg.backup.enabled}
          onChange={saveEnabled}
        />
      </Row>
      <Row
        label="백업 폴더"
        hint="절대 경로. vault 안쪽 경로는 거절됩니다. Dropbox 예: ~/Dropbox/Danbi_Backup, iCloud 예: ~/Library/Mobile Documents/com~apple~CloudDocs/Danbi_Backup"
        stack
      >
        <div className="flex w-full flex-wrap items-center gap-2">
          <input
            type="text"
            value={path}
            onChange={(e) => setPath(e.target.value)}
            placeholder="/Users/name/Dropbox/Danbi_Backup"
            className="h-7 min-w-[220px] flex-1 rounded-sm border border-hairline bg-surface-elevated px-2 font-mono text-[11px] text-ink outline-none placeholder:text-stone"
          />
          <button
            onClick={async () => {
              try {
                const picked = await openDialog({
                  directory: true,
                  multiple: false,
                  title: "백업 폴더 선택",
                });
                if (typeof picked === "string") {
                  setPath(picked);
                }
              } catch (e) {
                setErr(String(e));
              }
            }}
            className="inline-flex h-7 shrink-0 items-center gap-1 rounded-sm border border-hairline bg-surface-elevated px-2 text-[12px] text-body hover:text-on-dark"
          >
            <FolderOpen size={11} /> 폴더 찾기
          </button>
          <button
            disabled={!dirty || validating}
            onClick={savePath}
            className={cn(
              "inline-flex h-7 shrink-0 items-center rounded-sm px-2 text-[12px] font-medium transition-colors",
              dirty && !validating
                ? "bg-primary text-on-primary hover:bg-primary-pressed"
                : "bg-surface-elevated text-ash",
            )}
          >
            {validating ? "확인 중…" : "저장"}
          </button>
        </div>
      </Row>
      <Row
        label="지금 백업"
        hint="자동 백업을 기다리지 않고 즉시 한 번 실행합니다."
      >
        <button
          onClick={runNow}
          disabled={running || !cfg.backup.path}
          className={cn(
            "inline-flex h-7 items-center gap-1 rounded-sm border border-hairline bg-surface-elevated px-2 text-[12px] text-body hover:text-on-dark",
            (running || !cfg.backup.path) && "opacity-50",
          )}
        >
          <RefreshCw size={11} className={running ? "animate-spin" : ""} />
          {running ? "실행 중…" : "백업 실행"}
        </button>
      </Row>
      <Row
        label="마지막 실행"
        hint="마지막 백업 시각과 결과 요약."
      >
        <div className="flex flex-col items-end gap-0.5 text-right">
          <code className="rounded-sm border border-hairline bg-surface-elevated px-2 py-1 font-mono text-[11px] text-on-dark-mute">
            {lastAt ?? "아직 없음"}
          </code>
          {result && (
            <span className="text-caption-sm text-mute">{result}</span>
          )}
        </div>
      </Row>

      {err && (
        <div className="mt-3 rounded-md border border-hairline bg-surface-elevated p-2 font-mono text-[11px] text-accent-red">
          {err}
        </div>
      )}

      <div className="mt-5 rounded-md border border-hairline bg-surface-elevated p-3 text-caption-sm leading-[1.6] text-mute">
        <div className="mb-1 text-body">주의</div>
        <ul className="list-disc space-y-0.5 pl-4">
          <li>
            <code className="text-on-dark">.git/</code>,{" "}
            <code className="text-on-dark">.danbi/</code> 는 기본 제외돼요 —
            백업 폴더에는 깨끗한 마크다운만 들어갑니다.
          </li>
          <li>
            백업 폴더에서 직접 수정한 내용은 vault 로 돌아오지 않아요. sync 가
            아니라 일방향 backup 입니다.
          </li>
          <li>
            Dropbox · iCloud 가 업로드를 완료해야 실제로 백업된 상태예요.
            네트워크가 끊긴 채로는 로컬 mirror 까지만 보장돼요.
          </li>
          <li>
            다른 기기에서 같이 쓰고 싶다면 백업 대신 git remote 방식이 더
            안전해요 (향후 지원 예정).
          </li>
        </ul>
      </div>
    </>
  );
}

function timeAgo(ms: number): string {
  const diff = Date.now() - ms;
  if (diff < 60_000) return "방금";
  if (diff < 3_600_000) return `${Math.floor(diff / 60_000)}분 전`;
  if (diff < 86_400_000) return `${Math.floor(diff / 3_600_000)}시간 전`;
  return `${Math.floor(diff / 86_400_000)}일 전`;
}

function VectorPanel() {
  const cfg = useApp((s) => s.cfg);
  const setCfg = useApp((s) => s.setCfg);
  const [stats, setStats] = useState<{
    count: number;
    oldest: number | null;
    newest: number | null;
    model: string | null;
  } | null>(null);
  const [estimate, setEstimate] =
    useState<import("@/lib/ipc").VectorEstimateResponse | null>(null);
  const [estimating, setEstimating] = useState(false);
  const [running, setRunning] = useState(false);
  const [progress, setProgress] = useState<
    import("@/lib/ipc").ReindexProgress | null
  >(null);
  const [result, setResult] = useState<string | null>(null);
  const [err, setErr] = useState<string | null>(null);
  const [ollamaUrl, setOllamaUrl] = useState("http://localhost:11434");
  const [ollamaModel, setOllamaModel] = useState("nomic-embed-text");
  const [voyageKey, setVoyageKey] = useState("");
  const [voyageModel, setVoyageModel] = useState("voyage-multilingual-2");
  const [geminiModel, setGeminiModel] = useState("gemini-embedding-001");

  async function refresh() {
    try {
      setStats(await ipc.vectorStats());
    } catch {
      /* ignore */
    }
  }
  async function refreshEstimate() {
    setEstimating(true);
    try {
      const e = await ipc.vectorEstimateReindex();
      setEstimate(e);
    } catch {
      setEstimate(null);
    } finally {
      setEstimating(false);
    }
  }
  useEffect(() => {
    refresh();
    refreshEstimate();
  }, []);

  // Hydrate editor state from the currently-saved embed provider so the
  // user sees what's live instead of the defaults.
  useEffect(() => {
    if (cfg?.embed_provider?.kind === "ollama") {
      setOllamaUrl(cfg.embed_provider.base_url ?? "http://localhost:11434");
      if (cfg.embed_model) setOllamaModel(cfg.embed_model);
    } else if (cfg?.embed_provider?.kind === "voyage") {
      if (cfg.embed_model) setVoyageModel(cfg.embed_model);
    } else if (cfg?.embed_provider?.kind === "google") {
      if (cfg.embed_model) setGeminiModel(cfg.embed_model);
    }
  }, [cfg?.embed_provider, cfg?.embed_model]);

  type EmbedMode = "same" | "ollama" | "voyage" | "gemini";
  const embedMode: EmbedMode =
    cfg?.embed_provider?.kind === "ollama"
      ? "ollama"
      : cfg?.embed_provider?.kind === "voyage"
        ? "voyage"
        : cfg?.embed_provider?.kind === "google"
          ? "gemini"
          : "same";

  async function applyEmbedMode(mode: EmbedMode, opts?: { voyageKey?: string }) {
    if (!cfg?.vault_path) return;
    let embed_provider: DanbiConfig["embed_provider"] = null;
    let embed_model: string | null = null;
    if (mode === "ollama") {
      embed_provider = {
        kind: "ollama",
        base_url: ollamaUrl.trim() || "http://localhost:11434",
      };
      embed_model = ollamaModel.trim() || "nomic-embed-text";
    } else if (mode === "voyage") {
      if (opts?.voyageKey) {
        await ipc.storeVoyageApiKey(opts.voyageKey);
      }
      embed_provider = {
        kind: "voyage",
        api_key_ref: "keychain:danbi-voyage",
      };
      embed_model = voyageModel.trim() || "voyage-multilingual-2";
    } else if (mode === "gemini") {
      embed_provider = {
        kind: "google",
        api_key_ref: "keychain:danbi-google",
      };
      embed_model = geminiModel.trim() || "gemini-embedding-001";
    }
    const next: DanbiConfig = {
      ...cfg,
      embed_provider,
      embed_model,
    };
    await ipc.saveConfig(cfg.vault_path, next);
    setCfg(next);
    setResult(
      mode === "same"
        ? "LLM provider 와 동일한 임베딩을 쓰도록 되돌렸어요."
        : `${mode} 임베딩으로 전환했어요. 기존 인덱스의 dimension 이 다르면 재인덱싱이 필요합니다.`,
    );
    await refreshEstimate();
  }

  // backend 가 emit 하는 진행 이벤트 구독. running 상태일 때만 활성.
  useEffect(() => {
    let unsubProgress: (() => void) | null = null;
    let unsubDone: (() => void) | null = null;
    (async () => {
      const { onReindexProgress, onReindexDone } = await import(
        "@/lib/ipc"
      );
      unsubProgress = await onReindexProgress((p) => setProgress(p));
      unsubDone = await onReindexDone(() => setProgress(null));
    })();
    return () => {
      unsubProgress?.();
      unsubDone?.();
    };
  }, []);

  async function reindex() {
    setRunning(true);
    setErr(null);
    setResult(null);
    setProgress(null);
    try {
      const r = await ipc.vectorReindex();
      const detail =
        r.embedded === 0 && r.total > 0
          ? `총 ${r.total}개 · 모두 캐시 적중 (변경된 파일 없음). dimension/모델이 바뀌지 않았다면 정상이에요.`
          : `총 ${r.total}개 · ${r.embedded} 새로 embedding · ${r.skipped} 스킵 · ${r.removed} 삭제`;
      setResult(detail);
      console.log("[danbi] reindex result", r);
      await refresh();
      await refreshEstimate();
    } catch (e) {
      console.error("[danbi] reindex failed", e);
      setErr(String(e));
    } finally {
      setRunning(false);
      setProgress(null);
    }
  }

  async function clear() {
    try {
      await ipc.vectorClear();
      setResult("인덱스를 비웠어요.");
      await refresh();
    } catch (e) {
      setErr(String(e));
    }
  }

  // 현재 활성화된 임베딩 모델을 한 줄로 요약. 모델 이름과 provider 종류만
  // 섞어서 보여준다. config 의 embed_model 이 비어있으면 stats 의 모델로
  // fallback (재인덱싱 후 자동 채워짐).
  const activeEmbedKind = cfg?.embed_provider?.kind ?? null;
  const activeEmbedModel =
    cfg?.embed_model ?? stats?.model ?? "(설정 안 됨)";

  // 단순 UI 로 가야 하는지 판정.
  // - preset === "claude_code" 면 명시적으로 단순화
  // - preset 이 비어있어도 LLM provider 가 없으면 단순 UI 로 (legacy
  //   vault 가 preset:null 인 경우가 흔함)
  // 이렇게 하면 사용자가 vault 를 새 화면에 들고 와도 옛 UI 가 다시
  // 튀어나오지 않음.
  const useSimpleUi =
    cfg?.preset === "claude_code" ||
    (cfg?.preset == null && cfg?.provider == null);

  return (
    <>
      <SectionTitle
        title="AI 연동"
        hint="임베딩 provider 를 연결하면 의미 검색 (RRF 하이브리드) · daily 노트 요약 · purpose 자동 작성 · 관련 노트 제안이 활성화됩니다. 같은 키가 LLM 자동화에도 재사용돼요."
      />

      {cfg && (
        <AiConnectCard
          cfg={cfg}
          onSaved={async (next) => {
            setCfg(next);
            await refresh();
          }}
        />
      )}

      {/* "현재 모델 + 재설정 + 재인덱싱" 한 카드. 사용자 요청:
       *  벡터/LLM 정보가 너무 많아서 핵심 3개만 남기기. */}
      <div className="mb-4 rounded-lg border border-hairline bg-surface-elevated p-4">
        <div className="flex items-center justify-between gap-4">
          <div className="min-w-0 flex-1">
            <div className="text-[12px] uppercase tracking-[0.4px] text-mute">
              현재 임베딩 모델
            </div>
            <div className="mt-1 flex items-center gap-2">
              <code className="font-mono text-[13px] text-ink">
                {activeEmbedModel}
              </code>
              {activeEmbedKind && (
                <span className="rounded-xs bg-surface-card px-1.5 py-0.5 text-[10px] uppercase tracking-[0.4px] text-on-dark-mute">
                  {activeEmbedKind}
                </span>
              )}
            </div>
            <div className="mt-1 text-[11px] text-stone">
              {stats?.count ?? 0} 문서 인덱스됨
            </div>
          </div>
          <div className="flex shrink-0 items-center gap-2">
            <button
              onClick={() => {
                // 온보딩으로 점프 — App.tsx 의 force-onboarding 플래그
                // 사용. 사용자가 임베딩 키/모델을 다시 고를 수 있다.
                try {
                  localStorage.setItem("danbi.forceOnboarding", "1");
                } catch {
                  /* no-op */
                }
                window.location.reload();
              }}
              className="inline-flex h-9 items-center gap-1.5 rounded-md border border-hairline bg-surface px-3 text-[12px] text-body hover:border-hairline-strong hover:text-on-dark"
            >
              임베딩 재설정
            </button>
            <button
              onClick={reindex}
              disabled={running}
              className={cn(
                "inline-flex h-9 shrink-0 items-center gap-2 rounded-md bg-primary px-4 text-[13px] font-semibold text-on-primary transition-colors hover:bg-primary-pressed disabled:bg-surface disabled:text-ash",
              )}
            >
              <RefreshCw size={13} className={running ? "animate-spin" : ""} />
              {running ? "실행 중…" : "지금 재인덱싱"}
            </button>
          </div>
        </div>
      </div>

      {(running || progress) && (
        <ReindexProgressCard
          progress={progress}
          providerKind={activeEmbedKind}
        />
      )}

      {result && (
        <div className="mb-4 rounded-md border border-hairline bg-surface-elevated p-3 text-[12px] leading-[1.6] text-body">
          <div className="mb-1 inline-flex items-center gap-1.5 text-[11px] font-semibold uppercase tracking-[0.5px] text-accent-green">
            <span className="h-1.5 w-1.5 rounded-full bg-accent-green" />
            결과
          </div>
          <div className="font-mono text-on-dark-mute">{result}</div>
        </div>
      )}
      {err && (
        <div className="mb-4 rounded-md border border-accent-red/40 bg-accent-red-soft p-3">
          <div className="mb-1 inline-flex items-center gap-1.5 text-[11px] font-semibold uppercase tracking-[0.5px] text-accent-red">
            <span className="h-1.5 w-1.5 rounded-full bg-accent-red" />
            재인덱싱 실패
          </div>
          {err.includes("voyage") && err.includes("429") ? (
            <div className="space-y-2 text-[12px] leading-[1.7] text-on-dark-mute">
              <div className="font-medium text-ink">
                Voyage AI 무료 티어 rate limit 에 걸렸어요
              </div>
              <div>
                결제수단을 등록하지 않은 무료 계정은 <strong>분당 3회 호출 / 분당 10K 토큰</strong>으로
                제한됩니다. 단비가 한 번 자동 재시도했지만 여전히 막혔어요.
              </div>
              <div>
                <span className="text-ink">해결 방법:</span> Voyage dashboard 에서 결제수단을
                등록하면 표준 rate limit 가 풀려요.{" "}
                <strong className="text-on-dark">실제 과금되지 않습니다</strong> —
                무료 200M 토큰은 그대로 유지됩니다.
              </div>
              <button
                type="button"
                onClick={async () => {
                  try {
                    const { openUrl } = await import(
                      "@tauri-apps/plugin-opener"
                    );
                    await openUrl(
                      "https://dashboard.voyageai.com/organization/billing",
                    );
                  } catch {
                    /* no-op */
                  }
                }}
                className="inline-flex h-7 items-center gap-1 rounded-sm bg-accent-red px-2 text-[11px] font-semibold text-on-dark hover:opacity-90"
              >
                Voyage Billing 페이지 열기 ↗
              </button>
              <details className="mt-2">
                <summary className="cursor-pointer text-[11px] text-mute hover:text-on-dark-mute">
                  원본 에러 보기
                </summary>
                <pre className="mt-1 whitespace-pre-wrap font-mono text-[11px] leading-[1.6] text-mute">
                  {err}
                </pre>
              </details>
            </div>
          ) : err.includes("gemini") && err.includes("429") ? (
            <div className="space-y-2 text-[12px] leading-[1.7] text-on-dark-mute">
              <div className="font-medium text-ink">
                Gemini 무료 티어 quota 한도에 걸렸어요
              </div>
              <div>
                <code className="text-on-dark">gemini-embedding-001</code> 무료
                티어는 <strong>분당 5회 / 일 100회 호출</strong> 입니다. 단비가
                자동으로 12초씩 대기하며 재시도하지만, 일일 한도가 다 차면
                내일까지 기다려야 해요.
              </div>
              <div>
                <span className="text-ink">대안</span> · 작은 단위로 나눠서
                프로젝트별 재인덱싱 (각 프로젝트 홈 헤더의 버튼) · AI Studio 에서
                결제수단 등록 시 분당 100회로 풀림 (실제 과금 거의 없음)
              </div>
              <button
                type="button"
                onClick={async () => {
                  try {
                    const { openUrl } = await import(
                      "@tauri-apps/plugin-opener"
                    );
                    await openUrl(
                      "https://aistudio.google.com/app/apikey",
                    );
                  } catch {
                    /* no-op */
                  }
                }}
                className="inline-flex h-7 items-center gap-1 rounded-sm bg-accent-red px-2 text-[11px] font-semibold text-on-dark hover:opacity-90"
              >
                AI Studio · API Keys ↗
              </button>
              <details className="mt-2">
                <summary className="cursor-pointer text-[11px] text-mute hover:text-on-dark-mute">
                  원본 에러 보기
                </summary>
                <pre className="mt-1 whitespace-pre-wrap font-mono text-[11px] leading-[1.6] text-mute">
                  {err}
                </pre>
              </details>
            </div>
          ) : (
            <div className="whitespace-pre-wrap font-mono text-[12px] leading-[1.6] text-accent-red">
              {err}
            </div>
          )}
        </div>
      )}

      {/* 사용자 피드백: "LLM 관련 인덱스 같은거 다 안보이게 / 임베딩
          모델 + 현재 설정 + 재설정 버튼만 남기자". 아래 Provider 선택
          (Segmented) / 인덱스 상태 / 예상 비용 / 인덱스 지우기 / 주의
          블록은 모두 isClaudeCode === false 일 때만 노출. */}
      {!useSimpleUi && (
      <Row
        stack
        label="임베딩 Provider"
        hint="LLM 호출과 임베딩을 분리할 수 있어요. Ollama 로 보내면 로컬 머신에서 무료·무제한으로 인덱싱됩니다 — 유료 LLM 쓰면서도 재인덱싱 비용 걱정 없이 쓰는 패턴이에요."
      >
        <div className="flex flex-col gap-3">
          <Segmented
            value={embedMode}
            options={[
              { value: "same", label: "LLM 과 동일" },
              { value: "ollama", label: "Ollama (로컬·무료)" },
              { value: "voyage", label: "Voyage AI (월 200M 토큰 무료)" },
              { value: "gemini", label: "Google Gemini (일 1,500 요청 무료)" },
            ]}
            onChange={(v) => applyEmbedMode(v as EmbedMode)}
          />
          {embedMode === "ollama" && (
            <div className="flex flex-col gap-2 rounded-md border border-hairline bg-surface-elevated p-3">
              <div className="text-caption-sm text-mute">
                먼저 터미널에서{" "}
                <code className="text-on-dark">
                  ollama pull {ollamaModel || "nomic-embed-text"}
                </code>{" "}
                로 모델을 받아두세요. (≈ 270MB · `nomic-embed-text` 기준)
              </div>
              <div className="flex flex-wrap items-center gap-2">
                <label className="flex flex-1 min-w-[240px] flex-col gap-1">
                  <span className="text-[11px] uppercase tracking-[0.4px] text-mute">
                    Base URL
                  </span>
                  <input
                    value={ollamaUrl}
                    onChange={(e) => setOllamaUrl(e.target.value)}
                    placeholder="http://localhost:11434"
                    className="h-8 rounded-sm border border-hairline bg-surface px-2 font-mono text-[12px] text-ink outline-none focus:border-hairline-strong"
                  />
                </label>
                <label className="flex flex-1 min-w-[180px] flex-col gap-1">
                  <span className="text-[11px] uppercase tracking-[0.4px] text-mute">
                    모델
                  </span>
                  <input
                    value={ollamaModel}
                    onChange={(e) => setOllamaModel(e.target.value)}
                    placeholder="nomic-embed-text"
                    className="h-8 rounded-sm border border-hairline bg-surface px-2 font-mono text-[12px] text-ink outline-none focus:border-hairline-strong"
                  />
                </label>
                <button
                  onClick={() => applyEmbedMode("ollama")}
                  className="mt-[18px] inline-flex h-8 items-center rounded-sm bg-primary px-3 text-[12px] font-medium text-on-primary hover:bg-primary-pressed"
                >
                  저장
                </button>
              </div>
            </div>
          )}
          {embedMode === "voyage" && (
            <div className="flex flex-col gap-2 rounded-md border border-hairline bg-surface-elevated p-3">
              {/* If voyage is already the active embed provider, the
                  key is already in Keychain. Show that as a stable
                  status row instead of a perpetually-empty input. */}
              {cfg?.embed_provider?.kind === "voyage" && !voyageKey && (
                <div className="flex items-center justify-between gap-2 text-[12px]">
                  <span className="inline-flex items-center gap-1.5 text-accent-green">
                    <span className="h-1.5 w-1.5 rounded-full bg-accent-green" />
                    API 키가 Keychain 에 저장돼 있어요
                    <code className="font-mono text-stone">
                      keychain:danbi-voyage
                    </code>
                  </span>
                  <button
                    onClick={() => setVoyageKey(" ")} // any non-empty triggers the input form below
                    className="inline-flex h-7 items-center rounded-sm border border-hairline bg-surface px-2 text-[11px] text-body hover:text-on-dark"
                  >
                    다시 입력
                  </button>
                </div>
              )}
              <div className="text-caption-sm text-mute">
                voyageai.com 에서 키 발급 (Google/GitHub OAuth, 3분). 무료 티어 월 200M 토큰 · 한국어 품질 권장.
              </div>
              {(cfg?.embed_provider?.kind !== "voyage" ||
                voyageKey !== "") && (
                <div className="flex flex-wrap items-center gap-2">
                  <label className="flex flex-1 min-w-[260px] flex-col gap-1">
                    <span className="text-[11px] uppercase tracking-[0.4px] text-mute">
                      API Key
                    </span>
                    <input
                      type="password"
                      value={voyageKey === " " ? "" : voyageKey}
                      onChange={(e) => setVoyageKey(e.target.value)}
                      placeholder="pa-…"
                      className="h-8 rounded-sm border border-hairline bg-surface px-2 font-mono text-[12px] text-ink outline-none focus:border-hairline-strong"
                    />
                  </label>
                  <label className="flex flex-1 min-w-[200px] flex-col gap-1">
                    <span className="text-[11px] uppercase tracking-[0.4px] text-mute">
                      모델
                    </span>
                    <input
                      value={voyageModel}
                      onChange={(e) => setVoyageModel(e.target.value)}
                      placeholder="voyage-multilingual-2"
                      className="h-8 rounded-sm border border-hairline bg-surface px-2 font-mono text-[12px] text-ink outline-none focus:border-hairline-strong"
                    />
                  </label>
                  <button
                    disabled={!voyageKey.trim()}
                    onClick={async () => {
                      await applyEmbedMode("voyage", {
                        voyageKey: voyageKey.trim(),
                      });
                      setVoyageKey("");
                    }}
                    className="mt-[18px] inline-flex h-8 items-center rounded-sm bg-primary px-3 text-[12px] font-medium text-on-primary hover:bg-primary-pressed disabled:bg-surface disabled:text-ash"
                  >
                    저장
                  </button>
                </div>
              )}
            </div>
          )}
          {embedMode === "gemini" && (
            <div className="flex flex-col gap-2 rounded-md border border-hairline bg-surface-elevated p-3">
              <div className="text-caption-sm text-mute">
                Google AI Studio (aistudio.google.com) 에서 API 키 발급. 무료 티어 일 1,500 요청. 한국어 품질 좋음. **메인 LLM 을 Google 로 설정한 뒤에** 이 옵션을 써주세요 — 같은 키를 재사용합니다.
              </div>
              <div className="flex flex-wrap items-center gap-2">
                <label className="flex flex-1 min-w-[200px] flex-col gap-1">
                  <span className="text-[11px] uppercase tracking-[0.4px] text-mute">
                    모델
                  </span>
                  <input
                    value={geminiModel}
                    onChange={(e) => setGeminiModel(e.target.value)}
                    placeholder="gemini-embedding-001"
                    className="h-8 rounded-sm border border-hairline bg-surface px-2 font-mono text-[12px] text-ink outline-none focus:border-hairline-strong"
                  />
                </label>
                <button
                  onClick={() => applyEmbedMode("gemini")}
                  className="mt-[18px] inline-flex h-8 items-center rounded-sm bg-primary px-3 text-[12px] font-medium text-on-primary hover:bg-primary-pressed"
                >
                  저장
                </button>
              </div>
            </div>
          )}
        </div>
      </Row>
      )}

      {!useSimpleUi && (
        <>
          <Row
            label="인덱스 상태"
            hint="현재 저장된 embedding 개수와 마지막 업데이트 시각."
          >
            <div className="flex flex-col items-end gap-0.5 text-right">
              <code className="rounded-sm border border-hairline bg-surface-elevated px-2 py-1 font-mono text-[11px] text-on-dark-mute">
                {stats?.count ?? 0} 문서
              </code>
              {stats?.model && (
                <span className="text-caption-sm text-mute">
                  모델: {stats.model}
                </span>
              )}
            </div>
          </Row>
          {embedMode !== "voyage" && embedMode !== "ollama" && (
            <Row
              label="예상 비용"
              hint="지금 재인덱싱을 돌리면 드는 대략적인 비용 — vault 전체를 걸어보지 않고 글자수 기반으로 미리 계산합니다."
              stack
            >
              <EstimateBox
                estimate={estimate}
                estimating={estimating}
                onRefresh={refreshEstimate}
              />
            </Row>
          )}
          <Row
            label="인덱스 지우기"
            hint="모든 embedding 을 삭제합니다. 다음 검색이 일반 BM25로 돌아가요."
          >
            <button
              onClick={clear}
              disabled={running}
              className="inline-flex h-7 items-center gap-1 rounded-sm border border-hairline bg-surface-elevated px-2 text-[12px] text-body hover:text-on-dark"
            >
              <X size={11} /> 지우기
            </button>
          </Row>
          <div className="mt-5 rounded-md border border-hairline bg-surface-elevated p-3 text-caption-sm leading-[1.6] text-mute">
            <div className="mb-1 text-body">주의</div>
            <ul className="list-disc space-y-0.5 pl-4">
              <li>
                Embedding 은 현재 LLM provider 의 embedding 엔드포인트를 사용해요.
                Bedrock · OpenAI · NVIDIA · Ollama 지원, Anthropic · Google 은 미지원.
              </li>
              <li>
                재인덱싱은 문서 수 × embedding 호출 비용이 들어요. 첫 실행 전 예상
                비용을 확인해 주세요.
              </li>
              <li>
                벡터 스토어는 <code className="text-on-dark">.danbi/vectors.json</code>{" "}
                에 저장돼요. 같은 모델을 유지하면 다음 재인덱싱은 변경된 파일만
                처리해서 훨씬 빨라요.
              </li>
            </ul>
          </div>
        </>
      )}
    </>
  );
}

/** Compact pre-flight cost summary for an embedding reindex. Breaks the
 *  total file count into "캐시됨 / 변경됨" so users can see how much of
 *  the estimate is actually going to trigger paid calls. */
function EstimateBox({
  estimate,
  estimating,
  onRefresh,
}: {
  estimate: import("@/lib/ipc").VectorEstimateResponse | null;
  estimating: boolean;
  onRefresh: () => void;
}) {
  if (!estimate) {
    return (
      <div className="flex items-center gap-2 text-caption-sm text-stone">
        {estimating ? "계산 중…" : "예상 비용 정보가 없습니다."}
        <button
          onClick={onRefresh}
          disabled={estimating}
          className="inline-flex h-6 items-center gap-1 rounded-sm border border-hairline bg-surface-elevated px-2 text-[11px] text-body hover:text-on-dark"
        >
          <RefreshCw
            size={10}
            className={estimating ? "animate-spin" : ""}
          />
          다시 계산
        </button>
      </div>
    );
  }
  const { estimate: e, krw, krw_per_usd } = estimate;
  const hasPending = e.pending_files > 0;
  return (
    <div className="flex flex-col gap-2 rounded-md border border-hairline bg-surface-elevated p-3">
      <div className="flex items-baseline gap-1.5">
        <span className="text-stone">₩</span>
        <span className="text-[22px] font-medium leading-none text-ink">
          {Math.round(krw).toLocaleString("ko-KR")}
        </span>
        <span className="ml-2 text-caption-sm text-stone">
          예상 비용 · 1 USD = ₩{Math.round(krw_per_usd)}
        </span>
      </div>
      <div className="grid grid-cols-3 gap-2 text-caption-sm">
        <Fact
          label="전체"
          value={`${e.total_files.toLocaleString()}개`}
          hint="파일"
        />
        <Fact
          label="캐시됨"
          value={`${e.fresh_files.toLocaleString()}개`}
          hint="비용 없음"
          accent="green"
        />
        <Fact
          label="변경됨"
          value={`${e.pending_files.toLocaleString()}개`}
          hint={
            hasPending
              ? `≈ ${e.pending_tokens.toLocaleString()} tokens`
              : "없음"
          }
          accent={hasPending ? "yellow" : undefined}
        />
      </div>
      <div className="flex items-center justify-between">
        <span className="font-mono text-caption-sm text-stone">
          {e.model || "모델 미지정 (embedding 비활성)"}
        </span>
        <button
          onClick={onRefresh}
          disabled={estimating}
          className="inline-flex h-6 items-center gap-1 rounded-sm border border-hairline bg-surface px-2 text-[11px] text-body hover:text-on-dark"
        >
          <RefreshCw
            size={10}
            className={estimating ? "animate-spin" : ""}
          />
          다시 계산
        </button>
      </div>
    </div>
  );
}

function Fact({
  label,
  value,
  hint,
  accent,
}: {
  label: string;
  value: string;
  hint: string;
  accent?: "yellow" | "green";
}) {
  const color =
    accent === "yellow"
      ? "text-accent-yellow"
      : accent === "green"
        ? "text-accent-green"
        : "text-ink";
  return (
    <div className="flex flex-col rounded-sm bg-surface px-2 py-1.5">
      <span className="text-[11px] uppercase tracking-[0.4px] text-mute">
        {label}
      </span>
      <span className={cn("mt-0.5 text-[13px] font-medium", color)}>
        {value}
      </span>
      <span className="text-caption-sm text-mute">{hint}</span>
    </div>
  );
}

/** Settings 내 AI 연동 in-place 폼.
 *  - 현재 상태 표시 (어느 provider · 어느 모델)
 *  - 라디오 3개 (없음 · Gemini · Bedrock) + 입력 필드 + LLM 모델 드롭다운
 *  - 저장 시 cfg.embed_provider/embed_model/automation_model 갱신,
 *    키는 Keychain 으로
 *  - 재시작·재로드 없이 즉시 반영 (다음 검색·요약·purpose·ghost 호출이
 *    새 provider/model 로 전환됨)
 *
 *  온보딩의 ProviderRadio 와 다른 컴포넌트로 둠 — 온보딩은 첫 설치
 *  플로우에서 라이트한 미리보기, 여기는 변경·재구성·모델 선택에 특화. */

/** provider 별 LLM 자동화 모델 카탈로그. 임베딩은 provider 마다 1개로
 *  고정이라 따로 노출하지 않는다. 비용·속도·정확도 트레이드오프를
 *  사용자가 인지할 수 있게 hint 한 줄. */
const AUTOMATION_MODELS: Record<
  "google" | "bedrock",
  Array<{ id: string; label: string; hint: string }>
> = {
  google: [
    {
      id: "gemini-2.5-flash-lite",
      label: "Gemini 2.5 Flash Lite",
      hint: "가벼움 · 일 한도 큼 · 디폴트",
    },
    {
      id: "gemini-2.5-flash",
      label: "Gemini 2.5 Flash",
      hint: "균형 · 일 한도 작음",
    },
    {
      id: "gemini-3-flash",
      label: "Gemini 3 Flash",
      hint: "최신 · 일 한도 작음",
    },
    {
      id: "gemini-3-1-flash-lite",
      label: "Gemini 3.1 Flash Lite",
      hint: "최신 + 가벼움 · 일 500 RPD",
    },
  ],
  bedrock: [
    {
      id: "us.anthropic.claude-haiku-4-5-20251001-v1:0",
      label: "Claude Haiku 4.5",
      hint: "가벼움 · 디폴트 · US inference profile",
    },
    {
      id: "us.anthropic.claude-sonnet-4-6-20251001-v1:0",
      label: "Claude Sonnet 4.6",
      hint: "균형 · 비용 ~5배",
    },
    {
      id: "us.anthropic.claude-opus-4-7-20260101-v1:0",
      label: "Claude Opus 4.7",
      hint: "최고 정확도 · 비용 ~25배",
    },
  ],
};

function AiConnectCard({
  cfg,
  onSaved,
}: {
  cfg: DanbiConfig;
  onSaved: (next: DanbiConfig) => Promise<void> | void;
}) {
  type Kind = "none" | "google" | "bedrock";
  const currentKind: Kind =
    cfg.embed_provider?.kind === "google"
      ? "google"
      : cfg.embed_provider?.kind === "bedrock"
        ? "bedrock"
        : "none";
  const currentRegion =
    cfg.embed_provider?.kind === "bedrock"
      ? cfg.embed_provider.region ?? "us-east-1"
      : "us-east-1";
  const currentAutomation = cfg.automation_model?.trim() ?? "";

  const [kind, setKind] = useState<Kind>(currentKind);
  const [geminiKey, setGeminiKey] = useState("");
  const [bedrockRegion, setBedrockRegion] = useState(currentRegion);
  const [automation, setAutomation] = useState<string>(currentAutomation);
  const [saving, setSaving] = useState(false);
  const [flash, setFlash] = useState<{
    tone: "ok" | "err";
    text: string;
  } | null>(null);

  // kind 가 바뀌면 자동화 모델 디폴트도 그 provider 로 리셋. 사용자가
  // 명시적으로 골라둔 게 있으면 그건 그대로 유지하지만, provider 자체가
  // 바뀌면 이전 모델 ID 가 호환이 안 되므로 비움.
  useEffect(() => {
    if (kind === "none") {
      setAutomation("");
      return;
    }
    if (kind !== currentKind) {
      setAutomation("");
      return;
    }
    setAutomation(currentAutomation);
  }, [kind, currentKind, currentAutomation]);

  const automationOptions = kind === "none" ? [] : AUTOMATION_MODELS[kind];
  const automationDefault = automationOptions[0]?.id ?? "";

  const dirty =
    kind !== currentKind ||
    (kind === "google" && geminiKey.trim().length > 0) ||
    (kind === "bedrock" && bedrockRegion !== currentRegion) ||
    automation !== currentAutomation;

  async function save() {
    if (!cfg.vault_path) return;
    setSaving(true);
    setFlash(null);
    try {
      let embed_provider: DanbiConfig["embed_provider"] = null;
      let embed_model: string | null = null;
      if (kind === "google") {
        if (geminiKey.trim()) {
          await ipc.storeGoogleApiKey(geminiKey.trim());
        } else if (currentKind !== "google") {
          // kind 를 google 로 바꿨는데 키 입력은 비어있음 — 기존 keychain
          // 키를 그대로 쓸 의도가 아니면 사용자에게 알리고 멈춤.
          setFlash({
            tone: "err",
            text: "Gemini 키를 입력해주세요 (또는 기존 키 그대로 두려면 다른 provider 선택 후 다시 google).",
          });
          setSaving(false);
          return;
        }
        embed_provider = {
          kind: "google",
          api_key_ref: "keychain:danbi-google",
        };
        embed_model = "gemini-embedding-001";
      } else if (kind === "bedrock") {
        embed_provider = {
          kind: "bedrock",
          auth_mode: "env",
          profile: null,
          region: bedrockRegion.trim() || "us-east-1",
        };
        embed_model = "amazon.titan-embed-text-v2:0";
      }
      // 자동화 LLM 모델: 사용자가 명시 선택했으면 저장, 없으면 cfg 에서
      // null 로 비워서 백엔드의 provider 별 디폴트가 사용되게.
      const automation_model: string | null =
        kind === "none"
          ? null
          : automation.trim().length > 0
            ? automation.trim()
            : null;
      const next: DanbiConfig = {
        ...cfg,
        embed_provider,
        embed_model,
        automation_model,
      };
      await ipc.saveConfig(cfg.vault_path, next);
      await onSaved(next);
      setGeminiKey("");
      setFlash({
        tone: "ok",
        text:
          kind === "none"
            ? "AI 연동을 껐어요. 검색은 BM25 만으로 동작합니다."
            : kind === "google"
              ? "Gemini 임베딩 연결됨 — 의미 검색·요약·자동 작성·ghost 제안이 활성화됐어요."
              : "Bedrock Titan 임베딩 연결됨 — AWS 자격증명은 SDK 가 자동 로드합니다.",
      });
    } catch (e) {
      setFlash({ tone: "err", text: String(e) });
    } finally {
      setSaving(false);
    }
  }

  return (
    <div className="mb-4 rounded-lg border border-hairline bg-surface-elevated p-4">
      <div className="mb-3 flex items-center justify-between gap-2">
        <div className="text-[13px] font-medium text-on-dark">
          {currentKind === "none" ? "현재 연결 안 됨" : "현재 연결됨"}
          {currentKind !== "none" && (
            <span className="ml-2 rounded-xs bg-accent-blue-soft px-1.5 py-0.5 font-mono text-[11px] text-accent-blue">
              {currentKind === "google" ? "Gemini" : "Bedrock"}
            </span>
          )}
        </div>
        {flash && (
          <span
            className={cn(
              "text-[11px]",
              flash.tone === "ok" ? "text-accent-green" : "text-accent-red",
            )}
          >
            {flash.text}
          </span>
        )}
      </div>

      <div className="flex flex-col gap-2">
        <AiKindRow
          active={kind === "none"}
          onClick={() => setKind("none")}
          title="연결 안 함"
          desc="키워드 검색 (BM25 + 한국어 n-gram) 만 사용. AI 자동화 비활성."
        />
        <AiKindRow
          active={kind === "google"}
          onClick={() => setKind("google")}
          title="Google Gemini"
          logo="G"
          desc="gemini-embedding-001 · 무료 한도 · 카드 등록 불필요."
        >
          {kind === "google" && (
            <div className="mt-2 flex items-center gap-2">
              <input
                type="password"
                value={geminiKey}
                onChange={(e) => setGeminiKey(e.target.value)}
                placeholder={
                  currentKind === "google"
                    ? "기존 키 유지 — 새 키 입력 시 덮어씀"
                    : "AI…"
                }
                className="h-9 flex-1 rounded-md border border-hairline bg-surface px-3 font-mono text-[12px] text-ink outline-none focus:border-hairline-strong"
              />
              <a
                href="https://aistudio.google.com/app/apikey"
                target="_blank"
                rel="noreferrer"
                className="inline-flex h-9 items-center rounded-md border border-hairline bg-surface px-3 text-[12px] text-body hover:border-hairline-strong hover:text-on-dark"
              >
                키 발급 ↗
              </a>
            </div>
          )}
        </AiKindRow>
        <AiKindRow
          active={kind === "bedrock"}
          onClick={() => setKind("bedrock")}
          title="AWS Bedrock"
          logo="AWS"
          desc="amazon.titan-embed-text-v2 · ~/.aws/credentials 또는 AWS_* 환경변수 자동 감지."
        >
          {kind === "bedrock" && (
            <div className="mt-2 flex items-center gap-2">
              <span className="text-[12px] text-mute">Region</span>
              <input
                type="text"
                value={bedrockRegion}
                onChange={(e) => setBedrockRegion(e.target.value)}
                placeholder="us-east-1"
                className="h-9 w-[160px] rounded-md border border-hairline bg-surface px-3 font-mono text-[12px] text-ink outline-none focus:border-hairline-strong"
              />
              <span className="text-[11px] text-stone">
                자격증명은 단비에 저장되지 않아요 — AWS SDK 자동 로드.
              </span>
            </div>
          )}
        </AiKindRow>
      </div>

      {/* LLM 자동화 모델 드롭다운 — provider 가 선택돼있을 때만 노출.
          요약 / purpose 자동 작성 / ghost 제안 등 모든 자동화에 사용됨.
          임베딩은 provider 당 1개로 고정이라 따로 노출 안 함. */}
      {kind !== "none" && automationOptions.length > 0 && (
        <div className="mt-3 rounded-md border border-hairline bg-surface p-3">
          <div className="mb-2 flex items-center justify-between gap-2">
            <span className="text-[12px] font-medium text-on-dark">
              자동화 LLM 모델
            </span>
            <span className="text-[11px] text-stone">
              요약 · purpose 작성 · ghost 제안에 사용
            </span>
          </div>
          <div className="flex items-center gap-2">
            <select
              value={automation || automationDefault}
              onChange={(e) => setAutomation(e.target.value)}
              className="h-9 flex-1 rounded-md border border-hairline bg-surface-elevated px-3 font-mono text-[12px] text-ink outline-none focus:border-hairline-strong"
            >
              {automationOptions.map((m) => (
                <option key={m.id} value={m.id}>
                  {m.label} — {m.hint}
                </option>
              ))}
            </select>
          </div>
        </div>
      )}

      <div className="mt-4 flex items-center justify-between gap-2">
        <button
          type="button"
          onClick={() => {
            // App.tsx 가 localStorage 의 danbi.forceOnboarding 플래그를
            // 보고 다음 mount 에서 Onboarding 으로 라우팅. reload 한 번으로
            // 깨끗하게 진입.
            try {
              localStorage.setItem("danbi.forceOnboarding", "1");
            } catch {
              /* no-op */
            }
            window.location.reload();
          }}
          title="Welcome → AI 연동 → Vault → Template 4단계 마법사를 다시 진행"
          className="inline-flex h-8 items-center gap-1.5 rounded-md border border-hairline bg-surface-elevated px-3 text-[12px] text-body hover:border-hairline-strong hover:text-on-dark"
        >
          <Sparkles size={12} /> 온보딩 다시 보기
        </button>
        <button
          onClick={save}
          disabled={!dirty || saving}
          className={cn(
            "inline-flex h-8 items-center rounded-md px-4 text-[12px] font-medium transition-colors",
            dirty
              ? "bg-primary text-on-primary hover:bg-primary-pressed"
              : "bg-surface text-stone",
          )}
        >
          {saving ? "저장 중…" : "저장"}
        </button>
      </div>

      {kind !== "none" && currentKind !== "none" && cfg.embed_model && (
        <div className="mt-3 text-[11px] text-stone">
          provider 를 바꾸면 인덱스 dimension 이 달라져 재인덱싱이 필요할 수
          있어요. 저장 후 아래 "지금 재인덱싱" 버튼을 눌러주세요.
        </div>
      )}
    </div>
  );
}

function AiKindRow({
  active,
  onClick,
  title,
  desc,
  logo,
  children,
}: {
  active: boolean;
  onClick: () => void;
  title: string;
  desc: string;
  logo?: string;
  children?: React.ReactNode;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      className={cn(
        "flex flex-col rounded-md border p-3 text-left transition-colors",
        active
          ? "border-accent-blue bg-accent-blue-soft/40"
          : "border-hairline bg-surface hover:border-hairline-strong",
      )}
    >
      <div className="flex items-start gap-3">
        {logo && (
          <span className="grid h-7 w-7 shrink-0 place-items-center rounded-md bg-surface-card text-[11px] font-semibold text-on-dark">
            {logo}
          </span>
        )}
        <div className="flex-1">
          <div className="text-[13px] font-medium text-ink">{title}</div>
          <div className="mt-0.5 text-[12px] text-mute">{desc}</div>
        </div>
        {active && (
          <span className="mt-1 grid h-4 w-4 place-items-center rounded-full bg-accent-blue text-[9px] font-bold text-on-primary">
            ✓
          </span>
        )}
      </div>
      {children && <div onClick={(e) => e.stopPropagation()}>{children}</div>}
    </button>
  );
}

function AboutPanel({ cfg }: { cfg: DanbiConfig }) {
  return (
    <>
      <div className="mb-6 flex flex-col items-center gap-3 pt-2">
        <img
          src={appIconUrl}
          alt="Danbi"
          draggable={false}
          className="h-28 w-28 select-none rounded-2xl shadow-xl shadow-black/30"
        />
        <div className="text-center">
          <div className="text-[18px] font-medium text-ink">단비 (Danbi)</div>
          <div className="mt-0.5 text-caption-sm text-mute">
            v0.2 · © 2026 Dean Works inc.
          </div>
        </div>
      </div>

      <div className="mb-5 space-y-3 text-[14px] leading-[1.75] text-body">
        <p>
          <span className="text-on-dark">단비</span>는{" "}
          <span className="text-on-dark">Claude Code · Codex</span> 같은 외부 AI
          에이전트의 장기 기억이 되는 로컬 vault 에디터예요. LLM API 키를
          단비에 넣을 필요가 없어요 — 추론은 외부 에이전트가 자기 환경에서
          직접 합니다.
        </p>
        <p>
          카파시(Karpathy)의 <span className="text-on-dark">Wiki-LLM</span>{" "}
          아이디어에서 출발해, 마크다운 파일 하나하나가 에이전트의 외부 뇌가
          되도록 만들었어요. vault 에 쓰는 만큼 에이전트가 더 정확한 맥락으로
          답합니다.
        </p>
        <p>
          MCP(Model Context Protocol) 서버를 내장해서 외부 AI 에이전트가 vault
          를 직접 <span className="text-on-dark">읽고·쓸 수 있어요</span>.
          단비는 이 공유 뇌의 편집기·관리자·뷰어 역할에 집중합니다.
        </p>
        <p className="rounded-md border border-hairline bg-surface-elevated px-4 py-3 text-[12.5px] leading-[1.6] text-mute">
          <span className="text-on-dark">검색</span>은 기본 BM25 키워드 (한국어
          n-gram) 로 동작하고, 온보딩에서 Gemini 무료 임베딩 키를 입력하면{" "}
          <span className="text-on-dark">RRF 하이브리드</span> 로 의미 기반
          검색이 추가돼요. 임베딩을 켰을 때만 vault 본문이 Google 로 전송되고,
          그 외에는 외부 호출이 발생하지 않습니다. 키는 macOS 키체인에만
          저장돼요.
        </p>
        <p className="rounded-md border border-hairline bg-surface-elevated px-4 py-3 text-[12.5px] leading-[1.6] text-mute">
          새 프로젝트를 만들면 단비가{" "}
          <code className="rounded-xs border border-hairline bg-surface px-1 py-0.5 font-mono text-[11px] text-on-dark-mute">
            purpose.md
          </code>{" "}
          ·{" "}
          <code className="rounded-xs border border-hairline bg-surface px-1 py-0.5 font-mono text-[11px] text-on-dark-mute">
            schema.md
          </code>{" "}
          빈 골격을 자동으로 만들어둬요. 채우는 건{" "}
          <span className="text-on-dark">Claude Code · Codex</span> 같은 외부
          AI 에게 "이 프로젝트 정의·schema 정리해줘" 라고 시키면 vault 의 daily
          노트와 다른 도메인 파일을 자동으로 읽고 맥락에 맞게 채워줍니다 —
          단비 자체는 LLM 추론을 하지 않아요.
        </p>
      </div>

      <SectionTitle title="시스템" />
      <Row label="버전">
        <code className="rounded-sm border border-hairline bg-surface-elevated px-2 py-1 font-mono text-[11px] text-on-dark-mute">
          0.2
        </code>
      </Row>
      <Row label="플랫폼" hint="현재는 macOS 전용입니다.">
        <code className="rounded-sm border border-hairline bg-surface-elevated px-2 py-1 font-mono text-[11px] text-on-dark-mute">
          macOS 11+ · Apple Silicon / Intel
        </code>
      </Row>
      <Row
        label="Vault 경로"
        hint="프로젝트 마크다운과 설정이 저장되는 폴더. 자동으로 git repo로 초기화돼요."
      >
        <code className="rounded-sm border border-hairline bg-surface-elevated px-2 py-1 font-mono text-[11px] text-on-dark-mute">
          {cfg.vault_path ?? "—"}
        </code>
      </Row>
      <Row
        label="인증 정보"
        hint="API 키는 macOS 키체인에만 저장됩니다. Vault나 config.json에는 저장되지 않아요."
      >
        <code className="rounded-sm border border-hairline bg-surface-elevated px-2 py-1 font-mono text-[11px] text-on-dark-mute">
          Keychain
        </code>
      </Row>

      <div className="mt-6 border-t border-hairline pt-4">
        <SectionTitle title="유지보수" />
        <Row
          label="증분 캐시 초기화"
          hint="Ghost scan · Briefing · Compound 등 LLM 결과 캐시를 모두 지웁니다. 모델을 바꿨거나 결과가 이상할 때만 눌러주세요."
        >
          <button
            onClick={async () => {
              try {
                await ipc.cacheClear();
              } catch {
                /* ignore */
              }
            }}
            className="inline-flex h-7 items-center gap-1 rounded-sm border border-hairline bg-surface-elevated px-2 text-[12px] text-body hover:text-on-dark"
          >
            <RefreshCw size={11} /> 캐시 지우기
          </button>
        </Row>
      </div>

      <div className="mt-6 border-t border-hairline pt-4 text-caption-sm leading-[1.6] text-stone">
        제작 도움: Tauri v2 · Vite · React · tantivy
      </div>
    </>
  );
}

/** 진행 화면. 백엔드의 vector::reindex 가 emit 하는
 *  vector:reindex_progress 이벤트를 받아 한 카드로 보여준다.
 *  - phase=embedding: 진행률 바 + 현재 파일명
 *  - phase=waiting:   "다음 호출까지 N초" 카운트다운 + 왜 기다리는지 설명
 *  사용자가 "왜 멈춰있는지" 의아해하지 않도록 메시지를 명시적으로 단다. */
export function ReindexProgressCard({
  progress,
  providerKind,
}: {
  progress: import("@/lib/ipc").ReindexProgress | null;
  providerKind: string | null;
}) {
  const total = progress?.total ?? 0;
  const done = progress?.done ?? 0;
  const pct = total > 0 ? Math.min(100, Math.round((done / total) * 100)) : 0;
  const phase = progress?.phase ?? "embedding";
  const isWaiting = phase === "waiting";
  const waitSecs = progress?.wait_secs ?? null;
  const isGemini = providerKind === "google";

  return (
    <div className="mb-4 rounded-lg border border-hairline bg-surface-elevated p-4">
      <div className="flex items-center justify-between gap-3">
        <div className="flex items-center gap-2">
          <Loader2
            size={14}
            className={cn(
              "text-accent-blue",
              !isWaiting && "animate-spin",
            )}
          />
          <span className="text-[13px] font-semibold text-ink">
            {isWaiting ? "rate-limit 대기 중" : "임베딩 중"}
          </span>
          {progress?.last_file && !isWaiting && (
            <code className="font-mono text-[11px] text-on-dark-mute">
              {progress.last_file}
            </code>
          )}
        </div>
        <div className="text-[12px] tabular-nums text-on-dark-mute">
          {done} / {total} ({pct}%)
        </div>
      </div>

      {/* 진행률 바 */}
      <div className="mt-3 h-1.5 w-full overflow-hidden rounded-full bg-surface">
        <div
          className={cn(
            "h-full transition-[width] duration-500 ease-out",
            isWaiting ? "bg-accent-yellow" : "bg-accent-blue",
          )}
          style={{ width: `${pct}%` }}
        />
      </div>

      {/* 카운트다운 + 이유 설명 */}
      {isWaiting && waitSecs !== null && (
        <div className="mt-3 flex items-start gap-2 rounded-md border border-hairline bg-surface px-3 py-2 text-[12px] leading-[1.6] text-body">
          <span className="mt-[2px] inline-flex h-4 min-w-[28px] items-center justify-center rounded-xs bg-accent-yellow/20 px-1.5 font-mono text-[11px] font-semibold text-accent-yellow tabular-nums">
            {waitSecs}s
          </span>
          <div className="flex-1">
            <div className="text-ink">다음 batch 호출까지 대기 중…</div>
            {isGemini && (
              <div className="mt-0.5 text-stone">
                Gemini 무료 티어는 분당 5회 호출 제한이라, 단비가 12초 간격으로
                나눠 호출해 한도를 안 넘기게 합니다. 실제 멈춘 게 아니라
                정상 동작이에요.
              </div>
            )}
          </div>
        </div>
      )}

      {/* 처음 시작할 때 작은 설명 — 왜 시간이 걸리는지 미리 한 줄 */}
      {!isWaiting && isGemini && total > 5 && (
        <div className="mt-3 text-[11px] leading-[1.6] text-stone">
          Gemini 무료 한도(분당 5회) 안에서 4 파일씩 묶어 호출합니다. 약{" "}
          <span className="text-on-dark-mute tabular-nums">
            {Math.ceil(Math.max(0, total - done) / 4) * 13}초
          </span>{" "}
          예상.
        </div>
      )}
    </div>
  );
}
