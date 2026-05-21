import { useEffect, useState } from "react";
import { FileText, Layers, Loader2, Sparkles } from "lucide-react";
import { Dialog } from "@/components/Dialog";
import { PrimaryButton, SecondaryButton } from "@/components/WizardShell";
import { ipc, type CompoundPreview } from "@/lib/ipc";
import { cn } from "@/lib/utils";
import { useApp } from "@/state/store";

type Phase =
  | { kind: "closed" }
  | { kind: "fetching" }
  | { kind: "ready"; preview: CompoundPreview }
  | { kind: "applying" }
  | { kind: "done"; project: string; domain: string }
  | { kind: "error"; message: string };

export function CompoundDialog({
  open,
  topic,
  project,
  targetDomain,
  userMessage,
  onClose,
}: {
  open: boolean;
  topic: string;
  project: string;
  targetDomain: string;
  userMessage: string;
  onClose: () => void;
}) {
  const [phase, setPhase] = useState<Phase>({ kind: "closed" });
  const [maxSources, setMaxSources] = useState<number>(8);
  const selectDomain = useApp((s) => s.selectDomain);

  // Kick off the preview as soon as the dialog opens with a fresh topic.
  useEffect(() => {
    if (!open) {
      setPhase({ kind: "closed" });
      return;
    }
    setPhase({ kind: "fetching" });
    ipc
      .compoundPreview({
        topic,
        project,
        targetDomain,
        maxSources,
      })
      .then((preview) => setPhase({ kind: "ready", preview }))
      .catch((e) => setPhase({ kind: "error", message: String(e) }));
    // Intentionally depend only on open/topic/target — maxSources change
    // triggers a manual refresh via button below.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open, topic, project, targetDomain]);

  async function regenerate(newMax: number) {
    setMaxSources(newMax);
    setPhase({ kind: "fetching" });
    try {
      const preview = await ipc.compoundPreview({
        topic,
        project,
        targetDomain,
        maxSources: newMax,
      });
      setPhase({ kind: "ready", preview });
    } catch (e) {
      setPhase({ kind: "error", message: String(e) });
    }
  }

  async function apply() {
    if (phase.kind !== "ready") return;
    const p = phase.preview;
    setPhase({ kind: "applying" });
    try {
      await ipc.compoundApply({
        project: p.target_project,
        domain: p.target_domain,
        draft: p.plan.draft,
        summary: p.plan.summary,
        userMessage,
      });
      setPhase({
        kind: "done",
        project: p.target_project,
        domain: p.target_domain,
      });
      // Navigate to the newly written file.
      selectDomain(p.target_project, p.target_domain);
      setTimeout(() => onClose(), 600);
    } catch (e) {
      setPhase({ kind: "error", message: String(e) });
    }
  }

  if (!open) return null;

  return (
    <Dialog
      open={open}
      onClose={onClose}
      title={`Compound · ${project}/${targetDomain}`}
      width={720}
      footer={
        <div className="flex w-full items-center justify-between gap-2">
          <div className="text-caption-sm text-stone">
            {phase.kind === "ready" && (
              <span>
                소스 {phase.preview.sources.length}개 ·{" "}
                <CostEstimate
                  inputChars={phase.preview.approx_input_chars}
                  outputChars={phase.preview.approx_output_chars}
                />
              </span>
            )}
          </div>
          <div className="flex items-center gap-2">
            <SecondaryButton onClick={onClose}>취소</SecondaryButton>
            <PrimaryButton
              onClick={apply}
              disabled={phase.kind !== "ready"}
            >
              {phase.kind === "applying" ? (
                <span className="inline-flex items-center gap-1.5">
                  <Loader2 size={12} className="animate-spin" /> 저장 중…
                </span>
              ) : (
                "합성하기"
              )}
            </PrimaryButton>
          </div>
        </div>
      }
    >
      <div className="flex flex-col gap-4">
        <div className="rounded-md border border-hairline bg-surface-elevated p-3 text-[13px] leading-[1.6] text-body">
          <div className="mb-1 flex items-center gap-1.5 text-caption-sm uppercase tracking-[0.4px] text-accent-blue">
            <Sparkles size={11} /> 주제
          </div>
          <div className="text-ink">{topic}</div>
        </div>

        {/* Source count slider */}
        <div className="flex items-center gap-2 text-caption-sm">
          <Layers size={11} className="text-mute" />
          <span className="text-mute">소스 수:</span>
          {[3, 5, 8, 12].map((n) => (
            <button
              key={n}
              onClick={() => regenerate(n)}
              disabled={phase.kind === "fetching" || phase.kind === "applying"}
              className={cn(
                "rounded-sm border px-2 py-0.5 text-[11px] transition-colors",
                n === maxSources
                  ? "border-accent-blue bg-accent-blue-soft text-accent-blue"
                  : "border-hairline bg-surface-elevated text-body hover:text-on-dark",
              )}
            >
              {n}
            </button>
          ))}
          <span className="ml-auto text-stone">
            적을수록 빠르고 저렴합니다.
          </span>
        </div>

        {phase.kind === "fetching" && (
          <div className="flex items-center gap-2 rounded-md border border-hairline bg-surface-elevated p-4 text-[13px] text-mute">
            <Loader2 size={14} className="animate-spin" />
            관련 문서를 찾고 Writer가 합성 초안을 작성 중…
          </div>
        )}

        {phase.kind === "error" && (
          <div className="rounded-md border border-hairline bg-surface-elevated p-3 font-mono text-[12px] text-accent-red">
            {phase.message}
          </div>
        )}

        {phase.kind === "done" && (
          <div className="rounded-md border border-hairline bg-accent-green-soft p-3 text-[13px] text-accent-green">
            ✓ {phase.project}/{phase.domain} 에 합성 완료
          </div>
        )}

        {phase.kind === "ready" && (
          <>
            <section>
              <div className="mb-1 text-caption-sm uppercase tracking-[0.4px] text-mute">
                요약
              </div>
              <div className="text-[13px] leading-[1.5] text-ink">
                {phase.preview.plan.summary}
              </div>
              {phase.preview.plan.detail && (
                <div className="mt-1 text-caption-md leading-[1.6] text-body">
                  {phase.preview.plan.detail}
                </div>
              )}
            </section>

            <section>
              <div className="mb-1.5 flex items-center justify-between text-caption-sm uppercase tracking-[0.4px] text-mute">
                <span>사용한 소스</span>
                <span>
                  {phase.preview.plan.sources.length} /{" "}
                  {phase.preview.sources.length}
                </span>
              </div>
              <div className="flex flex-wrap gap-1.5">
                {phase.preview.sources.map((s) => {
                  const used = phase.preview.plan.sources.some(
                    (c) => c.project === s.project && c.domain === s.domain,
                  );
                  return (
                    <span
                      key={`${s.project}/${s.domain}`}
                      className={cn(
                        "inline-flex items-center gap-1 rounded-full border px-2 py-0.5 text-[11px]",
                        used
                          ? "border-accent-blue bg-accent-blue-soft text-accent-blue"
                          : "border-hairline bg-surface-elevated text-stone line-through",
                      )}
                    >
                      <FileText size={9} />
                      <span className="text-mute">{s.project}</span>
                      <span className="text-stone">/</span>
                      <span className="font-mono">{s.domain}</span>
                    </span>
                  );
                })}
              </div>
            </section>

            <section>
              <div className="mb-1 text-caption-sm uppercase tracking-[0.4px] text-mute">
                초안 (이대로 저장됩니다)
              </div>
              <pre className="max-h-[240px] overflow-auto rounded-sm border border-hairline bg-surface-elevated p-3 font-mono text-[11px] leading-[1.6] text-body whitespace-pre-wrap break-words">
                {phase.preview.plan.draft}
              </pre>
            </section>
          </>
        )}
      </div>
    </Dialog>
  );
}

/**
 * Back-of-envelope: input ~0.25 tok/char, output ~0.5 tok/char (Korean).
 * Claude Sonnet 4.6 rates as of 2026-05.
 */
function CostEstimate({
  inputChars,
  outputChars,
}: {
  inputChars: number;
  outputChars: number;
}) {
  const inTok = inputChars * 0.25;
  const outTok = outputChars * 0.5;
  const cost = (inTok / 1_000_000) * 3 + (outTok / 1_000_000) * 15;
  return (
    <span>
      예상 비용 <span className="text-body">≈ ${cost.toFixed(3)}</span>
    </span>
  );
}
