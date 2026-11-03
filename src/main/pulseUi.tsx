import { useCallback, useState } from "react";
import { Check, Loader2 } from "lucide-react";
import { ipc, type TodoItem } from "@/lib/ipc";
import { useProjectColor } from "@/lib/projectColor";
import { cn } from "@/lib/utils";
import { useApp } from "@/state/store";

/** 다시 시작하기(0.10) 화면들이 같이 쓰는 조각 — 할 일 한 줄, 완료 처리,
 *  "N일 전" 표기. accent 색은 프로젝트 색 점에만 쓴다. */

const DAY = 86_400_000;

/** "오늘" · "어제" · "N일 전" · "N주 전" · "N달 전". */
export function daysAgo(ms: number | null | undefined): string {
  if (!ms) return "기록 없음";
  const start = new Date();
  start.setHours(0, 0, 0, 0);
  const d = Math.floor((start.getTime() + DAY - 1 - ms) / DAY);
  if (d <= 0) return "오늘";
  if (d === 1) return "어제";
  if (d < 14) return `${d}일 전`;
  if (d < 60) return `${Math.floor(d / 7)}주 전`;
  return `${Math.floor(d / 30)}달 전`;
}

/** daily/2026-11-02.md → "11-02 daily", 그 밖엔 파일 이름. */
export function sourceLabel(domain: string): string {
  const m = domain.match(/(\d{4})-(\d{2})-(\d{2})\.md$/);
  if (domain.startsWith("daily/") && m) return `${m[2]}-${m[3]} daily`;
  const base = domain.split("/").pop() ?? domain;
  return base.replace(/\.md$/i, "");
}

/** 분 → "3.5시간" · "40분". */
export function formatMinutes(min: number): string {
  if (min < 60) return `${min}분`;
  const h = min / 60;
  return `${h >= 10 ? Math.round(h) : Math.round(h * 10) / 10}시간`;
}

/** 할 일 완료 — 파일의 그 줄을 `[x]` 로 바꾸고 커밋. 토스트에 되돌리기는
 *  없지만 같은 줄을 다시 `[ ]` 로 돌리는 `undo` 를 돌려준다. */
export function useTodoToggle() {
  const showToast = useApp((s) => s.showToast);
  const bumpTodos = useApp((s) => s.bumpTodos);
  const [busy, setBusy] = useState<string | null>(null);
  const toggle = useCallback(
    async (t: TodoItem): Promise<boolean> => {
      const key = todoKey(t);
      setBusy(key);
      try {
        await ipc.todoSetDone(t, true);
        bumpTodos();
        return true;
      } catch (e) {
        showToast("err", String(e));
        return false;
      } finally {
        setBusy(null);
      }
    },
    [bumpTodos, showToast],
  );
  return { toggle, busy };
}

export function todoKey(t: Pick<TodoItem, "project" | "domain" | "line">): string {
  return `${t.project}/${t.domain}#${t.line}`;
}

/** 할 일 한 줄 — [□] 텍스트 · 출처. 체크하면 완료, 출처를 누르면 문서. */
export function TodoRow({
  item,
  showProject = false,
  busy,
  onDone,
  onOpen,
}: {
  item: TodoItem;
  showProject?: boolean;
  busy: boolean;
  onDone: () => void;
  onOpen: () => void;
}) {
  const color = useProjectColor()(item.project);
  return (
    <li className="group flex items-start gap-2.5 px-4 py-2">
      <button
        type="button"
        onClick={onDone}
        disabled={busy}
        title="완료 — 문서의 이 줄을 [x] 로 바꿔요"
        className={cn(
          "mt-[1px] grid h-4 w-4 shrink-0 place-items-center rounded-xs border border-hairline-strong text-transparent transition-colors",
          "hover:border-on-dark-mute hover:text-mute disabled:opacity-50",
        )}
      >
        {busy ? <Loader2 size={10} className="animate-spin text-mute" /> : <Check size={10} />}
      </button>
      <div className="min-w-0 flex-1">
        <div className="text-[13px] leading-[1.5] text-body">{item.text}</div>
        <button
          type="button"
          onClick={onOpen}
          className="mt-0.5 flex min-w-0 items-center gap-1.5 text-[11px] text-stone hover:text-on-dark"
          title={`${item.project}/${item.domain}${item.heading ? ` · ${item.heading}` : ""}`}
        >
          {showProject && (
            <>
              <span className="h-1.5 w-1.5 shrink-0 rounded-full" style={{ background: color }} />
              <span className="shrink-0 text-mute">{item.project}</span>
              <span>·</span>
            </>
          )}
          <span className="truncate font-mono">{sourceLabel(item.domain)}</span>
          {item.heading && !item.daily && (
            <span className="truncate">· {item.heading}</span>
          )}
        </button>
      </div>
    </li>
  );
}
