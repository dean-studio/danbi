import { Check } from "lucide-react";
import type { BedrockModel } from "@/lib/ipc";
import { cn } from "@/lib/utils";

export function isInferenceProfile(id: string): boolean {
  // Cross-region profiles look like "us.<provider>.<model>…"
  return /^[a-z]{2,3}\./.test(id);
}

export function claudeVersionScore(id: string): number {
  // Quick ranking so Claude 4.x beats 3.x.
  const m = id.match(/claude-([a-z]+)-(\d+)(?:-(\d+))?/i);
  if (!m) return 0;
  const major = parseInt(m[2], 10);
  const minor = m[3] ? parseInt(m[3], 10) : 0;
  return major * 100 + minor;
}

export function sortBedrockModels(list: BedrockModel[]): BedrockModel[] {
  return [...list].sort((a, b) => {
    const ap = isInferenceProfile(a.id) ? 0 : 1;
    const bp = isInferenceProfile(b.id) ? 0 : 1;
    if (ap !== bp) return ap - bp;
    const av = claudeVersionScore(a.id);
    const bv = claudeVersionScore(b.id);
    if (av !== bv) return bv - av;
    return a.id.localeCompare(b.id);
  });
}

export function filterInvokable(list: BedrockModel[]): BedrockModel[] {
  const ok = list.filter(
    (m) =>
      m.on_demand &&
      m.modalities_in.includes("TEXT") &&
      m.modalities_out.includes("TEXT"),
  );
  return ok.length > 0 ? ok : list;
}

/**
 * Extracts a short, readable label from a model id.
 *   "us.anthropic.claude-haiku-4-5-20251001-v1:0" -> "haiku 4.5"
 *   "gemini-2.5-flash-lite"                       -> "gemini 2.5 flash-lite"
 *   "gemini-3-flash"                              -> "gemini 3 flash"
 *   "gemini-3-1-flash-lite"                       -> "gemini 3.1 flash-lite"
 *   "gemini-embedding-001"                        -> "gemini embedding"
 *   "amazon.titan-embed-text-v2:0"                -> "titan embed v2"
 *   "text-embedding-3-small"                      -> "text-embedding-3-small"
 */
export function shortModel(id: string | null | undefined): string {
  if (!id) return "—";

  // Bedrock Claude (us./eu./apac. prefix 무시)
  const claude = id.match(/claude-(haiku|sonnet|opus)-(\d+)(?:-(\d+))?/i);
  if (claude) {
    const family = claude[1].toLowerCase();
    const v = claude[3] ? `${claude[2]}.${claude[3]}` : claude[2];
    return `${family} ${v}`;
  }

  // Gemini embedding ("gemini-embedding-001")
  if (/^gemini-embedding/i.test(id)) {
    return "gemini embedding";
  }

  // Gemini chat models — major-(minor)-tier-variant
  // "gemini-2.5-flash-lite" / "gemini-2-5-flash-lite" / "gemini-3-flash" /
  // "gemini-3-1-flash-lite"
  const gemini = id.match(
    /^gemini-(\d+)(?:[.-](\d+))?-(flash|pro)(?:-(.+))?$/i,
  );
  if (gemini) {
    const v = gemini[2] ? `${gemini[1]}.${gemini[2]}` : gemini[1];
    const tier = gemini[3].toLowerCase();
    const variant = gemini[4] ? ` ${gemini[4]}` : "";
    return `gemini ${v} ${tier}${variant}`;
  }

  // Bedrock Titan embed
  if (/titan-embed/i.test(id)) {
    const v = id.match(/v(\d+)/i);
    return v ? `titan embed v${v[1]}` : "titan embed";
  }

  // Voyage embed
  if (/^voyage-/i.test(id)) return id;

  // Generic fallback — strip dates / region prefix and return last hyphen
  // chunk pair (e.g. "amazon.titan-embed-text-v2:0" → "titan-embed-text").
  return id
    .replace(/^(us|eu|ap|apac)\./, "")
    .replace(/^anthropic\./, "")
    .replace(/^amazon\./, "")
    .replace(/-\d{8}.*$/, "")
    .replace(/:\d+$/, "");
}

/**
 * Scrollable picker list. Active row uses the blue accent soft background
 * plus a left vertical indicator and a check icon — unmistakable.
 */
