import { useCallback, useMemo, useRef, useState } from "react";
import { ArrowUp, FileText, Loader2, Paperclip, Undo2, X } from "lucide-react";
import {
  ipc,
  type Attachment,
  type Extracted,
  type RoutingContext,
  type RoutingResult,
} from "@/lib/ipc";
import { cn } from "@/lib/utils";
import { intentMeta, useApp, type ChatTurn } from "@/state/store";
import { CompoundDialog } from "@/main/CompoundDialog";

function newId() {
  return Math.random().toString(36).slice(2, 10);
}

export function CommandBar() {
  const cfg = useApp((s) => s.cfg);
  const tree = useApp((s) => s.tree);
  const selection = useApp((s) => s.selection);
  const selectProject = useApp((s) => s.selectProject);
  const selectDomain = useApp((s) => s.selectDomain);
  const turns = useApp((s) => s.turns);
  const addTurn = useApp((s) => s.addTurn);
  const patchTurn = useApp((s) => s.patchTurn);
  const clearTurns = useApp((s) => s.clearTurns);

  async function applyTurn(turnId: string) {
    const t = useApp.getState().turns.find((x) => x.id === turnId);
    if (!t?.plan?.op || !t.route?.project || !t.route?.domain) return;
    patchTurn(turnId, { status: "applying" });
    try {
      const res = await ipc.applyPlan({
        project: t.route.project,
        domain: t.route.domain,
        intent: t.route.intent,
        userMessage: t.user,
        summary: t.plan.summary,
        op: t.plan.op,
      });
      patchTurn(turnId, { status: "applied", commitAfter: res.commit_after });
    } catch (e) {
      patchTurn(turnId, { status: "error", error: String(e) });
    }
  }

  async function undoLast() {
    try {
      await ipc.undoLast();
    } catch (e) {
      console.error(e);
    }
  }

  const [input, setInput] = useState("");
  const [sending, setSending] = useState(false);
  const [attachments, setAttachments] = useState<Extracted[]>([]);
  const [dragOver, setDragOver] = useState(false);
  const [attachErr, setAttachErr] = useState<string | null>(null);
  const [compoundOpen, setCompoundOpen] = useState<{
    topic: string;
    project: string;
    targetDomain: string;
    userMessage: string;
  } | null>(null);
  const inputRef = useRef<HTMLTextAreaElement | null>(null);
  const fileRef = useRef<HTMLInputElement | null>(null);


  const extractFile = useCallback(async (file: File) => {
    try {
      const buf = new Uint8Array(await file.arrayBuffer());
      const ext = await ipc.extractFileBytes(
        file.name || `attachment-${Date.now()}`,
        buf,
      );
      setAttachments((prev) => {
        if (prev.some((p) => p.filename === ext.filename && p.bytes === ext.bytes)) {
          return prev;
        }
        return [...prev, ext];
      });
      setAttachErr(null);
    } catch (e) {
      console.error("[danbi] attach failed:", e);
      setAttachErr(String(e));
    }
  }, []);

  async function onDrop(e: React.DragEvent<HTMLDivElement>) {
    e.preventDefault();
    setDragOver(false);
    const files = Array.from(e.dataTransfer.files ?? []);
    for (const f of files) await extractFile(f);
  }

  const ctx: RoutingContext = useMemo(() => {
    const projects = tree?.projects.map((p) => p.name) ?? [];
    const domains: Record<string, string[]> = {};
    for (const p of tree?.projects ?? []) {
      domains[p.name] = p.domains.map((d) => d.name);
    }
    return {
      projects,
      domains,
      sticky_project: selection.project,
      sticky_domain: selection.domain,
    };
  }, [tree, selection.project, selection.domain]);

  function toAttachmentPayload(list: Extracted[]): Attachment[] {
    return list.map((a) => ({
      filename: a.filename,
      kind: a.kind,
      text: a.text,
      truncated: a.truncated,
    }));
  }

  async function send(text: string) {
    const trimmed = text.trim();
    const hasAttachments = attachments.length > 0;
    if ((!trimmed && !hasAttachments) || sending) return;
    if (!cfg) return;
    setSending(true);
    const id = newId();
    const userText =
      trimmed ||
      `첨부된 ${attachments.length}개 문서를 요약해서 정리해줘`;
    const currentAttachments = attachments;
    addTurn({
      id,
      user: userText,
      status: "routing",
      attachments: toAttachmentPayload(currentAttachments),
      createdAt: Date.now(),
    });
    setInput("");
    setAttachments([]);
    try {
      const payload = toAttachmentPayload(currentAttachments);
      const route = await ipc.routeMessage(userText, ctx, payload);
      if (route.needs_clarification) {
        patchTurn(id, { status: "clarify", route });
        setSending(false);
        return;
      }
      await runPreview(id, userText, route, payload);
    } catch (e) {
      patchTurn(id, { status: "error", error: String(e) });
    } finally {
      setSending(false);
    }
  }

  async function runPreview(
    turnId: string,
    userText: string,
    route: RoutingResult,
    attachmentPayload?: Attachment[],
  ) {
    if (!route.project || !route.domain) {
      patchTurn(turnId, { status: "clarify", route });
      return;
    }
    // Compound: hand off to the dedicated dialog so the user can approve
    // sources + see cost estimate before we spend the Writer tokens.
    if (route.intent === "compound") {
      patchTurn(turnId, { status: "ready", route });
      setCompoundOpen({
        topic: userText,
        project: route.project,
        targetDomain: route.domain,
        userMessage: userText,
      });
      return;
    }
    patchTurn(turnId, { status: "planning", route });
    try {
      const plan = await ipc.previewPlan({
        message: userText,
        project: route.project,
        domain: route.domain,
        intent: route.intent,
        attachments: attachmentPayload,
      });
      patchTurn(turnId, { status: "ready", route, plan });
    } catch (e) {
      patchTurn(turnId, { status: "error", route, error: String(e) });
    }
  }

  async function onChip(
    turnId: string,
    kind: "project" | "domain",
    value: string,
  ) {
    const t = useApp.getState().turns.find((x) => x.id === turnId);
    if (!t || !t.route) return;

    if (kind === "project") {
      selectProject(value);
      const merged: RoutingResult = {
        ...t.route,
        project: value,
        domain: null,
        candidate_projects: [],
      };
      // after picking project, we must re-ask for domain
      const firstDomain = (tree?.projects.find((p) => p.name === value)?.domains[0])
        ?.name;
      if (firstDomain && t.route.intent !== "unknown") {
        // still clarify for domain; user can also just click a chip
        patchTurn(turnId, {
          status: "clarify",
          route: {
            ...merged,
            needs_clarification: true,
            clarification_type: "domain",
            candidate_domains:
              tree?.projects.find((p) => p.name === value)?.domains.map((d) => d.name) ??
              [],
          },
        });
      } else {
        patchTurn(turnId, {
          status: "clarify",
          route: {
            ...merged,
            needs_clarification: true,
            clarification_type: "domain",
            candidate_domains:
              tree?.projects.find((p) => p.name === value)?.domains.map((d) => d.name) ??
              [],
          },
        });
      }
    } else {
      if (!t.route.project) return;
      selectDomain(t.route.project, value);
      const merged: RoutingResult = {
        ...t.route,
        domain: value,
        needs_clarification: false,
        clarification_type: null,
        candidate_domains: [],
      };
      await runPreview(turnId, t.user, merged, t.attachments);
    }
  }

  function cancel(turnId: string) {
    patchTurn(turnId, { status: "cancelled" });
  }

  return (
    <div className="flex flex-col border-t border-hairline bg-surface">
      {turns.length > 0 && (
        <div className="max-h-80 overflow-y-auto border-b border-hairline px-5 py-3">
          <div className="flex flex-col gap-3">
            {turns.map((t) => (
              <TurnView
                key={t.id}
                turn={t}
                onChip={onChip}
                onCancel={cancel}
                onApply={applyTurn}
              />
            ))}
          </div>
          <div className="mt-2 flex items-center justify-between">
            <button
              onClick={undoLast}
              className="inline-flex items-center gap-1 text-caption-sm text-stone transition-colors hover:text-mute"
              title="마지막 적용 되돌리기 (git reset)"
            >
              <Undo2 size={11} /> 되돌리기
            </button>
            <button
              onClick={clearTurns}
              className="text-caption-sm text-stone transition-colors hover:text-mute"
            >
              대화 지우기
            </button>
          </div>
        </div>
      )}

      <div
        onDragOver={(e) => {
          e.preventDefault();
          if (!dragOver) setDragOver(true);
        }}
        onDragLeave={() => setDragOver(false)}
        onDrop={onDrop}
        className={cn(
          "relative flex flex-col gap-2 px-5 py-3 transition-colors",
          dragOver && "bg-accent-blue-soft/40",
        )}
      >
        {dragOver && (
          <div className="pointer-events-none absolute inset-2 flex items-center justify-center rounded-md border border-dashed border-accent-blue text-caption-sm text-accent-blue">
            문서를 놓아서 첨부
          </div>
        )}
        {attachErr && (
          <div className="rounded-md border border-hairline bg-surface-elevated p-2 font-mono text-[11px] text-accent-red">
            {attachErr}
          </div>
        )}
        {attachments.length > 0 && (
          <div className="flex flex-wrap gap-1.5">
            {attachments.map((a, i) => (
              <span
                key={`${a.filename}-${i}`}
                className="inline-flex items-center gap-1.5 rounded-full border border-hairline bg-surface-elevated px-2 py-1 text-[12px] text-body"
                title={`${a.kind} · ${a.bytes.toLocaleString()} bytes${a.truncated ? " · truncated" : ""}`}
              >
                <FileText size={11} className="text-mute" />
                <span className="max-w-[240px] truncate font-mono text-[11px]">
                  {a.filename}
                </span>
                <span className="text-stone">
                  {formatBytes(a.text.length)}
                </span>
                <button
                  onClick={() =>
                    setAttachments((prev) => prev.filter((_, j) => j !== i))
                  }
                  className="ml-0.5 text-stone transition-colors hover:text-accent-red"
                  title="제거"
                >
                  <X size={11} />
                </button>
              </span>
            ))}
          </div>
        )}
        <form
          onSubmit={(e) => {
            e.preventDefault();
            send(input);
          }}
          className="flex items-end gap-2"
        >
          <StickyChip />
          <button
            type="button"
            onClick={() => fileRef.current?.click()}
            className="grid h-9 w-9 shrink-0 place-items-center rounded-md border border-hairline bg-surface-elevated text-mute transition-colors hover:border-hairline-strong hover:text-on-dark"
            title="파일 첨부"
          >
            <Paperclip size={14} />
          </button>
          <input
            ref={fileRef}
            type="file"
            multiple
            accept=".pdf,.docx,.md,.markdown,.txt,.csv,.json,.yaml,.yml,.toml,.log"
            className="hidden"
            onChange={async (e) => {
              const files = Array.from(e.target.files ?? []);
              for (const f of files) await extractFile(f);
              if (fileRef.current) fileRef.current.value = "";
            }}
          />
          <textarea
            ref={inputRef}
            value={input}
            onChange={(e) => setInput(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter" && !e.shiftKey && !e.nativeEvent.isComposing) {
                e.preventDefault();
                send(input);
              }
            }}
            placeholder={
              !cfg?.provider
                ? "LLM 미연결 — Provider 연결 후 사용할 수 있어요"
                : selection.project && selection.domain
                  ? `${selection.project}/${selection.domain} 에 어떤 변경을 할까요?`
                  : "프로젝트와 도메인을 선택하거나, 명령을 입력하세요"
            }
            rows={1}
            disabled={!cfg?.provider}
            className="min-h-9 flex-1 resize-none rounded-md border border-hairline bg-surface-elevated px-3 py-2 text-[14px] text-ink outline-none transition-colors placeholder:text-stone focus:border-hairline-strong disabled:cursor-not-allowed disabled:opacity-60"
            style={{ maxHeight: 160 }}
          />
          <button
            type="submit"
            disabled={
              !cfg?.provider ||
              (!input.trim() && attachments.length === 0) ||
              sending
            }
            title={
              !cfg?.provider
                ? "LLM 연결 필요 (Settings → Provider)"
                : "전송 (Enter)"
            }
            className="grid h-9 w-9 shrink-0 place-items-center rounded-md bg-primary text-on-primary transition-colors hover:bg-primary-pressed disabled:bg-surface-elevated disabled:text-ash"
          >
            {sending ? <Loader2 size={14} className="animate-spin" /> : <ArrowUp size={14} />}
          </button>
        </form>
      </div>

      {compoundOpen && (
        <CompoundDialog
          open={true}
          topic={compoundOpen.topic}
          project={compoundOpen.project}
          targetDomain={compoundOpen.targetDomain}
          userMessage={compoundOpen.userMessage}
          onClose={() => setCompoundOpen(null)}
        />
      )}
    </div>
  );
}

