import { useEffect } from "react";
import { Check, ChevronRight } from "lucide-react";
import { RelationTag, useReviewRows, type ReviewRow } from "@/main/ReviewView";
import { cn } from "@/lib/utils";

/**
 * 검토함 요약 카드 — 홈 우측 열·프로젝트 홈의 "검토 N ▸". ReviewView 와
 * 같은 큐(`useReviewRows`)를 상위 몇 건만 보여주고 인라인으로 승인/무시.
 */
export function ReviewMini({
  project,
  max = 4,
  onOpenDoc,
  onOpenAll,
  onCount,
}: {
  project?: string | null;
  max?: number;
  onOpenDoc: (project: string, domain: string) => void;
  onOpenAll: () => void;
  /** 호스트의 요약 한 줄("검토 대기 N")용 — 같은 큐를 두 번 받지 않게. */
  onCount?: (n: number) => void;
}) {
  const { rows, loading, initial, pendingTotal, actions } = useReviewRows(project);
  useEffect(() => {
    onCount?.(pendingTotal);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [pendingTotal]);
  const top = rows.slice(0, max);
  const more = pendingTotal - top.length;

  return (
    <section className="rounded-lg border border-hairline bg-surface">
      <button
        onClick={onOpenAll}
        className="flex w-full items-center justify-between px-4 py-2.5 text-left transition-colors hover:bg-surface-elevated"
        title="검토함 열기 (⌘⇧R)"
      >
        <span className="text-[13px] font-medium text-ink">
          검토함{pendingTotal > 0 ? ` ${pendingTotal}` : ""}
        </span>
        <ChevronRight size={13} className="text-mute" />
      </button>
      {initial && loading ? (
        <ul className="flex flex-col gap-1 border-t border-hairline p-3">
          {[0, 1].map((i) => (
            <li key={i} className="h-10 animate-pulse rounded-md bg-surface-elevated" />
          ))}
        </ul>
      ) : top.length === 0 ? (
        <div className="border-t border-hairline px-4 py-3 text-[12px] text-mute">
          검토할 게 없어요.
        </div>
      ) : (
        <ul className="border-t border-hairline">
          {top.map((r) => (
            <MiniRow
              key={r.key}
              row={r}
              scoped={!!project}
              onOpen={() => {
                const t = actions.open(r);
                if (t) onOpenDoc(t.project, t.domain);
              }}
              onApprove={r.approveLabel ? () => actions.approve(r) : undefined}
              onDismiss={() => actions.dismiss(r)}
            />
          ))}
        </ul>
      )}
      {more > 0 && (
        <div className="flex items-center justify-between border-t border-hairline px-4 py-2 text-[11px] text-mute">
          <span>… +{more}</span>
          <button onClick={onOpenAll} className="hover:text-on-dark">
            모두 보기
          </button>
        </div>
      )}
    </section>
  );
}

function MiniRow({
  row,
  scoped,
  onOpen,
  onApprove,
  onDismiss,
}: {
  row: ReviewRow;
  scoped: boolean;
  onOpen: () => void;
  onApprove?: () => void;
  onDismiss: () => void;
}) {
  const canOpen = !!row.domain;
  return (
    <li className="flex flex-col gap-1 border-b border-hairline px-4 py-2 last:border-b-0">
      <div className="flex items-center gap-1.5">
        <span
          className={cn(
            "shrink-0 rounded-xs bg-surface-elevated px-1.5 text-[10px] uppercase tracking-[0.4px]",
            row.badgeTone,
          )}
        >
          {row.badge}
        </span>
        <button
          onClick={onOpen}
          disabled={!canOpen}
          className="flex min-w-0 flex-1 items-center gap-1 text-left text-[12px] disabled:cursor-default"
          title={row.reason}
        >
          {!scoped && <span className="shrink-0 text-mute">{row.project}</span>}
          {row.domain && (
            <span className="truncate font-mono text-body">{row.domain}</span>
          )}
        </button>
      </div>
      <div className="flex items-center gap-2">
        {row.tag && <RelationTag>{row.tag}</RelationTag>}
        <span className="min-w-0 flex-1 truncate text-[11px] text-mute" title={row.reason}>
          {row.reason}
        </span>
        <span className="flex shrink-0 items-center gap-1">
          {onApprove && (
            <SmallButton onClick={onApprove}>
              <Check size={10} /> {row.approveLabel}
            </SmallButton>
          )}
          <SmallButton onClick={onDismiss} muted>
            무시
          </SmallButton>
        </span>
      </div>
    </li>
  );
}

function SmallButton({
  onClick,
  children,
  muted,
}: {
  onClick: () => void;
  children: React.ReactNode;
  muted?: boolean;
}) {
  return (
    <button
      onClick={(e) => {
        e.stopPropagation();
        onClick();
      }}
      className={cn(
        "inline-flex h-6 items-center gap-1 rounded-sm border border-hairline bg-surface-elevated px-2 text-[11px] transition-colors hover:border-hairline-strong hover:text-on-dark",
        muted ? "text-stone" : "text-body",
      )}
    >
      {children}
    </button>
  );
}
