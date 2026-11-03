import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Archive, ArrowRight, ChevronRight, CornerDownLeft, Pencil, Play } from "lucide-react";
import { ipc, type ProjectNode, type ResumeCard as Card } from "@/lib/ipc";
import { cn } from "@/lib/utils";
import { useApp } from "@/state/store";
import { TodoRow, daysAgo, sourceLabel, todoKey, useTodoToggle } from "@/main/pulseUi";

/** 프로젝트 홈 맨 위 재진입 카드 — "어디까지 했더라" 를 한 장에.
 *  쉬는 중/보관이면 위에 띠 + [다시 시작]. 다음 한 걸음(크게) → 열린 할 일
 *  | 지금 유효한 결정 · 체크리스트 진행. LLM 없음(파일·git 만). 흰 CTA 는
 *  헤더의 '새 노트' 가 쓰므로 여기 버튼은 전부 테두리 버튼. */
export function ResumeCard({ project, node }: { project: string; node: ProjectNode }) {
  const [card, setCard] = useState<Card | null>(null);
  const [error, setError] = useState<string | null>(null);
  const todoRev = useApp((s) => s.todoRev);
  const selectDomain = useApp((s) => s.selectDomain);
  const setView = useApp((s) => s.setView);
  const setCfg = useApp((s) => s.setCfg);
  const showToast = useApp((s) => s.showToast);
  const { toggle, busy } = useTodoToggle();

  // 문서가 바뀌면 다시 읽는다 — 프로젝트 안 최신 mtime 을 서명으로.
  const sig = useMemo(() => {
    let max = 0;
    let n = 0;
    const walk = (ds: { modified_ms?: number | null }[]) => {
      for (const d of ds) {
        n += 1;
        if ((d.modified_ms ?? 0) > max) max = d.modified_ms ?? 0;
      }
    };
    walk(node.domains);
    const sub = (fs: ProjectNode["subfolders"]) => {
      for (const f of fs) {
        walk(f.domains);
        sub(f.subfolders ?? []);
      }
    };
    sub(node.subfolders);
    return `${max}:${n}`;
  }, [node]);

  const load = useCallback(async () => {
    try {
      setCard(await ipc.resumeCard(project));
      setError(null);
    } catch (e) {
      setError(String(e));
    }
  }, [project]);

  useEffect(() => {
    setCard(null);
  }, [project]);
  useEffect(() => {
    load();
  }, [load, todoRev, sig]);

  async function resume() {
    try {
      setCfg(await ipc.projectResume(project));
      showToast("ok", `${project} 다시 시작 — 사이드바 위로 올라왔어요`);
      load();
    } catch (e) {
      showToast("err", String(e));
    }
  }
  async function archive() {
    try {
      setCfg(await ipc.projectSetArchived(project, true));
      showToast("ok", `${project} 보관 — 사이드바 '쉬는 중' 아래로 옮겼어요`);
      load();
    } catch (e) {
      showToast("err", String(e));
    }
  }

  if (error) {
    return (
      <section className="rounded-lg border border-hairline bg-surface px-5 py-4 text-[12px] text-mute">
        재진입 카드를 못 불러왔어요: {error}
      </section>
    );
  }
  if (!card) {
    return <section className="h-[132px] animate-pulse rounded-lg border border-hairline bg-surface" />;
  }

  const resting = card.state !== "active";
  return (
    <section className="overflow-hidden rounded-lg border border-hairline bg-surface">
      {resting && (
        <div className="flex flex-wrap items-center justify-between gap-2 border-b border-hairline bg-surface-elevated px-5 py-2.5">
          <span className="text-[12px] text-body">
            {card.state === "archived"
              ? "보관한 프로젝트예요. 할 일함과 시간 분배에서 빠져 있어요."
              : `${card.idle_days ?? "?"}일 동안 쉬었어요. 아래에서 멈춘 곳부터 이어가면 돼요.`}
          </span>
          <div className="flex items-center gap-1.5">
            <OutlineBtn onClick={resume}>
              <Play size={11} /> {card.state === "archived" ? "보관 해제" : "다시 시작"}
            </OutlineBtn>
            {card.state === "resting" && (
              <OutlineBtn onClick={archive} subtle>
                <Archive size={11} /> 보관
              </OutlineBtn>
            )}
          </div>
        </div>
      )}

      <div className="px-5 pt-4">
        <div className="flex flex-wrap items-center gap-x-2 gap-y-1 text-[12px] text-mute">
          <span>마지막 작업 {daysAgo(card.last_activity_ms)}</span>
          {card.last_note && (
            <>
              <span className="text-stone">·</span>
              <button
                onClick={() => selectDomain(project, card.last_note!.domain)}
                className="min-w-0 max-w-full truncate text-left hover:text-on-dark"
                title={card.last_note.domain}
              >
                <span className="font-mono">{sourceLabel(card.last_note.domain)}</span>: {card.last_note.summary}
              </button>
            </>
          )}
        </div>
        <NextStepBlock project={project} card={card} onChanged={load} />
      </div>

      <div
        className="mt-3 grid border-t border-hairline"
        style={{ gridTemplateColumns: "minmax(0,1fr) 240px" }}
      >
        <div className="min-w-0 border-r border-hairline">
          <div className="flex items-center justify-between px-4 pb-1 pt-3">
            <span className="text-caption-sm uppercase tracking-[0.4px] text-mute">
              열린 할 일 {card.todo_total}
            </span>
            {card.todo_total > 0 && (
              <button
                onClick={() => setView("todos")}
                className="inline-flex items-center gap-0.5 text-[11px] text-mute hover:text-on-dark"
                title="할 일함에서 이 프로젝트만 보기"
              >
                모두 보기 <ChevronRight size={11} />
              </button>
            )}
          </div>
          {card.todos.length === 0 ? (
            <div className="px-4 pb-4 pt-1 text-[12px] leading-[1.6] text-mute">
              열린 할 일이 없어요. 문서에 <span className="font-mono">- [ ] 할 일</span> 이나{" "}
              <span className="font-mono">### TODO</span> 아래 목록을 적으면 여기 모여요.
            </div>
          ) : (
            <ul className="pb-2">
              {card.todos.map((t) => (
                <TodoRow
                  key={todoKey(t)}
                  item={t}
                  busy={busy === todoKey(t)}
                  onDone={() => toggle(t)}
                  onOpen={() => selectDomain(t.project, t.domain)}
                />
              ))}
            </ul>
          )}
        </div>
        <div className="flex min-w-0 flex-col gap-3 px-4 py-3">
          <SideList
            label="지금 유효한 결정"
            empty="문서 종류를 결정(decision)으로 둔 문서가 여기 보여요."
            rows={card.decisions.map((d) => ({
              key: d.domain,
              title: sourceLabel(d.domain),
              hint: d.modified.slice(5),
              onClick: () => selectDomain(project, d.domain),
            }))}
          />
          {card.checklists.length > 0 && (
            <SideList
              label="체크리스트"
              rows={card.checklists.map((c) => ({
                key: c.domain,
                title: c.title || sourceLabel(c.domain),
                hint: `${c.done}/${c.open + c.done}`,
                progress: c.done / Math.max(1, c.open + c.done),
                onClick: () => selectDomain(project, c.domain),
              }))}
            />
          )}
          {card.decisions.length === 0 && card.checklists.length === 0 && card.recent_docs.length > 0 && (
            <SideList
              label="최근 문서"
              rows={card.recent_docs.map((d) => ({
                key: d.domain,
                title: d.title || sourceLabel(d.domain),
                hint: daysAgo(d.modified_ms),
                onClick: () => selectDomain(project, d.domain),
              }))}
            />
          )}
        </div>
      </div>
    </section>
  );
}