function formatBytes(n: number): string {
  if (n < 1024) return `${n}B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)}KB`;
  return `${(n / (1024 * 1024)).toFixed(1)}MB`;
}

function StickyChip() {
  const selection = useApp((s) => s.selection);
  if (!selection.project && !selection.domain) return null;
  return (
    <div className="mb-1 flex h-9 items-center gap-1 rounded-md border border-hairline bg-surface-elevated px-2 text-caption-sm">
      <span className="text-mute">{selection.project ?? "?"}</span>
      {selection.domain && (
        <>
          <span className="text-stone">/</span>
          <span className="font-mono text-on-dark">{selection.domain}</span>
        </>
      )}
    </div>
  );
}

function TurnView({
  turn,
  onChip,
  onCancel,
  onApply,
}: {
  turn: ChatTurn;
  onChip: (id: string, kind: "project" | "domain", value: string) => void;
  onCancel: (id: string) => void;
  onApply: (id: string) => void;
}) {
  return (
    <div className="flex flex-col gap-2 rounded-md border border-hairline bg-surface-elevated p-3">
      <div className="flex items-start justify-between gap-3">
        <div className="flex-1 text-[14px] leading-[1.5] text-ink">
          <span className="mr-2 text-caption-sm uppercase tracking-[0.4px] text-stone">
            you
          </span>
          {turn.user}
          {turn.attachments && turn.attachments.length > 0 && (
            <div className="mt-1.5 flex flex-wrap gap-1">
              {turn.attachments.map((a, i) => (
                <span
                  key={`${a.filename}-${i}`}
                  className="inline-flex items-center gap-1 rounded-xs bg-surface px-1.5 py-0.5 text-[11px] text-on-dark-mute"
                  title={`${a.kind} · ${a.text.length} chars${a.truncated ? " · truncated" : ""}`}
                >
                  <FileText size={9} className="text-stone" />
                  <span className="font-mono">{a.filename}</span>
                </span>
              ))}
            </div>
          )}
        </div>
        {turn.status !== "ready" && turn.status !== "cancelled" && turn.status !== "error" && (
          <button
            onClick={() => onCancel(turn.id)}
            className="shrink-0 text-mute transition-colors hover:text-on-dark"
            title="취소"
          >
            <X size={14} />
          </button>
        )}
      </div>

      {turn.status === "routing" && (
        <StatusLine icon="spin" label="라우팅 중…" />
      )}
      {turn.status === "planning" && (
        <StatusLine icon="spin" label="계획 작성 중…" />
      )}
      {turn.status === "applying" && (
        <StatusLine icon="spin" label="문서에 적용 중…" />
      )}
      {turn.status === "cancelled" && (
        <StatusLine icon="dot" label="취소됨" tone="mute" />
      )}
      {turn.status === "error" && (
        <div className="rounded-md border border-hairline bg-surface px-2 py-1.5 font-mono text-[12px] text-accent-red">
          {turn.error}
        </div>
      )}

      {turn.status === "clarify" && turn.route && (
        <ClarifyBlock turn={turn} onChip={onChip} />
      )}

      {(turn.status === "ready" ||
        turn.status === "applying" ||
        turn.status === "applied") &&
        turn.route &&
        turn.plan && <PlanBlock turn={turn} onApply={onApply} />}
    </div>
  );
}