export type ModelRecommendation = {
  /** Model id to highlight as the recommended pick for this role. */
  id: string;
  /** Short Korean reason shown below the model name. */
  reason: string;
};

export function ModelPicker({
  value,
  onChange,
  options,
  recommendation,
}: {
  value: string;
  onChange: (v: string) => void;
  options: BedrockModel[];
  recommendation?: ModelRecommendation | null;
}) {
  // Pin the recommended model to the top so the user's eye lands on it
  // first — classic "sorted by fit" pattern rather than alphabetical.
  const ordered = recommendation
    ? [
        ...options.filter((m) => m.id === recommendation.id),
        ...options.filter((m) => m.id !== recommendation.id),
      ]
    : options;

  return (
    <div className="rounded-md border border-hairline bg-surface">
      <div className="max-h-[360px] overflow-auto">
        {ordered.length === 0 && (
          <div className="px-3 py-3 text-[13px] text-stone">
            사용 가능한 모델이 없어요.
          </div>
        )}
        {ordered.map((m) => {
          const active = m.id === value;
          const recommended = recommendation?.id === m.id;
          return (
            <button
              key={m.id}
              onClick={() => onChange(m.id)}
              className={cn(
                "relative flex w-full items-center gap-3 border-l-2 px-3 py-2.5 text-left transition-colors",
                active
                  ? "border-accent-blue bg-accent-blue-soft"
                  : recommended
                    ? "border-accent-green/60 bg-accent-green-soft/40 hover:bg-accent-green-soft/60"
                    : "border-transparent bg-transparent hover:bg-surface-elevated",
              )}
            >
              <div className="flex-1 overflow-hidden">
                <div className="flex items-center gap-1.5">
                  <span
                    className={cn(
                      "truncate text-[14px]",
                      active ? "text-on-dark" : "text-ink",
                    )}
                  >
                    {m.name ?? m.id}
                  </span>
                  {recommended && !active && (
                    <span className="inline-flex items-center rounded-xs bg-accent-green-soft px-1.5 py-0.5 text-[10px] font-semibold uppercase tracking-[0.5px] text-accent-green">
                      추천
                    </span>
                  )}
                  {active && (
                    <span className="rounded-xs bg-accent-blue-soft px-1 py-0.5 text-[10px] font-medium uppercase tracking-[0.4px] text-accent-blue">
                      선택됨
                    </span>
                  )}
                </div>
                <div
                  className={cn(
                    "truncate font-mono text-[11px]",
                    active ? "text-accent-blue" : "text-mute",
                  )}
                >
                  {m.id}
                </div>
                {recommended && (
                  <div
                    className={cn(
                      "mt-0.5 truncate text-[11px]",
                      active ? "text-accent-blue" : "text-accent-green",
                    )}
                  >
                    {recommendation!.reason}
                  </div>
                )}
              </div>
              {active ? (
                <Check size={14} className="shrink-0 text-accent-blue" />
              ) : m.provider ? (
                <span className="shrink-0 rounded-xs bg-surface-elevated px-1.5 py-0.5 text-[11px] text-on-dark-mute">
                  {m.provider}
                </span>
              ) : null}
            </button>
          );
        })}
      </div>
    </div>
  );
}

/**
 * Compact summary of the current selection — a card shown above the picker.
 */
export function CurrentSelectionCard({
  roleLabel,
  id,
  meta,
}: {
  roleLabel: string;
  id: string | null;
  meta?: string;
}) {
  return (
    <div className="flex items-center gap-3 rounded-md border border-accent-blue bg-accent-blue-soft px-3 py-2.5">
      <div className="grid h-8 w-8 shrink-0 place-items-center rounded-md bg-surface text-[10px] font-medium uppercase tracking-[0.4px] text-accent-blue">
        {roleLabel}
      </div>
      <div className="flex-1 overflow-hidden">
        <div className="flex items-center gap-1.5">
          <span className="truncate text-[14px] font-medium text-ink">
            {shortModel(id)}
          </span>
          {meta && (
            <span className="rounded-xs bg-surface px-1.5 py-0.5 text-[11px] text-on-dark-mute">
              {meta}
            </span>
          )}
        </div>
        <div
          className="truncate font-mono text-[11px] text-accent-blue"
          title={id ?? undefined}
        >
          {id ?? "선택된 모델이 없습니다"}
        </div>
      </div>
    </div>
  );
}