/** 다음 한 걸음 — 큰 한 줄 + [완료] [고치기]. 없으면 바로 입력칸. */
function NextStepBlock({
  project,
  card,
  onChanged,
}: {
  project: string;
  card: Card;
  onChanged: () => void;
}) {
  const next = card.next;
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState("");
  const [saving, setSaving] = useState(false);
  const inputRef = useRef<HTMLInputElement>(null);
  const showToast = useApp((s) => s.showToast);
  const bumpTodos = useApp((s) => s.bumpTodos);
  const selectDomain = useApp((s) => s.selectDomain);

  useEffect(() => {
    setEditing(false);
  }, [project]);
  useEffect(() => {
    if (editing) inputRef.current?.focus();
  }, [editing]);

  async function save(text: string) {
    setSaving(true);
    try {
      await ipc.nextStepSet(project, text);
      setEditing(false);
      setDraft("");
      onChanged();
    } catch (e) {
      showToast("err", String(e));
    } finally {
      setSaving(false);
    }
  }

  async function complete() {
    if (!next) return;
    setSaving(true);
    try {
      if (next.origin === "note" && next.todo) {
        await ipc.todoSetDone(next.todo, true);
        bumpTodos();
      } else {
        await ipc.nextStepSet(project, "");
        onChanged();
      }
      showToast("ok", "완료했어요. 다음 한 걸음도 적어 둘까요?");
      setDraft("");
      setEditing(true);
    } catch (e) {
      showToast("err", String(e));
    } finally {
      setSaving(false);
    }
  }

  if (editing || !next) {
    return (
      <div className="mt-2.5">
        <div className="text-caption-sm uppercase tracking-[0.4px] text-mute">다음 한 걸음</div>
        <div className="mt-1.5 flex items-center gap-2">
          <input
            ref={inputRef}
            value={draft}
            onChange={(e) => setDraft(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter" && !e.nativeEvent.isComposing) {
                e.preventDefault();
                if (draft.trim()) save(draft);
              } else if (e.key === "Escape") {
                setEditing(false);
              }
            }}
            placeholder="다음에 이 프로젝트를 열면 제일 먼저 할 일 한 줄"
            className="h-9 flex-1 rounded-md border border-hairline bg-surface-elevated px-3 text-[14px] text-ink placeholder:text-stone focus:border-hairline-strong focus:outline-none"
          />
          <OutlineBtn onClick={() => draft.trim() && save(draft)} disabled={!draft.trim() || saving}>
            <CornerDownLeft size={11} /> 적기
          </OutlineBtn>
          {next && (
            <OutlineBtn onClick={() => setEditing(false)} subtle>
              취소
            </OutlineBtn>
          )}
        </div>
        {!next && (
          <div className="mt-1.5 text-[11px] text-stone">
            Claude Code 가 세션을 마칠 때 <span className="font-mono">danbi_log(next_step)</span> 으로 남기거나,
            daily 노트의 <span className="font-mono">- 다음 할 일: …</span> 줄이 있으면 자동으로 채워져요.
          </div>
        )}
      </div>
    );
  }

  const origin =
    next.origin === "note" && next.todo
      ? `${sourceLabel(next.todo.domain)} 노트에서`
      : next.by && next.by !== "human"
        ? `${next.by} 가 남김`
        : "직접 적음";
  return (
    <div className="mt-2.5">
      <div className="text-caption-sm uppercase tracking-[0.4px] text-mute">다음 한 걸음</div>
      <div className="mt-1 flex items-start gap-3">
        <ArrowRight size={16} className="mt-[3px] shrink-0 text-on-dark-mute" />
        <div className="min-w-0 flex-1">
          <div className="text-[16px] font-medium leading-[1.45] text-ink">{next.text}</div>
          <div className="mt-0.5 text-[11px] text-stone">
            {next.todo ? (
              <button
                onClick={() => selectDomain(project, next.todo!.domain)}
                className="hover:text-on-dark"
              >
                {origin}
              </button>
            ) : (
              origin
            )}{" "}
            · {daysAgo(next.at_ms)}
          </div>
        </div>
        <div className="flex shrink-0 items-center gap-1.5">
          <OutlineBtn onClick={complete} disabled={saving}>
            완료
          </OutlineBtn>
          <OutlineBtn
            onClick={() => {
              setDraft(next.text);
              setEditing(true);
            }}
            subtle
            title="고치기"
          >
            <Pencil size={11} />
          </OutlineBtn>
        </div>
      </div>
    </div>
  );
}