function StatusLine({
  icon,
  label,
  tone = "neutral",
}: {
  icon: "spin" | "dot";
  label: string;
  tone?: "neutral" | "mute";
}) {
  return (
    <div
      className={cn(
        "flex items-center gap-2 text-caption-sm",
        tone === "mute" ? "text-stone" : "text-mute",
      )}
    >
      {icon === "spin" ? (
        <Loader2 size={12} className="animate-spin" />
      ) : (
        <span className="h-1.5 w-1.5 rounded-full bg-stone" />
      )}
      {label}
    </div>
  );
}

function Chip({
  active,
  onClick,
  children,
  mono,
}: {
  active?: boolean;
  onClick: () => void;
  children: React.ReactNode;
  mono?: boolean;
}) {
  return (
    <button
      onClick={onClick}
      className={cn(
        "inline-flex items-center rounded-full border border-hairline px-2.5 py-1 text-[13px] transition-colors",
        active
          ? "bg-surface-card text-on-dark"
          : "bg-transparent text-body hover:bg-surface hover:text-on-dark",
        mono && "font-mono",
      )}
    >
      {children}
    </button>
  );
}

function ClarifyBlock({
  turn,
  onChip,
}: {
  turn: ChatTurn;
  onChip: (id: string, kind: "project" | "domain", value: string) => void;
}) {
  const route = turn.route!;
  if (route.clarification_type === "project") {
    const candidates =
      route.candidate_projects.length > 0
        ? route.candidate_projects
        : useApp.getState().tree?.projects.map((p) => p.name) ?? [];
    return (
      <div className="flex flex-col gap-1.5">
        <div className="text-caption-sm text-mute">
          어떤 프로젝트에 대한 작업일까요?
        </div>
        <div className="flex flex-wrap gap-1.5">
          {candidates.length === 0 ? (
            <span className="text-caption-sm text-stone">
              등록된 프로젝트가 없어요. 먼저 사이드바에서 추가해 주세요.
            </span>
          ) : (
            candidates.map((p) => (
              <Chip key={p} onClick={() => onChip(turn.id, "project", p)}>
                {p}
              </Chip>
            ))
          )}
        </div>
      </div>
    );
  }
  if (route.clarification_type === "domain" && route.project) {
    const candidates =
      route.candidate_domains.length > 0
        ? route.candidate_domains
        : useApp
            .getState()
            .tree?.projects.find((p) => p.name === route.project)
            ?.domains.map((d) => d.name) ?? [];
    return (
      <div className="flex flex-col gap-1.5">
        <div className="text-caption-sm text-mute">
          <span className="text-body">{route.project}</span> 의 어떤 도메인일까요?
        </div>
        <div className="flex flex-wrap gap-1.5">
          {candidates.length === 0 ? (
            <span className="text-caption-sm text-stone">
              도메인 파일이 없어요. 사이드바에서 추가해 주세요.
            </span>
          ) : (
            candidates.map((d) => (
              <Chip key={d} mono onClick={() => onChip(turn.id, "domain", d)}>
                {d}
              </Chip>
            ))
          )}
        </div>
      </div>
    );
  }
  return (
    <div className="text-caption-sm text-mute">
      요청이 불확실해요. 더 구체적으로 말씀해 주실 수 있을까요?
    </div>
  );
}

