import { Link2 } from "lucide-react";
import { useApp } from "@/state/store";

/**
 * Small strip under the document view showing "이 문서를 참조하는 곳" —
 * other markdown files that link to the currently selected domain via
 * [[project/domain]] syntax. Clickable.
 */
export function Backlinks() {
  const selection = useApp((s) => s.selection);
  const index = useApp((s) => s.linkIndex);
  const selectDomain = useApp((s) => s.selectDomain);

  if (!selection.project || !selection.domain || !index) return null;

  const key = `${selection.project}/${selection.domain}`;
  const incoming = index.incoming[key] ?? [];

  if (incoming.length === 0) return null;

  return (
    <div className="shrink-0 border-t border-hairline bg-surface px-5 py-2">
      <div className="flex items-center gap-1.5 text-caption-sm text-mute">
        <Link2 size={10} />
        <span>이 문서를 참조하는 곳</span>
        <span className="text-stone">·</span>
        <span>{incoming.length}</span>
      </div>
      <div className="mt-1 flex flex-wrap gap-1.5">
        {incoming.map((l, i) => (
          <button
            key={`${l.project}/${l.domain}-${i}`}
            onClick={() => selectDomain(l.project, l.domain)}
            className="inline-flex items-center gap-1 rounded-full border border-hairline bg-surface-elevated px-2 py-0.5 text-[11px] text-body transition-colors hover:border-hairline-strong hover:text-on-dark"
          >
            <span className="text-mute">{l.project}</span>
            <span className="text-stone">/</span>
            <span className="font-mono">{l.domain}</span>
          </button>
        ))}
      </div>
    </div>
  );
}