function SideList({
  label,
  rows,
  empty,
}: {
  label: string;
  empty?: string;
  rows: { key: string; title: string; hint?: string; progress?: number; onClick: () => void }[];
}) {
  return (
    <div className="min-w-0">
      <div className="mb-1 text-caption-sm uppercase tracking-[0.4px] text-mute">{label}</div>
      {rows.length === 0 ? (
        empty ? <div className="text-[11px] leading-[1.5] text-stone">{empty}</div> : null
      ) : (
        <ul className="flex flex-col">
          {rows.map((r) => (
            <li key={r.key}>
              <button
                onClick={r.onClick}
                className="flex w-full min-w-0 flex-col gap-1 rounded-sm px-1 py-1 text-left hover:bg-surface-elevated"
                title={r.title}
              >
                <span className="flex w-full min-w-0 items-center gap-2 text-[12px]">
                  <span className="min-w-0 flex-1 truncate text-body">{r.title}</span>
                  {r.hint && <span className="shrink-0 font-mono text-[11px] text-stone">{r.hint}</span>}
                </span>
                {r.progress !== undefined && (
                  <span className="block h-[3px] w-full overflow-hidden rounded-full bg-surface-elevated">
                    <span
                      className="block h-full rounded-full bg-on-dark-mute"
                      style={{ width: `${Math.round(r.progress * 100)}%` }}
                    />
                  </span>
                )}
              </button>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}

export function OutlineBtn({
  children,
  onClick,
  disabled,
  subtle,
  title,
}: {
  children: React.ReactNode;
  onClick: () => void;
  disabled?: boolean;
  subtle?: boolean;
  title?: string;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      disabled={disabled}
      title={title}
      className={cn(
        "inline-flex h-7 shrink-0 items-center gap-1 rounded-md border px-2.5 text-[12px] transition-colors disabled:opacity-50",
        subtle
          ? "border-transparent text-mute hover:border-hairline hover:text-on-dark"
          : "border-hairline bg-surface text-body hover:border-hairline-strong hover:text-on-dark",
      )}
    >
      {children}
    </button>
  );
}