function PlanBlock({
  turn,
  onApply,
}: {
  turn: ChatTurn;
  onApply: (id: string) => void;
}) {
  const route = turn.route!;
  const plan = turn.plan!;
  const meta = intentMeta(route.intent);

  const isAsk = route.intent === "ask";
  const hasOp = plan.op != null;
  const applied = turn.status === "applied";
  const applying = turn.status === "applying";

  const opBadge = plan.op
    ? plan.op.op === "append"
      ? "append"
      : plan.op.op === "insert_after"
        ? "insert"
        : plan.op.op === "replace_section"
          ? "replace"
          : "rewrite"
    : null;

  return (
    <div className="flex flex-col gap-2">
      <div className="flex flex-wrap items-center gap-2 text-caption-sm">
        <span
          className={cn(
            "rounded-xs px-1.5 py-0.5 text-[11px] uppercase tracking-[0.4px]",
            meta.tone === "write"
              ? "bg-accent-blue-soft text-accent-blue"
              : meta.tone === "ask"
                ? "bg-accent-green-soft text-accent-green"
                : "bg-surface text-mute",
          )}
        >
          {meta.label}
        </span>
        {opBadge && !isAsk && (
          <span className="rounded-xs bg-surface px-1.5 py-0.5 text-[11px] uppercase tracking-[0.4px] text-on-dark-mute">
            {opBadge}
          </span>
        )}
        <span className="text-mute">{route.project}</span>
        <span className="text-stone">/</span>
        <span className="font-mono text-body">{route.domain}</span>
        <span className="ml-auto text-stone">
          confidence {(route.confidence * 100).toFixed(0)}%
        </span>
      </div>

      <div className="rounded-md border border-hairline bg-surface p-3">
        <div className="text-[13px] leading-[1.5] text-ink">{plan.summary}</div>
        {plan.detail && (
          <div className="mt-1.5 text-caption-md leading-[1.6] text-body">
            {plan.detail}
          </div>
        )}
        {isAsk && plan.answer && (
          <div className="mt-2 rounded-sm border border-hairline bg-surface-elevated p-2 text-[13px] leading-[1.6] text-ink whitespace-pre-wrap">
            {plan.answer}
          </div>
        )}
        {!isAsk && plan.draft && (
          <pre className="mt-2 max-h-40 overflow-auto rounded-sm border border-hairline bg-surface-elevated p-2 font-mono text-[11px] leading-[1.6] text-body whitespace-pre-wrap break-words">
            {plan.draft}
          </pre>
        )}
      </div>

      {!isAsk && (
        <div className="flex items-center gap-2">
          {applied ? (
            <div className="inline-flex items-center gap-1.5 rounded-md border border-hairline bg-accent-green-soft px-2.5 py-1 text-[12px] text-accent-green">
              <span className="h-1.5 w-1.5 rounded-full bg-accent-green" />
              적용됨
              {turn.commitAfter && (
                <span className="font-mono text-[11px] text-accent-green/80">
                  {turn.commitAfter.slice(0, 7)}
                </span>
              )}
            </div>
          ) : (
            <button
              onClick={() => onApply(turn.id)}
              disabled={!hasOp || applying}
              className="inline-flex h-8 items-center rounded-md bg-primary px-3 text-[13px] font-medium text-on-primary transition-colors hover:bg-primary-pressed disabled:bg-surface-elevated disabled:text-ash"
              title={hasOp ? "문서에 적용" : "적용할 수 있는 편집이 없어요"}
            >
              {applying ? (
                <span className="inline-flex items-center gap-1.5">
                  <Loader2 size={12} className="animate-spin" />
                  적용 중…
                </span>
              ) : (
                "적용"
              )}
            </button>
          )}
          {!hasOp && (
            <span className="text-caption-sm text-stone">
              모델이 편집 계획을 만들지 못했어요. 요청을 더 구체적으로
              바꿔 보세요.
            </span>
          )}
        </div>
      )}
    </div>
  );
}
